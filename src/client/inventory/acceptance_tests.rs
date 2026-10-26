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
    /// Consume exactly one original click (and the modern response request).
    async fn click(&mut self);
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
                            InventorySendStatus::Received,
                            "{version:?} transfer={transfer} grid={crafting_input} timing={timing} injection={injection}"
                        );
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
        InventorySendStatus::Received
    );
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
