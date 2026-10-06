use super::*;
use crate::MinecraftVersion;

/// One version-independent consumer of exact received item data.
pub(crate) async fn common_item_data_scenario(
    client: &Client,
    slot: usize,
    name: &str,
    count: u32,
    data: ItemData,
) {
    let first = client.player_state().await.unwrap();
    let received = first.inventory.slots[slot].as_ref().unwrap();
    assert!(matches!(received.source, ValueSource::Received { .. }));
    let SlotKnowledge::Item { item } = &received.value else {
        panic!("received item")
    };
    assert_eq!((item.name.as_str(), item.count), (name, count));
    assert_eq!(item.data, data);
    assert_eq!(item.id.version(), first.session.version);
    assert_eq!(item.id.kind(), registry::RegistryKind::Item);
    let second = client.player_state().await.unwrap();
    assert_eq!(second.session, first.session);
    assert_eq!(second.inventory.slots[slot], first.inventory.slots[slot]);
}

pub(crate) async fn common_open_start_scenario(
    client: &Client,
    mode: GameMode,
    target: [i32; 3],
) -> container::ContainerOpenRecord {
    use container::ContainerOpenStage;
    let before = client.player_state().await.unwrap();
    let record = match mode {
        GameMode::Survival => {
            assert!(client.creative().open_container(target).await.is_err());
            client.survival().open_container(target).await.unwrap()
        }
        GameMode::Creative => {
            assert!(client.survival().open_container(target).await.is_err());
            client.creative().open_container(target).await.unwrap()
        }
        _ => panic!("fixture mode"),
    };
    assert_eq!(record.id.session(), before.session);
    assert_eq!(record.initial.received_pose, before.received_pose);
    assert_eq!(record.initial.position, before.position);
    assert_eq!(record.target.position, target);
    assert_eq!(record.target.face, BlockFace::North);
    assert_eq!(record.expected_menu, "minecraft:generic_9x3");
    assert!(record.send.dispatched);
    assert_eq!(record.stage, ContainerOpenStage::Dispatched);
    assert!(record.observed_screen.is_none());
    assert!(record.protocol_processing.is_none());
    assert!(record.received_cursor.is_none());
    assert!(client.player_state().await.unwrap().pending_dispatch);
    assert!(client.survival().open_container(target).await.is_err());
    assert!(client.creative().open_container(target).await.is_err());
    assert!(client.survival().look([0.0; 2]).await.is_err());
    assert!(client.creative().look([0.0; 2]).await.is_err());
    assert_eq!(
        client
            .creative()
            .container_open_record()
            .await
            .unwrap()
            .unwrap()
            .id,
        record.id
    );
    record
}
pub(crate) async fn common_open_completed_scenario(
    client: &Client,
    id: container::ContainerOpenId,
) -> container::ContainerOpenRecord {
    use container::ContainerOpenStage;
    let r = client
        .survival()
        .container_open_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(r.id, id);
    assert_eq!(r.stage, ContainerOpenStage::ObservedContents);
    assert!(r.send.dispatched);
    assert!(r.requires_inspection.is_none());
    let screen = r.observed_screen.as_ref().unwrap();
    assert_eq!(screen.id.session(), id.session());
    assert!(screen.id.opened_sequence() > r.send.after_sequence);
    assert!(screen.full_contents_sequence.unwrap() > screen.id.opened_sequence());
    assert!(
        matches!(r.received_cursor.as_ref().unwrap().source,ValueSource::Received {sequence} if sequence>screen.id.opened_sequence())
    );
    match id.session().version {
        MinecraftVersion::Java1_16_1 => assert!(r.protocol_processing.is_none()),
        MinecraftVersion::Java1_21_11 => {
            let ack = r.protocol_processing.as_ref().unwrap();
            assert!(ack.acknowledged_sequence >= r.send.interaction_sequence.unwrap());
            assert!(ack.receive_sequence > r.send.after_sequence);
        }
    }
    assert!(!client.player_state().await.unwrap().pending_dispatch);
    assert_eq!(
        client.screen_state().await.unwrap().screen.unwrap().id,
        screen.id
    );
    r
}

pub(crate) async fn common_closed_player_screen_scenario(
    client: &Client,
    close: container::ContainerCloseId,
) {
    use container::PlayerScreenAccess;
    let player = client.player_state().await.unwrap();
    let screen = client.screen_state().await.unwrap();
    assert_eq!(
        player.inventory.player_screen,
        Some(PlayerScreenAccess::SubmittedClose { close })
    );
    assert_eq!(screen.player_screen, player.inventory.player_screen);
    assert_eq!(screen.active_window, Some(close.screen().window_id()));
    assert_eq!(screen.screen.unwrap().id, close.screen());
    assert!(!player.pending_dispatch);
    assert!(
        client
            .survival()
            .close_container(close.screen())
            .await
            .is_err()
    );
}

pub(crate) async fn common_container_close_scenario(
    client: &Client,
    mode: GameMode,
    screen: container::ScreenId,
) -> container::ContainerCloseRecord {
    let wrong = if mode == GameMode::Survival {
        client.creative().close_container(screen).await
    } else {
        client.survival().close_container(screen).await
    };
    assert!(wrong.is_err());
    let record = if mode == GameMode::Survival {
        client.survival().close_container(screen).await.unwrap()
    } else {
        client.creative().close_container(screen).await.unwrap()
    };
    assert_eq!(record.mode, mode);
    assert_eq!(record.id.screen(), screen);
    assert_eq!(record.initial_screen.id, screen);
    assert!(record.dispatched);
    assert_eq!(record.stage, container::ContainerCloseStage::Dispatched);
    assert!(record.server_close_sequence.is_none());
    assert!(record.requires_inspection.is_none());
    assert!(client.survival().close_container(screen).await.is_err());
    assert!(client.creative().close_container(screen).await.is_err());
    assert!(
        client
            .survival()
            .swap_container_hotbar(screen, 0, 0)
            .await
            .is_err()
    );
    assert!(
        client
            .creative()
            .swap_container_hotbar(screen, 0, 0)
            .await
            .is_err()
    );
    assert_eq!(
        client
            .survival()
            .container_close_record()
            .await
            .unwrap()
            .unwrap()
            .id,
        record.id
    );
    assert_eq!(
        client
            .creative()
            .container_close_record()
            .await
            .unwrap()
            .unwrap()
            .id,
        record.id
    );
    record
}

pub(crate) async fn common_container_capture_scenario(
    client: &Client,
    full: bool,
) -> container::ScreenId {
    let capture = client.screen_state().await.unwrap();
    let screen = capture.screen.unwrap();
    assert_eq!(capture.active_window, Some(3));
    assert_eq!(screen.id.session(), capture.session);
    assert_eq!(screen.id.window_id(), 3);
    assert!(screen.id.opened_sequence() <= capture.receive_sequence);
    assert_eq!(screen.menu_name.as_deref(), Some("minecraft:generic_9x3"));
    let layout = screen.layout.unwrap();
    assert_eq!(layout.total_slots, 63);
    assert_eq!(layout.player_slots.len(), 36);
    assert_eq!(
        (
            layout.player_slots[0].screen_slot,
            layout.player_slots[0].player_slot
        ),
        (27, 9)
    );
    assert_eq!(
        (
            layout.player_slots[27].screen_slot,
            layout.player_slots[27].player_slot
        ),
        (54, 36)
    );
    assert_eq!(screen.slots.len(), 63);
    if full {
        assert!(screen.full_contents_sequence.is_some());
        assert!(
            matches!(&screen.slots[0],Some(v) if matches!(&v.value,SlotKnowledge::Item{item} if item.name=="minecraft:stone" && item.count==3))
        );
        assert!(
            matches!(&screen.slots[27],Some(v) if matches!(&v.value,SlotKnowledge::Item{item} if item.name=="minecraft:dirt" && item.count==2))
        );
        assert_eq!(
            screen.slots[54].as_ref().unwrap().value,
            SlotKnowledge::Empty
        );
        let player = client.player_state().await.unwrap();
        assert_eq!(player.inventory.slots[9], screen.slots[27]);
        assert_eq!(player.inventory.slots[36], screen.slots[54]);
        assert_eq!(capture.cursor.as_ref().unwrap().value, SlotKnowledge::Empty);
    } else {
        assert!(screen.full_contents_sequence.is_none());
        assert!(screen.slots.iter().all(Option::is_none));
        assert!(capture.cursor.is_none());
    }
    screen.id
}

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
        record.source_before,
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
    assert!(client.player_state().await.unwrap().pending_dispatch);
    let after = client.player_state().await.unwrap();
    assert_eq!(
        after.inventory.slots[usize::from(main)],
        Some(record.source_before.clone())
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
pub(crate) async fn common_container_swap_start_scenario(
    client: &Client,
    mode: GameMode,
    screen: container::ScreenId,
) -> inventory::InventorySwapRecord {
    let before = client.screen_state().await.unwrap().screen.unwrap();
    let record = match mode {
        GameMode::Survival => client.survival().swap_container_hotbar(screen, 0, 0).await,
        GameMode::Creative => client.creative().swap_container_hotbar(screen, 0, 0).await,
        _ => panic!("mode"),
    }
    .unwrap();
    assert_eq!(
        record.source,
        inventory::InventorySwapSource::Container { screen }
    );
    assert_eq!(record.source_slot, 0);
    assert_eq!(record.hotbar_screen_slot, 54);
    assert_eq!(record.initial_screen.as_ref().unwrap().id, before.id);
    assert_eq!(record.source_before, before.slots[0].clone().unwrap());
    assert_eq!(record.hotbar_before, before.slots[54].clone().unwrap());
    assert_eq!(record.window_id(), 3);
    assert!(record.send.dispatched);
    assert_eq!(record.stage, inventory::InventorySwapStage::Pending);
    assert!(client.player_state().await.unwrap().pending_dispatch);
    assert!(
        client
            .survival()
            .swap_container_hotbar(screen, 0, 0)
            .await
            .is_err()
    );
    assert!(
        client
            .creative()
            .swap_container_hotbar(screen, 0, 0)
            .await
            .is_err()
    );
    assert!(client.survival().swap_hotbar(9, 0).await.is_err());
    assert!(client.creative().select_hotbar(0).await.is_err());
    record
}
pub(crate) async fn common_refused_shulker_swap_scenario(
    client: &Client,
    mode: GameMode,
    screen: container::ScreenId,
) {
    let before = client.screen_state().await.unwrap();
    let player = client.player_state().await.unwrap();
    assert_eq!(
        before.screen.as_ref().unwrap().menu_name.as_deref(),
        Some("minecraft:shulker_box")
    );
    let error = match mode {
        GameMode::Survival => client.survival().swap_container_hotbar(screen, 0, 0).await,
        GameMode::Creative => client.creative().swap_container_hotbar(screen, 0, 0).await,
        _ => panic!("mode"),
    }
    .unwrap_err();
    assert_eq!(error.kind(), crate::ErrorKind::InvalidInput);
    assert!(
        client
            .survival()
            .inventory_swap_record()
            .await
            .unwrap()
            .is_none()
    );
    let after = client.screen_state().await.unwrap();
    assert_eq!(after.receive_sequence, before.receive_sequence);
    assert_eq!(after.screen.unwrap().slots, before.screen.unwrap().slots);
    let after = client.player_state().await.unwrap();
    assert_eq!(after.inventory.slots, player.inventory.slots);
    assert_eq!(after.inventory.cursor, player.inventory.cursor);
    assert!(!after.pending_dispatch);
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
            .swap_hotbar(record.source_slot as u8, record.hotbar)
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
        record.source_receipt.as_ref().unwrap().value,
        record.hotbar_before.value
    );
    assert_eq!(
        record.hotbar_receipt.as_ref().unwrap().value,
        record.source_before.value
    );
    for receipt in [&record.source_receipt, &record.hotbar_receipt] {
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

pub(crate) fn common_storage_target_states(
    version: MinecraftVersion,
) -> Vec<crate::NativeBlockState> {
    let text = match version {
        MinecraftVersion::Java1_16_1 => {
            include_str!("../../data/client_api/storage_outlines-1.16.1.json")
        }
        MinecraftVersion::Java1_21_11 => {
            include_str!("../../data/client_api/storage_outlines-1.21.11.json")
        }
    };
    let value: serde_json::Value = serde_json::from_str(text).unwrap();
    value["states"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| serde_json::from_value(s["state"].clone()).unwrap())
        .collect()
}
// Identical public consumer for both adapters/modes. The host seeds the complete
// native state at [8,66,11], with dry floor/standing eye and a +Z rotation.
pub(crate) async fn common_storage_target_scenario(
    client: &Client,
    mode: GameMode,
    state: &crate::NativeBlockState,
) {
    let before = client.player_state().await.unwrap();
    let query = match mode {
        GameMode::Survival => {
            assert!(client.creative().target_block(4.5).await.is_err());
            client.survival().target_block(4.5).await.unwrap()
        }
        GameMode::Creative => {
            assert!(client.survival().target_block(4.5).await.is_err());
            client.creative().target_block(4.5).await.unwrap()
        }
        _ => panic!("fixture mode"),
    };
    assert_eq!(query.initial.session, before.session);
    assert_eq!(query.initial.game_mode, Some(mode));
    assert_eq!(query.initial.position, before.position);
    assert_eq!(query.initial.received_pose, before.received_pose);
    let hit = query.hit.unwrap();
    assert_eq!(hit.position, [8, 66, 11]);
    assert_eq!(&hit.state, state);
    assert_eq!(hit.face, BlockFace::North);
    assert!((hit.point[0] - 8.5).abs() < 1e-12);
    assert!((hit.point[1] - query.eye[1]).abs() < 1e-12);
    assert!((11.0..=11.5).contains(&hit.point[2]));
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
        assert!(matches!(
            Capabilities::for_version(version).support(Feature::Containers),
            Support::Restricted(_)
        ));
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

pub(crate) async fn common_crafting_input_start_scenario(
    client: &Client,
    mode: GameMode,
) -> inventory::InventoryClickRecord {
    let crafting = client.received_crafting().await.unwrap().unwrap();
    assert_eq!(crafting.dimensions(), [2, 2]);
    assert_eq!(
        crafting.input(0, 0).unwrap().unwrap().value(),
        &SlotKnowledge::Empty
    );
    let (source, slot) = crafting.input_source(0, 0).unwrap();
    assert_eq!((source, slot), (inventory::InventorySource::Player, 1));
    common_pickup_start_scenario(
        client,
        mode,
        source,
        slot,
        inventory::InventoryClickButton::Right,
    )
    .await
}
pub(crate) async fn common_crafting_input_complete_scenario(
    client: &Client,
    id: inventory::InventoryClickId,
) {
    let record = client
        .survival()
        .inventory_click_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.id, id);
    assert_eq!(
        record.stage,
        inventory::InventoryClickStage::ObservedClicked,
        "{:?}",
        record.requires_inspection
    );
    let crafting = client.received_crafting().await.unwrap().unwrap();
    let input = crafting.input(0, 0).unwrap().unwrap();
    assert!(input.receive_sequence() > record.send.after_sequence);
    assert_eq!(input.value(), &record.source_receipt.unwrap().value);
    let item = input.item().unwrap();
    assert_eq!(
        (item.stack().name.as_str(), item.stack().count),
        ("minecraft:oak_planks", 1)
    );
    let result = crafting.result().unwrap().item().unwrap();
    assert_eq!(
        (result.stack().name.as_str(), result.stack().count),
        ("minecraft:oak_button", 1)
    );
    assert!(std::ptr::eq(
        crafting.registry_state(),
        result.registry_state()
    ));
    let inventory = client.received_inventory().await.unwrap();
    let cursor = inventory.cursor().unwrap();
    assert!(cursor.receive_sequence() > record.send.after_sequence);
    assert_eq!(cursor.value(), &record.cursor_receipt.unwrap().value);
    assert_eq!(cursor.item().unwrap().stack().count, 2);
}

pub(crate) async fn common_pickup_start_scenario(
    client: &Client,
    mode: GameMode,
    source: inventory::InventoryClickSource,
    slot: u16,
    button: inventory::InventoryClickButton,
) -> inventory::InventoryClickRecord {
    use inventory::InventoryClickStage;
    let record = match mode {
        GameMode::Survival => client
            .survival()
            .click_inventory(source, slot, button)
            .await
            .unwrap(),
        GameMode::Creative => client
            .creative()
            .click_inventory(source, slot, button)
            .await
            .unwrap(),
        _ => unreachable!(),
    };
    assert!(record.send.dispatched);
    assert_eq!(record.stage, InventoryClickStage::Pending);
    assert_eq!(record.button, button);
    assert!(matches!(
        record.source_before.source,
        ValueSource::Received { .. }
    ));
    assert!(matches!(
        record.cursor_before.source,
        ValueSource::Received { .. }
    ));
    assert_eq!(record.prediction.source.source, ValueSource::Predicted);
    assert_eq!(record.prediction.cursor.source, ValueSource::Predicted);
    assert!(record.source_receipt.is_none() && record.cursor_receipt.is_none());
    assert!(
        client
            .survival()
            .click_inventory(source, slot, button)
            .await
            .is_err()
    );
    assert!(
        client
            .creative()
            .click_inventory(source, slot, button)
            .await
            .is_err()
    );
    record
}
pub(crate) async fn common_pickup_complete_scenario(
    client: &Client,
    id: inventory::InventoryClickId,
) -> inventory::InventoryClickRecord {
    let record = client
        .survival()
        .inventory_click_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.id, id);
    assert_eq!(
        record.stage,
        inventory::InventoryClickStage::ObservedClicked,
        "{record:#?}"
    );
    assert!(record.requires_inspection.is_none());
    for (receipt, prediction) in [
        (
            record.source_receipt.as_ref().unwrap(),
            &record.prediction.source,
        ),
        (
            record.cursor_receipt.as_ref().unwrap(),
            &record.prediction.cursor,
        ),
    ] {
        assert_eq!(receipt.value, prediction.value);
        assert!(
            matches!(receipt.source, ValueSource::Received { sequence } if sequence > record.send.after_sequence)
        );
    }
    record
}

pub(crate) async fn common_transfer_start_scenario(
    client: &Client,
    mode: GameMode,
    source: inventory::InventorySource,
    slot: u16,
) -> inventory::InventoryTransferRecord {
    let before_cursor = client
        .player_state()
        .await
        .unwrap()
        .inventory
        .cursor
        .unwrap();
    let record = match mode {
        GameMode::Survival => client
            .survival()
            .transfer_inventory(source, slot)
            .await
            .unwrap(),
        GameMode::Creative => client
            .creative()
            .transfer_inventory(source, slot)
            .await
            .unwrap(),
        _ => unreachable!(),
    };
    assert!(record.send.dispatched);
    assert_eq!(record.stage, inventory::InventoryTransferStage::Pending);
    assert_eq!(record.cursor_before, before_cursor);
    assert!(!record.changed_slots.is_empty());
    for change in &record.changed_slots {
        assert!(matches!(change.before.source, ValueSource::Received { .. }));
        assert_eq!(change.prediction.source, ValueSource::Predicted);
        assert!(change.receipt.is_none());
    }
    assert!(
        client
            .survival()
            .transfer_inventory(source, slot)
            .await
            .is_err()
    );
    assert!(
        client
            .creative()
            .transfer_inventory(source, slot)
            .await
            .is_err()
    );
    record
}
pub(crate) async fn common_transfer_complete_scenario(
    client: &Client,
    id: inventory::InventoryTransferId,
) -> inventory::InventoryTransferRecord {
    let record = client
        .survival()
        .inventory_transfer_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.id, id);
    assert_eq!(
        record.stage,
        inventory::InventoryTransferStage::ObservedTransferred,
        "{record:#?}"
    );
    assert!(record.requires_inspection.is_none());
    assert_eq!(
        record.cursor_inspected.as_ref().unwrap().value,
        SlotKnowledge::Empty
    );
    for change in &record.changed_slots {
        let receipt = change.receipt.as_ref().unwrap();
        assert_eq!(receipt.value, change.prediction.value);
        assert!(
            matches!(receipt.source,ValueSource::Received{sequence} if sequence>record.send.after_sequence)
        );
    }
    record
}

// Same consumer drives both transports; adapters only provide original packets.
pub(crate) fn common_cursor_close_start(
    client: &Client,
    mode: GameMode,
    screen: container::ScreenId,
) -> tokio::task::JoinHandle<crate::Result<container::ContainerCloseRecord>> {
    let client = client.clone();
    tokio::spawn(async move {
        match mode {
            GameMode::Survival => client.survival().close_container(screen).await,
            GameMode::Creative => client.creative().close_container(screen).await,
            _ => unreachable!(),
        }
    })
}
pub(crate) async fn common_cursor_close_retained(
    client: &Client,
    steps: usize,
) -> container::ContainerCloseRecord {
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if let Some(record) = client.survival().container_close_record().await.unwrap() {
                assert!(record.requires_inspection.is_none(), "{record:?}");
                if record.return_steps.len() == steps
                    && record
                        .return_steps
                        .last()
                        .is_some_and(|s| s.send.dispatched)
                {
                    assert_eq!(
                        record.stage,
                        container::ContainerCloseStage::ReturningCursor
                    );
                    assert!(!record.dispatched);
                    assert_eq!(record.return_plan.len(), 2);
                    for step in &record.return_steps {
                        assert_eq!(step.id.close(), Some(record.id));
                    }
                    return record;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}

/// Same public consumer for both adapters; semantic equality permits native re-encoding.
pub(crate) async fn common_data_swap_completed_scenario(
    client: &Client,
    id: inventory::InventorySwapId,
    before: &ReceivedInventory,
) -> inventory::InventorySwapRecord {
    let record = client
        .survival()
        .inventory_swap_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.id, id);
    assert_eq!(record.stage, inventory::InventorySwapStage::ObservedSwapped);
    assert!(record.send.dispatched && record.requires_inspection.is_none());
    let after = client.received_inventory().await.unwrap();
    for (destination, predecessor, diagnostic) in [
        (9, 36, &record.source_receipt),
        (36, 9, &record.hotbar_receipt),
    ] {
        let expected = before.slot(predecessor).unwrap().unwrap();
        let actual = after.slot(destination).unwrap().unwrap();
        assert!(actual.receive_sequence() > record.send.after_sequence);
        assert_eq!(actual.value(), &diagnostic.as_ref().unwrap().value);
        match (expected.item(), actual.item()) {
            (Some(a), Some(b)) => assert!(a.native_equivalent(&b).unwrap()),
            (None, None) => assert_eq!(actual.value(), &SlotKnowledge::Empty),
            _ => panic!("swap lost or invented a stack"),
        }
    }
    record
}

/// Data-preserving split/merge consumer, shared by both actual adapter fixtures.
pub(crate) async fn common_data_pickup_complete_scenario(
    client: &Client,
    id: inventory::InventoryClickId,
    original: &ReceivedInventory,
    counts: (u32, u32),
) {
    let record = client
        .survival()
        .inventory_click_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.id, id);
    assert_eq!(
        record.stage,
        inventory::InventoryClickStage::ObservedClicked,
        "{:?}",
        record.requires_inspection
    );
    let after = client.received_inventory().await.unwrap();
    let original = original.slot(9).unwrap().unwrap().item().unwrap();
    for (slot, count, diagnostic) in [
        (
            after.slot(9).unwrap().unwrap(),
            counts.0,
            &record.source_receipt,
        ),
        (after.cursor().unwrap(), counts.1, &record.cursor_receipt),
    ] {
        assert!(slot.receive_sequence() > record.send.after_sequence);
        assert_eq!(slot.value(), &diagnostic.as_ref().unwrap().value);
        if count == 0 {
            assert_eq!(slot.value(), &SlotKnowledge::Empty);
        } else {
            let item = slot.item().unwrap();
            assert_eq!(item.stack().count, count);
            assert!(original.native_data_equivalent(&item).unwrap());
        }
    }
}

pub(crate) async fn common_data_transfer_complete_scenario(
    client: &Client,
    id: inventory::InventoryTransferId,
    original: &ReceivedInventory,
    expected: &[(usize, u32)],
) -> inventory::InventoryTransferRecord {
    common_data_transfer_complete_from_scenario(client, id, original, 9, expected).await
}

pub(crate) async fn common_data_transfer_complete_from_scenario(
    client: &Client,
    id: inventory::InventoryTransferId,
    original: &ReceivedInventory,
    original_slot: usize,
    expected: &[(usize, u32)],
) -> inventory::InventoryTransferRecord {
    let record = client
        .survival()
        .inventory_transfer_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.id, id);
    assert_eq!(
        record.stage,
        inventory::InventoryTransferStage::ObservedTransferred,
        "{record:#?}"
    );
    assert!(record.requires_inspection.is_none());
    let actual = client.received_inventory().await.unwrap();
    let original = original
        .slot(original_slot)
        .unwrap()
        .unwrap()
        .item()
        .unwrap();
    for &(index, count) in expected {
        let slot = actual.slot(index).unwrap().unwrap();
        let change = record
            .changed_slots
            .iter()
            .find(|s| s.player_slot == Some(index))
            .unwrap();
        assert!(slot.receive_sequence() > record.send.after_sequence);
        assert_eq!(change.receipt.as_ref().unwrap().value, *slot.value());
        if count == 0 {
            assert_eq!(*slot.value(), SlotKnowledge::Empty);
        } else {
            let item = slot.item().unwrap();
            assert_eq!(item.stack().count, count);
            assert!(original.native_data_equivalent(&item).unwrap());
        }
    }
    assert_eq!(
        record.cursor_inspected.as_ref().unwrap(),
        &record.cursor_before
    );
    record
}

pub(crate) async fn common_held_cursor_transfer_complete_scenario(
    client: &Client,
    id: inventory::InventoryTransferId,
    before: &ReceivedInventory,
) {
    let record = client
        .survival()
        .inventory_transfer_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.id, id);
    assert_eq!(
        record.stage,
        inventory::InventoryTransferStage::ObservedTransferred
    );
    assert!(record.requires_inspection.is_none());
    let actual = client.received_inventory().await.unwrap();
    let original = before.slot(9).unwrap().unwrap().item().unwrap();
    for index in [9, 36] {
        let slot = actual.slot(index).unwrap().unwrap();
        let change = record
            .changed_slots
            .iter()
            .find(|s| s.player_slot == Some(index))
            .unwrap();
        assert!(slot.receive_sequence() > record.send.after_sequence);
        assert_eq!(change.receipt.as_ref().unwrap().value, *slot.value());
        if index == 9 {
            assert_eq!(*slot.value(), SlotKnowledge::Empty);
        } else {
            assert!(original.native_equivalent(&slot.item().unwrap()).unwrap());
        }
    }
    let held = before.cursor().unwrap().item().unwrap();
    let cursor = actual.cursor().unwrap();
    assert!(held.native_equivalent(&cursor.item().unwrap()).unwrap());
    assert_eq!(
        record.cursor_inspected.as_ref().unwrap().value,
        *cursor.value()
    );
    assert!(record.cursor_before.value != SlotKnowledge::Empty);
}

pub(crate) async fn common_data_cursor_close_complete_scenario(
    client: &Client,
    complete: &container::ContainerCloseRecord,
    before: &ReceivedInventory,
    expected: &[(usize, u32)],
) {
    assert!(complete.dispatched && complete.requires_inspection.is_none());
    let original = before.cursor().unwrap().item().unwrap();
    let actual = client.received_inventory().await.unwrap();
    assert_eq!(*actual.cursor().unwrap().value(), SlotKnowledge::Empty);
    for &(index, count) in expected {
        let slot = actual.slot(index).unwrap().unwrap();
        let item = slot.item().unwrap();
        assert_eq!(item.stack().count, count);
        assert!(original.native_data_equivalent(&item).unwrap());
        let step = complete
            .return_steps
            .iter()
            .find(|s| {
                s.initial_screen
                    .as_ref()
                    .unwrap()
                    .layout
                    .as_ref()
                    .unwrap()
                    .player_slots
                    .iter()
                    .any(|m| m.player_slot == index && m.screen_slot == usize::from(s.source_slot))
            })
            .unwrap();
        assert_eq!(step.stage, inventory::InventoryClickStage::ObservedClicked);
        for receipt in [
            step.source_receipt.as_ref().unwrap(),
            step.cursor_receipt.as_ref().unwrap(),
        ] {
            assert!(
                matches!(receipt.source, ValueSource::Received {sequence} if sequence > step.send.after_sequence)
            );
        }
        assert_eq!(*slot.value(), step.source_receipt.as_ref().unwrap().value);
    }
}

pub(crate) async fn common_creative_landing_scenario(client: &Client) {
    use super::survival::{MotionStatus, SurvivalControl};
    let ops = client.creative();
    let before = client.player_state().await.unwrap();
    let p = before.position.as_ref().unwrap().value;
    ops.set_flying(true).await.unwrap();
    assert!(ops.land().await.is_err());
    ops.move_flying([p[0], p[1] + 1., p[2]], [0.; 2])
        .await
        .unwrap();
    let airborne = client.flight_record().unwrap();
    assert!(ops.land().await.is_err());
    assert_eq!(client.flight_record().unwrap().attempt, airborne.attempt);
    ops.move_flying(p, [0.; 2]).await.unwrap();
    let ground_flight = client.flight_record().unwrap();
    let record = ops.land().await.unwrap();
    assert!(record.dispatched && record.requires_inspection.is_none());
    assert_eq!(record.stage, super::FlightStage::Submitted);
    assert_eq!(record.command, super::FlightCommand::Land);
    let landing = record.landing.unwrap();
    assert_eq!(landing.flight_attempt, ground_flight.attempt);
    assert_eq!(landing.declared_controller_velocity, [0.; 3]);
    assert!(landing.disable_dispatched && landing.neutral_dispatched);
    assert_eq!(landing.motion.dispatched_ticks, 2);
    assert_eq!(landing.motion.attempted_tick, 2);
    assert_eq!(landing.motion.status, MotionStatus::Predicted);
    assert_eq!(landing.motion.preview.initial_frame.velocity, [0.; 3]);
    assert!(!landing.motion.preview.frames[0].on_ground);
    assert!(landing.motion.preview.frames[1].on_ground && landing.motion.preview.frames[1].resting);
    assert!(
        landing
            .motion
            .preview
            .frames
            .iter()
            .all(|f| f.position == p)
    );
    let after = client.player_state().await.unwrap();
    assert_eq!(after.session, before.session);
    assert_eq!(after.received_pose, before.received_pose);
    let ground = ops
        .preview_path(&[SurvivalControl {
            yaw: 0.,
            input: Default::default(),
        }])
        .await
        .unwrap();
    assert_eq!(ground.initial_frame.position, p);
    assert!(ground.frames[0].resting);
    assert!(ops.land().await.is_err());
    let started = ops
        .start_predicted_path(
            &[SurvivalControl {
                yaw: 0.,
                input: Default::default(),
            }; 2],
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let r = ops.motion_record().await.unwrap().unwrap();
            assert_eq!(r.run_id, started.run_id);
            assert!(r.problem.is_none());
            if r.status == MotionStatus::Predicted {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(started.run_id > landing.motion.run_id);
}
