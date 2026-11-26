//! Bounded binary file, HTTP and connected TCP streams; UDP preserves packets.
use crate::{FilesystemPolicy, bytes, http::HttpConfig, paths};
use scr_runtime::{
    MapKey, NativeError, NativeFunction, NativeObject, NativeResult, NativeSignature, NativeType,
    Runtime, RuntimeContext, Value,
};
use std::{
    cell::{Cell, RefCell},
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    net::{SocketAddr, TcpListener, TcpStream, UdpSocket},
    path::PathBuf,
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

#[path = "sockets.rs"]
mod sockets;
#[derive(Clone, Debug)]
pub struct StreamConfig {
    pub max_handles: usize,
    pub max_chunk_bytes: usize,
    pub max_transfer_bytes: u64,
    pub timeout: Duration,
}
impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            max_handles: 64,
            max_chunk_bytes: 65536,
            max_transfer_bytes: 64 * 1024 * 1024,
            timeout: Duration::from_secs(30),
        }
    }
}
impl StreamConfig {
    fn validate(&self) -> NativeResult<()> {
        if self.max_handles > 1024
            || !(1..=1024 * 1024).contains(&self.max_chunk_bytes)
            || self.max_transfer_bytes > 1024 * 1024 * 1024
            || self.timeout.is_zero()
            || self.timeout > Duration::from_secs(3600)
        {
            return Err(error("invalid host stream limits"));
        }
        Ok(())
    }
}
fn error(message: &str) -> NativeError {
    NativeError::new("StreamError", message)
}
fn limit() -> NativeError {
    NativeError::new("StreamLimitError", "stream transfer limit exceeded")
}
fn io_error(e: io::Error) -> NativeError {
    if matches!(
        e.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    ) {
        NativeError::new("TimeoutError", "stream operation timed out")
    } else {
        NativeError::new("StreamError", format!("stream I/O failed: {e}"))
    }
}
struct Registration {
    owner: Weak<()>,
    count: Weak<Cell<usize>>,
    config: StreamConfig,
}
thread_local! {static REGISTRATIONS: RefCell<Vec<Registration>>=const {RefCell::new(Vec::new())};}
type WeakState = Weak<RefCell<Option<State>>>;
type SharedState = Rc<RefCell<Option<State>>>;
pub(crate) struct InputScope {
    alive: Cell<bool>,
    handles: RefCell<Vec<WeakState>>,
}
impl std::fmt::Debug for InputScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RequestInputScope")
            .field("alive", &self.alive.get())
            .finish()
    }
}
impl InputScope {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            alive: Cell::new(true),
            handles: RefCell::new(vec![]),
        })
    }
    pub fn close(&self) {
        self.alive.set(false);
        for handle in self.handles.borrow_mut().drain(..) {
            if let Some(state) = handle.upgrade() {
                state.borrow_mut().take();
            }
        }
    }
}
struct Lease(Rc<Cell<usize>>);
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1)
    }
}
fn reserve(count: &Rc<Cell<usize>>, config: &StreamConfig) -> NativeResult<Lease> {
    if count.get() >= config.max_handles {
        return Err(NativeError::new(
            "StreamLimitError",
            "open stream handle limit exceeded",
        ));
    }
    count.set(count.get() + 1);
    Ok(Lease(count.clone()))
}
enum Backend {
    File(File),
    Http(Box<dyn Read>),
    Input(Box<dyn Read>),
    Tcp(TcpStream),
    Udp(UdpSocket),
    Listener(TcpListener),
}
struct State {
    backend: Backend,
    _lease: Lease,
    used: u64,
    eof: bool,
}
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Bytes,
    Datagram,
    Listener,
}
struct Resource {
    state: SharedState,
    owner: Rc<()>,
    readable: bool,
    writable: bool,
    seekable: bool,
    config: StreamConfig,
    ceiling: u64,
    timeout: Cell<Duration>,
    blocking: Cell<bool>,
    network_scope: &'static str,
    kind: Kind,
    deadline: Option<Instant>,
    cap_read: &'static str,
    cap_write: &'static str,
    status: Option<u16>,
    headers: Value,
    request_guard: Option<Rc<InputScope>>,
    extra_payload: usize,
}
impl std::fmt::Debug for Resource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stream")
            .field("closed", &self.state.borrow().is_none())
            .finish()
    }
}
impl NativeObject for Resource {
    fn type_name(&self) -> &str {
        match self.kind {
            Kind::Bytes => "ByteStream",
            Kind::Datagram => "DatagramSocket",
            Kind::Listener => "SocketListener",
        }
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn payload_bytes(&self) -> usize {
        crate::http::header_bytes(&self.headers).saturating_add(self.extra_payload)
    }
    fn get_property(&self, name: &str) -> Result<Value, String> {
        Ok(match name {
            "blocking" => Value::Bool(self.blocking.get()),
            "closed" => Value::Bool(self.state.borrow().is_none()),
            "readable" => Value::Bool(self.readable),
            "writable" => Value::Bool(self.writable),
            "seekable" => Value::Bool(self.seekable),
            "status" => self
                .status
                .map_or(Value::Null, |n| Value::Int(i64::from(n))),
            "headers" => self.headers.clone(),
            _ => return Err(format!("unknown stream property {name}")),
        })
    }
}
fn resource<'a>(context: &RuntimeContext<'_>, value: &'a Value) -> NativeResult<&'a Resource> {
    context.check_cancelled()?;
    let Value::NativeObject(object) = value else {
        return Err(NativeError::type_error("expected a stream handle"));
    };
    let stream = object
        .as_any()
        .and_then(|o| o.downcast_ref::<Resource>())
        .ok_or_else(|| NativeError::type_error("expected a real stream handle"))?;
    if !context
        .runtime_identity
        .as_ref()
        .is_some_and(|owner| Rc::ptr_eq(owner, &stream.owner))
    {
        return Err(error("stream belongs to another runtime"));
    }
    Ok(stream)
}
impl Resource {
    fn check(&self, context: &RuntimeContext<'_>, writing: bool) -> NativeResult<()> {
        context.check_cancelled()?;
        if self.request_guard.as_ref().is_some_and(|g| !g.alive.get()) {
            self.state.borrow_mut().take();
            return Err(error("request stream has expired"));
        }
        if self.state.borrow().is_none() {
            return Err(error("stream is closed"));
        }
        if writing && !self.writable || !writing && !self.readable {
            return Err(error("stream mode does not permit this operation"));
        }
        let capability = if writing {
            self.cap_write
        } else {
            self.cap_read
        };
        if capability != "web.input" {
            context.permissions.require(capability)?;
        }
        if self.cap_read == "network.datagram" {
            context.permissions.require(self.network_scope)?;
        }
        if self.deadline.is_some_and(|d| Instant::now() >= d) {
            self.state.borrow_mut().take();
            return Err(NativeError::new(
                "TimeoutError",
                "HTTP stream deadline exceeded",
            ));
        }
        Ok(())
    }
    fn byte_stream(&self) -> NativeResult<()> {
        if self.kind != Kind::Bytes {
            Err(error(
                "UDP requires send/receive operations preserving datagrams",
            ))
        } else {
            Ok(())
        }
    }
    fn read_into(&self, context: &RuntimeContext<'_>, buffer: &mut [u8]) -> NativeResult<usize> {
        self.check(context, false)?;
        self.byte_stream()?;
        if buffer.is_empty() {
            return Ok(0);
        }
        let mut guard = self.state.borrow_mut();
        let state = guard.as_mut().unwrap();
        let remaining = self.ceiling.saturating_sub(state.used);
        let n = buffer.len().min(remaining.max(1) as usize);
        let result = (|| {
            let count = match &mut state.backend {
                Backend::File(file) => file.read(&mut buffer[..n]).map_err(io_error)?,
                Backend::Input(reader) => reader.read(&mut buffer[..n]).map_err(io_error)?,
                Backend::Http(reader) => reader
                    .read(&mut buffer[..n])
                    .map_err(|e| crate::http::failure(e.into()))?,
                Backend::Tcp(socket) => sockets::io(
                    context,
                    self.timeout.get(),
                    self.blocking.get(),
                    socket,
                    mio::Interest::READABLE,
                    |s| s.read(&mut buffer[..n]),
                )?,
                Backend::Udp(_) | Backend::Listener(_) => unreachable!(),
            };
            if count as u64 > remaining {
                return Err(limit());
            }
            state.used += count as u64;
            state.eof = count == 0;
            Ok(count)
        })();
        if result
            .as_ref()
            .is_err_and(|e: &NativeError| e.kind != "WouldBlockError")
        {
            guard.take();
        }
        result
    }
    fn write_bytes(&self, context: &RuntimeContext<'_>, buffer: &[u8]) -> NativeResult<usize> {
        self.check(context, true)?;
        self.byte_stream()?;
        let mut guard = self.state.borrow_mut();
        let state = guard.as_mut().unwrap();
        if buffer.len() as u64 > self.ceiling.saturating_sub(state.used) {
            return Err(limit());
        }
        let result = match &mut state.backend {
            Backend::File(file) => file.write(buffer).map_err(io_error),
            Backend::Tcp(socket) => sockets::io(
                context,
                self.timeout.get(),
                self.blocking.get(),
                socket,
                mio::Interest::WRITABLE,
                |s| s.write(buffer),
            ),
            _ => Err(error("stream is not writable")),
        }
        .and_then(|n| {
            if n == 0 && !buffer.is_empty() {
                Err(error("stream made no write progress"))
            } else {
                state.used += n as u64;
                state.eof = false;
                Ok(n)
            }
        });
        if result
            .as_ref()
            .is_err_and(|e: &NativeError| e.kind != "WouldBlockError")
        {
            guard.take();
        }
        result
    }
}
struct Options {
    timeout_specified: bool,
    timeout: Duration,
    ceiling: u64,
    http: Value,
}
fn options(value: Option<&Value>, config: &StreamConfig) -> NativeResult<Options> {
    let mut result = Options {
        timeout_specified: false,
        timeout: config.timeout,
        ceiling: config.max_transfer_bytes,
        http: Value::map(vec![]),
    };
    let entries = match value {
        None => return Ok(result),
        Some(Value::List(v)) if v.is_empty() => return Ok(result),
        Some(Value::Map(v)) => v,
        _ => {
            return Err(NativeError::type_error(
                "stream options require a map or empty list",
            ));
        }
    };
    let Value::Map(http) = &mut result.http else {
        unreachable!()
    };
    for (key, value) in entries {
        match (key, value) {
            (MapKey::String(key), Value::Int(n))
                if key == "timeoutMs" && *n > 0 && *n as u128 <= config.timeout.as_millis() =>
            {
                result.timeout = Duration::from_millis(*n as u64);
                result.timeout_specified = true;
            }
            (MapKey::String(key), Value::Int(n))
                if key == "maxBytes" && *n >= 0 && *n as u64 <= config.max_transfer_bytes =>
            {
                result.ceiling = *n as u64
            }
            (MapKey::String(key), _)
                if matches!(key.as_str(), "headers" | "maxBodyBytes" | "maxRedirects") =>
            {
                http.push((MapKey::String(key.clone()), value.clone()))
            }
            _ => {
                return Err(NativeError::type_error(
                    "unknown stream option or value exceeds host ceiling",
                ));
            }
        }
    }
    Ok(result)
}
fn text(value: &Value) -> &str {
    let Value::String(s) = value else {
        unreachable!()
    };
    s
}
fn integer(value: &Value) -> i64 {
    let Value::Int(n) = value else { unreachable!() };
    *n
}
fn chunk(value: &Value, config: &StreamConfig) -> NativeResult<usize> {
    usize::try_from(integer(value))
        .ok()
        .filter(|n| *n <= config.max_chunk_bytes)
        .ok_or_else(|| NativeError::new("RangeError", "chunk length exceeds stream limit"))
}
fn file_path(context: &RuntimeContext<'_>, uri: &str) -> NativeResult<PathBuf> {
    let Some(path) = uri.strip_prefix("file://") else {
        return paths::resolve(context, uri);
    };
    if !path.starts_with('/') || path.contains(['?', '#']) {
        return Err(error(
            "file:// requires an absolute local path without query or fragment",
        ));
    }
    let decoded = crate::url::invoke(
        "Url::decodeComponent",
        context,
        &[Value::String(path.into())],
    )?;
    let path = text(&decoded);
    #[cfg(windows)]
    let path = if path.as_bytes().get(2) == Some(&b':') {
        &path[1..]
    } else {
        path
    };
    paths::resolve(context, path)
}
fn file(
    context: &RuntimeContext<'_>,
    policy: &FilesystemPolicy,
    path: &std::path::Path,
    mode: &str,
) -> NativeResult<(File, bool, bool)> {
    let read = matches!(mode, "r" | "r+" | "w+" | "a+" | "x+");
    let write = matches!(mode, "r+" | "w" | "w+" | "a" | "a+" | "x" | "x+");
    if !read && !write {
        return Err(error("file mode must be r, r+, w, w+, a, a+, x or x+"));
    }
    if read {
        policy.check(context, path, false)?
    }
    if write {
        policy.check(context, path, true)?
    }
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if !meta.is_file() || meta.file_type().is_symlink() {
                return Err(error(
                    "file stream requires a regular file without a final symlink",
                ));
            }
            if write && meta.permissions().readonly() {
                return Err(NativeError::new(
                    "PermissionError",
                    "stream destination is read-only",
                ));
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(io_error(e)),
    }
    let mut builder = OpenOptions::new();
    builder
        .read(read)
        .write(write)
        .append(mode.starts_with('a'));
    if mode.starts_with('x') {
        builder.create_new(true);
    } else if matches!(mode, "w" | "w+" | "a" | "a+") {
        builder.create(true);
    }
    let file = builder.open(path).map_err(io_error)?;
    if !file.metadata().map_err(io_error)?.is_file() {
        return Err(error("opened stream is not a regular file"));
    }
    if mode.starts_with('w') {
        context.check_cancelled()?;
        file.set_len(0).map_err(io_error)?;
    }
    Ok((file, read, write))
}
fn endpoint(uri: &str) -> NativeResult<SocketAddr> {
    let (_, endpoint) = uri
        .split_once("://")
        .ok_or_else(|| error("invalid transport URI"))?;
    endpoint
        .parse::<SocketAddr>()
        .ok()
        .filter(|a| a.port() != 0)
        .ok_or_else(|| error("transport URI requires a literal IP address and port 1..65535"))
}
fn new_handle(
    runtime_owner: Rc<()>,
    lease: Lease,
    config: StreamConfig,
    options: Options,
    backend: Backend,
    access: (bool, bool),
    cap: &'static str,
) -> Resource {
    let seekable = matches!(backend, Backend::File(_));
    let kind = match backend {
        Backend::Udp(_) => Kind::Datagram,
        Backend::Listener(_) => Kind::Listener,
        _ => Kind::Bytes,
    };
    Resource {
        state: Rc::new(RefCell::new(Some(State {
            backend,
            _lease: lease,
            used: 0,
            eof: false,
        }))),
        owner: runtime_owner,
        readable: access.0,
        writable: access.1,
        seekable,
        config,
        ceiling: options.ceiling,
        timeout: Cell::new(options.timeout),
        blocking: Cell::new(true),
        network_scope: "network.connect",
        kind,
        deadline: None,
        cap_read: cap,
        cap_write: if cap == "filesystem.read" {
            "filesystem.write"
        } else {
            cap
        },
        status: None,
        headers: Value::map(vec![]),
        request_guard: None,
        extra_payload: 0,
    }
}

pub(crate) fn register(
    runtime: &mut Runtime,
    config: StreamConfig,
    policy: FilesystemPolicy,
    http_config: HttpConfig,
) -> NativeResult<()> {
    config.validate()?;
    let policy = policy.prepare()?;
    let owner = runtime.native_identity();
    let count = Rc::new(Cell::new(0));
    REGISTRATIONS.with(|items| {
        let mut items = items.borrow_mut();
        items.retain(|r| r.owner.strong_count() != 0 && r.count.strong_count() != 0);
        items.push(Registration {
            owner: Rc::downgrade(&owner),
            count: Rc::downgrade(&count),
            config: config.clone(),
        })
    });
    sockets::register(runtime, owner.clone(), count.clone(), config.clone())?;
    let handle = NativeType::Object("ByteStream");
    let packet = NativeType::Object("DatagramSocket");
    for (namespace, operation) in ["Stream", "Socket"].into_iter().flat_map(|namespace| {
        [
            "open", "request", "read", "write", "flush", "close", "seek", "tell", "eof", "copy",
            "send", "receive",
        ]
        .into_iter()
        .filter(move |op| {
            namespace == "Stream" || matches!(*op, "read" | "write" | "receive" | "close")
        })
        .map(move |op| (namespace, op))
    }) {
        let signature = match operation {
            "open" => NativeSignature::new(vec![NativeType::String], NativeType::Any)
                .with_optional(vec![NativeType::String, NativeType::Any]),
            "request" => NativeSignature::new(
                vec![NativeType::String, NativeType::String, handle.clone()],
                handle.clone(),
            )
            .with_optional(vec![NativeType::Any]),
            "read" => NativeSignature::new(vec![handle.clone(), NativeType::Int], NativeType::Any),
            "write" => {
                NativeSignature::new(vec![handle.clone(), NativeType::Bytes], NativeType::Int)
            }
            "send" => {
                NativeSignature::new(vec![packet.clone(), NativeType::Bytes], NativeType::Int)
            }
            "receive" => {
                NativeSignature::new(vec![packet.clone(), NativeType::Int], NativeType::Map)
            }
            "seek" => NativeSignature::new(vec![handle.clone(), NativeType::Int], NativeType::Int)
                .with_optional(vec![NativeType::String]),
            "tell" => NativeSignature::new(vec![handle.clone()], NativeType::Int),
            "eof" => NativeSignature::new(vec![handle.clone()], NativeType::Bool),
            "copy" => NativeSignature::new(
                vec![handle.clone(), handle.clone(), NativeType::Int],
                NativeType::Int,
            ),
            _ => NativeSignature::new(
                vec![NativeType::OneOf(vec![
                    handle.clone(),
                    packet.clone(),
                    NativeType::Object("SocketListener"),
                ])],
                NativeType::Null,
            ),
        };
        let owner = owner.clone();
        let count = count.clone();
        let config = config.clone();
        let policy = policy.clone();
        let http_config = http_config.clone();
        runtime.register(
            format!("{namespace}::{operation}"),
            NativeFunction::new(signature, move |context, args| {
                context.check_cancelled()?;
                if !context
                    .runtime_identity
                    .as_ref()
                    .is_some_and(|id| Rc::ptr_eq(id, &owner))
                {
                    return Err(error("stream API belongs to another runtime"));
                }
                bytes::budget(context, size_of::<Value>())?;
                if operation == "open" || operation == "request" {
                    bytes::budget(context, 2 * size_of::<Value>())?;
                    let uri = text(&args[if operation == "request" { 1 } else { 0 }]);
                    let network = uri.starts_with("tcp://") || uri.starts_with("udp://");
                    let mode = if operation == "request" {
                        "r"
                    } else {
                        args.get(1)
                            .map(text)
                            .unwrap_or(if network { "rw" } else { "r" })
                    };
                    let opts = options(
                        args.get(if operation == "request" { 3 } else { 2 }),
                        &config,
                    )?;
                    let lease = reserve(&count, &config)?;
                    let result = if uri.starts_with("http://") || uri.starts_with("https://") {
                        if mode != "r" {
                            return Err(error(
                                "HTTP streams are read-only; Stream::request sends a source stream",
                            ));
                        }
                        let source = if operation == "request" {
                            Some(resource(context, &args[2])?)
                        } else {
                            None
                        };
                        if let Some(source) = source {
                            source.check(context, false)?;
                            if !source.seekable && source.cap_read!="web.input" {
                                return Err(error(
                                    "HTTP upload source must be a readable file stream or request input",
                                ));
                            }
                        }
                        let mut adapter = source.map(|source| Source {
                            source,
                            context,
                            remaining: opts.ceiling,
                        });
                        let mut http_config = http_config.clone();
                        http_config.timeout = http_config.timeout.min(opts.timeout);
                        http_config.max_response_bytes =
                            http_config.max_response_bytes.min(opts.ceiling);
                        let response = crate::http::open_stream(
                            context,
                            &http_config,
                            if operation == "request" {
                                text(&args[0])
                            } else {
                                "GET"
                            },
                            uri,
                            &opts.http,
                            adapter.as_mut().map(|s| s as &mut dyn Read),
                        )?;
                        let mut result = new_handle(
                            owner.clone(),
                            lease,
                            config.clone(),
                            opts,
                            Backend::Http(response.reader),
                            (true, false),
                            "http.request",
                        );
                        result.ceiling = response.limit;
                        result.status = Some(response.status);
                        result.headers = response.headers;
                        result.deadline = Some(response.deadline);
                        result
                    } else if operation == "request" {
                        return Err(error("Stream::request requires an absolute HTTP/HTTPS URL"));
                    } else if network {
                        if mode != "rw" {
                            return Err(error("connected transports require rw mode"));
                        }
                        if let Value::Map(entries) = &opts.http {
                            if !entries.is_empty() {
                                return Err(error(
                                    "HTTP options are not valid for transport streams",
                                ));
                            }
                        }
                        context.permissions.require("network.connect")?;
                        let address = endpoint(uri)?;
                        let (backend, cap) = if uri.starts_with("tcp://") {
                            let connected = TcpStream::connect_timeout(&address, opts.timeout);
                            context.check_cancelled()?;
                            let socket = connected.map_err(io_error)?;
                            socket.set_nonblocking(true).map_err(io_error)?;
                            (Backend::Tcp(socket), "network.connect")
                        } else {
                            context.permissions.require("network.datagram")?;
                            let socket = UdpSocket::bind(if address.is_ipv4() {
                                "0.0.0.0:0"
                            } else {
                                "[::]:0"
                            })
                            .map_err(io_error)?;
                            socket.connect(address).map_err(io_error)?;
                            socket.set_nonblocking(true).map_err(io_error)?;
                            (Backend::Udp(socket), "network.datagram")
                        };
                        context.check_cancelled()?;
                        new_handle(
                            owner.clone(),
                            lease,
                            config.clone(),
                            opts,
                            backend,
                            (true, true),
                            cap,
                        )
                    } else {
                        if uri.contains("://") && !uri.starts_with("file://") {
                            return Err(error("unsupported stream wrapper"));
                        }
                        if let Value::Map(entries) = &opts.http {
                            if !entries.is_empty() {
                                return Err(error("HTTP options are not valid for file streams"));
                            }
                        }
                        if opts.timeout_specified {
                            return Err(error(
                                "timeoutMs applies to network streams, not filesystem I/O",
                            ));
                        }
                        let path = file_path(context, uri)?;
                        let (file, read, write) = file(context, &policy, &path, mode)?;
                        new_handle(
                            owner.clone(),
                            lease,
                            config.clone(),
                            opts,
                            Backend::File(file),
                            (read, write),
                            "filesystem.read",
                        )
                    };
                    bytes::budget(
                        context,
                        size_of::<Value>().saturating_add(result.payload_bytes()),
                    )?;
                    return Ok(Value::NativeObject(Rc::new(result)));
                }
                let source = resource(context, &args[0])?;
                if namespace == "Socket"
                    && !matches!(
                        source.cap_read,
                        "network.connect" | "network.listen" | "network.datagram"
                    )
                {
                    return Err(error("operation requires a socket"));
                }
                match operation {
                    "close" => {
                        source.state.borrow_mut().take();
                        Ok(Value::Null)
                    }
                    "read" => {
                        let size = chunk(&args[1], &source.config)?;
                        source.check(context, false)?;
                        source.byte_stream()?;
                        bytes::payload(context, size)?;
                        if size == 0 {
                            return Ok(Value::Bytes(Rc::from([])));
                        }
                        let mut buffer = Vec::new();
                        buffer
                            .try_reserve_exact(size)
                            .map_err(|_| bytes::resource("cannot allocate stream chunk"))?;
                        buffer.resize(size, 0);
                        let n = source.read_into(context, &mut buffer)?;
                        buffer.truncate(n);
                        Ok(if n == 0 {
                            Value::Null
                        } else {
                            Value::Bytes(buffer.into())
                        })
                    }
                    "write" => {
                        let Value::Bytes(bytes) = &args[1] else {
                            unreachable!()
                        };
                        if bytes.len() > source.config.max_chunk_bytes {
                            return Err(limit());
                        }
                        Ok(Value::Int(source.write_bytes(context, bytes)? as i64))
                    }
                    "eof" => {
                        source.check(context, false)?;
                        source.byte_stream()?;
                        Ok(Value::Bool(source.state.borrow().as_ref().unwrap().eof))
                    }
                    "tell" | "seek" => {
                        if !source.seekable {
                            return Err(error("stream is not seekable"));
                        }
                        source.check(context, !source.readable)?;
                        let offset = if operation == "tell" {
                            SeekFrom::Current(0)
                        } else {
                            let offset = integer(&args[1]);
                            match args.get(2).map(text).unwrap_or("start") {
                                "start" => SeekFrom::Start(
                                    u64::try_from(offset)
                                        .map_err(|_| error("start offset must be nonnegative"))?,
                                ),
                                "current" => SeekFrom::Current(offset),
                                "end" => SeekFrom::End(offset),
                                _ => {
                                    return Err(error("seek origin must be start, current or end"));
                                }
                            }
                        };
                        let mut guard = source.state.borrow_mut();
                        let state = guard.as_mut().unwrap();
                        let Backend::File(file) = &mut state.backend else {
                            unreachable!()
                        };
                        let result = file.seek(offset).map_err(io_error).and_then(|position| {
                            i64::try_from(position)
                                .map(Value::Int)
                                .map_err(|_| error("stream position exceeds integer range"))
                        });
                        if result.is_err() {
                            guard.take();
                        } else if operation == "seek" {
                            guard.as_mut().unwrap().eof = false;
                        }
                        result
                    }
                    "flush" => {
                        source.check(context, true)?;
                        let mut guard = source.state.borrow_mut();
                        let state = guard.as_mut().unwrap();
                        let result = match &mut state.backend {
                            Backend::File(file) => file.flush(),
                            Backend::Tcp(socket) => socket.flush(),
                            Backend::Udp(_) => Ok(()),
                            Backend::Http(_) | Backend::Input(_) | Backend::Listener(_) => {
                                unreachable!()
                            }
                        };
                        if result.is_err() {
                            guard.take();
                        }
                        result.map_err(io_error)?;
                        Ok(Value::Null)
                    }
                    "copy" => {
                        let destination = resource(context, &args[1])?;
                        source.check(context, false)?;
                        destination.check(context, true)?;
                        source.byte_stream()?;
                        destination.byte_stream()?;
                        if std::ptr::eq(source, destination) {
                            return Err(error("copy requires different stream handles"));
                        }
                        let maximum = u64::try_from(integer(&args[2]))
                            .ok()
                            .filter(|n| *n <= config.max_transfer_bytes)
                            .ok_or_else(limit)?;
                        if maximum
                            > source
                                .ceiling
                                .saturating_sub(source.state.borrow().as_ref().unwrap().used)
                            || maximum
                                > destination.ceiling.saturating_sub(
                                    destination.state.borrow().as_ref().unwrap().used,
                                )
                        {
                            return Err(limit());
                        }
                        let mut copied = 0u64;
                        let mut buffer = vec![
                            0;
                            source
                                .config
                                .max_chunk_bytes
                                .min(destination.config.max_chunk_bytes)
                                .min(65536)
                        ];
                        while copied < maximum {
                            context.check_cancelled()?;
                            let size = buffer.len().min((maximum - copied) as usize);
                            let n = source.read_into(context, &mut buffer[..size])?;
                            if n == 0 {
                                break;
                            }
                            let mut written = 0;
                            while written < n {
                                context.check_cancelled()?;
                                written += destination.write_bytes(context, &buffer[written..n])?;
                            }
                            copied += n as u64;
                        }
                        Ok(Value::Int(copied as i64))
                    }
                    "send" | "receive" => datagram(context, source, operation, &args[1]),
                    _ => unreachable!(),
                }
            }),
        )?;
    }
    Ok(())
}
struct Source<'a, 'b> {
    source: &'a Resource,
    context: &'a RuntimeContext<'b>,
    remaining: u64,
}
impl Read for Source<'_, '_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let size = buffer
            .len()
            .min(self.source.config.max_chunk_bytes)
            .min(self.remaining.max(1) as usize);
        let n = self
            .source
            .read_into(self.context, &mut buffer[..size])
            .map_err(io::Error::other)?;
        if n as u64 > self.remaining {
            self.source.state.borrow_mut().take();
            return Err(io::Error::other("HTTP upload exceeds stream limit"));
        }
        self.remaining -= n as u64;
        Ok(n)
    }
}
fn datagram(
    context: &RuntimeContext<'_>,
    source: &Resource,
    operation: &str,
    value: &Value,
) -> NativeResult<Value> {
    source.check(context, operation == "send")?;
    let mut guard = source.state.borrow_mut();
    let state = guard.as_mut().unwrap();
    let Backend::Udp(socket) = &mut state.backend else {
        return Err(error("operation requires a datagram socket"));
    };
    let result = if operation == "send" {
        let Value::Bytes(bytes) = value else {
            unreachable!()
        };
        if bytes.len() > 65507
            || bytes.len() > source.config.max_chunk_bytes
            || bytes.len() as u64 > source.ceiling.saturating_sub(state.used)
        {
            return Err(limit());
        }
        sockets::io(
            context,
            source.timeout.get(),
            source.blocking.get(),
            socket,
            mio::Interest::WRITABLE,
            |s| s.send(bytes),
        )
        .and_then(|n| {
            if n != bytes.len() {
                return Err(error("partial datagram send"));
            }
            state.used += n as u64;
            Ok(Value::Int(n as i64))
        })
    } else {
        let size = chunk(value, &source.config)?;
        if size > 65507 {
            return Err(NativeError::new(
                "RangeError",
                "datagram result length exceeds 65507 bytes",
            ));
        }
        // Preflight the maximum returned address (IPv6), map slots, and payload.
        bytes::budget(
            context,
            5 * size_of::<Value>()
                + 4 * size_of::<MapKey>()
                + "dataaddressporttruncated".len()
                + 45
                + size,
        )?;
        let mut buffer = vec![0; 65536];
        sockets::io(
            context,
            source.timeout.get(),
            source.blocking.get(),
            socket,
            mio::Interest::READABLE,
            |s| s.recv_from(&mut buffer),
        )
        .and_then(|(n, address)| {
            if n as u64 > source.ceiling.saturating_sub(state.used) {
                return Err(limit());
            }
            state.used += n as u64;
            let truncated = n > size;
            buffer.truncate(n.min(size));
            Ok(Value::map(vec![
                (MapKey::String("data".into()), Value::Bytes(buffer.into())),
                (
                    MapKey::String("address".into()),
                    Value::String(address.ip().to_string()),
                ),
                (
                    MapKey::String("port".into()),
                    Value::Int(i64::from(address.port())),
                ),
                (MapKey::String("truncated".into()), Value::Bool(truncated)),
            ]))
        })
    };
    if result.as_ref().is_err_and(|e| e.kind != "WouldBlockError") {
        guard.take();
    }
    result
}

/// Request-owned read-only input; private spools need no ambient filesystem grant.
pub(crate) fn request_input(
    context: &RuntimeContext<'_>,
    reader: Box<dyn Read>,
    size: u64,
    scope: Rc<InputScope>,
    payload: usize,
) -> NativeResult<Value> {
    context.check_cancelled()?;
    if !scope.alive.get() {
        return Err(error("request stream has expired"));
    }
    bytes::budget(context, 2 * size_of::<Value>() + payload)?;
    let owner = context
        .runtime_identity
        .clone()
        .ok_or_else(|| error("request input requires a runtime owner"))?;
    let (count, config) = REGISTRATIONS
        .with(|items| {
            let mut items = items.borrow_mut();
            items.retain(|r| r.owner.strong_count() != 0 && r.count.strong_count() != 0);
            items
                .iter()
                .find(|r| r.owner.upgrade().is_some_and(|id| Rc::ptr_eq(&id, &owner)))
                .and_then(|r| Some((r.count.upgrade()?, r.config.clone())))
        })
        .ok_or_else(|| error("request input requires the registered Stream module"))?;
    let lease = reserve(&count, &config)?;
    let mut input = new_handle(
        owner,
        lease,
        config.clone(),
        Options {
            timeout_specified: false,
            timeout: config.timeout,
            ceiling: size.min(config.max_transfer_bytes),
            http: Value::map(vec![]),
        },
        Backend::Input(reader),
        (true, false),
        "web.input",
    );
    input.request_guard = Some(scope.clone());
    input.extra_payload = payload;
    let mut handles = scope.handles.borrow_mut();
    handles.retain(|h| h.strong_count() != 0);
    handles.push(Rc::downgrade(&input.state));
    drop(handles);
    Ok(Value::NativeObject(Rc::new(input)))
}

/// Validate a regular-file download before response metadata is committed.
pub(crate) fn download_size(context: &RuntimeContext<'_>, value: &Value) -> NativeResult<u64> {
    let source = resource(context, value)?;
    source.check(context, false)?;
    let mut state = source.state.borrow_mut();
    let state = state.as_mut().unwrap();
    let Backend::File(file) = &mut state.backend else {
        return Err(error("downloads require a readable file stream"));
    };
    let size = file
        .metadata()
        .map_err(io_error)?
        .len()
        .saturating_sub(file.stream_position().map_err(io_error)?);
    if size > source.ceiling.saturating_sub(state.used) {
        return Err(limit());
    }
    Ok(size)
}
pub(crate) fn download(
    context: &mut RuntimeContext<'_>,
    value: &Value,
    size: u64,
) -> NativeResult<Value> {
    let source = resource(context, value)?;
    context.output.flush()?;
    let mut sent = 0u64;
    let mut buffer = vec![0; source.config.max_chunk_bytes.min(65536)];
    while sent < size {
        context.check_cancelled()?;
        let count = buffer.len().min((size - sent) as usize);
        let n = source.read_into(context, &mut buffer[..count])?;
        if n == 0 {
            return Err(error("download source ended before its measured size"));
        }
        context.output.write_all(&buffer[..n])?;
        context.output.flush()?;
        sent += n as u64;
    }
    Ok(Value::Int(sent as i64))
}
