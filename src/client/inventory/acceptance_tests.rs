//! The same adversarial receive script runs against both real packet adapters.
use super::*;
use crate::MinecraftVersion;
use crate::client::registry::Registry;
use crate::client::{Client, SlotKnowledge, inventory::InventoryClickButton as Button};
use tokio::time::Duration;

pub(crate) trait Fixture {
    fn client(&self) -> Client;
    async fn inventory(&mut self, slots: &[SlotKnowledge], cursor: &SlotKnowledge);
    async fn slot(&mut self, slot: u16, value: &SlotKnowledge);
    async fn cursor(&mut self, value: &SlotKnowledge);
    async fn response(&mut self, send: &InventorySendRecord, accepted: bool);
    async fn collect(&mut self, entity: i32, own: bool);
    async fn item_entity(&mut self, entity: i32, count: u32);
    async fn open_crafting_table(&mut self);
    async fn close_received(&mut self);
    /// Full contents only on legacy; modern packet includes its cursor.
    async fn contents(&mut self, window: i32, slots: &[SlotKnowledge], cursor: &SlotKnowledge);
    async fn complete(
        &mut self,
        send: &InventorySendRecord,
        slots: &[SlotKnowledge],
        cursor: &SlotKnowledge,
        accepted: bool,
    ) {
        if self.client().version() == MinecraftVersion::Java1_16_1 {
            self.response(send, accepted).await;
            self.inventory(slots, cursor).await;
        } else {
            self.inventory(slots, cursor).await;
            self.response(send, accepted).await;
        }
    }
    /// Consume exactly one original click (and the modern response request).
    async fn click(&mut self);
    /// Consume the additional deadline probe, validating its no-effect wire shape.
    async fn crafting_probe(&mut self);
}
pub(crate) async fn collect_and_external_log(f: &mut impl Fixture) {
    let client = f.client();
    let version = client.version();
    let mut slots = vec![SlotKnowledge::Empty; 46];
    slots[9] = item(version, "minecraft:cobblestone", 3);
    f.inventory(&slots, &SlotKnowledge::Empty).await;
    let boundary = client
        .inventory_changes_after(0)
        .await
        .unwrap()
        .receive_sequence;
    let send = client
        .survival()
        .send_inventory_click(InventorySource::Player, 9, Button::Left)
        .await
        .unwrap();
    f.click().await;
    f.collect(111, false).await; // another player's animation is not our pickup
    f.collect(222, true).await;
    f.collect(222, true).await; // deduplicate entity IDs
    slots[9] = SlotKnowledge::Empty;
    let cursor = item(version, "minecraft:cobblestone", 4);
    f.complete(&send, &slots, &cursor, false).await;
    assert_eq!(
        client
            .survival()
            .inventory_send_record()
            .await
            .unwrap()
            .unwrap()
            .status,
        InventorySendStatus::Diverged
    );
    let divergence = client.inventory_divergence(send.id).await.unwrap().unwrap();
    assert!(divergence.causes.contains(&InventoryCause::Collected {
        entity_ids: vec![222]
    }));
    assert_eq!(divergence.actual.items.len(), 1);
    assert_eq!(divergence.actual.items[0].change, 1);
    assert_eq!(divergence.effect(), InventoryEffect::Different);
    let log = client.inventory_changes_after(boundary).await.unwrap();
    assert!(
        log.changes
            .iter()
            .any(|c| c.causes.contains(&InventoryCause::Send { id: send.id }))
    );
    assert!(
        log.changes
            .iter()
            .any(|c| c.causes.contains(&InventoryCause::Collected {
                entity_ids: vec![222]
            }))
    );
    // /give-equivalent, after the in-flight operation has ended.
    f.slot(19, &item(version, "minecraft:oak_log", 5)).await;
    let external = client
        .inventory_changes_after(log.receive_sequence)
        .await
        .unwrap();
    assert_eq!(external.changes.len(), 1);
    assert_eq!(external.changes[0].causes, [InventoryCause::External]);
    assert_eq!(external.changes[0].delta.items[0].change, 5);
    assert!(external.changes[0].delta.complete);
    assert!(
        client
            .inventory_changes_after(external.receive_sequence)
            .await
            .unwrap()
            .changes
            .is_empty()
    );
}
/// A vanilla no-op: the server processes the send (1.16.1 negative reply, or
/// 1.21.11 fence) and resends full contents/cursor still equal to `before`.
pub(crate) async fn no_effect_click_and_transfer(f: &mut impl Fixture) {
    let client = f.client();
    let version = client.version();
    let src = InventorySource::Player;
    let mut slots = vec![SlotKnowledge::Empty; 46];
    slots[9] = item(version, "minecraft:cobblestone", 3);
    for transfer in [false, true] {
        f.inventory(&slots, &SlotKnowledge::Empty).await;
        let boundary = client
            .inventory_changes_after(0)
            .await
            .unwrap()
            .receive_sequence;
        let send = if transfer {
            client.survival().send_inventory_transfer(src, 9).await
        } else {
            client
                .survival()
                .send_inventory_click(src, 9, Button::Left)
                .await
        }
        .unwrap();
        f.click().await;
        // Strict records do not see a fresh receipt equal to `before` as an
        // outcome; until the executor boundary completes they stay Pending.
        f.complete(&send, &slots, &SlotKnowledge::Empty, false)
            .await;
        let record = client
            .survival()
            .inventory_send_record()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.id, send.id);
        assert_eq!(record.status, InventorySendStatus::Diverged);
        let d = client.inventory_divergence(send.id).await.unwrap().unwrap();
        assert_eq!(
            d.effect(),
            InventoryEffect::NoEffect,
            "{version:?} {transfer}"
        );
        assert!(d.actual.slots.is_empty() && d.actual.items.is_empty());
        // Comparison false supplies protocol progress, never a rejection cause.
        assert!(d.causes.is_empty());
        if transfer {
            let strict = client
                .survival()
                .inventory_transfer_record()
                .await
                .unwrap()
                .unwrap();
            assert_eq!(strict.stage, InventoryTransferStage::ObservedUnchanged);
            assert!(strict.requires_inspection.is_none());
        } else {
            let strict = client
                .survival()
                .inventory_click_record()
                .await
                .unwrap()
                .unwrap();
            assert_eq!(strict.stage, InventoryClickStage::ObservedUnchanged);
            assert!(strict.requires_inspection.is_none());
            assert!(strict.source_receipt.is_none() && strict.cursor_receipt.is_none());
        }
        assert!(
            client
                .inventory_changes_after(boundary)
                .await
                .unwrap()
                .changes
                .is_empty()
        );
    }
    // The terminal stage releases the strict domain for the next ordinary click.
    let next = client
        .survival()
        .send_inventory_click(src, 9, Button::Left)
        .await
        .unwrap();
    f.click().await;
    slots[9] = SlotKnowledge::Empty;
    f.complete(
        &next,
        &slots,
        &item(version, "minecraft:cobblestone", 3),
        false,
    )
    .await;
    assert_eq!(
        client
            .survival()
            .inventory_send_record()
            .await
            .unwrap()
            .unwrap()
            .status,
        InventorySendStatus::Received
    );
    assert_eq!(
        client
            .survival()
            .inventory_click_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        InventoryClickStage::ObservedClicked
    );
}
pub(crate) async fn crafting_result_outcomes(f: &mut impl Fixture) {
    use crate::client::crafting::CraftingResultDestination;
    let client = f.client();
    for destination in [
        CraftingResultDestination::Cursor,
        CraftingResultDestination::Inventory,
    ] {
        for interruption in 0..3 {
            let collect = interruption == 1;
            let external = interruption == 2;
            let mut slots = vec![SlotKnowledge::Empty; 46];
            slots[0] = item(client.version(), "minecraft:oak_planks", 4);
            slots[1] = item(client.version(), "minecraft:oak_log", 1);
            f.inventory(&slots, &SlotKnowledge::Empty).await;
            let grid = client.received_crafting().await.unwrap().unwrap();
            let sent = client
                .survival()
                .send_crafting_result(&grid, destination)
                .await
                .unwrap();
            f.click().await;
            // No further server boundary arrives: submitting a take is not
            // completion, and repeated strict Look cannot clear its history.
            for _ in 0..64 {
                assert!(client.survival().look([0.0, 0.0]).await.is_err());
                let retained = client
                    .survival()
                    .crafting_take_record()
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(retained.stage, crate::client::CraftingTakeStage::Pending);
            }
            if collect {
                f.collect(333, true).await;
            }
            slots[0] = SlotKnowledge::Empty;
            slots[1] = SlotKnowledge::Empty;
            let cursor = if destination == CraftingResultDestination::Cursor {
                item(client.version(), "minecraft:oak_planks", 4)
            } else {
                slots[44] = item(client.version(), "minecraft:oak_planks", 4);
                SlotKnowledge::Empty
            };
            if external {
                // An unrelated /give can land in the native shift destination
                // range. A fresh full grid and output increase cannot certify it.
                slots[19] = item(client.version(), "minecraft:dirt", 2);
            }
            f.complete(&sent, &slots, &cursor, false).await;
            assert_eq!(
                client
                    .survival()
                    .inventory_send_record()
                    .await
                    .unwrap()
                    .unwrap()
                    .status,
                InventorySendStatus::Diverged,
                "{:?} {destination:?} interruption={interruption}",
                client.version()
            );
            let divergence = client.inventory_divergence(sent.id).await.unwrap();
            assert!(divergence.is_some());
            if let Some(d) = divergence {
                assert!(matches!(
                    d.intended,
                    InventoryIntendedChange::CraftingResult { .. }
                ));
                assert_eq!(
                    d.causes.contains(&InventoryCause::Collected {
                        entity_ids: vec![333]
                    }),
                    collect
                );
                assert_eq!(
                    d.actual
                        .items
                        .iter()
                        .any(|v| v.item.name == "minecraft:dirt" && v.change == 2),
                    external
                );
                assert!(
                    d.actual
                        .items
                        .iter()
                        .any(|v| v.item.name == "minecraft:oak_log" && v.change == -1)
                );
                assert!(
                    d.actual
                        .items
                        .iter()
                        .any(|v| v.item.name == "minecraft:oak_planks" && v.change == 4)
                );
            }
        }
    }
}
pub(crate) fn item(version: MinecraftVersion, name: &str, count: u32) -> SlotKnowledge {
    let definition = Registry::for_version(version).item(name).unwrap();
    SlotKnowledge::Item {
        item: crate::client::ItemStack {
            id: definition.id,
            name: definition.name,
            count,
            data: crate::client::ItemData::Default,
        },
    }
}
pub(crate) async fn pickup_matrix(f: &mut impl Fixture) {
    let client = f.client();
    let version = client.version();
    let src = InventorySource::Player;
    let stone = item(version, "minecraft:stone", 7);
    let dirt = item(version, "minecraft:dirt", 1);
    for transfer in [false, true] {
        for crafting_input in [false, true] {
            for timing in 0..4 {
                for injection in 0..4 {
                    let response_first = version == MinecraftVersion::Java1_16_1;
                    {
                        let source = if crafting_input { 1 } else { 9 };
                        let mut slots = vec![SlotKnowledge::Empty; 46];
                        slots[source] = stone.clone();
                        slots[20] = dirt.clone();
                        let mut cursor = SlotKnowledge::Empty;
                        f.inventory(&slots, &cursor).await;
                        let captured = client.received_inventory().await.unwrap();
                        if timing == 0 {
                            inject(f, injection, source, version, &mut slots, &mut cursor).await;
                        }
                        let send = if transfer {
                            client
                                .survival()
                                .send_inventory_transfer(src, source as u16)
                                .await
                                .unwrap()
                        } else {
                            client
                                .survival()
                                .send_inventory_click(src, source as u16, Button::Left)
                                .await
                                .unwrap()
                        };
                        assert!(send.dispatched);
                        f.click().await;
                        assert!(
                            client
                                .survival()
                                .send_inventory_click(src, 20, Button::Left)
                                .await
                                .is_err()
                        );
                        if timing == 0 {
                            assert!(
                                captured.receive_sequence()
                                    < client.player_state().await.unwrap().receive_sequence
                            );
                        }
                        if timing == 1 {
                            inject(f, injection, source, version, &mut slots, &mut cursor).await;
                        }
                        // Independent plain-stack server model; never copy predicted result values.
                        if transfer {
                            quick_move(&mut slots, source);
                        } else {
                            pickup(&mut slots[source], &mut cursor);
                        }
                        if response_first {
                            f.response(&send, false).await;
                            assert_eq!(
                                client
                                    .survival()
                                    .inventory_send_record()
                                    .await
                                    .unwrap()
                                    .unwrap()
                                    .status,
                                InventorySendStatus::Pending
                            );
                        }
                        if timing == 2 {
                            f.slot(source as u16, &slots[source]).await;
                            inject(f, injection, source, version, &mut slots, &mut cursor).await;
                        }
                        f.inventory(&slots, &cursor).await;
                        if !response_first {
                            assert_eq!(
                                client
                                    .survival()
                                    .inventory_send_record()
                                    .await
                                    .unwrap()
                                    .unwrap()
                                    .status,
                                InventorySendStatus::Pending
                            );
                            f.response(&send, false).await;
                        }
                        assert_eq!(
                            client
                                .survival()
                                .inventory_send_record()
                                .await
                                .unwrap()
                                .unwrap()
                                .status,
                            if matches!(timing, 1 | 2) {
                                InventorySendStatus::Diverged
                            } else {
                                InventorySendStatus::Received
                            },
                            "{version:?} transfer={transfer} grid={crafting_input} timing={timing} injection={injection}"
                        );
                        let divergence = client.inventory_divergence(send.id).await.unwrap();
                        assert_eq!(divergence.is_some(), matches!(timing, 1 | 2));
                        if let Some(d) = divergence {
                            assert_eq!(d.send, send.id);
                            assert!(d.current.receive_sequence > d.before.receive_sequence);
                            assert!(d.causes.contains(&InventoryCause::External));
                            if version == MinecraftVersion::Java1_16_1 {
                                assert!(!d.causes.contains(&InventoryCause::Rejected));
                            }
                        }
                        let old = if transfer {
                            serde_json::to_value(
                                client
                                    .survival()
                                    .inventory_transfer_record()
                                    .await
                                    .unwrap()
                                    .unwrap(),
                            )
                            .unwrap()
                        } else {
                            serde_json::to_value(
                                client
                                    .survival()
                                    .inventory_click_record()
                                    .await
                                    .unwrap()
                                    .unwrap(),
                            )
                            .unwrap()
                        };
                        if timing == 0 || timing == 3 {
                            assert!(old["requires_inspection"].is_null(), "{old}");
                        }
                        if matches!(timing, 1 | 2) && matches!(injection, 0 | 2 | 3) {
                            assert!(!old["requires_inspection"].is_null(), "{old}");
                        }
                        if timing == 3 {
                            inject(f, injection, source, version, &mut slots, &mut cursor).await;
                        }
                        let current = client.player_state().await.unwrap();
                        for (actual, expected) in current.inventory.slots.iter().zip(&slots) {
                            assert_eq!(&actual.as_ref().unwrap().value, expected);
                        }
                        assert_eq!(current.inventory.cursor.as_ref().unwrap().value, cursor);
                        let next = client
                            .survival()
                            .send_inventory_click(src, 20, Button::Left)
                            .await
                            .unwrap();
                        f.click().await;
                        assert_ne!(send.id, next.id);
                        pickup(&mut slots[20], &mut cursor);
                        f.complete(&next, &slots, &cursor, true).await;
                        assert_eq!(
                            client
                                .survival()
                                .inventory_send_record()
                                .await
                                .unwrap()
                                .unwrap()
                                .status,
                            InventorySendStatus::Received
                        );
                        assert_eq!(
                            client
                                .survival()
                                .inventory_click_record()
                                .await
                                .unwrap()
                                .unwrap()
                                .stage,
                            InventoryClickStage::ObservedClicked
                        );
                    }
                }
            }
        }
    }
}
pub(crate) async fn missing_response_resync_and_late_response(f: &mut impl Fixture) {
    let client = f.client();
    let source = InventorySource::Player;
    let mut slots = vec![SlotKnowledge::Empty; 46];
    slots[9] = item(client.version(), "minecraft:stone", 7);
    let mut cursor = SlotKnowledge::Empty;
    f.inventory(&slots, &cursor).await;
    let old = client
        .survival()
        .send_inventory_click(source, 9, Button::Left)
        .await
        .unwrap();
    f.click().await;
    pickup(&mut slots[9], &mut cursor);
    f.inventory(&slots, &cursor).await; // state alone cannot acknowledge this click
    tokio::time::sleep(std::time::Duration::from_millis(10100)).await;
    f.crafting_probe().await; // drain the one owned no-effect recovery before explicit resync
    assert_eq!(
        client
            .survival()
            .inventory_send_record()
            .await
            .unwrap()
            .unwrap()
            .status,
        InventorySendStatus::ResyncRequired
    );
    assert!(
        client
            .survival()
            .send_inventory_click(source, 9, Button::Left)
            .await
            .is_err()
    );
    let sync = client.survival().resync_inventory().await.unwrap();
    f.click().await;
    f.response(&sync, false).await; // ACK alone does not apply state
    assert_eq!(
        client
            .survival()
            .inventory_send_record()
            .await
            .unwrap()
            .unwrap()
            .status,
        InventorySendStatus::Pending
    );
    f.inventory(&slots, &cursor).await;
    if client.version() == MinecraftVersion::Java1_21_11 {
        f.response(&sync, false).await;
    }
    assert_eq!(
        client
            .survival()
            .inventory_send_record()
            .await
            .unwrap()
            .unwrap()
            .status,
        InventorySendStatus::Diverged
    );
    let divergence = client.inventory_divergence(old.id).await.unwrap().unwrap();
    assert!(divergence.causes.contains(&InventoryCause::TimedOutResync));
    assert_eq!(divergence.send, old.id);
    assert_eq!(divergence.before.receive_sequence, old.after_sequence);
    let next = client
        .survival()
        .send_inventory_click(source, 9, Button::Left)
        .await
        .unwrap();
    f.click().await;
    pickup(&mut slots[9], &mut cursor);
    f.complete(&old, &slots, &cursor, true).await; // expired action/token, arriving during the next click
    assert_eq!(
        client
            .survival()
            .inventory_send_record()
            .await
            .unwrap()
            .unwrap()
            .status,
        InventorySendStatus::Pending
    );
    f.response(&next, false).await;
    if client.version() == MinecraftVersion::Java1_16_1 {
        f.inventory(&slots, &cursor).await;
    }
    assert_eq!(
        client
            .survival()
            .inventory_send_record()
            .await
            .unwrap()
            .unwrap()
            .status,
        InventorySendStatus::Received
    );
    assert_eq!(
        client
            .survival()
            .inventory_click_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        InventoryClickStage::ObservedClicked
    );
    let received = client.player_state().await.unwrap();
    assert_eq!(
        received.inventory.slots[9].as_ref().unwrap().value,
        slots[9]
    );
    assert_eq!(received.inventory.cursor.as_ref().unwrap().value, cursor);
}
fn quick_move(slots: &mut [SlotKnowledge], source: usize) {
    let range = if (9..36).contains(&source) {
        36..45
    } else if source >= 36 {
        9..36
    } else {
        9..45
    };
    let SlotKnowledge::Item { mut item } =
        std::mem::replace(&mut slots[source], SlotKnowledge::Empty)
    else {
        return;
    };
    for target in &mut slots[range.clone()] {
        if let SlotKnowledge::Item { item: existing } = target {
            if existing.id == item.id && existing.data == item.data {
                let amount = item.count.min(64 - existing.count);
                existing.count += amount;
                item.count -= amount;
            }
        }
    }
    if item.count > 0 {
        for target in &mut slots[range] {
            if *target == SlotKnowledge::Empty {
                *target = SlotKnowledge::Item { item };
                return;
            }
        }
        slots[source] = SlotKnowledge::Item { item };
    }
}
fn pickup(source: &mut SlotKnowledge, cursor: &mut SlotKnowledge) {
    if let (SlotKnowledge::Item { item: target }, SlotKnowledge::Item { item: held }) =
        (&mut *source, &*cursor)
    {
        if target.id == held.id && target.data == held.data {
            target.count += held.count;
            *cursor = SlotKnowledge::Empty;
            return;
        }
    }
    std::mem::swap(source, cursor);
}
async fn inject(
    f: &mut impl Fixture,
    kind: u8,
    source: usize,
    version: MinecraftVersion,
    slots: &mut [SlotKnowledge],
    cursor: &mut SlotKnowledge,
) {
    let index = if kind == 1 { 19 } else { source };
    let value = if kind <= 1 {
        let count = match &slots[index] {
            SlotKnowledge::Item { item } if item.name == "minecraft:stone" => item.count + 1,
            _ => 1,
        };
        item(version, "minecraft:stone", count)
    } else {
        item(version, "minecraft:dirt", 1)
    };
    if kind == 3 {
        *cursor = value;
        f.cursor(cursor).await;
    } else {
        slots[index] = value;
        f.slot(index as u16, &slots[index]).await;
    }
}
/// I9: an own item use opens a bounded window over the used hand only. The
/// consumption there is `OwnAction`; a /give-equivalent elsewhere in the same
/// window stays `External`, also when both arrive in one full contents.
pub(crate) async fn own_action_and_external_log(f: &mut impl Fixture) {
    use crate::client::Hand;
    let client = f.client();
    let version = client.version();
    let mut slots = vec![SlotKnowledge::Empty; 46];
    slots[45] = item(version, "minecraft:snowball", 4);
    f.inventory(&slots, &SlotKnowledge::Empty).await;
    let boundary = client
        .inventory_changes_after(0)
        .await
        .unwrap()
        .receive_sequence;
    client.survival().use_item(Hand::Off).await.unwrap();
    slots[45] = item(version, "minecraft:snowball", 3);
    f.slot(45, &slots[45]).await;
    slots[19] = item(version, "minecraft:oak_log", 5);
    f.slot(19, &slots[19]).await;
    slots[45] = item(version, "minecraft:snowball", 2);
    slots[20] = item(version, "minecraft:dirt", 1);
    f.inventory(&slots, &SlotKnowledge::Empty).await;
    let log = client.inventory_changes_after(boundary).await.unwrap();
    let off = InventoryLocation::Player { slot: 45 };
    assert_eq!(log.changes.len(), 3, "{log:?}");
    let [own] = log.changes[0].causes.as_slice() else {
        panic!("one own-action hint: {:?}", log.changes[0].causes);
    };
    let InventoryCause::OwnAction {
        action,
        ordinal,
        interaction_sequence,
        locations,
    } = own
    else {
        panic!("own action: {own:?}");
    };
    assert_eq!(*action, InventoryOwnActionKind::UseItem);
    assert!(*ordinal >= 1);
    assert_eq!(
        interaction_sequence.is_some(),
        version == MinecraftVersion::Java1_21_11
    );
    assert_eq!(locations, &[off]);
    assert_eq!(log.changes[0].delta.items[0].change, -1);
    assert_eq!(log.changes[1].causes, [InventoryCause::External]);
    assert_eq!(log.changes[1].delta.items[0].change, 5);
    // One full contents with own consumption and an unrelated slot: the
    // own-action hint names only the hand, External remains for the rest.
    let mixed = &log.changes[2];
    assert_eq!(mixed.delta.slots.len(), 2);
    assert!(mixed.causes.contains(&InventoryCause::External));
    assert!(mixed.causes.iter().any(|c| matches!(
        c,
        InventoryCause::OwnAction { locations, .. } if locations == &[off]
    )));
}

/// The held #305 missing-boundary reproduction now has a bounded record-owned end.
/// Fixtures configure 300 ms per phase; only real packet receipts can resolve it.
pub(crate) async fn crafting_deadline(
    f: &mut impl Fixture,
    destination: crate::client::CraftingResultDestination,
    outcome: u8,
) {
    use crate::client::{CraftingTakeInspectionReason as Reason, CraftingTakeStage as Stage};
    let client = f.client();
    let mut slots = vec![SlotKnowledge::Empty; 46];
    slots[0] = item(client.version(), "minecraft:oak_planks", 4);
    slots[1] = item(client.version(), "minecraft:oak_log", 1);
    f.inventory(&slots, &SlotKnowledge::Empty).await;
    let grid = client.received_crafting().await.unwrap().unwrap();
    let sent = client
        .survival()
        .send_crafting_result(&grid, destination)
        .await
        .unwrap();
    f.click().await;
    let take = client
        .survival()
        .crafting_take_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(take.stage, Stage::Pending);
    assert!(!take.is_resolved() && !take.deadline_resync_dispatched);
    for _ in 0..64 {
        assert!(client.survival().look([0.0, 0.0]).await.is_err());
    }
    f.crafting_probe().await;
    let retained = client
        .survival()
        .inventory_send_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained.id, sent.id); // probe cannot replace the original executor
    let pending = client
        .survival()
        .crafting_take_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.id, take.id);
    assert_eq!(pending.stage, Stage::Pending);
    assert!(pending.deadline_resync_dispatched && !pending.is_resolved());
    if outcome == 0 {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let r = client
                    .survival()
                    .crafting_take_record()
                    .await
                    .unwrap()
                    .unwrap();
                if r.stage != Stage::Pending {
                    assert_eq!(r.stage, Stage::RequiresInspection);
                    assert_eq!(
                        r.inspection_reason,
                        Some(Reason::ReceiptDeadlineAfterResync)
                    );
                    assert!(!r.is_resolved());
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
    if outcome != 2 {
        slots[0] = SlotKnowledge::Empty;
        slots[1] = SlotKnowledge::Empty;
    }
    let cursor = if outcome != 2 && destination == crate::client::CraftingResultDestination::Cursor
    {
        item(client.version(), "minecraft:oak_planks", 4)
    } else {
        if outcome != 2 {
            slots[44] = item(client.version(), "minecraft:oak_planks", 4);
        }
        SlotKnowledge::Empty
    };
    f.complete(&sent, &slots, &cursor, false).await;
    let latest = client
        .survival()
        .crafting_take_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest.id, take.id);
    if outcome == 0 {
        assert_eq!(latest.stage, Stage::RequiresInspection);
        assert_eq!(
            latest.inspection_reason,
            Some(Reason::ReceiptDeadlineAfterResync)
        );
        assert!(!latest.is_resolved());
        assert!(latest.player_after_inspection.is_some());
        assert!(latest.crafting_after_inspection.is_some());
        assert!(client.survival().look([0.0, 0.0]).await.is_err());
        assert!(
            client
                .survival()
                .send_crafting_result(&grid, destination)
                .await
                .is_err()
        );
    } else {
        assert!(latest.is_resolved(), "{latest:?}");
        assert_eq!(
            latest.stage,
            if outcome == 2 {
                Stage::NotApplied
            } else if destination == crate::client::CraftingResultDestination::Cursor {
                Stage::ObservedTaken
            } else {
                Stage::ObservedTransferred
            }
        );
        // An old timer cannot regress a successfully received outcome.
        tokio::time::sleep(std::time::Duration::from_millis(350)).await;
        assert_eq!(
            client
                .survival()
                .crafting_take_record()
                .await
                .unwrap()
                .unwrap()
                .stage,
            latest.stage
        );
    }
}

/// Characterize the ownership fork without changing either strict take or the
/// partial crafting comparison. A collect is evidence of an event, not a recipe
/// delta or proof that an arbitrary storage gain came from that entity.
pub(crate) async fn crafting_pickup_ownership(
    f: &mut impl Fixture,
    pickup_slot: Option<usize>,
    collector: Option<bool>,
) {
    use crate::client::{CraftingTakeStage, OperationalCraftingStage};
    let client = f.client();
    let mut slots = vec![SlotKnowledge::Empty; 46];
    let output = item(client.version(), "minecraft:oak_planks", 4);
    slots[0] = output.clone();
    slots[1] = item(client.version(), "minecraft:oak_log", 1);
    f.inventory(&slots, &SlotKnowledge::Empty).await;
    f.item_entity(1182, 1).await;
    let grid = client.received_crafting().await.unwrap().unwrap();
    let send = client
        .survival()
        .send_operational_crafting_result(&grid)
        .await
        .unwrap();
    f.click().await;
    if let Some(own) = collector {
        f.collect(1182, own).await;
    }
    slots[0] = SlotKnowledge::Empty;
    slots[1] = SlotKnowledge::Empty;
    if let Some(slot) = pickup_slot {
        slots[slot] = item(client.version(), "minecraft:oak_log", 1);
    }
    f.complete(&send, &slots, &output, false).await;
    let take = client
        .survival()
        .crafting_take_record()
        .await
        .unwrap()
        .unwrap();
    let equipped_changed = pickup_slot == Some(36);
    assert_eq!(
        take.stage,
        if equipped_changed {
            CraftingTakeStage::RequiresInspection
        } else {
            CraftingTakeStage::ObservedTaken
        }
    );
    if equipped_changed {
        assert!(
            take.requires_inspection
                .as_deref()
                .unwrap()
                .contains("equipped hand changed")
        );
    }
    // Even a normal craft is deliberately not an exact material prediction.
    assert_eq!(
        client
            .survival()
            .inventory_send_record()
            .await
            .unwrap()
            .unwrap()
            .status,
        InventorySendStatus::Diverged
    );
    let divergence = client.inventory_divergence(send.id).await.unwrap().unwrap();
    assert_eq!(divergence.send, send.id);
    assert_eq!(
        divergence.causes.contains(&InventoryCause::Collected {
            entity_ids: vec![1182]
        }),
        collector == Some(true)
    );
    let InventoryIntendedChange::CraftingResult { cursor, .. } = &divergence.intended else {
        panic!("partial recipe intention")
    };
    assert_eq!(cursor.slots.len(), 1);
    assert_eq!(cursor.slots[0].location, InventoryLocation::Cursor);
    assert_eq!(divergence.current.cursor.as_ref().unwrap().value, output);
    assert_eq!(
        divergence.current.player_slots[1].as_ref().unwrap().value,
        SlotKnowledge::Empty
    );
    for (slot, count) in [(0, -4), (1, -1)] {
        let delta = divergence
            .actual
            .slots
            .iter()
            .find(|d| d.location == InventoryLocation::Player { slot })
            .unwrap();
        assert_eq!(delta.items[0].change, count);
    }
    if let Some(slot) = pickup_slot {
        let delta = divergence
            .actual
            .slots
            .iter()
            .find(|d| d.location == InventoryLocation::Player { slot })
            .unwrap();
        assert_eq!(delta.items[0].change, 1);
    }
    let operational = client.operational_crafting_outcome(send.id).await.unwrap();
    let success = (pickup_slot.is_none() && collector != Some(true))
        || (pickup_slot.is_some() && collector == Some(true));
    assert_eq!(
        operational.stage,
        if success {
            OperationalCraftingStage::Crafted
        } else {
            OperationalCraftingStage::RequiresInspection
        },
        "{operational:?}"
    );
    assert_eq!(
        operational.external.len(),
        usize::from(success && pickup_slot.is_some())
    );
    assert!(
        client
            .survival()
            .send_inventory_click(InventorySource::Player, 10, Button::Left)
            .await
            .is_err()
    );
    if success {
        let continued = client.continue_operational_crafting(send.id).await.unwrap();
        assert!(continued.released);
        assert!(
            continued
                .basis
                .as_ref()
                .unwrap()
                .basis_matches_current(&client.capture_inventory_view().await.unwrap())
        );
        assert_eq!(
            continued
                .basis
                .as_ref()
                .unwrap()
                .cursor
                .as_ref()
                .unwrap()
                .value,
            output
        );
        // This is a new cursor put-down, never another result click.
        let next = client
            .survival()
            .send_inventory_click(InventorySource::Player, 10, Button::Left)
            .await
            .unwrap();
        f.click().await;
        slots[10] = output.clone();
        f.complete(&next, &slots, &SlotKnowledge::Empty, false)
            .await;
    } else {
        assert!(client.continue_operational_crafting(send.id).await.is_err());
    }
    // Historical conflict stays latched after another fresh full boundary.
    f.inventory(&slots, &output).await;
    assert_eq!(
        client
            .survival()
            .crafting_take_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        take.stage
    );
    client.disconnect().await.unwrap();
}

/// Foreign animations are neither pickup proof nor a conflict for this owner.
/// Run through both packet adapters, including missing foreign item metadata.
pub(crate) async fn crafting_foreign_collect(f: &mut impl Fixture, stock_conflict: bool) {
    use crate::client::OperationalCraftingStage as Stage;
    let client = f.client();
    let version = client.version();
    let mut slots = vec![SlotKnowledge::Empty; 46];
    let output = item(version, "minecraft:oak_planks", 4);
    slots[0] = output.clone();
    slots[1] = item(version, "minecraft:oak_log", 1);
    slots[9] = item(version, "minecraft:oak_log", 2);
    f.item_entity(1182, 1).await;
    f.item_entity(1183, 1).await;
    f.inventory(&slots, &SlotKnowledge::Empty).await;
    let grid = client.received_crafting().await.unwrap().unwrap();
    let send = client
        .survival()
        .send_operational_crafting_result(&grid)
        .await
        .unwrap();
    f.click().await;
    let pending = client.operational_crafting_outcome(send.id).await.unwrap();
    assert_eq!(pending.stage, Stage::Pending);
    for entity in [1183, 9999] {
        f.collect(entity, false).await;
        let after = client.operational_crafting_outcome(send.id).await.unwrap();
        assert_eq!(
            serde_json::to_value(&after).unwrap(),
            serde_json::to_value(&pending).unwrap()
        );
    }
    f.collect(1182, true).await;
    assert_eq!(
        client
            .operational_crafting_outcome(send.id)
            .await
            .unwrap()
            .stage,
        Stage::Pending
    );
    let mut first_failure = None;
    if stock_conflict {
        // A transient removal must not disappear behind a matching full proof.
        f.slot(9, &item(version, "minecraft:oak_log", 1)).await;
        let failed = client.operational_crafting_outcome(send.id).await.unwrap();
        assert_eq!(failed.stage, Stage::RequiresInspection);
        first_failure = failed.requires_inspection;
        assert!(
            first_failure
                .as_deref()
                .unwrap()
                .contains("unexplained intermediate stock change")
        );
        f.slot(9, &slots[9]).await;
    }
    slots[0] = SlotKnowledge::Empty;
    slots[1] = SlotKnowledge::Empty;
    slots[10] = item(version, "minecraft:oak_log", 1);
    f.complete(&send, &slots, &output, false).await;
    let outcome = client.operational_crafting_outcome(send.id).await.unwrap();
    if stock_conflict {
        assert_eq!(outcome.stage, Stage::RequiresInspection);
        assert_eq!(outcome.requires_inspection, first_failure);
        assert!(client.continue_operational_crafting(send.id).await.is_err());
    } else {
        assert_eq!(outcome.stage, Stage::Crafted, "{outcome:?}");
        assert!(outcome.requires_inspection.is_none());
        assert_eq!(outcome.external.len(), 1);
        assert_eq!(outcome.external[0].metadata.entity.native_id(), 1182);
        assert_eq!(outcome.external[0].collector, 42);
        assert_eq!(outcome.external[0].count, 1);
        let delta = outcome.external_delta.as_ref().unwrap();
        assert_eq!(delta.items.len(), 1);
        assert_eq!(delta.items[0].change, 1);
        assert_eq!(delta.slots.len(), 1);
        assert_eq!(
            delta.slots[0].location,
            InventoryLocation::Player { slot: 10 }
        );
        // Claimed Crafted remains guarded until explicit continuation, but an
        // unrelated player's pickup cannot invalidate that proven basis.
        for entity in [1183, 9999] {
            f.collect(entity, false).await;
            let after = client.operational_crafting_outcome(send.id).await.unwrap();
            assert_eq!(
                serde_json::to_value(&after).unwrap(),
                serde_json::to_value(&outcome).unwrap()
            );
        }
        assert!(
            client
                .continue_operational_crafting(send.id)
                .await
                .unwrap()
                .released
        );
    }
    let divergence = client.inventory_divergence(send.id).await.unwrap().unwrap();
    let collected: Vec<_> = divergence
        .causes
        .iter()
        .filter_map(|cause| {
            if let InventoryCause::Collected { entity_ids } = cause {
                Some(entity_ids.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(collected, vec![vec![1182]]);
    client.disconnect().await.unwrap();
}

/// Packet evidence failures must stay latched even after a matching later full resync.
pub(crate) async fn crafting_pickup_adversarial(f: &mut impl Fixture, case: u8) {
    use crate::client::OperationalCraftingStage as Stage;
    let client = f.client();
    let mut slots = vec![SlotKnowledge::Empty; 46];
    let output = item(client.version(), "minecraft:oak_planks", 4);
    slots[0] = output.clone();
    slots[1] = item(client.version(), "minecraft:oak_log", 1);
    f.item_entity(if case == 1 { 9999 } else { 1182 }, 1).await;
    f.inventory(&slots, &SlotKnowledge::Empty).await;
    let grid = client.received_crafting().await.unwrap().unwrap();
    let send = client
        .survival()
        .send_operational_crafting_result(&grid)
        .await
        .unwrap();
    f.click().await;
    f.collect(1182, true).await;
    if case == 4 {
        f.collect(1182, true).await;
    }
    if case == 7 {
        for id in 2000..2064 {
            f.item_entity(id, 1).await;
            f.collect(id, true).await;
        }
    }
    if case == 8 {
        f.cursor(&item(client.version(), "minecraft:stone", 1))
            .await;
    }
    if case == 5 {
        f.slot(36, &item(client.version(), "minecraft:oak_log", 1))
            .await;
        f.slot(36, &SlotKnowledge::Empty).await;
    }
    slots[0] = SlotKnowledge::Empty;
    if case != 3 {
        slots[1] = SlotKnowledge::Empty;
    }
    slots[36] = item(
        client.version(),
        if case == 2 {
            "minecraft:stone"
        } else {
            "minecraft:oak_log"
        },
        if case == 0 { 2 } else { 1 },
    );
    f.complete(&send, &slots, &output, false).await;
    if case == 9 {
        assert_eq!(
            client
                .operational_crafting_outcome(send.id)
                .await
                .unwrap()
                .stage,
            Stage::Crafted
        );
        f.collect(1182, true).await;
    }
    if case == 6 {
        assert_eq!(
            client
                .operational_crafting_outcome(send.id)
                .await
                .unwrap()
                .stage,
            Stage::Crafted
        );
        slots[37] = item(client.version(), "minecraft:stone", 1);
        f.slot(37, &slots[37]).await;
    }
    let outcome = client.operational_crafting_outcome(send.id).await.unwrap();
    assert_eq!(
        outcome.stage,
        Stage::RequiresInspection,
        "case={case}: {outcome:?}"
    );
    let first = outcome.requires_inspection;
    f.inventory(&slots, &output).await;
    let later = client.operational_crafting_outcome(send.id).await.unwrap();
    assert_eq!(later.stage, Stage::RequiresInspection);
    assert_eq!(later.requires_inspection, first);
    assert!(client.continue_operational_crafting(send.id).await.is_err());
    client.disconnect().await.unwrap();
}

/// The six excerpt move shapes, with an old cursor on both sides of send.
/// The supplied reports lack raw sends/later cursor, so these are deterministic
/// schedules supported by their frozen sequence relationships, not replay claims.
pub(crate) async fn mo7_excerpt(f: &mut impl Fixture, case: usize) {
    let client = f.client();
    let v = client.version();
    let table = matches!(case, 0 | 1 | 3 | 4);
    let mut slots = vec![SlotKnowledge::Empty; 46];
    let (name, count, slot, right) = match case {
        0 => ("stick", 2, 11, false),
        1 => ("stick", 2, 11, false),
        2 => ("oak_planks", 3, 9, false),
        3 => ("stick", 3, 8, true),
        4 => ("wooden_axe", 1, 12, false),
        _ => ("oak_log", 1, 1, true),
    };
    let mut old_cursor = item(v, &format!("minecraft:{name}"), count);
    if case == 4 && v == MinecraftVersion::Java1_16_1 {
        if let SlotKnowledge::Item { item } = &mut old_cursor {
            item.data = crate::client::ItemData::LegacyNbt {
                bytes: vec![10, 0, 0, 3, 0, 6, 68, 97, 109, 97, 103, 101, 0, 0, 0, 0, 0],
            };
        }
    }
    if table {
        slots[10] = item(v, "minecraft:oak_planks", 3);
    }
    if case == 4 {
        slots[11] = item(v, "minecraft:stick", 2);
    }
    if case == 5 {
        slots[9] = item(v, "minecraft:oak_planks", 4);
    }
    if matches!(case, 0 | 1 | 3) {
        for index in [1, 2, 4] {
            slots[index] = item(v, "minecraft:oak_planks", 1);
        }
        slots[5] = item(v, "minecraft:stick", 1);
        if case != 3 {
            slots[8] = item(v, "minecraft:stick", 1);
            slots[0] = item(v, "minecraft:wooden_axe", 1);
        }
    }
    if case == 2 {
        slots[1] = item(v, "minecraft:oak_planks", 1);
        slots[0] = item(v, "minecraft:oak_button", 1);
    }
    f.inventory(&vec![SlotKnowledge::Empty; 46], &old_cursor)
        .await;
    if table {
        f.open_crafting_table().await;
    }
    let window = if table { 3 } else { 0 };
    f.contents(window, &slots, &old_cursor).await;
    f.cursor(&old_cursor).await;
    let source = if table {
        InventorySource::Container {
            screen: client.screen_state().await.unwrap().screen.unwrap().id,
        }
    } else {
        InventorySource::Player
    };
    let sent = client
        .survival()
        .send_inventory_click(
            source,
            slot,
            if right { Button::Right } else { Button::Left },
        )
        .await
        .unwrap();
    f.click().await;
    f.cursor(&old_cursor).await; // newer than send, still belonging to previous state
    if v == MinecraftVersion::Java1_16_1 {
        f.response(&sent, false).await;
    }
    slots[slot as usize] = if right {
        item(v, &format!("minecraft:{name}"), 1)
    } else {
        old_cursor.clone()
    };
    let cursor = if right && count > 1 {
        item(v, &format!("minecraft:{name}"), count - 1)
    } else {
        SlotKnowledge::Empty
    };
    if case == 3 {
        slots[0] = item(v, "minecraft:wooden_axe", 1);
    }
    if case == 5 {
        slots[0] = item(v, "minecraft:oak_planks", 4);
    }
    f.contents(window, &slots, &cursor).await;
    let pending = client
        .survival()
        .inventory_send_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        pending.status,
        InventorySendStatus::Pending,
        "case={case}, {v:?}"
    );
    assert!(
        client
            .inventory_divergence(sent.id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        client
            .survival()
            .send_inventory_click(source, slot, Button::Left)
            .await
            .is_err()
    );
    if v == MinecraftVersion::Java1_16_1 {
        f.cursor(&cursor).await;
    } else {
        f.response(&sent, false).await;
    }
    let done = client
        .survival()
        .inventory_send_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.id, sent.id);
    assert_eq!(
        done.status,
        InventorySendStatus::Received,
        "case={case}, {v:?}"
    );
    assert!(
        client
            .inventory_divergence(sent.id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        client
            .survival()
            .inventory_click_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        InventoryClickStage::ObservedClicked
    );
}

/// Missing cursor/full boundary: one no-effect probe, same original identity;
/// original reply cannot complete the probe, and a second timeout is typed.
pub(crate) async fn mo7_deadline(f: &mut impl Fixture, outcome: usize, transfer: bool) {
    let client = f.client();
    let v = client.version();
    let mut slots = vec![SlotKnowledge::Empty; 46];
    let stack = item(v, "minecraft:stone", 3);
    slots[9] = stack.clone();
    f.inventory(&slots, &SlotKnowledge::Empty).await;
    let sent = if transfer {
        client
            .survival()
            .send_inventory_transfer(InventorySource::Player, 9)
            .await
    } else {
        client
            .survival()
            .send_inventory_click(InventorySource::Player, 9, Button::Left)
            .await
    }
    .unwrap();
    f.click().await;
    tokio::time::sleep(Duration::from_millis(350)).await;
    f.crafting_probe().await;
    let probe = client
        .survival()
        .inventory_send_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(probe.id, sent.id);
    assert!(probe.deadline_resync_dispatched);
    assert_eq!(probe.status, InventorySendStatus::Pending);
    // A late original reply/fence, even followed by fresh state, does not own
    // the new probe. Its native action/token is a distinct response debt.
    f.response(&sent, false).await;
    f.contents(0, &slots, &SlotKnowledge::Empty).await;
    f.cursor(&SlotKnowledge::Empty).await;
    assert_eq!(
        client
            .survival()
            .inventory_send_record()
            .await
            .unwrap()
            .unwrap()
            .status,
        InventorySendStatus::Pending
    );
    assert!(
        client
            .inventory_divergence(sent.id)
            .await
            .unwrap()
            .is_none()
    );
    if outcome == 2 {
        tokio::time::sleep(Duration::from_millis(350)).await;
        let expired = client
            .survival()
            .inventory_send_record()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(expired.status, InventorySendStatus::ResyncRequired);
        assert_eq!(
            expired.inspection_reason,
            Some(InventorySendInspectionReason::ReceiptDeadlineAfterResync)
        );
        f.complete(&probe, &slots, &SlotKnowledge::Empty, false)
            .await;
        assert_eq!(
            client
                .survival()
                .inventory_send_record()
                .await
                .unwrap()
                .unwrap()
                .status,
            InventorySendStatus::ResyncRequired
        );
        assert!(
            client
                .inventory_divergence(sent.id)
                .await
                .unwrap()
                .is_none()
        );
    } else {
        let cursor = if outcome == 0 {
            slots[9] = SlotKnowledge::Empty;
            if transfer {
                slots[36] = stack.clone();
                SlotKnowledge::Empty
            } else {
                stack
            }
        } else {
            SlotKnowledge::Empty
        };
        f.complete(&probe, &slots, &cursor, false).await;
        let done = client
            .survival()
            .inventory_send_record()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(done.id, sent.id);
        assert_eq!(
            done.status,
            if outcome == 0 {
                InventorySendStatus::Received
            } else {
                InventorySendStatus::Diverged
            }
        );
        if outcome == 1 {
            assert_eq!(
                client
                    .inventory_divergence(sent.id)
                    .await
                    .unwrap()
                    .unwrap()
                    .effect(),
                InventoryEffect::NoEffect
            );
        }
    }
}

/// Identical deterministic packet schedules on both protocol adapters.
pub(crate) async fn inspected_recovery(f: &mut impl Fixture, case: u8) {
    use super::{InspectedCraftingRecoveryResult as Recovery, InspectedRecoveryRejection as R};
    use crate::client::OperationalCraftingStage as Stage;
    let client = f.client();
    let mut slots = vec![SlotKnowledge::Empty; 46];
    let output = item(client.version(), "minecraft:oak_planks", 4);
    slots[0] = output.clone();
    slots[1] = item(client.version(), "minecraft:oak_log", 1);
    f.inventory(&slots, &SlotKnowledge::Empty).await;
    let old = client.received_player_snapshot().await.unwrap();
    f.item_entity(1182, 1).await;
    let grid = client.received_crafting().await.unwrap().unwrap();
    let send = client
        .survival()
        .send_operational_crafting_result(&grid)
        .await
        .unwrap();
    f.click().await;
    // Claiming Pending cannot be used as an inspection escape hatch.
    client.operational_crafting_outcome(send.id).await.unwrap();
    assert!(matches!(
        client
            .recover_inspected_operational_crafting(send.id, &old)
            .await
            .unwrap(),
        Recovery::Rejected {
            reason: R::NotInspectionTerminal
        }
    ));
    if case == 2 {
        f.collect(1182, true).await;
    }
    slots[0] = SlotKnowledge::Empty;
    slots[1] = SlotKnowledge::Empty;
    slots[10] = item(client.version(), "minecraft:oak_log", 1);
    if case == 3 {
        slots[36] = slots[10].clone();
    }
    // Early contradictory stock packet latches inspection while response debt remains.
    if case == 0 {
        f.slot(10, &slots[10]).await;
        assert_eq!(
            client
                .operational_crafting_outcome(send.id)
                .await
                .unwrap()
                .stage,
            Stage::Pending
        );
        f.slot(10, &SlotKnowledge::Empty).await;
        let pending_basis = client.received_player_snapshot().await.unwrap();
        assert_eq!(
            client
                .operational_crafting_outcome(send.id)
                .await
                .unwrap()
                .stage,
            Stage::RequiresInspection
        );
        assert!(matches!(
            client
                .recover_inspected_operational_crafting(send.id, &pending_basis)
                .await
                .unwrap(),
            Recovery::Rejected {
                reason: R::PendingDebt
            }
        ));
    }
    f.complete(&send, &slots, &output, false).await;
    if case == 2 {
        let crafted = client.operational_crafting_outcome(send.id).await.unwrap();
        assert_eq!(crafted.stage, Stage::Crafted);
        assert_eq!(crafted.external.len(), 1);
        // Inspection after proof must preserve already-proven External evidence.
        f.collect(1182, true).await;
    }
    let original = client.operational_crafting_outcome(send.id).await.unwrap();
    assert_eq!(original.stage, Stage::RequiresInspection, "{original:?}");
    assert!(client.continue_operational_crafting(send.id).await.is_err());
    let take = client
        .survival()
        .crafting_take_record()
        .await
        .unwrap()
        .unwrap();
    let original_send = client
        .survival()
        .inventory_send_record()
        .await
        .unwrap()
        .unwrap();
    let divergence = client.inventory_divergence(send.id).await.unwrap();
    let fresh = client.received_player_snapshot().await.unwrap();
    if case == 1 {
        f.open_crafting_table().await;
        assert!(matches!(
            client
                .recover_inspected_operational_crafting(send.id, &fresh)
                .await
                .unwrap(),
            Recovery::Rejected {
                reason: R::ScopeMismatch
            }
        ));
        f.close_received().await;
        f.inventory(&slots, &output).await;
        let returned = client.received_player_snapshot().await.unwrap();
        assert!(matches!(
            client
                .recover_inspected_operational_crafting(send.id, &returned)
                .await
                .unwrap(),
            Recovery::Rejected {
                reason: R::ScopeMismatch
            }
        ));
        client.disconnect().await.unwrap();
        return;
    }
    let mut missing_full = fresh.clone();
    missing_full.inventory.last_full_contents = crate::client::FieldState::Missing;
    assert!(matches!(
        client
            .recover_inspected_operational_crafting(send.id, &missing_full)
            .await
            .unwrap(),
        Recovery::Rejected {
            reason: R::IncompleteOrOldBasis
        }
    ));
    let mut missing_cursor = fresh.clone();
    missing_cursor.inventory.cursor = crate::client::FieldState::Missing;
    assert!(matches!(
        client
            .recover_inspected_operational_crafting(send.id, &missing_cursor)
            .await
            .unwrap(),
        Recovery::Rejected {
            reason: R::IncompleteOrOldBasis
        }
    ));
    assert!(matches!(
        client
            .recover_inspected_operational_crafting(send.id, &old)
            .await
            .unwrap(),
        Recovery::Rejected {
            reason: R::IncompleteOrOldBasis
        }
    ));
    for world in [false, true] {
        let mut wrong = fresh.clone();
        if world {
            wrong.boundary.session.world_generation += 1;
        } else {
            wrong.boundary.session.connection_id += 1;
        }
        assert!(matches!(
            client
                .recover_inspected_operational_crafting(send.id, &wrong)
                .await
                .unwrap(),
            Recovery::Rejected {
                reason: R::ScopeMismatch
            }
        ));
    }
    // A post-capture slot receipt, even with identical value, invalidates the basis.
    f.slot(10, &slots[10]).await;
    assert!(matches!(
        client
            .recover_inspected_operational_crafting(send.id, &fresh)
            .await
            .unwrap(),
        Recovery::Rejected {
            reason: R::StaleBasis
        }
    ));
    let basis = if case == 0 {
        let resync = client.survival().resync_inventory().await.unwrap();
        f.click().await;
        let pending = client.received_player_snapshot().await.unwrap();
        assert!(matches!(
            client
                .recover_inspected_operational_crafting(send.id, &pending)
                .await
                .unwrap(),
            Recovery::Rejected {
                reason: R::PendingDebt
            }
        ));
        // A full contents/cursor observation cannot stand in for missing response debt.
        f.inventory(&slots, &output).await;
        let no_response = client.received_player_snapshot().await.unwrap();
        assert!(matches!(
            client
                .recover_inspected_operational_crafting(send.id, &no_response)
                .await
                .unwrap(),
            Recovery::Rejected {
                reason: R::PendingDebt
            }
        ));
        f.complete(&resync, &slots, &output, false).await;
        client.received_player_snapshot().await.unwrap()
    } else {
        f.inventory(&slots, &output).await;
        client.received_player_snapshot().await.unwrap()
    };
    assert!(
        client
            .survival()
            .send_inventory_click(InventorySource::Player, 11, Button::Left)
            .await
            .is_err()
    );
    let result = client
        .recover_inspected_operational_crafting(send.id, &basis)
        .await
        .unwrap();
    let Recovery::Released { recovery } = result else {
        panic!("{result:?}");
    };
    assert_eq!(
        serde_json::to_value(&recovery.original).unwrap(),
        serde_json::to_value(&original).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&recovery.original_send).unwrap(),
        serde_json::to_value(&original_send).unwrap()
    );
    // Inspection diagnostics may accumulate fresh observations, never change verdict or ID.
    assert_eq!(
        serde_json::to_value(&recovery.original_divergence).unwrap(),
        serde_json::to_value(&divergence).unwrap()
    );
    assert_eq!(recovery.original_take.id, take.id);
    assert_eq!(recovery.original_take.stage, take.stage);
    assert_eq!(
        recovery.original_take.requires_inspection,
        take.requires_inspection
    );
    assert_eq!(recovery.basis_sequence, basis.boundary().ordinal());
    assert!(recovery.contents_sequence > send.after_sequence);
    assert_eq!(recovery.basis.cursor.as_ref().unwrap().value, output);
    assert!(matches!(
        client
            .recover_inspected_operational_crafting(send.id, &basis)
            .await
            .unwrap(),
        Recovery::Rejected {
            reason: R::NotInspectionTerminal
        }
    ));
    // Strict inspection stays conservative. Acceptance plans a new put-down.
    if take.stage == crate::client::CraftingTakeStage::RequiresInspection {
        assert!(
            client
                .survival()
                .click_inventory(InventorySource::Player, 11, Button::Left)
                .await
                .is_err()
        );
    }
    let next = client
        .survival()
        .send_inventory_click(InventorySource::Player, 11, Button::Left)
        .await
        .unwrap();
    f.click().await;
    slots[11] = output;
    f.complete(&next, &slots, &SlotKnowledge::Empty, false)
        .await;
    assert_eq!(
        client
            .survival()
            .inventory_send_record()
            .await
            .unwrap()
            .unwrap()
            .status,
        InventorySendStatus::Received
    );
    assert_eq!(
        serde_json::to_value(
            client
                .inspected_crafting_recovery(send.id)
                .await
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(&recovery).unwrap()
    );
    client.disconnect().await.unwrap();
}

pub(crate) async fn inspected_table_recovery(f: &mut impl Fixture, reopen: bool) {
    use super::{InspectedCraftingRecoveryResult as Recovery, InspectedRecoveryRejection as R};
    let client = f.client();
    let mut slots = vec![SlotKnowledge::Empty; 46];
    f.inventory(&slots, &SlotKnowledge::Empty).await;
    f.item_entity(1182, 1).await;
    f.open_crafting_table().await;
    let output = item(client.version(), "minecraft:oak_planks", 4);
    slots[0] = output.clone();
    slots[1] = item(client.version(), "minecraft:oak_log", 1);
    f.contents(3, &slots, &SlotKnowledge::Empty).await;
    if client.version() == MinecraftVersion::Java1_16_1 {
        f.cursor(&SlotKnowledge::Empty).await;
    }
    let grid = client.received_crafting().await.unwrap().unwrap();
    let send = client
        .survival()
        .send_operational_crafting_result(&grid)
        .await
        .unwrap();
    f.click().await;
    slots[0] = SlotKnowledge::Empty;
    slots[1] = SlotKnowledge::Empty;
    slots[10] = item(client.version(), "minecraft:oak_log", 1); // unexplained actual gain
    if client.version() == MinecraftVersion::Java1_16_1 {
        f.response(&send, false).await;
        f.contents(3, &slots, &output).await;
        f.cursor(&output).await;
    } else {
        f.contents(3, &slots, &output).await;
        f.response(&send, false).await;
    }
    let original = client.operational_crafting_outcome(send.id).await.unwrap();
    assert_eq!(
        original.stage,
        crate::client::OperationalCraftingStage::RequiresInspection
    );
    let basis = client.received_player_snapshot().await.unwrap();
    if reopen {
        f.open_crafting_table().await; // same native ID, different opening identity
        f.contents(3, &slots, &output).await;
        assert!(matches!(
            client
                .recover_inspected_operational_crafting(send.id, &basis)
                .await
                .unwrap(),
            Recovery::Rejected {
                reason: R::ScopeMismatch
            }
        ));
    } else {
        let result = client
            .recover_inspected_operational_crafting(send.id, &basis)
            .await
            .unwrap();
        let Recovery::Released { recovery } = result else {
            panic!("{result:?}")
        };
        assert_eq!(
            serde_json::to_value(recovery.original).unwrap(),
            serde_json::to_value(original).unwrap()
        );
        assert_eq!(recovery.basis.source, send.id.source());
        let next = client
            .survival()
            .send_inventory_click(send.id.source(), 11, Button::Left)
            .await
            .unwrap();
        f.click().await;
        slots[11] = output;
        if client.version() == MinecraftVersion::Java1_16_1 {
            f.response(&next, false).await;
            f.contents(3, &slots, &SlotKnowledge::Empty).await;
            f.cursor(&SlotKnowledge::Empty).await;
        } else {
            f.contents(3, &slots, &SlotKnowledge::Empty).await;
            f.response(&next, false).await;
        }
        assert_eq!(
            client
                .survival()
                .inventory_send_record()
                .await
                .unwrap()
                .unwrap()
                .status,
            InventorySendStatus::Received
        );
    }
    client.disconnect().await.unwrap();
}

/// Public capture keeps the original boundary even across later receives/close.
pub(crate) async fn atomic_inventory_capture(f: &mut impl Fixture) {
    use crate::client::container::{ActiveScreen, ActiveScreenUnknown, PlayerScreenAccess};
    let client = f.client();
    let mut slots = vec![SlotKnowledge::Empty; 46];
    f.inventory(&slots, &SlotKnowledge::Empty).await;
    f.open_crafting_table().await;
    f.contents(3, &slots, &SlotKnowledge::Empty).await;
    f.cursor(&SlotKnowledge::Empty).await;
    let old = client.survival().capture_inventory_view().await.unwrap();
    let screen = old.screen().active_screen().as_open().unwrap().id;
    assert_eq!(old.player().session, old.screen().session);
    assert_eq!(old.player().receive_sequence, old.screen().receive_sequence);
    assert_eq!(old.player().inventory.window_id, old.screen().active_window);
    assert_eq!(
        old.player().inventory.player_screen,
        old.screen().player_screen
    );
    assert_eq!(old.player().inventory.cursor, old.screen().cursor);
    let basis = old.snapshot();
    assert_eq!(basis.screen.as_ref().map(|s| s.id), Some(screen));
    assert!(basis.basis_matches_current(&old));
    // Selection is relative to each observation, including invalid/missing IDs.
    for window in [None, Some(0), Some(4), Some(3)] {
        let mut observation = old.screen().clone();
        observation.active_window = window;
        let mut inventory = old.player().inventory.clone();
        inventory.window_id = window;
        let expected = InventorySnapshot::from_inventory(
            observation.session,
            observation.receive_sequence,
            &inventory,
            observation.screen.clone(),
        );
        assert_eq!(
            expected.screen.as_ref().map(|s| s.id),
            observation.active_screen().as_open().map(|s| s.id)
        );
        assert_eq!(
            observation.active_screen().as_open().is_some(),
            window == Some(3)
        );
    }
    let writes = async {
        for count in 1..=32 {
            slots[10] = item(client.version(), "minecraft:oak_log", count);
            f.contents(3, &slots, &SlotKnowledge::Empty).await;
            tokio::task::yield_now().await;
        }
    };
    let reads = async {
        for _ in 0..64 {
            let cut = client.creative().capture_inventory_view().await.unwrap();
            assert_eq!(cut.player().receive_sequence, cut.screen().receive_sequence);
            assert_eq!(cut.player().session, cut.screen().session);
            // Table screen slot 10 aliases canonical player slot 9.
            assert_eq!(
                cut.player().inventory.slots[9],
                cut.screen().active_screen().as_open().unwrap().slots[10]
            );
            tokio::task::yield_now().await;
        }
    };
    tokio::join!(writes, reads);
    let changed = client.capture_inventory_view().await.unwrap();
    assert!(changed.player().receive_sequence > old.player().receive_sequence);
    assert!(!basis.basis_matches_current(&changed));
    assert!(basis.basis_matches_current(&old));
    let mut wrong_session = changed.snapshot();
    wrong_session.session.world_generation += 1;
    assert!(!wrong_session.basis_matches_current(&changed));
    wrong_session = changed.snapshot();
    wrong_session.session.connection_id += 1;
    assert!(!wrong_session.basis_matches_current(&changed));
    // Identical-value new receipts are accepted by ordinary continuation.
    let same_values = changed.snapshot();
    f.contents(3, &slots, &SlotKnowledge::Empty).await;
    assert!(same_values.basis_matches_current(&client.capture_inventory_view().await.unwrap()));
    let close = client.survival().close_container(screen).await.unwrap();
    let closed = client.capture_inventory_view().await.unwrap();
    assert!(matches!(closed.screen().player_screen,
        Some(crate::client::container::PlayerScreenAccess::SubmittedClose { close: id }) if id == close.id));
    assert_eq!(closed.screen().screen.as_ref().unwrap().id, screen);
    assert!(matches!(
        closed.screen().active_screen(),
        ActiveScreen::Closed
    ));
    assert!(closed.snapshot().screen.is_none());
    assert_eq!(closed.snapshot().source, InventorySource::Player);
    assert!(!same_values.basis_matches_current(&closed));
    assert!(old.screen().active_screen().as_open().is_some());

    // A real new opening reuses the same numeric ID, but not its ScreenId.
    f.open_crafting_table().await;
    f.contents(3, &slots, &SlotKnowledge::Empty).await;
    f.cursor(&SlotKnowledge::Empty).await;
    let reopened = client.capture_inventory_view().await.unwrap();
    let new_screen = reopened.screen().active_screen().as_open().unwrap().id;
    assert_eq!(screen.window_id(), new_screen.window_id());
    assert_ne!(screen.opened_sequence(), new_screen.opened_sequence());
    assert!(!same_values.basis_matches_current(&reopened));
    let mut different_source = reopened.snapshot();
    different_source.source = InventorySource::Player;
    assert!(!different_source.basis_matches_current(&reopened));
    let mut different_cursor = reopened.snapshot();
    different_cursor.cursor = None;
    assert!(!different_cursor.basis_matches_current(&reopened));

    // Every observation variant shares snapshot selection, including stale
    // closes for another opening, missing evidence, and mismatched windows.
    for retained in [
        old.screen().screen.clone(),
        reopened.screen().screen.clone(),
        None,
    ] {
        for access in [
            None,
            Some(PlayerScreenAccess::Received),
            closed.screen().player_screen,
        ] {
            for window in [None, Some(0), Some(3), Some(4)] {
                let mut observation = reopened.screen().clone();
                observation.screen = retained.clone();
                observation.player_screen = access;
                observation.active_window = window;
                let selection = observation.active_screen();
                if matches!(access, Some(PlayerScreenAccess::SubmittedClose { .. })) {
                    if retained.as_ref().is_some_and(|s| s.id == screen) {
                        assert!(matches!(selection, ActiveScreen::Closed));
                    } else {
                        assert!(matches!(selection,
                            ActiveScreen::Unknown(ActiveScreenUnknown::CloseTargetsDifferentScreen {
                                closed, observed
                            }) if closed == screen && observed == retained.as_ref().map(|s| s.id)));
                    }
                } else if retained.is_some() {
                    if window == Some(3) {
                        assert!(matches!(selection, ActiveScreen::Open(_)));
                    } else {
                        assert!(matches!(
                            selection,
                            ActiveScreen::Unknown(ActiveScreenUnknown::WindowMismatch)
                        ));
                    }
                } else if window == Some(0) {
                    assert!(matches!(selection, ActiveScreen::Closed));
                } else {
                    assert!(matches!(
                        selection,
                        ActiveScreen::Unknown(ActiveScreenUnknown::MissingScreen)
                    ));
                }
                let mut inventory = reopened.player().inventory.clone();
                inventory.window_id = window;
                inventory.player_screen = access;
                let snapshot = InventorySnapshot::from_inventory(
                    observation.session,
                    observation.receive_sequence,
                    &inventory,
                    retained.clone(),
                );
                assert_eq!(
                    snapshot.screen.as_ref().map(|s| s.id),
                    selection.as_open().map(|s| s.id)
                );
            }
        }
    }
}
