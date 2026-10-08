use super::*;
use crate::client::{
    GameMode, InventoryObservation, ItemData, ItemStack, SlotKnowledge,
    container::{ScreenReceipts, ScreenTitle},
    received,
    registry::{Registry, received::ReceivedRegistries},
};

fn player(version: MinecraftVersion) -> PlayerObservation {
    PlayerObservation {
        using_item: None,
        session: SessionStamp {
            version,
            connection_id: 7,
            world_generation: 2,
        },
        receive_sequence: 20,
        pending_dispatch: false,
        dimension: None,
        position: None,
        received_pose: None,
        rotation: [0.; 2],
        game_mode: Some(GameMode::Survival),
        may_fly: None,
        health: None,
        selected_hotbar: None,
        inventory: InventoryObservation {
            window_id: Some(0),
            player_screen: Some(PlayerScreenAccess::Received),
            ..Default::default()
        },
    }
}
fn screen(player: &PlayerObservation) -> ScreenObservation {
    ScreenObservation {
        session: player.session,
        receive_sequence: player.receive_sequence,
        active_window: player.inventory.window_id,
        player_screen: player.inventory.player_screen,
        screen: None,
        cursor: player.inventory.cursor.clone(),
    }
}
fn registries(player: &PlayerObservation) -> ServerRegistryObservation {
    let mut received = ReceivedRegistries::default();
    received.reset(10);
    received.finish();
    received.capture(player.session, player.receive_sequence)
}
fn item(version: MinecraftVersion) -> SlotKnowledge {
    let definition = Registry::for_version(version)
        .item("minecraft:oak_planks")
        .unwrap();
    SlotKnowledge::Item {
        item: ItemStack {
            id: definition.id,
            name: definition.name,
            count: 3,
            data: ItemData::Default,
        },
    }
}

#[test]
fn crafting_received_missing_empty_display_and_registry_boundaries_stay_distinct() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let mut p = player(version);
        p.inventory.slots[1] = Some(received(item(version), 12));
        p.inventory.slots[2] = Some(received(SlotKnowledge::Empty, 13));
        p.inventory.slots[0] = Some(received(item(version), 14));
        p.inventory.local_cache = Some(vec![SlotKnowledge::Empty; 46]);
        let snapshot = ReceivedCrafting::capture(&p, &screen(&p), registries(&p))
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.dimensions(), [2, 2]);
        assert_eq!(
            snapshot.input(0, 0).unwrap().unwrap().value(),
            &item(version)
        );
        assert_eq!(
            snapshot.input(1, 0).unwrap().unwrap().value(),
            &SlotKnowledge::Empty
        );
        assert!(snapshot.input(0, 1).unwrap().is_none());
        assert!(snapshot.input(2, 0).is_err());
        assert!(snapshot.input(0, usize::MAX).is_err());
        assert_eq!(
            snapshot.input_source(0, 1).unwrap(),
            (InventorySource::Player, 3)
        );
        let result = snapshot.result().unwrap();
        assert_eq!(result.receive_sequence(), 14);
        assert!(std::ptr::eq(
            snapshot.registry_state(),
            result.registry_state()
        ));
        assert_eq!(
            snapshot.input(0, 0).unwrap().unwrap().receive_sequence(),
            12
        );
        for source in [
            super::super::ValueSource::Predicted,
            super::super::ValueSource::Received { sequence: 9 },
            super::super::ValueSource::Received { sequence: 21 },
        ] {
            p.inventory.slots[1].as_mut().unwrap().source = source;
            assert!(ReceivedCrafting::capture(&p, &screen(&p), registries(&p)).is_err());
        }
        p.inventory.slots[1] = Some(received(item(version), 12));
        let mut mismatch = screen(&p);
        mismatch.receive_sequence += 1;
        assert!(ReceivedCrafting::capture(&p, &mismatch, registries(&p)).is_err());
        let mut foreign = p.clone();
        foreign.session.world_generation += 1;
        assert!(ReceivedCrafting::capture(&foreign, &screen(&p), registries(&p)).is_err());
        // An old immutable receipt retains its original registry/world owner.
        assert_eq!(snapshot.session(), p.session);
        assert_eq!(result.registry_state().session(), p.session);
    }
}

#[test]
fn crafting_table_uses_original_layout_and_separates_reused_openings() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let mut p = player(version);
        p.inventory.window_id = Some(3);
        p.inventory.player_screen = None;
        let native_id = native_menu(version, "minecraft:crafting")
            .unwrap()
            .native_id;
        let mut table = ScreenReceipts::open(version, 3, native_id, ScreenTitle::Unavailable, 11);
        assert_eq!(table.layout.as_ref().unwrap().total_slots, 46);
        assert_eq!(table.layout.as_ref().unwrap().player_slots.len(), 36);
        // A partial screen is observable without inventing a full-content packet.
        table
            .slot(9, Some(received(item(version), 12)), Some(2), 12)
            .unwrap();
        let mut s = screen(&p);
        s.screen = Some(table.capture(p.session));
        let snapshot = ReceivedCrafting::capture(&p, &s, registries(&p))
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.dimensions(), [3, 3]);
        assert!(snapshot.result().is_none());
        assert!(snapshot.input(0, 0).unwrap().is_none());
        assert_eq!(
            snapshot.input(2, 2).unwrap().unwrap().value(),
            &item(version)
        );
        let (source, slot) = snapshot.input_source(2, 2).unwrap();
        let InventorySource::Container { screen: old } = source else {
            panic!()
        };
        assert_eq!(slot, 9);
        let reopened = ScreenReceipts::open(version, 3, native_id, ScreenTitle::Unavailable, 15)
            .capture(p.session);
        assert_ne!(old, reopened.id);
        assert_eq!(old.window_id(), reopened.id.window_id());
        s.screen = Some(reopened);
        let fresh = ReceivedCrafting::capture(&p, &s, registries(&p))
            .unwrap()
            .unwrap();
        assert_ne!(fresh.input_source(2, 2).unwrap().0, source);
        s.screen.as_mut().unwrap().layout = None;
        assert!(ReceivedCrafting::capture(&p, &s, registries(&p)).is_err());
        s.screen = None;
        assert!(
            ReceivedCrafting::capture(&p, &s, registries(&p))
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn crafting_original_slot_policy_excludes_result_and_preserves_native_capacity() {
    for (version, maximum, table_id, generic) in [
        (MinecraftVersion::Java1_16_1, 64, 11, "bhw"),
        (MinecraftVersion::Java1_21_11, 99, 12, "dji"),
    ] {
        for (name, width) in [("minecraft:player", 2), ("minecraft:crafting", 3)] {
            let menu = native_menu(version, name).unwrap();
            assert_eq!(menu.total_slots, 46);
            assert_eq!((menu.grid_width, menu.grid_height), (width, width));
            assert_eq!(menu.input_slots.len(), width * width);
            assert_eq!(menu.result_slot, 0);
            assert_eq!(menu.native_id, (width == 3).then_some(table_id));
            let mut positions = vec![false; width * width];
            for input in &menu.input_slots {
                assert!(!positions[input.grid_index]);
                positions[input.grid_index] = true;
                assert_ne!(input.screen_slot, menu.result_slot);
                let policy = regular_slot(version, name, input.screen_slot).unwrap();
                assert!(policy.may_pickup);
                assert_eq!(policy.base_capacity, maximum);
                assert!(policy.rejected_default_items.is_empty());
                assert_eq!(
                    menu.slot_policies
                        .iter()
                        .find(|p| p.slot == input.screen_slot)
                        .unwrap()
                        .native_class,
                    generic
                );
            }
            assert!(regular_slot(version, name, menu.result_slot).is_none());
            assert!(
                crate::client::inventory::slot_policy::pickup(
                    version,
                    name,
                    0,
                    crate::client::inventory::InventoryClickButton::Left,
                    &item(version),
                    &SlotKnowledge::Empty
                )
                .is_err()
            );
            assert!(regular_slot(version, name, 46).is_none());
        }
    }
}

#[test]
fn crafting_original_evidence_binds_owned_sources_and_outputs() {
    use sha2::{Digest, Sha256};
    let manifest: serde_json::Value = serde_json::from_str(include_str!(
        "../../../data/client_api/crafting_menu_source.json"
    ))
    .unwrap();
    for (path, bytes) in [
        (
            "scripts/ExportInventoryTransfers.java",
            include_bytes!("../../../scripts/ExportInventoryTransfers.java").as_slice(),
        ),
        (
            "scripts/ExportCraftingMenus.java",
            include_bytes!("../../../scripts/ExportCraftingMenus.java").as_slice(),
        ),
        (
            "scripts/export_crafting_menus.py",
            include_bytes!("../../../scripts/export_crafting_menus.py").as_slice(),
        ),
    ] {
        assert_eq!(
            manifest["generators_sha256"][path],
            hex::encode(Sha256::digest(bytes))
        );
    }
    for (index, path, bytes) in [
        (
            0,
            "data/client_api/crafting_menus-1.16.1.json.gz",
            include_bytes!("../../../data/client_api/crafting_menus-1.16.1.json.gz").as_slice(),
        ),
        (
            1,
            "data/client_api/crafting_menus-1.21.11.json.gz",
            include_bytes!("../../../data/client_api/crafting_menus-1.21.11.json.gz").as_slice(),
        ),
    ] {
        assert_eq!(
            manifest["runs"][index]["files_sha256"][path],
            hex::encode(Sha256::digest(bytes))
        );
    }
}

#[test]
fn crafting_table_close_plans_player_cursor_return_and_retains_received_ingredients() {
    use crate::client::{GameMode, Health, ValueSource};
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        for mode in [GameMode::Survival, GameMode::Creative] {
            let mut p = player(version);
            p.game_mode = Some(mode);
            p.inventory.window_id = Some(3);
            p.inventory.player_screen = None;
            p.health = Some(received(
                Health {
                    health: 20.,
                    food: 20,
                    saturation: 5.,
                },
                12,
            ));
            let mut single = item(version);
            let SlotKnowledge::Item { item: stack } = &mut single else {
                unreachable!()
            };
            stack.count = 1;
            p.inventory.cursor = Some(received(single.clone(), 15));
            let mut table = ScreenReceipts::open(
                version,
                3,
                native_menu(version, "minecraft:crafting")
                    .unwrap()
                    .native_id,
                ScreenTitle::Unavailable,
                11,
            );
            let mut slots = vec![Some(received(SlotKnowledge::Empty, 15)); 46];
            slots[9] = Some(received(single.clone(), 15));
            let player_mapping = table
                .layout
                .as_ref()
                .unwrap()
                .player_slots
                .iter()
                .find(|m| m.player_slot == 9)
                .unwrap()
                .clone();
            slots[player_mapping.screen_slot] = Some(received(single.clone(), 15));
            table.full_items(slots, Some(7), 15).unwrap();
            let current = table.capture(p.session);
            for mapping in &current.layout.as_ref().unwrap().player_slots {
                p.inventory.slots[mapping.player_slot] = current.slots[mapping.screen_slot].clone();
            }
            let record = crate::client::container::prepare_close(
                p.clone(),
                current.clone(),
                current.id,
                mode,
                None,
            )
            .unwrap();
            assert_eq!(record.return_plan.len(), 1);
            let step = &record.return_plan[0];
            assert_eq!((step.screen_slot, step.player_slot), (10, 9));
            assert_eq!(step.source_prediction.source, ValueSource::Predicted);
            assert_eq!(step.cursor_prediction_after.value, SlotKnowledge::Empty);
            assert_eq!(
                record.initial_screen.slots[9],
                Some(received(single.clone(), 15))
            );
            assert_eq!(
                record.initial.inventory.slots[9],
                Some(received(single.clone(), 15))
            );
            assert!(!record.dispatched);
            let fresh = ScreenReceipts::open(
                version,
                3,
                table.native_menu_id,
                ScreenTitle::Unavailable,
                17,
            )
            .capture(p.session);
            assert_ne!(fresh.id, current.id);
            assert!(
                crate::client::container::prepare_close(p, current, fresh.id, mode, None).is_err()
            );
        }
    }
}

fn take_fixture(version: MinecraftVersion) -> (PlayerObservation, ReceivedCrafting) {
    let mut p = player(version);
    p.inventory.screen_revision = Some(7);
    p.inventory.cursor = Some(received(SlotKnowledge::Empty, 15));
    for slot in 0..=4 {
        p.inventory.slots[slot] = Some(received(SlotKnowledge::Empty, 15));
    }
    p.inventory.slots[0] = Some(received(item(version), 15));
    p.inventory.slots[1] = Some(received(item(version), 15));
    let grid = ReceivedCrafting::capture(&p, &screen(&p), registries(&p))
        .unwrap()
        .unwrap();
    (p, grid)
}
#[test]
fn crafting_take_requires_complete_sealed_unchanged_grid_known_cursor_and_mode() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let (p, grid) = take_fixture(version);
        let prepare = |p: PlayerObservation, grid: &ReceivedCrafting| {
            take::prepare(
                p.clone(),
                screen(&p),
                registries(&p),
                grid,
                GameMode::Survival,
                1,
            )
        };
        let record = prepare(p.clone(), &grid).unwrap();
        assert_eq!(
            record.cursor_prediction.source,
            crate::client::ValueSource::Predicted
        );
        assert!(record.after.is_none());
        assert!(record.send.request_full_resync);
        assert_eq!(record.send.screen_revision, Some(7));
        assert_eq!(record.send.sent_screen_revision, Some(0));
        let mut changed = p.clone();
        changed.inventory.slots[1] = Some(received(item(version), 16));
        assert!(prepare(changed, &grid).is_err());
        for slot in 0..=4 {
            let mut missing = p.clone();
            missing.inventory.slots[slot] = None;
            assert!(prepare(missing, &grid).is_err());
        }
        let mut held = p.clone();
        held.inventory.cursor = Some(received(item(version), 15));
        let combined = prepare(held, &grid).unwrap();
        let SlotKnowledge::Item { item } = combined.cursor_prediction.value else {
            panic!("merged cursor absent")
        };
        assert_eq!(item.count, 6);
        let mut unknown = p.clone();
        unknown.inventory.cursor = None;
        assert!(prepare(unknown, &grid).is_err());
        let mut wrong = p.clone();
        wrong.game_mode = Some(GameMode::Creative);
        assert!(prepare(wrong, &grid).is_err());
        let mut busy = p.clone();
        busy.pending_dispatch = true;
        assert!(prepare(busy, &grid).is_err());
    }
}
#[test]
fn crafting_take_regenerated_result_is_not_predicted_empty_and_partial_grid_is_not_completion() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let (p, grid) = take_fixture(version);
        let mut r = take::prepare(
            p.clone(),
            screen(&p),
            registries(&p),
            &grid,
            GameMode::Survival,
            1,
        )
        .unwrap();
        r.send.dispatched = true;
        let mut next = p.clone();
        next.receive_sequence = 25;
        next.inventory.cursor = Some(received(item(version), 21));
        next.inventory.slots[0] = Some(received(item(version), 22));
        next.inventory.slots[1] = Some(received(SlotKnowledge::Empty, 23));
        take::receive(&mut r, &next, &screen(&next), &registries(&next));
        assert!(r.cursor_receipt.is_some());
        assert!(r.after.is_none());
        assert!(!r.ready());
        for slot in 0..=4 {
            next.inventory.slots[slot] = Some(received(SlotKnowledge::Empty, 24));
        }
        next.inventory.slots[0] = Some(received(item(version), 24));
        // A bucket remainder is an actual new item; no uniform decrement was applied.
        let bucket = Registry::for_version(version)
            .item("minecraft:bucket")
            .unwrap();
        let remainder = SlotKnowledge::Item {
            item: ItemStack {
                id: bucket.id,
                name: bucket.name,
                count: 1,
                data: ItemData::Default,
            },
        };
        next.inventory.slots[1] = Some(received(remainder.clone(), 24));
        take::receive(&mut r, &next, &screen(&next), &registries(&next));
        let after = r.after.as_ref().unwrap();
        assert_eq!(after.result().unwrap().value(), &item(version));
        assert_eq!(after.input(0, 0).unwrap().unwrap().value(), &remainder);
        assert_eq!(
            r.before.input(0, 0).unwrap().unwrap().value(),
            &item(version)
        );
        assert_eq!(r.ready(), version == MinecraftVersion::Java1_21_11);
        if version == MinecraftVersion::Java1_16_1 {
            r.legacy_reply = Some(crate::client::inventory::InventoryTransactionReply {
                window_id: 0,
                action: 1,
                accepted: false,
                receive_sequence: 25,
            });
            assert!(r.ready());
        }
    }
}
#[test]
fn crafting_take_conflict_is_latched_even_when_later_output_matches() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let (p, grid) = take_fixture(version);
        let mut r = take::prepare(
            p.clone(),
            screen(&p),
            registries(&p),
            &grid,
            GameMode::Survival,
            1,
        )
        .unwrap();
        r.send.dispatched = true;
        let mut next = p.clone();
        next.receive_sequence = 25;
        next.game_mode = Some(GameMode::Creative);
        take::receive(&mut r, &next, &screen(&next), &registries(&next));
        let reason = r.requires_inspection.clone();
        assert!(reason.is_some());
        next.game_mode = Some(GameMode::Survival);
        next.inventory.cursor = Some(received(item(version), 24));
        take::receive(&mut r, &next, &screen(&next), &registries(&next));
        assert_eq!(r.requires_inspection, reason);
        assert!(!r.ready());
        assert!(r.cursor_receipt.is_none());
    }
}

#[test]
fn crafting_take_table_requires_actual_full_boundary_and_original_opening() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let mut p = player(version);
        p.inventory.window_id = Some(3);
        p.inventory.player_screen = None;
        p.inventory.cursor = Some(received(SlotKnowledge::Empty, 15));
        let mut table = ScreenReceipts::open(
            version,
            3,
            native_menu(version, "minecraft:crafting")
                .unwrap()
                .native_id,
            ScreenTitle::Unavailable,
            11,
        );
        let mut slots = vec![Some(received(SlotKnowledge::Empty, 15)); 46];
        slots[0] = Some(received(item(version), 15));
        table.full_items(slots, Some(7), 15).unwrap();
        let mut s = screen(&p);
        s.screen = Some(table.capture(p.session));
        let grid = ReceivedCrafting::capture(&p, &s, registries(&p))
            .unwrap()
            .unwrap();
        let mut r = take::prepare(
            p.clone(),
            s.clone(),
            registries(&p),
            &grid,
            GameMode::Survival,
            1,
        )
        .unwrap();
        r.send.dispatched = true;
        p.receive_sequence = 25;
        p.inventory.cursor = Some(received(item(version), 21));
        // Fresh individual slots sharing an invented ordinal are not table full authority.
        for slot in 0..=9 {
            table
                .slot(slot, Some(received(SlotKnowledge::Empty, 24)), Some(8), 24)
                .unwrap();
        }
        s = screen(&p);
        s.screen = Some(table.capture(p.session));
        take::receive(&mut r, &p, &s, &registries(&p));
        assert!(r.after.is_none());
        assert!(r.requires_inspection.is_none());
        table
            .full_items(
                vec![Some(received(SlotKnowledge::Empty, 24)); 46],
                Some(8),
                24,
            )
            .unwrap();
        s.screen = Some(table.capture(p.session));
        take::receive(&mut r, &p, &s, &registries(&p));
        assert!(r.after.is_some());
        s.screen = Some(
            ScreenReceipts::open(
                version,
                3,
                table.native_menu_id,
                ScreenTitle::Unavailable,
                25,
            )
            .capture(p.session),
        );
        take::receive(&mut r, &p, &s, &registries(&p));
        assert!(r.requires_inspection.is_some());
        assert!(!r.ready());
    }
}

#[test]
fn crafting_result_merge_requires_entire_capacity_semantic_data_and_fresh_combined_receipt() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let (base, grid) = take_fixture(version);
        let prepare = |p: PlayerObservation| {
            take::prepare(
                p.clone(),
                screen(&p),
                registries(&p),
                &grid,
                GameMode::Survival,
                1,
            )
        };
        let SlotKnowledge::Item { item: output } = item(version) else {
            unreachable!()
        };
        // This fixture's received result is three planks, not a partial transfer.
        let mut held = output.clone();
        held.count = 61;
        let mut p = base.clone();
        p.inventory.cursor = Some(received(SlotKnowledge::Item { item: held.clone() }, 15));
        let mut record = prepare(p.clone()).unwrap();
        assert!(
            matches!(&record.cursor_prediction.value,SlotKnowledge::Item{item} if item.count==64)
        );
        assert_eq!(
            record.cursor_before.value,
            SlotKnowledge::Item { item: held.clone() }
        );
        let mut overflow = p.clone();
        held.count = 62;
        overflow.inventory.cursor = Some(received(SlotKnowledge::Item { item: held }, 15));
        assert!(prepare(overflow).is_err());
        let dirt = Registry::for_version(version)
            .item("minecraft:dirt")
            .unwrap();
        let mut other = p.clone();
        other.inventory.cursor = Some(received(
            SlotKnowledge::Item {
                item: ItemStack {
                    id: dirt.id,
                    name: dirt.name,
                    count: 1,
                    data: ItemData::Default,
                },
            },
            15,
        ));
        assert!(prepare(other).is_err());
        let mut named = p.clone();
        if let Some(SlotKnowledge::Item { item }) =
            named.inventory.cursor.as_mut().map(|v| &mut v.value)
        {
            item.data = if version == MinecraftVersion::Java1_16_1 {
                ItemData::LegacyNbt {
                    bytes: vec![
                        10, 0, 0, 8, 0, 6, b'm', b'a', b'r', b'k', b'e', b'r', 0, 1, b'x', 0,
                    ],
                }
            } else {
                // Actual native custom_data patch form, not a name-only comparison.
                let mut raw = vec![];
                crate::protocol::put_varint(&mut raw, 1);
                crate::protocol::put_varint(&mut raw, 0);
                crate::protocol::put_varint(
                    &mut raw,
                    Registry::for_version(version)
                        .item_component("minecraft:custom_data")
                        .unwrap()
                        .id
                        .value(),
                );
                raw.extend([
                    10, 8, 0, 6, b'm', b'a', b'r', b'k', b'e', b'r', 0, 1, b'x', 0,
                ]);
                ItemData::ModernComponents {
                    patch: crate::versions::java_1_21_11::item_components::decode_patch(&raw)
                        .unwrap(),
                }
            };
        }
        assert!(prepare(named).is_err());
        record.send.dispatched = true;
        // An old or unchanged held cursor is not proof of the result increment.
        p.receive_sequence = 25;
        for slot in 0..=4 {
            p.inventory.slots[slot] = Some(received(SlotKnowledge::Empty, 24));
        }
        take::receive(&mut record, &p, &screen(&p), &registries(&p));
        assert!(record.after.is_some() && record.cursor_receipt.is_none() && !record.ready());
        p.inventory.cursor = Some(received(record.cursor_before.value.clone(), 25));
        take::receive(&mut record, &p, &screen(&p), &registries(&p));
        assert!(record.cursor_receipt.is_none() && record.requires_inspection.is_none());
        p.receive_sequence = 26;
        p.inventory.cursor = Some(received(record.cursor_prediction.value.clone(), 26));
        take::receive(&mut record, &p, &screen(&p), &registries(&p));
        assert!(record.cursor_receipt.is_some());
        if version == MinecraftVersion::Java1_16_1 {
            record.legacy_reply = Some(crate::client::inventory::InventoryTransactionReply {
                window_id: 0,
                action: 1,
                accepted: false,
                receive_sequence: 26,
            });
        }
        assert!(record.ready());
        // A different fresh cursor latches conflict before later matching data.
        let mut conflict = prepare(base.clone()).unwrap();
        conflict.send.dispatched = true;
        let mut wrong = p.clone();
        wrong.inventory.cursor = Some(received(record.cursor_before.value.clone(), 27));
        take::receive(&mut conflict, &wrong, &screen(&wrong), &registries(&wrong));
        assert!(conflict.requires_inspection.is_some());
    }
}

#[test]
fn crafting_result_shift_requires_one_whole_initial_move_and_received_full_inventory_gain() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        for held in [false, true] {
            let (mut p, grid) = take_fixture(version);
            for index in 9..=45 {
                p.inventory.slots[index] = Some(received(SlotKnowledge::Empty, 15));
            }
            if held {
                p.inventory.cursor = Some(received(item(version), 15));
            }
            let prepare = |p: PlayerObservation, grid: &ReceivedCrafting| {
                take::prepare_transfer(
                    p.clone(),
                    screen(&p),
                    registries(&p),
                    grid,
                    GameMode::Survival,
                    1,
                )
            };
            let mut record = prepare(p.clone(), &grid).unwrap();
            assert_eq!(record.destination, CraftingResultDestination::Inventory);
            assert_eq!(record.cursor_prediction.value, record.cursor_before.value);
            assert_eq!(record.inventory_output_before, Some(0));
            let comparison = record.legacy_full_comparison().unwrap();
            assert!(matches!(comparison,SlotKnowledge::Item{item} if item.name=="minecraft:dirt"));
            let mut missing = p.clone();
            missing.inventory.slots[20] = None;
            assert!(prepare(missing, &grid).is_err());
            let mut full = p.clone();
            for index in 9..45 {
                full.inventory.slots[index] = Some(received(
                    {
                        let d = Registry::for_version(version)
                            .item("minecraft:dirt")
                            .unwrap();
                        SlotKnowledge::Item {
                            item: ItemStack {
                                id: d.id,
                                name: d.name,
                                count: 64,
                                data: ItemData::Default,
                            },
                        }
                    },
                    15,
                ));
            }
            assert!(prepare(full, &grid).is_err());
            // Generic moveItemStackTo visits only the first empty slot per native
            // result round; summing all empties would falsely admit oversized output.
            let mut oversized = p.clone();
            if let SlotKnowledge::Item { item } =
                &mut oversized.inventory.slots[0].as_mut().unwrap().value
            {
                item.count = 127;
            }
            let oversized_grid =
                ReceivedCrafting::capture(&oversized, &screen(&oversized), registries(&oversized))
                    .unwrap()
                    .unwrap();
            assert!(prepare(oversized, &oversized_grid).is_err());
            let mut changed = p.clone();
            changed.inventory.slots[44] = Some(received(SlotKnowledge::Empty, 21));
            assert!(
                take::validate_before(&record, &changed, &screen(&changed), &registries(&changed))
                    .is_err()
            );
            record.send.dispatched = true;
            p.receive_sequence = 24;
            for index in 0..=4 {
                p.inventory.slots[index] = Some(received(SlotKnowledge::Empty, 24));
            }
            // Input/result alone, or output gain in an individual update, is not a
            // full main/hotbar boundary. No predicted craft count exists here.
            p.inventory.slots[44] = Some(received(
                {
                    let SlotKnowledge::Item { mut item } = item(version) else {
                        unreachable!()
                    };
                    item.count = 6;
                    SlotKnowledge::Item { item }
                },
                24,
            ));
            take::receive(&mut record, &p, &screen(&p), &registries(&p));
            assert!(record.after.is_none() && record.inventory_after.is_none() && !record.ready());
            for index in 9..44 {
                p.inventory.slots[index] = Some(received(SlotKnowledge::Empty, 24));
            }
            take::receive(&mut record, &p, &screen(&p), &registries(&p));
            assert!(record.after.is_some() && record.inventory_after.is_some() && !record.ready());
            assert_eq!(record.inventory_output_increase, Some(6));
            p.receive_sequence = 25;
            p.inventory.cursor = Some(received(record.cursor_before.value.clone(), 25));
            take::receive(&mut record, &p, &screen(&p), &registries(&p));
            if version == MinecraftVersion::Java1_16_1 {
                record.legacy_reply = Some(crate::client::inventory::InventoryTransactionReply {
                    window_id: 0,
                    action: 1,
                    accepted: false,
                    receive_sequence: 25,
                });
            }
            assert!(record.ready());
            assert_eq!(
                record.completed_stage(),
                CraftingTakeStage::ObservedTransferred
            );
            // Once actual full output was retained, a conflicting destination is
            // latched while waiting for final evidence; restoration cannot erase it.
            let mut conflict = prepare(
                {
                    let (mut q, _) = take_fixture(version);
                    for i in 9..=45 {
                        q.inventory.slots[i] = Some(received(SlotKnowledge::Empty, 15));
                    }
                    q
                },
                &grid,
            )
            .unwrap();
            conflict.send.dispatched = true;
            let mut q = p.clone();
            q.inventory.cursor = Some(received(SlotKnowledge::Empty, 15));
            take::receive(&mut conflict, &q, &screen(&q), &registries(&q));
            q.receive_sequence = 26;
            q.inventory.slots[44] = Some(received(SlotKnowledge::Empty, 26));
            take::receive(&mut conflict, &q, &screen(&q), &registries(&q));
            assert!(conflict.requires_inspection.is_some());
        }
    }
}
