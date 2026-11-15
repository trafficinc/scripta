//! Bounded HTTP body delivery and cancellation back to the owning worker.
use http_body_util::Full;
use hyper::body::{Body as HttpBody, Bytes, Frame, SizeHint};
use std::{
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};
use tokio::sync::{Notify, mpsc};
#[derive(Default)]
pub(crate) struct Cancel {
    cancelled: AtomicBool,
    notify: Notify,
}
impl Cancel {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    pub fn trigger(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.notify.notify_one();
    }
    pub async fn wait(&self) {
        loop {
            if self.cancelled.load(Ordering::Acquire) {
                return;
            }
            self.notify.notified().await
        }
    }
}
pub(crate) struct CancelGuard(pub Option<Arc<Cancel>>);
impl Drop for CancelGuard {
    fn drop(&mut self) {
        if let Some(cancel) = &self.0 {
            cancel.trigger()
        }
    }
}
pub(crate) enum Body {
    Observed {
        body: Box<Body>,
        trace: Arc<crate::observe::Trace>,
        finished: bool,
    },
    Full(Full<Bytes>),
    Suppressed,
    Stream {
        receive: mpsc::Receiver<Bytes>,
        failed: Arc<AtomicBool>,
        cancel: Arc<Cancel>,
        finished: bool,
    },
}
impl Body {
    pub fn observe(self, trace: Arc<crate::observe::Trace>) -> Self {
        let finished = self.is_end_stream();
        if finished {
            trace.finish("complete");
        }
        Self::Observed {
            body: Box::new(self),
            trace,
            finished,
        }
    }
    pub fn full(bytes: impl Into<Bytes>) -> Self {
        Self::Full(Full::new(bytes.into()))
    }
    pub fn stream(
        receive: mpsc::Receiver<Bytes>,
        failed: Arc<AtomicBool>,
        cancel: Arc<Cancel>,
    ) -> Self {
        Self::Stream {
            receive,
            failed,
            cancel,
            finished: false,
        }
    }
}
impl Drop for Body {
    fn drop(&mut self) {
        if let Self::Observed {
            trace,
            finished: false,
            ..
        } = self
        {
            trace.failure("body_dropped");
            trace.finish("interrupted");
        }
        if let Self::Stream {
            cancel,
            finished: false,
            ..
        } = self
        {
            cancel.trigger()
        }
    }
}
impl HttpBody for Body {
    type Data = Bytes;
    type Error = io::Error;
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        match self.get_mut() {
            Self::Observed {
                body,
                trace,
                finished,
            } => {
                let result = Pin::new(&mut **body).poll_frame(cx);
                match &result {
                    Poll::Ready(Some(Ok(frame))) => {
                        if let Some(bytes) = frame.data_ref() {
                            trace.bytes(bytes.len());
                        }
                        if body.is_end_stream() {
                            *finished = true;
                            trace.finish("complete");
                        }
                    }
                    Poll::Ready(Some(Err(_))) => {
                        *finished = true;
                        trace.failure("body_error");
                        trace.finish("failed");
                    }
                    Poll::Ready(None) => {
                        *finished = true;
                        trace.finish("complete");
                    }
                    Poll::Pending => {}
                }
                result
            }
            Self::Full(body) => match Pin::new(body).poll_frame(cx) {
                Poll::Ready(Some(Ok(frame))) => Poll::Ready(Some(Ok(frame))),
                Poll::Ready(None) => Poll::Ready(None),
                Poll::Pending => Poll::Pending,
                Poll::Ready(Some(Err(never))) => match never {},
            },
            Self::Suppressed => Poll::Ready(None),
            Self::Stream {
                receive,
                failed,
                finished,
                ..
            } => {
                if *finished {
                    return Poll::Ready(None);
                }
                match receive.poll_recv(cx) {
                    Poll::Pending => Poll::Pending,
                    Poll::Ready(Some(bytes)) => Poll::Ready(Some(Ok(Frame::data(bytes)))),
                    Poll::Ready(None) => {
                        *finished = true;
                        if failed.load(Ordering::Acquire) {
                            Poll::Ready(Some(Err(io::Error::other(
                                "response ended after transmission failure",
                            ))))
                        } else {
                            Poll::Ready(None)
                        }
                    }
                }
            }
        }
    }
    fn is_end_stream(&self) -> bool {
        match self {
            Self::Observed { body, .. } => body.is_end_stream(),
            Self::Full(body) => body.is_end_stream(),
            Self::Suppressed => true,
            Self::Stream { finished, .. } => *finished,
        }
    }
    fn size_hint(&self) -> SizeHint {
        match self {
            Self::Observed { body, .. } => body.size_hint(),
            Self::Full(body) => body.size_hint(),
            _ => SizeHint::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    #[tokio::test]
    async fn queue_is_bounded_failure_survives_a_full_queue_and_drop_cancels() {
        let (send, receive) = mpsc::channel(1);
        let failed = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(Cancel::default());
        let mut body = Body::stream(receive, failed.clone(), cancel.clone());
        send.send(Bytes::from_static(b"first")).await.unwrap();
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(10),
                send.send(Bytes::from_static(b"blocked"))
            )
            .await
            .is_err()
        );
        failed.store(true, Ordering::Release);
        drop(send);
        assert_eq!(
            body.frame().await.unwrap().unwrap().into_data().unwrap(),
            b"first"[0..5]
        );
        assert!(body.frame().await.unwrap().is_err());
        assert!(body.frame().await.is_none());
        let (_, receive) = mpsc::channel(1);
        let body = Body::stream(receive, failed, cancel.clone());
        drop(body);
        cancel.wait().await;
    }
}
