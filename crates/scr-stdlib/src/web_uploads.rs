//! Trusted host descriptors for private request-owned upload spools.
use crate::{FilesystemPolicy, bytes, filesystem, paths};
use scr_runtime::{
    NativeError, NativeFunction, NativeObject, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};
use std::{
    cell::Cell,
    fmt, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    rc::Rc,
};

/// Called only after destination policy checks. The supervisor must retain ownership
/// of the returned staging path until the request ends, including worker termination.
pub type Stage = Rc<dyn Fn(usize, &Path) -> NativeResult<PathBuf>>;
#[derive(Debug)]
pub struct File {
    pub field_name: String,
    pub filename: String,
    pub media_type: Option<String>,
    pub size: u64,
    pub path: PathBuf,
}
#[derive(Debug)]
pub struct Multipart {
    pub is_multipart: bool,
    pub body: PathBuf,
    pub body_size: u64,
    pub fields: Vec<(String, String)>,
    pub files: Vec<File>,
}
/// Drop before allowing the supervisor to remove request storage.
pub struct RequestResources {
    alive: Rc<Cell<bool>>,
    inputs: Rc<crate::streams::InputScope>,
}
impl Drop for RequestResources {
    fn drop(&mut self) {
        self.alive.set(false);
        self.inputs.close();
    }
}
pub(crate) struct State {
    data: Multipart,
    moved: Vec<Cell<bool>>,
    identity: Rc<()>,
    alive: Rc<Cell<bool>>,
    policy: FilesystemPolicy,
    stage: Stage,
    inputs: Rc<crate::streams::InputScope>,
}
impl fmt::Debug for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MultipartRequest").finish_non_exhaustive()
    }
}
fn error(message: &str) -> NativeError {
    NativeError::new("UploadError", message)
}
impl State {
    pub(crate) fn new(
        runtime: &Runtime,
        data: Multipart,
        policy: FilesystemPolicy,
        stage: Stage,
    ) -> NativeResult<(Rc<Self>, RequestResources)> {
        if data.body_size > i64::MAX as u64 || data.files.iter().any(|f| f.size > i64::MAX as u64) {
            return Err(error("upload size exceeds integer range"));
        }
        let alive = Rc::new(Cell::new(true));
        let inputs = crate::streams::InputScope::new();
        let moved = data.files.iter().map(|_| Cell::new(false)).collect();
        Ok((
            Rc::new(Self {
                data,
                moved,
                identity: runtime.native_identity(),
                alive: alive.clone(),
                policy: policy.prepare()?,
                stage,
                inputs: inputs.clone(),
            }),
            RequestResources { alive, inputs },
        ))
    }
    pub(crate) fn check(&self, context: &RuntimeContext<'_>) -> NativeResult<()> {
        context.check_cancelled()?;
        if !self.alive.get()
            || !context
                .runtime_identity
                .as_ref()
                .is_some_and(|id| Rc::ptr_eq(id, &self.identity))
        {
            return Err(error("upload belongs to another or expired request"));
        }
        Ok(())
    }
    pub(crate) fn payload_bytes(&self) -> usize {
        self.data
            .fields
            .iter()
            .fold(0usize, |n, (k, v)| {
                n.saturating_add(k.len() + v.len() + 2 * size_of::<Value>())
            })
            .saturating_add(
                self.data
                    .files
                    .iter()
                    .map(|f| {
                        f.field_name.len()
                            + f.filename.len()
                            + f.media_type.as_ref().map_or(0, String::len)
                            + size_of::<Value>()
                    })
                    .sum::<usize>(),
            )
    }
    pub(crate) fn is_multipart(&self) -> bool {
        self.data.is_multipart
    }
    pub(crate) fn body_size(&self) -> u64 {
        self.data.body_size
    }
    pub(crate) fn body_stream(&self, context: &RuntimeContext<'_>) -> NativeResult<Value> {
        self.check(context)?;
        let file = filesystem::open_read(&self.data.body)?;
        if file.metadata()?.len() != self.data.body_size {
            return Err(error("request spool size changed"));
        }
        crate::streams::request_input(
            context,
            Box::new(file),
            self.data.body_size,
            self.inputs.clone(),
            0,
        )
    }
    pub(crate) fn body(&self, context: &RuntimeContext<'_>) -> NativeResult<Vec<u8>> {
        self.check(context)?;
        let size = usize::try_from(self.data.body_size)
            .map_err(|_| bytes::resource("request body size exceeds address range"))?;
        bytes::payload(context, size)?;
        let mut result = Vec::new();
        result
            .try_reserve_exact(size)
            .map_err(|_| bytes::resource("cannot allocate request body"))?;
        copy(context, &self.data.body, self.data.body_size, &mut result)?;
        Ok(result)
    }
    pub(crate) fn fields(&self, context: &RuntimeContext<'_>) -> NativeResult<Value> {
        self.check(context)?;
        let mut cost = size_of::<Value>();
        for (k, v) in &self.data.fields {
            bytes::grow(
                context,
                &mut cost,
                3 * size_of::<Value>() + k.len() + v.len(),
            )?;
        }
        Ok(Value::List(
            self.data
                .fields
                .iter()
                .map(|(k, v)| Value::list(vec![Value::String(k.clone()), Value::String(v.clone())]))
                .collect(),
        ))
    }
    pub(crate) fn files(self: &Rc<Self>, context: &RuntimeContext<'_>) -> NativeResult<Value> {
        self.check(context)?;
        let mut cost = size_of::<Value>();
        for file in &self.data.files {
            bytes::grow(
                context,
                &mut cost,
                size_of::<Value>()
                    + file.field_name.len()
                    + file.filename.len()
                    + file.media_type.as_ref().map_or(0, String::len),
            )?;
        }
        Ok(Value::List(
            (0..self.data.files.len())
                .map(|index| {
                    Value::NativeObject(Rc::new(Upload {
                        state: self.clone(),
                        index,
                    }))
                })
                .collect(),
        ))
    }
    fn file_stream(&self, context: &RuntimeContext<'_>, index: usize) -> NativeResult<Value> {
        self.check(context)?;
        if self.moved[index].get() {
            return Err(error("upload has already been moved"));
        }
        let file = &self.data.files[index];
        let reader = filesystem::open_read(&file.path)?;
        if reader.metadata()?.len() != file.size {
            return Err(error("upload spool size changed"));
        }
        crate::streams::request_input(context, Box::new(reader), file.size, self.inputs.clone(), 0)
    }
    fn save(
        &self,
        context: &RuntimeContext<'_>,
        index: usize,
        destination: &str,
    ) -> NativeResult<Value> {
        self.check(context)?;
        bytes::budget(context, size_of::<Value>())?;
        if self.moved[index].get() {
            return Err(error("upload has already been moved"));
        }
        let destination = paths::resolve(context, destination)?;
        self.policy.check(context, &destination, true)?;
        absent(&destination)?;
        let parent = destination
            .parent()
            .ok_or_else(|| error("destination has no parent directory"))?;
        let parent = fs::canonicalize(parent)?;
        self.policy.check(context, &parent, true)?;
        let stage = (self.stage)(index, &parent)?;
        let stage = tempfile::TempPath::try_from_path(stage)?;
        let metadata = fs::symlink_metadata(&stage)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || stage.parent() != Some(parent.as_path())
        {
            return Err(error("invalid upload staging file"));
        }
        let mut output = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&stage)?;
        let file = &self.data.files[index];
        copy(context, &file.path, file.size, &mut output)?;
        output.flush()?;
        output.sync_all()?;
        drop(output);
        self.policy.check(context, &destination, true)?;
        absent(&destination)?;
        stage
            .persist_noclobber(&destination)
            .map_err(|_| error("cannot save upload without overwriting destination"))?;
        self.moved[index].set(true);
        Ok(Value::Null)
    }
}
fn absent(path: &Path) -> NativeResult<()> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(NativeError::new(
            "AlreadyExistsError",
            "upload destination already exists",
        )),
        Err(e) => Err(e.into()),
    }
}
fn copy(
    context: &RuntimeContext<'_>,
    path: &Path,
    size: u64,
    output: &mut impl Write,
) -> NativeResult<()> {
    let mut file = filesystem::open_read(path)?;
    if file.metadata()?.len() != size {
        return Err(error("upload spool size changed"));
    }
    let mut remaining = size;
    let mut buffer = [0; 65536];
    while remaining != 0 {
        context.check_cancelled()?;
        let n = file.read(&mut buffer[..remaining.min(65536) as usize])?;
        if n == 0 {
            return Err(error("upload spool is incomplete"));
        }
        output.write_all(&buffer[..n])?;
        remaining -= n as u64;
    }
    context.check_cancelled()?;
    if file.read(&mut buffer[..1])? != 0 {
        return Err(error("upload spool size changed"));
    }
    Ok(())
}
#[derive(Debug)]
struct Upload {
    state: Rc<State>,
    index: usize,
}
impl NativeObject for Upload {
    fn type_name(&self) -> &str {
        "UploadedFile"
    }
    fn payload_bytes(&self) -> usize {
        let f = &self.state.data.files[self.index];
        f.field_name.len() + f.filename.len() + f.media_type.as_ref().map_or(0, String::len)
    }
    fn get_property(&self, name: &str) -> Result<Value, String> {
        if !self.state.alive.get() {
            return Err("upload request has expired".into());
        }
        let file = &self.state.data.files[self.index];
        Ok(match name {
            "fieldName" => Value::String(file.field_name.clone()),
            "filename" => Value::String(file.filename.clone()),
            "mediaType" => file.media_type.clone().map_or(Value::Null, Value::String),
            "size" => Value::Int(file.size as i64),
            "moved" => Value::Bool(self.state.moved[self.index].get()),
            "stream" => {
                let state = self.state.clone();
                let index = self.index;
                Value::native_function(
                    "UploadedFile::stream",
                    NativeFunction::new(
                        NativeSignature::new(vec![], NativeType::Object("ByteStream")),
                        move |context, _| state.file_stream(context, index),
                    ),
                )
            }
            "moveTo" => {
                let state = self.state.clone();
                let index = self.index;
                Value::native_function(
                    "UploadedFile::moveTo",
                    NativeFunction::new(
                        NativeSignature::new(vec![NativeType::String], NativeType::Null),
                        move |context, args| {
                            let Value::String(destination) = &args[0] else {
                                unreachable!()
                            };
                            state.save(context, index, destination)
                        },
                    ),
                )
            }
            _ => return Err(format!("unknown UploadedFile property {name}")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scr_runtime::ContextOptions;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    #[test]
    fn exact_results_cancellation_and_failed_staging_leave_no_destination() {
        let directory = tempfile::tempdir().unwrap();
        let raw = directory.path().join("raw");
        std::fs::write(&raw, b"12345").unwrap();
        let runtime = Runtime::new();
        let identity = runtime.native_identity();
        let flag = Arc::new(AtomicBool::new(false));
        let interrupt = flag.clone();
        let stages = Rc::new(std::cell::RefCell::new(vec![]));
        let owned = stages.clone();
        let stage: Stage = Rc::new(move |_, parent| {
            let path = tempfile::Builder::new()
                .prefix(".scripta-upload-")
                .tempfile_in(parent)?
                .into_temp_path();
            let result = path.to_path_buf();
            owned.borrow_mut().push(path);
            interrupt.store(true, Ordering::Relaxed);
            Ok(result)
        });
        let (state, _guard) = State::new(
            &runtime,
            Multipart {
                is_multipart: true,
                body: raw.clone(),
                body_size: 5,
                fields: vec![("a".into(), "é".into())],
                files: vec![File {
                    field_name: "f".into(),
                    filename: "x".into(),
                    media_type: None,
                    size: 5,
                    path: raw,
                }],
            },
            FilesystemPolicy::default(),
            stage,
        )
        .unwrap();
        let mut options = ContextOptions::default();
        let mut input = std::io::empty();
        let mut out = vec![];
        let mut err = vec![];
        let mut context = RuntimeContext {
            working_directory: &mut options.working_directory,
            script_path: None,
            cli_arguments: &[],
            environment: &mut options.environment,
            input: &mut input,
            output: &mut out,
            error: &mut err,
            cancellation: Some(flag.clone()),
            runtime_identity: Some(identity),
            permissions: &options.permissions,
            call_span: Default::default(),
            max_value_bytes: None,
        };
        for (kind, required) in [
            ("body", size_of::<Value>() + 5),
            ("fields", 4 * size_of::<Value>() + 3),
            ("files", 2 * size_of::<Value>() + 2),
        ] {
            for (limit, ok) in [(required, true), (required - 1, false)] {
                context.max_value_bytes = Some(limit);
                let result = match kind {
                    "body" => state.body(&context).map(|_| ()),
                    "fields" => state.fields(&context).map(|_| ()),
                    _ => state.files(&context).map(|_| ()),
                };
                assert_eq!(result.is_ok(), ok, "{kind} {limit}");
                if let Err(e) = result {
                    assert_eq!(e.kind, "ResourceLimitError")
                }
            }
        }
        context.max_value_bytes = None;
        let destination = directory.path().join("saved");
        let failure = state
            .save(&context, 0, destination.to_str().unwrap())
            .unwrap_err();
        assert_eq!(failure.kind, "Interrupted");
        assert!(!destination.exists());
        assert!(!state.moved[0].get());
        assert!(stages.borrow().iter().all(|p| !p.exists()));
        assert_eq!(state.body(&context).unwrap_err().kind, "Interrupted");
    }
}
