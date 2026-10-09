mod reader_throughput_tests {
    use super::*;
    use std::future::{Future, poll_fn};

    #[tokio::test]
    async fn keepalive_preserves_coherence_gate_ordering() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        {
            let gate = bot.coherent_state_gate.lock().await;
            let apply = bot.apply_packet(0x20, 42_i64.to_be_bytes().to_vec());
            tokio::pin!(apply);
            poll_fn(|cx| {
                assert!(apply.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            drop(gate);
            assert!(timeout(Duration::from_secs(1), apply).await.unwrap().unwrap());
        }
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn free_update_locks_are_ready_with_exhausted_budget() {
        let entities = RwLock::new(7);
        let player = Mutex::new(11);
        while tokio::task::coop::has_budget_remaining() {
            tokio::task::consume_budget().await;
        }
        let write = write_entity_update(&entities);
        let lock = lock_packet_state(&player);
        tokio::pin!(write, lock);
        let guard = poll_fn(|cx| match write.as_mut().poll(cx) {
            Poll::Ready(guard) => Poll::Ready(guard),
            Poll::Pending => panic!("free entity update yielded"),
        })
        .await;
        assert_eq!(*guard, 7);
        let guard = poll_fn(|cx| match lock.as_mut().poll(cx) {
            Poll::Ready(guard) => Poll::Ready(guard),
            Poll::Pending => panic!("free player state lock yielded"),
        })
        .await;
        assert_eq!(*guard, 11);
    }

    #[tokio::test]
    async fn update_locks_respect_held_and_queued_owners() {
        let entities = RwLock::new(0);
        let first = entities.read().await;
        let writer = entities.write();
        tokio::pin!(writer);
        poll_fn(|cx| {
            assert!(writer.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        let update = write_entity_update(&entities);
        tokio::pin!(update);
        poll_fn(|cx| {
            assert!(update.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        drop(first);
        let mut writer = writer.await;
        *writer = 7;
        poll_fn(|cx| {
            assert!(update.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        drop(writer);
        assert_eq!(*update.await, 7);

        let player = Mutex::new(0);
        let first = player.lock().await;
        let owner = player.lock();
        tokio::pin!(owner);
        poll_fn(|cx| {
            assert!(owner.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        let update = lock_packet_state(&player);
        tokio::pin!(update);
        poll_fn(|cx| {
            assert!(update.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        drop(first);
        let mut owner = owner.await;
        *owner = 11;
        poll_fn(|cx| {
            assert!(update.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        drop(owner);
        assert_eq!(*update.await, 11);
    }

    #[tokio::test]
    async fn ready_packet_backlog_yields_after_32_frames_and_allows_cancel() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        let wire = [1, 0x7f].repeat(100);
        let consumed = Arc::new(AtomicUsize::new(0));
        let reader = CountingRead {
            inner: wire.as_slice(),
            consumed: consumed.clone(),
            consumed_notify: Arc::new(Notify::new()),
        };
        let (_capture_tx, capture_rx) = mpsc::channel(2);
        let (_movement_tx, movement_rx) = mpsc::channel(2);
        {
            let read = bot.read_loop(reader, capture_rx, movement_rx);
            tokio::pin!(read);
            tokio::task::yield_now().await;
            poll_fn(|cx| {
                assert!(
                    read.as_mut().poll(cx).is_pending(),
                    "ready backlog did not yield"
                );
                Poll::Ready(())
            })
            .await;
            assert_eq!(consumed.load(Ordering::Acquire), 32 * 2);
            bot.cancel.notify_waiters();
            read.await.unwrap();
            assert_eq!(consumed.load(Ordering::Acquire), 32 * 2);
        }
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn free_cache_read_does_not_spend_cooperative_budget() {
        let cache = RwLock::new(7);
        while tokio::task::coop::has_budget_remaining() {
            tokio::task::consume_budget().await;
        }
        let read = read_session_cache(&cache);
        tokio::pin!(read);
        let guard = poll_fn(|cx| match read.as_mut().poll(cx) {
            Poll::Ready(guard) => Poll::Ready(guard),
            Poll::Pending => panic!("free cardinality read yielded with exhausted budget"),
        })
        .await;
        assert_eq!(*guard, 7);
    }

    #[tokio::test]
    async fn cache_read_respects_held_and_queued_writers() {
        let cache = RwLock::new(0);
        let first_reader = cache.read().await;
        let write = cache.write();
        tokio::pin!(write);
        poll_fn(|cx| {
            assert!(write.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        let read = read_session_cache(&cache);
        tokio::pin!(read);
        poll_fn(|cx| {
            assert!(
                read.as_mut().poll(cx).is_pending(),
                "queued writer was bypassed"
            );
            Poll::Ready(())
        })
        .await;
        drop(first_reader);
        let mut writer = write.await;
        *writer = 11;
        poll_fn(|cx| {
            assert!(
                read.as_mut().poll(cx).is_pending(),
                "held writer was bypassed"
            );
            Poll::Ready(())
        })
        .await;
        drop(writer);
        assert_eq!(*read.await, 11);
    }

    #[tokio::test]
    async fn all_limit_checks_complete_with_free_caches_and_exhausted_budget() {
        let (bot, server, release) =
            connected_test_bot(ConnectionOptions::default(), Vec::new()).await;
        while tokio::task::coop::has_budget_remaining() {
            tokio::task::consume_budget().await;
        }
        {
            let check = bot.enforce_session_limits();
            tokio::pin!(check);
            poll_fn(|cx| match check.as_mut().poll(cx) {
                Poll::Ready(result) => {
                    result.unwrap();
                    Poll::Ready(())
                }
                Poll::Pending => panic!("free per-packet limit check yielded"),
            })
            .await;
        }
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn non_growing_packet_still_checks_external_inventory_records() {
        let options = ConnectionOptions {
            max_cached_records: 1,
            ..ConnectionOptions::default()
        };
        let (bot, server, release) = connected_test_bot(options, Vec::new()).await;
        bot.inventory.write().await.windows.insert(0, vec![None; 2]);
        let error = bot.apply_packet(0x3f, vec![0]).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("session cache contains 2 records, limit is 1")
        );
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn play_reader_does_not_reacquire_writer_for_compression() {
        for compression in [None, Some(256), Some(0)] {
            let (bot, server, release) =
                connected_test_bot(ConnectionOptions::default(), Vec::new()).await;
            bot.writer.lock().await.compression = compression;
            let (mut tx, reader) = tokio::io::duplex(1024);
            let (_capture_tx, capture_rx) = mpsc::channel(2);
            let (_movement_tx, movement_rx) = mpsc::channel(2);
            {
                let read = bot.read_loop(reader, capture_rx, movement_rx);
                tokio::pin!(read);
                poll_fn(|cx| {
                    assert!(read.as_mut().poll(cx).is_pending());
                    Poll::Ready(())
                })
                .await;
                let writer = bot.writer.lock().await;
                // A second frame proves that the loop, not only the first read,
                // remains independent of the outgoing writer mutex.
                for slot in [1, 2] {
                    write_packet(&mut tx, compression, 0x3f, &[slot])
                        .await
                        .unwrap();
                    timeout(Duration::from_secs(1), async {
                        loop {
                            poll_fn(|cx| {
                                assert!(read.as_mut().poll(cx).is_pending());
                                Poll::Ready(())
                            })
                            .await;
                            if bot.inventory.read().await.selected_hotbar == slot {
                                break;
                            }
                            tokio::task::yield_now().await;
                        }
                    })
                    .await
                    .expect("play reader waited on the outgoing writer");
                }
                drop(writer);
                bot.cancel.notify_waiters();
                read.await.unwrap();
            }
            release.send(()).unwrap();
            drop(bot);
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn buffered_partial_frame_survives_a_coherent_capture() {
        use tokio::io::AsyncWriteExt;

        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        let (mut tx, reader) = tokio::io::duplex(64);
        let (capture_tx, capture_rx) = mpsc::channel(2);
        let (_movement_tx, movement_rx) = mpsc::channel(2);
        {
            let read = bot.read_loop(tokio::io::BufReader::new(reader), capture_rx, movement_rx);
            tokio::pin!(read);
            tx.write_all(&[2, 0x3f]).await.unwrap();
            poll_fn(|cx| {
                assert!(read.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            let (reply, response) = oneshot::channel();
            capture_tx
                .send(crate::observation::CaptureCommand {
                    request: CoherentObservationRequest::default(),
                    reply,
                })
                .await
                .unwrap();
            tokio::select! {
                result = &mut read => panic!("reader ended during capture: {result:?}"),
                result = response => { result.unwrap().unwrap(); }
            }
            tx.write_all(&[5]).await.unwrap();
            timeout(Duration::from_secs(1), async {
                loop {
                    poll_fn(|cx| {
                        assert!(read.as_mut().poll(cx).is_pending());
                        Poll::Ready(())
                    })
                    .await;
                    if bot.inventory.read().await.selected_hotbar == 5 {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            bot.cancel.notify_waiters();
            read.await.unwrap();
        }
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn buffered_login_preserves_prefetched_play_frames() {
        use tokio::io::AsyncWriteExt;

        for compression in [None, Some(256), Some(0)] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let (mut reader, mut writer) = stream.into_split();
                read_packet(&mut reader, None).await.unwrap();
                let (_, login) = read_packet(&mut reader, None).await.unwrap();
                let mut wire = Vec::new();
                if let Some(threshold) = compression {
                    let mut payload = Vec::new();
                    put_varint(&mut payload, threshold);
                    write_packet(&mut wire, None, 0x03, &payload).await.unwrap();
                }
                write_packet(
                    &mut wire,
                    compression,
                    0x02,
                    &crate::client::login::test_legacy_success(&login),
                )
                    .await
                    .unwrap();
                write_packet(&mut wire, compression, 0x3f, &[3])
                    .await
                    .unwrap();
                write_packet(&mut wire, compression, 0x3f, &[4])
                    .await
                    .unwrap();
                writer.write_all(&wire).await.unwrap();
                let mut byte = [0_u8; 1];
                let _ = reader.read(&mut byte).await;
            });
            let bot = Bot::connect(
                Server::new("127.0.0.1", port),
                Player::offline("BufferedProbe"),
                Arc::new(crate::SharedChunkStorage::default()),
                ConnectionOptions::default(),
            )
            .await
            .unwrap();
            timeout(Duration::from_secs(1), async {
                loop {
                    if bot.inventory.read().await.selected_hotbar == 4 {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("play frames prefetched during login were lost");
            drop(bot);
            server.await.unwrap();
        }
    }
}
