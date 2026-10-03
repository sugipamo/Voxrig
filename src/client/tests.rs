use super::*;

pub(crate) async fn common_swap_start_scenario(
    client: &Client,
    mode: GameMode,
    main: u8,
    hotbar: u8,
) -> inventory::InventorySwapRecord {
    let before = client.player_state().await.unwrap();
    let record = match mode {
        GameMode::Survival => client.survival().swap_hotbar(main, hotbar).await,
        GameMode::Creative => client.creative().swap_hotbar(main, hotbar).await,
        _ => panic!("fixture mode"),
    }
    .unwrap();
    assert_eq!(record.id.session(), before.session);
    assert_eq!(record.initial.receive_sequence, before.receive_sequence);
    assert_eq!(
        record.main_before,
        before.inventory.slots[usize::from(main)].clone().unwrap()
    );
    assert_eq!(
        record.hotbar_before,
        before.inventory.slots[36 + usize::from(hotbar)]
            .clone()
            .unwrap()
    );
    assert!(record.send.dispatched);
    assert_eq!(record.stage, inventory::InventorySwapStage::Pending);
    let after = client.player_state().await.unwrap();
    assert_eq!(
        after.inventory.slots[usize::from(main)],
        Some(record.main_before.clone())
    );
    assert_eq!(
        after.inventory.slots[36 + usize::from(hotbar)],
        Some(record.hotbar_before.clone())
    );
    assert!(client.survival().swap_hotbar(main, hotbar).await.is_err());
    assert!(client.creative().swap_hotbar(main, hotbar).await.is_err());
    assert!(client.survival().select_hotbar(1).await.is_err());
    assert!(client.creative().select_hotbar(1).await.is_err());
    record
}
pub(crate) async fn common_swap_pending_scenario(client: &Client) {
    let record = client
        .survival()
        .inventory_swap_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.stage, inventory::InventorySwapStage::Pending);
    assert!(
        client
            .survival()
            .swap_hotbar(record.main_slot, record.hotbar)
            .await
            .is_err()
    );
}
pub(crate) async fn common_swap_completed_scenario(
    client: &Client,
    id: inventory::InventorySwapId,
) -> inventory::InventorySwapRecord {
    let record = client
        .survival()
        .inventory_swap_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.id, id);
    assert_eq!(record.stage, inventory::InventorySwapStage::ObservedSwapped);
    assert_eq!(
        record.main_receipt.as_ref().unwrap().value,
        record.hotbar_before.value
    );
    assert_eq!(
        record.hotbar_receipt.as_ref().unwrap().value,
        record.main_before.value
    );
    for receipt in [&record.main_receipt, &record.hotbar_receipt] {
        assert!(
            matches!(receipt.as_ref().unwrap().source,ValueSource::Received{sequence} if sequence>record.send.after_sequence)
        );
    }
    record
}

#[test]
fn legacy_raw_inventory_receipts_use_screen_slots_without_inventing_menu_or_cursor() {
    use super::observation::LegacyReceipts;
    use crate::versions::java_1_16_1::{ItemStack as NativeStack, SlotUpdate};
    let mut receipts = LegacyReceipts::default();
    receipts.inventory.window_id = Some(7);
    let expected: Vec<usize> = (36..45)
        .chain(9..36)
        .chain((5..9).rev())
        .chain([45])
        .collect();
    for (raw, screen) in expected.into_iter().enumerate() {
        let sequence = raw as u64 + 10;
        receipts
            .slot(&SlotUpdate {
                window_id: -2,
                slot: raw as i16,
                item: Some(NativeStack {
                    item_id: 1,
                    count: 1,
                    nbt: None,
                }),
                packet_sequence: sequence,
            })
            .unwrap();
        let slot = receipts.inventory.slots[screen].as_ref().unwrap();
        assert!(matches!(slot.value, SlotKnowledge::Item { .. }));
        assert_eq!(slot.source, ValueSource::Received { sequence });
    }
    assert_eq!(receipts.inventory.window_id, Some(7));
    assert!(receipts.inventory.cursor.is_none());
    assert!(receipts.inventory.slots[..5].iter().all(Option::is_none));
    assert!(receipts.inventory.local_cache.is_none());
    receipts
        .slot(&SlotUpdate {
            window_id: -2,
            slot: 0,
            item: None,
            packet_sequence: 70,
        })
        .unwrap();
    assert_eq!(
        receipts.inventory.slots[36].as_ref().unwrap(),
        &ObservedValue {
            value: SlotKnowledge::Empty,
            source: ValueSource::Received { sequence: 70 }
        }
    );
    let before = serde_json::to_value(&receipts.inventory).unwrap();
    receipts
        .slot(&SlotUpdate {
            window_id: -2,
            slot: 41,
            item: None,
            packet_sequence: 71,
        })
        .unwrap();
    assert_eq!(serde_json::to_value(&receipts.inventory).unwrap(), before);
}

// Same consumer and fixtures on both adapters: dry floor at y=64, no other blocks.
// Only look sends a packet; the outline reads must leave the capture unchanged.
pub(crate) async fn common_target_scenario(client: &Client) {
    assert!(
        client
            .survival()
            .target_block(4.5)
            .await
            .unwrap()
            .hit
            .is_none()
    );
    client.survival().look([0.0, 90.0]).await.unwrap();
    let before = client.player_state().await.unwrap();
    let query = client.survival().target_block(4.5).await.unwrap();
    assert_eq!(query.initial.session, before.session);
    assert_eq!(query.initial.position, before.position);
    assert_eq!(query.initial.received_pose, before.received_pose);
    assert_eq!(query.eye, [8.5, 65.0 + f64::from(1.62f32), 8.5]);
    let hit = query.hit.unwrap();
    assert_eq!(hit.position, [8, 64, 8]);
    assert_eq!(hit.state.name, "minecraft:stone");
    assert_eq!(hit.face, BlockFace::Up);
    assert!((hit.point[1] - 65.0).abs() < 1e-12);
    assert!((hit.distance - f64::from(1.62f32)).abs() < 1e-12);
    assert!(
        client
            .survival()
            .target_block(1.0)
            .await
            .unwrap()
            .hit
            .is_none()
    );
    for reach in [0.0, -1.0, f64::NAN, f64::INFINITY, 4.5001] {
        assert_eq!(
            client
                .survival()
                .target_block(reach)
                .await
                .unwrap_err()
                .kind(),
            crate::ErrorKind::InvalidInput
        );
    }
    let after = client.player_state().await.unwrap();
    assert_eq!(before.position, after.position);
    assert_eq!(before.received_pose, after.received_pose);
    assert_eq!(before.rotation, after.rotation);
    assert_eq!(before.receive_sequence, after.receive_sequence);
    assert_eq!(before.pending_dispatch, after.pending_dispatch);
}

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

// Identical mining consumer calls run against both native adapters. Only fixture
// construction and packet application live in version-specific test modules.
pub(crate) async fn common_mining_start_scenario(
    client: &Client,
    target: [i32; 3],
    face: BlockFace,
) -> survival::MiningRecord {
    let before = client.player_state().await.unwrap();
    let record = client.survival().start_mining(target, face).await.unwrap();
    assert_eq!(record.initial.session, before.session);
    assert_eq!(record.initial.position, before.position);
    assert_eq!(record.initial.received_pose, before.received_pose);
    assert_eq!(record.initial.inventory.cursor, before.inventory.cursor);
    assert_eq!(record.target, target);
    assert_eq!(record.face, face);
    assert!(record.start.dispatched);
    assert!(!record.continuation_validated);
    assert!(record.finish.is_none() && record.abort.is_none());
    assert_eq!(record.stage, survival::MiningStage::Mining);
    let current = client.survival().mining_record().await.unwrap().unwrap();
    assert_eq!(current.id, record.id);
    assert!(client.survival().select_hotbar(0).await.is_err());
    assert!(client.survival().look([0.0; 2]).await.is_err());
    assert!(client.survival().start_mining(target, face).await.is_err());
    assert!(
        client
            .survival()
            .preview_path(&[survival::SurvivalControl {
                yaw: 0.0,
                input: Default::default()
            }])
            .await
            .is_err()
    );
    record
}
pub(crate) async fn common_mining_finish_scenario(client: &Client, id: survival::MiningId) {
    let finish = client.survival().finish_mining(id).await.unwrap();
    assert!(finish.finish.unwrap().dispatched);
    assert_eq!(finish.stage, survival::MiningStage::PendingAfterFinish);
    assert!(client.survival().finish_mining(id).await.is_err());
    let abort = client.survival().abort_mining(id).await.unwrap();
    assert!(abort.abort.unwrap().dispatched);
    assert_eq!(abort.stage, survival::MiningStage::PendingAfterFinish);
    assert!(client.survival().abort_mining(id).await.is_err());
    assert!(client.survival().finish_mining(id).await.is_err());
}
pub(crate) async fn common_mining_removal_scenario(client: &Client, id: survival::MiningId) {
    let result = client.survival().mining_record().await.unwrap().unwrap();
    assert_eq!(result.id, id);
    assert_eq!(result.stage, survival::MiningStage::ObservedRemoved);
    assert!(!result.continuation_validated);
    let target = result.target_receipt.unwrap();
    assert!(target.receive_sequence > result.start.after_sequence);
    assert_eq!(target.state.name, "minecraft:air");
    assert!(client.survival().look([0.0; 2]).await.is_err());
    assert!(client.survival().finish_mining(id).await.is_err());
    assert!(client.survival().abort_mining(id).await.is_err());
    assert!(
        client
            .survival()
            .start_mining(result.target, result.face)
            .await
            .is_err()
    );
}

// The consumer never chooses a version-specific placement entry point.
pub(crate) async fn common_placement_start_scenario(
    client: &Client,
    support: [i32; 3],
    face: BlockFace,
    target: [i32; 3],
) -> survival::PlacementRecord {
    let before = client.player_state().await.unwrap();
    let record = client.survival().place_cube(support, face).await.unwrap();
    assert_eq!(record.initial.session, before.session);
    assert_eq!(record.initial.position, before.position);
    assert_eq!(record.initial.received_pose, before.received_pose);
    assert_eq!(record.initial.inventory.cursor, before.inventory.cursor);
    assert_eq!(record.support, support);
    assert_eq!(record.target, target);
    assert_eq!(record.face, face);
    assert_eq!(record.stage, survival::PlacementStage::Pending);
    assert!(record.send.dispatched);
    assert!(record.target_receipt.is_none() && record.material_receipt.is_none());
    assert!(client.survival().place_cube(support, face).await.is_err());
    assert!(client.survival().select_hotbar(0).await.is_err());
    assert!(client.survival().target_block(4.5).await.is_err());
    record
}
pub(crate) async fn common_placement_pending_scenario(client: &Client) {
    let record = client.survival().placement_record().await.unwrap().unwrap();
    assert_eq!(record.stage, survival::PlacementStage::Pending);
    assert!(
        client
            .survival()
            .look(record.initial.rotation)
            .await
            .is_err()
    );
}
pub(crate) async fn common_placement_completed_scenario(
    client: &Client,
    id: survival::PlacementId,
) -> survival::PlacementRecord {
    let record = client.survival().placement_record().await.unwrap().unwrap();
    assert_eq!(record.id, id);
    assert_eq!(record.stage, survival::PlacementStage::ObservedPlaced);
    let target = record.target_receipt.as_ref().unwrap();
    assert_eq!(target.value, record.expected);
    assert!(
        matches!(target.source,ValueSource::Received{sequence} if sequence>record.send.after_sequence)
    );
    let material = record.material_receipt.as_ref().unwrap();
    assert_eq!(
        material.value,
        survival::placement::remaining(&record.held_before)
    );
    assert!(
        matches!(material.source,ValueSource::Received{sequence} if sequence>record.send.after_sequence)
    );
    client.survival().select_hotbar(0).await.unwrap();
    record
}
