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
        contract::validate_slots(main, hotbar)?;
        self.common_swap_source(
            mode,
            contract::InventorySwapSource::PlayerMain,
            u16::from(main),
            hotbar,
        )
        .await
    }
    pub(crate) async fn common_swap_container_hotbar(
        &self,
        mode: api::GameMode,
        screen: api::container::ScreenId,
        slot: u16,
        hotbar: u8,
    ) -> Result<InventorySwapRecord> {
        self.common_swap_source(
            mode,
            contract::InventorySwapSource::Container { screen },
            slot,
            hotbar,
        )
        .await
    }
    async fn common_swap_source(
        &self,
        mode: api::GameMode,
        source: contract::InventorySwapSource,
        slot: u16,
        hotbar: u8,
    ) -> Result<InventorySwapRecord> {
        let gate = self.coherent_state_gate.lock().await;
        self.common_motion_admission().await?;
        if let contract::InventorySwapSource::Container { screen } = source {
            if self
                .common_container_close
                .lock()
                .await
                .as_ref()
                .is_some_and(|r| r.id.screen() == screen)
            {
                return Err(contract::unavailable(
                    "container has a retained close intent; old opening cannot be clicked",
                ));
            }
        }
        let revision = self
            .connection
            .motion_admission_revision()
            .await
            .map_err(|e| contract::unavailable(format!("inventory admission: {e:?}")))?;
        {
            let inventory = self.inventory.read().await;
            if !inventory.pending_clicks.is_empty()
                || inventory.cursor.is_some()
                || (matches!(source, contract::InventorySwapSource::PlayerMain)
                    && inventory.open_window.is_some())
            {
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
        let initial = self.common_player_unlocked().await?;
        let screen = self
            .common_receipts
            .lock()
            .await
            .container
            .as_ref()
            .map(|s| s.capture(initial.session));
        let record = contract::prepare_source(
            initial,
            mode,
            source,
            slot,
            hotbar,
            attempt,
            if matches!(source, contract::InventorySwapSource::PlayerMain) {
                None
            } else {
                screen
            },
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
            let screen = self
                .common_receipts
                .lock()
                .await
                .container
                .as_ref()
                .map(|s| s.capture(current.session));
            let slots = contract::source_slots(&record, &current, screen.as_ref())?;
            if slots
                .get(usize::from(record.source_slot))
                .and_then(Option::as_ref)
                != Some(&record.source_before)
                || slots
                    .get(usize::from(record.hotbar_screen_slot))
                    .and_then(Option::as_ref)
                    != Some(&record.hotbar_before)
            {
                return Err(contract::unavailable(
                    "inventory capture changed before I/O",
                ));
            }
            let action = self
                .connection
                .begin_window_swap(id.attempt(), revision, record.window_id() as i8)
                .await
                .map_err(|e| contract::unavailable(format!("inventory reservation: {e:?}")))?;
            let comparison = match (&record.source_before.value, &record.hotbar_before.value) {
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
                    record.source_slot,
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
                && i32::from(reply.window_id) == s.record.window_id()
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
        let screen = self
            .common_receipts
            .lock()
            .await
            .container
            .as_ref()
            .map(|s| s.capture(current.session));
        let inventory = self.inventory.read().await;
        let window = inventory
            .open_window
            .as_ref()
            .map_or(0, |s| i32::from(s.id));
        let native_conflict = inventory.cursor.is_some()
            || window != snapshot.record.window_id()
            || !inventory.pending_clicks.is_empty();
        drop(inventory);
        let complete = {
            let mut guard = self.common_inventory_swap.lock().await;
            let record = &mut guard.as_mut().expect("retained").record;
            contract::receive(record, &current, screen.as_ref());
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
    #[test]
    fn pickup_comparison_encoding_matches_original_native_packet_codec() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../data/client_api/regular_click_packets-1.16.1.json"
        ))
        .unwrap();
        assert_eq!(cases.as_array().unwrap().len(), 120);
        for case in cases.as_array().unwrap() {
            let comparison = case["cursor_comparison"]["item"]
                .as_str()
                .map(|name| ItemStack {
                    item_id: crate::item_id(name.strip_prefix("minecraft:").unwrap()).unwrap(),
                    count: case["cursor_comparison"]["count"]
                        .as_i64()
                        .unwrap()
                        .try_into()
                        .unwrap(),
                    nbt: None,
                });
            let mut payload = vec![case["window"].as_u64().unwrap() as u8];
            payload.extend((case["slot"].as_i64().unwrap() as i16).to_be_bytes());
            payload.push(case["button"].as_u64().unwrap() as u8);
            payload.extend((case["revision"].as_i64().unwrap() as i16).to_be_bytes());
            payload.push(0); // original PICKUP mode; action, not a modern revision
            write_slot(&mut payload, comparison.as_ref());
            assert_eq!(hex::encode(payload), case["payload_hex"].as_str().unwrap());
        }
    }
    #[tokio::test]
    async fn player_screen_after_close_respawn_never_reuses_prior_local_basis() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed_container(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let id = client.screen_state().await.unwrap().screen.unwrap().id;
        let close = client.survival().close_container(id).await.unwrap();
        packets.recv().await.unwrap();
        let mut respawn = Vec::new();
        put_string(&mut respawn, "minecraft:overworld");
        put_string(&mut respawn, "world");
        respawn.extend(0i64.to_be_bytes());
        respawn.extend([0, 0, 0, 0, 0]);
        bot.apply_packet(0x3a, respawn).await.unwrap();
        let current = client.player_state().await.unwrap();
        assert_ne!(current.session, close.initial.session);
        assert!(current.inventory.player_screen.is_none());
        assert!(client.survival().swap_hotbar(9, 0).await.is_err());
        assert_eq!(
            client
                .survival()
                .container_close_record()
                .await
                .unwrap()
                .unwrap()
                .id,
            close.id
        );
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn player_screen_after_close_uses_same_consumer_and_two_actual_destinations() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed_container(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let id = client.screen_state().await.unwrap().screen.unwrap().id;
        let close = client.survival().close_container(id).await.unwrap();
        assert_eq!(packets.recv().await.unwrap(), (0x0a, vec![3]));
        api::tests::common_closed_player_screen_scenario(&client, close.id).await;
        let first =
            api::tests::common_swap_start_scenario(&client, api::GameMode::Survival, 9, 0).await;
        let (packet, payload) = packets.recv().await.unwrap();
        assert_eq!(packet, 0x09);
        assert_eq!(payload[0], 0);
        assert!(first.send.screen_revision.is_none());
        slot(&bot, 9, &first.hotbar_before.value, false).await;
        api::tests::common_swap_pending_scenario(&client).await;
        slot(&bot, 36, &first.source_before.value, false).await;
        api::tests::common_swap_pending_scenario(&client).await;
        ack(&bot, first.send.legacy_action.unwrap(), false).await;
        assert_eq!(packets.recv().await.unwrap().0, 0x07);
        api::tests::common_swap_completed_scenario(&client, first.id).await;
        api::tests::common_closed_player_screen_scenario(&client, close.id).await;
        let mut mode = vec![3];
        mode.extend(1f32.to_be_bytes());
        bot.apply_packet(0x1e, mode).await.unwrap();
        let second =
            api::tests::common_swap_start_scenario(&client, api::GameMode::Creative, 9, 0).await;
        assert_eq!(packets.recv().await.unwrap().1[0], 0);
        slot(&bot, 9, &second.hotbar_before.value, false).await;
        slot(&bot, 36, &second.source_before.value, false).await;
        ack(&bot, second.send.legacy_action.unwrap(), false).await;
        packets.recv().await.unwrap();
        api::tests::common_swap_completed_scenario(&client, second.id).await;
        bot.disconnect().await.unwrap();
        assert_eq!(
            client
                .creative()
                .inventory_swap_record()
                .await
                .unwrap()
                .unwrap()
                .id,
            second.id
        );
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn player_screen_after_close_latches_a_new_opening_before_matching_restoration() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed_container(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let id = client.screen_state().await.unwrap().screen.unwrap().id;
        client.survival().close_container(id).await.unwrap();
        packets.recv().await.unwrap();
        let first = client.survival().swap_hotbar(9, 0).await.unwrap();
        packets.recv().await.unwrap();
        // Same numeric ID is a different received opening and revokes the local basis.
        seed_container(&bot).await;
        assert!(
            client
                .player_state()
                .await
                .unwrap()
                .inventory
                .player_screen
                .is_none()
        );
        slot(&bot, 9, &first.hotbar_before.value, false).await;
        slot(&bot, 36, &first.source_before.value, false).await;
        ack(&bot, first.send.legacy_action.unwrap(), true).await;
        let result = client
            .survival()
            .inventory_swap_record()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.stage, InventorySwapStage::RequiresInspection);
        assert!(client.survival().swap_hotbar(9, 0).await.is_err());
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn common_container_close_refuses_received_nonempty_or_missing_cursor_and_pending_swap() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed_container(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let mut open = vec![3, 2];
        put_string(&mut open, "{}");
        bot.apply_packet(0x2e, open).await.unwrap();
        let unknown = client.screen_state().await.unwrap();
        assert!(unknown.cursor.is_none());
        assert!(
            client
                .survival()
                .close_container(unknown.screen.unwrap().id)
                .await
                .is_err()
        );
        assert!(
            client
                .survival()
                .container_close_record()
                .await
                .unwrap()
                .is_none()
        );
        seed_container(&bot).await;
        let id = client.screen_state().await.unwrap().screen.unwrap().id;
        let mut cursor = vec![255, 255, 255];
        write_slot(
            &mut cursor,
            Some(&ItemStack {
                item_id: 1,
                count: 1,
                nbt: None,
            }),
        );
        bot.apply_packet(0x16, cursor).await.unwrap();
        assert!(client.survival().close_container(id).await.is_err());
        assert!(
            client
                .survival()
                .container_close_record()
                .await
                .unwrap()
                .is_none()
        );
        bot.apply_packet(0x16, vec![255, 255, 255, 0])
            .await
            .unwrap();
        client
            .survival()
            .swap_container_hotbar(id, 0, 0)
            .await
            .unwrap();
        assert_eq!(packets.recv().await.unwrap().0, 0x09);
        assert!(client.survival().close_container(id).await.is_err());
        assert!(
            client
                .survival()
                .container_close_record()
                .await
                .unwrap()
                .is_none()
        );
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn common_container_close_retains_dispatch_and_only_matches_original_opening_reply() {
        use api::container::ContainerCloseStage;
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed_container(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let id = client.screen_state().await.unwrap().screen.unwrap().id;
        let first =
            api::tests::common_container_close_scenario(&client, api::GameMode::Survival, id).await;
        assert_eq!(packets.recv().await.unwrap(), (0x0a, vec![3]));
        assert!(bot.open_window_state().await.is_none());
        // Local closure does not fabricate a received close or erase received screen history.
        assert_eq!(client.screen_state().await.unwrap().screen.unwrap().id, id);
        bot.apply_packet(0x13, vec![4]).await.unwrap();
        assert_eq!(
            client
                .survival()
                .container_close_record()
                .await
                .unwrap()
                .unwrap()
                .stage,
            ContainerCloseStage::Dispatched
        );
        bot.apply_packet(0x13, vec![3]).await.unwrap();
        let observed = client
            .survival()
            .container_close_record()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(observed.stage, ContainerCloseStage::ObservedClosed);
        assert!(observed.server_close_sequence.unwrap() > first.initial.receive_sequence);
        seed_container(&bot).await;
        let new = client.screen_state().await.unwrap().screen.unwrap().id;
        assert_ne!(new, id);
        let mut mode = vec![3];
        mode.extend(1f32.to_be_bytes());
        bot.apply_packet(0x1e, mode).await.unwrap();
        let second =
            api::tests::common_container_close_scenario(&client, api::GameMode::Creative, new)
                .await;
        assert_eq!(second.id.attempt(), first.id.attempt() + 1);
        assert_eq!(packets.recv().await.unwrap(), (0x0a, vec![3]));
        seed_container(&bot).await;
        let newest = client.screen_state().await.unwrap().screen.unwrap().id;
        assert_ne!(newest, new);
        bot.apply_packet(0x13, vec![3]).await.unwrap();
        // Same numeric window was reopened; its close cannot acknowledge the former opening.
        assert_eq!(
            client
                .creative()
                .container_close_record()
                .await
                .unwrap()
                .unwrap()
                .stage,
            ContainerCloseStage::Dispatched
        );
        bot.disconnect().await.unwrap();
        assert_eq!(
            client
                .creative()
                .container_close_record()
                .await
                .unwrap()
                .unwrap()
                .id,
            second.id
        );
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn common_container_close_cancelled_legacy_waiter_keeps_one_owned_write() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed_container(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let id = client.screen_state().await.unwrap().screen.unwrap().id;
        let writer = bot.writer.lock().await;
        let ops = client.survival();
        let waiter = tokio::spawn(async move { ops.close_container(id).await });
        timeout(Duration::from_secs(1), async {
            while bot.common_container_close.lock().await.is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        drop(writer);
        assert_eq!(
            timeout(Duration::from_secs(1), packets.recv())
                .await
                .unwrap()
                .unwrap(),
            (0x0a, vec![3])
        );
        let record = client
            .survival()
            .container_close_record()
            .await
            .unwrap()
            .unwrap();
        assert!(record.dispatched);
        assert_eq!(
            record.stage,
            api::container::ContainerCloseStage::Dispatched
        );
        assert!(client.survival().close_container(id).await.is_err());
        assert!(
            timeout(Duration::from_millis(20), packets.recv())
                .await
                .is_err()
        );
        bot.disconnect().await.unwrap();
        assert_eq!(
            client
                .survival()
                .container_close_record()
                .await
                .unwrap()
                .unwrap()
                .stage,
            api::container::ContainerCloseStage::Dispatched
        );
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    async fn seed_container(bot: &Bot) {
        seed(bot).await;
        let mut open = vec![3, 2];
        put_string(&mut open, "{}");
        bot.apply_packet(0x2e, open).await.unwrap();
        let mut full = vec![3, 0, 63];
        for slot in 0..63 {
            let item = match slot {
                0 => Some(ItemStack {
                    item_id: crate::item_id("stone").unwrap(),
                    count: 3,
                    nbt: None,
                }),
                54 => Some(ItemStack {
                    item_id: crate::item_id("dirt").unwrap(),
                    count: 2,
                    nbt: None,
                }),
                _ => None,
            };
            write_slot(&mut full, item.as_ref());
        }
        bot.apply_packet(0x14, full).await.unwrap();
        bot.apply_packet(0x16, vec![255, 255, 255, 0])
            .await
            .unwrap();
    }
    async fn container_slot(bot: &Bot, index: i16, value: &api::SlotKnowledge) {
        let mut payload = vec![3];
        payload.extend(index.to_be_bytes());
        let item = match value {
            api::SlotKnowledge::Empty => None,
            api::SlotKnowledge::Item { item } => Some(ItemStack {
                item_id: item.id.value(),
                count: item.count as i8,
                nbt: None,
            }),
            _ => panic!("value"),
        };
        write_slot(&mut payload, item.as_ref());
        bot.apply_packet(0x16, payload).await.unwrap();
    }
    async fn container_ack(bot: &Bot, record: &InventorySwapRecord) {
        let mut p = vec![3];
        p.extend(record.send.legacy_action.unwrap().to_be_bytes());
        p.push(0);
        bot.apply_packet(0x12, p).await.unwrap();
    }
    #[tokio::test]
    async fn shulker_slot_refusal_is_shared_by_both_modes_before_owner_or_click() {
        for mode in [api::GameMode::Survival, api::GameMode::Creative] {
            let (bot, mut packets, release, server) =
                super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
            seed(&bot).await;
            let mut open = vec![3, 19]; // original native SHULKER_BOX registry ID
            put_string(&mut open, "{}");
            bot.apply_packet(0x2e, open).await.unwrap();
            let mut full = vec![3, 0, 63];
            for index in 0..63 {
                let item = match index {
                    0 => Some(ItemStack {
                        item_id: crate::item_id("stone").unwrap(),
                        count: 3,
                        nbt: None,
                    }),
                    54 => Some(ItemStack {
                        item_id: crate::item_id("white_shulker_box").unwrap(),
                        count: 1,
                        nbt: None,
                    }),
                    _ => None,
                };
                write_slot(&mut full, item.as_ref());
            }
            bot.apply_packet(0x14, full).await.unwrap();
            bot.apply_packet(0x16, vec![255, 255, 255, 0])
                .await
                .unwrap();
            if mode == api::GameMode::Creative {
                let mut p = vec![3];
                p.extend(1f32.to_be_bytes());
                bot.apply_packet(0x1e, p).await.unwrap();
            }
            let client = crate::Client::from_java_1_16_1(bot.clone());
            let screen = client.screen_state().await.unwrap().screen.unwrap().id;
            api::tests::common_refused_shulker_swap_scenario(&client, mode, screen).await;
            while let Ok(packet) = packets.try_recv() {
                assert_ne!(packet.0, 0x09);
            }
            // A valid ordinary incoming stack on the same opening remains usable.
            let dirt = api::registry::Registry::for_version(crate::MinecraftVersion::Java1_16_1)
                .item("minecraft:dirt")
                .unwrap();
            container_slot(
                &bot,
                54,
                &api::SlotKnowledge::Item {
                    item: api::ItemStack {
                        id: dirt.id,
                        name: dirt.name,
                        count: 2,
                        data: api::ItemData::Default,
                    },
                },
            )
            .await;
            let record = match mode {
                api::GameMode::Survival => {
                    client.survival().swap_container_hotbar(screen, 0, 0).await
                }
                api::GameMode::Creative => {
                    client.creative().swap_container_hotbar(screen, 0, 0).await
                }
                _ => unreachable!(),
            }
            .unwrap();
            assert!(record.send.dispatched);
            while packets.recv().await.unwrap().0 != 0x09 {}
            bot.disconnect().await.unwrap();
            drop(release);
            drop(client);
            drop(bot);
            server.await.unwrap();
        }
    }
    #[tokio::test]
    async fn container_swap_same_consumer_receives_exact_slots_and_next_creative_exchange() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed_container(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let screen = client.screen_state().await.unwrap().screen.unwrap().id;
        assert!(
            client
                .survival()
                .swap_container_hotbar(screen, 54, 0)
                .await
                .is_err()
        );
        assert!(
            client
                .survival()
                .swap_container_hotbar(screen, 63, 0)
                .await
                .is_err()
        );
        let record = crate::client::tests::common_container_swap_start_scenario(
            &client,
            api::GameMode::Survival,
            screen,
        )
        .await;
        let packet = packets.recv().await.unwrap();
        assert_eq!(packet.0, 0x09);
        assert_eq!(&packet.1[..7], &[3, 0, 0, 0, 0, 1, 2]);
        container_slot(&bot, 0, &record.hotbar_before.value).await;
        crate::client::tests::common_swap_pending_scenario(&client).await;
        container_slot(&bot, 54, &record.source_before.value).await;
        assert_eq!(
            client
                .survival()
                .inventory_swap_record()
                .await
                .unwrap()
                .unwrap()
                .stage,
            InventorySwapStage::Pending
        );
        container_ack(&bot, &record).await;
        let complete =
            crate::client::tests::common_swap_completed_scenario(&client, record.id).await;
        assert!(!complete.legacy_reply.unwrap().accepted);
        let mut mode = vec![3];
        mode.extend(1f32.to_be_bytes());
        bot.apply_packet(0x1e, mode).await.unwrap();
        let next = crate::client::tests::common_container_swap_start_scenario(
            &client,
            api::GameMode::Creative,
            screen,
        )
        .await;
        while packets.recv().await.unwrap().0 != 0x09 {}
        assert_eq!(next.send.legacy_action, Some(2));
        assert_ne!(next.id, record.id);
        container_slot(&bot, 0, &next.hotbar_before.value).await;
        container_slot(&bot, 54, &next.source_before.value).await;
        container_ack(&bot, &next).await;
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
        server.await.unwrap();
    }
    #[tokio::test]
    async fn container_swap_new_opening_cannot_restore_pending_result() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed_container(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let id = client.screen_state().await.unwrap().screen.unwrap().id;
        let record = client
            .survival()
            .swap_container_hotbar(id, 0, 0)
            .await
            .unwrap();
        packets.recv().await.unwrap();
        seed_container(&bot).await;
        container_slot(&bot, 0, &record.hotbar_before.value).await;
        container_slot(&bot, 54, &record.source_before.value).await;
        container_ack(&bot, &record).await;
        let result = client
            .survival()
            .inventory_swap_record()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.stage, InventorySwapStage::RequiresInspection);
        assert!(
            client
                .survival()
                .swap_container_hotbar(id, 0, 0)
                .await
                .is_err()
        );
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn container_swap_cancelled_waiter_keeps_one_owned_send() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed_container(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let id = client.screen_state().await.unwrap().screen.unwrap().id;
        let writer = bot.writer.lock().await;
        let ops = client.survival();
        let waiter = tokio::spawn(async move { ops.swap_container_hotbar(id, 0, 0).await });
        timeout(Duration::from_secs(1), async {
            while bot.common_inventory_swap.lock().await.is_none() {
                tokio::task::yield_now().await
            }
        })
        .await
        .unwrap();
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        drop(writer);
        let (packet, bytes) = timeout(Duration::from_secs(1), packets.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(packet, 0x09);
        assert_eq!(bytes[0], 3);
        let record = client
            .survival()
            .inventory_swap_record()
            .await
            .unwrap()
            .unwrap();
        assert!(record.send.dispatched);
        assert_eq!(record.stage, InventorySwapStage::Pending);
        assert!(
            client
                .survival()
                .swap_container_hotbar(id, 0, 0)
                .await
                .is_err()
        );
        assert!(
            timeout(Duration::from_millis(20), packets.recv())
                .await
                .is_err()
        );
        bot.disconnect().await.unwrap();
        assert_eq!(
            client
                .survival()
                .inventory_swap_record()
                .await
                .unwrap()
                .unwrap()
                .stage,
            InventorySwapStage::RequiresInspection
        );
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
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
        slot(&bot, 0, &record.source_before.value, true).await;
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
        slot(&bot, 0, &next.source_before.value, true).await;
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
        slot(&bot, 9, &record.source_before.value, false).await;
        slot(&bot, 9, &record.hotbar_before.value, false).await;
        slot(&bot, 0, &record.source_before.value, true).await;
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
        slot(&bot, 0, &record.source_before.value, true).await;
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
