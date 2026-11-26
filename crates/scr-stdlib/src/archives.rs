//! Bounded archive codecs and exclusive, owned-tree extraction.
use super::{FilesystemPolicy, failure, metadata};
use crate::{bytes, paths};
use scr_runtime::{
    MapKey, NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, BufRead, Cursor, Read, Seek, SeekFrom, Write},
    mem::size_of,
    path::{Path, PathBuf},
    rc::Rc,
};
#[path = "gzip.rs"]
mod gzip;
#[path = "archive_tar.rs"]
mod tar_codec;
#[path = "archive_zip.rs"]
mod zip_codec;
const CEILING: usize = 64 * 1024 * 1024;
const CHUNK: usize = 8192;
fn invalid(message: &str) -> NativeError {
    NativeError::new("ArchiveError", message)
}
fn limit(message: &str) -> NativeError {
    bytes::resource(message)
}
#[derive(Clone, Copy, PartialEq)]
enum Format {
    Zip,
    Tar,
    Gzip,
}
struct Options {
    input: usize,
    output: usize,
    entries: usize,
    name: usize,
    depth: usize,
    stored: bool,
}
impl Options {
    fn parse(value: Option<&Value>, format: Format, operation: &str) -> NativeResult<Self> {
        let mut result = Self {
            input: CEILING,
            output: CEILING,
            entries: 10000,
            name: 1024,
            depth: 64,
            stored: false,
        };
        if let Some(Value::Map(values)) = value {
            for (key, value) in values {
                let MapKey::String(key) = key else {
                    return Err(NativeError::type_error(
                        "archive option keys must be strings",
                    ));
                };
                if key == "compression" && format == Format::Zip && operation == "encode" {
                    result.stored = match value {
                        Value::String(s) if s == "stored" => true,
                        Value::String(s) if s == "deflate" => false,
                        _ => {
                            return Err(NativeError::type_error(
                                "compression must be stored or deflate",
                            ));
                        }
                    };
                    continue;
                }
                let maximum = match key.as_str() {
                    "maxInputBytes" | "maxOutputBytes" => CEILING,
                    "maxEntries" if format != Format::Gzip => 10000,
                    "maxMembers" if format == Format::Gzip && operation == "decompress" => 10000,
                    "maxNameBytes" if format != Format::Gzip => 4096,
                    "maxDepth" if format != Format::Gzip => 128,
                    _ => {
                        return Err(NativeError::type_error(
                            "unknown option for this archive operation",
                        ));
                    }
                };
                let Value::Int(n) = value else {
                    return Err(NativeError::type_error("archive limits must be ints"));
                };
                let n = usize::try_from(*n)
                    .ok()
                    .filter(|n| *n <= maximum)
                    .ok_or_else(|| {
                        NativeError::new(
                            "RangeError",
                            format!("{key} must be between 0 and {maximum}"),
                        )
                    })?;
                match key.as_str() {
                    "maxInputBytes" => result.input = n,
                    "maxOutputBytes" => result.output = n,
                    "maxEntries" | "maxMembers" => result.entries = n,
                    "maxNameBytes" => result.name = n,
                    "maxDepth" => result.depth = n,
                    _ => unreachable!(),
                }
            }
        }
        Ok(result)
    }
    fn input(&self, size: usize) -> NativeResult<()> {
        if size > self.input {
            Err(limit("archive exceeds maxInputBytes"))
        } else {
            Ok(())
        }
    }
}
fn name<'a>(input: &'a str, directory: bool, options: &Options) -> NativeResult<&'a str> {
    let value = if directory {
        input.strip_suffix('/').unwrap_or(input)
    } else {
        input
    };
    if value.len() > options.name {
        return Err(limit("archive path exceeds maxNameBytes"));
    }
    if value.is_empty()
        || value
            .bytes()
            .any(|b| b < 32 || b == 127 || b"\\:<>\"|?*".contains(&b))
    {
        return Err(invalid("unsafe archive path"));
    }
    let mut depth = 0;
    for part in value.split('/') {
        depth += 1;
        if depth > options.depth {
            return Err(limit("archive path exceeds maxDepth"));
        }
        if matches!(part, "" | "." | "..") || part.ends_with(['.', ' ']) {
            return Err(invalid("unsafe archive path component"));
        }
        let stem = part
            .split('.')
            .next()
            .unwrap()
            .trim_matches(' ')
            .to_ascii_uppercase();
        if matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$"
        ) || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|n| {
                matches!(
                    n,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        }) {
            return Err(invalid("reserved Windows archive path"));
        }
    }
    Ok(value)
}
struct Node {
    path: String,
    directory: bool,
    explicit: bool,
}
#[derive(Default)]
struct Names {
    nodes: BTreeMap<String, Node>,
    records: usize,
}
impl Names {
    fn add(
        &mut self,
        context: &RuntimeContext<'_>,
        path: &str,
        directory: bool,
        options: &Options,
    ) -> NativeResult<()> {
        self.records += 1;
        if self.records > options.entries {
            return Err(limit("archive exceeds maxEntries"));
        }
        let mut current = String::new();
        let parts: Vec<_> = path.split('/').collect();
        for (index, part) in parts.iter().enumerate() {
            context.check_cancelled()?;
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(part);
            let last = index + 1 == parts.len();
            let is_directory = !last || directory;
            let key = current.to_ascii_lowercase();
            if let Some(node) = self.nodes.get_mut(&key) {
                if node.path != current
                    || !node.directory
                    || !is_directory
                    || (last && node.explicit)
                {
                    return Err(invalid("conflicting or duplicate archive paths"));
                }
                if last {
                    node.explicit = true;
                }
            } else {
                if self.nodes.len() >= options.entries {
                    return Err(limit("archive path nodes exceed maxEntries"));
                }
                self.nodes.insert(
                    key,
                    Node {
                        path: current.clone(),
                        directory: is_directory,
                        explicit: last,
                    },
                );
            }
        }
        Ok(())
    }
}
struct Entry {
    path: String,
    data: Option<Vec<u8>>,
}
struct Source<'a> {
    path: &'a str,
    data: Option<&'a [u8]>,
}
fn sources<'a>(
    context: &RuntimeContext<'_>,
    values: &'a Value,
    options: &Options,
) -> NativeResult<Vec<Source<'a>>> {
    let Value::List(values) = values else {
        unreachable!()
    };
    if values.len() > options.entries {
        return Err(limit("archive exceeds maxEntries"));
    }
    let mut names = Names::default();
    let mut total = 0usize;
    let mut result = Vec::new();
    for value in values {
        context.check_cancelled()?;
        let bad = || {
            NativeError::type_error(
                "entries must be maps with exactly path:string and data:bytes-or-null",
            )
        };
        let Value::Map(fields) = value else {
            return Err(bad());
        };
        if fields.len() != 2 {
            return Err(bad());
        }
        let mut path = None;
        let mut data = None;
        for (key, value) in fields {
            match (key, value) {
                (MapKey::String(k), Value::String(s)) if k == "path" && path.is_none() => {
                    path = Some(s.as_str())
                }
                (MapKey::String(k), Value::Bytes(b)) if k == "data" && data.is_none() => {
                    data = Some(Some(b.as_ref()))
                }
                (MapKey::String(k), Value::Null) if k == "data" && data.is_none() => {
                    data = Some(None)
                }
                _ => return Err(bad()),
            }
        }
        let (path, data) = (path.ok_or_else(bad)?, data.ok_or_else(bad)?);
        let path = name(path, data.is_none(), options)?;
        names.add(context, path, data.is_none(), options)?;
        total = total
            .checked_add(data.map_or(0, <[u8]>::len))
            .ok_or_else(|| limit("archive input size overflow"))?;
        options.input(total)?;
        result
            .try_reserve(1)
            .map_err(|_| limit("archive entry allocation failed"))?;
        result.push(Source { path, data });
    }
    Ok(result)
}
fn row_budget(
    context: &RuntimeContext<'_>,
    logical: &mut usize,
    expanded: &mut usize,
    path: &str,
    size: usize,
    options: &Options,
) -> NativeResult<()> {
    *expanded = expanded
        .checked_add(size)
        .filter(|n| *n <= options.output)
        .ok_or_else(|| limit("expanded archive exceeds maxOutputBytes"))?;
    let fixed = size_of::<Value>()
        + 2 * (size_of::<MapKey>() + size_of::<Value>())
        + "path".len()
        + "data".len();
    bytes::grow(context, logical, fixed)?;
    bytes::grow(context, logical, path.len())?;
    bytes::grow(context, logical, size)
}
fn values(context: &RuntimeContext<'_>, entries: Vec<Entry>) -> NativeResult<Value> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(entries.len())
        .map_err(|_| limit("archive result allocation failed"))?;
    for entry in entries {
        context.check_cancelled()?;
        result.push(Value::map(vec![
            (MapKey::String("path".into()), Value::String(entry.path)),
            (
                MapKey::String("data".into()),
                entry.data.map_or(Value::Null, |b| Value::Bytes(b.into())),
            ),
        ]));
    }
    Ok(Value::list(result))
}
// Convert library I/O errors without losing cancellation/resource error kinds.
struct Io<'a, 'ctx> {
    context: &'a RuntimeContext<'ctx>,
    failure: RefCell<Option<NativeError>>,
}
impl<'a, 'ctx> Io<'a, 'ctx> {
    fn new(context: &'a RuntimeContext<'ctx>) -> Self {
        Self {
            context,
            failure: RefCell::new(None),
        }
    }
    fn fail(&self, error: NativeError) -> io::Error {
        if self.failure.borrow().is_none() {
            *self.failure.borrow_mut() = Some(error);
        }
        io::Error::other("archive I/O failed")
    }
    fn check(&self) -> io::Result<()> {
        if self.failure.borrow().is_some() {
            return Err(io::Error::other("archive I/O already failed"));
        }
        self.context.check_cancelled().map_err(|e| self.fail(e))
    }
    fn result<T, E>(&self, result: Result<T, E>, message: &str) -> NativeResult<T> {
        self.context.check_cancelled()?;
        if let Some(error) = self.failure.borrow().as_ref() {
            return Err(error.clone());
        }
        result.map_err(|_| invalid(message))
    }
}
struct Input<'a, 'ctx> {
    io: &'a Io<'a, 'ctx>,
    cursor: Cursor<&'a [u8]>,
}
impl<'a, 'ctx> Input<'a, 'ctx> {
    fn new(io: &'a Io<'a, 'ctx>, data: &'a [u8]) -> Self {
        Self {
            io,
            cursor: Cursor::new(data),
        }
    }
}
impl Read for Input<'_, '_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.io.check()?;
        let n = buffer.len().min(CHUNK);
        self.cursor.read(&mut buffer[..n])
    }
}
impl BufRead for Input<'_, '_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.io.check()?;
        let data = self.cursor.fill_buf()?;
        Ok(&data[..data.len().min(CHUNK)])
    }
    fn consume(&mut self, n: usize) {
        self.cursor.consume(n)
    }
}
struct Output<'a, 'ctx> {
    io: &'a Io<'a, 'ctx>,
    cursor: Cursor<Vec<u8>>,
    maximum: usize,
    end: u64,
}
impl<'a, 'ctx> Output<'a, 'ctx> {
    fn new(io: &'a Io<'a, 'ctx>, maximum: usize) -> Self {
        Self {
            io,
            cursor: Cursor::new(Vec::new()),
            maximum,
            end: 0,
        }
    }
    fn bytes(self) -> Vec<u8> {
        self.cursor.into_inner()
    }
}
// After recording a failure, discard subsequent encoder writes without allocating.
// Some third-party writers finalize on Drop and print diagnostics on I/O errors.
// A discard sink lets that cleanup finish quietly; Io::result always propagates
// the sticky native error and never publishes this incomplete buffer.
impl Write for Output<'_, '_> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let _ = self.io.check();
        let next = self.cursor.position().saturating_add(data.len() as u64);
        let size = next.max(self.end);
        if self.io.failure.borrow().is_none() {
            let validation = (|| {
                if size > self.maximum as u64 {
                    return Err(limit("archive output exceeds maxOutputBytes"));
                }
                bytes::payload(self.io.context, size as usize)?;
                let extra = size as usize - self.cursor.get_ref().len();
                self.cursor
                    .get_mut()
                    .try_reserve(extra)
                    .map_err(|_| limit("archive output allocation failed"))?;
                Ok(())
            })();
            if let Err(error) = validation {
                let _ = self.io.fail(error);
            }
        }
        self.end = size;
        if self.io.failure.borrow().is_some() {
            self.cursor.set_position(next);
            Ok(data.len())
        } else {
            self.cursor.write(data)
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        let _ = self.io.check();
        Ok(())
    }
}
impl Seek for Output<'_, '_> {
    fn seek(&mut self, offset: SeekFrom) -> io::Result<u64> {
        let _ = self.io.check();
        let end = i128::from(self.end);
        let position = match offset {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::End(n) => end + i128::from(n),
            SeekFrom::Current(n) => i128::from(self.cursor.position()) + i128::from(n),
        };
        if (position < 0 || position > end) && self.io.failure.borrow().is_none() {
            let _ = self
                .io
                .fail(invalid("archive writer sought outside its output"));
        }
        let position = position.clamp(0, i128::from(u64::MAX)) as u64;
        self.cursor.set_position(position);
        Ok(position)
    }
}

fn body(
    context: &RuntimeContext<'_>,
    io: &Io<'_, '_>,
    reader: &mut impl Read,
    size: usize,
) -> NativeResult<Vec<u8>> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(size)
        .map_err(|_| limit("expanded archive allocation failed"))?;
    let mut buffer = [0; CHUNK];
    loop {
        context.check_cancelled()?;
        let n = io.result(
            reader.read(&mut buffer),
            "invalid or truncated archive entry data",
        )?;
        if n == 0 {
            break;
        }
        if result.len() + n > size {
            return Err(invalid("archive entry exceeds its declared size"));
        }
        result.extend_from_slice(&buffer[..n]);
    }
    if result.len() != size {
        return Err(invalid("truncated archive entry"));
    }
    Ok(result)
}
struct OwnedRoot {
    path: PathBuf,
    identity: Option<same_file::Handle>,
    policy: Rc<FilesystemPolicy>,
    committed: bool,
}
impl OwnedRoot {
    fn cleanup(&mut self) -> NativeResult<()> {
        if self.committed {
            return Ok(());
        }
        self.policy.check_path(&self.path, true)?;
        let info = match fs::symlink_metadata(&self.path) {
            Ok(info) => info,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                self.committed = true;
                return Ok(());
            }
            Err(e) => return Err(failure("inspect extraction cleanup root", &self.path, e)),
        };
        if info.file_type().is_symlink() || !info.is_dir() {
            return Err(invalid("extraction root was replaced; refusing cleanup"));
        }
        let current = same_file::Handle::from_path(&self.path)
            .map_err(|e| failure("identify extraction cleanup root", &self.path, e))?;
        if self.identity.as_ref() != Some(&current) {
            return Err(invalid(
                "extraction root identity changed; refusing cleanup",
            ));
        }
        drop(current);
        fs::remove_dir_all(&self.path)
            .map_err(|e| failure("remove failed extraction", &self.path, e))?;
        self.identity.take();
        self.committed = true;
        Ok(())
    }
}
impl Drop for OwnedRoot {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}
fn extract(
    context: &RuntimeContext<'_>,
    entries: Vec<Entry>,
    destination: &str,
    options: &Options,
    policy: Rc<FilesystemPolicy>,
) -> NativeResult<Value> {
    extract_with(context, entries, destination, options, policy, || {
        context.check_cancelled()
    })
}
fn private_directory(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.recursive(false).create(path)
}
fn extract_with(
    context: &RuntimeContext<'_>,
    entries: Vec<Entry>,
    destination: &str,
    options: &Options,
    policy: Rc<FilesystemPolicy>,
    mut checkpoint: impl FnMut() -> NativeResult<()>,
) -> NativeResult<Value> {
    let requested = paths::resolve(context, destination)?;
    policy.check(context, &requested, true)?;
    if requested.file_name().is_none() {
        return Err(invalid(
            "extraction destination requires a new directory name",
        ));
    }
    // Reject even empty directories and dangling final links before canonicalizing the parent.
    match fs::symlink_metadata(&requested) {
        Ok(_) => {
            return Err(NativeError::new(
                "AlreadyExistsError",
                "extraction destination already exists",
            ));
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(failure("inspect extraction destination", &requested, e)),
    }
    let parent = requested
        .parent()
        .ok_or_else(|| invalid("extraction destination has no parent"))?;
    policy.check(context, parent, true)?;
    metadata(parent, true)?;
    let parent =
        fs::canonicalize(parent).map_err(|e| failure("resolve extraction parent", parent, e))?;
    let destination = parent.join(requested.file_name().unwrap());
    policy.check(context, &destination, true)?;
    let mut names = Names::default();
    for entry in &entries {
        names.add(context, &entry.path, entry.data.is_none(), options)?;
    }
    context.check_cancelled()?;
    private_directory(&destination)
        .map_err(|e| failure("create extraction root", &destination, e))?;
    let identity = match same_file::Handle::from_path(&destination) {
        Ok(identity) => identity,
        Err(error) => {
            let cleanup = fs::remove_dir(&destination);
            let mut error = failure("retain extraction root", &destination, error);
            if let Err(e) = cleanup {
                error.message.push_str(&format!("; cleanup failed: {e}"));
            }
            return Err(error);
        }
    };
    let mut root = OwnedRoot {
        path: destination,
        identity: Some(identity),
        policy: policy.clone(),
        committed: false,
    };
    let result: NativeResult<()> = (|| {
        // Every logical parent is created exclusively once. Filesystem-specific
        // Unicode/case aliases therefore fail rather than merging directories.
        for node in names.nodes.values().filter(|node| node.directory) {
            checkpoint()?;
            let path = root.path.join(&node.path);
            policy.check(context, &path, true)?;
            private_directory(&path)
                .map_err(|e| failure("create extracted directory", &path, e))?;
        }
        for entry in entries {
            checkpoint()?;
            if let Some(data) = entry.data {
                let path = root.path.join(&entry.path);
                policy.check(context, &path, true)?;
                let mut open = OpenOptions::new();
                open.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    open.mode(0o600);
                }
                let mut file = open
                    .open(&path)
                    .map_err(|e| failure("create extracted file", &path, e))?;
                if !file
                    .metadata()
                    .map_err(|e| failure("inspect extracted file", &path, e))?
                    .is_file()
                {
                    return Err(NativeError::new(
                        "FilesystemError",
                        "extracted destination is not a regular file",
                    ));
                }
                for chunk in data.chunks(CHUNK) {
                    checkpoint()?;
                    file.write_all(chunk)
                        .map_err(|e| failure("write extracted file", &path, e))?;
                }
                file.flush()
                    .map_err(|e| failure("flush extracted file", &path, e))?;
            }
        }
        checkpoint()?;
        Ok(())
    })();
    if let Err(mut error) = result {
        if let Err(cleanup) = root.cleanup() {
            error
                .message
                .push_str(&format!("; extraction cleanup failed: {cleanup}"));
        }
        return Err(error);
    }
    root.committed = true;
    Ok(Value::Null)
}
fn data(value: &Value) -> &[u8] {
    let Value::Bytes(data) = value else {
        unreachable!()
    };
    data
}
pub(super) fn register(runtime: &mut Runtime, policy: Rc<FilesystemPolicy>) -> NativeResult<()> {
    use NativeType::{Bytes, List, Map, Null, String as Text};
    for (namespace, format) in [
        ("Zip", Format::Zip),
        ("Tar", Format::Tar),
        ("Gzip", Format::Gzip),
    ] {
        let operations: &[&str] = if format == Format::Gzip {
            &["compress", "decompress"]
        } else {
            &["encode", "decode", "extract"]
        };
        for &operation in operations {
            let parameters = match operation {
                "encode" => vec![List],
                "extract" => vec![Bytes, Text],
                _ => vec![Bytes],
            };
            let returns = match operation {
                "decode" => List,
                "extract" => Null,
                _ => Bytes,
            };
            let policy = policy.clone();
            runtime.register(
                format!("{namespace}::{operation}"),
                NativeFunction::new(
                    NativeSignature::new(parameters, returns).with_optional(vec![Map]),
                    move |context, args| {
                        context.check_cancelled()?;
                        let options = Options::parse(
                            args.get(if operation == "extract" { 2 } else { 1 }),
                            format,
                            operation,
                        )?;
                        if format == Format::Gzip {
                            return gzip::invoke(context, operation, data(&args[0]), &options);
                        }
                        if operation == "encode" {
                            let entries = sources(context, &args[0], &options)?;
                            let result = if format == Format::Zip {
                                zip_codec::encode(context, &entries, &options)
                            } else {
                                tar_codec::encode(context, &entries, &options)
                            }?;
                            return Ok(Value::Bytes(result.into()));
                        }
                        if operation == "extract" {
                            let destination = paths::resolve(context, paths::string(&args[1])?)?;
                            policy.check(context, &destination, true)?;
                        }
                        options.input(data(&args[0]).len())?;
                        bytes::payload(context, 0)?;
                        let entries = if format == Format::Zip {
                            zip_codec::decode(context, data(&args[0]), &options)
                        } else {
                            tar_codec::decode(context, data(&args[0]), &options)
                        }?;
                        if operation == "decode" {
                            values(context, entries)
                        } else {
                            extract(
                                context,
                                entries,
                                paths::string(&args[1])?,
                                &options,
                                policy.clone(),
                            )
                        }
                    },
                ),
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scr_runtime::ContextOptions;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    fn with_context(test: impl FnOnce(&mut RuntimeContext<'_>)) {
        let mut settings = ContextOptions::default();
        let mut input = std::io::empty();
        let mut output = Vec::new();
        let mut error = Vec::new();
        test(&mut RuntimeContext {
            working_directory: &mut settings.working_directory,
            script_path: None,
            cli_arguments: &[],
            environment: &mut settings.environment,
            input: &mut input,
            output: &mut output,
            error: &mut error,
            runtime_identity: None,
            cancellation: None,
            permissions: &settings.permissions,
            call_span: Default::default(),
            max_value_bytes: None,
        });
    }
    #[test]
    fn exact_logical_archive_and_gzip_results_include_bytes_keys_and_slots() {
        with_context(|context| {
            let input = Value::list(vec![Value::map(vec![
                (MapKey::String("path".into()), Value::String("n".into())),
                (
                    MapKey::String("data".into()),
                    Value::Bytes([0u8, 255, 1].as_slice().into()),
                ),
            ])]);
            let decoded_budget = 4 * size_of::<Value>() + 2 * size_of::<MapKey>() + 8 + 1 + 3;
            for format in [Format::Zip, Format::Tar] {
                context.max_value_bytes = None;
                let options = Options::parse(None, format, "encode").unwrap();
                let entries = sources(context, &input, &options).unwrap();
                let encode = |context: &RuntimeContext<'_>| {
                    if format == Format::Zip {
                        zip_codec::encode(context, &entries, &options)
                    } else {
                        tar_codec::encode(context, &entries, &options)
                    }
                };
                let encoded = encode(context).unwrap();
                let needed = size_of::<Value>() + encoded.len();
                context.max_value_bytes = Some(needed);
                assert_eq!(encode(context).unwrap(), encoded);
                context.max_value_bytes = Some(needed - 1);
                assert_eq!(encode(context).unwrap_err().kind, "ResourceLimitError");
                let decode = |context: &RuntimeContext<'_>| {
                    if format == Format::Zip {
                        zip_codec::decode(context, &encoded, &options)
                    } else {
                        tar_codec::decode(context, &encoded, &options)
                    }
                };
                context.max_value_bytes = Some(decoded_budget);
                let result = decode(context).unwrap();
                assert_eq!(values(context, result).unwrap(), input);
                context.max_value_bytes = Some(decoded_budget - 1);
                assert_eq!(decode(context).err().unwrap().kind, "ResourceLimitError");
            }
            context.max_value_bytes = None;
            let options = Options::parse(None, Format::Gzip, "compress").unwrap();
            let compressed = gzip::invoke(context, "compress", b"abc", &options).unwrap();
            let Value::Bytes(data) = &compressed else {
                unreachable!()
            };
            for (operation, input, expected, needed) in [
                (
                    "compress",
                    b"abc".as_slice(),
                    compressed.clone(),
                    size_of::<Value>() + data.len(),
                ),
                (
                    "decompress",
                    data.as_ref(),
                    Value::Bytes(b"abc".as_slice().into()),
                    size_of::<Value>() + 3,
                ),
            ] {
                context.max_value_bytes = Some(needed);
                assert_eq!(
                    gzip::invoke(context, operation, input, &options).unwrap(),
                    expected
                );
                context.max_value_bytes = Some(needed - 1);
                assert_eq!(
                    gzip::invoke(context, operation, input, &options)
                        .unwrap_err()
                        .kind,
                    "ResourceLimitError"
                );
            }
        });
    }
    #[test]
    fn zip_deflate_roundtrips_across_output_chunk_boundaries() {
        with_context(|context| {
            let options = Options::parse(None, Format::Zip, "encode").unwrap();
            for size in [0, 1, CHUNK - 1, CHUNK, CHUNK + 1, CHUNK * 3 + 17] {
                let data: Vec<_> = (0..size).map(|i| ((i * 7 + i / 99) % 251) as u8).collect();
                let encoded = zip_codec::encode(
                    context,
                    &[Source {
                        path: "data",
                        data: Some(&data),
                    }],
                    &options,
                )
                .unwrap();
                let mut decoded = zip_codec::decode(context, &encoded, &options).unwrap();
                assert_eq!(decoded.len(), 1);
                assert_eq!(decoded.pop().unwrap().data.unwrap(), data, "size {size}");
            }
        });
    }
    #[test]
    fn cancellation_during_partial_extraction_removes_only_its_owned_new_root() {
        with_context(|context| {
            let parent = tempfile::tempdir().unwrap();
            fs::write(parent.path().join("keep"), b"unchanged").unwrap();
            let destination = parent.path().join("out");
            let options = Options::parse(None, Format::Zip, "extract").unwrap();
            for cancel_at in 1..=5 {
                let flag = Arc::new(AtomicBool::new(false));
                context.cancellation = Some(flag.clone());
                let mut checks = 0;
                let entries = vec![Entry {
                    path: "data".into(),
                    data: Some(vec![0; CHUNK * 3]),
                }];
                let result = extract_with(
                    context,
                    entries,
                    destination.to_str().unwrap(),
                    &options,
                    Rc::new(FilesystemPolicy::default()),
                    || {
                        checks += 1;
                        if checks == cancel_at {
                            flag.store(true, Ordering::Release);
                        }
                        context.check_cancelled()
                    },
                );
                assert_eq!(result.unwrap_err().kind, "Interrupted");
                assert_eq!(checks, cancel_at);
                assert!(!destination.exists());
                assert_eq!(fs::read(parent.path().join("keep")).unwrap(), b"unchanged");
            }
        });
    }
    #[test]
    fn replaced_extraction_roots_are_not_deleted_during_failure_cleanup() {
        with_context(|context| {
            let parent = tempfile::tempdir().unwrap();
            let destination = parent.path().join("out");
            let moved = parent.path().join("moved");
            let options = Options::parse(None, Format::Tar, "extract").unwrap();
            let result = extract_with(
                context,
                vec![Entry {
                    path: "data".into(),
                    data: Some(b"data".to_vec()),
                }],
                destination.to_str().unwrap(),
                &options,
                Rc::new(FilesystemPolicy::default()),
                || {
                    fs::rename(&destination, &moved).unwrap();
                    fs::create_dir(&destination).unwrap();
                    fs::write(destination.join("replacement"), b"safe").unwrap();
                    Err(NativeError::new(
                        "Interrupted",
                        "injected cancellation after replacement",
                    ))
                },
            );
            let error = result.unwrap_err();
            assert_eq!(error.kind, "Interrupted");
            assert!(error.message.contains("cleanup failed"));
            assert_eq!(fs::read(destination.join("replacement")).unwrap(), b"safe");
            assert!(moved.is_dir());
        });
    }
    #[test]
    fn cancellation_propagates_through_codecs_and_hard_count_bounds_are_exact() {
        with_context(|context| {
            let options = Options::parse(None, Format::Zip, "encode").unwrap();
            options.input(CEILING).unwrap();
            assert_eq!(
                options.input(CEILING + 1).unwrap_err().kind,
                "ResourceLimitError"
            );
            let mut logical = size_of::<Value>();
            let mut expanded = 0;
            row_budget(context, &mut logical, &mut expanded, "n", CEILING, &options).unwrap();
            assert_eq!(
                row_budget(context, &mut logical, &mut expanded, "m", 1, &options)
                    .unwrap_err()
                    .kind,
                "ResourceLimitError"
            );
            let entries = [Source {
                path: "n",
                data: Some(b"data"),
            }];
            let zip = zip_codec::encode(context, &entries, &options).unwrap();
            let tar = tar_codec::encode(context, &entries, &options).unwrap();
            let gzip = gzip::invoke(context, "compress", b"data", &options).unwrap();
            context.cancellation = Some(Arc::new(AtomicBool::new(true)));
            assert_eq!(
                zip_codec::encode(context, &entries, &options)
                    .unwrap_err()
                    .kind,
                "Interrupted"
            );
            assert_eq!(
                tar_codec::encode(context, &entries, &options)
                    .unwrap_err()
                    .kind,
                "Interrupted"
            );
            assert_eq!(
                zip_codec::decode(context, &zip, &options)
                    .err()
                    .unwrap()
                    .kind,
                "Interrupted"
            );
            assert_eq!(
                tar_codec::decode(context, &tar, &options)
                    .err()
                    .unwrap()
                    .kind,
                "Interrupted"
            );
            assert_eq!(
                gzip::invoke(context, "compress", b"data", &options)
                    .unwrap_err()
                    .kind,
                "Interrupted"
            );
            let Value::Bytes(gzip) = gzip else {
                unreachable!()
            };
            assert_eq!(
                gzip::invoke(context, "decompress", &gzip, &options)
                    .unwrap_err()
                    .kind,
                "Interrupted"
            );
        });
    }
}
