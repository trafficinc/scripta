//! Descriptor-based UTF-8 line reads with bounded working buffers.
use super::{FilesystemPolicy, failure, open_read, paths};
use scr_runtime::{
    NativeError, NativeFunction, NativeObject, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};
use std::{
    cell::RefCell,
    fs::File,
    io::{BufRead, BufReader},
    mem::size_of,
    path::PathBuf,
    rc::Rc,
};

const DEFAULT_MAX_LINE_BYTES: usize = 1024 * 1024;
const BUFFER_BYTES: usize = 8 * 1024;

#[derive(Debug)]
struct FileStream {
    reader: RefCell<Option<BufReader<File>>>,
    path: PathBuf,
    max_line_bytes: usize,
    owner: Rc<()>,
}
impl NativeObject for FileStream {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn type_name(&self) -> &str {
        "FileStream"
    }
    fn get_property(&self, name: &str) -> Result<Value, String> {
        match name {
            "closed" => Ok(Value::Bool(self.reader.borrow().is_none())),
            _ => Err(format!("unknown FileStream property {name}")),
        }
    }
}
fn stream(value: &Value) -> NativeResult<&FileStream> {
    let Value::NativeObject(object) = value else {
        return Err(NativeError::type_error("operation requires a FileStream"));
    };
    object
        .as_any()
        .and_then(|o| o.downcast_ref::<FileStream>())
        .ok_or_else(|| NativeError::type_error("operation requires a real FileStream"))
}
fn resource(message: &str) -> NativeError {
    NativeError::new("ResourceLimitError", message)
}

fn read_line(
    context: &RuntimeContext<'_>,
    reader: &mut impl BufRead,
    max_line_bytes: usize,
    path: &std::path::Path,
) -> NativeResult {
    let mut bytes = Vec::new();
    // Permit two terminator bytes beyond the output payload allowance, then
    // check the exact logical result after stripping LF/CRLF.
    let payload_limit = context
        .max_value_bytes
        .map(|limit| {
            limit
                .checked_sub(size_of::<Value>())
                .ok_or_else(|| resource("line result exceeds execution value byte limit"))
        })
        .transpose()?;
    loop {
        context.check_cancelled()?;
        let available = reader
            .fill_buf()
            .map_err(|e| failure("read line", path, e))?;
        if available.is_empty() {
            if bytes.is_empty() {
                return Ok(Value::Null);
            }
            break;
        }
        let newline = available.iter().position(|&b| b == b'\n');
        let count = newline.map_or(available.len(), |i| i + 1);
        let length = bytes
            .len()
            .checked_add(count)
            .ok_or_else(|| resource("line result size overflow"))?;
        if length > max_line_bytes {
            return Err(resource("line exceeds maxLineBytes"));
        }
        if payload_limit.is_some_and(|limit| length > limit.saturating_add(2)) {
            return Err(resource("line result exceeds execution value byte limit"));
        }
        bytes
            .try_reserve_exact(count)
            .map_err(|_| resource("line result allocation failed"))?;
        bytes.extend_from_slice(&available[..count]);
        reader.consume(count);
        if newline.is_some() {
            break;
        }
    }
    context.check_cancelled()?;
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    if payload_limit.is_some_and(|limit| bytes.len() > limit) {
        return Err(resource("line result exceeds execution value byte limit"));
    }
    String::from_utf8(bytes)
        .map(Value::String)
        .map_err(|_| NativeError::new("EncodingError", "line is not valid UTF-8"))
}

fn next_line<R: BufRead>(
    context: &RuntimeContext<'_>,
    state: &mut Option<R>,
    max_line_bytes: usize,
    path: &std::path::Path,
) -> NativeResult {
    let reader = state
        .as_mut()
        .ok_or_else(|| NativeError::new("StreamError", "FileStream is closed"))?;
    let result = read_line(context, reader, max_line_bytes, path);
    if result.is_err() {
        state.take();
    }
    result
}

pub(super) fn register(runtime: &mut Runtime, policy: Rc<FilesystemPolicy>) -> NativeResult<()> {
    let owner = Rc::new(());
    let open_owner = owner.clone();
    let read_policy = policy.clone();
    runtime.register(
        "File::openLines",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::String], NativeType::Object("FileStream"))
                .with_optional(vec![NativeType::Int]),
            move |context, args| {
                let max_line_bytes = match args.get(1) {
                    Some(Value::Int(value)) => usize::try_from(*value)
                        .ok()
                        .filter(|&n| n > 0)
                        .ok_or_else(|| {
                            NativeError::new("RangeError", "maxLineBytes must be a positive int")
                        })?,
                    None => DEFAULT_MAX_LINE_BYTES,
                    _ => unreachable!("validated int parameter"),
                };
                let path = paths::resolve(context, paths::string(&args[0])?)?;
                policy.check(context, &path, false)?;
                let reader = BufReader::with_capacity(BUFFER_BYTES, open_read(&path)?);
                Ok(Value::NativeObject(Rc::new(FileStream {
                    reader: RefCell::new(Some(reader)),
                    path,
                    max_line_bytes,
                    owner: open_owner.clone(),
                })))
            },
        ),
    )?;
    runtime.register(
        "File::readLine",
        NativeFunction::new(
            NativeSignature::new(
                vec![NativeType::Object("FileStream")],
                NativeType::OneOf(vec![NativeType::String, NativeType::Null]),
            ),
            move |context, args| {
                let stream = stream(&args[0])?;
                if !Rc::ptr_eq(&owner, &stream.owner) {
                    return Err(NativeError::type_error(
                        "FileStream belongs to another registration",
                    ));
                }
                context.permissions.require("filesystem.read")?;
                if !read_policy.read {
                    return Err(NativeError::new(
                        "PermissionError",
                        "filesystem policy denies this operation",
                    ));
                }
                next_line(
                    context,
                    &mut *stream.reader.borrow_mut(),
                    stream.max_line_bytes,
                    &stream.path,
                )
            },
        ),
    )?;
    runtime.register(
        "File::close",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::Object("FileStream")], NativeType::Null),
            |_, args| {
                stream(&args[0])?.reader.borrow_mut().take();
                Ok(Value::Null)
            },
        ),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scr_runtime::ContextOptions;
    use std::{
        io::{self, Cursor, Read},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    fn with_context<T>(
        limit: Option<usize>,
        flag: Option<Arc<AtomicBool>>,
        test: impl FnOnce(&mut RuntimeContext<'_>) -> T,
    ) -> T {
        let mut options = ContextOptions::default();
        let mut input = Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();
        let mut error = Vec::new();
        test(&mut RuntimeContext {
            working_directory: &mut options.working_directory,
            script_path: None,
            cli_arguments: &[],
            environment: &mut options.environment,
            input: &mut input,
            output: &mut output,
            error: &mut error,
            cancellation: flag,
            permissions: &options.permissions,
            call_span: Default::default(),
            max_value_bytes: limit,
        })
    }

    #[test]
    fn exact_payload_limits_account_for_lf_crlf_and_bare_cr() {
        let slot = size_of::<Value>();
        for (input, payload) in [
            ("é🦀\r\n", "é🦀"),
            ("é🦀\n", "é🦀"),
            ("é🦀", "é🦀"),
            ("é🦀\r", "é🦀\r"),
            ("\n", ""),
            ("\r\n", ""),
        ] {
            let required = slot + payload.len();
            let expected = Value::String(payload.into());
            // Two-byte buffers split multibyte UTF-8 and CRLF boundaries.
            with_context(Some(required), None, |context| {
                let mut reader = Some(BufReader::with_capacity(2, Cursor::new(input.as_bytes())));
                assert_eq!(
                    next_line(context, &mut reader, 32, PathBuf::from("test").as_path()).unwrap(),
                    expected
                );
                assert!(reader.is_some());
                assert_eq!(
                    next_line(context, &mut reader, 32, PathBuf::from("test").as_path()).unwrap(),
                    Value::Null
                );
            });
            with_context(Some(required - 1), None, |context| {
                let mut reader = Some(BufReader::with_capacity(2, Cursor::new(input.as_bytes())));
                assert_eq!(
                    next_line(context, &mut reader, 32, PathBuf::from("test").as_path())
                        .unwrap_err()
                        .kind,
                    "ResourceLimitError"
                );
                assert!(reader.is_none());
            });
        }
    }

    struct FailingReader;
    impl Read for FailingReader {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("injected read failure"))
        }
    }
    impl BufRead for FailingReader {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            Err(io::Error::other("injected read failure"))
        }
        fn consume(&mut self, _: usize) {}
    }
    struct CancellingReader {
        flag: Arc<AtomicBool>,
    }
    impl Read for CancellingReader {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            unreachable!()
        }
    }
    impl BufRead for CancellingReader {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            self.flag.store(true, Ordering::Relaxed);
            Ok(b"part")
        }
        fn consume(&mut self, _: usize) {}
    }

    #[test]
    fn read_errors_and_cancellation_close_the_reader_without_returning_partial_data() {
        with_context(None, None, |context| {
            let mut reader = Some(FailingReader);
            let error =
                next_line(context, &mut reader, 100, PathBuf::from("test").as_path()).unwrap_err();
            assert_eq!(error.kind, "FilesystemError");
            assert!(reader.is_none());
        });
        let flag = Arc::new(AtomicBool::new(false));
        with_context(None, Some(flag.clone()), |context| {
            let mut reader = Some(CancellingReader { flag });
            let error =
                next_line(context, &mut reader, 100, PathBuf::from("test").as_path()).unwrap_err();
            assert_eq!(error.kind, "Interrupted");
            assert!(reader.is_none());
        });
    }

    #[test]
    fn runtime_teardown_and_explicit_close_release_descriptors() {
        use fs2::FileExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.txt");
        std::fs::write(&path, "data\n").unwrap();
        for explicit in [false, true] {
            let mut runtime = Runtime::new();
            crate::register(&mut runtime).unwrap();
            runtime
                .set_global("path", Value::String(path.to_str().unwrap().into()))
                .unwrap();
            runtime
                .execute_source("<?scr $stream = File::openLines($path);", &mut Vec::new())
                .unwrap();
            {
                let handle = stream(runtime.get_global("stream").unwrap()).unwrap();
                handle
                    .reader
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .get_ref()
                    .lock_exclusive()
                    .unwrap();
            }
            let probe = File::open(&path).unwrap();
            assert!(probe.try_lock_exclusive().is_err());
            if explicit {
                runtime
                    .execute_source("<?scr File::close($stream);", &mut Vec::new())
                    .unwrap();
            } else {
                drop(runtime);
            }
            probe.try_lock_exclusive().unwrap();
        }
    }
}
