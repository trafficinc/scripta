use crate::observe::{Gauge, GaugeKind, Observer, RequestGuard, Trace, add};
use crate::response::{Body, Cancel, CancelGuard};
use crate::uploads::{self, Budget, Spool};
use crate::worker::{
    BODY_CEILING, CHUNK_LIMIT, ChildMessage, HEADER_LIMIT, META_LIMIT, ParentMessage, RequestMeta,
    ResponseMeta,
};
use serde_json::json;

use hyper::{
    Request, Response, StatusCode,
    body::{Bytes, Incoming},
    header::{HeaderName, HeaderValue},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use std::{
    convert::Infallible,
    io,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{Mutex, Semaphore, mpsc, oneshot, watch},
    task::JoinSet,
    time::{Instant as Deadline, timeout, timeout_at},
};

/// Host limits apply independently of per-script runtime limits.
#[derive(Clone, Debug)]
pub struct Config {
    pub path: PathBuf,
    pub source: Arc<String>,
    pub host: IpAddr,
    pub port: u16,
    pub workers: usize,
    pub worker_max_requests: u64,
    pub watch: bool,
    pub watch_debounce: Duration,
    pub(crate) generation: Option<Arc<crate::dev::Snapshot>>,
    pub queue: usize,
    pub max_connections: usize,
    pub max_body_bytes: usize,
    pub max_response_bytes: usize,
    pub request_timeout: Duration,
    pub connection_timeout: Duration,
    pub header_timeout: Duration,
    pub drain_timeout: Duration,
    pub uploads: crate::UploadConfig,
    pub sessions: scr_stdlib::web::session::Config,
    pub trusted_proxies: Vec<String>,
    pub allowed_hosts: Vec<String>,
    pub log_format: crate::LogFormat,
    pub health_address: Option<SocketAddr>,
}
impl Config {
    pub fn new(path: PathBuf, source: String) -> Self {
        Self {
            path,
            source: Arc::new(source),
            host: IpAddr::from([127, 0, 0, 1]),
            port: 8080,
            workers: std::thread::available_parallelism()
                .map_or(2, usize::from)
                .min(4),
            worker_max_requests: 0,
            watch: false,
            watch_debounce: Duration::from_millis(200),
            generation: None,
            queue: 32,
            max_connections: 128,
            max_body_bytes: 1024 * 1024,
            max_response_bytes: 8 * 1024 * 1024,
            request_timeout: Duration::from_secs(10),
            connection_timeout: Duration::from_secs(60),
            header_timeout: Duration::from_secs(10),
            drain_timeout: Duration::from_secs(11),
            uploads: crate::UploadConfig::default(),
            sessions: scr_stdlib::web::session::Config::new(PathBuf::new()),
            trusted_proxies: vec![],
            allowed_hosts: vec![],
            log_format: crate::LogFormat::Text,
            health_address: None,
        }
    }
    /// Validate limits and storage paths without creating directories or binding sockets.
    pub fn validate(&self) -> io::Result<()> {
        crate::proxy::Policy::new(&self.trusted_proxies, &self.allowed_hosts)?;
        if let Some(address) = self.health_address {
            if !address.ip().is_loopback() {
                return Err(io::Error::other(
                    "health-address must bind an explicit loopback IP",
                ));
            }
            if address.port() != 0 && address == SocketAddr::new(self.host, self.port) {
                return Err(io::Error::other(
                    "health-address conflicts with the application listener",
                ));
            }
        }
        if self.worker_max_requests > 1_000_000 {
            return Err(io::Error::other("worker-max-requests must be 0..1000000"));
        }
        crate::dev::validate(self)?;
        self.uploads.validate()?;
        self.sessions
            .validate()
            .map_err(|_| io::Error::other("invalid session host limits"))?;
        validate_directory(&self.sessions.directory, "session", true)?;
        validate_directory(&self.uploads.directory, "upload", false)?;
        for (name, value) in [
            ("connection-timeout-ms", self.connection_timeout),
            ("header-timeout-ms", self.header_timeout),
            ("drain-timeout-ms", self.drain_timeout),
        ] {
            if value.is_zero() || value > Duration::from_secs(3600) {
                return Err(io::Error::other(format!("{name} must be 1..3600000")));
            }
        }
        for (name, value, min, max) in [
            ("workers", self.workers, 1, 64),
            ("queue", self.queue, 1, 4096),
            ("max-connections", self.max_connections, 1, 4096),
            ("max-body-bytes", self.max_body_bytes, 0, BODY_CEILING),
            (
                "max-response-bytes",
                self.max_response_bytes,
                1,
                BODY_CEILING,
            ),
        ] {
            if !(min..=max).contains(&value) {
                return Err(io::Error::other(format!("{name} must be {min}..{max}")));
            }
        }
        if self.request_timeout.is_zero() || self.request_timeout > Duration::from_secs(3600) {
            return Err(io::Error::other("timeout-ms must be 1..3600000"));
        }
        Ok(())
    }
}
// Deliberately read-only: startup prepares storage after configuration inspection.
fn validate_directory(path: &std::path::Path, name: &str, recursive: bool) -> io::Result<()> {
    if path.as_os_str().is_empty() {
        return Ok(());
    }
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(io::Error::other(format!(
                    "{name} directory must be a real directory"
                )));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o077 != 0 {
                    return Err(io::Error::other(format!(
                        "{name} directory must have private Unix permissions (0700)"
                    )));
                }
            }
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(std::path::Path::new("."));
            if parent.is_dir() {
                Ok(())
            } else if recursive && parent != path {
                // Missing session ancestors may be created by prepare(), but an
                // existing non-directory ancestor must fail even in check mode.
                match std::fs::metadata(parent) {
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {
                        validate_directory(parent, name, true)
                    }
                    _ => Err(io::Error::other(format!("invalid {name} directory parent"))),
                }
            } else {
                Err(io::Error::other(format!("invalid {name} directory parent")))
            }
        }
        Err(_) => Err(io::Error::other(format!("cannot inspect {name} directory"))),
    }
}
#[derive(Clone)]
struct Host {
    config: Arc<Config>,
    proxy: Arc<crate::proxy::Policy>,
    queue: mpsc::Sender<Job>,
    budget: Arc<Budget>,
    observer: Arc<Observer>,
}
struct Job {
    trace: Arc<Trace>,
    queued: Option<Gauge>,
    logs: usize,
    application_error: bool,
    metadata: RequestMeta,
    body: Vec<u8>,
    spool: Option<Spool>,
    deadline: Deadline,
    reply: Option<oneshot::Sender<Result<(ResponseMeta, Body), u16>>>,
    stream: Option<mpsc::Sender<Bytes>>,
    failed: Arc<AtomicBool>,
    cancel: Arc<Cancel>,
}
struct Worker {
    _lease: Gauge,
    child: Child,
    input: ChildStdin,
    output: ChildStdout,
}
async fn write_frame(output: &mut ChildStdin, bytes: &[u8]) -> io::Result<()> {
    let size = u32::try_from(bytes.len()).map_err(io::Error::other)?;
    output.write_all(&size.to_be_bytes()).await?;
    output.write_all(bytes).await?;
    output.flush().await
}
async fn read_frame(input: &mut ChildStdout, limit: usize) -> io::Result<Vec<u8>> {
    let size = input.read_u32().await? as usize;
    if size > limit {
        return Err(io::Error::other("worker response exceeds frame limit"));
    }
    let mut bytes = vec![0; size];
    input.read_exact(&mut bytes).await?;
    Ok(bytes)
}
impl Worker {
    async fn start(config: &Config, observer: &Arc<Observer>) -> io::Result<Self> {
        let mut command = Command::new(std::env::current_exe()?);
        command
            .arg("__web-worker")
            .arg(&config.path)
            .arg(config.max_response_bytes.to_string())
            .arg(&config.sessions.directory)
            .arg(config.sessions.ttl.as_secs().to_string())
            .arg(config.sessions.max_sessions.to_string())
            .arg(config.sessions.max_bytes.to_string())
            .arg(config.sessions.secure.to_string())
            .arg(&config.uploads.directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if let Some(generation) = &config.generation {
            command.arg(generation.root());
        }
        // Console termination reaches only the supervisor, which drains requests
        // before stopping its workers.
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(windows)]
        command.creation_flags(0x0000_0200); // CREATE_NEW_PROCESS_GROUP
        let mut child = command.spawn()?;
        let mut input = child.stdin.take().unwrap();
        let mut output = child.stdout.take().unwrap();
        // A lease counts only workers which completed their ready handshake.
        let handshake = async {
            write_frame(&mut input, config.source.as_bytes()).await?;
            if read_frame(&mut output, 16).await? != b"ready" {
                return Err(io::Error::other("worker initialization failed"));
            }
            if observer.stopping() {
                return Err(io::Error::other("server draining"));
            }
            Ok(())
        }
        .await;
        if let Err(error) = handshake {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(error);
        }
        add(&observer.stats.worker_starts, 1);
        observer.event(
            "worker_started",
            None,
            json!({"workerPid":child.id()}),
            "worker started",
        );
        let worker = Self {
            _lease: Gauge::new(observer, GaugeKind::Worker),
            child,
            input,
            output,
        };
        Ok(worker)
    }
    async fn request(&mut self, job: &mut Job, config: &Config) -> io::Result<()> {
        #[derive(serde::Serialize)]
        struct Message<'a> {
            kind: &'static str,
            metadata: &'a RequestMeta,
        }
        let metadata = serde_json::to_vec(&Message {
            kind: "Request",
            metadata: &job.metadata,
        })?;
        if metadata.len() > META_LIMIT {
            return Err(io::Error::other("request metadata exceeds limit"));
        }
        write_frame(&mut self.input, &metadata).await?;
        write_frame(&mut self.input, &job.body).await?;
        let mut started = false;
        let mut total = 0usize;
        loop {
            let message = read_frame(&mut self.output, META_LIMIT).await?;
            match serde_json::from_slice::<ChildMessage>(&message)? {
                ChildMessage::Diagnostic {
                    code,
                    start,
                    end,
                    category,
                } => {
                    if code.len() > 16
                        || !code
                            .bytes()
                            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
                        || !matches!(category.as_str(), "script" | "session" | "output" | "exit")
                    {
                        return Err(io::Error::other("invalid diagnostic frame"));
                    }
                    job.application_error = true;
                    job.trace.failure("application_error");
                    job.trace.event("request_diagnostic",json!({"code":code,"spanStart":start,"spanEnd":end,"category":category,"workerPid":self.child.id()}),"request diagnostic");
                }
                ChildMessage::ApplicationLog { message, truncated } => {
                    if message.len() > 4100 {
                        return Err(io::Error::other("application log frame exceeds limit"));
                    }
                    if job.logs < 16 {
                        job.logs += 1;
                        job.trace.event("application_log",json!({"message":message,"truncated":truncated,"workerPid":self.child.id()}),"application log");
                    }
                }
                ChildMessage::Response { metadata } => {
                    if started {
                        return Err(io::Error::other("duplicate response"));
                    }
                    let body =
                        read_frame(&mut self.output, config.max_response_bytes.max(64)).await?;
                    let reply = job
                        .reply
                        .take()
                        .ok_or_else(|| io::Error::other("missing response receiver"))?;
                    reply
                        .send(Ok((metadata, Body::full(body))))
                        .map_err(|_| io::Error::other("client disconnected"))?;
                    return Ok(());
                }
                ChildMessage::Start { metadata } => {
                    if started {
                        return Err(io::Error::other("duplicate response start"));
                    }
                    started = true;
                    let body = if job.metadata.method == "HEAD"
                        || matches!(metadata.status, 204 | 205 | 304)
                    {
                        Body::Suppressed
                    } else {
                        let (send, receive) = mpsc::channel(1);
                        job.stream = Some(send);
                        Body::stream(receive, job.failed.clone(), job.cancel.clone())
                    };
                    job.reply
                        .take()
                        .ok_or_else(|| io::Error::other("missing response receiver"))?
                        .send(Ok((metadata, body)))
                        .map_err(|_| io::Error::other("client disconnected"))?;
                }
                ChildMessage::Chunk => {
                    if !started {
                        return Err(io::Error::other("body chunk before headers"));
                    }
                    let bytes = read_frame(&mut self.output, CHUNK_LIMIT).await?;
                    total = total
                        .checked_add(bytes.len())
                        .filter(|n| *n <= config.max_response_bytes)
                        .ok_or_else(|| io::Error::other("response output limit exceeded"))?;
                    if let Some(send) = &job.stream {
                        send.send(Bytes::from(bytes))
                            .await
                            .map_err(|_| io::Error::other("client disconnected"))?;
                    }
                }
                ChildMessage::Flush => {
                    if !started {
                        return Err(io::Error::other("flush before headers"));
                    }
                    write_frame(
                        &mut self.input,
                        &serde_json::to_vec(&ParentMessage::FlushAck)?,
                    )
                    .await?;
                }
                ChildMessage::Done { success } => {
                    if !started || !success {
                        return Err(io::Error::other("streamed request failed"));
                    }
                    return Ok(());
                }
                ChildMessage::StageRequest { index, parent } => {
                    let path = parent
                        .path()
                        .ok()
                        .and_then(|parent| {
                            job.spool
                                .as_mut()?
                                .stage(index, job.metadata.multipart.as_ref()?, &parent)
                                .ok()
                        })
                        .map(|p| uploads::WirePath::new(&p));
                    write_frame(
                        &mut self.input,
                        &serde_json::to_vec(&ParentMessage::StageReply { path })?,
                    )
                    .await?;
                }
            }
        }
    }
    async fn stop(mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
}
// The handshake alone does not reset failure history: a process which keeps
// dying immediately must not turn successful handshakes into a restart storm.
#[derive(Default)]
struct Backoff {
    failures: u32,
}
impl Backoff {
    fn next(&mut self) -> Duration {
        let millis = 100u64
            .saturating_mul(1u64 << self.failures.min(6))
            .min(5000);
        self.failures = self.failures.saturating_add(1);
        Duration::from_millis(millis)
    }
    fn reset(&mut self) {
        self.failures = 0;
    }
}
async fn wait_shutdown(observer: &Observer) {
    while !observer.stopping() {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
async fn worker_loop(
    mut worker: Option<Worker>,
    config: Arc<Config>,
    queue: Arc<Mutex<mpsc::Receiver<Job>>>,
    observer: Arc<Observer>,
    mut force_stop: watch::Receiver<bool>,
) {
    let mut backoff = Backoff::default();
    let mut completed = 0u64;
    let mut healthy_since = Deadline::now();
    loop {
        if *force_stop.borrow() {
            break;
        }
        if worker.is_none() {
            if observer.stopping() {
                break;
            }
            let _recovering = Gauge::new(&observer, GaugeKind::Recovering);
            let delay = backoff.next();
            observer.event(
                "worker_recovering",
                None,
                json!({"delayMs":delay.as_millis() as u64}),
                "worker recovery scheduled",
            );
            let result = tokio::select! {
                biased;
                _ = force_stop.changed() => break,
                () = wait_shutdown(&observer) => break,
                result = async {
                    tokio::time::sleep(delay).await;
                    timeout(Duration::from_secs(30), Worker::start(&config, &observer)).await
                } => result,
            };
            match result {
                Ok(Ok(replacement)) => {
                    worker = Some(replacement);
                    completed = 0;
                    healthy_since = Deadline::now();
                    add(&observer.stats.worker_restarts, 1);
                }
                _ => {
                    add(&observer.stats.worker_startup_failures, 1);
                    observer.event(
                        "worker_startup_failed",
                        None,
                        json!({}),
                        "worker startup failed; retrying with backoff",
                    );
                    continue;
                }
            }
        }
        enum Next {
            Stop,
            Failed,
            Job,
        }
        let current = worker.as_mut().unwrap();
        let mut incoming_job = None;
        let next = tokio::select! {
            biased;
            _ = force_stop.changed() => Next::Stop,
            _ = current.child.wait() => Next::Failed,
            _ = current.output.read_u8() => Next::Failed,
            job = async { queue.lock().await.recv().await } => { incoming_job = job; Next::Job },
        };
        if healthy_since.elapsed() >= Duration::from_secs(30) {
            backoff.reset();
        }
        let mut job = match next {
            Next::Stop => break,
            Next::Failed => {
                add(&observer.stats.worker_failures, 1);
                observer.event(
                    "worker_idle_failure",
                    None,
                    json!({"workerPid":worker.as_ref().and_then(|w|w.child.id())}),
                    "idle worker exited or sent unexpected IPC",
                );
                worker.take().unwrap().stop().await;
                continue;
            }
            Next::Job => match incoming_job {
                Some(job) => job,
                None => break,
            },
        };
        drop(job.queued.take());
        if job.reply.as_ref().is_none_or(|reply| reply.is_closed())
            || Deadline::now() >= job.deadline
        {
            job.trace.failure(if Deadline::now() >= job.deadline {
                "timeout"
            } else {
                "request_cancelled"
            });
            if let Some(reply) = job.reply.take() {
                let _ = reply.send(Err(504));
            }
            continue;
        }
        let _busy = Gauge::new(&observer, GaugeKind::Busy);
        let cancel = job.cancel.clone();
        let result = tokio::select! {
            biased;
            _=force_stop.changed()=>Ok(Err(io::Error::other("shutdown deadline"))),
            result=timeout_at(job.deadline, worker.as_mut().unwrap().request(&mut job,&config))=>result,
            ()=cancel.wait()=>Ok(Err(io::Error::other("client disconnected"))),
        };
        if !matches!(result, Ok(Ok(()))) {
            let status = if result.is_err() { 504 } else { 502 };
            let reason = if *force_stop.borrow() {
                "shutdown_deadline"
            } else if result.is_err() {
                "timeout"
            } else if cancel.is_cancelled() {
                "client_disconnect"
            } else if job.application_error {
                "application_error"
            } else {
                "worker_failure"
            };
            job.trace.failure(reason);
            add(&observer.stats.worker_failures, 1);
            job.trace.event(
                "worker_failure",
                json!({"reason":reason,"workerPid":worker.as_ref().and_then(|w|w.child.id())}),
                if job.reply.is_some() {
                    "worker failed; replacing worker"
                } else {
                    "response failed after headers committed; ending response and replacing worker"
                },
            );
            job.failed.store(true, Ordering::Release);
            if let Some(worker) = worker.take() {
                worker.stop().await;
            }
            if let Some(reply) = job.reply.take() {
                let _ = reply.send(Err(status));
            }
        } else {
            backoff.reset();
            completed = completed.saturating_add(1);
        }
        if *force_stop.borrow() {
            break;
        }
        if config.worker_max_requests != 0
            && completed >= config.worker_max_requests
            && !observer.stopping()
        {
            add(&observer.stats.worker_recycles, 1);
            observer.event(
                "worker_recycled",
                None,
                json!({"workerPid":worker.as_ref().and_then(|w|w.child.id()),"requests":completed}),
                "worker request limit reached; recycling",
            );
            if let Some(current) = worker.take() {
                current.stop().await;
            }
        }
    }
    if let Some(worker) = worker {
        worker.stop().await;
    }
}
// JoinSet failures must not silently remove a worker-loop slot. A separate
// bounded backoff also covers repeated panics in the Rust worker-loop task.
fn supervise_workers(
    workers: &mut JoinSet<()>,
    config: &Arc<Config>,
    queue: &Arc<Mutex<mpsc::Receiver<Job>>>,
    observer: &Arc<Observer>,
    force_stop: &watch::Receiver<bool>,
    backoff: &mut Backoff,
) {
    while workers.try_join_next().is_some() {
        if observer.stopping() {
            continue;
        }
        add(&observer.stats.worker_loop_failures, 1);
        let delay = backoff.next();
        observer.event(
            "worker_loop_failed",
            None,
            json!({"delayMs":delay.as_millis() as u64}),
            "worker loop terminated; restoring slot",
        );
        let config = config.clone();
        let queue = queue.clone();
        let observer = observer.clone();
        let mut force_stop = force_stop.clone();
        workers.spawn(async move {
            let recovering = Gauge::new(&observer, GaugeKind::Recovering);
            tokio::select! {
                biased;
                _ = force_stop.changed() => return,
                () = wait_shutdown(&observer) => return,
                () = tokio::time::sleep(delay) => {},
            }
            drop(recovering);
            worker_loop(None, config, queue, observer, force_stop).await;
        });
    }
}
type HttpResponse = Response<Body>;
fn plain(status: u16, text: &'static str) -> HttpResponse {
    let mut response = Response::new(Body::full(Bytes::from_static(text.as_bytes())));
    *response.status_mut() = StatusCode::from_u16(status).unwrap();
    response.headers_mut().insert(
        "content-type",
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}
async fn dispatch(request: Request<Incoming>, peer: SocketAddr, host: Host) -> HttpResponse {
    // Hyper may already have buffered a pipelined request when shutdown begins.
    // Only requests whose service began before the flag are admitted to drain.
    if host.observer.stopping() {
        let mut response = plain(503, "Server draining\n");
        response
            .headers_mut()
            .insert("connection", HeaderValue::from_static("close"));
        return response;
    }
    let Some(trace) = host.observer.request() else {
        return plain(503, "Request identifier capacity exhausted\n");
    };
    let mut guard = RequestGuard(Some(trace.clone()));
    let head = request.method() == hyper::Method::HEAD;
    let deadline = Deadline::now() + host.config.request_timeout;
    let response = match timeout_at(
        deadline,
        handle(request, peer, &host, deadline, trace.clone()),
    )
    .await
    {
        Ok(response) => response,
        Err(_) => {
            trace.failure("timeout");
            plain(504, "Gateway Timeout\n")
        }
    };
    trace.headers(response.status().as_u16());
    let (mut parts, body) = response.into_parts();
    parts
        .headers
        .insert("x-request-id", HeaderValue::from_str(&trace.id).unwrap());
    let body = if head {
        use hyper::body::Body as _;
        if let Some(length) = body.size_hint().exact() {
            parts
                .headers
                .insert("content-length", HeaderValue::from(length));
        }
        Body::full(Bytes::new())
    } else {
        body
    };
    guard.0 = None;
    Response::from_parts(parts, body.observe(trace))
}
async fn handle(
    request: Request<Incoming>,
    peer: SocketAddr,
    host: &Host,
    deadline: Deadline,
    trace: Arc<Trace>,
) -> HttpResponse {
    let config = host.config.clone();
    let proxy = host.proxy.clone();
    let queue = host.queue.clone();
    let budget = host.budget.clone();
    let (parts, mut incoming) = request.into_parts();
    if parts.method == hyper::Method::CONNECT
        || parts.uri.scheme().is_some()
        || parts.uri.authority().is_some()
        || !parts.uri.path().starts_with('/')
    {
        return plain(400, "Bad Request\n");
    }
    let connection = match proxy.resolve(
        peer,
        &parts.headers,
        parts.version == hyper::Version::HTTP_11,
    ) {
        Ok(connection) => connection,
        Err(code) => return plain(code, "Invalid request authority or proxy metadata\n"),
    };
    let mut headers = Vec::new();
    let mut header_bytes = 0usize;
    for (key, value) in &parts.headers {
        header_bytes = header_bytes.saturating_add(key.as_str().len() + value.as_bytes().len() + 4);
        if header_bytes > HEADER_LIMIT {
            return plain(431, "Request Header Fields Too Large\n");
        }
        let Ok(value) = value.to_str() else {
            return plain(400, "Bad Request\n");
        };
        headers.push((key.as_str().into(), value.into()));
    }
    if parts.uri.to_string().len() > 8192 {
        return plain(414, "URI Too Long\n");
    }
    if parts
        .headers
        .get("content-length")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.parse::<u64>().ok())
        .is_some_and(|n| n > config.max_body_bytes as u64)
    {
        return plain(413, "Content Too Large\n");
    }
    // Reserve bounded queue capacity before accepting/buffering a potentially slow body.
    let Ok(permit) = queue.try_reserve() else {
        add(&host.observer.stats.overload, 1);
        trace.failure("overload");
        return plain(503, "Service Unavailable\n");
    };
    let queued = Gauge::new(&host.observer, GaugeKind::Queue);
    let boundary = match uploads::boundary(&parts.headers) {
        Ok(b) => b,
        Err(code) => return plain(code, "Invalid multipart Content-Type\n"),
    };
    let (body, spool, multipart) = if let Some(boundary) = boundary {
        match uploads::receive(
            &mut incoming,
            &config.uploads.directory,
            budget,
            &config.uploads,
            config.max_body_bytes,
            boundary,
        )
        .await
        {
            Ok((spool, metadata)) => (vec![], Some(spool), Some(metadata)),
            Err(code) => return plain(code, "Multipart request rejected\n"),
        }
    } else {
        use hyper::body::Body as _;
        if incoming.is_end_stream() {
            (vec![], None, None)
        } else {
            match uploads::receive_raw(
                &mut incoming,
                &config.uploads.directory,
                budget,
                config.max_body_bytes,
            )
            .await
            {
                Ok((spool, metadata)) => (vec![], Some(spool), Some(metadata)),
                Err(code) => return plain(code, "Request body rejected\n"),
            }
        }
    };
    let (reply, answer) = oneshot::channel();
    let cancel = Arc::new(Cancel::default());
    let mut guard = CancelGuard(Some(cancel.clone()));
    permit.send(Job {
        trace: trace.clone(),
        queued: Some(queued),
        logs: 0,
        application_error: false,
        metadata: RequestMeta {
            method: parts.method.to_string(),
            path: parts.uri.path().into(),
            query: parts.uri.query().unwrap_or("").into(),
            headers,
            multipart,
            connection,
            request_id: trace.id.clone(),
        },
        body,
        spool,
        deadline,
        reply: Some(reply),
        stream: None,
        failed: Arc::new(AtomicBool::new(false)),
        cancel,
    });
    let (metadata, body) = match answer.await {
        Ok(Ok(response)) => response,
        Ok(Err(504)) => return plain(504, "Gateway Timeout\n"),
        _ => return plain(502, "Bad Gateway\n"),
    };
    guard.0 = None;
    let Ok(status) = StatusCode::from_u16(metadata.status) else {
        return plain(502, "Bad Gateway\n");
    };
    let mut response = Response::new(if matches!(metadata.status, 204 | 205 | 304) {
        Body::Suppressed
    } else {
        body
    });
    *response.status_mut() = status;
    for (key, value) in metadata.headers {
        let (Ok(key), Ok(value)) = (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(&value),
        ) else {
            return plain(502, "Bad Gateway\n");
        };
        response.headers_mut().append(key, value);
    }
    response
        .headers_mut()
        .entry("content-type")
        .or_insert(HeaderValue::from_static("text/html; charset=utf-8"));
    response
}
/// Run until the host shutdown flag is set. Existing requests drain for a bounded
/// interval, after which remaining child processes are killed by their owners.
pub fn serve(config: Config, shutdown: Arc<AtomicBool>) -> Result<(), Box<dyn std::error::Error>> {
    config.validate()?;
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?
        .block_on(run(config, shutdown))
}
async fn run(
    mut config: Config,
    shutdown: Arc<AtomicBool>,
) -> Result<(), Box<dyn std::error::Error>> {
    let observer = Observer::new(config.log_format, config.queue, shutdown.clone());
    observer
        .stats
        .configured_workers
        .store(config.workers as u64, Ordering::Relaxed);
    let temporary_sessions = if config.sessions.directory.as_os_str().is_empty() {
        Some(
            tempfile::Builder::new()
                .prefix("scripta-sessions-")
                .tempdir()?,
        )
    } else {
        None
    };
    if let Some(directory) = &temporary_sessions {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
        }
        config.sessions.directory = directory.path().into();
    }
    config.sessions.prepare()?;
    let temporary_uploads = if config.uploads.directory.as_os_str().is_empty() {
        let directory = tempfile::Builder::new()
            .prefix("scripta-uploads-")
            .tempdir()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
        }
        config.uploads.directory = directory.path().into();
        Some(directory)
    } else {
        None
    };
    if !config.uploads.directory.exists() {
        let builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        let builder = {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = builder;
            builder.mode(0o700);
            builder
        };
        builder.create(&config.uploads.directory)?;
    }
    let metadata = std::fs::symlink_metadata(&config.uploads.directory)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("upload directory must be a private directory without a final symlink".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("upload directory must have private permissions (0700)".into());
        }
    }
    config.uploads.directory = std::fs::canonicalize(&config.uploads.directory)?;
    let proxy = Arc::new(crate::proxy::Policy::new(
        &config.trusted_proxies,
        &config.allowed_hosts,
    )?);
    let budget = Budget::new(config.uploads.max_disk_bytes, Some(observer.clone()));
    let mut config = Arc::new(config);
    let mut listener = Some(TcpListener::bind(SocketAddr::new(config.host, config.port)).await?);
    let health_stop = Arc::new(AtomicBool::new(false));
    let health = if let Some(address) = config.health_address {
        let listener = TcpListener::bind(address).await?;
        observer.listening(listener.local_addr()?, config.workers, true);
        Some(tokio::spawn(crate::health::run(
            listener,
            observer.clone(),
            health_stop.clone(),
        )))
    } else {
        None
    };
    let mut watcher = if config.watch {
        let (snapshot, watcher) = crate::dev::start(&config, observer.clone()).await?;
        let mut next = (*config).clone();
        next.path = snapshot.entry.clone();
        next.source = Arc::new(snapshot.source.clone());
        next.generation = Some(snapshot);
        config = Arc::new(next);
        Some(watcher)
    } else {
        None
    };
    let mut prepared: Vec<Worker> = vec![];
    let mut generation_id = 0u64;
    loop {
        observer.reloading.store(false, Ordering::Release);
        let mut candidate = None;
        let (queue, receiver) = mpsc::channel(config.queue);
        let receiver = Arc::new(Mutex::new(receiver));
        let (force_stop, force_receiver) = watch::channel(false);
        let mut workers = JoinSet::new();
        for _ in 0..config.workers {
            if shutdown.load(Ordering::Relaxed) {
                return Ok(());
            }
            let worker = if let Some(mut worker) = prepared.pop() {
                worker._lease = Gauge::new(&observer, GaugeKind::Worker);
                add(&observer.stats.worker_starts, 1);
                worker
            } else {
                timeout(Duration::from_secs(30), Worker::start(&config, &observer)).await??
            };
            workers.spawn(worker_loop(
                Some(worker),
                config.clone(),
                receiver.clone(),
                observer.clone(),
                force_receiver.clone(),
            ));
        }
        observer.startup_done.store(true, Ordering::Release);
        if let Some(snapshot) = &config.generation {
            observer.event(
                if generation_id == 0 {
                    "watch_ready"
                } else {
                    "reload_ready"
                },
                None,
                json!({"generation":generation_id,"snapshotRoot":snapshot.root()}),
                "development source generation ready",
            );
        }
        observer.listening(
            listener.as_ref().unwrap().local_addr()?,
            config.workers,
            false,
        );
        let host = Host {
            config: config.clone(),
            proxy: proxy.clone(),
            queue: queue.clone(),
            budget: budget.clone(),
            observer: observer.clone(),
        };
        let slots = Arc::new(Semaphore::new(config.max_connections));
        let (draining, _) = watch::channel(false);
        let mut connections = JoinSet::new();
        let mut task_backoff = Backoff::default();
        let mut task_healthy_since = Deadline::now();
        while !shutdown.load(Ordering::Relaxed) {
            if let Some(watcher) = watcher.as_mut() {
                if watcher.changes.has_changed().unwrap_or(false) {
                    let snapshot = watcher.changes.borrow_and_update().clone();
                    if let Some(snapshot) = snapshot {
                        let mut next = (*config).clone();
                        next.path = snapshot.entry.clone();
                        next.source = Arc::new(snapshot.source.clone());
                        next.generation = Some(snapshot);
                        let staging =
                            Observer::new(config.log_format, config.queue, shutdown.clone());
                        let result = tokio::select! {
                        () = wait_shutdown(&observer) => false,
                        result = timeout(Duration::from_secs(30), async {
                            for _ in 0..next.workers {
                                if shutdown.load(Ordering::Acquire) {
                                    return Err(io::Error::other("server stopping"));
                                }
                                prepared.push(Worker::start(&next, &staging).await?);
                            }
                            Ok::<_, io::Error>(())
                        }) => matches!(result, Ok(Ok(()))),
                        };
                        if result && !shutdown.load(Ordering::Acquire) {
                            candidate = Some(Arc::new(next));
                            break;
                        }
                        while let Some(worker) = prepared.pop() {
                            worker.stop().await;
                        }
                        observer.event(
                            "reload_failed",
                            None,
                            json!({"reason":"worker_startup"}),
                            "reload worker startup failed; retaining current generation; verify private storage permissions, source limits, and --check-config",
                        );
                    }
                }
            }
            if task_healthy_since.elapsed() >= Duration::from_secs(30) {
                task_backoff.reset();
                task_healthy_since = Deadline::now();
            }
            let failures = observer.stats.worker_loop_failures.load(Ordering::Relaxed);
            supervise_workers(
                &mut workers,
                &config,
                &receiver,
                &observer,
                &force_receiver,
                &mut task_backoff,
            );
            if observer.stats.worker_loop_failures.load(Ordering::Relaxed) != failures {
                task_healthy_since = Deadline::now();
            }
            while connections.try_join_next().is_some() {}
            match timeout(
                Duration::from_millis(100),
                listener.as_ref().unwrap().accept(),
            )
            .await
            {
                Err(_) => continue,
                Ok(Err(error)) => return Err(error.into()),
                Ok(Ok((stream, peer))) => {
                    let Ok(slot) = slots.clone().try_acquire_owned() else {
                        add(&observer.stats.rejected_connections, 1);
                        drop(stream);
                        continue;
                    };
                    let host = host.clone();
                    let mut drain = draining.subscribe();
                    connections.spawn(async move {
                        let _slot = slot;
                        let connection_timeout = host.config.connection_timeout;
                        let header_timeout = host.config.header_timeout;
                        let observer = host.observer.clone();
                        let admitted = Arc::new(AtomicBool::new(false));
                        let request_admitted = admitted.clone();
                        let service = service_fn(move |request| {
                            let host = host.clone();
                            request_admitted.store(true, Ordering::Release);
                            async move { Ok::<_, Infallible>(dispatch(request, peer, host).await) }
                        });
                        let mut builder = http1::Builder::new();
                        builder
                            .timer(TokioTimer::new())
                            .header_read_timeout(header_timeout)
                            .max_buf_size(HEADER_LIMIT)
                            .max_headers(100);
                        let connection = builder.serve_connection(TokioIo::new(stream), service);
                        // Also bound slow response readers and idle persistent connections.
                        tokio::pin!(connection);
                        let lifetime = tokio::time::sleep(connection_timeout);
                        tokio::pin!(lifetime);
                        let result = tokio::select! {
                            biased;
                            _ = drain.changed() => {
                                if !admitted.load(Ordering::Acquire) {
                                    // Hyper can wait for an incomplete initial header
                                    // during graceful shutdown. No service has run here,
                                    // so dropping this connection cannot lose a response.
                                    return;
                                }
                                connection.as_mut().graceful_shutdown();
                                tokio::select! {
                                    result = &mut connection => Ok(result),
                                    _ = &mut lifetime => Err(()),
                                }
                            }
                            result = &mut connection => Ok(result),
                            _ = &mut lifetime => Err(()),
                        };
                        let failure = match result {
                            Err(_) => Some(("connection_lifetime", true)),
                            Ok(Err(error)) => Some((
                                if error.is_timeout() {
                                    "header_timeout"
                                } else {
                                    "http_connection"
                                },
                                error.is_timeout(),
                            )),
                            Ok(Ok(())) => None,
                        };
                        if let Some((reason, timed_out)) = failure {
                            add(
                                if timed_out {
                                    &observer.stats.connection_timeouts
                                } else {
                                    &observer.stats.connection_errors
                                },
                                1,
                            );
                            observer.event(
                                "connection_failure",
                                None,
                                json!({"reason":reason}),
                                "connection failed",
                            );
                        }
                    });
                }
            }
        }
        observer
            .reloading
            .store(candidate.is_some(), Ordering::Release);
        if candidate.is_some() {
            observer.startup_done.store(false, Ordering::Release);
        }
        observer.event(
            if candidate.is_some() {
                "reload_draining"
            } else {
                "server_draining"
            },
            None,
            json!({}),
            "server draining",
        );
        if candidate.is_none() {
            drop(listener.take());
        }
        let drain_deadline = Deadline::now() + config.drain_timeout;
        // Wake every connection, including idle/partial-header connections.
        draining.send_replace(true);
        let drain = async {
            while let Some(result) = connections.join_next().await {
                let _ = result;
            }
        };
        let _ = timeout_at(drain_deadline, drain).await;
        connections.abort_all();
        while connections.join_next().await.is_some() {}
        drop(host);
        drop(queue);
        let _ = timeout_at(drain_deadline, async {
            while workers.join_next().await.is_some() {}
        })
        .await;
        // Terminate and reap children explicitly before dropping queued spools or
        // temporary stores. No second request/drain allowance is granted here.
        force_stop.send_replace(true);
        while workers.join_next().await.is_some() {}
        drop(receiver);
        if let Some(next) = candidate {
            if !shutdown.load(Ordering::Acquire) {
                config = next;
                generation_id = generation_id.saturating_add(1);
                continue;
            }
        }
        while let Some(worker) = prepared.pop() {
            worker.stop().await;
        }
        break;
    }
    drop(listener);
    drop(watcher);
    health_stop.store(true, Ordering::Release);
    if let Some(health) = health {
        let _ = health.await;
    }
    observer.event(
        "server_stopped",
        None,
        observer.snapshot(),
        "server stopped",
    );
    drop(temporary_uploads);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_backoff_caps_and_resets_without_overflow() {
        let mut backoff = Backoff::default();
        let delays: Vec<_> = (0..10).map(|_| backoff.next().as_millis()).collect();
        assert_eq!(
            delays,
            [100, 200, 400, 800, 1600, 3200, 5000, 5000, 5000, 5000]
        );
        backoff.failures = u32::MAX;
        assert_eq!(backoff.next(), Duration::from_secs(5));
        backoff.reset();
        assert_eq!(backoff.next(), Duration::from_millis(100));
    }

    #[tokio::test]
    async fn unexpected_worker_loop_panic_restores_a_slot_and_shutdown_cancels_it() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let observer = Observer::new(crate::LogFormat::Json, 1, shutdown.clone());
        let config = Arc::new(Config::new("fixture.scripta".into(), "<?scripta".into()));
        let (_queue, receiver) = mpsc::channel(1);
        let queue = Arc::new(Mutex::new(receiver));
        let (force, stop) = watch::channel(false);
        let mut workers = JoinSet::new();
        let failed = workers.spawn(async { panic!("injected worker-loop failure") });
        // Leave the failed task in JoinSet for the production supervisor to reap.
        while !failed.is_finished() {
            tokio::task::yield_now().await;
        }
        supervise_workers(
            &mut workers,
            &config,
            &queue,
            &observer,
            &stop,
            &mut Backoff::default(),
        );
        assert_eq!(
            observer.stats.worker_loop_failures.load(Ordering::Relaxed),
            1
        );
        assert_eq!(workers.len(), 1);
        shutdown.store(true, Ordering::Release);
        force.send_replace(true);
        timeout(Duration::from_secs(1), async {
            while let Some(result) = workers.join_next().await {
                result.unwrap();
            }
        })
        .await
        .unwrap();
        assert_eq!(observer.stats.recovering_workers.load(Ordering::Relaxed), 0);
        assert_eq!(observer.stats.worker_starts.load(Ordering::Relaxed), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn malformed_truncated_and_out_of_order_worker_ipc_fail_and_release_capacity() {
        let cases = [
            b"\0\0\0\x01{".to_vec(),
            b"\0\0\0\x20partial".to_vec(),
            u32::MAX.to_be_bytes().to_vec(),
            {
                let frame = br#"{"kind":"Chunk"}"#;
                let mut bytes = (frame.len() as u32).to_be_bytes().to_vec();
                bytes.extend_from_slice(frame);
                bytes
            },
        ];
        for bytes in cases {
            let observer =
                Observer::new(crate::LogFormat::Json, 1, Arc::new(AtomicBool::new(false)));
            let escaped = bytes
                .iter()
                .map(|byte| format!("\\{byte:03o}"))
                .collect::<String>();
            let mut child = Command::new("/bin/sh")
                .args([
                    "-c",
                    &format!("printf '%b' '{escaped}'; exec 1>&-; read ignored"),
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .unwrap();
            let worker = Worker {
                input: child.stdin.take().unwrap(),
                output: child.stdout.take().unwrap(),
                child,
                _lease: Gauge::new(&observer, GaugeKind::Worker),
            };
            let (send, receive) = mpsc::channel(1);
            let (reply, answer) = oneshot::channel();
            let trace = observer.request().unwrap();
            send.send(Job {
                trace,
                queued: Some(Gauge::new(&observer, GaugeKind::Queue)),
                logs: 0,
                application_error: false,
                metadata: RequestMeta {
                    method: "GET".into(),
                    path: "/".into(),
                    query: String::new(),
                    headers: vec![],
                    multipart: None,
                    connection: Default::default(),
                    request_id: "test".into(),
                },
                body: vec![],
                spool: None,
                deadline: Deadline::now() + Duration::from_secs(2),
                reply: Some(reply),
                stream: None,
                failed: Arc::new(AtomicBool::new(false)),
                cancel: Arc::new(Cancel::default()),
            })
            .await
            .unwrap();
            // Exercise active IPC rather than idle detection: the fixture bytes
            // are consumed directly by the request handler with a live reply.
            let queue = Arc::new(Mutex::new(receive));
            let mut job = queue.lock().await.recv().await.unwrap();
            drop(job.queued.take());
            let mut worker = worker;
            let config = Config::new("fixture.scripta".into(), "<?scripta".into());
            assert!(
                timeout(Duration::from_secs(1), worker.request(&mut job, &config))
                    .await
                    .unwrap()
                    .is_err()
            );
            worker.stop().await;
            drop(job);
            assert!(answer.await.is_err());
            assert_eq!(observer.stats.workers.load(Ordering::Relaxed), 0);
            assert_eq!(observer.stats.queued.load(Ordering::Relaxed), 0);
        }
    }
}
