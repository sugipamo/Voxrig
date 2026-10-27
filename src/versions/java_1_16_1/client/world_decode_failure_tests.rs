//! Failure propagation through the common API, without a native subscriber.
use super::*;
use crate::ErrorKind;
use crate::client::EventKind;

fn light_packet() -> Vec<u8> {
    // Column 0,0, trusted edges, sky section 0, no other channels.
    let mut p = vec![0, 0, 1, 2, 0, 0, 0];
    put_varint(&mut p, 2048);
    p.extend([0xab; 2048]);
    p
}

#[tokio::test]
async fn world_decode_failure_withdraws_cached_observations_at_the_receive_boundary() {
    for (id, payload) in [
        (0x21, vec![]), // No coordinates to localize the loss.
        (0x21, tests::empty_chunk_packet(0, 0)[..12].to_vec()),
        (0x24, vec![]),
        (0x24, light_packet()[..12].to_vec()),
        (0x24, [light_packet(), vec![0]].concat()), // Trailing data.
    ] {
        let (bot, server, release) = tests::ready_test_bot(ConnectionOptions::default()).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        bot.apply_packet(0x21, tests::empty_chunk_packet(0, 0))
            .await
            .unwrap();
        bot.apply_packet(0x24, light_packet()).await.unwrap();
        let saved = client.chunk([0, 0]).await.unwrap().unwrap();
        assert_eq!(saved.sky_light([0, 0, 0]), Some(11));
        assert!(client.chunk_context([0, 0]).await.unwrap().is_some());
        let before = client.events_after(0).await.unwrap();
        let error = bot.apply_packet(id, payload).await.unwrap_err();
        let diagnostic = format!("{:#}", error.diagnostic());
        assert!(diagnostic.contains(&format!("play packet {id:#04x}")));
        // apply_packet is the decoder fixture. The supervisor has not yet run;
        // reads must already fail while the transport still appears ready.
        assert!(!bot.is_stopped());
        let snapshot_error = client.received_player_snapshot().await.unwrap_err();
        assert_eq!(snapshot_error.kind(), error.kind());
        assert_eq!(snapshot_error.to_string(), diagnostic);
        assert_eq!(client.chunk([0, 0]).await.unwrap_err().kind(), error.kind());
        assert_eq!(
            client.loaded_chunks().await.unwrap_err().kind(),
            error.kind()
        );
        assert_eq!(
            client.chunk_context([0, 0]).await.unwrap_err().kind(),
            error.kind()
        );
        let log = client
            .wait_for_events(before.cursor, Duration::ZERO)
            .await
            .unwrap();
        assert_eq!(log.events.len(), 1);
        assert_eq!(log.events[0].kind, EventKind::ObservationInvalidated);
        assert_eq!(log.events[0].receive_sequence, before.receive_sequence + 1);
        assert_eq!(log.receive_sequence, before.receive_sequence + 1);
        // A previously returned immutable value is a historical observation,
        // not a live cache that can silently become current again.
        assert_eq!(saved.receive_sequence, before.receive_sequence);
        assert_eq!(saved.sky_light([0, 0, 0]), Some(11));
        release.send(()).unwrap();
        client.disconnect().await.unwrap();
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
}

#[tokio::test]
async fn world_decode_failure_stops_real_reader_and_reaches_common_waiter() {
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;

    for id in [0x21, 0x24] {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (go, start) = tokio::sync::oneshot::channel();
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
            start.await.unwrap();
            write_packet(&mut stream, None, id, &[]).await.unwrap();
            // Following packets cannot restore eligibility after this failure.
            let _ = write_packet(&mut stream, None, 0x21, &tests::empty_chunk_packet(1, 0)).await;
            let mut buffer = [0; 4096];
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    match stream.read(&mut buffer).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
            })
            .await
            .expect("reader failure did not close the writer");
        });
        let client = crate::Client::connect(crate::ConnectionConfig::offline(
            crate::client::Server::new("127.0.0.1", port),
            "DecodeFailure",
            crate::MinecraftVersion::Java1_16_1,
        ))
        .await
        .unwrap();
        let before = client.events_after(0).await.unwrap();
        let wait = client.wait_for_events(before.cursor, Duration::from_secs(5));
        tokio::pin!(wait);
        std::future::poll_fn(|cx| {
            assert!(wait.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        go.send(()).unwrap();
        let mut observed = wait.await.unwrap();
        assert_eq!(observed.events[0].kind, EventKind::ObservationInvalidated);
        while !observed
            .events
            .iter()
            .any(|e| e.kind == EventKind::Disconnected)
        {
            observed = client
                .wait_for_events(observed.cursor, Duration::from_secs(5))
                .await
                .unwrap();
        }
        server.await.unwrap();
        let log = client.events_after(before.cursor).await.unwrap();
        assert_eq!(
            log.events.iter().map(|e| e.kind).collect::<Vec<_>>(),
            [EventKind::ObservationInvalidated, EventKind::Disconnected]
        );
        assert!(
            log.events
                .iter()
                .all(|e| e.receive_sequence == before.receive_sequence + 1)
        );
        assert!(client.chunk([1, 0]).await.is_err());
        assert!(client.chunk_context([0, 0]).await.is_err());
        let error = client.received_player_snapshot().await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains(&format!("play packet {id:#04x}"))
        );
        assert!(
            client
                .wait_for_events(log.cursor, Duration::from_secs(1))
                .await
                .is_err()
        );
        assert_eq!(
            client.events_after(before.cursor).await.unwrap().events,
            log.events
        );
    }
}

#[tokio::test]
async fn world_decode_failure_common_log_gap_is_explicit() {
    let (bot, server, release) = tests::ready_test_bot(ConnectionOptions::default()).await;
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let before = client.events_after(0).await.unwrap();
    // Unrelated traffic can evict a consumer's cursor. Never return a seemingly
    // complete suffix, even when the loss-of-observation event is retained.
    for _ in 0..crate::client::events::MAX_RETAINED_EVENTS {
        bot.emit(Event::ChunkLoaded { x: 1, z: 0 });
    }
    let recent = client.events_after(before.cursor).await.unwrap().cursor;
    bot.apply_packet(0x24, vec![]).await.unwrap_err();
    assert_eq!(
        client.events_after(before.cursor).await.unwrap_err().kind(),
        ErrorKind::State
    );
    assert_eq!(
        client
            .wait_for_events(before.cursor, Duration::ZERO)
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::State
    );
    assert_eq!(
        client.events_after(recent).await.unwrap().events[0].kind,
        EventKind::ObservationInvalidated
    );
    release.send(()).unwrap();
    client.disconnect().await.unwrap();
    drop(client);
    drop(bot);
    server.await.unwrap();
}
