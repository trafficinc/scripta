//! Live output spools and a bounded, independently serviced stdin queue.
use super::*;
use std::{collections::VecDeque, process::ChildStdin, sync::atomic::AtomicUsize};

const QUEUE_BYTES: usize = 64 * 1024;
const MAX_SESSIONS: usize = 64;
static SESSIONS: AtomicUsize = AtomicUsize::new(0);

pub(super) struct Permit;
impl Permit {
    pub(super) fn acquire() -> NativeResult<Self> {
        let mut count = SESSIONS.load(Ordering::Relaxed);
        loop {
            if count >= MAX_SESSIONS {
                return Err(bytes::resource(
                    "interactive process resource limit reached",
                ));
            }
            match SESSIONS.compare_exchange_weak(
                count,
                count + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(current) => count = current,
            }
        }
        Ok(Self)
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        SESSIONS.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(super) struct Output {
    // Keep the name owned until the process handle drops. The reopened reader
    // has its own cursor, so seeking it cannot redirect a child's file writes.
    file: tempfile::NamedTempFile,
    reader: File,
    offset: u64,
}
pub(super) fn capture(
    mode: Stream,
    interactive: bool,
) -> NativeResult<(Stdio, Option<File>, Option<Output>)> {
    if !interactive {
        let (stdio, file) = super::capture(mode)?;
        return Ok((stdio, file, None));
    }
    let file = tempfile::Builder::new()
        .prefix("scripta-process-")
        .append(true)
        .tempfile()
        .map_err(os_error)?;
    let reader = file.reopen().map_err(os_error)?;
    let writer = file.as_file().try_clone().map_err(os_error)?;
    let metadata = file.as_file().try_clone().map_err(os_error)?;
    Ok((
        Stdio::from(writer),
        Some(metadata),
        Some(Output {
            file,
            reader,
            offset: 0,
        }),
    ))
}

#[derive(Default)]
struct InputState {
    queue: VecDeque<u8>,
    pending: usize,
    accepted: usize,
    closed: bool,
    stopped: bool,
    error: Option<NativeError>,
}
pub(super) struct Input {
    state: Mutex<InputState>,
    ready: Condvar,
    limit: usize,
}
fn lock_error() -> NativeError {
    NativeError::new("ProcessError", "interactive stdin lock poisoned")
}
impl Input {
    fn lock(&self) -> NativeResult<std::sync::MutexGuard<'_, InputState>> {
        self.state.lock().map_err(|_| lock_error())
    }
    pub(super) fn is_closed(&self) -> NativeResult<bool> {
        self.lock().map(|state| state.closed || state.stopped)
    }
    fn stop(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.stopped = true;
            state.queue = VecDeque::new();
            state.pending = 0;
            self.ready.notify_all();
        }
    }
    fn write(&self, context: &RuntimeContext<'_>, data: &[u8]) -> NativeResult<Value> {
        context.check_cancelled()?;
        let mut state = self.lock()?;
        if let Some(error) = &state.error {
            return Err(error.clone());
        }
        if state.closed {
            return Err(NativeError::new(
                "ProcessError",
                "interactive stdin is closed",
            ));
        }
        if state.stopped {
            return Err(NativeError::new(
                "BrokenPipeError",
                "child stdin is no longer available",
            ));
        }
        if data.len() > self.limit.saturating_sub(state.accepted) {
            return Err(NativeError::new(
                "InputLimitError",
                "cumulative interactive input exceeds configured limit",
            ));
        }
        let count = data.len().min(QUEUE_BYTES.saturating_sub(state.pending));
        bytes::payload(context, state.pending + count)?;
        state
            .queue
            .try_reserve(count)
            .map_err(|_| bytes::resource("interactive input queue allocation failed"))?;
        state.queue.extend(&data[..count]);
        state.pending += count;
        state.accepted += count;
        self.ready.notify_one();
        Ok(Value::Int(count as i64))
    }
    fn close(&self, context: &RuntimeContext<'_>) -> NativeResult<Value> {
        context.check_cancelled()?;
        bytes::payload(context, 0)?;
        let mut state = self.lock()?;
        if let Some(error) = &state.error {
            return Err(error.clone());
        }
        state.closed = true;
        self.ready.notify_one();
        Ok(Value::Null)
    }
    fn deliver(&self, mut stdin: ChildStdin) {
        let mut chunk = [0; IO_CHUNK];
        loop {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            while state.queue.is_empty() && !state.closed && !state.stopped {
                let Ok(next) = self.ready.wait(state) else {
                    return;
                };
                state = next;
            }
            if state.stopped || state.queue.is_empty() {
                return;
            }
            let count = chunk.len().min(state.queue.len());
            for byte in &mut chunk[..count] {
                *byte = state.queue.pop_front().expect("queued byte");
            }
            drop(state);
            let result = stdin.write_all(&chunk[..count]);
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            state.pending = state.pending.saturating_sub(count);
            if let Err(error) = result {
                state.error = Some(NativeError::new(
                    if error.kind() == io::ErrorKind::BrokenPipe {
                        "BrokenPipeError"
                    } else {
                        "ProcessError"
                    },
                    "interactive stdin delivery failed",
                ));
                state.stopped = true;
                state.queue = VecDeque::new();
                state.pending = 0;
                return;
            }
        }
    }
}

pub(super) struct Writer {
    input: Arc<Input>,
    worker: Option<JoinHandle<()>>,
}
impl Writer {
    pub(super) fn spawn(
        stdin: ChildStdin,
        input: Arc<Input>,
        permit: Arc<Permit>,
    ) -> NativeResult<Self> {
        let state = input.clone();
        let worker = thread::Builder::new()
            .name("scr-process-stdin".into())
            .spawn(move || {
                let _permit = permit;
                state.deliver(stdin);
            })
            .map_err(os_error)?;
        Ok(Self {
            input,
            worker: Some(worker),
        })
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        self.input.stop();
        if let Some(worker) = self.worker.take() {
            // Managed readers have been killed/reaped by monitor before this
            // drop. Escaped readers must not cause an unbounded host join.
            let start = Instant::now();
            while !worker.is_finished() && start.elapsed() < Duration::from_millis(100) {
                thread::sleep(Duration::from_millis(1));
            }
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}

pub(super) struct Io {
    pub(super) input: Arc<Input>,
    stdout: RefCell<Output>,
    stderr: RefCell<Output>,
    _permit: Arc<Permit>,
    output_limit: usize,
}
impl Io {
    pub(super) fn new(
        stdout: Output,
        stderr: Output,
        limit: usize,
        output_limit: usize,
        permit: Arc<Permit>,
    ) -> Self {
        Self {
            input: Arc::new(Input {
                state: Mutex::new(InputState::default()),
                ready: Condvar::new(),
                limit,
            }),
            stdout: RefCell::new(stdout),
            stderr: RefCell::new(stderr),
            _permit: permit,
            output_limit,
        }
    }
    pub(super) fn pending_bytes(&self) -> usize {
        self.input
            .state
            .lock()
            .map(|state| state.pending)
            .unwrap_or(usize::MAX)
    }
    fn read(
        &self,
        context: &RuntimeContext<'_>,
        handle: &ProcessHandle,
        stderr: bool,
        maximum: usize,
        timeout: Duration,
    ) -> NativeResult<Value> {
        let start = Instant::now();
        loop {
            context.check_cancelled()?;
            let completed = handle.snapshot()?;
            if let Some(Err(error)) = &completed {
                return Err(error.clone());
            }
            let stdout_len = self
                .stdout
                .borrow()
                .file
                .as_file()
                .metadata()
                .map_err(os_error)?
                .len();
            let stderr_len = self
                .stderr
                .borrow()
                .file
                .as_file()
                .metadata()
                .map_err(os_error)?
                .len();
            if stdout_len.saturating_add(stderr_len) > self.output_limit as u64 {
                handle.shared.kill.store(true, Ordering::Relaxed);
                handle.join()?;
                if let Some(Err(error)) = handle.snapshot()? {
                    return Err(error);
                }
                return Err(NativeError::new(
                    "OutputLimitError",
                    "combined interactive output exceeds configured limit",
                ));
            }
            let mut output = if stderr {
                self.stderr.borrow_mut()
            } else {
                self.stdout.borrow_mut()
            };
            let length = if stderr { stderr_len } else { stdout_len };
            let available =
                usize::try_from(length.saturating_sub(output.offset)).unwrap_or(usize::MAX);
            if available != 0 {
                let count = maximum.min(available);
                bytes::payload(context, count)?;
                let mut data = Vec::new();
                data.try_reserve_exact(count)
                    .map_err(|_| bytes::resource("interactive output allocation failed"))?;
                data.resize(count, 0);
                let offset = output.offset;
                output
                    .reader
                    .seek(SeekFrom::Start(offset))
                    .map_err(os_error)?;
                let count = output.reader.read(&mut data).map_err(os_error)?;
                data.truncate(count);
                context.check_cancelled()?;
                output.offset += count as u64;
                return Ok(Value::Bytes(data.into()));
            }
            if completed.is_some() {
                bytes::payload(context, 0)?;
                return Ok(Value::Null);
            }
            if start.elapsed() >= timeout {
                bytes::payload(context, 0)?;
                return Ok(Value::Bytes(Vec::<u8>::new().into()));
            }
            drop(output);
            thread::sleep(Duration::from_millis(5).min(timeout.saturating_sub(start.elapsed())));
        }
    }
}
fn owned<'a>(
    context: &RuntimeContext<'_>,
    args: &'a [Value],
) -> NativeResult<(&'a ProcessHandle, &'a Io)> {
    let handle = super::handle(&args[..1])?;
    if !handle.owner_matches(context) {
        return Err(type_error(
            "interactive ProcessHandle belongs to another runtime",
        ));
    }
    let io = handle
        .io
        .as_ref()
        .ok_or_else(|| NativeError::new("ProcessError", "handle is not interactive"))?;
    Ok((handle, io))
}
pub(super) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    for method in ["readStdout", "readStderr"] {
        runtime.register(
            format!("Process::{method}"),
            NativeFunction::new(
                NativeSignature::new(
                    vec![NativeType::Object("ProcessHandle"), NativeType::Int],
                    NativeType::OneOf(vec![NativeType::Bytes, NativeType::Null]),
                )
                .with_optional(vec![NativeType::Int]),
                move |context, args| {
                    context.permissions.require("process.execute")?;
                    let (handle, io) = owned(context, args)?;
                    let Value::Int(maximum) = args[1] else {
                        unreachable!()
                    };
                    let timeout = match args.get(2) {
                        Some(Value::Int(value)) => *value,
                        None => 0,
                        _ => unreachable!(),
                    };
                    if !(1..=65536).contains(&maximum) || !(0..=30000).contains(&timeout) {
                        return Err(NativeError::new(
                            "RangeError",
                            "maxBytes must be 1..65536 and timeoutMs 0..30000",
                        ));
                    }
                    io.read(
                        context,
                        handle,
                        method == "readStderr",
                        maximum as usize,
                        Duration::from_millis(timeout as u64),
                    )
                },
            ),
        )?;
    }
    runtime.register(
        "Process::writeStdin",
        NativeFunction::new(
            NativeSignature::new(
                vec![
                    NativeType::Object("ProcessHandle"),
                    NativeType::OneOf(vec![NativeType::String, NativeType::Bytes]),
                ],
                NativeType::Int,
            ),
            |context, args| {
                context.permissions.require("process.execute")?;
                let (handle, io) = owned(context, args)?;
                if let Some(Err(error)) = handle.snapshot()? {
                    return Err(error);
                }
                let data: &[u8] = match &args[1] {
                    Value::String(text) => text.as_bytes(),
                    Value::Bytes(data) => data.as_ref(),
                    _ => unreachable!(),
                };
                io.input.write(context, data)
            },
        ),
    )?;
    runtime.register(
        "Process::closeStdin",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::Object("ProcessHandle")], NativeType::Null),
            |context, args| {
                context.permissions.require("process.execute")?;
                let (_, io) = owned(context, args)?;
                io.input.close(context)
            },
        ),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scr_runtime::ContextOptions;
    static PERMIT_TEST: Mutex<()> = Mutex::new(());

    #[test]
    fn queue_and_chunk_budgets_are_exact_and_failures_do_not_consume_data() {
        let _serial = PERMIT_TEST.lock().unwrap();
        let mut settings = ContextOptions::default();
        let mut input = io::empty();
        let mut output = Vec::new();
        let mut error = Vec::new();
        let mut context = RuntimeContext {
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
            max_value_bytes: Some(size_of::<Value>() + QUEUE_BYTES),
        };
        let (_, mut stdout, live_stdout) = capture(Stream::Capture, true).unwrap();
        let (_, _, live_stderr) = capture(Stream::Capture, true).unwrap();
        let io = Io::new(
            live_stdout.unwrap(),
            live_stderr.unwrap(),
            QUEUE_BYTES * 2,
            4,
            Arc::new(Permit::acquire().unwrap()),
        );
        let stdout_path = io.stdout.borrow().file.path().to_owned();
        let stderr_path = io.stderr.borrow().file.path().to_owned();
        assert_eq!(
            io.input
                .write(&context, &vec![b'x'; QUEUE_BYTES + 1])
                .unwrap(),
            Value::Int(QUEUE_BYTES as i64)
        );
        assert_eq!(io.input.write(&context, b"x").unwrap(), Value::Int(0));
        assert_eq!(io.pending_bytes(), QUEUE_BYTES);
        assert_eq!(io.input.lock().unwrap().accepted, QUEUE_BYTES);
        io.input.stop();
        // New state lets us verify allocation checks happen before enqueueing.
        let fresh = Input {
            state: Mutex::new(InputState::default()),
            ready: Condvar::new(),
            limit: 2,
        };
        context.max_value_bytes = Some(size_of::<Value>() + 1);
        assert_eq!(
            fresh.write(&context, b"ab").unwrap_err().kind,
            "ResourceLimitError"
        );
        assert_eq!(fresh.lock().unwrap().accepted, 0);
        context.max_value_bytes = Some(size_of::<Value>() + 2);
        assert_eq!(fresh.write(&context, b"ab").unwrap(), Value::Int(2));
        assert_eq!(
            fresh.write(&context, b"c").unwrap_err().kind,
            "InputLimitError"
        );
        assert_eq!(
            fresh
                .lock()
                .unwrap()
                .queue
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            b"ab"
        );
        fresh.close(&context).unwrap();
        fresh.close(&context).unwrap();
        assert_eq!(fresh.write(&context, b"").unwrap_err().kind, "ProcessError");
        stdout.as_mut().unwrap().write_all(b"abcd").unwrap();
        let handle = ProcessHandle {
            pid: 0,
            shared: Arc::new(Shared::default()),
            worker: RefCell::new(None),
            io: None,
            owner: None,
        };
        context.max_value_bytes = Some(size_of::<Value>() + 3);
        assert_eq!(
            io.read(&context, &handle, false, 4, Duration::ZERO)
                .unwrap_err()
                .kind,
            "ResourceLimitError"
        );
        assert_eq!(io.stdout.borrow().offset, 0);
        context.max_value_bytes = Some(size_of::<Value>() + 4);
        assert_eq!(
            io.read(&context, &handle, false, 4, Duration::ZERO)
                .unwrap(),
            Value::Bytes(b"abcd".as_slice().into())
        );
        assert_eq!(
            io.read(&context, &handle, false, 4, Duration::ZERO)
                .unwrap(),
            Value::Bytes(Vec::<u8>::new().into())
        );
        stdout.as_mut().unwrap().seek(SeekFrom::Start(0)).unwrap();
        stdout.as_mut().unwrap().write_all(b"e").unwrap();
        // Append-only output and already-consumed bytes still count to the cap.
        assert_eq!(stdout.as_ref().unwrap().metadata().unwrap().len(), 5);
        assert_eq!(
            io.read(&context, &handle, false, 1, Duration::ZERO)
                .unwrap_err()
                .kind,
            "OutputLimitError"
        );
        assert_eq!(io.stdout.borrow().offset, 4);
        context.cancellation = Some(Arc::new(AtomicBool::new(true)));
        assert_eq!(
            io.read(&context, &handle, false, 4, Duration::ZERO)
                .unwrap_err()
                .kind,
            "Interrupted"
        );
        assert_eq!(fresh.write(&context, b"").unwrap_err().kind, "Interrupted");
        assert_eq!(fresh.close(&context).unwrap_err().kind, "Interrupted");
        drop(io);
        drop(stdout);
        assert!(!stdout_path.exists());
        assert!(!stderr_path.exists());
    }

    #[test]
    fn shutdown_is_bounded_even_when_an_os_writer_cannot_be_interrupted() {
        let input = Arc::new(Input {
            state: Mutex::new(InputState {
                queue: vec![b'x'; QUEUE_BYTES].into(),
                pending: QUEUE_BYTES,
                ..Default::default()
            }),
            ready: Condvar::new(),
            limit: QUEUE_BYTES,
        });
        let (release, blocked) = std::sync::mpsc::channel();
        let (done, finished) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            blocked.recv().unwrap();
            done.send(()).unwrap();
        });
        let writer = Writer {
            input: input.clone(),
            worker: Some(worker),
        };
        let start = Instant::now();
        drop(writer);
        assert!(start.elapsed() < Duration::from_secs(2));
        let state = input.lock().unwrap();
        assert!(state.stopped);
        assert_eq!(state.pending, 0);
        assert_eq!(state.queue.capacity(), 0);
        drop(state);
        release.send(()).unwrap();
        finished.recv_timeout(Duration::from_secs(2)).unwrap();
    }

    #[test]
    fn writer_permits_enforce_exact_ceiling_and_release_on_drop() {
        let _serial = PERMIT_TEST.lock().unwrap();
        let permits: Vec<_> = (0..MAX_SESSIONS)
            .map(|_| Permit::acquire().unwrap())
            .collect();
        assert!(matches!(Permit::acquire(), Err(error) if error.kind == "ResourceLimitError"));
        drop(permits);
        let _permit = Permit::acquire().unwrap();
    }
}
