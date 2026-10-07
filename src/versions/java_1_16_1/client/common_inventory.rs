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
            if super::lock_packet_state(&self.common_container_close)
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
        let attempt = super::lock_packet_state(&self.common_inventory_swap)
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
        let registries = self
            .common_receipts
            .lock()
            .await
            .registries
            .capture(initial.session, initial.receive_sequence);
        let record = contract::prepare_received_source(
            (initial, registries),
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
        *super::lock_packet_state(&self.common_inventory_swap).await = Some(NativeInventorySwap {
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
            let record = super::lock_packet_state(&self.common_inventory_swap)
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
            if !contract::same_received_value(
                slots
                    .get(usize::from(record.source_slot))
                    .and_then(Option::as_ref),
                &record.source_before,
            ) || !contract::same_received_value(
                slots
                    .get(usize::from(record.hotbar_screen_slot))
                    .and_then(Option::as_ref),
                &record.hotbar_before,
            ) {
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
                let mut guard = super::lock_packet_state(&self.common_inventory_swap).await;
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
                        nbt: match &comparison.data {
                            api::ItemData::Default => None,
                            api::ItemData::LegacyNbt { bytes } => Some(bytes.clone()),
                            _ => {
                                return Err(contract::unavailable(
                                    "legacy swap comparison contains foreign item data",
                                ));
                            }
                        },
                    },
                )
                .await?;
            let mut guard = super::lock_packet_state(&self.common_inventory_swap).await;
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
    pub(super) async fn interrupt_common_inventory_swap(&self, reason: impl std::fmt::Display) {
        if let Some(run) = super::lock_packet_state(&self.common_inventory_swap)
            .await
            .as_mut()
        {
            contract::inspection(&mut run.record, reason);
        }
    }
    pub(super) async fn common_inventory_context_received(&self) -> Result<()> {
        self.reconcile_common_inventory_swap(false).await
    }
    pub(super) async fn common_inventory_reply_received(&self, reply: WindowTransaction) {
        let mut guard = super::lock_packet_state(&self.common_inventory_swap).await;
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
        let Some(snapshot) = super::lock_packet_state(&self.common_inventory_swap)
            .await
            .clone()
        else {
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
            let mut guard = super::lock_packet_state(&self.common_inventory_swap).await;
            let record = &mut guard.as_mut().expect("retained").record;
            let registries = self
                .common_receipts
                .lock()
                .await
                .registries
                .capture(current.session, current.receive_sequence);
            contract::receive_with_registries(record, &current, screen.as_ref(), &registries);
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
                    let mut guard = super::lock_packet_state(&self.common_inventory_swap).await;
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

impl crate::client::adapter::InventorySwapOps for Bot {
    async fn swap_hotbar(
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
    async fn swap_container_hotbar(
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
    async fn inventory_swap_record(&self) -> Result<Option<InventorySwapRecord>> {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn common_item_data_same_consumer_preserves_original_constructor_nbt() {
        let profiles: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../data/client_api/cursor_return_profiles-1.16.1.json"
        ))
        .unwrap();
        let definition = profiles["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["name"] == "minecraft:diamond_helmet")
            .unwrap();
        let bytes = definition["default_legacy_nbt"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b.as_u64().unwrap() as u8)
            .collect::<Vec<_>>();
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        for mode in [api::GameMode::Survival, api::GameMode::Creative] {
            let mut changed = vec![3];
            changed.extend(
                (if mode == api::GameMode::Creative {
                    1_f32
                } else {
                    0_f32
                })
                .to_be_bytes(),
            );
            bot.apply_packet(0x1e, changed).await.unwrap();
            let mut packet = vec![0];
            packet.extend(9_i16.to_be_bytes());
            write_slot(
                &mut packet,
                Some(&ItemStack {
                    item_id: definition["native_id"].as_i64().unwrap() as i32,
                    count: 1,
                    nbt: Some(bytes.clone()),
                }),
            );
            bot.apply_packet(0x16, packet).await.unwrap();
            api::tests::common_item_data_scenario(
                &client,
                9,
                "minecraft:diamond_helmet",
                1,
                api::ItemData::LegacyNbt {
                    bytes: bytes.clone(),
                },
            )
            .await;
            assert!(packets.try_recv().is_err());
        }
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
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
    async fn common_container_close_refuses_missing_cursor_insufficient_capacity_and_pending_swap()
    {
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
        let definition = api::registry::Registry::for_version(crate::MinecraftVersion::Java1_16_1)
            .item("minecraft:stone")
            .unwrap();
        let full = api::SlotKnowledge::Item {
            item: api::ItemStack {
                id: definition.id,
                name: definition.name,
                count: 64,
                data: api::ItemData::Default,
            },
        };
        for index in 27..63 {
            container_slot(&bot, index, &full).await;
        }
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
        seed_container(&bot).await;
        let id = client.screen_state().await.unwrap().screen.unwrap().id;
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
    #[tokio::test]
    async fn ordinary_pickup_both_modes_player_storage_and_appended_player_require_cursor_and_reply()
     {
        use contract::{
            InventoryClickButton as Button, InventoryClickSource as Source,
            InventoryClickStage as Stage,
        };
        for mode in [api::GameMode::Survival, api::GameMode::Creative] {
            for storage_slot in [None, Some(0), Some(54)] {
                let (bot, mut packets, release, server) =
                    super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
                if storage_slot.is_some() {
                    seed_container(&bot).await;
                } else {
                    seed(&bot).await;
                }
                if mode == api::GameMode::Creative {
                    let mut p = vec![3];
                    p.extend(1f32.to_be_bytes());
                    bot.apply_packet(0x1e, p).await.unwrap();
                }
                let client = crate::Client::from_java_1_16_1(bot.clone());
                let source = if storage_slot.is_some() {
                    Source::Container {
                        screen: client.screen_state().await.unwrap().screen.unwrap().id,
                    }
                } else {
                    Source::Player
                };
                let index = storage_slot.unwrap_or(9);
                let first = if storage_slot == Some(54) {
                    Button::Left
                } else {
                    Button::Right
                };
                for button in [first, Button::Right, Button::Left] {
                    let record = api::tests::common_pickup_start_scenario(
                        &client, mode, source, index, button,
                    )
                    .await;
                    let packet = loop {
                        let p = packets.recv().await.unwrap();
                        if p.0 == 0x09 {
                            break p;
                        }
                    };
                    assert_eq!(packet.1[0], record.window_id() as u8);
                    assert_eq!(packet.1[3], button.native());
                    assert_eq!(packet.1[6], 0);
                    if let Some(index) = storage_slot {
                        container_slot(&bot, index as i16, &record.prediction.source.value).await;
                    } else {
                        slot(&bot, 9, &record.prediction.source.value, false).await;
                    }
                    assert_eq!(
                        client
                            .survival()
                            .inventory_click_record()
                            .await
                            .unwrap()
                            .unwrap()
                            .stage,
                        Stage::Pending
                    );
                    let mut p = vec![255, 255, 255];
                    let native = match &record.prediction.cursor.value {
                        api::SlotKnowledge::Empty => None,
                        api::SlotKnowledge::Item { item } => Some(ItemStack {
                            item_id: item.id.value(),
                            count: item.count as i8,
                            nbt: None,
                        }),
                        _ => unreachable!(),
                    };
                    write_slot(&mut p, native.as_ref());
                    bot.apply_packet(0x16, p).await.unwrap();
                    assert_eq!(
                        client
                            .survival()
                            .inventory_click_record()
                            .await
                            .unwrap()
                            .unwrap()
                            .stage,
                        Stage::Pending
                    );
                    let mut p = vec![record.window_id() as u8];
                    p.extend(record.send.legacy_action.unwrap().to_be_bytes());
                    p.push(0);
                    bot.apply_packet(0x12, p).await.unwrap();
                    let complete =
                        api::tests::common_pickup_complete_scenario(&client, record.id).await;
                    assert!(!complete.legacy_reply.unwrap().accepted);
                }
                assert!(matches!(
                    client
                        .player_state()
                        .await
                        .unwrap()
                        .inventory
                        .cursor
                        .unwrap()
                        .value,
                    api::SlotKnowledge::Empty
                ));
                bot.disconnect().await.unwrap();
                assert_eq!(
                    client
                        .survival()
                        .inventory_click_record()
                        .await
                        .unwrap()
                        .unwrap()
                        .stage,
                    Stage::ObservedClicked
                );
                drop(release);
                drop(client);
                drop(bot);
                server.await.unwrap();
            }
        }
    }
    #[tokio::test]
    async fn ordinary_pickup_cancelled_caller_keeps_owned_write_and_record_is_prompt_behind_writer()
    {
        use contract::{
            InventoryClickButton as Button, InventoryClickSource as Source,
            InventoryClickStage as Stage,
        };
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let writer = bot.writer.lock().await;
        let ops = client.survival();
        let waiter =
            tokio::spawn(async move { ops.click_inventory(Source::Player, 9, Button::Left).await });
        timeout(Duration::from_secs(1), async {
            loop {
                if bot
                    .common_inventory_click
                    .lock()
                    .await
                    .as_ref()
                    .is_some_and(|s| s.record.send.legacy_action.is_some())
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let intent = timeout(
            Duration::from_millis(100),
            client.survival().inventory_click_record(),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        assert!(!intent.send.dispatched);
        assert_eq!(intent.stage, Stage::Pending);
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
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
                .common_inventory_click
                .lock()
                .await
                .as_ref()
                .unwrap()
                .record
                .send
                .dispatched
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            client
                .survival()
                .click_inventory(Source::Player, 9, Button::Left)
                .await
                .is_err()
        );
        assert!(
            timeout(Duration::from_millis(20), packets.recv())
                .await
                .is_err()
        );
        bot.disconnect().await.unwrap();
        let retained = client
            .survival()
            .inventory_click_record()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retained.id, intent.id);
        assert!(retained.send.dispatched);
        assert_eq!(retained.stage, Stage::RequiresInspection);
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn shift_transfer_cancelled_caller_keeps_owned_write_and_record_is_prompt_behind_writer()
    {
        use contract::{InventorySource as Source, InventoryTransferStage as Stage};
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let writer = bot.writer.lock().await;
        let ops = client.survival();
        let waiter = tokio::spawn(async move { ops.transfer_inventory(Source::Player, 9).await });
        timeout(Duration::from_secs(1), async {
            loop {
                if bot
                    .common_inventory_transfer
                    .lock()
                    .await
                    .as_ref()
                    .is_some_and(|s| s.record.send.legacy_action.is_some())
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let intent = timeout(
            Duration::from_millis(100),
            client.survival().inventory_transfer_record(),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        assert!(!intent.send.dispatched);
        assert_eq!(intent.stage, Stage::Pending);
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
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
                .common_inventory_transfer
                .lock()
                .await
                .as_ref()
                .unwrap()
                .record
                .send
                .dispatched
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            client
                .survival()
                .transfer_inventory(Source::Player, 9)
                .await
                .is_err()
        );
        assert!(
            timeout(Duration::from_millis(20), packets.recv())
                .await
                .is_err()
        );
        bot.disconnect().await.unwrap();
        let retained = client
            .survival()
            .inventory_transfer_record()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retained.id, intent.id);
        assert!(retained.send.dispatched);
        assert_eq!(retained.stage, Stage::RequiresInspection);
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    async fn transfer_value(bot: &Bot, window: u8, index: i16, value: &api::SlotKnowledge) {
        let mut p = vec![window];
        p.extend(index.to_be_bytes());
        let native = match value {
            api::SlotKnowledge::Empty => None,
            api::SlotKnowledge::Item { item } => Some(ItemStack {
                item_id: item.id.value(),
                count: item.count as i8,
                nbt: match &item.data {
                    api::ItemData::Default => None,
                    api::ItemData::LegacyNbt { bytes } => Some(bytes.clone()),
                    api::ItemData::ModernComponents { .. } => {
                        unreachable!("legacy received stack cannot contain modern components")
                    }
                },
            }),
            _ => unreachable!(),
        };
        write_slot(&mut p, native.as_ref());
        bot.apply_packet(0x16, p).await.unwrap();
    }
    #[tokio::test]
    async fn shift_transfer_same_consumer_both_modes_storage_player_equipment_and_native_reply() {
        use contract::{InventorySource as Source, InventoryTransferStage as Stage};
        for mode in [api::GameMode::Survival, api::GameMode::Creative] {
            for scenario in [0, 1, 2, 3] {
                let (bot, mut packets, release, server) =
                    super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
                if scenario == 0 {
                    seed_container(&bot).await;
                } else {
                    seed(&bot).await;
                }
                if scenario >= 2 {
                    let name = if scenario == 2 {
                        "minecraft:diamond_helmet"
                    } else {
                        "minecraft:carved_pumpkin"
                    };
                    let profile: serde_json::Value = serde_json::from_str(include_str!(
                        "../../../../data/client_api/inventory_transfer_profiles-1.16.1.json"
                    ))
                    .unwrap();
                    let nbt = &profile["routes"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|r| r["name"] == name)
                        .unwrap()["default_legacy_nbt"];
                    let data = if nbt == &serde_json::json!([0]) {
                        api::ItemData::Default
                    } else {
                        api::ItemData::LegacyNbt {
                            bytes: nbt
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|v| v.as_u64().unwrap() as u8)
                                .collect(),
                        }
                    };
                    let definition =
                        api::registry::Registry::for_version(crate::MinecraftVersion::Java1_16_1)
                            .item(name)
                            .unwrap();
                    transfer_value(
                        &bot,
                        0,
                        9,
                        &api::SlotKnowledge::Item {
                            item: api::ItemStack {
                                id: definition.id,
                                name: definition.name,
                                count: if scenario == 2 { 1 } else { 7 },
                                data,
                            },
                        },
                    )
                    .await;
                }
                if mode == api::GameMode::Creative {
                    let mut p = vec![3];
                    p.extend(1f32.to_be_bytes());
                    bot.apply_packet(0x1e, p).await.unwrap();
                }
                let client = crate::Client::from_java_1_16_1(bot.clone());
                let source = if scenario == 0 {
                    Source::Container {
                        screen: client.screen_state().await.unwrap().screen.unwrap().id,
                    }
                } else {
                    Source::Player
                };
                let record = api::tests::common_transfer_start_scenario(
                    &client,
                    mode,
                    source,
                    if scenario == 0 { 0 } else { 9 },
                )
                .await;
                let packet = loop {
                    let p = packets.recv().await.unwrap();
                    if p.0 == 0x09 {
                        break p;
                    }
                };
                assert_eq!(packet.1[6], 1);
                if scenario == 0 {
                    assert!(
                        record
                            .changed_slots
                            .iter()
                            .any(|s| s.slot == 62 && s.player_slot == Some(44))
                    );
                }
                if scenario >= 2 {
                    assert!(record.changed_slots.iter().any(|s| s.slot == 5));
                }
                if scenario == 3 {
                    assert_eq!(record.changed_slots.len(), 3);
                    assert!(matches!(
                        record.legacy_return_prediction.as_ref().unwrap().value,
                        api::SlotKnowledge::Item { .. }
                    ));
                    assert_eq!(
                        record.send.legacy_comparison.as_ref(),
                        Some(&api::SlotKnowledge::Empty)
                    );
                }
                for change in &record.changed_slots {
                    transfer_value(
                        &bot,
                        record.window_id() as u8,
                        change.slot as i16,
                        &change.prediction.value,
                    )
                    .await;
                    assert_eq!(
                        client
                            .survival()
                            .inventory_transfer_record()
                            .await
                            .unwrap()
                            .unwrap()
                            .stage,
                        Stage::Pending
                    );
                }
                // Cursor's original actual ordinal is a valid unchanged baseline.
                let before_cursor = record.cursor_before.source;
                let mut ack = vec![record.window_id() as u8];
                ack.extend(record.send.legacy_action.unwrap().to_be_bytes());
                ack.push(0);
                bot.apply_packet(0x12, ack).await.unwrap();
                let complete =
                    api::tests::common_transfer_complete_scenario(&client, record.id).await;
                assert_eq!(complete.cursor_inspected.unwrap().source, before_cursor);
                assert!(!complete.legacy_reply.unwrap().accepted);
                bot.disconnect().await.unwrap();
                assert_eq!(
                    client
                        .creative()
                        .inventory_transfer_record()
                        .await
                        .unwrap()
                        .unwrap()
                        .stage,
                    Stage::ObservedTransferred
                );
                drop(release);
                drop(client);
                drop(bot);
                server.await.unwrap();
            }
        }
    }
    #[tokio::test]
    async fn cursor_return_close_same_consumer_both_modes_waits_for_each_actual_step_and_reply() {
        for mode in [api::GameMode::Survival, api::GameMode::Creative] {
            let (bot, mut packets, release, server) =
                super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
            seed_container(&bot).await;
            let client = crate::Client::from_java_1_16_1(bot.clone());
            let screen = client.screen_state().await.unwrap().screen.unwrap().id;
            let definition =
                api::registry::Registry::for_version(crate::MinecraftVersion::Java1_16_1)
                    .item("minecraft:stone")
                    .unwrap();
            let item = |count| api::SlotKnowledge::Item {
                item: api::ItemStack {
                    id: definition.id,
                    name: definition.name.clone(),
                    count,
                    data: api::ItemData::Default,
                },
            };
            let mut health = Vec::from(20f32.to_be_bytes());
            put_varint(&mut health, 20);
            health.extend(5f32.to_be_bytes());
            bot.apply_packet(0x49, health).await.unwrap();
            container_slot(&bot, 27, &item(63)).await;
            let mut cursor = vec![255, 255, 255];
            write_slot(
                &mut cursor,
                Some(&ItemStack {
                    item_id: definition.id.value(),
                    count: 5,
                    nbt: None,
                }),
            );
            bot.apply_packet(0x16, cursor).await.unwrap();
            if mode == api::GameMode::Creative {
                let mut p = vec![3];
                p.extend(1f32.to_be_bytes());
                bot.apply_packet(0x1e, p).await.unwrap();
            }
            let waiter = api::tests::common_cursor_close_start(&client, mode, screen);
            for index in 0..2 {
                let (id, payload) = timeout(Duration::from_secs(1), packets.recv())
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(id, 0x09);
                assert_eq!(payload[0], 3);
                let record = api::tests::common_cursor_close_retained(&client, index + 1).await;
                let step = &record.return_steps[index];
                assert_eq!(step.source_slot, 27 + index as u16);
                assert_eq!(
                    i16::from_be_bytes([payload[1], payload[2]]),
                    step.source_slot as i16
                );
                assert!(client.survival().swap_hotbar(9, 0).await.is_err());
                assert!(
                    timeout(Duration::from_millis(20), packets.recv())
                        .await
                        .is_err()
                );
                container_slot(
                    &bot,
                    step.source_slot as i16,
                    &item(if index == 0 { 64 } else { 4 }),
                )
                .await;
                let mut cursor = vec![255, 255, 255];
                let held = (index == 0).then_some(ItemStack {
                    item_id: definition.id.value(),
                    count: 4,
                    nbt: None,
                });
                write_slot(&mut cursor, held.as_ref());
                bot.apply_packet(0x16, cursor).await.unwrap();
                assert!(
                    timeout(Duration::from_millis(20), packets.recv())
                        .await
                        .is_err()
                );
                let mut reply = vec![3];
                reply.extend(step.send.legacy_action.unwrap().to_be_bytes());
                reply.push(0);
                bot.apply_packet(0x12, reply).await.unwrap();
                assert_eq!(packets.recv().await.unwrap().0, 0x07);
            }
            assert_eq!(
                timeout(Duration::from_secs(1), packets.recv())
                    .await
                    .unwrap()
                    .unwrap(),
                (0x0a, vec![3])
            );
            let complete = waiter.await.unwrap().unwrap();
            assert!(complete.dispatched);
            assert!(
                complete
                    .return_steps
                    .iter()
                    .all(|s| s.stage == contract::InventoryClickStage::ObservedClicked)
            );
            assert!(
                complete
                    .return_steps
                    .iter()
                    .all(|s| s.legacy_reply.as_ref().is_some_and(|r| !r.accepted))
            );
            assert!(complete.server_close_sequence.is_none());
            assert!(
                timeout(Duration::from_millis(20), packets.recv())
                    .await
                    .is_err()
            );
            drop(release);
            drop(client);
            drop(bot);
            server.await.unwrap();
        }
    }
    #[test]
    fn all_default_cursor_return_comparisons_preserve_original_native_nbt_codecs() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../data/client_api/cursor_return_packets-1.16.1.json"
        ))
        .unwrap();
        assert_eq!(cases.as_array().unwrap().len(), 1948);
        for case in cases.as_array().unwrap() {
            let slot = case["slot"].as_u64().unwrap() as i16;
            let bytes: Vec<u8> = serde_json::from_value(case["cursor"]["nbt"].clone()).unwrap();
            let item = ItemStack {
                item_id: api::registry::Registry::for_version(crate::MinecraftVersion::Java1_16_1)
                    .item(case["item"].as_str().unwrap())
                    .unwrap()
                    .id
                    .value(),
                count: 1,
                nbt: (bytes != [0]).then_some(bytes),
            };
            assert!(contract::transfer_policy::legacy_comparison_supported(
                &item
            ));
            let mut payload = vec![3];
            payload.extend(slot.to_be_bytes());
            payload.push(0);
            payload.extend(slot.to_be_bytes());
            payload.push(0);
            write_slot(&mut payload, Some(&item));
            assert_eq!(hex::encode(payload), case["payload_hex"].as_str().unwrap());
        }
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
                nbt: match &item.data {
                    api::ItemData::Default => None,
                    api::ItemData::LegacyNbt { bytes } => Some(bytes.clone()),
                    _ => panic!("foreign legacy fixture data"),
                },
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
    #[tokio::test]
    async fn data_swap_same_consumer_preserves_nbt_in_both_modes_and_legacy_comparison_payload() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let registry = api::registry::Registry::for_version(crate::MinecraftVersion::Java1_16_1);
        let definition = registry.item("minecraft:stone").unwrap();
        // Original named compound with one int field, preserved by ordinary codecs.
        let nbt = hex::decode("0a0000030001610000000700").unwrap();
        let value = api::SlotKnowledge::Item {
            item: api::ItemStack {
                id: definition.id,
                name: definition.name,
                count: 3,
                data: api::ItemData::LegacyNbt { bytes: nbt.clone() },
            },
        };
        for mode in [api::GameMode::Survival, api::GameMode::Creative] {
            let mut packet = vec![3];
            packet.extend(
                (if mode == api::GameMode::Creative {
                    1f32
                } else {
                    0f32
                })
                .to_be_bytes(),
            );
            bot.apply_packet(0x1e, packet).await.unwrap();
            slot(&bot, 9, &value, false).await;
            let before = client.received_inventory().await.unwrap();
            let record = api::tests::common_swap_start_scenario(&client, mode, 9, 0).await;
            let (id, payload) = packets.recv().await.unwrap();
            assert_eq!(id, 0x09);
            assert!(payload.ends_with(&nbt));
            assert_eq!(
                record.send.legacy_comparison.as_ref().unwrap().data,
                api::ItemData::LegacyNbt { bytes: nbt.clone() }
            );
            slot(&bot, 9, &record.hotbar_before.value, false).await;
            api::tests::common_swap_pending_scenario(&client).await;
            slot(&bot, 36, &record.source_before.value, false).await;
            api::tests::common_swap_pending_scenario(&client).await;
            ack(&bot, record.send.legacy_action.unwrap(), false).await;
            assert_eq!(packets.recv().await.unwrap().0, 0x07);
            api::tests::common_data_swap_completed_scenario(&client, record.id, &before).await;
            // Give the next iteration a distinct hotbar destination.
            let dirt = registry.item("minecraft:dirt").unwrap();
            slot(
                &bot,
                36,
                &api::SlotKnowledge::Item {
                    item: api::ItemStack {
                        id: dirt.id,
                        name: dirt.name,
                        count: 2,
                        data: api::ItemData::Default,
                    },
                },
                false,
            )
            .await;
        }
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn data_pickup_same_consumer_both_modes_keeps_nbt_cursor_and_actual_legacy_reply() {
        use api::inventory::{
            InventoryClickButton as Button, InventoryClickSource as Source,
            InventoryClickStage as Stage,
        };
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let registry = api::registry::Registry::for_version(crate::MinecraftVersion::Java1_16_1);
        let item = registry.item("minecraft:stone").unwrap();
        let value = |count| {
            if count == 0 {
                api::SlotKnowledge::Empty
            } else {
                api::SlotKnowledge::Item {
                    item: api::ItemStack {
                        id: item.id,
                        name: item.name.clone(),
                        count,
                        data: api::ItemData::LegacyNbt {
                            bytes: hex::decode("0a0000030001610000000700").unwrap(),
                        },
                    },
                }
            }
        };
        for mode in [api::GameMode::Survival, api::GameMode::Creative] {
            let mut packet = vec![3];
            packet.extend(
                (if mode == api::GameMode::Creative {
                    1f32
                } else {
                    0f32
                })
                .to_be_bytes(),
            );
            bot.apply_packet(0x1e, packet).await.unwrap();
            slot(&bot, 9, &value(7), false).await;
            let original = client.received_inventory().await.unwrap();
            for (button, source_count, cursor_count) in [
                (Button::Right, 3, 4),
                (Button::Right, 4, 3),
                (Button::Left, 7, 0),
            ] {
                let record = api::tests::common_pickup_start_scenario(
                    &client,
                    mode,
                    Source::Player,
                    9,
                    button,
                )
                .await;
                assert!(!record.send.request_full_resync);
                assert_eq!(packets.recv().await.unwrap().0, 0x09);
                slot(&bot, 9, &value(source_count), false).await;
                let mut cursor = vec![255, 255, 255];
                let cursor_stack = if cursor_count == 0 {
                    None
                } else {
                    Some(ItemStack {
                        item_id: item.id.value(),
                        count: cursor_count as i8,
                        nbt: Some(hex::decode("0a0000030001610000000700").unwrap()),
                    })
                };
                write_slot(&mut cursor, cursor_stack.as_ref());
                bot.apply_packet(0x16, cursor).await.unwrap();
                assert_eq!(
                    client
                        .survival()
                        .inventory_click_record()
                        .await
                        .unwrap()
                        .unwrap()
                        .stage,
                    Stage::Pending
                );
                ack(&bot, record.send.legacy_action.unwrap(), false).await;
                assert_eq!(packets.recv().await.unwrap().0, 0x07);
                api::tests::common_data_pickup_complete_scenario(
                    &client,
                    record.id,
                    &original,
                    (source_count, cursor_count),
                )
                .await;
            }
        }
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn crafting_input_same_consumer_modes_requires_source_cursor_and_actual_reply() {
        use api::inventory::{
            InventoryClickButton as Button, InventoryClickStage as Stage, InventorySource as Source,
        };
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let cursor = |count| {
            let mut payload = vec![255, 255, 255];
            write_slot(
                &mut payload,
                Some(&ItemStack {
                    item_id: crate::item_id("oak_planks").unwrap(),
                    count,
                    nbt: None,
                }),
            );
            payload
        };
        let stack = |name: &str, count| {
            let definition =
                api::registry::Registry::for_version(crate::MinecraftVersion::Java1_16_1)
                    .item(&format!("minecraft:{name}"))
                    .unwrap();
            api::SlotKnowledge::Item {
                item: api::ItemStack {
                    id: definition.id,
                    name: definition.name,
                    count,
                    data: api::ItemData::Default,
                },
            }
        };
        for mode in [api::GameMode::Survival, api::GameMode::Creative] {
            let mut change = vec![3];
            change.extend(
                (if mode == api::GameMode::Creative {
                    1f32
                } else {
                    0f32
                })
                .to_be_bytes(),
            );
            bot.apply_packet(0x1e, change).await.unwrap();
            slot(&bot, 1, &api::SlotKnowledge::Empty, false).await;
            bot.apply_packet(0x16, cursor(3)).await.unwrap();
            let record = api::tests::common_crafting_input_start_scenario(&client, mode).await;
            assert_eq!(packets.recv().await.unwrap().0, 0x09);
            // Result display alone is never the input/cursor click outcome.
            slot(&bot, 0, &stack("oak_button", 1), false).await;
            assert_eq!(
                client
                    .survival()
                    .inventory_click_record()
                    .await
                    .unwrap()
                    .unwrap()
                    .stage,
                Stage::Pending
            );
            slot(&bot, 1, &stack("oak_planks", 1), false).await;
            assert_eq!(
                client
                    .survival()
                    .inventory_click_record()
                    .await
                    .unwrap()
                    .unwrap()
                    .stage,
                Stage::Pending
            );
            bot.apply_packet(0x16, cursor(2)).await.unwrap();
            assert_eq!(
                client
                    .survival()
                    .inventory_click_record()
                    .await
                    .unwrap()
                    .unwrap()
                    .stage,
                Stage::Pending
            );
            ack(&bot, record.send.legacy_action.unwrap(), false).await;
            assert_eq!(packets.recv().await.unwrap().0, 0x07);
            api::tests::common_crafting_input_complete_scenario(&client, record.id).await;
            assert!(
                client
                    .survival()
                    .click_inventory(Source::Player, 0, Button::Left)
                    .await
                    .is_err()
            );
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(20), packets.recv())
                    .await
                    .is_err()
            );
        }
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn crafting_take_cancelled_caller_retains_owned_write_and_prompt_history() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed(&bot).await;
        let mut result = vec![0, 0, 0];
        write_slot(
            &mut result,
            Some(&ItemStack {
                item_id: crate::item_id("stick").unwrap(),
                count: 4,
                nbt: None,
            }),
        );
        bot.apply_packet(0x16, result).await.unwrap();
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let grid = client.received_crafting().await.unwrap().unwrap();
        let writer = bot.writer.lock().await;
        let ops = client.survival();
        let waiter = tokio::spawn(async move { ops.take_crafting_result(&grid).await });
        let pending = timeout(Duration::from_secs(1), async {
            loop {
                if let Some(r) = client.survival().crafting_take_record().await.unwrap() {
                    break r;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!pending.send.dispatched);
        assert!(pending.after.is_none());
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        drop(writer);
        assert_eq!(
            timeout(Duration::from_secs(1), packets.recv())
                .await
                .unwrap()
                .unwrap()
                .0,
            0x09
        );
        let sent = timeout(Duration::from_secs(1), async {
            loop {
                let r = client
                    .survival()
                    .crafting_take_record()
                    .await
                    .unwrap()
                    .unwrap();
                if r.send.dispatched {
                    break r;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(sent.id, pending.id);
        assert!(
            client
                .survival()
                .take_crafting_result(&sent.before)
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
                .crafting_take_record()
                .await
                .unwrap()
                .unwrap()
                .stage,
            api::crafting::CraftingTakeStage::RequiresInspection
        );
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn recipe_placement_cancelled_waiter_keeps_one_write_conservation_and_revocation_fence() {
        use api::crafting::RecipePlacementStage;
        for revoke in [false, true] {
            let (bot, mut packets, release, server) =
                super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
            seed(&bot).await;
            bot.apply_packet(0x3f, vec![0]).await.unwrap();
            let plank = |count| api::SlotKnowledge::Item {
                item: api::ItemStack {
                    id: api::registry::Registry::for_version(crate::MinecraftVersion::Java1_16_1)
                        .item("minecraft:oak_planks")
                        .unwrap()
                        .id,
                    name: "minecraft:oak_planks".into(),
                    count,
                    data: api::ItemData::Default,
                },
            };
            slot(&bot, 9, &plank(10), false).await;
            slot(&bot, 36, &api::SlotKnowledge::Empty, false).await;
            {
                let mut receipts = bot.common_receipts.lock().await;
                receipts.registries.finish();
                receipts.recipes = api::tests::recipe_placement_book_fixture(
                    crate::MinecraftVersion::Java1_16_1,
                    bot.protocol_packet_sequence.load(Ordering::Acquire),
                );
            }
            let client = crate::Client::from_java_1_16_1(bot.clone());
            let plan = api::tests::common_recipe_placement_plan(&client).await;
            let old_plan = plan.clone();
            let writer = bot.writer.lock().await;
            let ops = client.survival();
            let waiter = tokio::spawn(async move { ops.place_recipe(&plan).await });
            let pending = timeout(Duration::from_secs(1), async {
                loop {
                    if let Some(record) = client.survival().recipe_placement_record().await.unwrap()
                    {
                        break record;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert!(!pending.send.dispatched && pending.after.is_none());
            waiter.abort();
            assert!(waiter.await.unwrap_err().is_cancelled());
            if revoke {
                let revoked = client.revoke_connection();
                assert_eq!(revoked.connection_id(), pending.id.session().connection_id);
                let retained = timeout(
                    Duration::from_millis(100),
                    client.survival().recipe_placement_record(),
                )
                .await
                .unwrap()
                .unwrap()
                .unwrap();
                assert_eq!(retained.id, pending.id);
                assert_eq!(retained.stage, RecipePlacementStage::RequiresInspection);
                drop(writer);
                assert!(client.survival().place_recipe(&old_plan).await.is_err());
                drop(release);
                drop(client);
                drop(bot);
                server.await.unwrap();
                assert!(packets.try_recv().is_err());
                continue;
            }
            drop(writer);
            assert_eq!(
                timeout(Duration::from_secs(1), packets.recv())
                    .await
                    .unwrap()
                    .unwrap(),
                (0x19, api::crafting::dispatch::payload(&pending).unwrap())
            );
            slot(&bot, 1, &plank(1), false).await;
            slot(&bot, 3, &plank(1), false).await;
            let partial = client
                .survival()
                .recipe_placement_record()
                .await
                .unwrap()
                .unwrap();
            assert!(partial.send.dispatched && partial.after.is_none());
            assert_eq!(partial.stage, RecipePlacementStage::Pending);
            assert!(client.survival().place_recipe(&old_plan).await.is_err());
            assert!(client.survival().select_hotbar(0).await.is_err());
            slot(&bot, 9, &plank(8), false).await;
            let complete = client
                .survival()
                .recipe_placement_record()
                .await
                .unwrap()
                .unwrap();
            assert_eq!(complete.id, pending.id);
            assert_eq!(complete.stage, RecipePlacementStage::ObservedPlaced);
            assert!(complete.after.is_some() && complete.inventory_after.is_some());
            assert!(client.survival().place_recipe(&old_plan).await.is_err());
            client.survival().select_hotbar(0).await.unwrap();
            assert_eq!(packets.recv().await.unwrap().0, 0x24);
            assert!(
                timeout(Duration::from_millis(20), packets.recv())
                    .await
                    .is_err()
            );
            bot.disconnect().await.unwrap();
            assert_eq!(
                client
                    .survival()
                    .recipe_placement_record()
                    .await
                    .unwrap()
                    .unwrap()
                    .stage,
                RecipePlacementStage::ObservedPlaced
            );
            drop(release);
            drop(client);
            drop(bot);
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn recipe_ghost_requires_actual_returns_and_fresh_context_even_without_slot_changes() {
        use api::RecipePlacementStage;
        for with_input in [false, true] {
            let (bot, mut packets, release, server) =
                super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
            seed(&bot).await;
            bot.apply_packet(0x3f, vec![0]).await.unwrap();
            let item = |name: &str, count| {
                let definition =
                    api::registry::Registry::for_version(crate::MinecraftVersion::Java1_16_1)
                        .item(name)
                        .unwrap();
                api::SlotKnowledge::Item {
                    item: api::ItemStack {
                        id: definition.id,
                        name: definition.name,
                        count,
                        data: api::ItemData::Default,
                    },
                }
            };
            slot(&bot, 9, &item("minecraft:oak_planks", 1), false).await;
            slot(&bot, 36, &api::SlotKnowledge::Empty, false).await;
            if with_input {
                slot(&bot, 1, &item("minecraft:dirt", 1), false).await;
            }
            {
                let mut receipts = bot.common_receipts.lock().await;
                receipts.registries.finish();
                receipts.recipes = api::tests::recipe_placement_book_fixture(
                    crate::MinecraftVersion::Java1_16_1,
                    bot.protocol_packet_sequence.load(Ordering::Acquire),
                );
            }
            let ghost = |window| {
                let mut packet = vec![window];
                put_string(&mut packet, "minecraft:stick");
                packet
            };
            bot.apply_packet(0x30, ghost(0)).await.unwrap();
            let client = crate::Client::from_java_1_16_1(bot.clone());
            let prior = client.received_recipe_ghost().await.unwrap().unwrap();
            assert!(prior.recipe().is_some());
            assert_eq!(prior.recipe_name(), Some("minecraft:stick"));
            let plan = api::tests::common_ghost_recipe_plan(&client).await;
            let request = client.survival().place_recipe(&plan).await.unwrap();
            assert_eq!(packets.recv().await.unwrap().0, 0x19);
            assert_eq!(
                client
                    .survival()
                    .recipe_placement_record()
                    .await
                    .unwrap()
                    .unwrap()
                    .stage,
                RecipePlacementStage::Pending
            );
            bot.apply_packet(0x30, ghost(3)).await.unwrap();
            assert!(client.received_recipe_ghost().await.unwrap().is_none());
            assert_eq!(
                client
                    .survival()
                    .recipe_placement_record()
                    .await
                    .unwrap()
                    .unwrap()
                    .stage,
                RecipePlacementStage::Pending
            );
            bot.apply_packet(0x30, ghost(0)).await.unwrap();
            if with_input {
                let pending = client
                    .survival()
                    .recipe_placement_record()
                    .await
                    .unwrap()
                    .unwrap();
                assert!(pending.ghost.is_some() && pending.after.is_none());
                assert!(client.survival().select_hotbar(0).await.is_err());
                slot(&bot, 1, &api::SlotKnowledge::Empty, false).await;
                assert_eq!(
                    client
                        .survival()
                        .recipe_placement_record()
                        .await
                        .unwrap()
                        .unwrap()
                        .stage,
                    RecipePlacementStage::Pending
                );
                slot(&bot, 36, &item("minecraft:dirt", 1), false).await;
            }
            api::tests::common_ghost_completed(&client, request.id).await;
            assert!(client.survival().place_recipe(&plan).await.is_err());
            let fresh = api::tests::common_ghost_recipe_plan(&client).await;
            let second = client.survival().place_recipe(&fresh).await.unwrap();
            assert_eq!(packets.recv().await.unwrap().0, 0x19);
            bot.apply_packet(0x30, ghost(0)).await.unwrap();
            api::tests::common_ghost_completed(&client, second.id).await;
            assert!(client.survival().place_recipe(&fresh).await.is_err());
            client.survival().select_hotbar(0).await.unwrap();
            assert_eq!(packets.recv().await.unwrap().0, 0x24);
            assert!(
                timeout(Duration::from_millis(20), packets.recv())
                    .await
                    .is_err()
            );
            bot.disconnect().await.unwrap();
            assert_eq!(
                client
                    .survival()
                    .recipe_placement_record()
                    .await
                    .unwrap()
                    .unwrap()
                    .stage,
                RecipePlacementStage::ObservedGhost
            );
            drop(release);
            drop(client);
            drop(bot);
            server.await.unwrap();
        }
    }
    #[tokio::test]
    async fn crafting_take_merge_cancelled_waiter_preserves_one_write_combined_cursor_and_full_grid()
     {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed(&bot).await;
        let item = |name: &str, count| api::SlotKnowledge::Item {
            item: {
                let def = api::registry::Registry::for_version(crate::MinecraftVersion::Java1_16_1)
                    .item(name)
                    .unwrap();
                api::ItemStack {
                    id: def.id,
                    name: def.name,
                    count,
                    data: api::ItemData::Default,
                }
            },
        };
        for index in 0..=4 {
            slot(
                &bot,
                index,
                &match index {
                    0 => item("minecraft:stick", 4),
                    1 | 3 => item("minecraft:oak_planks", 2),
                    _ => api::SlotKnowledge::Empty,
                },
                false,
            )
            .await;
        }
        let cursor = |count| {
            let mut p = vec![255, 255, 255];
            write_slot(
                &mut p,
                Some(&ItemStack {
                    item_id: crate::item_id("stick").unwrap(),
                    count,
                    nbt: None,
                }),
            );
            p
        };
        bot.apply_packet(0x16, cursor(4)).await.unwrap();
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let grid = client.received_crafting().await.unwrap().unwrap();
        let writer = bot.writer.lock().await;
        let old = grid.clone();
        let ops = client.survival();
        let waiter = tokio::spawn(async move { ops.take_crafting_result(&old).await });
        let pending = timeout(Duration::from_secs(1), async {
            loop {
                if let Some(r) = client.survival().crafting_take_record().await.unwrap() {
                    break r;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            matches!(&pending.cursor_before.value,api::SlotKnowledge::Item{item} if item.count==4)
        );
        assert!(
            matches!(&pending.cursor_prediction.value,api::SlotKnowledge::Item{item} if item.count==8)
        );
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        drop(writer);
        assert_eq!(packets.recv().await.unwrap().0, 0x09);
        let sent = timeout(Duration::from_secs(1), async {
            loop {
                let r = client
                    .survival()
                    .crafting_take_record()
                    .await
                    .unwrap()
                    .unwrap();
                if r.send.dispatched {
                    break r;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        bot.apply_packet(0x16, cursor(8)).await.unwrap();
        let half = client
            .survival()
            .crafting_take_record()
            .await
            .unwrap()
            .unwrap();
        assert!(half.cursor_receipt.is_some() && half.after.is_none());
        assert!(client.survival().select_hotbar(0).await.is_err());
        let mut reply = vec![0];
        reply.extend(sent.send.legacy_action.unwrap().to_be_bytes());
        reply.push(0);
        bot.apply_packet(0x12, reply).await.unwrap();
        assert_eq!(packets.recv().await.unwrap().0, 0x07);
        let mut full = vec![0, 0, 46];
        for index in 0..46 {
            let stack = match index {
                0 => Some(ItemStack {
                    item_id: crate::item_id("stick").unwrap(),
                    count: 4,
                    nbt: None,
                }),
                1 | 3 => Some(ItemStack {
                    item_id: crate::item_id("oak_planks").unwrap(),
                    count: 1,
                    nbt: None,
                }),
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
            write_slot(&mut full, stack.as_ref());
        }
        bot.apply_packet(0x14, full).await.unwrap();
        let complete = client
            .survival()
            .crafting_take_record()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(complete.id, pending.id);
        assert_eq!(complete.stage, api::CraftingTakeStage::ObservedTaken);
        assert!(client.survival().take_crafting_result(&grid).await.is_err());
        client.survival().select_hotbar(0).await.unwrap();
        assert_eq!(packets.recv().await.unwrap().0, 0x24);
        assert!(
            timeout(Duration::from_millis(20), packets.recv())
                .await
                .is_err()
        );
        bot.disconnect().await.unwrap();
        assert_eq!(
            client
                .survival()
                .crafting_take_record()
                .await
                .unwrap()
                .unwrap()
                .stage,
            api::CraftingTakeStage::ObservedTaken
        );
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn crafting_result_shift_cancelled_waiter_keeps_one_native_move_and_actual_inventory_gain()
     {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed(&bot).await;
        let item = |name: &str, count| api::SlotKnowledge::Item {
            item: {
                let def = api::registry::Registry::for_version(crate::MinecraftVersion::Java1_16_1)
                    .item(name)
                    .unwrap();
                api::ItemStack {
                    id: def.id,
                    name: def.name,
                    count,
                    data: api::ItemData::Default,
                }
            },
        };
        for index in 0..=4 {
            slot(
                &bot,
                index,
                &match index {
                    0 => item("minecraft:stick", 4),
                    1 | 3 => item("minecraft:oak_planks", 2),
                    _ => api::SlotKnowledge::Empty,
                },
                false,
            )
            .await;
        }
        let cursor = |count| {
            let mut p = vec![255, 255, 255];
            write_slot(
                &mut p,
                Some(&ItemStack {
                    item_id: crate::item_id("stick").unwrap(),
                    count,
                    nbt: None,
                }),
            );
            p
        };
        bot.apply_packet(0x16, cursor(4)).await.unwrap();
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let grid = client.received_crafting().await.unwrap().unwrap();
        let writer = bot.writer.lock().await;
        let old = grid.clone();
        let ops = client.survival();
        let waiter = tokio::spawn(async move { ops.transfer_crafting_result(&old).await });
        let pending = timeout(Duration::from_secs(1), async {
            loop {
                if let Some(r) = client.survival().crafting_take_record().await.unwrap() {
                    break r;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            matches!(&pending.cursor_before.value,api::SlotKnowledge::Item{item} if item.count==4)
        );
        assert!(
            matches!(&pending.cursor_prediction.value,api::SlotKnowledge::Item{item} if item.count==4)
        );
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        drop(writer);
        let (packet_id, payload) = packets.recv().await.unwrap();
        assert_eq!(packet_id, 0x09);
        assert_eq!(payload[6], 1);
        let sent = timeout(Duration::from_secs(1), async {
            loop {
                let r = client
                    .survival()
                    .crafting_take_record()
                    .await
                    .unwrap()
                    .unwrap();
                if r.send.dispatched {
                    break r;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        bot.apply_packet(0x16, cursor(4)).await.unwrap();
        let half = client
            .survival()
            .crafting_take_record()
            .await
            .unwrap()
            .unwrap();
        assert!(half.cursor_receipt.is_some() && half.after.is_none());
        assert!(client.survival().select_hotbar(0).await.is_err());
        let mut reply = vec![0];
        reply.extend(sent.send.legacy_action.unwrap().to_be_bytes());
        reply.push(0);
        bot.apply_packet(0x12, reply).await.unwrap();
        assert_eq!(packets.recv().await.unwrap().0, 0x07);
        let mut full = vec![0, 0, 46];
        for index in 0..46 {
            let stack = match index {
                44 => Some(ItemStack {
                    item_id: crate::item_id("stick").unwrap(),
                    count: 8,
                    nbt: None,
                }),
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
            write_slot(&mut full, stack.as_ref());
        }
        bot.apply_packet(0x14, full).await.unwrap();
        let complete = client
            .survival()
            .crafting_take_record()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(complete.inventory_output_increase, Some(8));
        assert!(complete.inventory_after.is_some());
        assert_eq!(complete.id, pending.id);
        assert_eq!(complete.stage, api::CraftingTakeStage::ObservedTransferred);
        assert!(
            client
                .survival()
                .transfer_crafting_result(&grid)
                .await
                .is_err()
        );
        client.survival().select_hotbar(0).await.unwrap();
        assert_eq!(packets.recv().await.unwrap().0, 0x24);
        assert!(
            timeout(Duration::from_millis(20), packets.recv())
                .await
                .is_err()
        );
        bot.disconnect().await.unwrap();
        assert_eq!(
            client
                .survival()
                .crafting_take_record()
                .await
                .unwrap()
                .unwrap()
                .stage,
            api::CraftingTakeStage::ObservedTransferred
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
                nbt: match &item.data {
                    api::ItemData::Default => None,
                    api::ItemData::LegacyNbt { bytes } => Some(bytes.clone()),
                    _ => panic!("foreign fixture data"),
                },
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
    async fn refreshed_predecessors_allow_one_swap_but_never_count_as_results_or_heal_conflicts() {
        for conflict in [false, true] {
            let (bot, mut packets, release, server) =
                super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
            seed(&bot).await;
            let client = crate::Client::from_java_1_16_1(bot.clone());
            let revision = bot.connection.motion_admission_revision().await.unwrap();
            // Pause at the real before-I/O boundary deterministically. Every
            // predecessor below comes from the normal native packet handler.
            let record = contract::prepare_source(
                client.player_state().await.unwrap(),
                api::GameMode::Survival,
                contract::InventorySwapSource::PlayerMain,
                9,
                0,
                1,
                None,
            )
            .unwrap();
            *bot.common_inventory_swap.lock().await = Some(NativeInventorySwap {
                record: record.clone(),
                released: false,
            });
            slot(&bot, 9, &record.source_before.value, false).await;
            slot(&bot, 36, &record.hotbar_before.value, false).await;
            bot.apply_packet(0x16, vec![255, 255, 255, 0])
                .await
                .unwrap();
            let refreshed = client.player_state().await.unwrap();
            assert_ne!(
                refreshed.inventory.slots[9],
                Some(record.source_before.clone())
            );
            assert_ne!(
                refreshed.inventory.slots[36],
                Some(record.hotbar_before.clone())
            );
            if conflict {
                let mut changed = record.source_before.value.clone();
                let api::SlotKnowledge::Item { item } = &mut changed else {
                    unreachable!()
                };
                item.count += 1;
                slot(&bot, 9, &changed, false).await;
                slot(&bot, 9, &record.source_before.value, false).await;
            }
            let send_boundary = bot.protocol_packet_sequence.load(Ordering::Acquire);
            let sent = bot.common_swap_send_owned(record.id, revision).await;
            if conflict {
                assert!(sent.is_err());
                assert!(
                    timeout(Duration::from_millis(20), packets.recv())
                        .await
                        .is_err()
                );
                let retained = client
                    .survival()
                    .inventory_swap_record()
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(retained.stage, InventorySwapStage::RequiresInspection);
                assert!(!retained.send.dispatched);
                assert!(retained.source_receipt.is_none());
                assert!(retained.hotbar_receipt.is_none());
            } else {
                let sent = sent.unwrap();
                assert_eq!(packets.recv().await.unwrap().0, 0x09);
                assert_eq!(sent.source_before, record.source_before);
                assert_eq!(sent.hotbar_before, record.hotbar_before);
                assert_eq!(sent.send.after_sequence, send_boundary);
                assert!(sent.send.after_sequence > sent.initial.receive_sequence);
                assert!(sent.source_receipt.is_none());
                assert!(sent.hotbar_receipt.is_none());
                assert_eq!(sent.stage, InventorySwapStage::Pending);
                slot(&bot, 9, &record.hotbar_before.value, false).await;
                slot(&bot, 36, &record.source_before.value, false).await;
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
                ack(&bot, sent.send.legacy_action.unwrap(), false).await;
                assert_eq!(packets.recv().await.unwrap().0, 0x07);
                api::tests::common_swap_completed_scenario(&client, record.id).await;
                assert!(
                    timeout(Duration::from_millis(20), packets.recv())
                        .await
                        .is_err()
                );
            }
            bot.disconnect().await.unwrap();
            drop(release);
            drop(client);
            drop(bot);
            server.await.unwrap();
        }
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

    #[tokio::test]
    async fn held_cursor_transfer_same_consumer_modes_preserves_normalized_cursor_and_actual_reply()
    {
        use contract::{InventorySource as Source, InventoryTransferStage as Stage};
        for mode in [api::GameMode::Survival, api::GameMode::Creative] {
            let (bot, mut packets, release, server) =
                super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
            seed(&bot).await;
            if mode == api::GameMode::Creative {
                let mut p = vec![3];
                p.extend(1f32.to_be_bytes());
                bot.apply_packet(0x1e, p).await.unwrap();
            }
            let registry =
                api::registry::Registry::for_version(crate::MinecraftVersion::Java1_16_1);
            let definition = registry.item("minecraft:stone").unwrap();
            let source = api::SlotKnowledge::Item {
                item: api::ItemStack {
                    id: definition.id,
                    name: definition.name,
                    count: 7,
                    data: api::ItemData::Default,
                },
            };
            transfer_value(&bot, 0, 9, &source).await;
            transfer_value(&bot, 0, 36, &api::SlotKnowledge::Empty).await;
            transfer_value(
                &bot,
                255,
                -1,
                &api::inventory::held_cursor_tests::data_cursor(
                    crate::MinecraftVersion::Java1_16_1,
                    false,
                ),
            )
            .await;
            let client = crate::Client::from_java_1_16_1(bot.clone());
            let before = client.received_inventory().await.unwrap();
            let record =
                api::tests::common_transfer_start_scenario(&client, mode, Source::Player, 9).await;
            assert_eq!(packets.recv().await.unwrap().0, 0x09);
            transfer_value(
                &bot,
                255,
                -1,
                &api::inventory::held_cursor_tests::data_cursor(
                    crate::MinecraftVersion::Java1_16_1,
                    true,
                ),
            )
            .await;
            transfer_value(&bot, 0, 9, &api::SlotKnowledge::Empty).await;
            assert_eq!(
                client
                    .survival()
                    .inventory_transfer_record()
                    .await
                    .unwrap()
                    .unwrap()
                    .stage,
                Stage::Pending
            );
            transfer_value(&bot, 0, 36, &source).await;
            assert_eq!(
                client
                    .survival()
                    .inventory_transfer_record()
                    .await
                    .unwrap()
                    .unwrap()
                    .stage,
                Stage::Pending
            );
            ack(&bot, record.send.legacy_action.unwrap(), false).await;
            assert_eq!(packets.recv().await.unwrap().0, 0x07);
            api::tests::common_held_cursor_transfer_complete_scenario(&client, record.id, &before)
                .await;
            drop(client);
            let _ = release.send(());
            drop(bot);
            server.await.unwrap();
        }
    }
    #[tokio::test]
    async fn armor_transfer_same_consumer_modes_preserves_data_and_waits_for_native_reply() {
        use contract::{InventorySource as Source, InventoryTransferStage as Stage};
        for mode in [api::GameMode::Survival, api::GameMode::Creative] {
            for binding in [false, true] {
                let (bot, mut packets, release, server) =
                    super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
                seed(&bot).await;
                if mode == api::GameMode::Creative {
                    let mut p = vec![3];
                    p.extend(1f32.to_be_bytes());
                    bot.apply_packet(0x1e, p).await.unwrap();
                }
                let value = api::inventory::armor_tests::fixture(
                    crate::MinecraftVersion::Java1_16_1,
                    binding,
                );
                transfer_value(&bot, 0, 5, &value).await;
                transfer_value(&bot, 0, 9, &api::SlotKnowledge::Empty).await;
                let client = crate::Client::from_java_1_16_1(bot.clone());
                let before = client.received_inventory().await.unwrap();
                if binding && mode == api::GameMode::Survival {
                    let error = client
                        .survival()
                        .transfer_inventory(Source::Player, 5)
                        .await
                        .unwrap_err();
                    assert_eq!(error.kind(), crate::ErrorKind::InvalidInput);
                    assert!(error.to_string().contains("no effect"));
                    assert!(
                        client
                            .survival()
                            .inventory_transfer_record()
                            .await
                            .unwrap()
                            .is_none()
                    );
                    assert!(
                        timeout(Duration::from_millis(15), packets.recv())
                            .await
                            .is_err()
                    );
                } else {
                    let record = api::tests::common_transfer_start_scenario(
                        &client,
                        mode,
                        Source::Player,
                        5,
                    )
                    .await;
                    assert_eq!(packets.recv().await.unwrap().0, 0x09);
                    transfer_value(&bot, 0, 5, &api::SlotKnowledge::Empty).await;
                    transfer_value(&bot, 0, 9, &value).await;
                    assert_eq!(
                        client
                            .survival()
                            .inventory_transfer_record()
                            .await
                            .unwrap()
                            .unwrap()
                            .stage,
                        Stage::Pending
                    );
                    ack(&bot, record.send.legacy_action.unwrap(), false).await;
                    assert_eq!(packets.recv().await.unwrap().0, 0x07);
                    api::tests::common_data_transfer_complete_from_scenario(
                        &client,
                        record.id,
                        &before,
                        5,
                        &[(5, 0), (9, 1)],
                    )
                    .await;
                }
                drop(client);
                let _ = release.send(());
                drop(bot);
                server.await.unwrap();
            }
        }
    }
    #[tokio::test]
    async fn data_transfer_same_consumer_both_modes_partial_merge_and_split_return_preserves_nbt() {
        use contract::{InventorySource as Source, InventoryTransferStage as Stage};
        for mode in [api::GameMode::Survival, api::GameMode::Creative] {
            let (bot, mut packets, release, server) =
                super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
            seed(&bot).await;
            if mode == api::GameMode::Creative {
                let mut p = vec![3];
                p.extend(1f32.to_be_bytes());
                bot.apply_packet(0x1e, p).await.unwrap();
            }
            let stone = api::registry::Registry::for_version(crate::MinecraftVersion::Java1_16_1)
                .item("minecraft:stone")
                .unwrap();
            let value = |count| {
                if count == 0 {
                    api::SlotKnowledge::Empty
                } else {
                    api::SlotKnowledge::Item {
                        item: api::ItemStack {
                            id: stone.id,
                            name: stone.name.clone(),
                            count,
                            data: api::ItemData::LegacyNbt {
                                bytes: hex::decode("0a0000030001610000000700").unwrap(),
                            },
                        },
                    }
                }
            };
            transfer_value(&bot, 0, 9, &value(7)).await;
            transfer_value(&bot, 0, 36, &value(60)).await;
            for i in 37..45 {
                transfer_value(
                    &bot,
                    0,
                    i,
                    &api::legacy_slot(Some(&ItemStack {
                        item_id: 9,
                        count: 64,
                        nbt: None,
                    }))
                    .unwrap(),
                )
                .await;
            }
            let client = crate::Client::from_java_1_16_1(bot.clone());
            let original = client.received_inventory().await.unwrap();
            for (source, updates, expected) in [
                (9, vec![(9, 3), (36, 64)], vec![(9, 3), (36, 64)]),
                (
                    36,
                    vec![(36, 0), (9, 64), (10, 3)],
                    vec![(36, 0), (9, 64), (10, 3)],
                ),
            ] {
                let record = api::tests::common_transfer_start_scenario(
                    &client,
                    mode,
                    Source::Player,
                    source,
                )
                .await;
                let (packet, _) = packets.recv().await.unwrap();
                assert_eq!(packet, 0x09);
                for (slot, count) in updates {
                    transfer_value(&bot, 0, slot, &value(count)).await;
                }
                assert_eq!(
                    client
                        .survival()
                        .inventory_transfer_record()
                        .await
                        .unwrap()
                        .unwrap()
                        .stage,
                    Stage::Pending
                );
                ack(&bot, record.send.legacy_action.unwrap(), false).await;
                let (packet, _) = packets.recv().await.unwrap();
                assert_eq!(packet, 0x07);
                api::tests::common_data_transfer_complete_scenario(
                    &client, record.id, &original, &expected,
                )
                .await;
            }
            drop(client);
            let _ = release.send(());
            drop(bot);
            server.await.unwrap();
        }
    }
    #[tokio::test]
    async fn data_cursor_close_same_consumer_both_modes_waits_for_data_steps_and_actual_replies() {
        for mode in [api::GameMode::Survival, api::GameMode::Creative] {
            let (bot, mut packets, release, server) =
                super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
            seed_container(&bot).await;
            let client = crate::Client::from_java_1_16_1(bot.clone());
            let screen = client.screen_state().await.unwrap().screen.unwrap().id;
            let definition =
                api::registry::Registry::for_version(crate::MinecraftVersion::Java1_16_1)
                    .item("minecraft:stone")
                    .unwrap();
            let item = |count| api::SlotKnowledge::Item {
                item: api::ItemStack {
                    id: definition.id,
                    name: definition.name.clone(),
                    count,
                    data: api::ItemData::LegacyNbt {
                        bytes: hex::decode("0a0000030001610000000700").unwrap(),
                    },
                },
            };
            let mut health = Vec::from(20f32.to_be_bytes());
            put_varint(&mut health, 20);
            health.extend(5f32.to_be_bytes());
            bot.apply_packet(0x49, health).await.unwrap();
            container_slot(&bot, 27, &item(63)).await;
            let mut cursor = vec![255, 255, 255];
            write_slot(
                &mut cursor,
                Some(&ItemStack {
                    item_id: definition.id.value(),
                    count: 5,
                    nbt: Some(hex::decode("0a0000030001610000000700").unwrap()),
                }),
            );
            bot.apply_packet(0x16, cursor).await.unwrap();
            if mode == api::GameMode::Creative {
                let mut p = vec![3];
                p.extend(1f32.to_be_bytes());
                bot.apply_packet(0x1e, p).await.unwrap();
            }
            let before = client.received_inventory().await.unwrap();
            let waiter = api::tests::common_cursor_close_start(&client, mode, screen);
            for index in 0..2 {
                let (id, payload) = timeout(Duration::from_secs(1), packets.recv())
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(id, 0x09);
                assert_eq!(payload[0], 3);
                let record = api::tests::common_cursor_close_retained(&client, index + 1).await;
                let step = &record.return_steps[index];
                assert_eq!(step.source_slot, 27 + index as u16);
                assert_eq!(
                    i16::from_be_bytes([payload[1], payload[2]]),
                    step.source_slot as i16
                );
                assert!(client.survival().swap_hotbar(9, 0).await.is_err());
                assert!(
                    timeout(Duration::from_millis(20), packets.recv())
                        .await
                        .is_err()
                );
                container_slot(
                    &bot,
                    step.source_slot as i16,
                    &item(if index == 0 { 64 } else { 4 }),
                )
                .await;
                let mut cursor = vec![255, 255, 255];
                let held = (index == 0).then_some(ItemStack {
                    item_id: definition.id.value(),
                    count: 4,
                    nbt: Some(hex::decode("0a0000030001610000000700").unwrap()),
                });
                write_slot(&mut cursor, held.as_ref());
                bot.apply_packet(0x16, cursor).await.unwrap();
                assert!(
                    timeout(Duration::from_millis(20), packets.recv())
                        .await
                        .is_err()
                );
                let mut reply = vec![3];
                reply.extend(step.send.legacy_action.unwrap().to_be_bytes());
                reply.push(0);
                bot.apply_packet(0x12, reply).await.unwrap();
                assert_eq!(packets.recv().await.unwrap().0, 0x07);
            }
            assert_eq!(
                timeout(Duration::from_secs(1), packets.recv())
                    .await
                    .unwrap()
                    .unwrap(),
                (0x0a, vec![3])
            );
            let complete = waiter.await.unwrap().unwrap();
            api::tests::common_data_cursor_close_complete_scenario(
                &client,
                &complete,
                &before,
                &[(9, 64), (10, 4)],
            )
            .await;
            assert!(complete.dispatched);
            assert!(
                complete
                    .return_steps
                    .iter()
                    .all(|s| s.stage == contract::InventoryClickStage::ObservedClicked)
            );
            assert!(
                complete
                    .return_steps
                    .iter()
                    .all(|s| s.legacy_reply.as_ref().is_some_and(|r| !r.accepted))
            );
            assert!(complete.server_close_sequence.is_none());
            assert!(
                timeout(Duration::from_millis(20), packets.recv())
                    .await
                    .is_err()
            );
            drop(release);
            drop(client);
            drop(bot);
            server.await.unwrap();
        }
    }
}
