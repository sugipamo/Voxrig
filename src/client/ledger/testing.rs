//! Bounded, explicitly released transport for adapter writer regressions.
use std::{
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};
use tokio::io::AsyncWrite;

#[derive(Default)]
struct State {
    bytes: Vec<u8>,
    allowed: usize,
    flush: bool,
    fail_after: Option<usize>,
    fail_flush: bool,
    polls: usize,
    flush_polls: usize,
    flushes: usize,
    wake: Option<Waker>,
}
#[derive(Clone, Default)]
pub(crate) struct Control(Arc<Mutex<State>>);
pub(crate) struct BoundedWriter(Control);
impl Control {
    pub(crate) fn writer(&self) -> BoundedWriter {
        BoundedWriter(self.clone())
    }
    pub(crate) fn release(&self, allowed: usize, flush: bool) {
        let mut s = self.0.lock().unwrap();
        s.allowed = allowed;
        s.flush = flush;
        if let Some(wake) = s.wake.take() {
            wake.wake();
        }
    }
    pub(crate) fn fail(&self, after: Option<usize>, flush: bool) {
        let mut s = self.0.lock().unwrap();
        s.fail_after = after;
        s.fail_flush = flush;
        if let Some(wake) = s.wake.take() {
            wake.wake();
        }
    }
    pub(crate) fn bytes(&self) -> Vec<u8> {
        self.0.lock().unwrap().bytes.clone()
    }
    pub(crate) fn progress(&self) -> (usize, usize, usize) {
        let s = self.0.lock().unwrap();
        (s.polls, s.flush_polls, s.flushes)
    }
    pub(crate) async fn until(&self, predicate: impl Fn(&Self) -> bool) {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while !predicate(self) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("writer reached explicit boundary");
    }
}
impl AsyncWrite for BoundedWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        input: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let mut s = self.0.0.lock().unwrap();
        s.polls += 1;
        if s.fail_after.is_some_and(|limit| s.bytes.len() >= limit) {
            return Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into()));
        }
        let count = input
            .len()
            .min(s.allowed.saturating_sub(s.bytes.len()))
            .min(
                s.fail_after
                    .unwrap_or(usize::MAX)
                    .saturating_sub(s.bytes.len()),
            );
        if count == 0 {
            s.wake = Some(cx.waker().clone());
            return Poll::Pending;
        }
        s.bytes.extend_from_slice(&input[..count]);
        Poll::Ready(Ok(count))
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let mut s = self.0.0.lock().unwrap();
        s.flush_polls += 1;
        if s.fail_flush {
            return Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into()));
        }
        if !s.flush {
            s.wake = Some(cx.waker().clone());
            return Poll::Pending;
        }
        s.flushes += 1;
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
