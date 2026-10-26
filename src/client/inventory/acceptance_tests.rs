//! The same adversarial receive script runs against both real packet adapters.
use super::*;
use crate::MinecraftVersion;
use crate::client::registry::Registry;
use crate::client::{Client, SlotKnowledge, inventory::InventoryClickButton as Button};

pub(crate) trait Fixture {
    fn client(&self) -> Client;
    async fn inventory(&mut self, slots: &[SlotKnowledge], cursor: &SlotKnowledge);
    async fn slot(&mut self, slot: u16, value: &SlotKnowledge);
    async fn cursor(&mut self, value: &SlotKnowledge);
    async fn response(&mut self, send: &InventorySendRecord, accepted: bool);
    async fn collect(&mut self, entity: i32, own: bool);
    /// Consume exactly one original click (and the modern response request).
    async fn click(&mut self);
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
    f.inventory(&slots, &cursor).await;
    f.response(&send, false).await;
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
pub(crate) async fn crafting_result_outcomes(f: &mut impl Fixture) {
    use crate::client::crafting::CraftingResultDestination;
    let client = f.client();
    for destination in [
        CraftingResultDestination::Cursor,
        CraftingResultDestination::Inventory,
    ] {
        for collect in [false, true] {
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
            f.inventory(&slots, &cursor).await;
            f.response(&sent, false).await;
            assert_eq!(
                client
                    .survival()
                    .inventory_send_record()
                    .await
                    .unwrap()
                    .unwrap()
                    .status,
                if collect {
                    InventorySendStatus::Diverged
                } else {
                    InventorySendStatus::Received
                },
                "{:?} {destination:?} collect={collect}",
                client.version()
            );
            let divergence = client.inventory_divergence(sent.id).await.unwrap();
            assert_eq!(divergence.is_some(), collect);
            if let Some(d) = divergence {
                assert!(matches!(
                    d.intended,
                    InventoryIntendedChange::CraftingResult { .. }
                ));
                assert!(d.causes.contains(&InventoryCause::Collected {
                    entity_ids: vec![333]
                }));
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
fn item(version: MinecraftVersion, name: &str, count: u32) -> SlotKnowledge {
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
                    for response_first in [false, true] {
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
                                assert!(d.causes.contains(&InventoryCause::Rejected));
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
                        f.inventory(&slots, &cursor).await;
                        f.response(&next, true).await;
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
    tokio::time::sleep(std::time::Duration::from_millis(5100)).await;
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
    f.inventory(&slots, &cursor).await;
    f.response(&old, true).await; // expired action/token, arriving during the next click
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
