//! Direct argv execution with bounded temporary-file capture and owned children.
use crate::{environment, paths};
use process_wrap::std::{StdChildWrapper, StdCommandWrap, StdCommandWrapper};
use scr_runtime::{
    MapKey, NativeError, NativeFunction, NativeObject, NativeResult, NativeSignature, NativeType,
    Runtime, RuntimeContext, Value,
};
use std::{
    any::Any,
    cell::RefCell,
    collections::BTreeMap,
    ffi::OsString,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::PathBuf,
    process::{Command, ExitStatus, Stdio},
    rc::Rc,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Debug, Clone)]
pub struct ProcessConfig {
    pub default_timeout: Option<Duration>,
    /// A host ceiling: per-call options may lower, never raise this limit.
    pub max_output_bytes: usize,
}
impl Default for ProcessConfig {
    fn default() -> Self {
        Self {
            default_timeout: Some(Duration::from_secs(60)),
            max_output_bytes: 8 * 1024 * 1024,
        }
    }
}
#[derive(Clone, Copy)]
enum Stream {
    Capture,
    Inherit,
    Null,
}
struct Options {
    cwd: PathBuf,
    environment: BTreeMap<OsString, OsString>,
    timeout: Option<Duration>,
    limit: usize,
    stdout: Stream,
    stderr: Stream,
    stdin_inherit: bool,
}
fn type_error(message: &str) -> NativeError {
    NativeError::type_error(message)
}
fn os_error(error: io::Error) -> NativeError {
    let kind = match error.kind() {
        io::ErrorKind::NotFound => "CommandNotFoundError",
        io::ErrorKind::PermissionDenied => "PermissionError",
        _ => "ProcessError",
    };
    NativeError::new(kind, error.to_string())
}
fn number(value: &Value, key: &str) -> NativeResult<u64> {
    if let Value::Int(value) = value {
        return u64::try_from(*value)
            .map_err(|_| type_error(&format!("{key} must be a nonnegative int")));
    }
    Err(type_error(&format!("{key} must be a nonnegative int")))
}
fn stream(value: &Value) -> NativeResult<Stream> {
    match value {
        Value::String(value) if value == "capture" => Ok(Stream::Capture),
        Value::String(value) if value == "inherit" => Ok(Stream::Inherit),
        Value::String(value) if value == "null" => Ok(Stream::Null),
        _ => Err(type_error("stream mode must be capture, inherit, or null")),
    }
}
fn options(
    context: &RuntimeContext<'_>,
    value: Option<&Value>,
    config: &ProcessConfig,
) -> NativeResult<Options> {
    let mut result = Options {
        cwd: paths::resolve(context, ".")?,
        environment: context.environment.clone(),
        timeout: config.default_timeout,
        limit: config.max_output_bytes,
        stdout: Stream::Capture,
        stderr: Stream::Capture,
        stdin_inherit: false,
    };
    if let Some(value) = value {
        let Value::Map(entries) = value else {
            return Err(type_error("process options must be a map"));
        };
        for (key, value) in entries {
            let MapKey::String(key) = key else {
                return Err(type_error("process option keys must be strings"));
            };
            match key.as_str() {
                "cwd" => result.cwd = paths::resolve(context, paths::string(value)?)?,
                "timeoutMs" => {
                    result.timeout = if *value == Value::Null {
                        None
                    } else {
                        Some(Duration::from_millis(number(value, key)?))
                    }
                }
                "maxOutputBytes" => {
                    result.limit = usize::try_from(number(value, key)?)
                        .map_err(|_| type_error("output limit is too large"))?;
                    if result.limit > config.max_output_bytes {
                        return Err(type_error("output limit exceeds the host ceiling"));
                    }
                }
                "stdout" => result.stdout = stream(value)?,
                "stderr" => result.stderr = stream(value)?,
                "stdin" => match value {
                    Value::String(value) if value == "inherit" => result.stdin_inherit = true,
                    Value::String(value) if value == "null" => result.stdin_inherit = false,
                    _ => return Err(type_error("stdin mode must be inherit or null")),
                },
                "env" => {
                    let entries = match value {
                        Value::Map(entries) => entries.as_slice(),
                        Value::List(entries) if entries.is_empty() => &[],
                        _ => return Err(type_error("env overrides must be a map")),
                    };
                    for (key, value) in entries {
                        let MapKey::String(name) = key else {
                            return Err(type_error("environment override names must be strings"));
                        };
                        environment::name(name)?;
                        let key = environment::key(&result.environment, name);
                        match value {
                            Value::Null => {
                                result.environment.remove(&key);
                            }
                            Value::String(value) => {
                                environment::value(value)?;
                                result.environment.insert(key, value.into());
                            }
                            _ => {
                                return Err(type_error(
                                    "environment overrides must be string or null",
                                ));
                            }
                        }
                    }
                }
                _ => return Err(type_error(&format!("unknown process option {key}"))),
            }
        }
    }
    // Script writes/overrides are validated above. Preserve the native host
    // snapshot unchanged, including Windows' reserved =drive: entries and
    // non-UTF-8 OS strings; Command reports invalid host data on spawn.
    Ok(result)
}
fn capture(mode: Stream) -> NativeResult<(Stdio, Option<File>)> {
    match mode {
        Stream::Capture => {
            let file = tempfile::tempfile().map_err(os_error)?;
            Ok((Stdio::from(file.try_clone().map_err(os_error)?), Some(file)))
        }
        Stream::Inherit => Ok((Stdio::inherit(), None)),
        Stream::Null => Ok((Stdio::null(), None)),
    }
}
fn captured_size(file: &Option<File>) -> NativeResult<u64> {
    file.as_ref().map_or(Ok(0), |file| {
        file.metadata().map(|info| info.len()).map_err(os_error)
    })
}
fn check_size(stdout: &Option<File>, stderr: &Option<File>, limit: usize) -> NativeResult<()> {
    if captured_size(stdout)?.saturating_add(captured_size(stderr)?) > limit as u64 {
        return Err(NativeError::new(
            "OutputLimitError",
            "combined captured output exceeds configured limit",
        ));
    }
    Ok(())
}
fn read_capture(file: &mut Option<File>, remaining: &mut usize) -> NativeResult<String> {
    let Some(file) = file else {
        return Ok(String::new());
    };
    file.seek(SeekFrom::Start(0)).map_err(os_error)?;
    let mut bytes = Vec::new();
    file.take((*remaining as u64).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(os_error)?;
    if bytes.len() > *remaining {
        return Err(NativeError::new(
            "OutputLimitError",
            "captured output exceeds configured limit",
        ));
    }
    *remaining -= bytes.len();
    String::from_utf8(bytes).map_err(|_| {
        NativeError::new(
            "EncodingError",
            "captured process output is not valid UTF-8",
        )
    })
}
#[derive(Debug)]
struct Outcome {
    exit_code: Option<i64>,
    stdout: String,
    stderr: String,
    success: bool,
    duration_ms: i64,
}
#[derive(Debug)]
struct ResultObject(Arc<Outcome>);
impl NativeObject for ResultObject {
    fn type_name(&self) -> &str {
        "ProcessResult"
    }
    fn get_property(&self, name: &str) -> Result<Value, String> {
        match name {
            "exitCode" => Ok(self.0.exit_code.map_or(Value::Null, Value::Int)),
            "stdout" => Ok(Value::String(self.0.stdout.clone())),
            "stderr" => Ok(Value::String(self.0.stderr.clone())),
            "success" => Ok(Value::Bool(self.0.success)),
            "durationMs" => Ok(Value::Int(self.0.duration_ms)),
            _ => Err(format!("unknown ProcessResult property {name}")),
        }
    }
}
fn result_value(outcome: Arc<Outcome>) -> Value {
    Value::NativeObject(Rc::new(ResultObject(outcome)))
}
// Applied before platform wrappers: failed Windows job assignment/resumption
// must drop an owned child that is killed and reaped, including suspended ones.
#[derive(Debug)]
struct SetupCleanup;
#[derive(Debug)]
struct SetupChild(Option<Box<dyn StdChildWrapper>>);
impl StdCommandWrapper for SetupCleanup {
    fn wrap_child(
        &mut self,
        child: Box<dyn StdChildWrapper>,
        _: &StdCommandWrap,
    ) -> io::Result<Box<dyn StdChildWrapper>> {
        Ok(Box::new(SetupChild(Some(child))))
    }
}
impl StdChildWrapper for SetupChild {
    fn inner(&self) -> &std::process::Child {
        self.0.as_ref().expect("owned setup child").inner()
    }
    fn inner_mut(&mut self) -> &mut std::process::Child {
        self.0.as_mut().expect("owned setup child").inner_mut()
    }
    fn into_inner(mut self: Box<Self>) -> std::process::Child {
        self.0.take().expect("owned setup child").into_inner()
    }
}
impl Drop for SetupChild {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            if child.inner_mut().try_wait().ok().flatten().is_none() {
                let _ = child.inner_mut().kill();
                let _ = child.inner_mut().wait();
            }
        }
    }
}

struct ChildGuard(Option<Box<dyn StdChildWrapper>>);
impl ChildGuard {
    fn child(&mut self) -> NativeResult<&mut Box<dyn StdChildWrapper>> {
        self.0
            .as_mut()
            .ok_or_else(|| NativeError::new("ProcessError", "child already reaped"))
    }
    fn cleanup(&mut self) -> NativeResult<ExitStatus> {
        let child = self.child()?;
        let killed = child.start_kill();
        let status = child.inner_mut().try_wait().map_err(os_error)?;
        if let Err(error) = killed {
            // No process group remains is a normal race with successful exit.
            if error.kind() != io::ErrorKind::NotFound
                && error.kind() != io::ErrorKind::InvalidInput
                && error.raw_os_error() != Some(3)
            {
                return Err(os_error(error));
            }
        }
        let status = match status {
            Some(status) => status,
            None => {
                if let Err(error) = child.inner_mut().kill() {
                    if child.inner_mut().try_wait().map_err(os_error)?.is_none() {
                        return Err(os_error(error));
                    }
                }
                child.inner_mut().wait().map_err(os_error)?
            }
        };
        self.0.take();
        Ok(status)
    }
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.start_kill();
            if child.inner_mut().try_wait().ok().flatten().is_none() {
                let _ = child.inner_mut().kill();
                let _ = child.inner_mut().wait();
            }
        }
    }
}
#[derive(Default)]
struct Shared {
    result: Mutex<Option<NativeResult<Arc<Outcome>>>>,
    completed: Condvar,
    kill: AtomicBool,
}
struct ProcessHandle {
    pid: u32,
    shared: Arc<Shared>,
    worker: RefCell<Option<JoinHandle<()>>>,
}
impl std::fmt::Debug for ProcessHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessHandle")
            .field("pid", &self.pid)
            .finish()
    }
}
impl ProcessHandle {
    fn snapshot(&self) -> NativeResult<Option<NativeResult<Arc<Outcome>>>> {
        self.shared
            .result
            .lock()
            .map(|result| result.clone())
            .map_err(|_| NativeError::new("ProcessError", "process monitor lock poisoned"))
    }
    fn wait(&self, context: &RuntimeContext<'_>) -> NativeResult<Value> {
        let mut slot = self
            .shared
            .result
            .lock()
            .map_err(|_| NativeError::new("ProcessError", "process monitor lock poisoned"))?;
        loop {
            if let Err(error) = context.check_cancelled() {
                drop(slot);
                self.shared.kill.store(true, Ordering::Relaxed);
                self.join()?;
                return Err(error);
            }
            if let Some(result) = slot.clone() {
                drop(slot);
                self.join()?;
                return result.map(result_value);
            }
            // Notifications remove a second poll-delay floor after monitor completion.
            // The timeout preserves cooperative cancellation even without a notification.
            let (next, _) = self
                .shared
                .completed
                .wait_timeout(slot, Duration::from_millis(5))
                .map_err(|_| NativeError::new("ProcessError", "process monitor lock poisoned"))?;
            slot = next;
        }
    }
    fn join(&self) -> NativeResult<()> {
        if let Some(worker) = self.worker.borrow_mut().take() {
            worker
                .join()
                .map_err(|_| NativeError::new("ProcessError", "process monitor panicked"))?;
        }
        Ok(())
    }
}
impl Drop for ProcessHandle {
    fn drop(&mut self) {
        self.shared.kill.store(true, Ordering::Relaxed);
        let _ = self.join();
    }
}
impl NativeObject for ProcessHandle {
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn type_name(&self) -> &str {
        "ProcessHandle"
    }
    fn get_property(&self, name: &str) -> Result<Value, String> {
        match name {
            "pid" => Ok(Value::Int(i64::from(self.pid))),
            "running" => self
                .snapshot()
                .map(|result| Value::Bool(result.is_none()))
                .map_err(|error| error.to_string()),
            _ => Err(format!("unknown ProcessHandle property {name}")),
        }
    }
}
fn monitor(
    mut child: ChildGuard,
    mut stdout: Option<File>,
    mut stderr: Option<File>,
    options: Options,
    shared: &Shared,
    cancellation: Option<&AtomicBool>,
    start: Instant,
) -> NativeResult<Arc<Outcome>> {
    let result = (|| {
        loop {
            if cancellation.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
                return Err(NativeError::new("Interrupted", "execution interrupted"));
            }
            check_size(&stdout, &stderr, options.limit)?;
            if shared.kill.load(Ordering::Relaxed) {
                break;
            }
            if options
                .timeout
                .is_some_and(|timeout| start.elapsed() >= timeout)
            {
                return Err(NativeError::new(
                    "TimeoutError",
                    "process exceeded configured timeout",
                ));
            }
            if child
                .child()?
                .inner_mut()
                .try_wait()
                .map_err(os_error)?
                .is_some()
            {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    })();
    let cleanup = child.cleanup();
    if let Err(error) = result {
        if let Err(cleanup) = cleanup {
            return Err(NativeError::new(
                error.kind,
                format!("{}; cleanup failed: {cleanup}", error.message),
            ));
        }
        return Err(error);
    }
    let status = cleanup?;
    check_size(&stdout, &stderr, options.limit)?;
    let mut remaining = options.limit;
    Ok(Arc::new(Outcome {
        exit_code: status.code().map(i64::from),
        success: status.success(),
        stdout: read_capture(&mut stdout, &mut remaining)?,
        stderr: read_capture(&mut stderr, &mut remaining)?,
        duration_ms: i64::try_from(start.elapsed().as_millis()).unwrap_or(i64::MAX),
    }))
}
fn launch(
    context: &mut RuntimeContext<'_>,
    command: &str,
    args: &[String],
    options: Options,
    shell: bool,
) -> NativeResult<ProcessHandle> {
    if command.is_empty() || command.contains('\0') || args.iter().any(|value| value.contains('\0'))
    {
        return Err(type_error(
            "command must be nonempty and command/arguments cannot contain NUL",
        ));
    }
    #[cfg(windows)]
    if PathBuf::from(command)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("bat") || ext.eq_ignore_ascii_case("cmd"))
    {
        return Err(NativeError::new(
            "ProcessError",
            "batch files require explicit Process::shell",
        ));
    }
    let directory = std::fs::metadata(&options.cwd).map_err(|error| {
        NativeError::new(
            "ProcessError",
            format!("working directory '{}': {error}", options.cwd.display()),
        )
    })?;
    if !directory.is_dir() {
        return Err(NativeError::new(
            "ProcessError",
            "working directory is not a directory",
        ));
    }
    let path = paths::checked_path(command)?;
    let executable = if path.is_absolute() || path.components().count() > 1 {
        options.cwd.join(path)
    } else {
        PathBuf::from(command)
    };
    if matches!(options.stdout, Stream::Inherit) {
        context.output.flush()?;
    }
    if matches!(options.stderr, Stream::Inherit) {
        context.error.flush()?;
    }
    let mut builder = Command::new(executable);
    #[cfg(windows)]
    if shell {
        use std::os::windows::process::CommandExt;
        // cmd.exe parses a command line, not MSVCRT argv escaping. This raw
        // string is used only by the explicitly permission-gated shell API.
        let code = args
            .last()
            .ok_or_else(|| type_error("shell code is required"))?;
        builder.args(["/D", "/S", "/C"]).raw_arg(code);
    } else {
        builder.args(args);
    }
    #[cfg(not(windows))]
    {
        let _ = shell;
        builder.args(args);
    }
    builder
        .current_dir(&options.cwd)
        .env_clear()
        .envs(&options.environment)
        .stdin(if options.stdin_inherit {
            Stdio::inherit()
        } else {
            Stdio::null()
        });
    let (out, stdout) = capture(options.stdout)?;
    let (err, stderr) = capture(options.stderr)?;
    builder.stdout(out).stderr(err);
    context.check_cancelled()?;
    let start = Instant::now();
    let mut wrapped = StdCommandWrap::from(builder);
    wrapped.wrap(SetupCleanup);
    #[cfg(unix)]
    wrapped.wrap(process_wrap::std::ProcessGroup::leader());
    #[cfg(windows)]
    wrapped.wrap(process_wrap::std::JobObject);
    let child = wrapped.spawn().map_err(|error| {
        let mut error = os_error(error);
        error.message = format!("start '{command}': {}", error.message);
        error
    })?;
    let pid = child.id();
    let guard = ChildGuard(Some(child));
    let shared = Arc::new(Shared::default());
    let monitor_state = shared.clone();
    let cancellation = context.cancellation.clone();
    let worker = thread::Builder::new()
        .name(format!("scr-process-{pid}"))
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                monitor(
                    guard,
                    stdout,
                    stderr,
                    options,
                    &monitor_state,
                    cancellation.as_deref(),
                    start,
                )
            }))
            .unwrap_or_else(|_| Err(NativeError::new("ProcessError", "process monitor panicked")));
            if let Ok(mut slot) = monitor_state.result.lock() {
                *slot = Some(result);
                monitor_state.completed.notify_all();
            }
        })
        .map_err(os_error)?;
    Ok(ProcessHandle {
        pid,
        shared,
        worker: RefCell::new(Some(worker)),
    })
}
fn handle(args: &[Value]) -> NativeResult<&ProcessHandle> {
    let [Value::NativeObject(object)] = args else {
        return Err(type_error("operation requires a ProcessHandle"));
    };
    object
        .as_any()
        .and_then(|object| object.downcast_ref::<ProcessHandle>())
        .ok_or_else(|| type_error("operation requires a real ProcessHandle"))
}
/// Register process APIs. Shell is separately permission-gated by process.shell.
pub fn register(runtime: &mut Runtime, config: ProcessConfig) -> NativeResult<()> {
    for method in ["run", "spawn", "shell"] {
        let config = config.clone();
        let optional = if method == "shell" {
            vec![NativeType::Map]
        } else {
            vec![NativeType::List, NativeType::Map]
        };
        runtime.register(
            format!("Process::{method}"),
            NativeFunction::new(
                NativeSignature::new(
                    vec![NativeType::String],
                    if method == "spawn" {
                        NativeType::Object("ProcessHandle")
                    } else {
                        NativeType::Object("ProcessResult")
                    },
                )
                .with_optional(optional),
                move |context, values| {
                    context.permissions.require("process.execute")?;
                    let Some(Value::String(command)) = values.first() else {
                        return Err(type_error("command must be a string"));
                    };
                    let (command, arguments, options_value) = if method == "shell" {
                        context.permissions.require("process.shell")?;
                        #[cfg(unix)]
                        let (shell, arguments) =
                            ("/bin/sh".to_owned(), vec!["-c".to_owned(), command.clone()]);
                        #[cfg(windows)]
                        let (shell, arguments) = (
                            "cmd.exe".to_owned(),
                            vec![
                                "/D".to_owned(),
                                "/S".to_owned(),
                                "/C".to_owned(),
                                command.clone(),
                            ],
                        );
                        (shell, arguments, values.get(1))
                    } else {
                        let arguments = match values.get(1) {
                            None => Vec::new(),
                            Some(Value::List(args)) => args
                                .iter()
                                .map(|arg| paths::string(arg).map(str::to_owned))
                                .collect::<NativeResult<Vec<_>>>()?,
                            _ => return Err(type_error("arguments must be a list of strings")),
                        };
                        (command.clone(), arguments, values.get(2))
                    };
                    let options = options(context, options_value, &config)?;
                    let child = launch(context, &command, &arguments, options, method == "shell")?;
                    if method == "spawn" {
                        Ok(Value::NativeObject(Rc::new(child)))
                    } else {
                        child.wait(context)
                    }
                },
            ),
        )?;
    }
    runtime.register(
        "Process::currentId",
        NativeFunction::new(NativeSignature::new(vec![], NativeType::Int), |_, _| {
            Ok(Value::Int(i64::from(std::process::id())))
        }),
    )?;
    runtime.register(
        "Process::exists",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::Int], NativeType::Bool),
            |context, args| {
                context.permissions.require("process.inspect")?;
                let [Value::Int(pid)] = args else {
                    return Err(type_error("pid must be an int"));
                };
                let pid = u32::try_from(*pid)
                    .ok()
                    .filter(|pid| *pid > 0)
                    .ok_or_else(|| type_error("pid must be a positive process identifier"))?;
                #[cfg(unix)]
                if pid > i32::MAX as u32 {
                    return Err(type_error("pid exceeds the native signed PID range"));
                }
                let mut system = sysinfo::System::new();
                let pid = sysinfo::Pid::from_u32(pid);
                system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
                Ok(Value::Bool(system.process(pid).is_some()))
            },
        ),
    )?;
    for method in ["wait", "poll", "kill"] {
        runtime.register(
            format!("Process::{method}"),
            NativeFunction::new(
                NativeSignature::new(
                    vec![NativeType::Object("ProcessHandle")],
                    NativeType::OneOf(vec![NativeType::Object("ProcessResult"), NativeType::Null]),
                ),
                move |context, args| {
                    context.permissions.require("process.execute")?;
                    let handle = handle(args)?;
                    match method {
                        "wait" => handle.wait(context),
                        "kill" => {
                            handle.shared.kill.store(true, Ordering::Relaxed);
                            handle.wait(context)
                        }
                        "poll" => handle
                            .snapshot()?
                            .map_or(Ok(Value::Null), |result| result.map(result_value)),
                        _ => Err(NativeError::new(
                            "ProcessError",
                            "unknown process handle operation",
                        )),
                    }
                },
            ),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod setup_tests {
    use super::*;
    #[derive(Debug)]
    struct FailSetup(Arc<std::sync::atomic::AtomicU32>);
    impl StdCommandWrapper for FailSetup {
        fn wrap_child(
            &mut self,
            child: Box<dyn StdChildWrapper>,
            _: &StdCommandWrap,
        ) -> io::Result<Box<dyn StdChildWrapper>> {
            self.0.store(child.id(), Ordering::Relaxed);
            Err(io::Error::other("injected setup failure"))
        }
    }
    #[test]
    fn failed_platform_setup_kills_and_reaps_the_new_child() {
        let _guard = crate::DESCRIPTOR_LIFECYCLE_TEST
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let pid = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let mut command = StdCommandWrap::with_new(std::env::current_exe().unwrap(), |command| {
            command
                .args(["--exact", "no-matching-test"])
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        });
        command.wrap(SetupCleanup).wrap(FailSetup(pid.clone()));
        assert!(command.spawn().is_err());
        let pid = sysinfo::Pid::from_u32(pid.load(Ordering::Relaxed));
        let mut system = sysinfo::System::new();
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        assert!(
            system.process(pid).is_none(),
            "failed setup left an unreaped child"
        );
    }
}
