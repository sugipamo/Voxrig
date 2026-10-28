//! Explosion knockback reaching a running common PlayerControl (audit P10).
use super::*;
use crate::client::GameMode;
use crate::client::control::{ControlKind, ControlRecord, ControlStatus, Controls};
use std::time::Duration;
use tokio::time::timeout;

fn explosion(motion: [f32; 3]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for value in [8.5f32, 63.0, 8.5, 4.0] {
        bytes.extend(value.to_be_bytes());
    }
    bytes.extend(0i32.to_be_bytes());
    for value in motion {
        bytes.extend(value.to_be_bytes());
    }
    bytes
}

async fn ticks_after(
    control: &crate::client::PlayerControl,
    after: u64,
) -> crate::client::control::ControlRecord {
    timeout(Duration::from_secs(2), async {
        loop {
            let record = control.control_record().await.unwrap().unwrap();
            if record.dispatched_ticks >= after {
                break record;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap()
}

fn y(record: &ControlRecord) -> f64 {
    record.frame.as_ref().unwrap().position[1]
}

#[tokio::test]
async fn running_ground_control_applies_explosion_knockback_once() {
    let (bot, _packets, release, server) = tests::operation_test_bot(0x7fff, 0, vec![]).await;
    common_motion::tests::seed_motion(&bot).await;
    bot.player.lock().await.entity_id = Some(42);
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let control = client.player_control(GameMode::Survival);
    let start = control.start_control().await.unwrap();
    assert_eq!(start.explosion_impulses, 0);
    let settled = ticks_after(&control, 2).await;
    assert_eq!(y(&settled), 65.0);
    let before = control.control_record().await.unwrap().unwrap();
    bot.apply_packet(0x1c, explosion([0.0, 0.5, 0.0]))
        .await
        .unwrap();
    let pushed = control.control_record().await.unwrap().unwrap();
    assert_eq!(pushed.explosion_impulses, 1);
    assert_eq!(pushed.status, ControlStatus::Running);
    let after = ticks_after(&control, pushed.dispatched_ticks + 1).await;
    assert!(y(&after) > 65.0, "knockback lost: {before:?} -> {after:?}");
    // The impulse is consumed once: the arc peaks and falls back.
    let mut peak = y(&after);
    let mut tick = after.dispatched_ticks;
    loop {
        tick += 1;
        let next = ticks_after(&control, tick).await;
        if y(&next) < peak {
            break;
        }
        peak = y(&next);
        assert!(peak < 66.5, "impulse applied more than once: {peak}");
    }
    let landed = timeout(Duration::from_secs(4), async {
        loop {
            let record = control.control_record().await.unwrap().unwrap();
            if record.frame.as_ref().unwrap().on_ground {
                break record;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(y(&landed), 65.0);
    assert_eq!(landed.explosion_impulses, 1);
    assert_eq!(landed.velocity_updates, 0);
    let stopped = control.stop_control_for(start.session_id).await.unwrap();
    assert_eq!(stopped.explosion_impulses, 1);
    let _ = client.revoke_connection();
    drop(release);
    timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn running_flight_control_stops_on_explosion_without_another_movement() {
    let (bot, _packets, release, server) = tests::operation_test_bot(0x7fff, 0, vec![]).await;
    common_motion::tests::seed_motion(&bot).await;
    bot.player.lock().await.entity_id = Some(42);
    let mut mode = vec![3];
    mode.extend(1f32.to_be_bytes());
    bot.apply_packet(0x1e, mode).await.unwrap();
    let mut abilities = vec![6];
    abilities.extend(0.05f32.to_be_bytes());
    abilities.extend(0.1f32.to_be_bytes());
    bot.apply_packet(0x31, abilities).await.unwrap();
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let control = client.player_control(GameMode::Creative);
    let start = control
        .start_flight_control(Controls {
            jump: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(start.kind, ControlKind::Flight);
    let flying = ticks_after(&control, 2).await;
    bot.apply_packet(0x1c, explosion([0.0, 0.5, 0.0]))
        .await
        .unwrap();
    // The stop is recorded under the control lock: no later tick dispatches.
    let ticks = control
        .control_record()
        .await
        .unwrap()
        .unwrap()
        .dispatched_ticks;
    let stopped = timeout(Duration::from_secs(2), async {
        loop {
            let record = control.control_record().await.unwrap().unwrap();
            if matches!(record.status, ControlStatus::Stopped { .. }) {
                break record;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    assert!(flying.dispatched_ticks >= 2);
    assert_eq!(stopped.dispatched_ticks, ticks);
    assert_eq!(stopped.explosion_impulses, 0);
    assert_eq!(
        stopped.status,
        ControlStatus::Stopped {
            reason: "vehicle or explosion motion outside the engine".into()
        }
    );
    let _ = client.revoke_connection();
    drop(release);
    timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}
