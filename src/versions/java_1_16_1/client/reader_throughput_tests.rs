mod reader_throughput_tests {
    use super::*;
    use std::future::{Future, poll_fn};

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
                Poll::Ready(result) => Poll::Ready(result.unwrap()),
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
}
