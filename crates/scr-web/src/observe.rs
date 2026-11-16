//! Bounded host events, request lifecycle accounting, and private health snapshots.
use serde_json::{Value, json};
use std::{
    io::{self, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum LogFormat {
    #[default]
    Text,
    Json,
}

// Stable on the minimum Rust toolchain as well as the pinned compiler.
fn update(counter: &AtomicU64, operation: impl Fn(u64) -> Option<u64>) -> Result<u64, u64> {
    let mut current = counter.load(Ordering::Relaxed);
    loop {
        let Some(next) = operation(current) else {
            return Err(current);
        };
        match counter.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(previous) => return Ok(previous),
            Err(actual) => current = actual,
        }
    }
}
pub(crate) fn add(counter: &AtomicU64, amount: u64) {
    let _ = update(counter, |n| Some(n.saturating_add(amount)));
}
fn subtract(counter: &AtomicU64) {
    let _ = update(counter, |n| Some(n.saturating_sub(1)));
}
#[derive(Default)]
pub(crate) struct Stats {
    pub requests: AtomicU64,
    pub active: AtomicU64,
    pub complete: AtomicU64,
    pub failed: AtomicU64,
    pub interrupted: AtomicU64,
    pub timeouts: AtomicU64,
    pub overload: AtomicU64,
    pub server_errors: AtomicU64,
    pub bytes: AtomicU64,
    pub queued: AtomicU64,
    pub busy: AtomicU64,
    pub workers: AtomicU64,
    pub worker_failures: AtomicU64,
    pub worker_starts: AtomicU64,
    pub worker_restarts: AtomicU64,
    pub configured_workers: AtomicU64,
    pub recovering_workers: AtomicU64,
    pub worker_startup_failures: AtomicU64,
    pub worker_recycles: AtomicU64,
    pub worker_loop_failures: AtomicU64,
    pub rejected_connections: AtomicU64,
    pub connection_timeouts: AtomicU64,
    pub connection_errors: AtomicU64,
}
pub(crate) struct Observer {
    pub stats: Stats,
    pub startup_done: AtomicBool,
    pub shutdown: Arc<AtomicBool>,
    format: LogFormat,
    writer: Mutex<Box<dyn Write + Send>>,
    prefix: String,
    next: AtomicU64,
    queue_limit: usize,
}
impl Observer {
    pub fn new(format: LogFormat, queue_limit: usize, shutdown: Arc<AtomicBool>) -> Arc<Self> {
        Arc::new(Self {
            stats: Stats::default(),
            startup_done: AtomicBool::new(false),
            shutdown,
            format,
            writer: Mutex::new(Box::new(io::stderr())),
            prefix: format!(
                "{:x}-{:x}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ),
            next: AtomicU64::new(0),
            queue_limit,
        })
    }
    pub fn event(
        &self,
        event: &'static str,
        request_id: Option<&str>,
        fields: Value,
        message: &'static str,
    ) {
        let mut record = fields.as_object().cloned().unwrap_or_default();
        record.insert("event".into(), event.into());
        record.insert(
            "timestampMs".into(),
            json!(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64
            ),
        );
        if let Some(id) = request_id {
            record.insert("requestId".into(), id.into());
        }
        let mut writer = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        // One bounded synchronous record, no background queue or per-client labels.
        if self.format == LogFormat::Json {
            let _ = writeln!(writer, "{}", Value::Object(record));
        } else {
            let _ = writeln!(writer, "web {message} {}", Value::Object(record));
        }
    }
    pub fn listening(&self, address: std::net::SocketAddr, workers: usize, health: bool) {
        if self.format == LogFormat::Text {
            let mut writer = self.writer.lock().unwrap_or_else(|e| e.into_inner());
            if health {
                let _ = writeln!(writer, "Scripta health listening on http://{address}");
            } else {
                let _ = writeln!(
                    writer,
                    "Scripta listening on http://{address} ({workers} workers)"
                );
            }
        } else {
            self.event(
                if health {
                    "health_listening"
                } else {
                    "server_listening"
                },
                None,
                json!({"address":address.to_string(),"workers":workers}),
                "listening",
            );
        }
    }
    pub fn request(self: &Arc<Self>) -> Option<Arc<Trace>> {
        let next = update(&self.next, |n| n.checked_add(1)).ok()?;
        add(&self.stats.requests, 1);
        add(&self.stats.active, 1);
        Some(Arc::new(Trace {
            observer: self.clone(),
            id: format!("{}-{next:x}", self.prefix),
            started: Instant::now(),
            state: Mutex::new(RequestState::default()),
        }))
    }
    pub fn ready(&self) -> bool {
        self.startup_done.load(Ordering::Acquire)
            && !self.shutdown.load(Ordering::Acquire)
            && self.stats.workers.load(Ordering::Relaxed) > 0
            && self.stats.queued.load(Ordering::Relaxed) < self.queue_limit as u64
    }
    pub fn snapshot(&self) -> Value {
        let s = &self.stats;
        let load = |n: &AtomicU64| n.load(Ordering::Relaxed);
        json!({"ready":self.ready(),"starting":!self.startup_done.load(Ordering::Acquire),"draining":self.shutdown.load(Ordering::Acquire),
            "requests":load(&s.requests),"activeRequests":load(&s.active),"responsesComplete":load(&s.complete),
            "responsesFailed":load(&s.failed),"responsesInterrupted":load(&s.interrupted),"timeouts":load(&s.timeouts),
            "overload":load(&s.overload),"serverErrors":load(&s.server_errors),"bodyBytes":load(&s.bytes),
            "queuedRequests":load(&s.queued),"busyWorkers":load(&s.busy),"readyWorkers":load(&s.workers),
            "configuredWorkers":load(&s.configured_workers),"recoveringWorkers":load(&s.recovering_workers),
            "workerStartupFailures":load(&s.worker_startup_failures),"workerRecycles":load(&s.worker_recycles),"workerLoopFailures":load(&s.worker_loop_failures),
            "workerFailures":load(&s.worker_failures),"workerStarts":load(&s.worker_starts),"workerRestarts":load(&s.worker_restarts),
            "rejectedConnections":load(&s.rejected_connections),"connectionTimeouts":load(&s.connection_timeouts),"connectionErrors":load(&s.connection_errors)})
    }
}
#[derive(Default)]
struct RequestState {
    status: Option<u16>,
    bytes: u64,
    finished: bool,
    reason: Option<&'static str>,
    timeout: bool,
}
pub(crate) struct Trace {
    pub observer: Arc<Observer>,
    pub id: String,
    started: Instant,
    state: Mutex<RequestState>,
}
impl Trace {
    pub fn headers(&self, status: u16) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).status = Some(status);
        if status >= 500 {
            add(&self.observer.stats.server_errors, 1);
        }
        self.event(
            "response_headers",
            json!({"status":status,"durationMs":self.started.elapsed().as_secs_f64()*1000.0}),
            "response headers ready",
        );
    }
    pub fn event(&self, event: &'static str, fields: Value, message: &'static str) {
        self.observer.event(event, Some(&self.id), fields, message);
    }
    pub fn failure(&self, reason: &'static str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.reason.is_none() {
            state.reason = Some(reason);
        }
        if reason == "timeout" && !state.timeout {
            state.timeout = true;
            add(&self.observer.stats.timeouts, 1);
        }
    }
    pub fn bytes(&self, n: usize) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.bytes = state.bytes.saturating_add(n as u64);
        add(&self.observer.stats.bytes, n as u64);
    }
    pub fn finish(&self, outcome: &'static str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.finished {
            return;
        }
        state.finished = true;
        let stats = &self.observer.stats;
        subtract(&stats.active);
        match outcome {
            "complete" => add(&stats.complete, 1),
            "failed" => add(&stats.failed, 1),
            _ => add(&stats.interrupted, 1),
        };
        self.event("response_finished",json!({"status":state.status,"bodyBytes":state.bytes,"outcome":outcome,"reason":state.reason,"durationMs":self.started.elapsed().as_secs_f64()*1000.0}),"response finished");
    }
}
impl Drop for Trace {
    fn drop(&mut self) {
        self.finish("interrupted");
    }
}
pub(crate) struct RequestGuard(pub Option<Arc<Trace>>);
impl Drop for RequestGuard {
    fn drop(&mut self) {
        if let Some(trace) = &self.0 {
            trace.failure("request_cancelled");
            trace.finish("interrupted");
        }
    }
}
pub(crate) struct Gauge {
    observer: Arc<Observer>,
    kind: GaugeKind,
}
pub(crate) enum GaugeKind {
    Queue,
    Busy,
    Worker,
    Recovering,
}
impl Gauge {
    pub fn new(observer: &Arc<Observer>, kind: GaugeKind) -> Self {
        let gauge = Self {
            observer: observer.clone(),
            kind,
        };
        add(gauge.counter(), 1);
        gauge
    }
    fn counter(&self) -> &AtomicU64 {
        match self.kind {
            GaugeKind::Queue => &self.observer.stats.queued,
            GaugeKind::Busy => &self.observer.stats.busy,
            GaugeKind::Worker => &self.observer.stats.workers,
            GaugeKind::Recovering => &self.observer.stats.recovering_workers,
        }
    }
}
impl Drop for Gauge {
    fn drop(&mut self) {
        subtract(self.counter());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl Write for Capture {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    fn observer() -> (Arc<Observer>, Capture) {
        let observer = Observer::new(LogFormat::Json, 1, Arc::new(AtomicBool::new(false)));
        let capture = Capture::default();
        *observer.writer.lock().unwrap() = Box::new(capture.clone());
        (observer, capture)
    }
    #[test]
    fn ids_events_completion_and_counters_are_bounded_and_exactly_once() {
        let (observer, capture) = observer();
        let trace = observer.request().unwrap();
        assert!(trace.id.len() <= 64);
        assert!(trace.id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'));
        trace.headers(504);
        trace.failure("timeout");
        trace.failure("timeout");
        trace.bytes(3);
        trace.finish("failed");
        trace.finish("interrupted");
        drop(trace);
        assert_eq!(observer.stats.timeouts.load(Ordering::Relaxed), 1);
        assert_eq!(observer.stats.active.load(Ordering::Relaxed), 0);
        assert_eq!(observer.stats.failed.load(Ordering::Relaxed), 1);
        assert_eq!(observer.stats.interrupted.load(Ordering::Relaxed), 0);
        let lines = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        let records: Vec<Value> = lines
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["requestId"], records[1]["requestId"]);
        assert_eq!(records[1]["bodyBytes"], 3);
        observer.next.store(u64::MAX, Ordering::Relaxed);
        assert!(observer.request().is_none());
        add(&observer.stats.requests, u64::MAX);
        add(&observer.stats.requests, 1);
        assert_eq!(observer.stats.requests.load(Ordering::Relaxed), u64::MAX);
    }
    #[test]
    fn readiness_tracks_startup_capacity_overload_drain_and_raii_cleanup() {
        let (observer, _) = observer();
        assert!(!observer.ready());
        let worker = Gauge::new(&observer, GaugeKind::Worker);
        assert!(!observer.ready());
        observer.startup_done.store(true, Ordering::Release);
        assert!(observer.ready());
        let queued = Gauge::new(&observer, GaugeKind::Queue);
        assert!(!observer.ready());
        drop(queued);
        assert!(observer.ready());
        observer.shutdown.store(true, Ordering::Release);
        assert!(!observer.ready());
        observer.shutdown.store(false, Ordering::Release);
        assert!(observer.ready());
        drop(worker);
        assert!(!observer.ready());
        let trace = observer.request().unwrap();
        drop(RequestGuard(Some(trace)));
        assert_eq!(observer.stats.interrupted.load(Ordering::Relaxed), 1);
        assert_eq!(observer.snapshot()["queuedRequests"], 0);
    }
    #[tokio::test]
    async fn body_observation_reports_stream_failure_drop_and_empty_completion() {
        use crate::response::{Body, Cancel};
        use http_body_util::BodyExt;
        use tokio::sync::mpsc;
        let (observer, capture) = observer();
        let (send, receive) = mpsc::channel(1);
        let failed = Arc::new(AtomicBool::new(false));
        let trace = observer.request().unwrap();
        trace.headers(200);
        let mut body =
            Body::stream(receive, failed.clone(), Arc::new(Cancel::default())).observe(trace);
        send.send(hyper::body::Bytes::from_static(b"ok"))
            .await
            .unwrap();
        body.frame().await.unwrap().unwrap();
        failed.store(true, Ordering::Release);
        drop(send);
        assert!(body.frame().await.unwrap().is_err());
        drop(body);
        let trace = observer.request().unwrap();
        trace.headers(200);
        let (_send, receive) = mpsc::channel(1);
        drop(
            Body::stream(
                receive,
                Arc::new(AtomicBool::new(false)),
                Arc::new(Cancel::default()),
            )
            .observe(trace),
        );
        let trace = observer.request().unwrap();
        trace.headers(204);
        drop(Body::full("").observe(trace));
        assert_eq!(observer.stats.failed.load(Ordering::Relaxed), 1);
        assert_eq!(observer.stats.interrupted.load(Ordering::Relaxed), 1);
        assert_eq!(observer.stats.complete.load(Ordering::Relaxed), 1);
        assert_eq!(observer.stats.bytes.load(Ordering::Relaxed), 2);
        assert_eq!(
            String::from_utf8(capture.0.lock().unwrap().clone())
                .unwrap()
                .lines()
                .count(),
            6
        );
    }
}
