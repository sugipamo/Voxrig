//! Legacy one-shot ordinary SWAP with actor ownership and real received evidence.
use super::*;
use crate::client::{self as api, inventory as contract};
use contract::{InventorySwapId, InventorySwapRecord, InventorySwapStage};
#[derive(Clone)]
pub(super) struct NativeInventorySwap {
    pub(super) record: InventorySwapRecord,
    pub(super) released: bool,
}

impl Bot {
    pub(crate) async fn common_swap_hotbar(
        &self,
        mode: api::GameMode,
        main: u8,
        hotbar: u8,
    ) -> Result<InventorySwapRecord> {
        let gate = self.coherent_state_gate.lock().await;
        self.common_motion_admission().await?;
        let revision = self
            .connection
            .motion_admission_revision()
            .await
            .map_err(|e| contract::unavailable(format!("inventory admission: {e:?}")))?;
        {
            let inventory = self.inventory.read().await;
            if inventory.open_window.is_some() || !inventory.pending_clicks.is_empty() {
                return Err(contract::unavailable(
                    "legacy inventory operation unresolved",
                ));
            }
        }
        let attempt = self
            .common_inventory_swap
            .lock()
            .await
            .as_ref()
            .map_or(Some(1), |p| p.record.id.attempt().checked_add(1))
            .ok_or_else(|| contract::unavailable("inventory attempts exhausted"))?;
        let record = contract::prepare(
            self.common_player_unlocked().await?,
            mode,
            main,
            hotbar,
            attempt,
        )?;
        let id = record.id;
        *self.common_inventory_swap.lock().await = Some(NativeInventorySwap {
            record,
            released: false,
        });
        let (reply, result) = tokio::sync::oneshot::channel();
        let bot = self.clone_internal();
        tokio::spawn(async move {
            let _ = reply.send(bot.common_swap_send_owned(id, revision).await);
        });
        drop(gate);
        result
            .await
            .map_err(|_| contract::unavailable("inventory owner result unavailable"))?
    }
    async fn common_swap_send_owned(
        &self,
        id: InventorySwapId,
        revision: u64,
    ) -> Result<InventorySwapRecord> {
        let _gate = self.coherent_state_gate.lock().await;
        let result = async {
            let record = self
                .common_inventory_swap
                .lock()
                .await
                .as_ref()
                .filter(|s| s.record.id == id)
                .map(|s| s.record.clone())
                .ok_or_else(|| contract::unavailable("inventory intent superseded"))?;
            if record.requires_inspection.is_some() {
                return Err(contract::unavailable(
                    "inventory context interrupted before I/O",
                ));
            }
            let current = self.common_player_unlocked().await?;
            if current.session != record.initial.session
                || current.game_mode != Some(record.mode)
                || current.inventory.window_id != Some(0)
                || current.inventory.cursor.as_ref().map(|v| &v.value)
                    != Some(&api::SlotKnowledge::Empty)
                || current.inventory.slots[usize::from(record.main_slot)].as_ref()
                    != Some(&record.main_before)
                || current.inventory.slots[36 + usize::from(record.hotbar)].as_ref()
                    != Some(&record.hotbar_before)
            {
                return Err(contract::unavailable(
                    "inventory capture changed before I/O",
                ));
            }
            let action = self
                .connection
                .begin_inventory_swap(id.attempt(), revision)
                .await
                .map_err(|e| contract::unavailable(format!("inventory reservation: {e:?}")))?;
            let comparison = match (&record.main_before.value, &record.hotbar_before.value) {
                (api::SlotKnowledge::Item { item }, _) | (_, api::SlotKnowledge::Item { item }) => {
                    item.clone()
                }
                _ => {
                    return Err(contract::unavailable(
                        "nonempty comparison predecessor missing",
                    ));
                }
            };
            {
                let mut guard = self.common_inventory_swap.lock().await;
                let record = &mut guard.as_mut().expect("retained").record;
                record.send.legacy_action = Some(action);
                record.send.legacy_comparison = Some(comparison.clone());
                record.send.after_sequence = self.protocol_packet_sequence.load(Ordering::Acquire);
            }
            self.connection
                .bounded_inventory_swap(
                    id.attempt(),
                    record.main_slot,
                    record.hotbar,
                    ItemStack {
                        item_id: comparison.id.value(),
                        count: comparison.count as i8,
                        nbt: None,
                    },
                )
                .await?;
            let mut guard = self.common_inventory_swap.lock().await;
            let record = &mut guard.as_mut().expect("retained").record;
            record.send.dispatched = true;
            Ok(record.clone())
        }
        .await;
        if let Err(error) = &result {
            self.interrupt_common_inventory_swap(format!(
                "inventory submission uncertain: {error}"
            ))
            .await;
        }
        result
    }
    pub(crate) async fn common_inventory_swap_record(&self) -> Result<Option<InventorySwapRecord>> {
        let bot = self.clone_internal();
        let (reply, result) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let value = async {
                let _gate = bot.coherent_state_gate.lock().await;
                bot.reconcile_common_inventory_swap(true).await?;
                Ok(bot
                    .common_inventory_swap
                    .lock()
                    .await
                    .as_ref()
                    .map(|s| s.record.clone()))
            }
            .await;
            let _ = reply.send(value);
        });
        result
            .await
            .map_err(|_| contract::unavailable("inventory inspection owner unavailable"))?
    }
    pub(super) async fn interrupt_common_inventory_swap(&self, reason: impl std::fmt::Display) {
        if let Some(run) = self.common_inventory_swap.lock().await.as_mut() {
            contract::inspection(&mut run.record, reason);
        }
    }
    pub(super) async fn common_inventory_context_received(&self) -> Result<()> {
        self.reconcile_common_inventory_swap(false).await
    }
    pub(super) async fn common_inventory_reply_received(&self, reply: WindowTransaction) {
        let mut guard = self.common_inventory_swap.lock().await;
        if let Some(run) = guard.as_mut().filter(|s| {
            s.record.stage != InventorySwapStage::ObservedSwapped
                && s.record.send.legacy_action == Some(reply.action)
                && reply.window_id == 0
                && reply.packet_sequence > s.record.send.after_sequence
        }) {
            if run.record.legacy_reply.is_none() {
                run.record.legacy_reply = Some(contract::InventoryTransactionReply {
                    window_id: reply.window_id,
                    action: reply.action,
                    accepted: reply.accepted,
                    receive_sequence: reply.packet_sequence,
                });
            }
            // False comparison triggers resync AFTER native applies the click.
            // Fresh exact destinations, not this boolean, decide the outcome.
        }
    }
    async fn reconcile_common_inventory_swap(&self, confirm: bool) -> Result<()> {
        let Some(snapshot) = self.common_inventory_swap.lock().await.clone() else {
            return Ok(());
        };
        if snapshot.record.stage == InventorySwapStage::ObservedSwapped {
            return Ok(());
        }
        if self.is_stopped() || self.connection_state() != ConnectionState::Ready {
            self.interrupt_common_inventory_swap("inventory connection closed or uncertain")
                .await;
            return Ok(());
        }
        let current = self.common_player_unlocked().await?;
        let inventory = self.inventory.read().await;
        let native_conflict = inventory.cursor.is_some()
            || inventory.open_window.is_some()
            || !inventory.pending_clicks.is_empty();
        drop(inventory);
        let complete = {
            let mut guard = self.common_inventory_swap.lock().await;
            let record = &mut guard.as_mut().expect("retained").record;
            contract::receive(record, &current);
            if native_conflict {
                contract::inspection(record, "legacy inventory cache/click context changed");
            }
            confirm && contract::destinations_ready(record) && record.legacy_reply.is_some()
        };
        if complete {
            match self
                .connection
                .finish_inventory_swap(snapshot.record.id.attempt())
                .await
            {
                Ok(()) => {
                    let mut guard = self.common_inventory_swap.lock().await;
                    let run = guard.as_mut().expect("retained");
                    run.released = true;
                    run.record.stage = InventorySwapStage::ObservedSwapped;
                }
                Err(e) => {
                    self.interrupt_common_inventory_swap(format!(
                        "inventory completion gate: {e:?}"
                    ))
                    .await
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn seed(bot: &Bot) {
        super::super::common_motion::tests::seed_motion(bot).await;
        let mut slots = vec![0, 0, 46];
        for slot in 0..46 {
            let stack = match slot {
                9 => Some(ItemStack {
                    item_id: crate::item_id("stone").unwrap(),
                    count: 3,
                    nbt: None,
                }),
                36 => Some(ItemStack {
                    item_id: crate::item_id("dirt").unwrap(),
                    count: 2,
                    nbt: None,
                }),
                _ => None,
            };
            write_slot(&mut slots, stack.as_ref());
        }
        bot.apply_packet(0x14, slots).await.unwrap();
        bot.apply_packet(0x16, vec![255, 255, 255, 0])
            .await
            .unwrap();
    }
    async fn slot(bot: &Bot, index: i16, value: &api::SlotKnowledge, raw: bool) {
        let mut payload = vec![if raw { 254 } else { 0 }];
        payload.extend(index.to_be_bytes());
        let stack = match value {
            api::SlotKnowledge::Empty => None,
            api::SlotKnowledge::Item { item } => Some(ItemStack {
                item_id: item.id.value(),
                count: item.count as i8,
                nbt: None,
            }),
            _ => panic!("fixture slot"),
        };
        write_slot(&mut payload, stack.as_ref());
        bot.apply_packet(0x16, payload).await.unwrap();
    }
    async fn ack(bot: &Bot, action: i16, accepted: bool) {
        let mut payload = vec![0];
        payload.extend(action.to_be_bytes());
        payload.push(u8::from(accepted));
        bot.apply_packet(0x12, payload).await.unwrap();
    }
    #[tokio::test]
    async fn common_swap_same_consumer_checks_destinations_native_ack_and_next_empty_swap() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let record = crate::client::tests::common_swap_start_scenario(
            &client,
            api::GameMode::Survival,
            9,
            0,
        )
        .await;
        let action = record.send.legacy_action.unwrap();
        let mut expected = vec![0, 0, 9, 0];
        expected.extend(action.to_be_bytes());
        expected.extend([2, 1, 1, 3, 0]);
        assert_eq!(packets.recv().await.unwrap(), (0x09, expected));
        assert!(record.send.screen_revision.is_none());
        ack(&bot, action + 1, true).await;
        slot(&bot, 9, &record.hotbar_before.value, false).await;
        crate::client::tests::common_swap_pending_scenario(&client).await;
        // Raw Inventory -2 hotbar receipt is still screen slot 36, with a real ordinal.
        slot(&bot, 0, &record.main_before.value, true).await;
        crate::client::tests::common_swap_pending_scenario(&client).await;
        ack(&bot, action, true).await;
        let completed =
            crate::client::tests::common_swap_completed_scenario(&client, record.id).await;
        assert!(completed.legacy_reply.unwrap().accepted);
        slot(&bot, 9, &api::SlotKnowledge::Empty, false).await;
        assert_eq!(
            client
                .survival()
                .inventory_swap_record()
                .await
                .unwrap()
                .unwrap()
                .stage,
            InventorySwapStage::ObservedSwapped
        );
        // Switch actual received mode, then exchange with an empty destination in creative.
        let mut mode = vec![3];
        mode.extend(1f32.to_be_bytes());
        bot.apply_packet(0x1e, mode).await.unwrap();
        let next = crate::client::tests::common_swap_start_scenario(
            &client,
            api::GameMode::Creative,
            9,
            0,
        )
        .await;
        assert_ne!(next.id, record.id);
        assert_eq!(next.send.legacy_action, Some(action + 1));
        packets.recv().await.unwrap();
        slot(&bot, 9, &next.hotbar_before.value, false).await;
        slot(&bot, 0, &next.main_before.value, true).await;
        ack(&bot, action + 1, true).await;
        crate::client::tests::common_swap_completed_scenario(&client, next.id).await;
        bot.disconnect().await.unwrap();
        assert_eq!(
            client
                .creative()
                .inventory_swap_record()
                .await
                .unwrap()
                .unwrap()
                .stage,
            InventorySwapStage::ObservedSwapped
        );
        drop(release);
        drop(client);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn common_swap_transient_slot_or_cursor_conflict_cannot_be_restored() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let record = client.survival().swap_hotbar(9, 0).await.unwrap();
        packets.recv().await.unwrap();
        slot(&bot, 9, &record.hotbar_before.value, false).await;
        slot(&bot, 9, &record.main_before.value, false).await;
        slot(&bot, 9, &record.hotbar_before.value, false).await;
        slot(&bot, 0, &record.main_before.value, true).await;
        ack(&bot, record.send.legacy_action.unwrap(), true).await;
        let result = client
            .survival()
            .inventory_swap_record()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.stage, InventorySwapStage::RequiresInspection);
        assert!(
            result
                .requires_inspection
                .unwrap()
                .contains("changed again")
        );
        assert!(client.survival().select_hotbar(0).await.is_err());
        drop(release);
        drop(client);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn common_swap_cancelled_waiter_keeps_one_owned_write_and_closed_diagnostics() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let writer = bot.writer.lock().await;
        let ops = client.survival();
        let task = tokio::spawn(async move { ops.swap_hotbar(9, 0).await });
        timeout(Duration::from_secs(1), async {
            while bot.common_inventory_swap.lock().await.is_none() {
                tokio::task::yield_now().await
            }
        })
        .await
        .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        drop(writer);
        assert_eq!(
            timeout(Duration::from_secs(1), packets.recv())
                .await
                .unwrap()
                .unwrap()
                .0,
            0x09
        );
        timeout(Duration::from_secs(1), async {
            while !bot
                .common_inventory_swap
                .lock()
                .await
                .as_ref()
                .unwrap()
                .record
                .send
                .dispatched
            {
                tokio::task::yield_now().await
            }
        })
        .await
        .unwrap();
        assert!(client.survival().swap_hotbar(9, 0).await.is_err());
        assert!(
            timeout(Duration::from_millis(10), packets.recv())
                .await
                .is_err()
        );
        bot.disconnect().await.unwrap();
        let record = client
            .survival()
            .inventory_swap_record()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.stage, InventorySwapStage::RequiresInspection);
        assert!(record.send.dispatched);
        drop(release);
        drop(client);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn common_swap_negative_comparison_needs_fresh_destinations_and_never_means_rollback() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let record = client.survival().swap_hotbar(9, 0).await.unwrap();
        packets.recv().await.unwrap();
        ack(&bot, record.send.legacy_action.unwrap(), false).await;
        assert_eq!(packets.recv().await.unwrap().0, 0x07);
        crate::client::tests::common_swap_pending_scenario(&client).await;
        slot(&bot, 9, &record.hotbar_before.value, false).await;
        slot(&bot, 0, &record.main_before.value, true).await;
        ack(&bot, record.send.legacy_action.unwrap(), true).await;
        let result = client
            .survival()
            .inventory_swap_record()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.stage, InventorySwapStage::ObservedSwapped);
        assert!(!result.legacy_reply.unwrap().accepted);
        drop(release);
        drop(client);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
}
