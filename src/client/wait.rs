//! Bounded waits for received state, common to both versions.
//!
//! Each wait re-observes after every applied packet and compares receive
//! sequences, so a change between the check and the wait is never missed.
//! A satisfied wait reports received client state, not server confirmation.
use super::adapter::{CoreOps, WaitOps};
use super::{Capture, ChatLog};
use crate::{Error, ErrorKind, NativeBlockState, Region, Result};
use std::time::Duration;

fn timed_out(what: &str) -> Error {
    Error::new(
        ErrorKind::Timeout,
        anyhow::anyhow!("timed out waiting for {what}"),
    )
}

impl super::Client {
    /// Wait until a packet with a receive sequence greater than `after` has been
    /// applied, and return the new sequence. Fails if the connection closes.
    pub async fn wait_for_receive(&self, after: u64, limit: Duration) -> Result<u64> {
        tokio::time::timeout(limit, self.wait_after(after))
            .await
            .map_err(|_| timed_out("a received packet"))?
    }

    /// Wait until the block at `position` satisfies `accept`. `None` means the
    /// cell is not loaded. Returns the capture in which it was satisfied.
    pub async fn wait_for_block(
        &self,
        position: [i32; 3],
        limit: Duration,
        mut accept: impl FnMut(Option<&NativeBlockState>) -> bool,
    ) -> Result<Capture> {
        let region = Region {
            min: position,
            max: position,
        };
        self.wait_for_capture(region, limit, "a block state", |capture| {
            accept(capture.world.blocks[0].state.as_ref())
        })
        .await
    }

    /// Wait until every cell of `region` is loaded. Air is a loaded state.
    pub async fn wait_for_loaded(&self, region: Region, limit: Duration) -> Result<Capture> {
        self.wait_for_capture(region, limit, "loaded terrain", |capture| {
            capture.world.blocks.iter().all(|cell| cell.state.is_some())
        })
        .await
    }

    /// Wait until at least one chat message arrives after `cursor`.
    pub async fn wait_for_chat(&self, cursor: u64, limit: Duration) -> Result<ChatLog> {
        tokio::time::timeout(limit, async {
            loop {
                let log = self.chat_after(cursor).await?;
                if !log.messages.is_empty() {
                    return Ok(log);
                }
                self.wait_after(log.receive_sequence).await?;
            }
        })
        .await
        .map_err(|_| timed_out("chat"))?
    }

    async fn wait_for_capture(
        &self,
        region: Region,
        limit: Duration,
        what: &str,
        mut done: impl FnMut(&Capture) -> bool,
    ) -> Result<Capture> {
        region.volume()?;
        tokio::time::timeout(limit, async {
            loop {
                let capture =
                    super::dispatch!(&self.adapter, a => CoreOps::capture(a, region).await)?;
                if done(&capture) {
                    return Ok(capture);
                }
                self.wait_after(capture.player.receive_sequence).await?;
            }
        })
        .await
        .map_err(|_| timed_out(what))?
    }

    async fn wait_after(&self, sequence: u64) -> Result<u64> {
        super::dispatch!(&self.adapter, a => WaitOps::wait_for_receive(a, sequence).await)
    }
}

#[cfg(test)]
mod tests {
    use crate::protocol::{read_packet, write_packet};
    use crate::{Client, ConnectionConfig, ErrorKind, MinecraftVersion};
    use std::time::Duration;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn waits_time_out_without_progress_and_fail_after_close() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_packet(&mut stream, None).await.unwrap();
            let (_, login) = read_packet(&mut stream, None).await.unwrap();
            write_packet(
                &mut stream,
                None,
                2,
                &crate::client::login::test_legacy_success(&login),
            )
            .await
            .unwrap();
            while read_packet(&mut stream, None).await.is_ok() {}
        });
        let client = Client::connect(ConnectionConfig::offline(
            crate::client::Server::new("127.0.0.1", port),
            "Waiter",
            MinecraftVersion::Java1_16_1,
        ))
        .await
        .unwrap();
        let pending = client
            .wait_for_receive(u64::MAX - 1, Duration::from_millis(100))
            .await;
        assert_eq!(pending.unwrap_err().kind(), ErrorKind::Timeout);
        client.disconnect().await.unwrap();
        let closed = client
            .wait_for_receive(u64::MAX - 1, Duration::from_secs(5))
            .await;
        assert_eq!(closed.unwrap_err().kind(), ErrorKind::Disconnected);
        server.await.unwrap();
    }
}
