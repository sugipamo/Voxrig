//! Owner state transitions are published without re-querying control_record.
use super::super::Bot;
use crate::client::GameMode;
use crate::client::control::{ControlStateEvent, ControlStatus, ControlStopCause};
use crate::protocol::put_string;
use std::time::Duration;
use tokio::time::timeout;

async fn wait_status(handle: &crate::client::PlayerControl, stopped: bool) {
    timeout(Duration::from_secs(2), async {
        loop {
            let record = handle.control_record().await.unwrap().unwrap();
            let done = if stopped {
                matches!(record.status, ControlStatus::Stopped { .. })
            } else {
                record.dispatched_ticks > 0
            };
            if done {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

async fn respawn(bot: &Bot) {
    bot.common_receipts.lock().await.selected_hotbar = Some(crate::client::received(0, 1));
    let mut respawn = Vec::new();
    put_string(&mut respawn, "minecraft:the_nether");
    put_string(&mut respawn, "minecraft:the_nether");
    respawn.extend(0i64.to_be_bytes());
    respawn.extend([0, 0, 0, 0, 1]);
    bot.apply_packet(0x3a, respawn).await.unwrap();
}

#[tokio::test]
async fn world_change_and_request_stops_are_logged_with_owner_and_typed_cause() {
    let (bot, _packets, release, server) =
        super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
    super::super::common_motion::tests::seed_motion(&bot).await;
    bot.player.lock().await.entity_id = Some(42);
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let handle = client.player_control(GameMode::Survival);
    assert_eq!(
        handle
            .control_state_changes_after(0)
            .await
            .unwrap()
            .sequence,
        0
    );
    let started = handle.start_control().await.unwrap();
    let old_world = bot.common_receipts.lock().await.generation;
    let log = handle.control_state_changes_after(0).await.unwrap();
    let [start] = &log.changes[..] else {
        panic!("{log:?}");
    };
    assert_eq!(start.transition.session_id, started.session_id);
    assert_eq!(start.transition.event, ControlStateEvent::Started);
    assert_eq!(
        start.transition.session.version,
        crate::MinecraftVersion::Java1_16_1
    );
    assert_eq!(start.transition.session.world_generation, old_world);
    wait_status(&handle, false).await;
    respawn(&bot).await;
    let respawn_sequence = bot
        .protocol_packet_sequence
        .load(std::sync::atomic::Ordering::Acquire);
    wait_status(&handle, true).await;
    let log = handle
        .control_state_changes_after(log.sequence)
        .await
        .unwrap();
    let [stop] = &log.changes[..] else {
        panic!("{log:?}");
    };
    let stop = &stop.transition;
    assert_eq!(stop.session_id, started.session_id);
    assert_eq!(stop.session.world_generation, old_world);
    assert!(stop.receive_sequence >= respawn_sequence);
    assert!(matches!(
        &stop.event,
        ControlStateEvent::Stopped {
            cause: ControlStopCause::WorldChanged,
            reason,
        } if reason.starts_with("world changed")
    ));
    // Re-reads and later stop calls never relabel the stopped owner.
    let tail = log.sequence;
    let _ = handle.stop_control_for(started.session_id).await;
    assert!(
        handle
            .control_state_changes_after(tail)
            .await
            .unwrap()
            .changes
            .is_empty()
    );
    // Readable through another handle and after disconnect; same cursor rules.
    let other = client.player_control(GameMode::Creative);
    let all = other.control_state_changes_after(0).await.unwrap();
    assert_eq!(all.changes.len(), 2);
    let _ = client.revoke_connection();
    assert_eq!(client.control_state_changes_after(0).await.unwrap(), all);
    assert_eq!(
        client
            .control_state_changes_after(all.sequence + 1)
            .await
            .unwrap_err()
            .kind(),
        crate::ErrorKind::InvalidInput
    );
    drop(release);
    timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn request_stop_is_logged_once_for_the_selected_owner() {
    let (bot, _packets, release, server) =
        super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
    super::super::common_motion::tests::seed_motion(&bot).await;
    bot.player.lock().await.entity_id = Some(42);
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let handle = client.player_control(GameMode::Survival);
    let started = handle.start_control().await.unwrap();
    wait_status(&handle, false).await;
    handle.stop_control_for(started.session_id).await.unwrap();
    let log = handle.control_state_changes_after(0).await.unwrap();
    let events: Vec<_> = log
        .changes
        .iter()
        .map(|c| (c.transition.session_id, c.transition.event.clone()))
        .collect();
    assert_eq!(
        events,
        vec![
            (started.session_id, ControlStateEvent::Started),
            (
                started.session_id,
                ControlStateEvent::Stopped {
                    cause: ControlStopCause::Requested,
                    reason: "stopped by request".into(),
                }
            ),
        ]
    );
    assert!(log.changes[1].transition.dispatched_ticks > 0);
    let _ = client.revoke_connection();
    drop(release);
    timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}
