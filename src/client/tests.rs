use super::*;

pub(crate) async fn common_motion_preview_scenario(client: &Client) {
    use super::survival::{SurvivalControl, SurvivalInput, TerminalClearance};
    let before = client.player_state().await.unwrap();
    let controls: Vec<_> = (0..35)
        .map(|tick| SurvivalControl {
            yaw: 35.57,
            input: SurvivalInput {
                forward: i8::from(tick < 5),
                jump: tick == 0,
                ..Default::default()
            },
        })
        .collect();
    let preview = client.survival().preview_path(&controls).await.unwrap();
    assert_eq!(preview.initial.session, before.session);
    assert_eq!(
        preview.initial_frame.position,
        before.position.as_ref().unwrap().value
    );
    assert_eq!(preview.controls, controls);
    assert_eq!(preview.frames.len(), controls.len());
    assert_eq!(
        preview.frames.last().unwrap().position[1],
        preview.initial_frame.position[1]
    );
    assert!(preview.frames.last().unwrap().resting);
    assert!(
        preview
            .frames
            .iter()
            .any(|frame| frame.position[1] > preview.initial_frame.position[1] + 1.2)
    );
    assert!(matches!(
        preview.terminal_clearance,
        TerminalClearance::Admitted { .. }
    ));
    let after = client.player_state().await.unwrap();
    assert_eq!(before.position, after.position);
    assert_eq!(before.received_pose, after.received_pose);
    assert!(client.survival().preview_path(&[]).await.is_err());
    assert!(
        client
            .survival()
            .preview_path(&vec![controls[0]; 121])
            .await
            .is_err()
    );
    for control in [
        SurvivalControl {
            yaw: f32::NAN,
            input: SurvivalInput::default(),
        },
        SurvivalControl {
            yaw: 0.0,
            input: SurvivalInput {
                forward: 2,
                ..Default::default()
            },
        },
    ] {
        assert!(client.survival().preview_path(&[control]).await.is_err());
    }
}

// Exactly the same consumer calls exercise both adapters. Native fixtures are
// established by each adapter's tests, outside this shared consumer body.
pub(crate) async fn common_creative_scenario(client: &Client) {
    assert_eq!(
        client.player_state().await.unwrap().game_mode,
        Some(GameMode::Creative)
    );
    assert!(client.survival().select_hotbar(0).await.is_err());
    assert!(client.survival().look([0.0, 0.0]).await.is_err());
    assert!(client.creative().look([0.0, 91.0]).await.is_err());
    let capture = client
        .capture(crate::Region {
            min: [0, 0, 1],
            max: [0, 0, 1],
        })
        .await
        .unwrap();
    assert_eq!(
        capture.world.connection_id,
        capture.player.session.connection_id
    );
    assert_eq!(capture.world.version, capture.player.session.version);
    assert_eq!(
        capture.world.receive_sequence,
        Some(capture.player.receive_sequence)
    );
    assert!(capture.world.blocks[0].state.is_some());
    let ops = client.creative();
    let receipt = ops
        .set_hotbar(0, Some(("minecraft:stone", 1)))
        .await
        .unwrap();
    assert_eq!(receipt.version, client.version());
    let observation = ops.player_state().await.unwrap();
    assert!(!matches!(&observation.inventory.slots[36], Some(slot)
        if matches!(slot.value, SlotKnowledge::Item { .. })));
    ops.look([10.0, 0.0]).await.unwrap();
    ops.select_hotbar(0).await.unwrap();
    ops.set_flying(true).await.unwrap();
    ops.move_flying([0.5, 2.0, 0.5], [10.0, 0.0]).await.unwrap();
    ops.break_block([0, 0, 1], crate::BlockFace::Up)
        .await
        .unwrap();
    ops.use_on_block([0, 0, 1], crate::BlockFace::Up, [0.5; 3])
        .await
        .unwrap();
    assert_eq!(
        ops.player_state()
            .await
            .unwrap()
            .received_pose
            .unwrap()
            .position,
        [0.5, 1.0, 0.5]
    );
    assert!(ops.move_flying([10.5, 2.0, 0.5], [0.0; 2]).await.is_err());
}

#[test]
fn setup_only_accepts_exact_implemented_versions() {
    for name in ["1.16.1", "1.21.11"] {
        let version: crate::MinecraftVersion = name.parse().unwrap();
        assert_eq!(version.to_string(), name);
    }
    for name in ["latest", "1.16", "1.99.1", " 1.16.1"] {
        assert!(name.parse::<crate::MinecraftVersion>().is_err());
    }
    for version in [
        crate::MinecraftVersion::Java1_16_1,
        crate::MinecraftVersion::Java1_21_11,
    ] {
        let mut config = ConnectionConfig::offline(Server::default(), "Probe", version);
        config.validate().unwrap();
        config.limits.max_chunks = 0;
        assert!(config.validate().is_err());
        assert_eq!(
            Capabilities::for_version(version).support(Feature::Containers),
            Support::NotImplemented
        );
    }
}

pub(crate) async fn common_motion_dispatch_scenario(client: &Client) {
    use super::survival::{MotionStatus, SurvivalControl, SurvivalInput};
    let controls: Vec<_> = (0..35)
        .map(|tick| SurvivalControl {
            yaw: 35.57,
            input: SurvivalInput {
                forward: i8::from(tick < 5),
                jump: tick == 0,
                ..Default::default()
            },
        })
        .collect();
    let before = client.player_state().await.unwrap();
    let ops = client.survival();
    let started = ops.start_predicted_path(&controls).await.unwrap();
    assert_eq!(started.status, MotionStatus::Running);
    assert_eq!(started.dispatched_ticks, 0);
    assert_eq!(started.attempted_tick, 0);
    assert_eq!(started.preview.initial.received_pose, before.received_pose);
    assert_eq!(started.session, before.session);
    assert!(ops.select_hotbar(0).await.is_err());
    assert!(ops.start_predicted_path(&controls).await.is_err());
    // Abort only a caller's observation wait. The finite owner must still dispatch.
    let waiting_ops = ops.clone();
    let waiting = tokio::spawn(async move {
        loop {
            let _ = waiting_ops.motion_record().await;
            tokio::task::yield_now().await;
        }
    });
    waiting.abort();
    let finished = tokio::time::timeout(std::time::Duration::from_secs(6), async {
        loop {
            let run = ops.motion_record().await.unwrap().unwrap();
            assert_eq!(run.run_id, started.run_id);
            if run.status != MotionStatus::Running {
                break run;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        finished.status,
        MotionStatus::Predicted,
        "{:?}",
        finished.problem
    );
    assert_eq!(finished.attempted_tick, 35);
    assert_eq!(finished.dispatched_ticks, 35);
    assert!(finished.problem.is_none());
    let after = client.player_state().await.unwrap();
    assert_eq!(after.received_pose, before.received_pose);
    assert_eq!(
        after.position.as_ref().unwrap().value,
        finished.preview.frames.last().unwrap().position
    );
    assert!(matches!(
        after.position.as_ref().unwrap().source,
        ValueSource::Submitted | ValueSource::Predicted
    ));
    assert!(!after.pending_dispatch);
    // Fresh endpoint admission allows a second common run; diagnostics don't authorize it.
    let next = ops
        .start_predicted_path(
            &[SurvivalControl {
                yaw: 0.0,
                input: Default::default(),
            }; 2],
        )
        .await
        .unwrap();
    assert_eq!(next.run_id, started.run_id + 1);
    assert_eq!(
        next.preview.initial_frame.position,
        after.position.unwrap().value
    );
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let run = ops.motion_record().await.unwrap().unwrap();
            if run.status != MotionStatus::Running {
                assert_eq!(run.status, MotionStatus::Predicted, "{:?}", run.problem);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
