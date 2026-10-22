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
                #[cfg(test)]
                tests::before_registration().await;
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
                #[cfg(test)]
                tests::before_registration().await;
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
pub(super) mod tests {
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
    struct Boundary {
        pause: bool,
        observations: std::cell::Cell<usize>,
    }
    tokio::task_local! { static BOUNDARY: Boundary; }

    // Only scoped scenarios yield here. The captured receive sequence must still
    // detect progress when notify_waiters runs before adapter registration.
    pub(in crate::client) async fn before_registration() {
        let pause = BOUNDARY.try_with(|boundary| {
            boundary.observations.set(boundary.observations.get() + 1);
            boundary.pause
        });
        if matches!(pause, Ok(true)) {
            tokio::task::yield_now().await;
        }
    }

    pub(in crate::client) async fn complete_after_update<T>(
        race: bool,
        wait: impl std::future::Future<Output = T>,
        update: impl std::future::Future<Output = ()>,
    ) -> T {
        use std::task::Poll;
        BOUNDARY
            .scope(
                Boundary {
                    pause: race,
                    observations: std::cell::Cell::new(0),
                },
                async {
                    let mut wait = std::pin::pin!(wait);
                    std::future::poll_fn(|cx| {
                        assert!(wait.as_mut().poll(cx).is_pending());
                        Poll::Ready(())
                    })
                    .await;
                    // For race=true, paused after the empty observation and before
                    // registration. Otherwise the adapter's receive wait is Pending.
                    assert_eq!(BOUNDARY.with(|b| b.observations.get()), 1);
                    update.await;
                    wait.await
                },
            )
            .await
    }

    impl Client {
        pub(crate) async fn assert_common_wait_progress<F: std::future::Future<Output = ()>>(
            client: &Client,
            mut update: impl FnMut(Option<[i32; 3]>) -> F,
        ) {
            let limit = Duration::from_secs(1);
            for (race, position) in [(false, [32, 64, 32]), (true, [64, 64, 64])] {
                let region = crate::Region {
                    min: position,
                    max: position,
                };
                assert!(
                    client.capture(region).await.unwrap().world.blocks[0]
                        .state
                        .is_none()
                );
                let before = client.player_state().await.unwrap();
                let loaded = complete_after_update(
                    race,
                    client.wait_for_loaded(region, limit),
                    update(Some(position)),
                )
                .await
                .unwrap();
                assert_eq!(loaded.player.session, before.session);
                assert_eq!(loaded.player.receive_sequence, before.receive_sequence + 1);
                assert_eq!(
                    loaded.world.receive_sequence,
                    Some(loaded.player.receive_sequence)
                );
                assert_eq!(
                    loaded.world.blocks[0].state.as_ref().unwrap().name,
                    "minecraft:stone"
                );

                let cursor = loaded.player.receive_sequence;
                let chat =
                    complete_after_update(race, client.wait_for_chat(cursor, limit), update(None))
                        .await
                        .unwrap();
                assert_eq!(chat.session, before.session);
                assert_eq!(chat.receive_sequence, cursor + 1);
                assert_eq!(chat.messages.len(), 1);
                assert_eq!(chat.messages[0].receive_sequence, chat.receive_sequence);
                assert_eq!(chat.messages[0].kind, crate::client::ChatKind::System);
                assert!(
                    client
                        .chat_after(chat.receive_sequence)
                        .await
                        .unwrap()
                        .messages
                        .is_empty()
                );
            }
            Client::assert_common_event_wait_progress(client, || update(None)).await;
        }

        // The adapter fixtures supply one loaded stone cell and one received chat.
        // Updating inside the predicate puts progress precisely after capture and
        // before wait_after can register a notification waiter.
        pub(crate) async fn assert_common_state_waits(
            client: &Client,
            position: [i32; 3],
            mut update: impl FnMut(),
        ) {
            let region = crate::Region {
                min: position,
                max: position,
            };
            let player = client.player_state().await.unwrap();
            let initial = player.receive_sequence;
            let ready = client
                .wait_for_block(position, Duration::ZERO, |state| {
                    state.is_some_and(|state| state.name == "minecraft:stone")
                })
                .await
                .unwrap();
            assert_eq!(ready.player.receive_sequence, initial);
            let loaded = client
                .wait_for_loaded(region, Duration::ZERO)
                .await
                .unwrap();
            assert_eq!(
                loaded.world.blocks[0].state.as_ref().unwrap().name,
                "minecraft:stone"
            );
            let chat = client.wait_for_chat(0, Duration::ZERO).await.unwrap();
            assert_eq!(chat.messages.len(), 1);
            assert_eq!(chat.messages[0].kind, crate::client::ChatKind::System);
            assert_eq!(chat.receive_sequence, initial);
            assert_eq!(
                client
                    .wait_for_receive(initial - 1, Duration::ZERO)
                    .await
                    .unwrap(),
                initial
            );

            let mut checks = 0;
            let updated = client
                .wait_for_block(position, Duration::from_secs(1), |state| {
                    checks += 1;
                    if checks == 1 {
                        assert_eq!(state.unwrap().name, "minecraft:stone");
                        update();
                        false
                    } else {
                        state.is_some_and(|state| state.name == "minecraft:dirt")
                    }
                })
                .await
                .unwrap();
            assert_eq!(checks, 2);
            assert_eq!(updated.player.receive_sequence, initial + 1);
            assert_eq!(
                updated.world.blocks[0].state.as_ref().unwrap().name,
                "minecraft:dirt"
            );

            // Fixtures pause Tokio time after establishing their loopback connection.
            let missing = crate::Region {
                min: [32, 64, 32],
                max: [32, 64, 32],
            };
            let limit = Duration::from_millis(5);
            assert_eq!(
                client
                    .wait_for_block(position, limit, |_| false)
                    .await
                    .unwrap_err()
                    .kind(),
                ErrorKind::Timeout
            );
            assert_eq!(
                client
                    .wait_for_loaded(missing, limit)
                    .await
                    .unwrap_err()
                    .kind(),
                ErrorKind::Timeout
            );
            assert_eq!(
                client
                    .wait_for_chat(initial + 1, limit)
                    .await
                    .unwrap_err()
                    .kind(),
                ErrorKind::Timeout
            );
            assert_eq!(
                client
                    .wait_for_receive(initial + 1, limit)
                    .await
                    .unwrap_err()
                    .kind(),
                ErrorKind::Timeout
            );

            use std::{cell::Cell, future::Future, task::Poll};
            let checks = Cell::new(0);
            let limit = Duration::from_secs(1);
            let mut block = std::pin::pin!(client.wait_for_block(position, limit, |_| {
                checks.set(checks.get() + 1);
                false
            }));
            let mut loaded = std::pin::pin!(client.wait_for_loaded(missing, limit));
            let mut chat = std::pin::pin!(client.wait_for_chat(initial + 1, limit));
            let mut receive = std::pin::pin!(client.wait_for_receive(initial + 1, limit));
            std::future::poll_fn(|cx| {
                assert!(block.as_mut().poll(cx).is_pending());
                assert!(loaded.as_mut().poll(cx).is_pending());
                assert!(chat.as_mut().poll(cx).is_pending());
                assert!(receive.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            assert_eq!(checks.get(), 1, "block predicate ran before closure");
            let _ = client.revoke_connection();
            assert_eq!(block.await.unwrap_err().kind(), ErrorKind::Disconnected);
            assert_eq!(loaded.await.unwrap_err().kind(), ErrorKind::Disconnected);
            assert_eq!(chat.await.unwrap_err().kind(), ErrorKind::Disconnected);
            assert_eq!(receive.await.unwrap_err().kind(), ErrorKind::Disconnected);

            // Characterize the current distinction between waits already registered
            // at closure and new state/chat reads made after closure on each adapter.
            let state_error = match player.session.version {
                MinecraftVersion::Java1_16_1 => ErrorKind::State,
                MinecraftVersion::Java1_21_11 => ErrorKind::Disconnected,
            };
            assert_eq!(
                client
                    .wait_for_block(position, limit, |_| true)
                    .await
                    .unwrap_err()
                    .kind(),
                state_error
            );
            assert_eq!(
                client
                    .wait_for_loaded(region, limit)
                    .await
                    .unwrap_err()
                    .kind(),
                state_error
            );
            assert_eq!(
                client.wait_for_chat(0, limit).await.unwrap_err().kind(),
                state_error
            );
            assert_eq!(
                client.wait_for_receive(0, limit).await.unwrap_err().kind(),
                ErrorKind::Disconnected
            );
        }
    }
}
