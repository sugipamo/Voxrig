//! F96 receipt order and bounded history, independent of adapter packet encoding.
use super::changes::InventoryChangeLedger;
use super::executor::Executor;
use super::outcome::Comparison;
use super::*;
use crate::MinecraftVersion;
use crate::client::container::{ScreenReceipts, ScreenTitle};
use crate::client::registry::Registry;
use crate::client::{InventoryObservation, ItemData, ItemStack, received};
use tokio::time::Duration;

fn item(version: MinecraftVersion, name: &str, count: u32) -> SlotKnowledge {
    let item = Registry::for_version(version).item(name).unwrap();
    SlotKnowledge::Item {
        item: ItemStack {
            id: item.id,
            name: item.name,
            count,
            data: ItemData::Default,
        },
    }
}
fn snapshot(version: MinecraftVersion, sequence: u64) -> InventorySnapshot {
    InventorySnapshot::from_inventory(
        SessionStamp {
            version,
            connection_id: 17,
            world_generation: 1,
        },
        sequence,
        &InventoryObservation {
            slots: vec![Some(received(SlotKnowledge::Empty, sequence)); 46],
            cursor: Some(received(SlotKnowledge::Empty, sequence)),
            window_id: Some(0),
            ..Default::default()
        },
        None,
    )
}
fn begin(
    e: &mut Executor,
    before: &InventorySnapshot,
    expected: InventorySnapshot,
    action: i16,
) -> InventorySendId {
    let legacy = before.session.version == MinecraftVersion::Java1_16_1;
    let id = e
        .begin(
            before.session,
            before.source,
            before.receive_sequence,
            legacy.then_some(action),
            Duration::from_secs(1),
            false,
        )
        .unwrap();
    e.comparison(Comparison::exact(id, before.clone(), expected));
    e.written(id, Duration::from_secs(1));
    id
}
fn response(e: &mut Executor, id: InventorySendId, action: i16, sequence: u64) {
    if id.session().version == MinecraftVersion::Java1_16_1 {
        e.response(id.session(), id.source(), action, sequence);
    } else {
        e.fence(id.session(), id.source(), id.number(), sequence);
    }
}
fn finish(e: &mut Executor, id: InventorySendId, action: i16, mut current: InventorySnapshot) {
    // Older comparison-only fixtures used one synthetic ordinal for the entire
    // response. Leave room for the three distinct legacy receive packets.
    let seq = current
        .receive_sequence
        .max(e.record().unwrap().after_sequence + 3);
    current.receive_sequence = seq;
    let window = e.record().unwrap().window();
    if id.session().version == MinecraftVersion::Java1_16_1 {
        response(e, id, action, seq - 2);
        e.contents(current.session, current.source, window, seq - 1, false);
        e.cursor(current.session, current.source, seq);
    } else {
        e.contents(current.session, current.source, window, seq, true);
        response(e, id, action, seq + 1);
    }
    e.receive(current, false);
}

#[tokio::test(start_paused = true)]
async fn completed_response_state_keeps_deadline_until_comparison_finishes() {
    let before = snapshot(MinecraftVersion::Java1_16_1, 10);
    let mut e = Executor::default();
    let id = begin(&mut e, &before, before.clone(), 1);
    e.response(before.session, before.source, 1, 11);
    e.contents(before.session, before.source, 0, 12, true);
    assert_eq!(e.record().unwrap().status, InventorySendStatus::Pending);
    // A missing comparison capture must never turn ready into an endless lock.
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(e.record().unwrap().status, InventorySendStatus::Pending);
    assert!(e.recovery_needed(id));
    assert!(e.admit().is_err());
    e.recovery_failed(id);
    assert_eq!(
        e.record().unwrap().inspection_reason,
        Some(InventorySendInspectionReason::ResyncDeliveryUncertain)
    );
    e.begin(
        before.session,
        before.source,
        13,
        Some(2),
        Duration::from_secs(1),
        true,
    )
    .unwrap();
    e.invalidate_owned(id); // retired worker cannot affect the recovery probe
    assert_eq!(e.record().unwrap().status, InventorySendStatus::Pending);
}

#[test]
fn received_scope_change_invalidates_pending_comparison() {
    let before = snapshot(MinecraftVersion::Java1_16_1, 10);
    for ready in [false, true] {
        for world_change in [false, true] {
            let mut e = Executor::default();
            begin(&mut e, &before, before.clone(), 1);
            if ready {
                e.response(before.session, before.source, 1, 11);
                e.contents(before.session, before.source, 0, 12, true);
            }
            let mut changed = table(MinecraftVersion::Java1_16_1, 13);
            if world_change {
                changed = before.clone();
                changed.session.world_generation += 1;
                changed.receive_sequence = 13;
            }
            e.receive(changed, false);
            assert_eq!(
                e.record().unwrap().status,
                InventorySendStatus::ResyncRequired
            );
            let id = e.record().unwrap().id;
            assert!(e.divergence(id).unwrap().is_none());
            assert!(e.admit().is_err());
            // An old matching capture cannot revive a retired send.
            let mut stale = before.clone();
            stale.receive_sequence = 14;
            e.receive(stale, false);
            assert_eq!(
                e.record().unwrap().status,
                InventorySendStatus::ResyncRequired
            );
        }
    }
}

#[test]
fn f96_seed2_action23_preserves_received_table_and_conflicting_log_cursor() {
    // F96_CLICK_CONFLICT.md: original 9201, invalidation 9214, ACK 9215,
    // table source 9216, oak_log cursor 9217. No collect packet is in that log.
    let version = MinecraftVersion::Java1_16_1;
    let mut before = snapshot(version, 9201);
    before.player_slots[37] = Some(received(SlotKnowledge::Empty, 9200));
    before.cursor = Some(received(item(version, "minecraft:crafting_table", 1), 9201));
    let mut expected = before.clone();
    expected.set(
        InventoryLocation::Player { slot: 37 },
        item(version, "minecraft:crafting_table", 1),
    );
    expected.set(InventoryLocation::Cursor, SlotKnowledge::Empty);
    let mut e = Executor::default();
    let id = begin(&mut e, &before, expected, 23);
    let mut current = before.clone();
    current.receive_sequence = 9214;
    e.receive(current.clone(), false);
    e.hint_for_test(InventoryCause::Rejected);
    response(&mut e, id, 23, 9215);
    current.player_slots[37] = Some(received(item(version, "minecraft:crafting_table", 1), 9216));
    current.receive_sequence = 9216;
    e.receive(current.clone(), false);
    assert_eq!(e.record().unwrap().status, InventorySendStatus::Pending);
    current.cursor = Some(received(item(version, "minecraft:oak_log", 1), 9217));
    current.receive_sequence = 9217;
    finish(&mut e, id, 23, current);
    assert_eq!(e.record().unwrap().status, InventorySendStatus::Diverged);
    let d = e.divergence(id).unwrap().unwrap();
    assert_eq!(d.before.receive_sequence, 9201);
    assert_eq!(d.current.receive_sequence, 9217);
    assert!(d.causes.contains(&InventoryCause::Rejected));
    assert!(d.causes.contains(&InventoryCause::External));
    assert!(
        !d.causes
            .iter()
            .any(|c| matches!(c, InventoryCause::Collected { .. }))
    );
    assert_eq!(d.actual.items.len(), 1);
    assert_eq!(d.actual.items[0].item.name, "minecraft:oak_log");
    assert_eq!(d.actual.items[0].change, 1);
}

fn table(version: MinecraftVersion, sequence: u64) -> InventorySnapshot {
    let mut before = snapshot(version, sequence);
    let menu = crate::client::crafting::native_menu(version, "minecraft:crafting").unwrap();
    let mut receipts = ScreenReceipts::open(
        version,
        4,
        menu.native_id,
        ScreenTitle::LegacyJson { json: "{}".into() },
        49850,
    );
    receipts.slots = vec![Some(received(SlotKnowledge::Empty, 49852)); 46];
    before.screen = Some(receipts.project(before.session, &before.player_slots));
    before.source = InventorySource::Container {
        screen: receipts.id(before.session),
    };
    before
}

#[test]
fn f96_window4_three_to_four_and_explicit_collect_evidence_in_both_versions() {
    // Historical source/cursor values are retained exactly. The historical
    // diagnostic contains no collect entity ID: test both no evidence and an
    // additional synthetic collect packet, never invent historical attribution.
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        for collect in [false, true] {
            let mut before = table(version, 49986);
            let location = before.screen_location(38);
            let InventoryLocation::Player { slot } = location else {
                panic!("canonical player alias")
            };
            before.player_slots[slot] =
                Some(received(item(version, "minecraft:cobblestone", 3), 49858));
            before.cursor = Some(received(SlotKnowledge::Empty, 49852));
            let mut expected = before.clone();
            expected.set(location, SlotKnowledge::Empty);
            expected.set(
                InventoryLocation::Cursor,
                item(version, "minecraft:cobblestone", 3),
            );
            let mut e = Executor::default();
            let id = begin(&mut e, &before, expected, 1);
            e.receive(before.clone(), false);
            if collect {
                e.collected(before.session, before.source, 314);
                e.changes.collect(314, 50036);
            }
            if version == MinecraftVersion::Java1_16_1 {
                e.hint_for_test(InventoryCause::Rejected);
            }
            let mut current = before.clone();
            current.player_slots[slot] = Some(received(SlotKnowledge::Empty, 50038));
            current.cursor = Some(received(item(version, "minecraft:cobblestone", 4), 50039));
            current.receive_sequence = 50039;
            finish(&mut e, id, 1, current);
            assert_eq!(e.record().unwrap().status, InventorySendStatus::Diverged);
            let d = e.divergence(id).unwrap().unwrap();
            assert_eq!(d.actual.items.len(), 1);
            assert_eq!(d.actual.items[0].change, 1);
            assert_eq!(d.actual.items[0].item.name, "minecraft:cobblestone");
            assert_eq!(d.actual.slots.len(), 2);
            assert_eq!(
                d.causes.contains(&InventoryCause::Collected {
                    entity_ids: vec![314]
                }),
                collect
            );
            let log = e.changes.after(49986, 50039).unwrap();
            assert_eq!(log.changes.len(), 1);
            assert!(log.changes[0].causes.contains(&InventoryCause::Send { id }));
            assert_eq!(log.changes[0].delta.items[0].change, 1);
        }
    }
}

#[test]
fn ordinary_click_and_transfer_match_while_transient_external_changes_latch() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        for external in [false, true] {
            let mut before = snapshot(version, 10);
            before.player_slots[9] = Some(received(item(version, "minecraft:stone", 3), 10));
            let mut expected = before.clone();
            expected.set(InventoryLocation::Player { slot: 9 }, SlotKnowledge::Empty);
            expected.set(
                InventoryLocation::Cursor,
                item(version, "minecraft:stone", 3),
            );
            let mut e = Executor::default();
            let id = begin(&mut e, &before, expected, 2);
            if external {
                let mut temporary = before.clone();
                temporary.player_slots[19] = Some(received(item(version, "minecraft:dirt", 1), 11));
                temporary.receive_sequence = 11;
                e.receive(temporary, false);
            }
            let mut current = before.clone();
            current.player_slots[9] = Some(received(SlotKnowledge::Empty, 12));
            current.cursor = Some(received(item(version, "minecraft:stone", 3), 12));
            current.receive_sequence = 12;
            finish(&mut e, id, 2, current);
            assert_eq!(
                e.record().unwrap().status,
                if external {
                    InventorySendStatus::Diverged
                } else {
                    InventorySendStatus::Received
                }
            );
            assert_eq!(e.divergence(id).unwrap().is_some(), external);
            assert!(!e.busy());
        }
    }
}

#[test]
fn unrelated_hydration_accepts_intent_and_preserves_received_evidence() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        for hydrate_item in [false, true] {
            let mut before = table(version, 10);
            before.player_slots[19] = None;
            before.cursor = None;
            before.screen.as_mut().unwrap().slots[1] = None;
            let mut expected = before.clone();
            expected.set(
                InventoryLocation::Player { slot: 9 },
                item(version, "minecraft:stone", 1),
            );
            let mut e = Executor::default();
            let id = begin(&mut e, &before, expected, 2);
            let mut current = before.clone();
            let hydrated = if hydrate_item {
                item(version, "minecraft:dirt", 1)
            } else {
                SlotKnowledge::Empty
            };
            current.player_slots[19] = Some(received(hydrated.clone(), 11));
            current.cursor = Some(received(hydrated.clone(), 11));
            current.screen.as_mut().unwrap().slots[1] = Some(received(hydrated, 11));
            current.receive_sequence = 11;
            // Hydration can precede the intended effect and must not latch interruption.
            e.receive(current.clone(), false);
            current.player_slots[9] = Some(received(item(version, "minecraft:stone", 1), 12));
            current.receive_sequence = 12;
            finish(&mut e, id, 2, current.clone());
            assert_eq!(e.record().unwrap().status, InventorySendStatus::Received);
            assert!(e.divergence(id).unwrap().is_none());
            let delta = before.delta(&current);
            assert!(!delta.complete); // None has not been rewritten as Empty.
            assert_eq!(delta.slots.len(), 4);
            assert_eq!(
                current.player_slots[19].as_ref().unwrap().source,
                crate::client::ValueSource::Received { sequence: 11 }
            );
            assert!(
                e.changes
                    .after(11, 12)
                    .unwrap()
                    .changes
                    .iter()
                    .all(|change| !change.causes.contains(&InventoryCause::External))
            );
        }
    }
}

#[test]
fn known_external_change_still_diverges_alongside_unrelated_hydration() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let mut before = snapshot(version, 10);
        before.player_slots[1] = None;
        let mut e = Executor::default();
        let id = begin(&mut e, &before, before.clone(), 2);
        let mut current = before.clone();
        current.player_slots[1] = Some(received(SlotKnowledge::Empty, 11));
        current.player_slots[19] = Some(received(item(version, "minecraft:dirt", 1), 11));
        current.receive_sequence = 11;
        e.receive(current.clone(), false);
        // Even if the known external mutation is reverted before the response,
        // its interruption remains latched; hydration does not mask it.
        current.player_slots[19] = before.player_slots[19].clone();
        current.receive_sequence = 12;
        finish(&mut e, id, 2, current);
        assert_eq!(e.record().unwrap().status, InventorySendStatus::Diverged);
        assert_eq!(
            e.divergence(id).unwrap().unwrap().causes,
            [InventoryCause::External]
        );
    }
}

#[test]
fn intended_unknown_location_is_not_exempt_from_agreement() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        for received_value in [
            None,
            Some(SlotKnowledge::Empty),
            Some(item(version, "minecraft:dirt", 1)),
            Some(item(version, "minecraft:stone", 1)),
        ] {
            let mut before = snapshot(version, 10);
            before.player_slots[9] = None;
            let mut expected = before.clone();
            expected.set(
                InventoryLocation::Player { slot: 9 },
                item(version, "minecraft:stone", 1),
            );
            let mut e = Executor::default();
            let id = begin(&mut e, &before, expected, 2);
            let mut current = before;
            current.player_slots[9] = received_value.map(|v| received(v, 11));
            current.receive_sequence = 11;
            // Rejection is an independent hint; a first receipt must not add External.
            e.hint_for_test(InventoryCause::Rejected);
            finish(&mut e, id, 2, current);
            assert_eq!(e.record().unwrap().status, InventorySendStatus::Diverged);
            assert_eq!(
                e.divergence(id).unwrap().unwrap().causes,
                [InventoryCause::Rejected]
            );
        }
    }
}

#[test]
fn external_stock_aliases_preview_unknowns_gaps_and_retired_scopes() {
    let version = MinecraftVersion::Java1_16_1;
    let mut ledger = InventoryChangeLedger::default();
    let before = snapshot(version, 10);
    ledger.receive(before.clone(), None, &[]);
    let mut next = before.clone();
    next.receive_sequence = 11;
    next.player_slots[9] = Some(received(item(version, "minecraft:stone", 3), 11));
    ledger.receive(next.clone(), None, &[]);
    let log = ledger.after(10, 11).unwrap();
    assert_eq!(log.changes[0].causes, [InventoryCause::External]);
    assert_eq!(log.changes[0].delta.items[0].change, 3);
    assert!(log.changes[0].delta.complete);
    assert!(ledger.after(11, 11).unwrap().changes.is_empty());
    assert!(ledger.after(12, 11).is_err());
    for seq in 12..=12 + MAX_RETAINED_INVENTORY_CHANGES as u64 {
        next.receive_sequence = seq;
        next.player_slots[9] = Some(received(
            item(version, "minecraft:stone", (seq % 64 + 1) as u32),
            seq,
        ));
        ledger.receive(next.clone(), None, &[]);
    }
    assert!(ledger.after(10, next.receive_sequence).is_err());
    assert_eq!(
        ledger
            .after(12, next.receive_sequence)
            .unwrap()
            .changes
            .len(),
        MAX_RETAINED_INVENTORY_CHANGES
    );
    let mut opened = table(version, 2000);
    ledger.receive(opened.clone(), None, &[]);
    assert!(ledger.after(next.receive_sequence, 2000).is_err());
    assert!(ledger.after(2000, 2000).unwrap().changes.is_empty());
    opened.receive_sequence = 2001;
    opened.player_slots[37] = Some(received(item(version, "minecraft:stone", 3), 2001));
    let mut projected = opened.screen.clone().unwrap();
    for mapping in &projected.layout.as_ref().unwrap().player_slots {
        projected.slots[mapping.screen_slot] = opened.player_slots[mapping.player_slot].clone();
    }
    opened.screen = Some(projected);
    ledger.receive(opened.clone(), None, &[]);
    assert_eq!(
        ledger.after(2000, 2001).unwrap().changes[0].delta.items[0].change,
        3
    ); // no alias doubling
    let mut world = opened;
    world.session.world_generation += 1;
    world.receive_sequence = 2002;
    ledger.receive(world, None, &[]);
    assert!(ledger.after(2001, 2002).is_err());
    assert!(ledger.after(2002, 2002).unwrap().changes.is_empty());
    let mut unknown = before.clone();
    unknown.player_slots[9] = None;
    assert!(!unknown.delta(&next).complete);
    let mut preview = before.clone();
    preview.player_slots[0] = Some(received(item(version, "minecraft:stone", 3), 11));
    assert!(before.delta(&preview).items.is_empty());
    assert_eq!(before.delta(&preview).slots.len(), 1);
}

#[test]
fn reused_window_is_a_new_scope_and_container_slot_zero_can_be_physical_stock() {
    let version = MinecraftVersion::Java1_16_1;
    let mut before = table(version, 50000);
    let mut chest = ScreenReceipts::open(
        version,
        4,
        Some(2),
        ScreenTitle::LegacyJson { json: "{}".into() },
        50000,
    );
    chest.slots = vec![Some(received(SlotKnowledge::Empty, 50000)); 63];
    before.screen = Some(chest.project(before.session, &before.player_slots));
    before.source = InventorySource::Container {
        screen: chest.id(before.session),
    };
    let mut after = before.clone();
    after.screen.as_mut().unwrap().slots[0] =
        Some(received(item(version, "minecraft:stone", 4), 50001));
    after.receive_sequence = 50001;
    assert_eq!(before.delta(&after).items[0].change, 4);
    let mut ledger = InventoryChangeLedger::default();
    ledger.receive(before.clone(), None, &[]);
    ledger.receive(after.clone(), None, &[]);
    assert_eq!(ledger.after(50000, 50001).unwrap().changes.len(), 1);
    chest.opened_sequence = 50002;
    let mut reopened = before;
    reopened.screen = Some(chest.project(reopened.session, &reopened.player_slots));
    reopened.source = InventorySource::Container {
        screen: chest.id(reopened.session),
    };
    reopened.receive_sequence = 50002;
    ledger.receive(reopened, None, &[]);
    assert!(ledger.after(50001, 50002).is_err());
    assert!(ledger.after(50002, 50002).unwrap().changes.is_empty());
}

#[test]
fn collect_hint_does_not_attach_to_arbitrary_later_give() {
    let version = MinecraftVersion::Java1_16_1;
    let mut ledger = InventoryChangeLedger::default();
    let before = snapshot(version, 10);
    ledger.receive(before.clone(), None, &[]);
    ledger.collect(9, 11);
    let mut after = before;
    after.receive_sequence = 200;
    after.player_slots[9] = Some(received(item(version, "minecraft:stone", 4), 200));
    ledger.receive(after, None, &[]);
    assert_eq!(
        ledger.after(10, 200).unwrap().changes[0].causes,
        [InventoryCause::External]
    );
}

#[test]
fn unchanged_full_boundary_is_no_effect_without_inventing_external() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        for case in 0..4 {
            let mut before = snapshot(version, 10);
            before.player_slots[9] = Some(received(item(version, "minecraft:stone", 3), 10));
            let mut expected = before.clone();
            expected.set(InventoryLocation::Player { slot: 9 }, SlotKnowledge::Empty);
            expected.set(
                InventoryLocation::Cursor,
                item(version, "minecraft:stone", 3),
            );
            let mut e = Executor::default();
            let id = begin(&mut e, &before, expected, 3);
            // Fresh receipts with the pre-send values: a server-processed no-op.
            let mut current = before.clone();
            for value in current.player_slots.iter_mut().flatten() {
                value.source = crate::client::ValueSource::Received { sequence: 12 };
            }
            current.cursor = Some(received(SlotKnowledge::Empty, 12));
            current.receive_sequence = 12;
            match case {
                // A transient external change, restored before the boundary.
                1 => {
                    let mut temporary = before.clone();
                    temporary.player_slots[19] =
                        Some(received(item(version, "minecraft:dirt", 1), 11));
                    temporary.receive_sequence = 11;
                    e.receive(temporary, false);
                }
                // Partial: an unrelated slot really changed.
                2 => {
                    current.player_slots[19] =
                        Some(received(item(version, "minecraft:dirt", 1), 12))
                }
                // The intended source is unknown at the boundary.
                3 => current.player_slots[9] = None,
                _ => {}
            }
            finish(&mut e, id, 3, current);
            assert_eq!(e.record().unwrap().status, InventorySendStatus::Diverged);
            let d = e.divergence(id).unwrap().unwrap();
            match case {
                0 => {
                    assert_eq!(d.effect(), InventoryEffect::NoEffect);
                    assert!(d.causes.is_empty());
                    assert_eq!(e.unchanged_boundary(), Some((id.session(), 10)));
                }
                1 => {
                    // Received hints are kept; the net effect is still none.
                    assert_eq!(d.effect(), InventoryEffect::NoEffect);
                    assert!(d.causes.contains(&InventoryCause::External));
                    assert_eq!(e.unchanged_boundary(), Some((id.session(), 10)));
                }
                _ => {
                    assert_eq!(d.effect(), InventoryEffect::Different, "case={case}");
                    assert!(d.causes.contains(&InventoryCause::External));
                    assert!(!d.causes.contains(&InventoryCause::Rejected));
                    assert_eq!(e.unchanged_boundary(), None);
                }
            }
            assert!(!e.busy());
        }
    }
}
