//! The live 1.21.11 sender closes a session on any uncertain frame boundary.
use super::*;
use tokio::io::AsyncWrite;

struct WriteAttempt<'a> {
    session: &'a Session,
    packet_id: i32,
    armed: bool,
}
impl Drop for WriteAttempt<'_> {
    fn drop(&mut self) {
        if self.armed {
            // Publish uncertainty before closure and before the writer lock is
            // released. Queued sends recheck this boundary under that same lock.
            let _ = self.session.interrupted_packet.compare_exchange(
                -1,
                self.packet_id,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            self.session.ledger.fail(Failure::UncertainDispatch);
            self.session.stop();
        }
    }
}

impl Session {
    pub(super) fn revoke(self: &Arc<Self>) {
        // Receipt issue and revocation share the ledger gate.
        self.ledger.fail(Failure::Revoked);
        if self.revoked.swap(true, Ordering::AcqRel) {
            return;
        }
        self.stop();
        if let Some(receiver) = self.receiver_abort.get() {
            receiver.abort();
        }
    }
    pub(super) fn stop(&self) {
        self.ledger.fail(Failure::Disconnected);
        self.stopped.store(true, Ordering::Release);
        if let Ok(mut state) = self.state.try_lock() {
            let session = state.snapshot_session();
            let reason = if self.revoked.load(Ordering::Acquire) {
                crate::client::InvalidationReason::Revoke
            } else {
                crate::client::InvalidationReason::Disconnect
            };
            if state.failure.is_none() || reason == crate::client::InvalidationReason::Revoke {
                state.snapshot.terminate(reason, session, None);
            }
        }
        // Wake both a blocked writer and the receive loop. Retain a permit for
        // teardown which has not yet started waiting.
        self.cancel.notify_waiters();
        self.cancel.notify_one();
        self.changed.notify_waiters();
        let writer = self.writer.clone();
        self.ledger
            .tasks()
            .start(async move { writer.lock().await.stream.shutdown().await });
    }

    pub(super) fn check_outbound(&self) -> Result<()> {
        let closed = self.stopped.load(Ordering::Acquire);
        let packet = self.interrupted_packet.load(Ordering::Acquire);
        if packet >= 0 {
            return Err(Error::new(
                ErrorKind::UncertainDispatch,
                anyhow::anyhow!(
                    "connection closed after interrupted frame for packet {packet}; server effects unknown; inspect history without retry"
                ),
            ));
        }
        if closed {
            return Err(Error::new(
                ErrorKind::Disconnected,
                anyhow::anyhow!("connection closed"),
            ));
        }
        Ok(())
    }

    pub(super) async fn send(&self, id: i32, payload: &[u8]) -> Result<()> {
        let work = ScopedWork::new(self.ledger.try_work()?);
        self.send_work(id, payload, work.guard()).await
    }

    pub(super) async fn send_with_attempt(
        &self,
        id: i32,
        payload: &[u8],
        write_started: Option<&AtomicBool>,
    ) -> Result<()> {
        let work = ScopedWork::new(self.ledger.try_work()?);
        self.send_work_with_attempt(id, payload, work.guard(), write_started)
            .await
    }

    pub(super) async fn send_work(&self, id: i32, payload: &[u8], work: &WorkGuard) -> Result<()> {
        self.send_work_with_attempt(id, payload, work, None).await
    }

    async fn send_work_with_attempt(
        &self,
        id: i32,
        payload: &[u8],
        work: &WorkGuard,
        write_started: Option<&AtomicBool>,
    ) -> Result<()> {
        let cancelled = self.cancel.notified();
        tokio::pin!(cancelled);
        cancelled.as_mut().enable();
        self.check_outbound()?;
        // Cancellation while waiting for this lock cannot interrupt a frame.
        let mut writer = tokio::select! {
            biased;
            _ = &mut cancelled => return self.check_outbound(),
            writer = self.writer.lock() => writer,
        };
        let compression = writer.compression;
        self.write_frame(
            &mut writer.stream,
            compression,
            id,
            payload,
            work,
            write_started,
        )
        .await
    }

    // Shared by the real TCP sender and bounded stream tests. The caller owns
    // the writer lock for the complete attempt, including guard destruction.
    #[cfg(test)]
    async fn write_test_frame<W: AsyncWrite + Unpin>(
        &self,
        stream: &mut SocketWriter<W>,
        compression: Option<i32>,
        id: i32,
        payload: &[u8],
    ) -> Result<()> {
        let work = ScopedWork::new(self.ledger.admit().expect("test sender admission"));
        self.write_frame(stream, compression, id, payload, work.guard(), None)
            .await
    }

    async fn write_frame<W: AsyncWrite + Unpin>(
        &self,
        stream: &mut SocketWriter<W>,
        compression: Option<i32>,
        id: i32,
        payload: &[u8],
        work: &WorkGuard,
        write_started: Option<&AtomicBool>,
    ) -> Result<()> {
        let cancelled = self.cancel.notified();
        tokio::pin!(cancelled);
        cancelled.as_mut().enable();
        self.check_outbound()?;
        let mut attempt = WriteAttempt {
            session: self,
            packet_id: id,
            armed: true,
        };
        let result = tokio::select! {
            biased;
            _ = &mut cancelled => Err(anyhow::anyhow!("connection closed during frame write")),
            result = stream.send_with_attempt(work, compression, id, payload, write_started) => result,
        };
        match result {
            Ok(()) => {
                attempt.armed = false;
                Ok(())
            }
            Err(error) => {
                drop(attempt);
                Err(Error::new(
                    ErrorKind::UncertainDispatch,
                    error.context(
                        "connection closed after uncertain frame write; server effects unknown",
                    ),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests;
