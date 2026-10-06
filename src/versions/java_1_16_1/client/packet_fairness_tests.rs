mod protocol_fairness_tests {
    use super::*;
    use std::future::{Future, poll_fn};

    async fn echo_probe_bot() -> (Bot, tokio::task::JoinHandle<()>, oneshot::Receiver<Vec<u8>>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (echo_tx, echo_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut reader, mut writer) = stream.into_split();
            read_packet(&mut reader, None).await.unwrap();
            let (_, login) = read_packet(&mut reader, None).await.unwrap();
            write_packet(&mut writer, None, 2, &crate::client::login::test_legacy_success(&login)).await.unwrap();
            loop {
                let (id, payload) = read_packet(&mut reader, None).await.unwrap();
                if id == 0x10 {
                    let _ = echo_tx.send(payload);
                    break;
                }
            }
            std::future::pending::<()>().await;
        });
        let bot = Bot::connect(
            Server::new("127.0.0.1", port),
            Player::offline("FairnessProbe"),
            Arc::new(crate::SharedChunkStorage::default()),
            ConnectionOptions::default(),
        )
        .await
        .unwrap();
        bot.player.lock().await.spawned = true;
        *bot.positioned.lock().await = true;
        bot.connection.mark_ready().await;
        timeout(Duration::from_secs(1), async {
            while bot.connection_state() != ConnectionState::Ready {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        (bot, server, echo_rx)
    }

    async fn queued_captures_cannot_starve_ready_packets(movement: bool) {
        let (bot, server, echo) = echo_probe_bot().await;
        let (mut wire, reader) = tokio::io::duplex(128);
        let keepalive = 12345_i64.to_be_bytes();
        let mut vitals = Vec::new();
        vitals.write_f32::<BigEndian>(7.0).unwrap();
        put_varint(&mut vitals, 20);
        vitals.write_f32::<BigEndian>(5.0).unwrap();
        write_packet(&mut wire, None, 0x20, &keepalive)
            .await
            .unwrap();
        write_packet(&mut wire, None, 0x49, &vitals).await.unwrap();

        let (capture_tx, capture_rx) = mpsc::channel(8);
        let (movement_tx, movement_rx) = mpsc::channel(8);
        let mut captures = Vec::new();
        let mut movements = Vec::new();
        for _ in 0..8 {
            if movement {
                let (reply, response) = oneshot::channel();
                movement_tx
                    .send(crate::observation::TraversalMovementFactsCommand {
                        request: crate::MovementSnapshotRequest {
                            expected_generation: bot.connection_generation(),
                            region: BlockRegion::new(
                                BlockPos { x: 0, y: 0, z: 0 },
                                BlockPos { x: 0, y: 0, z: 0 },
                            ),
                            entity_radius: 0,
                            max_entities: 0,
                        },
                        reply,
                    })
                    .await
                    .unwrap();
                movements.push(response);
            } else {
                let (reply, response) = oneshot::channel();
                capture_tx
                    .send(crate::observation::CaptureCommand {
                        request: CoherentObservationRequest::default(),
                        reply,
                    })
                    .await
                    .unwrap();
                captures.push(response);
            }
        }
        let loop_bot = bot.clone_internal();
        let task =
            tokio::spawn(async move { loop_bot.read_loop(reader, capture_rx, movement_rx).await });
        let mut fresh = 0;
        for (ordinal, response) in captures.into_iter().enumerate() {
            let observed = timeout(Duration::from_secs(2), response)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if ordinal == 0 {
                assert!(
                    observed.survival.vitals.is_none(),
                    "packet backlog displaced the first ready capture"
                );
            }
            fresh += usize::from(observed.survival.vitals.is_some_and(|v| v.health == 7.0));
        }
        for (ordinal, response) in movements.into_iter().enumerate() {
            let observed = timeout(Duration::from_secs(2), response)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if ordinal == 0 {
                assert!(
                    observed.survival.vitals.is_none(),
                    "packet backlog displaced the first ready capture"
                );
            }
            fresh += usize::from(observed.survival.vitals.is_some_and(|v| v.health == 7.0));
        }
        let echoed = timeout(Duration::from_secs(2), echo)
            .await
            .unwrap()
            .unwrap();
        bot.cancel.notify_waiters();
        timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        server.abort();
        assert_eq!(echoed, keepalive);
        assert!(
            fresh > 0,
            "all queued captures completed before ready protocol packets were processed"
        );
    }

    #[tokio::test]
    async fn coherent_capture_backlog_cannot_starve_keepalive_and_vitals() {
        queued_captures_cannot_starve_ready_packets(false).await;
    }

    #[tokio::test]
    async fn traversal_capture_backlog_cannot_starve_keepalive_and_vitals() {
        queued_captures_cannot_starve_ready_packets(true).await;
    }

    #[tokio::test]
    async fn expired_packet_deadline_is_checked_between_queued_captures() {
        let (mut bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        bot.connection_options.play_packet_timeout = Duration::from_millis(100);
        tokio::time::pause();
        let (_wire, reader) = tokio::io::duplex(64);
        let (capture_tx, capture_rx) = mpsc::channel(8);
        let (_movement_tx, movement_rx) = mpsc::channel(8);
        let mut responses = Vec::new();
        {
            let read = bot.read_loop(reader, capture_rx, movement_rx);
            tokio::pin!(read);
            poll_fn(|cx| {
                assert!(read.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            tokio::time::advance(Duration::from_millis(101)).await;
            for _ in 0..8 {
                let (reply, response) = oneshot::channel();
                capture_tx
                    .send(crate::observation::CaptureCommand {
                        request: CoherentObservationRequest::default(),
                        reply,
                    })
                    .await
                    .unwrap();
                responses.push(response);
            }
            assert_eq!(read.await.unwrap_err().to_string(), "play packet timed out");
        }
        let completed = responses
            .into_iter()
            .filter_map(|mut response| response.try_recv().ok())
            .filter(Result::is_ok)
            .count();
        tokio::time::resume();
        bot.cancel.notify_waiters();
        server.abort();
        drop(release);
        assert!(
            completed <= 1,
            "expired packet deadline was postponed by {completed} queued captures"
        );
    }

    #[tokio::test]
    async fn ready_cancel_keeps_priority_over_captures_and_packets() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        let (mut wire, reader) = tokio::io::duplex(64);
        let (capture_tx, capture_rx) = mpsc::channel(1);
        let (_movement_tx, movement_rx) = mpsc::channel(1);
        let (reply, mut response) = oneshot::channel();
        {
            let read = bot.read_loop(reader, capture_rx, movement_rx);
            tokio::pin!(read);
            poll_fn(|cx| {
                assert!(read.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            write_packet(&mut wire, None, 0x20, &12345_i64.to_be_bytes())
                .await
                .unwrap();
            capture_tx
                .send(crate::observation::CaptureCommand {
                    request: CoherentObservationRequest::default(),
                    reply,
                })
                .await
                .unwrap();
            bot.cancel.notify_waiters();
            timeout(Duration::from_secs(1), read)
                .await
                .unwrap()
                .unwrap();
        }
        assert_eq!(bot.protocol_packet_sequence.load(Ordering::Acquire), 0);
        assert!(matches!(
            response.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        ));
        bot.cancel.notify_waiters();
        server.abort();
        drop(release);
    }
    #[tokio::test]
    async fn generation_revocation_interrupts_a_capture_behind_the_coherent_gate() {
        let (bot, server, _) = echo_probe_bot().await;
        let gate = bot.coherent_state_gate.lock().await;
        let other = bot.clone();
        let capture = tokio::spawn(async move { other.capture_coherent_observation(CoherentObservationRequest::default()).await });
        tokio::task::yield_now().await;
        let receipt = bot.revoke_connection();
        assert_eq!(receipt.generation(), bot.connection_generation());
        assert_eq!(bot.connection_state(), ConnectionState::ConnectionStateUnknown);
        assert!(timeout(Duration::from_secs(1), capture).await.unwrap().unwrap().is_err());
        drop(gate);
        assert!(bot.apply_packet(0x20, 98765_i64.to_be_bytes().to_vec()).await.is_err());
        server.abort();
    }

}
