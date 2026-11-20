//! Optional loopback-only management listener; never dispatches application code.
use crate::{observe::Observer, response::Body};
use hyper::{Request, Response, body::Incoming, server::conn::http1, service::service_fn};
use hyper_util::rt::{TokioIo, TokioTimer};
use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{net::TcpListener, sync::Semaphore, task::JoinSet, time::timeout};

pub(crate) async fn run(listener: TcpListener, observer: Arc<Observer>, stop: Arc<AtomicBool>) {
    let slots = Arc::new(Semaphore::new(8));
    let mut connections = JoinSet::new();
    while !stop.load(Ordering::Acquire) {
        while connections.try_join_next().is_some() {}
        let Ok(Ok((stream, peer))) = timeout(Duration::from_millis(50), listener.accept()).await
        else {
            continue;
        };
        if !peer.ip().is_loopback() {
            continue;
        }
        let Ok(slot) = slots.clone().try_acquire_owned() else {
            continue;
        };
        let observer = observer.clone();
        connections.spawn(async move {
            let _slot = slot;
            let service = service_fn(move |request| {
                let response = handle(request, &observer);
                async { Ok::<_, Infallible>(response) }
            });
            let mut builder = http1::Builder::new();
            builder
                .timer(TokioTimer::new())
                .keep_alive(false)
                .header_read_timeout(Duration::from_secs(2))
                .max_buf_size(8192)
                .max_headers(32);
            let _ = timeout(
                Duration::from_secs(2),
                builder.serve_connection(TokioIo::new(stream), service),
            )
            .await;
        });
    }
    connections.abort_all();
    while connections.join_next().await.is_some() {}
}
fn handle(request: Request<Incoming>, observer: &Observer) -> Response<Body> {
    use hyper::body::Body as _;
    let head = request.method() == hyper::Method::HEAD;
    let (status, text) = if request.method() != hyper::Method::GET && !head {
        (405, "{\"error\":\"method not allowed\"}".into())
    } else if !request.body().is_end_stream()
        || request.uri().scheme().is_some()
        || request.uri().authority().is_some()
        || request.uri().query().is_some()
    {
        (400, "{\"error\":\"invalid health request\"}".into())
    } else {
        match request.uri().path() {
            "/livez" => (200, "{\"live\":true}".into()),
            "/readyz" => {
                let ready = observer.ready();
                (
                    if ready { 200 } else { 503 },
                    serde_json::json!({"ready":ready}).to_string(),
                )
            }
            "/metrics" => (200, observer.snapshot().to_string()),
            _ => (404, "{\"error\":\"not found\"}".into()),
        }
    };
    let mut response = Response::new(Body::full(if head { String::new() } else { text.clone() }));
    *response.status_mut() = hyper::StatusCode::from_u16(status).unwrap();
    response.headers_mut().insert(
        "content-type",
        hyper::header::HeaderValue::from_static("application/json"),
    );
    response.headers_mut().insert(
        "cache-control",
        hyper::header::HeaderValue::from_static("no-store"),
    );
    response
        .headers_mut()
        .insert("content-length", text.len().into());
    response
}
