//! Shared safe socket readiness and low-level TCP/UDP operations.
use super::*;
use mio::{Events, Interest, Poll, Token, event::Source as MioSource};
use socket2::{Domain, Protocol, SockRef, Socket, Type};
use std::net::Shutdown;

pub(super) trait Source {
    fn source(&self) -> io::Result<Box<dyn MioSource>>;
}
impl Source for TcpStream {
    fn source(&self) -> io::Result<Box<dyn MioSource>> {
        Ok(Box::new(mio::net::TcpStream::from_std(self.try_clone()?)))
    }
}
impl Source for UdpSocket {
    fn source(&self) -> io::Result<Box<dyn MioSource>> {
        Ok(Box::new(mio::net::UdpSocket::from_std(self.try_clone()?)))
    }
}
impl Source for TcpListener {
    fn source(&self) -> io::Result<Box<dyn MioSource>> {
        Ok(Box::new(mio::net::TcpListener::from_std(self.try_clone()?)))
    }
}
fn blocked() -> NativeError {
    NativeError::new("WouldBlockError", "socket is not ready")
}
pub(super) fn io<S: Source, T>(
    context: &RuntimeContext<'_>,
    timeout: Duration,
    blocking: bool,
    socket: &mut S,
    interest: Interest,
    mut operation: impl FnMut(&mut S) -> io::Result<T>,
) -> NativeResult<T> {
    let deadline = Instant::now() + timeout;
    let mut selector: Option<(Poll, Box<dyn MioSource>)> = None;
    let mut events = Events::with_capacity(8);
    loop {
        context.check_cancelled()?;
        match operation(socket) {
            Ok(value) => return Ok(value),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                if !blocking {
                    return Err(blocked());
                }
                if let Some((poll, source)) = selector.as_mut() {
                    poll.registry()
                        .reregister(&mut **source, Token(0), interest)
                        .map_err(io_error)?;
                } else {
                    let poll = Poll::new().map_err(io_error)?;
                    let mut source = socket.source().map_err(io_error)?;
                    poll.registry()
                        .register(&mut *source, Token(0), interest)
                        .map_err(io_error)?;
                    selector = Some((poll, source));
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(NativeError::new(
                        "TimeoutError",
                        "socket operation timed out",
                    ));
                }
                match selector
                    .as_mut()
                    .unwrap()
                    .0
                    .poll(&mut events, Some(remaining.min(Duration::from_millis(50))))
                {
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    other => other.map_err(io_error)?,
                };
            }
            Err(e) => return Err(io_error(e)),
        }
    }
}
fn address(host: &str, port: i64, allow_zero: bool) -> NativeResult<SocketAddr> {
    let ip = host
        .parse::<std::net::IpAddr>()
        .map_err(|_| error("socket address must be a literal IPv4 or IPv6 address"))?;
    let port = u16::try_from(port)
        .ok()
        .filter(|n| allow_zero || *n != 0)
        .ok_or_else(|| error("socket port is out of range"))?;
    Ok(SocketAddr::new(ip, port))
}
fn endpoint_value(address: SocketAddr) -> Value {
    Value::Map(vec![
        (
            MapKey::String("address".into()),
            Value::String(address.ip().to_string()),
        ),
        (
            MapKey::String("port".into()),
            Value::Int(i64::from(address.port())),
        ),
    ])
}
fn endpoint_budget(context: &RuntimeContext<'_>) -> NativeResult<()> {
    bytes::budget(
        context,
        3 * size_of::<Value>() + 2 * size_of::<MapKey>() + 11 + 45,
    )
}
struct OpenOptions {
    common: Options,
    blocking: bool,
    reuse: bool,
    ipv6: bool,
    backlog: i32,
    no_delay: bool,
}
fn open_options(value: Option<&Value>, config: &StreamConfig) -> NativeResult<OpenOptions> {
    let mut result = OpenOptions {
        common: options(None, config)?,
        blocking: true,
        reuse: false,
        ipv6: true,
        backlog: 128,
        no_delay: false,
    };
    let mut common = vec![];
    let entries = match value {
        None => return Ok(result),
        Some(Value::List(v)) if v.is_empty() => return Ok(result),
        Some(Value::Map(v)) => v,
        _ => {
            return Err(NativeError::type_error(
                "socket options require a map or empty list",
            ));
        }
    };
    for (k, v) in entries {
        match (k, v) {
            (MapKey::String(k), Value::Bool(v)) if k == "blocking" => result.blocking = *v,
            (MapKey::String(k), Value::Bool(v)) if k == "reuseAddress" => result.reuse = *v,
            (MapKey::String(k), Value::Bool(v)) if k == "ipv6Only" => result.ipv6 = *v,
            (MapKey::String(k), Value::Bool(v)) if k == "noDelay" => result.no_delay = *v,
            (MapKey::String(k), Value::Int(v)) if k == "backlog" && (1..=4096).contains(v) => {
                result.backlog = *v as i32
            }
            (MapKey::String(k), _) if k == "timeoutMs" || k == "maxBytes" => {
                common.push((MapKey::String(k.clone()), v.clone()))
            }
            _ => {
                return Err(NativeError::type_error(
                    "unknown socket option or invalid value",
                ));
            }
        }
    }
    result.common = options(Some(&Value::Map(common)), config)?;
    Ok(result)
}
fn network<'a>(context: &RuntimeContext<'_>, value: &'a Value) -> NativeResult<&'a Resource> {
    resource(context, value)
}
// Enforce the authority granted when the handle was created, including on poll/options.
fn check(context: &RuntimeContext<'_>, resource: &Resource) -> NativeResult<()> {
    resource.check(context, !resource.readable)?;
    if !matches!(
        resource.state.borrow().as_ref().unwrap().backend,
        Backend::Tcp(_) | Backend::Udp(_) | Backend::Listener(_)
    ) {
        return Err(error("operation requires a socket"));
    }
    Ok(())
}
fn sockref(state: &State) -> NativeResult<SockRef<'_>> {
    match &state.backend {
        Backend::Tcp(s) => Ok(SockRef::from(s)),
        Backend::Udp(s) => Ok(SockRef::from(s)),
        Backend::Listener(s) => Ok(SockRef::from(s)),
        _ => Err(error("operation requires a socket")),
    }
}
fn option(
    context: &RuntimeContext<'_>,
    resource: &Resource,
    key: &str,
    value: Option<&Value>,
) -> NativeResult<Value> {
    check(context, resource)?;
    let guard = resource.state.borrow();
    let state = guard.as_ref().unwrap();
    match key {
        "noDelay" => {
            let Backend::Tcp(socket) = &state.backend else {
                return Err(error("noDelay requires TCP"));
            };
            if let Some(Value::Bool(v)) = value {
                socket.set_nodelay(*v).map_err(io_error)?;
            } else if value.is_some() {
                return Err(NativeError::type_error("noDelay requires a bool"));
            }
            Ok(Value::Bool(socket.nodelay().map_err(io_error)?))
        }
        "keepAlive" => {
            if !matches!(state.backend, Backend::Tcp(_)) {
                return Err(error("keepAlive requires TCP"));
            }
            let socket = sockref(state)?;
            if let Some(Value::Bool(v)) = value {
                socket.set_keepalive(*v).map_err(io_error)?
            } else if value.is_some() {
                return Err(NativeError::type_error("keepAlive requires a bool"));
            }
            Ok(Value::Bool(socket.keepalive().map_err(io_error)?))
        }
        "receiveBufferBytes" | "sendBufferBytes" => {
            let socket = sockref(state)?;
            if let Some(value) = value {
                let Value::Int(n) = value else {
                    return Err(NativeError::type_error(
                        "socket buffer size requires an int",
                    ));
                };
                if !(1024..=4 * 1024 * 1024).contains(n) {
                    return Err(error("socket buffer size must be 1024..4194304"));
                }
                if key == "receiveBufferBytes" {
                    socket.set_recv_buffer_size(*n as usize)
                } else {
                    socket.set_send_buffer_size(*n as usize)
                }
                .map_err(io_error)?;
            }
            let size = if key == "receiveBufferBytes" {
                socket.recv_buffer_size()
            } else {
                socket.send_buffer_size()
            }
            .map_err(io_error)?;
            Ok(Value::Int(size as i64))
        }
        _ => Err(error("unsupported socket option")),
    }
}
pub(super) fn register(
    runtime: &mut Runtime,
    owner: Rc<()>,
    count: Rc<Cell<usize>>,
    config: StreamConfig,
) -> NativeResult<()> {
    let any = NativeType::OneOf(vec![
        NativeType::Object("ByteStream"),
        NativeType::Object("DatagramSocket"),
        NativeType::Object("SocketListener"),
    ]);
    for op in [
        "connect",
        "listen",
        "accept",
        "bindDatagram",
        "sendTo",
        "shutdown",
        "setBlocking",
        "setTimeout",
        "poll",
        "localAddress",
        "peerAddress",
        "setOption",
        "getOption",
    ] {
        let signature = match op {
            "connect" | "listen" | "bindDatagram" => {
                NativeSignature::new(vec![NativeType::String, NativeType::Int], NativeType::Any)
                    .with_optional(vec![NativeType::Any])
            }
            "accept" => NativeSignature::new(
                vec![NativeType::Object("SocketListener")],
                NativeType::Object("ByteStream"),
            ),
            "sendTo" => NativeSignature::new(
                vec![
                    NativeType::Object("DatagramSocket"),
                    NativeType::Bytes,
                    NativeType::String,
                    NativeType::Int,
                ],
                NativeType::Int,
            ),
            "shutdown" => NativeSignature::new(
                vec![NativeType::Object("ByteStream"), NativeType::String],
                NativeType::Null,
            ),
            "setBlocking" => {
                NativeSignature::new(vec![any.clone(), NativeType::Bool], NativeType::Null)
            }
            "setTimeout" => {
                NativeSignature::new(vec![any.clone(), NativeType::Int], NativeType::Null)
            }
            "poll" => NativeSignature::new(
                vec![NativeType::List, NativeType::List, NativeType::Int],
                NativeType::Map,
            ),
            "setOption" => NativeSignature::new(
                vec![any.clone(), NativeType::String, NativeType::Any],
                NativeType::Null,
            ),
            "getOption" => {
                NativeSignature::new(vec![any.clone(), NativeType::String], NativeType::Any)
            }
            _ => NativeSignature::new(vec![any.clone()], NativeType::Map),
        };
        let config = config.clone();
        let count = count.clone();
        let owner = owner.clone();
        runtime.register(
            format!("Socket::{op}"),
            NativeFunction::new(signature, move |context, args| {
                context.check_cancelled()?;
                bytes::budget(context, size_of::<Value>())?;
                if !context
                    .runtime_identity
                    .as_ref()
                    .is_some_and(|id| Rc::ptr_eq(id, &owner))
                {
                    return Err(error("socket API belongs to another runtime"));
                }
                if matches!(op, "connect" | "listen" | "bindDatagram") {
                    bytes::budget(context, 2 * size_of::<Value>())?;
                    let address = address(text(&args[0]), integer(&args[1]), op != "connect")?;
                    let opts = open_options(args.get(2), &config)?;
                    if let Some(Value::Map(entries))=args.get(2) {for (key,_) in entries {let MapKey::String(key)=key else {unreachable!()};let allowed=match op {"connect"=>matches!(key.as_str(),"timeoutMs"|"maxBytes"|"blocking"|"noDelay"),"listen"=>key!="noDelay",_=>!matches!(key.as_str(),"noDelay"|"backlog")};if !allowed || address.is_ipv4() && key=="ipv6Only" {return Err(error("socket option does not apply to this operation or address family"))}}}
                    let cap = match op {
                        "connect" => "network.connect",
                        "listen" => "network.listen",
                        _ => "network.datagram",
                    };
                    context.permissions.require(cap)?;
                    if op == "bindDatagram" {
                        context.permissions.require("network.bind")?;
                    }
                    let lease = reserve(&count, &config)?;
                    let backend = if op == "connect" {
                        let socket = TcpStream::connect_timeout(&address, opts.common.timeout);
                        context.check_cancelled()?;
                        let socket = socket.map_err(io_error)?;
                        socket.set_nonblocking(true).map_err(io_error)?;
                        socket.set_nodelay(opts.no_delay).map_err(io_error)?;
                        Backend::Tcp(socket)
                    } else {
                        let socket = Socket::new(
                            if address.is_ipv4() {
                                Domain::IPV4
                            } else {
                                Domain::IPV6
                            },
                            if op == "listen" {
                                Type::STREAM
                            } else {
                                Type::DGRAM
                            },
                            Some(if op == "listen" {
                                Protocol::TCP
                            } else {
                                Protocol::UDP
                            }),
                        )
                        .map_err(io_error)?;
                        if address.is_ipv6() {
                            socket.set_only_v6(opts.ipv6).map_err(io_error)?;
                        }
                        // SO_REUSEADDR has different exclusivity semantics on Windows.
                        #[cfg(windows)]
                        if opts.reuse {
                            return Err(error("reuseAddress is unsupported on Windows"));
                        }
                        socket.set_reuse_address(opts.reuse).map_err(io_error)?;
                        socket.set_nonblocking(true).map_err(io_error)?;
                        socket.bind(&address.into()).map_err(io_error)?;
                        if op == "listen" {
                            socket.listen(opts.backlog).map_err(io_error)?;
                            Backend::Listener(socket.into())
                        } else {
                            Backend::Udp(socket.into())
                        }
                    };
                    let result = new_handle(
                        owner.clone(),
                        lease,
                        config.clone(),
                        opts.common,
                        backend,
                        (true, op != "listen"),
                        cap,
                    );
                    result.blocking.set(opts.blocking);
                    let mut result = result;
                    if op == "bindDatagram" {
                        result.network_scope = "network.bind";
                    }
                    Ok(Value::NativeObject(Rc::new(result)))
                } else if op == "poll" {
                    poll_handles(context, args, &config)
                } else {
                    let handle = network(context, &args[0])?;
                    check(context, handle)?;
                    match op {
                        "accept" => {
                            bytes::budget(context, 2 * size_of::<Value>())?;
                            let lease = reserve(&count, &config)?;
                            let mut state = handle.state.borrow_mut();
                            let Backend::Listener(listener) = &mut state.as_mut().unwrap().backend
                            else {
                                return Err(error("accept requires a listener"));
                            };
                            let (socket, _) = io(
                                context,
                                handle.timeout.get(),
                                handle.blocking.get(),
                                listener,
                                Interest::READABLE,
                                |s| s.accept(),
                            )?;
                            socket.set_nonblocking(true).map_err(io_error)?;
                            let result = new_handle(
                                owner.clone(),
                                lease,
                                config.clone(),
                                Options {
                                    timeout_specified: false,
                                    timeout: handle.timeout.get(),
                                    ceiling: handle.ceiling,
                                    http: Value::Map(vec![]),
                                },
                                Backend::Tcp(socket),
                                (true, true),
                                "network.listen",
                            );
                            result.blocking.set(handle.blocking.get());
                            Ok(Value::NativeObject(Rc::new(result)))
                        }
                        "setBlocking" => {
                            let Value::Bool(blocking) = args[1] else {
                                unreachable!()
                            };
                            handle.blocking.set(blocking);
                            Ok(Value::Null)
                        }
                        "setTimeout" => {
                            let n = integer(&args[1]);
                            if n <= 0 || n as u128 > config.timeout.as_millis() {
                                return Err(error("timeout exceeds host socket ceiling"));
                            }
                            handle.timeout.set(Duration::from_millis(n as u64));
                            Ok(Value::Null)
                        }
                        "localAddress" | "peerAddress" => {
                            endpoint_budget(context)?;
                            let state = handle.state.borrow();
                            let state = state.as_ref().unwrap();
                            let address = match &state.backend {
                                Backend::Tcp(s) => {
                                    if op == "localAddress" {
                                        s.local_addr()
                                    } else {
                                        s.peer_addr()
                                    }
                                }
                                Backend::Udp(s) => {
                                    if op == "localAddress" {
                                        s.local_addr()
                                    } else {
                                        s.peer_addr()
                                    }
                                }
                                Backend::Listener(s) if op == "localAddress" => s.local_addr(),
                                _ => return Err(error("socket has no peer address")),
                            }
                            .map_err(io_error)?;
                            Ok(endpoint_value(address))
                        }
                        "shutdown" => {
                            let how = match text(&args[1]) {
                                "read" => Shutdown::Read,
                                "write" => Shutdown::Write,
                                "both" => Shutdown::Both,
                                _ => return Err(error("shutdown requires read, write or both")),
                            };
                            let state = handle.state.borrow();
                            let Backend::Tcp(socket) = &state.as_ref().unwrap().backend else {
                                return Err(error("shutdown requires TCP"));
                            };
                            socket.shutdown(how).map_err(io_error)?;
                            Ok(Value::Null)
                        }
                        "setOption" => {
                            option(context, handle, text(&args[1]), Some(&args[2]))?;
                            Ok(Value::Null)
                        }
                        "getOption" => option(context, handle, text(&args[1]), None),
                        "sendTo" => {
                            context.permissions.require("network.connect")?;
                            let Value::Bytes(data) = &args[1] else {
                                unreachable!()
                            };
                            let address = address(text(&args[2]), integer(&args[3]), false)?;
                            let mut state = handle.state.borrow_mut();
                            let state = state.as_mut().unwrap();
                            if data.len() > 65507
                                || data.len() > config.max_chunk_bytes
                                || data.len() as u64 > handle.ceiling.saturating_sub(state.used)
                            {
                                return Err(limit());
                            }
                            let Backend::Udp(socket) = &mut state.backend else {
                                return Err(error("sendTo requires UDP"));
                            };
                            if socket.peer_addr().is_ok() {
                                return Err(error(
                                    "sendTo requires an unconnected bound UDP socket",
                                ));
                            }
                            let n = io(
                                context,
                                handle.timeout.get(),
                                handle.blocking.get(),
                                socket,
                                Interest::WRITABLE,
                                |s| s.send_to(data, address),
                            )?;
                            if n != data.len() {
                                return Err(error("partial datagram send"));
                            }
                            state.used += n as u64;
                            Ok(Value::Int(n as i64))
                        }
                        _ => unreachable!(),
                    }
                }
            }),
        )?;
    }
    Ok(())
}
struct Watched {
    value: Value,
    source: Box<dyn MioSource>,
    read: bool,
    write: bool,
}
fn poll_handles(
    context: &RuntimeContext<'_>,
    args: &[Value],
    config: &StreamConfig,
) -> NativeResult<Value> {
    let [Value::List(read), Value::List(write), Value::Int(ms)] = args else {
        unreachable!()
    };
    if read.len() > config.max_handles
        || write.len() > config.max_handles
        || *ms < 0
        || *ms as u128 > config.timeout.as_millis()
    {
        return Err(error("poll count or timeout exceeds host limits"));
    }
    if read.is_empty() && write.is_empty() {
        return Err(error("poll requires at least one socket"));
    }
    let mut watched: Vec<Watched> = vec![];
    let mut cost = 3 * size_of::<Value>() + 2 * size_of::<MapKey>() + 16;
    for (values, reading) in [(read, true), (write, false)] {
        for value in values {
            context.check_cancelled()?;
            let handle = resource(context, value)?;
            check(context, handle)?;
            if !reading && handle.kind == Kind::Listener {
                return Err(error("listeners cannot be polled for writing"));
            }
            if let Some(existing)=watched.iter_mut().find(|w|matches!((&w.value,value),(Value::NativeObject(a),Value::NativeObject(b)) if Rc::ptr_eq(a,b))) {if reading {existing.read=true}else{existing.write=true}continue}
            let state = handle.state.borrow();
            let source = match &state.as_ref().unwrap().backend {
                Backend::Tcp(s) => s.source(),
                Backend::Udp(s) => s.source(),
                Backend::Listener(s) => s.source(),
                _ => return Err(error("poll requires sockets")),
            }
            .map_err(io_error)?;
            bytes::grow(
                context,
                &mut cost,
                2 * (size_of::<Value>() + handle.payload_bytes()),
            )?;
            watched.push(Watched {
                value: value.clone(),
                source,
                read: reading,
                write: !reading,
            });
        }
    }
    bytes::budget(context, cost)?;
    let mut poll = Poll::new().map_err(io_error)?;
    for (index, item) in watched.iter_mut().enumerate() {
        let interest = match (item.read, item.write) {
            (true, true) => Interest::READABLE | Interest::WRITABLE,
            (true, false) => Interest::READABLE,
            _ => Interest::WRITABLE,
        };
        poll.registry()
            .register(&mut *item.source, Token(index), interest)
            .map_err(io_error)?;
    }
    let mut events = Events::with_capacity(watched.len().max(1));
    let deadline = Instant::now() + Duration::from_millis(*ms as u64);
    let mut readable = vec![false; watched.len()];
    let mut writable = readable.clone();
    loop {
        context.check_cancelled()?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        match poll.poll(&mut events, Some(remaining.min(Duration::from_millis(50)))) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            other => other.map_err(io_error)?,
        };
        for event in &events {
            let index = event.token().0;
            if index < watched.len() {
                readable[index] |=
                    event.is_readable() || event.is_read_closed() || event.is_error();
                writable[index] |=
                    event.is_writable() || event.is_write_closed() || event.is_error();
            }
        }
        if readable.iter().chain(&writable).any(|ready| *ready) || Instant::now() >= deadline {
            break;
        }
    }
    let mut results = vec![];
    for (values, ready) in [(read, &readable), (write, &writable)] {
        let mut seen = std::collections::BTreeSet::new();
        let mut result = vec![];
        for value in values {
            context.check_cancelled()?;
            let Value::NativeObject(object) = value else {
                unreachable!()
            };
            let key = Rc::as_ptr(object) as *const () as usize;
            if !seen.insert(key) {
                continue;
            }
            if watched.iter().enumerate().any(|(i, w)| {
                ready[i]
                    && matches!(&w.value,Value::NativeObject(other) if Rc::ptr_eq(other,object))
            }) {
                result.push(value.clone())
            }
        }
        results.push(Value::List(result));
    }
    Ok(Value::Map(vec![
        (MapKey::String("readable".into()), results.remove(0)),
        (MapKey::String("writable".into()), results.remove(0)),
    ]))
}
