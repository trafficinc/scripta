use crate::response::{Body, Cancel, CancelGuard};
use crate::uploads::{self, Budget, Spool};
use crate::worker::{
    BODY_CEILING, CHUNK_LIMIT, ChildMessage, HEADER_LIMIT, META_LIMIT, ParentMessage, RequestMeta,
    ResponseMeta,
};

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
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{Mutex, Semaphore, mpsc, oneshot},
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
    pub queue: usize,
    pub max_connections: usize,
    pub max_body_bytes: usize,
    pub max_response_bytes: usize,
    pub request_timeout: Duration,
    pub uploads: crate::UploadConfig,
    pub sessions: scr_stdlib::web::session::Config,
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
            queue: 32,
            max_connections: 128,
            max_body_bytes: 1024 * 1024,
            max_response_bytes: 8 * 1024 * 1024,
            request_timeout: Duration::from_secs(10),
            uploads: crate::UploadConfig::default(),
            sessions: scr_stdlib::web::session::Config::new(PathBuf::new()),
        }
    }
    fn validate(&self) -> io::Result<()> {
        self.uploads.validate()?;
        if !(1..=64).contains(&self.workers)
            || !(1..=4096).contains(&self.queue)
            || !(1..=4096).contains(&self.max_connections)
            || self.max_body_bytes > BODY_CEILING
            || !(1..=BODY_CEILING).contains(&self.max_response_bytes)
            || self.request_timeout.is_zero()
            || self.request_timeout > Duration::from_secs(3600)
        {
            return Err(io::Error::other("invalid web server limits"));
        }
        Ok(())
    }
}
struct Job {
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
    async fn start(config: &Config) -> io::Result<Self> {
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
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        // Console termination reaches only the supervisor, which drains requests
        // before stopping its workers.
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(windows)]
        command.creation_flags(0x0000_0200); // CREATE_NEW_PROCESS_GROUP
        let mut child = command.spawn()?;
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let mut worker = Self {
            child,
            input,
            output,
        };
        write_frame(&mut worker.input, config.source.as_bytes()).await?;
        if read_frame(&mut worker.output, 16).await? != b"ready" {
            return Err(io::Error::other("worker initialization failed"));
        }
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
async fn worker_loop(
    mut worker: Option<Worker>,
    config: Arc<Config>,
    queue: Arc<Mutex<mpsc::Receiver<Job>>>,
) {
    loop {
        let Some(mut job) = queue.lock().await.recv().await else {
            break;
        };
        if job.reply.as_ref().is_none_or(|reply| reply.is_closed())
            || Deadline::now() >= job.deadline
        {
            if let Some(reply) = job.reply.take() {
                let _ = reply.send(Err(504));
            }
            continue;
        }
        let cancel = job.cancel.clone();
        let result = tokio::select! {
            result=timeout_at(job.deadline,async {
                if worker.is_none() {worker=Some(Worker::start(&config).await?);}
                worker.as_mut().unwrap().request(&mut job,&config).await
            })=>result,
            ()=cancel.wait()=>Ok(Err(io::Error::other("client disconnected"))),
        };
        if !matches!(result, Ok(Ok(()))) {
            let status = if result.is_err() { 504 } else { 502 };
            if job.reply.is_some() {
                eprintln!("web worker failed; replacing worker (HTTP {status})");
            } else {
                eprintln!(
                    "web response failed after headers committed; ending response and replacing worker"
                );
            }
            job.failed.store(true, Ordering::Release);
            if let Some(worker) = worker.take() {
                worker.stop().await;
            }
            if let Some(reply) = job.reply.take() {
                let _ = reply.send(Err(status));
            }
        }
    }
    if let Some(worker) = worker {
        worker.stop().await;
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
async fn dispatch(
    request: Request<Incoming>,
    config: Arc<Config>,
    queue: mpsc::Sender<Job>,
    budget: Arc<Budget>,
) -> HttpResponse {
    let started = Instant::now();
    let head = request.method() == hyper::Method::HEAD;
    let deadline = Deadline::now() + config.request_timeout;
    let response =
        match timeout_at(deadline, handle(request, config, queue, deadline, budget)).await {
            Ok(response) => response,
            Err(_) => plain(504, "Gateway Timeout\n"),
        };
    let status = response.status();
    eprintln!(
        "web status={} durationMs={:.3}",
        status.as_u16(),
        started.elapsed().as_secs_f64() * 1000.0
    );
    let (mut parts, body) = response.into_parts();
    if head {
        use hyper::body::Body as _;
        if let Some(length) = body.size_hint().exact() {
            parts
                .headers
                .insert("content-length", HeaderValue::from(length));
        }
        return Response::from_parts(parts, Body::full(Bytes::new()));
    }
    Response::from_parts(parts, body)
}
async fn handle(
    request: Request<Incoming>,
    config: Arc<Config>,
    queue: mpsc::Sender<Job>,
    deadline: Deadline,
    budget: Arc<Budget>,
) -> HttpResponse {
    let (parts, mut incoming) = request.into_parts();
    if parts.method == hyper::Method::CONNECT
        || parts.uri.scheme().is_some()
        || parts.uri.authority().is_some()
        || !parts.uri.path().starts_with('/')
    {
        return plain(400, "Bad Request\n");
    }
    let hosts: Vec<_> = parts.headers.get_all("host").iter().collect();
    if (parts.version == hyper::Version::HTTP_11 && hosts.len() != 1)
        || hosts.len() > 1
        || hosts.first().is_some_and(|host| {
            host.to_str()
                .ok()
                .and_then(|s| s.parse::<hyper::http::uri::Authority>().ok())
                .is_none_or(|authority| authority.as_str().contains('@'))
        })
    {
        return plain(400, "Bad Request\n");
    }
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
        return plain(503, "Service Unavailable\n");
    };
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
        metadata: RequestMeta {
            method: parts.method.to_string(),
            path: parts.uri.path().into(),
            query: parts.uri.query().unwrap_or("").into(),
            headers,
            multipart,
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
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
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
    let budget = Budget::new(config.uploads.max_disk_bytes);
    let config = Arc::new(config);
    let listener = TcpListener::bind(SocketAddr::new(config.host, config.port)).await?;
    let (queue, receiver) = mpsc::channel(config.queue);
    let receiver = Arc::new(Mutex::new(receiver));
    let mut workers = JoinSet::new();
    for _ in 0..config.workers {
        if shutdown.load(Ordering::Relaxed) {
            return Ok(());
        }
        let worker = timeout(Duration::from_secs(30), Worker::start(&config)).await??;
        workers.spawn(worker_loop(Some(worker), config.clone(), receiver.clone()));
    }
    eprintln!(
        "Scripta listening on http://{} ({} workers)",
        listener.local_addr()?,
        config.workers
    );
    let slots = Arc::new(Semaphore::new(config.max_connections));
    let mut connections = JoinSet::new();
    while !shutdown.load(Ordering::Relaxed) {
        while connections.try_join_next().is_some() {}
        match timeout(Duration::from_millis(100), listener.accept()).await {
            Err(_) => continue,
            Ok(Err(error)) => return Err(error.into()),
            Ok(Ok((stream, _))) => {
                let Ok(slot) = slots.clone().try_acquire_owned() else {
                    drop(stream);
                    continue;
                };
                let config = config.clone();
                let queue = queue.clone();
                let budget = budget.clone();
                connections.spawn(async move {
                    let _slot = slot;
                    let service = service_fn(move |request| {
                        let config = config.clone();
                        let queue = queue.clone();
                        let budget = budget.clone();
                        async move {
                            Ok::<_, Infallible>(dispatch(request, config, queue, budget).await)
                        }
                    });
                    let mut builder = http1::Builder::new();
                    builder
                        .timer(TokioTimer::new())
                        .header_read_timeout(Duration::from_secs(10))
                        .max_buf_size(HEADER_LIMIT)
                        .max_headers(100);
                    let connection = builder.serve_connection(TokioIo::new(stream), service);
                    // Also bound slow response readers and idle persistent connections.
                    let _ = timeout(Duration::from_secs(60), connection).await;
                });
            }
        }
    }
    // Stop accepting requests; close keep-alive connections after active work drains.
    let drain = async {
        while let Some(result) = connections.join_next().await {
            let _ = result;
        }
    };
    let _ = timeout(config.request_timeout + Duration::from_secs(1), drain).await;
    connections.abort_all();
    while connections.join_next().await.is_some() {}
    drop(queue);
    let _ = timeout(config.request_timeout + Duration::from_secs(1), async {
        while workers.join_next().await.is_some() {}
    })
    .await;
    workers.abort_all();
    while workers.join_next().await.is_some() {}
    drop(temporary_uploads);
    Ok(())
}
