use super::*;
use crate::client::{
    GameMode, SlotKnowledge,
    inventory::{InventorySource, InventoryTransferRecord, InventoryTransferStage as Stage},
};

fn oracle() -> &'static serde_json::Value {
    static FACTS: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
    FACTS.get_or_init(|| {
        serde_json::from_reader(flate2::read::GzDecoder::new(
            &include_bytes!(
                "../../../../../../../data/client_api/furnace_transfer_cases-1.21.11.json.gz"
            )[..],
        ))
        .unwrap()
    })
}
async fn fixture(
    menu: &str,
    mode: GameMode,
    source: usize,
    item: &str,
    blocked: bool,
    partial: bool,
) -> (CommonFixture, InventorySource) {
    let mut f = CommonFixture::new().await;
    crate::client::inventory::armor_tests::install(
        &mut f.api.bot.session.state.lock().await.registries,
        MinecraftVersion::Java1_21_11,
        None,
    );
    if mode == GameMode::Creative {
        let mut p = vec![3];
        p.extend(1f32.to_be_bytes());
        f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &p)
            .await;
    }
    f.receive(
        ids::play_clientbound::DECLARE_RECIPES,
        &hex::decode(oracle()["properties_packet_hex"].as_str().unwrap()).unwrap(),
    )
    .await;
    let native = crate::client::furnace::native_menu(MinecraftVersion::Java1_21_11, menu).unwrap();
    let mut open = vec![3];
    put_varint(&mut open, native.native_id);
    open.extend([10, 0]);
    f.receive(ids::play_clientbound::OPEN_WINDOW, &open).await;
    let mut full = vec![3, 7, 39];
    for index in 0..39 {
        put_slot(
            &mut full,
            &if index == source {
                plain(item, 7)
            } else if source < 3 && index >= 3 && (blocked || partial) {
                if partial && index == if source == 2 { 38 } else { 3 } {
                    plain(item, 62)
                } else {
                    plain("dirt", 64)
                }
            } else {
                InventorySlot::Empty
            },
        );
    }
    put_slot(&mut full, &InventorySlot::Empty);
    f.receive(ids::play_clientbound::WINDOW_ITEMS, &full).await;
    let id = f.client().screen_state().await.unwrap().screen.unwrap().id;
    (f, InventorySource::Container { screen: id })
}
async fn start(
    f: &CommonFixture,
    mode: GameMode,
    source: InventorySource,
    slot: u16,
) -> crate::Result<InventoryTransferRecord> {
    if mode == GameMode::Creative {
        f.client().creative().transfer_inventory(source, slot).await
    } else {
        f.client().survival().transfer_inventory(source, slot).await
    }
}
fn native(value: &SlotKnowledge) -> InventorySlot {
    match value {
        SlotKnowledge::Empty => InventorySlot::Empty,
        SlotKnowledge::Item { item } => InventorySlot::Item {
            item: PlainItem {
                name: item.name.clone(),
                item_id: item.id.value(),
                count: item.count as i32,
            },
        },
        _ => panic!("known fixture item required"),
    }
}
async fn record(f: &CommonFixture) -> InventoryTransferRecord {
    f.client()
        .survival()
        .inventory_transfer_record()
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn furnace_transfer_packets_all_menus_modes_roles_and_player_boundaries() {
    const CASES: &[(usize, u16, &str)] = &[
        (0, 3, "raw_iron"),
        (1, 3, "coal"),
        (2, 38, "iron_ingot"),
        (3, 1, "coal"),
        (29, 30, "dirt"),
        (30, 3, "dirt"),
        (38, 3, "bucket"),
    ];
    for (menu, _) in crate::client::furnace::TRANSFER_MENUS {
        for mode in [GameMode::Survival, GameMode::Creative] {
            for &(slot, destination, item) in CASES {
                let (mut f, source) = fixture(menu, mode, slot, item, false, false).await;
                let sent = start(&f, mode, source, slot as u16).await.unwrap();
                assert_eq!(
                    read_packet(&mut f.peer, None).await.unwrap(),
                    (
                        ids::play_serverbound::WINDOW_CLICK,
                        super::super::transfer::payload(&sent).unwrap()
                    )
                );
                assert_eq!(sent.changed_slots.len(), 2);
                assert!(sent.changed_slots.iter().any(|c| c.slot == destination));
                // Destination alone cannot establish the source decrease.
                let change = sent
                    .changed_slots
                    .iter()
                    .find(|c| c.slot == destination)
                    .unwrap();
                f.container_slot(destination, native(&change.prediction.value))
                    .await;
                assert_eq!(record(&f).await.stage, Stage::Pending);
                f.container_slot(slot as u16, InventorySlot::Empty).await;
                let done = record(&f).await;
                assert_eq!(done.stage, Stage::ObservedTransferred, "{done:?}");
                assert!(done.changed_slots.iter().all(|c| matches!(c.receipt.as_ref().unwrap().source,
                    crate::client::ValueSource::Received { sequence } if sequence > sent.send.after_sequence)));
                f.stop().await;
            }
            // Materials with fuel value choose input only in a menu accepting them.
            for item in [
                "oak_log",
                if menu == "minecraft:smoker" {
                    "beef"
                } else {
                    "raw_iron"
                },
            ] {
                let (mut f, source) = fixture(menu, mode, 3, item, false, false).await;
                let sent = start(&f, mode, source, 3).await.unwrap();
                let destination = if item == "oak_log" && menu != "minecraft:furnace" {
                    1
                } else {
                    0
                };
                assert!(sent.changed_slots.iter().any(|c| c.slot == destination));
                read_packet(&mut f.peer, None).await.unwrap();
                f.stop().await;
            }
        }
    }
}

#[tokio::test]
async fn furnace_transfer_partial_full_and_consumption_conflicts_never_heal() {
    for (menu, _) in crate::client::furnace::TRANSFER_MENUS {
        for mode in [GameMode::Survival, GameMode::Creative] {
            let (mut f, source) = fixture(menu, mode, 2, "iron_ingot", false, true).await;
            let sent = start(&f, mode, source, 2).await.unwrap();
            read_packet(&mut f.peer, None).await.unwrap();
            f.container_slot(38, plain("iron_ingot", 64)).await;
            assert_eq!(record(&f).await.stage, Stage::Pending);
            f.container_slot(2, plain("iron_ingot", 5)).await;
            assert_eq!(record(&f).await.stage, Stage::ObservedTransferred);
            assert_eq!(sent.changed_slots.len(), 2);
            f.stop().await;
            let (mut f, source) = fixture(menu, mode, 2, "iron_ingot", true, false).await;
            assert_eq!(
                start(&f, mode, source, 2).await.unwrap_err().kind(),
                crate::ErrorKind::InvalidInput
            );
            assert!(
                timeout(Duration::from_millis(15), read_packet(&mut f.peer, None))
                    .await
                    .is_err()
            );
            f.stop().await;
            for conflict in [0, 1, 2, 3] {
                let (mut f, source) = fixture(menu, mode, 3, "coal", false, false).await;
                let sent = start(&f, mode, source, 3).await.unwrap();
                read_packet(&mut f.peer, None).await.unwrap();
                // Unplanned input/output update, early fuel consumption, or a
                // source count between predecessor and prediction is inspection.
                f.container_slot(
                    conflict,
                    if conflict == 1 {
                        plain("coal", 6)
                    } else {
                        plain("stone", 1)
                    },
                )
                .await;
                assert_eq!(record(&f).await.stage, Stage::RequiresInspection);
                for change in &sent.changed_slots {
                    f.container_slot(change.slot, native(&change.prediction.value))
                        .await;
                }
                assert_eq!(record(&f).await.stage, Stage::RequiresInspection);
                assert!(
                    timeout(Duration::from_millis(15), read_packet(&mut f.peer, None))
                        .await
                        .is_err()
                );
                f.stop().await;
            }
        }
    }
}

#[tokio::test]
async fn furnace_transfer_missing_properties_tags_and_changed_ownership_are_not_success() {
    for missing in 0..4 {
        let (mut f, source) = fixture(
            "minecraft:furnace",
            GameMode::Survival,
            3,
            "coal",
            false,
            false,
        )
        .await;
        {
            let mut state = f.api.bot.session.state.lock().await;
            match missing {
                0 => state.furnace_inputs = Default::default(),
                1 => {
                    state.furnace_inputs.sets.clear();
                }
                2 => state.registries.reset(0),
                _ => {
                    state.furnace_inputs.sequence = Some(state.sequence + 1);
                }
            }
        }
        assert!(start(&f, GameMode::Survival, source, 3).await.is_err());
        assert!(
            timeout(Duration::from_millis(15), read_packet(&mut f.peer, None))
                .await
                .is_err()
        );
        f.stop().await;
    }
    for changed in 0..3 {
        let (mut f, source) = fixture(
            "minecraft:furnace",
            GameMode::Survival,
            3,
            "coal",
            false,
            false,
        )
        .await;
        let sent = start(&f, GameMode::Survival, source, 3).await.unwrap();
        read_packet(&mut f.peer, None).await.unwrap();
        match changed {
            0 => {
                f.receive(ids::play_clientbound::OPEN_WINDOW, &[3, 14, 10, 0])
                    .await
            }
            1 => {
                f.receive(
                    ids::play_clientbound::DECLARE_RECIPES,
                    &hex::decode(oracle()["properties_packet_hex"].as_str().unwrap()).unwrap(),
                )
                .await
            }
            _ => {
                let packet = {
                    let state = f.api.bot.session.state.lock().await;
                    state
                        .registries
                        .capture(sent.initial.session, state.sequence)
                        .tag_packet()
                        .unwrap()
                        .value
                        .as_ref()
                        .clone()
                };
                // Even identical memberships have a new received owner.
                f.receive(ids::play_clientbound::TAGS, &packet).await;
            }
        }
        assert_eq!(record(&f).await.stage, Stage::RequiresInspection);
        for change in &sent.changed_slots {
            f.container_slot(change.slot, native(&change.prediction.value))
                .await;
        }
        assert_eq!(record(&f).await.stage, Stage::RequiresInspection);
        f.stop().await;
    }
}

#[tokio::test]
async fn furnace_transfer_requires_every_destination_then_retains_completed_history() {
    for (menu, _) in crate::client::furnace::TRANSFER_MENUS {
        for mode in [GameMode::Survival, GameMode::Creative] {
            let (mut f, source) = fixture(menu, mode, 2, "iron_ingot", false, false).await;
            f.container_slot(38, plain("iron_ingot", 63)).await;
            f.container_slot(37, plain("iron_ingot", 62)).await;
            let sent = start(&f, mode, source, 2).await.unwrap();
            read_packet(&mut f.peer, None).await.unwrap();
            assert_eq!(sent.changed_slots.len(), 4);
            // Fresh source and two merges do not prove the remaining destination.
            for slot in [2, 38, 37] {
                let change = sent.changed_slots.iter().find(|c| c.slot == slot).unwrap();
                f.container_slot(slot, native(&change.prediction.value))
                    .await;
                assert_eq!(record(&f).await.stage, Stage::Pending);
            }
            let pending = record(&f).await;
            assert!(
                pending
                    .changed_slots
                    .iter()
                    .find(|c| c.slot == 36)
                    .unwrap()
                    .receipt
                    .is_none()
            );
            f.container_slot(36, plain("iron_ingot", 4)).await;
            let done = record(&f).await;
            assert_eq!(done.stage, Stage::ObservedTransferred);
            assert!(done.changed_slots.iter().all(|c| matches!(c.receipt.as_ref().unwrap().source,
                crate::client::ValueSource::Received { sequence } if sequence > sent.send.after_sequence)));
            let historical = serde_json::to_value(&done).unwrap();
            // Later autonomous input/fuel/output changes and recipe updates
            // cannot rewrite the completed operation's original receipts.
            for (slot, value) in [
                (0, plain("raw_iron", 1)),
                (1, plain("coal", 6)),
                (2, plain("iron_ingot", 1)),
            ] {
                f.container_slot(slot, value).await;
            }
            f.receive(ids::play_clientbound::DECLARE_RECIPES, &[0, 0])
                .await;
            assert_eq!(serde_json::to_value(record(&f).await).unwrap(), historical);
            assert!(
                timeout(Duration::from_millis(15), read_packet(&mut f.peer, None))
                    .await
                    .is_err()
            );
            f.stop().await;
        }
    }
}

#[tokio::test]
async fn furnace_transfer_received_material_set_wins_over_fuel_even_when_input_blocked() {
    for blocked in [false, true] {
        let (mut f, source) = fixture(
            "minecraft:furnace",
            GameMode::Survival,
            3,
            "coal",
            false,
            false,
        )
        .await;
        let mut packet = vec![1];
        crate::protocol::put_string(&mut packet, "minecraft:furnace_input");
        packet.push(1);
        put_varint(
            &mut packet,
            crate::client::registry::Registry::for_version(MinecraftVersion::Java1_21_11)
                .item("minecraft:coal")
                .unwrap()
                .id
                .value(),
        );
        packet.push(0); // Complete empty stonecutter suffix.
        f.receive(ids::play_clientbound::DECLARE_RECIPES, &packet)
            .await;
        if blocked {
            f.container_slot(0, plain("stone", 64)).await;
        }
        let result = start(&f, GameMode::Survival, source, 3).await;
        if blocked {
            assert_eq!(
                result.unwrap_err().to_string(),
                "shift transfer has no effect; no packet submitted"
            );
            assert!(
                timeout(Duration::from_millis(15), read_packet(&mut f.peer, None))
                    .await
                    .is_err()
            );
        } else {
            let sent = result.unwrap();
            assert!(sent.changed_slots.iter().any(|c| c.slot == 0));
            assert!(!sent.changed_slots.iter().any(|c| c.slot == 1));
            read_packet(&mut f.peer, None).await.unwrap();
        }
        f.stop().await;
    }
}

#[tokio::test]
async fn furnace_transfer_consumed_fuel_receipt_cannot_complete_or_heal() {
    for (menu, _) in crate::client::furnace::TRANSFER_MENUS {
        for mode in [GameMode::Survival, GameMode::Creative] {
            let (mut f, source) = fixture(menu, mode, 3, "coal", false, false).await;
            f.container_slot(3, plain("coal", 1)).await;
            f.container_slot(
                0,
                plain(
                    if menu == "minecraft:smoker" {
                        "beef"
                    } else {
                        "raw_iron"
                    },
                    1,
                ),
            )
            .await;
            let sent = start(&f, mode, source, 3).await.unwrap();
            read_packet(&mut f.peer, None).await.unwrap();
            f.container_slot(1, plain("coal", 1)).await;
            assert_eq!(record(&f).await.stage, Stage::Pending);
            // The server burns the last fuel before the source receipt arrives.
            // Empty equals the predecessor, but invalidates the earlier receipt.
            f.container_slot(1, InventorySlot::Empty).await;
            assert_eq!(record(&f).await.stage, Stage::RequiresInspection);
            f.container_slot(3, InventorySlot::Empty).await;
            f.container_slot(1, plain("coal", 1)).await;
            let retained = record(&f).await;
            assert_eq!(retained.stage, Stage::RequiresInspection);
            assert_eq!(retained.id, sent.id);
            assert!(retained.changed_slots.iter().all(|c| c.receipt.is_some()));
            assert!(
                timeout(Duration::from_millis(15), read_packet(&mut f.peer, None))
                    .await
                    .is_err()
            );
            f.stop().await;
        }
    }
}

#[tokio::test]
async fn furnace_transfer_reopened_window_requires_its_own_full_contents() {
    for (menu, _) in crate::client::furnace::TRANSFER_MENUS {
        for mode in [GameMode::Survival, GameMode::Creative] {
            let (mut f, old_source) = fixture(menu, mode, 3, "coal", false, false).await;
            let native =
                crate::client::furnace::native_menu(MinecraftVersion::Java1_21_11, menu).unwrap();
            let mut open = vec![3];
            put_varint(&mut open, native.native_id);
            open.extend([10, 0]);
            f.receive(ids::play_clientbound::OPEN_WINDOW, &open).await;
            // A source/destination pair and cursor do not establish the other
            // machine slots. The old full contents belong to another opening.
            f.container_slot(3, plain("coal", 7)).await;
            f.container_slot(1, InventorySlot::Empty).await;
            f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &[0])
                .await;
            let screen = f.client().screen_state().await.unwrap().screen.unwrap();
            let source = InventorySource::Container { screen: screen.id };
            assert_ne!(source, old_source);
            assert!(screen.full_contents_sequence.is_none());
            assert!(screen.slots[0].is_none());
            assert!(screen.slots[2].is_none());
            for source in [old_source, source] {
                assert!(start(&f, mode, source, 3).await.is_err());
                assert!(
                    f.client()
                        .survival()
                        .inventory_transfer_record()
                        .await
                        .unwrap()
                        .is_none()
                );
            }
            assert!(
                timeout(Duration::from_millis(15), read_packet(&mut f.peer, None))
                    .await
                    .is_err()
            );
            let mut full = vec![3, 7, 39];
            for slot in 0..39 {
                put_slot(
                    &mut full,
                    &if slot == 3 {
                        plain("coal", 7)
                    } else {
                        InventorySlot::Empty
                    },
                );
            }
            put_slot(&mut full, &InventorySlot::Empty);
            f.receive(ids::play_clientbound::WINDOW_ITEMS, &full).await;
            let sent = start(&f, mode, source, 3).await.unwrap();
            assert_eq!(sent.source, source);
            assert_eq!(sent.changed_slots.len(), 2);
            assert_eq!(
                read_packet(&mut f.peer, None).await.unwrap().0,
                ids::play_serverbound::WINDOW_CLICK
            );
            f.stop().await;
        }
    }
}
