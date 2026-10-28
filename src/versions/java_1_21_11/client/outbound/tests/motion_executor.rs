//! Packet fixtures for finite and continuous strategies of one executor.
use super::*;
use crate::client::{
    GameMode,
    adapter::ControlOps,
    survival::{MotionStatus, SurvivalControl, SurvivalInput},
};

#[tokio::test]
async fn executor_interruptions_discard_tail_neutralize_and_inherit_current_model() {
    for cause in ["correction", "impulse", "terrain", "world"] {
        for sent in [1u16, 2] {
            let (session, api, mut peer) = common_ground_fixture(GameMode::Survival).await;
            let client = crate::Client::from_java_1_21_11(api.bot.clone());
            let controls: Vec<_> = (0..35)
                .map(|tick| SurvivalControl {
                    yaw: 0.0,
                    input: SurvivalInput {
                        forward: i8::from(tick < 10),
                        ..Default::default()
                    },
                })
                .collect();
            let old = client
                .survival()
                .start_predicted_path(&controls)
                .await
                .unwrap();
            for _ in 0..sent {
                assert_eq!(
                    read_packet(&mut peer, None).await.unwrap().0,
                    ids::play_serverbound::PLAYER_INPUT
                );
                assert_eq!(
                    read_packet(&mut peer, None).await.unwrap().0,
                    ids::play_serverbound::POSITION_LOOK
                );
            }
            let stale = session.state.lock().await.survival_motion.clone().unwrap();
            let interrupted_at = {
                let mut state = session.state.lock().await;
                interrupt(&mut state, cause);
                if cause == "impulse" {
                    assert_eq!(
                        operations::continuous::ground_velocity(&state),
                        state.operations.local_player.velocity.map(|v| v.value)
                    );
                }
                state.sequence
            };
            timeout(Duration::from_secs(2), async {
                while session.state.lock().await.control.finite.is_some() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            let record = client.survival().motion_record().await.unwrap().unwrap();
            assert_eq!(record.status, MotionStatus::RequiresInspection, "{cause}");
            assert_eq!(record.dispatched_ticks, sent, "old tail sent: {cause}");
            // Typed received cause beside the unchanged free-text problem;
            // a local terrain stop is not an interruption.
            use crate::client::survival::MotionInterruptionKind as Kind;
            assert!(record.problem.is_some(), "{cause}");
            let kind = record.interruption.clone().map(|i| i.kind);
            match cause {
                "correction" => assert_eq!(
                    kind,
                    Some(Kind::Correction {
                        position: [9.5, 65.0, 8.5],
                        rotation: [0.0; 2],
                    })
                ),
                "impulse" => assert!(
                    matches!(kind, Some(Kind::Velocity { velocity: Some(v) }) if v[0] > 0.0),
                    "{kind:?}"
                ),
                "terrain" => assert_eq!(kind, None),
                _ => assert_eq!(kind, Some(Kind::WorldChanged)),
            }
            if let Some(interruption) = &record.interruption {
                assert_eq!(interruption.receive_sequence, interrupted_at, "{cause}");
            }
            if cause != "world" {
                assert_eq!(
                    read_packet(&mut peer, None).await.unwrap(),
                    (ids::play_serverbound::PLAYER_INPUT, vec![0])
                );
            }
            assert!(
                timeout(Duration::from_millis(1), read_packet(&mut peer, None))
                    .await
                    .is_err(),
                "unexpected old tail: {cause}"
            );
            // New strict execution is checked from current momentum, not the old preview endpoint.
            let rest: Vec<_> = (0..35)
                .map(|tick| SurvivalControl {
                    yaw: 180.,
                    input: SurvivalInput {
                        forward: i8::from(tick < 5),
                        ..Default::default()
                    },
                })
                .collect();
            let momentum = {
                let state = session.state.lock().await;
                state
                    .control
                    .session
                    .as_ref()
                    .map(|s| s.body.velocity)
                    .or_else(|| state.motion.received_pose.as_ref().and_then(|p| p.velocity))
                    .unwrap()
            };
            let next = client
                .survival()
                .start_predicted_path(&rest)
                .await
                .unwrap_or_else(|e| panic!("{cause}/{sent}: {e}"));
            assert!(next.run_id > old.run_id);
            assert_eq!(next.preview.initial_frame.velocity, momentum);
            assert!(
                api.run_survival_motion(&stale, None, [1; 16])
                    .await
                    .is_err()
            );
            assert_eq!(
                session
                    .state
                    .lock()
                    .await
                    .survival_motion
                    .as_ref()
                    .unwrap()
                    .run_id,
                next.run_id
            );
            for _ in &rest {
                assert_eq!(
                    read_packet(&mut peer, None).await.unwrap().0,
                    ids::play_serverbound::PLAYER_INPUT
                );
                assert_eq!(
                    read_packet(&mut peer, None).await.unwrap().0,
                    ids::play_serverbound::POSITION_LOOK
                );
            }
            timeout(Duration::from_secs(1), async {
                while session.state.lock().await.control.finite.is_some() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            let control = api
                .start_path_best_effort(GameMode::Survival, &rest[..2])
                .await
                .unwrap();
            assert!(control.session_id > 0);
            assert_eq!(
                read_packet(&mut peer, None).await.unwrap().0,
                ids::play_serverbound::PLAYER_INPUT
            );
            assert_eq!(
                read_packet(&mut peer, None).await.unwrap().0,
                ids::play_serverbound::POSITION_LOOK
            );
            api.stop_control().await.unwrap();
            // A completed historical strict endpoint cannot block the newer Body model.
            let latest = client.survival().start_predicted_path(&rest).await.unwrap();
            assert!(latest.run_id > next.run_id);
            session.stop();
        }
    }
}

#[tokio::test(start_paused = true)]
async fn executor_input_position_pair_serializes_invalidation_and_old_worker_cannot_release_new_owner()
 {
    for cause in ["correction", "impulse", "terrain", "world"] {
        let (session, api, mut peer) = common_ground_fixture(GameMode::Survival).await;
        let entered = Arc::new(Notify::new());
        let resume = Arc::new(Notify::new());
        let client = crate::Client::from_java_1_21_11(api.bot.clone());
        let controls: Vec<_> = (0..35)
            .map(|t| SurvivalControl {
                yaw: 0.,
                input: SurvivalInput {
                    forward: i8::from(t < 10),
                    ..Default::default()
                },
            })
            .collect();
        client
            .survival()
            .start_predicted_path(&controls)
            .await
            .unwrap();
        let stale = {
            let mut state = session.state.lock().await;
            state.control.finite.as_mut().unwrap().between_packets =
                Some((entered.clone(), resume.clone()));
            state.survival_motion.clone().unwrap()
        };
        entered.notified().await;
        assert_eq!(
            read_packet(&mut peer, None).await.unwrap().0,
            ids::play_serverbound::PLAYER_INPUT
        );
        let correction_session = session.clone();
        let mut correction = Box::pin(async move {
            let mut state = correction_session.state.lock().await;
            interrupt(&mut state, cause);
        });
        pending(correction.as_mut()).await; // State excludes correction between pair sends
        resume.notify_one();
        correction.await;
        tokio::time::advance(Duration::from_millis(50)).await;
        assert_eq!(
            read_packet(&mut peer, None).await.unwrap().0,
            ids::play_serverbound::POSITION_LOOK
        );
        if cause != "world" {
            assert_eq!(
                read_packet(&mut peer, None).await.unwrap(),
                (ids::play_serverbound::PLAYER_INPUT, vec![0])
            );
        }
        timeout(Duration::from_secs(1), async {
            while session.state.lock().await.control.finite.is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let next = api
            .start_control_best_effort(GameMode::Survival)
            .await
            .unwrap();
        assert!(
            api.run_survival_motion(&stale, None, [1; 16])
                .await
                .is_err()
        );
        let record = api.control_record().await.unwrap().unwrap();
        assert_eq!(record.session_id, next.session_id);
        assert!(matches!(
            record.status,
            crate::client::control::ControlStatus::Running
        ));
        api.stop_control().await.unwrap();
        session.stop();
    }
}

fn impulse() -> Vec<u8> {
    // Pinned native 1.21.11 packed vec3, small positive X (not legacy shorts).
    let packed = 1u64 | (17694u64 << 3) | (16383u64 << 18) | (16383u64 << 33);
    let mut value = vec![42, packed as u8, (packed >> 8) as u8];
    value.extend(((packed >> 16) as u32).to_be_bytes());
    value
}

fn interrupt(state: &mut State, cause: &str) {
    state.sequence += 1;
    match cause {
        "correction" => {
            let mut pose = vec![1];
            for v in [9.5f64, 65., 8.5, 0.125, 0., 0.0625] {
                pose.extend(v.to_be_bytes());
            }
            pose.extend([0; 12]);
            state
                .receive(ids::play_clientbound::POSITION, &pose, 256)
                .unwrap();
        }
        "impulse" => {
            let v = impulse();
            state
                .receive(ids::play_clientbound::ENTITY_VELOCITY, &v, 256)
                .unwrap();
        }
        "terrain" => {
            state.world.seed_replay_cell([8, 65, 10], 1);
        }
        _ => {
            state.loading.generation += 1;
            state.loading.attempt.as_mut().unwrap().generation = state.loading.generation;
            // Complete new-world baseline. No old-world input release is allowed.
            let mut pose = state.motion.received_pose.clone().unwrap();
            pose.generation = state.loading.generation;
            pose.receive_sequence = state.sequence;
            state.position = Some(pose.position);
            state.motion.receive(pose);
            operations::continuous::ground_motion_received(state, ids::play_clientbound::RESPAWN);
        }
    }
}
