//! Owner state transitions are published without re-querying control_record.
use super::*;
use crate::client::GameMode;
use crate::client::control::{ControlStateEvent, ControlStatus, ControlStopCause};

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

#[tokio::test]
async fn world_change_and_request_stops_are_logged_with_owner_and_typed_cause() {
    let (session, api, _peer) = common_ground_fixture(GameMode::Survival).await;
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    let handle = client.player_control(GameMode::Survival);
    let cursor = handle
        .control_state_changes_after(0)
        .await
        .unwrap()
        .sequence;
    assert_eq!(cursor, 0);
    let started = handle.start_control().await.unwrap();
    let old_world = session.state.lock().await.loading.generation;
    let log = handle.control_state_changes_after(cursor).await.unwrap();
    let [start] = &log.changes[..] else {
        panic!("{log:?}");
    };
    assert_eq!(start.transition.session_id, started.session_id);
    assert_eq!(start.transition.event, ControlStateEvent::Started);
    assert_eq!(
        start.transition.session.version,
        crate::MinecraftVersion::Java1_21_11
    );
    wait_status(&handle, false).await;
    // Respawn/dimension boundary: the running owner stops at its next tick.
    let respawn_sequence = {
        let mut state = session.state.lock().await;
        state.sequence += 1;
        state.loading.generation += 1;
        state.loading.attempt.as_mut().unwrap().generation = state.loading.generation;
        let mut pose = state.motion.received_pose.clone().unwrap();
        pose.generation = state.loading.generation;
        pose.receive_sequence = state.sequence;
        state.position = Some(pose.position);
        state.motion.receive(pose);
        operations::continuous::ground_motion_received(&mut state, ids::play_clientbound::RESPAWN);
        state.sequence
    };
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
    assert!(stop.dispatched_ticks > 0);
    assert!(matches!(
        &stop.event,
        ControlStateEvent::Stopped {
            cause: ControlStopCause::WorldChanged,
            reason,
        } if reason.starts_with("world changed")
    ));
    // A stopped owner is never relabelled by re-reads or later stops.
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
    // Explicit stop of a fresh owner in the new world.
    let next = handle.start_control().await.unwrap();
    wait_status(&handle, false).await;
    handle.stop_control_for(next.session_id).await.unwrap();
    let log = handle.control_state_changes_after(tail).await.unwrap();
    let events: Vec<_> = log
        .changes
        .iter()
        .map(|c| (c.transition.session_id, c.transition.event.clone()))
        .collect();
    assert_eq!(
        events,
        vec![
            (next.session_id, ControlStateEvent::Started),
            (
                next.session_id,
                ControlStateEvent::Stopped {
                    cause: ControlStopCause::Requested,
                    reason: "stopped by request".into(),
                }
            ),
        ]
    );
    assert_eq!(
        log.changes[0].transition.session.world_generation,
        old_world + 1
    );
    // Readable through another handle and after disconnect; same cursor rules.
    let other = client.player_control(GameMode::Creative);
    let all = other.control_state_changes_after(0).await.unwrap();
    assert_eq!(all.changes.len(), 4);
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
    session.stop();
}
