use super::*;
use crate::client::EventKind;

pub(super) fn packet(position: [i32; 3], state: i32, status: i32, successful: bool) -> Vec<u8> {
    let [x, y, z] = position;
    let mut bytes = BlockPos { x, y, z }.packed().to_be_bytes().to_vec();
    put_varint(&mut bytes, state);
    put_varint(&mut bytes, status);
    bytes.push(u8::from(successful));
    bytes
}

#[tokio::test]
async fn digging_ack_common_consumer_sees_all_actions_rejections_and_unmatched_corrections() {
    let (bot, server, release) = tests::ready_test_bot(ConnectionOptions::default()).await;
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let clone = client.clone();
    bot.apply_packet(0x21, tests::empty_chunk_packet(-1, 0))
        .await
        .unwrap();
    let position = [-1, 64, 2];
    let mut cursor = client.events_after(0).await.unwrap().cursor;
    for status in 0..=2 {
        for successful in [false, true] {
            // A success carrying non-air is still a received fact, not removal.
            // Repeated equal states must retain the separate rejection/acceptance.
            bot.apply_packet(0x07, packet(position, 42, status, successful))
                .await
                .unwrap();
            let log = clone.wait_for_events(cursor, Duration::ZERO).await.unwrap();
            assert_eq!(
                log.events.iter().map(|e| e.kind).collect::<Vec<_>>(),
                vec![
                    EventKind::BlocksChanged {
                        min: position,
                        max: position
                    },
                    EventKind::LegacyDiggingAcknowledged {
                        position,
                        block_state_id: 42,
                        status,
                        successful,
                    },
                ]
            );
            assert!(
                log.events
                    .iter()
                    .all(|e| e.receive_sequence == log.receive_sequence)
            );
            let chunk = client.chunk([-1, 0]).await.unwrap().unwrap();
            assert_eq!(chunk.state_id(position), Some(42));
            cursor = log.cursor;
        }
    }
    // No loaded chunk is fabricated by an ACK, but its raw correction is exposed.
    let unloaded = [48, 64, 0];
    bot.apply_packet(0x07, packet(unloaded, 1, 2, false))
        .await
        .unwrap();
    let log = client.events_after(cursor).await.unwrap();
    assert!(matches!(log.events[1].kind,
        EventKind::LegacyDiggingAcknowledged { position, successful: false, .. } if position == unloaded));
    assert!(client.chunk([3, 0]).await.unwrap().is_none());
    // World boundaries and closure remain in the same ordered log.
    bot.apply_packet(0x3a, tests::respawn_packet())
        .await
        .unwrap();
    let boundary = client.events_after(log.cursor).await.unwrap();
    assert!(
        boundary
            .events
            .iter()
            .any(|e| e.kind == EventKind::WorldChanged)
    );
    release.send(()).unwrap();
    client.disconnect().await.unwrap();
    assert_eq!(
        client.events_after(cursor).await.unwrap().events[..2],
        log.events
    );
    drop(clone);
    drop(client);
    drop(bot);
    server.await.unwrap();
}

#[tokio::test]
async fn digging_ack_event_eviction_is_an_explicit_error() {
    let (bot, server, release) = tests::ready_test_bot(ConnectionOptions::default()).await;
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let first = client.events_after(0).await.unwrap();
    let mut recent_cursor = first.cursor;
    for index in 0..=crate::client::events::MAX_RETAINED_EVENTS / 2 {
        bot.apply_packet(0x07, packet([0, 64, 0], 1, 2, false))
            .await
            .unwrap();
        if index == crate::client::events::MAX_RETAINED_EVENTS / 2 - 1 {
            recent_cursor = client.events_after(first.cursor).await.unwrap().cursor;
        }
    }
    assert_eq!(
        client.events_after(first.cursor).await.unwrap_err().kind(),
        crate::ErrorKind::State
    );
    assert_eq!(
        client
            .wait_for_events(first.cursor, Duration::ZERO)
            .await
            .unwrap_err()
            .kind(),
        crate::ErrorKind::State
    );
    let tail = client.events_after(recent_cursor).await.unwrap();
    assert_eq!(tail.events.len(), 2);
    assert!(matches!(
        tail.events[1].kind,
        EventKind::LegacyDiggingAcknowledged {
            successful: false,
            ..
        }
    ));
    release.send(()).unwrap();
    client.disconnect().await.unwrap();
    drop(client);
    drop(bot);
    server.await.unwrap();
}

#[tokio::test]
async fn malformed_digging_ack_publishes_no_correction_or_result() {
    let (bot, server, release) = tests::ready_test_bot(ConnectionOptions::default()).await;
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let before = client.events_after(0).await.unwrap();
    let mut bytes = packet([0, 64, 0], 1, 2, false);
    bytes.pop();
    assert!(bot.apply_packet(0x07, bytes).await.is_err());
    assert!(
        client
            .events_after(before.cursor)
            .await
            .unwrap()
            .events
            .is_empty()
    );
    release.send(()).unwrap();
    client.disconnect().await.unwrap();
    drop(client);
    drop(bot);
    server.await.unwrap();
}
