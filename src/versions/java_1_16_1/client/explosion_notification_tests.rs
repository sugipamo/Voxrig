use super::*;
use crate::client::{EventKind, explosions::*};

fn packet(offsets: &[[i8; 3]], motion: [f32; 3]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for value in [-0.5f32, 64.25, 15.75, 4.0] {
        bytes.extend(value.to_be_bytes());
    }
    bytes.extend((offsets.len() as i32).to_be_bytes());
    for offset in offsets {
        bytes.extend(offset.map(|v| v as u8));
    }
    for value in motion {
        bytes.extend(value.to_be_bytes());
    }
    bytes
}

#[tokio::test]
async fn explosion_notifies_common_consumers_with_applied_bounds_and_original_context() {
    let (bot, server, release) = tests::ready_test_bot(ConnectionOptions::default()).await;
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let clone = client.clone();
    let positions = [[-2, 64, 15], [1, 65, 16], [-2, 64, 15]];
    for (x, z) in [(-1, 0), (0, 1)] {
        bot.apply_packet(0x21, tests::empty_chunk_packet(x, z))
            .await
            .unwrap();
    }
    for [x, y, z] in positions {
        bot.world
            .lock()
            .await
            .set_block_for_test(BlockPos { x, y, z }, 1);
    }
    let cursor = client.events_after(0).await.unwrap().cursor;
    bot.apply_packet(
        0x1c,
        packet(&[[-1, 0, 0], [2, 1, 1], [-1, 0, 0]], [-0.25, 0.5, 1.25]),
    )
    .await
    .unwrap();
    let log = clone.explosions_after(0).await.unwrap();
    assert_eq!(log.explosions.len(), 1);
    let record = &log.explosions[0];
    assert_eq!(record.session.version, crate::MinecraftVersion::Java1_16_1);
    assert_eq!(record.explosion.center, [-0.5, 64.25, 15.75]);
    assert_eq!(record.explosion.radius, 4.0);
    assert_eq!(record.explosion.block_count, 3);
    assert_eq!(record.explosion.player_knockback, Some([-0.25, 0.5, 1.25]));
    assert_eq!(
        record.explosion.presentation,
        ExplosionPresentation::Java1_16_1 {
            affected_blocks: positions.to_vec()
        }
    );
    let changes = client.events_after(cursor).await.unwrap();
    assert_eq!(
        changes.events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![
            EventKind::BlocksChanged {
                min: [-2, 64, 15],
                max: [1, 65, 16]
            },
            EventKind::ExplosionReceived,
        ]
    );
    assert!(
        changes
            .events
            .iter()
            .all(|e| e.receive_sequence == record.receive_sequence)
    );
    for [x, y, z] in positions {
        assert_eq!(bot.world.lock().await.block(x, y, z), Some(0));
    }
    assert!(
        client
            .explosions_after(log.receive_sequence)
            .await
            .unwrap()
            .explosions
            .is_empty()
    );
    assert_eq!(
        client.explosions_after(0).await.unwrap().explosions,
        log.explosions
    );

    // A world transition must not relabel an earlier receipt.
    bot.apply_packet(0x3a, tests::respawn_packet())
        .await
        .unwrap();
    let cursor = client.events_after(changes.cursor).await.unwrap().cursor;
    bot.apply_packet(0x1c, packet(&[], [0.; 3])).await.unwrap();
    let later = client.explosions_after(log.receive_sequence).await.unwrap();
    assert_eq!(
        later.explosions[0].session.world_generation,
        bot.common_receipts.lock().await.generation
    );
    assert_ne!(
        later.explosions[0].session.world_generation,
        record.session.world_generation
    );
    assert_eq!(
        later.explosions[0].session.connection_id,
        record.session.connection_id
    );
    assert_eq!(
        later.explosions[0].explosion.player_knockback,
        Some([0.; 3])
    );
    assert_eq!(
        client
            .events_after(cursor)
            .await
            .unwrap()
            .events
            .iter()
            .map(|e| e.kind)
            .collect::<Vec<_>>(),
        vec![EventKind::ExplosionReceived]
    );
    release.send(()).unwrap();
    client.disconnect().await.unwrap();
    assert_eq!(
        client.explosions_after(0).await.unwrap().explosions[0],
        *record
    );
    drop(clone);
    drop(client);
    drop(bot);
    server.await.unwrap();
}

#[tokio::test]
async fn explosion_retention_reports_packet_and_position_gaps_without_rejecting_reception() {
    let (bot, server, release) = tests::ready_test_bot(ConnectionOptions::default()).await;
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let empty = packet(&[], [0.; 3]);
    bot.apply_packet(0x1c, empty.clone()).await.unwrap();
    let first = client.explosions_after(0).await.unwrap();
    let first_sequence = first.explosions[0].receive_sequence;
    for _ in 0..MAX_RETAINED_EXPLOSIONS {
        bot.apply_packet(0x1c, empty.clone()).await.unwrap();
    }
    assert_eq!(
        client.explosions_after(0).await.unwrap_err().kind(),
        crate::ErrorKind::State
    );
    assert_eq!(
        client
            .explosions_after(first_sequence)
            .await
            .unwrap()
            .explosions
            .len(),
        MAX_RETAINED_EXPLOSIONS
    );
    let tail = client
        .explosions_after(first_sequence)
        .await
        .unwrap()
        .receive_sequence;
    assert_eq!(
        client.explosions_after(tail + 1).await.unwrap_err().kind(),
        crate::ErrorKind::InvalidInput
    );

    // Large accepted packets exceed the storage budget, rather than gaining a
    // new protocol rejection or returning a truncated position list as complete.
    bot.apply_packet(
        0x1c,
        packet(&vec![[0; 3]; MAX_RETAINED_EXPLOSION_BLOCKS + 1], [0.; 3]),
    )
    .await
    .unwrap();
    assert_eq!(
        client.explosions_after(tail).await.unwrap_err().kind(),
        crate::ErrorKind::State
    );
    let now = client.events_after(0).await.unwrap().receive_sequence;
    assert!(
        client
            .explosions_after(now)
            .await
            .unwrap()
            .explosions
            .is_empty()
    );
    assert!(
        client
            .events_after(0)
            .await
            .unwrap()
            .events
            .iter()
            .any(|e| e.receive_sequence == now && e.kind == EventKind::ExplosionReceived)
    );

    // The position budget also applies to the sum of several retained packets.
    let half = packet(
        &vec![[0; 3]; MAX_RETAINED_EXPLOSION_BLOCKS / 2 + 1],
        [0.; 3],
    );
    bot.apply_packet(0x1c, half.clone()).await.unwrap();
    let first_large = client.explosions_after(now).await.unwrap();
    bot.apply_packet(0x1c, half).await.unwrap();
    assert_eq!(
        client.explosions_after(now).await.unwrap_err().kind(),
        crate::ErrorKind::State
    );
    assert_eq!(
        client
            .explosions_after(first_large.receive_sequence)
            .await
            .unwrap()
            .explosions
            .len(),
        1
    );

    // Common event eviction is also explicit, independently of explosion storage.
    for _ in 0..crate::client::events::MAX_RETAINED_EVENTS {
        bot.apply_packet(0x1c, empty.clone()).await.unwrap();
    }
    assert_eq!(
        client.events_after(0).await.unwrap_err().kind(),
        crate::ErrorKind::State
    );
    release.send(()).unwrap();
    client.disconnect().await.unwrap();
    drop(client);
    drop(bot);
    server.await.unwrap();
}

#[tokio::test]
async fn malformed_explosion_never_publishes_successful_receipt_or_invalidation() {
    let (bot, server, release) = tests::ready_test_bot(ConnectionOptions::default()).await;
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let cursor = client.events_after(0).await.unwrap().cursor;
    let mut truncated = packet(&[[0; 3]], [0.; 3]);
    truncated.pop();
    assert!(bot.apply_packet(0x1c, truncated).await.is_err());
    assert!(
        client
            .explosions_after(0)
            .await
            .unwrap()
            .explosions
            .is_empty()
    );
    assert!(client.events_after(cursor).await.unwrap().events.is_empty());
    release.send(()).unwrap();
    client.disconnect().await.unwrap();
    drop(client);
    drop(bot);
    server.await.unwrap();
}
