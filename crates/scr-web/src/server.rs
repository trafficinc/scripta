use crate::worker::{BODY_CEILING, HEADER_LIMIT, META_LIMIT, RequestMeta, ResponseMeta};
use http_body_util::{BodyExt, Full};
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
            sessions: scr_stdlib::web::session::Config::new(PathBuf::new()),
        }
    }
    fn validate(&self) -> io::Result<()> {
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
    deadline: Deadline,
    reply: oneshot::Sender<Result<(ResponseMeta, Vec<u8>), u16>>,
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
    async fn request(&mut self, job: &Job, config: &Config) -> io::Result<(ResponseMeta, Vec<u8>)> {
        write_frame(&mut self.input, &serde_json::to_vec(&job.metadata)?).await?;
        write_frame(&mut self.input, &job.body).await?;
        let metadata = read_frame(&mut self.output, META_LIMIT).await?;
        let metadata = serde_json::from_slice(&metadata)?;
        let body = read_frame(&mut self.output, config.max_response_bytes.max(64)).await?;
        Ok((metadata, body))
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
        let Some(job) = queue.lock().await.recv().await else {
            break;
        };
        if job.reply.is_closed() || Deadline::now() >= job.deadline {
            let _ = job.reply.send(Err(504));
            continue;
        }
        let result = timeout_at(job.deadline, async {
            if worker.is_none() {
                worker = Some(Worker::start(&config).await?);
            }
            worker.as_mut().unwrap().request(&job, &config).await
        })
        .await;
        let response = match result {
            Ok(Ok(response)) => Ok(response),
            error => {
                let status = if error.is_err() { 504 } else { 502 };
                eprintln!("web worker failed; replacing worker (HTTP {status})");
                if let Some(worker) = worker.take() {
                    worker.stop().await;
                }
                Err(status)
            }
        };
        let _ = job.reply.send(response);
    }
    if let Some(worker) = worker {
        worker.stop().await;
    }
}
type HttpResponse = Response<Full<Bytes>>;
fn plain(status: u16, text: &'static str) -> HttpResponse {
    let mut response = Response::new(Full::new(Bytes::from_static(text.as_bytes())));
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
) -> HttpResponse {
    let started = Instant::now();
    let head = request.method() == hyper::Method::HEAD;
    let deadline = Deadline::now() + config.request_timeout;
    let response = match timeout_at(deadline, handle(request, config, queue, deadline)).await {
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
        use hyper::body::Body;
        if let Some(length) = body.size_hint().exact() {
            parts
                .headers
                .insert("content-length", HeaderValue::from(length));
        }
        return Response::from_parts(parts, Full::new(Bytes::new()));
    }
    Response::from_parts(parts, body)
}
async fn handle(
    request: Request<Incoming>,
    config: Arc<Config>,
    queue: mpsc::Sender<Job>,
    deadline: Deadline,
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
    let mut body = Vec::new();
    while let Some(frame) = incoming.frame().await {
        let Ok(frame) = frame else {
            return plain(400, "Bad Request\n");
        };
        if let Ok(data) = frame.into_data() {
            if data.len() > config.max_body_bytes.saturating_sub(body.len()) {
                return plain(413, "Content Too Large\n");
            }
            body.extend_from_slice(&data);
        } else {
            return plain(400, "Request trailers are not supported\n");
        }
    }
    let (reply, answer) = oneshot::channel();
    permit.send(Job {
        metadata: RequestMeta {
            method: parts.method.to_string(),
            path: parts.uri.path().into(),
            query: parts.uri.query().unwrap_or("").into(),
            headers,
        },
        body,
        deadline,
        reply,
    });
    let (metadata, body) = match answer.await {
        Ok(Ok(response)) => response,
        Ok(Err(504)) => return plain(504, "Gateway Timeout\n"),
        _ => return plain(502, "Bad Gateway\n"),
    };
    let Ok(status) = StatusCode::from_u16(metadata.status) else {
        return plain(502, "Bad Gateway\n");
    };
    let mut response = Response::new(Full::new(if matches!(metadata.status, 204 | 205 | 304) {
        Bytes::new()
    } else {
        Bytes::from(body)
    }));
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
                connections.spawn(async move {
                    let _slot = slot;
                    let service = service_fn(move |request| {
                        let config = config.clone();
                        let queue = queue.clone();
                        async move { Ok::<_, Infallible>(dispatch(request, config, queue).await) }
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
    Ok(())
}
