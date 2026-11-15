//! Multipart parsing and supervisor-owned temporary disk accounting.
use http_body_util::BodyExt;
use hyper::{body::Incoming, header::HeaderMap};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWriteExt, ReadBuf};

#[derive(Clone, Debug)]
pub struct UploadConfig {
    pub directory: PathBuf,
    pub max_file_bytes: usize,
    pub max_files: usize,
    pub max_fields: usize,
    pub max_field_bytes: usize,
    pub max_fields_bytes: usize,
    pub max_header_bytes: usize,
    pub max_disk_bytes: usize,
}
impl Default for UploadConfig {
    fn default() -> Self {
        Self {
            directory: PathBuf::new(),
            max_file_bytes: 8 * 1024 * 1024,
            max_files: 16,
            max_fields: 128,
            max_field_bytes: 65536,
            max_fields_bytes: 65536,
            max_header_bytes: 8192,
            max_disk_bytes: 256 * 1024 * 1024,
        }
    }
}
impl UploadConfig {
    pub(crate) fn validate(&self) -> io::Result<()> {
        if self.max_file_bytes > super::worker::BODY_CEILING
            || self.max_files > 128
            || self.max_fields > 1024
            || self.max_field_bytes > 262144
            || self.max_fields_bytes > 262144
            || !(1..=32768).contains(&self.max_header_bytes)
            || !(1..=1024 * 1024 * 1024).contains(&self.max_disk_bytes)
        {
            return Err(io::Error::other("invalid upload limits"));
        }
        Ok(())
    }
}
pub(crate) struct Budget {
    used: AtomicUsize,
    limit: usize,
}
impl Budget {
    pub fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            used: AtomicUsize::new(0),
            limit,
        })
    }
}
pub(crate) struct Spool {
    pub directory: tempfile::TempDir,
    budget: Arc<Budget>,
    charged: usize,
    stages: Vec<tempfile::TempPath>,
}
impl Drop for Spool {
    fn drop(&mut self) {
        let mut cleaned = true;
        for path in self.stages.drain(..) {
            if let Err(error) = path.close() {
                if error.kind() != io::ErrorKind::NotFound {
                    cleaned = false;
                }
            }
        }
        if let Err(error) = std::fs::remove_dir_all(self.directory.path()) {
            if error.kind() != io::ErrorKind::NotFound {
                cleaned = false;
            }
        }
        if cleaned {
            self.budget.used.fetch_sub(self.charged, Ordering::AcqRel);
        } else {
            eprintln!(
                "upload storage cleanup failed; retaining disk reservation until server restart"
            );
        }
    }
}
#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct MultipartMeta {
    pub parsed: bool,
    pub spool: String,
    pub raw_size: u64,
    pub fields: Vec<(String, String)>,
    pub uploads: Vec<UploadMeta>,
}
#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct UploadMeta {
    pub field_name: String,
    pub filename: String,
    pub media_type: Option<String>,
    pub size: u64,
    pub file: String,
}
impl Spool {
    fn charge(&mut self, n: usize) -> Result<(), u16> {
        let mut used = self.budget.used.load(Ordering::Acquire);
        loop {
            let next = used
                .checked_add(n)
                .filter(|total| *total <= self.budget.limit)
                .ok_or(503u16)?;
            match self.budget.used.compare_exchange_weak(
                used,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(actual) => used = actual,
            }
        }
        self.charged += n;
        Ok(())
    }
    pub fn stage(
        &mut self,
        index: usize,
        metadata: &MultipartMeta,
        parent: &Path,
    ) -> io::Result<PathBuf> {
        let size = metadata
            .uploads
            .get(index)
            .ok_or_else(|| io::Error::other("invalid upload index"))?
            .size as usize;
        self.charge(size)
            .map_err(|_| io::Error::other("upload temporary disk limit exceeded"))?;
        match tempfile::Builder::new()
            .prefix(".scripta-upload-")
            .tempfile_in(parent)
        {
            Ok(file) => {
                let path = file.into_temp_path();
                let result = path.to_path_buf();
                self.stages.push(path);
                Ok(result)
            }
            Err(e) => {
                self.charged -= size;
                self.budget.used.fetch_sub(size, Ordering::AcqRel);
                Err(e)
            }
        }
    }
}
/// Strict RFC 2046 boundary syntax, including quoted boundaries handled by MIME.
pub(crate) fn boundary(headers: &HeaderMap) -> Result<Option<String>, u16> {
    let values: Vec<_> = headers.get_all("content-type").iter().collect();
    if values.len() > 1 {
        return Err(400);
    }
    let Some(value) = values.first() else {
        return Ok(None);
    };
    let raw = value.to_str().map_err(|_| 400u16)?;
    let media: mime::Mime = raw.parse().map_err(|_| 400u16)?;
    if media.type_() != mime::MULTIPART {
        return Ok(None);
    }
    if media.subtype() != mime::FORM_DATA {
        return Err(415);
    }
    let boundary = media.get_param("boundary").ok_or(400u16)?.as_str();
    if boundary.is_empty()
        || boundary.len() > 70
        || boundary.ends_with(' ')
        || !boundary
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"'()+_,-./:=? ".contains(&b))
        || media
            .params()
            .filter(|(n, _)| n.as_str() == "boundary")
            .count()
            != 1
    {
        return Err(400);
    }
    Ok(Some(boundary.into()))
}
// Multer bounds body sizes, but not raw header bytes. Limit how much its reader
// may consume while seeking the next header, allowing one buffered chunk.
struct Reader {
    file: tokio::fs::File,
    allowance: Arc<AtomicUsize>,
    exceeded: Arc<AtomicBool>,
    pause: bool,
}
impl AsyncRead for Reader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.pause {
            self.pause = false;
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        let count = self
            .allowance
            .load(Ordering::Relaxed)
            .min(8192)
            .min(buf.remaining());
        if count == 0 {
            self.exceeded.store(true, Ordering::Relaxed);
            return Poll::Ready(Err(io::Error::other("multipart header intake limit")));
        }
        let mut limited = ReadBuf::new(buf.initialize_unfilled_to(count));
        match Pin::new(&mut self.file).poll_read(cx, &mut limited) {
            Poll::Ready(Ok(())) => {
                let n = limited.filled().len();
                self.allowance.fetch_sub(n, Ordering::Relaxed);
                self.pause = n != 0;
                buf.advance(n);
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}
fn parser_error(error: multer::Error, exceeded: &AtomicBool) -> u16 {
    if exceeded.load(Ordering::Relaxed)
        || matches!(
            error,
            multer::Error::FieldSizeExceeded { .. } | multer::Error::StreamSizeExceeded { .. }
        )
    {
        413
    } else {
        400
    }
}
pub(crate) async fn receive_raw(
    incoming: &mut Incoming,
    root: &Path,
    budget: Arc<Budget>,
    max_body: usize,
) -> Result<(Spool, MultipartMeta), u16> {
    let directory = tempfile::Builder::new()
        .prefix("request-")
        .tempdir_in(root)
        .map_err(|_| 500u16)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
            .map_err(|_| 500u16)?;
    }
    let mut spool = Spool {
        directory,
        budget,
        charged: 0,
        stages: vec![],
    };
    let raw = spool.directory.path().join("body");
    let mut output = tokio::fs::File::create(&raw).await.map_err(|_| 500u16)?;
    let mut size = 0usize;
    while let Some(frame) = incoming.frame().await {
        let data = frame.map_err(|_| 400u16)?.into_data().map_err(|_| 400u16)?;
        if data.len() > max_body.saturating_sub(size) {
            return Err(413);
        }
        spool.charge(data.len())?;
        size += data.len();
        output.write_all(&data).await.map_err(|_| 500u16)?;
    }
    output.flush().await.map_err(|_| 500u16)?;
    drop(output);
    let meta = MultipartMeta {
        parsed: false,
        spool: spool
            .directory
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .into(),
        raw_size: size as u64,
        fields: vec![],
        uploads: vec![],
    };
    Ok((spool, meta))
}
pub(crate) async fn receive(
    incoming: &mut Incoming,
    root: &Path,
    budget: Arc<Budget>,
    config: &UploadConfig,
    max_body: usize,
    boundary: String,
) -> Result<(Spool, MultipartMeta), u16> {
    let (mut spool, mut meta) = receive_raw(incoming, root, budget, max_body).await?;
    meta.parsed = true;
    let raw = spool.directory.path().join("body");
    let allowance = Arc::new(AtomicUsize::new(0));
    let exceeded = Arc::new(AtomicBool::new(false));
    let reader = Reader {
        file: tokio::fs::File::open(&raw).await.map_err(|_| 500u16)?,
        allowance: allowance.clone(),
        exceeded: exceeded.clone(),
        pause: false,
    };
    let constraints = multer::Constraints::new().size_limit(
        multer::SizeLimit::new()
            .whole_stream(max_body as u64)
            .per_field(config.max_file_bytes.max(config.max_field_bytes) as u64),
    );
    let mut parser =
        multer::Multipart::with_reader_with_constraints(reader, boundary.clone(), constraints);
    let mut files = 0;
    let mut fields = 0;
    let mut field_bytes = 0usize;
    loop {
        allowance.store(
            config.max_header_bytes + 16384 + 4 * boundary.len(),
            Ordering::Relaxed,
        );
        let Some(mut field) = parser
            .next_field()
            .await
            .map_err(|e| parser_error(e, &exceeded))?
        else {
            break;
        };
        let headers = field.headers();
        let header_size: usize = headers
            .iter()
            .map(|(k, v)| k.as_str().len() + v.as_bytes().len() + 4)
            .sum();
        if header_size > config.max_header_bytes || headers.len() > 32 {
            return Err(413);
        }
        if headers.get_all("content-disposition").iter().count() != 1
            || headers.get_all("content-type").iter().count() > 1
        {
            return Err(400);
        }
        if headers.contains_key("content-transfer-encoding") {
            return Err(415);
        }
        for (_, v) in headers {
            std::str::from_utf8(v.as_bytes()).map_err(|_| 400u16)?;
        }
        let disposition = headers.get("content-disposition").ok_or(400u16)?;
        if !disposition
            .as_bytes()
            .split(|b| *b == b';')
            .next()
            .unwrap_or_default()
            .eq_ignore_ascii_case(b"form-data")
        {
            return Err(400);
        }
        let name = field.name().ok_or(400u16)?.to_owned();
        if name.len() > 256 || name.contains(['\0', '\r', '\n']) {
            return Err(400);
        }
        let filename = field.file_name().map(str::to_owned);
        let media = headers
            .get("content-type")
            .map(|v| std::str::from_utf8(v.as_bytes()).unwrap().to_owned());
        if filename
            .as_ref()
            .is_some_and(|s| s.len() > 1024 || s.contains(['\0', '\r', '\n']))
            || media.as_ref().is_some_and(|s| s.len() > 256)
        {
            return Err(413);
        }
        allowance.store(usize::MAX / 2, Ordering::Relaxed);
        if let Some(filename) = filename {
            files += 1;
            if files > config.max_files {
                return Err(413);
            }
            let file = format!("file-{}", files - 1);
            let mut output = tokio::fs::File::create(spool.directory.path().join(&file))
                .await
                .map_err(|_| 500u16)?;
            let mut size = 0usize;
            while let Some(chunk) = field
                .chunk()
                .await
                .map_err(|e| parser_error(e, &exceeded))?
            {
                if chunk.len() > config.max_file_bytes.saturating_sub(size) {
                    return Err(413);
                }
                spool.charge(chunk.len())?;
                size += chunk.len();
                output.write_all(&chunk).await.map_err(|_| 500u16)?;
            }
            output.flush().await.map_err(|_| 500u16)?;
            drop(output);
            if filename.is_empty() {
                if size != 0 {
                    return Err(400);
                };
                std::fs::remove_file(spool.directory.path().join(&file)).map_err(|_| 500u16)?;
                continue;
            }
            meta.uploads.push(UploadMeta {
                field_name: name,
                filename,
                media_type: media,
                size: size as u64,
                file,
            });
        } else {
            fields += 1;
            if fields > config.max_fields {
                return Err(413);
            }
            if let Some(media) = &media {
                let media: mime::Mime = media.parse().map_err(|_| 400u16)?;
                if media.type_() == mime::MULTIPART
                    || media
                        .get_param("charset")
                        .is_some_and(|s| !s.as_str().eq_ignore_ascii_case("utf-8"))
                {
                    return Err(415);
                }
            }
            field_bytes = field_bytes.checked_add(name.len()).ok_or(413u16)?;
            if field_bytes > config.max_fields_bytes {
                return Err(413);
            }
            let mut value = vec![];
            while let Some(chunk) = field
                .chunk()
                .await
                .map_err(|e| parser_error(e, &exceeded))?
            {
                if chunk.len() > config.max_field_bytes.saturating_sub(value.len())
                    || chunk.len() > config.max_fields_bytes.saturating_sub(field_bytes)
                {
                    return Err(413);
                }
                field_bytes += chunk.len();
                value.extend_from_slice(&chunk);
            }
            meta.fields
                .push((name, String::from_utf8(value).map_err(|_| 400u16)?));
        }
    }
    Ok((spool, meta))
}

/// Native paths in internal IPC, without requiring UTF-8 directory names.
#[derive(Serialize, Deserialize)]
pub(crate) struct WirePath(Vec<u32>);
impl WirePath {
    pub fn new(path: &Path) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            Self(
                path.as_os_str()
                    .as_bytes()
                    .iter()
                    .map(|b| u32::from(*b))
                    .collect(),
            )
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            Self(path.as_os_str().encode_wide().map(u32::from).collect())
        }
    }
    pub fn path(self) -> io::Result<PathBuf> {
        if self.0.len() > 32768 || self.0.contains(&0) {
            return Err(io::Error::other("invalid upload staging path"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let bytes = self
                .0
                .into_iter()
                .map(u8::try_from)
                .collect::<Result<Vec<_>, _>>()
                .map_err(io::Error::other)?;
            Ok(std::ffi::OsString::from_vec(bytes).into())
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt;
            let units = self
                .0
                .into_iter()
                .map(u16::try_from)
                .collect::<Result<Vec<_>, _>>()
                .map_err(io::Error::other)?;
            Ok(std::ffi::OsString::from_wide(&units).into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supervisor_owned_stages_and_spools_drop_together_and_release_disk_budget() {
        let root = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let budget = Budget::new(10);
        let mut spool = Spool {
            directory: tempfile::Builder::new()
                .prefix("request-")
                .tempdir_in(root.path())
                .unwrap(),
            budget: budget.clone(),
            charged: 0,
            stages: vec![],
        };
        spool.charge(5).unwrap();
        assert_eq!(spool.charge(6), Err(503));
        let metadata = MultipartMeta {
            parsed: true,
            spool: String::new(),
            raw_size: 5,
            fields: vec![],
            uploads: vec![UploadMeta {
                field_name: "f".into(),
                filename: "x".into(),
                media_type: None,
                size: 5,
                file: "file-0".into(),
            }],
        };
        let stage = spool.stage(0, &metadata, destination.path()).unwrap();
        std::fs::write(&stage, b"partial").unwrap();
        assert_eq!(budget.used.load(Ordering::Acquire), 10);
        assert!(spool.stage(0, &metadata, destination.path()).is_err());
        drop(spool);
        assert!(!stage.exists());
        assert!(std::fs::read_dir(root.path()).unwrap().next().is_none());
        assert_eq!(budget.used.load(Ordering::Acquire), 0);
    }
    #[test]
    fn boundaries_and_native_path_transport_have_explicit_bounds() {
        for (boundary, valid) in [
            ("a".repeat(70), true),
            ("a".repeat(71), false),
            ("".into(), false),
            ("a ".into(), false),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(
                "content-type",
                format!("multipart/form-data; boundary=\"{boundary}\"")
                    .parse()
                    .unwrap(),
            );
            assert_eq!(super::boundary(&headers).is_ok(), valid);
        }
        assert!(WirePath(vec![0]).path().is_err());
        assert!(WirePath(vec![1; 32769]).path().is_err());
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let path = PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/\xff".to_vec()));
            assert_eq!(WirePath::new(&path).path().unwrap(), path);
        }
    }
}
