//! Packet-owned inventory execution, independent of strict evidence histories.
use super::*;
use crate::client::{
    self as api,
    inventory::{self as inv, InventorySource},
};
impl Bot {
    pub(super) fn project_inventory(
        metadata: &InventoryState,
        receipts: &api::observation::LegacyReceipts,
    ) -> InventoryState {
        let mut inventory = metadata.clone();
        inventory.windows.clear();
        inventory.pending_clicks.clear();
        if receipts.inventory.slots.iter().any(Option::is_some) {
            inventory.windows.insert(
                0,
                receipts
                    .inventory
                    .slots
                    .iter()
                    .map(|v| {
                        v.as_ref().and_then(|v| match &v.value {
                            api::SlotKnowledge::Unavailable => None,
                            _ => super::common_click::legacy_comparison_stack(&v.value),
                        })
                    })
                    .collect(),
            );
        }
        inventory.cursor = receipts
            .inventory
            .cursor
            .as_ref()
            .and_then(|v| match &v.value {
                api::SlotKnowledge::Unavailable => None,
                _ => super::common_click::legacy_comparison_stack(&v.value),
            });
        let session = api::SessionStamp {
            version: crate::MinecraftVersion::Java1_16_1,
            connection_id: 0,
            world_generation: receipts.generation,
        };
        if let Some(screen) = receipts.screen(session) {
            inventory.open_window = Some(OpenWindow {
                id: screen.id.window_id() as i8,
                window_type: screen.native_menu_id.unwrap_or(-1),
                title_json: match &screen.title.value {
                    api::container::ScreenTitle::LegacyJson { json } => json.clone(),
                    _ => String::new(),
                },
                entity_id: receipts.legacy_open_extra.0,
                declared_slots: receipts.legacy_open_extra.1,
            });
            inventory.windows.insert(
                screen.id.window_id() as i8,
                screen
                    .slots
                    .iter()
                    .map(|v| {
                        v.as_ref().and_then(|v| match &v.value {
                            api::SlotKnowledge::Unavailable => None,
                            _ => super::common_click::legacy_comparison_stack(&v.value),
                        })
                    })
                    .collect(),
            );
        } else {
            inventory.open_window = None;
        }
        inventory
    }
    pub(super) async fn inventory_execution_scope(&self) -> (api::SessionStamp, InventorySource) {
        let receipts = self.common_receipts.lock().await;
        let session = api::SessionStamp {
            version: crate::MinecraftVersion::Java1_16_1,
            connection_id: self.connection_id(),
            world_generation: receipts.generation,
        };
        let mut source = receipts
            .container
            .as_ref()
            .map_or(InventorySource::Player, |s| InventorySource::Container {
                screen: s.id(session),
            });
        if receipts.inventory.window_id == Some(0)
            || matches!(
                api::container::player_screen_access(
                    session,
                    receipts.inventory.window_id,
                    receipts.container.as_ref().map(|s| s.id(session)),
                    self.common_container_close.lock().await.as_ref()
                ),
                Some(api::container::PlayerScreenAccess::SubmittedClose { .. })
            )
        {
            source = InventorySource::Player;
        }
        (session, source)
    }
    pub(super) fn inventory_resync_comparison(&self) -> Result<api::SlotKnowledge> {
        api::legacy_slot(Some(&ItemStack {
            item_id: crate::registry::item_id("stone").context("stone registry")?,
            count: i8::MAX,
            nbt: None,
        }))
    }
    pub(super) async fn require_received_click_preconditions(
        &self,
        window: i8,
        slot: i16,
    ) -> Result<()> {
        let receipts = self.common_receipts.lock().await;
        let known = if matches!(slot, -1 | -999) {
            true // protocol outside/no-effect slots still require an actually received cursor
        } else if window == 0 {
            usize::try_from(slot)
                .ok()
                .and_then(|s| receipts.inventory.slots.get(s))
                .is_some_and(Option::is_some)
        } else {
            let session = api::SessionStamp {
                version: crate::MinecraftVersion::Java1_16_1,
                connection_id: self.connection_id(),
                world_generation: receipts.generation,
            };
            receipts
                .screen(session)
                .filter(|s| s.id.window_id() == i32::from(window))
                .and_then(|s| {
                    usize::try_from(slot)
                        .ok()
                        .and_then(|i| s.slots.get(i).cloned())
                })
                .flatten()
                .is_some()
        };
        if !known || receipts.inventory.cursor.is_none() {
            return Err(inv::unavailable(
                "native click needs received source and cursor predecessors",
            ));
        }
        Ok(())
    }
    pub(super) async fn reserve_inventory_execution(
        &self,
        session: api::SessionStamp,
    ) -> Result<()> {
        let (current, source) = self.inventory_execution_scope().await;
        if session != current {
            return Err(inv::unavailable(
                "inventory opening changed before reservation",
            ));
        }
        lock_packet_state(&self.inventory_executor).await.begin(
            session,
            source,
            self.protocol_packet_sequence.load(Ordering::Acquire),
            None,
            self.connection_options.inventory_click_receipt_timeout,
            false,
        )?;
        Ok(())
    }
    pub(super) async fn begin_inventory_execution(
        &self,
        session: api::SessionStamp,
        action: i16,
        resync: bool,
    ) -> Result<inv::InventorySendId> {
        let (current, source) = self.inventory_execution_scope().await;
        if session != current {
            return Err(inv::unavailable("inventory generation changed before send"));
        }
        let mut executor = lock_packet_state(&self.inventory_executor).await;
        if let Some(record) = executor.record().filter(|r| {
            r.status == inv::InventorySendStatus::Pending
                && r.id.session() == session
                && r.id.source() == source
                && r.legacy_action == Some(action)
        }) {
            return Ok(record.id);
        }
        if !resync {
            if let Some(id) = executor.bind_legacy_action(
                session,
                source,
                action,
                self.protocol_packet_sequence.load(Ordering::Acquire),
                self.connection_options.inventory_click_receipt_timeout,
            ) {
                return Ok(id);
            }
        }
        executor.begin(
            session,
            source,
            self.protocol_packet_sequence.load(Ordering::Acquire),
            Some(action),
            self.connection_options.inventory_click_receipt_timeout,
            resync,
        )
    }
    pub(super) async fn inventory_execution_written(&self) {
        use crate::client::ledger::finite::Family;
        let record = {
            let mut executor = lock_packet_state(&self.inventory_executor).await;
            if let Some(record) = executor.record() {
                executor.written(
                    record.id,
                    self.connection_options.inventory_click_receipt_timeout,
                );
                Some(record)
            } else {
                None
            }
        };
        let Some(execution) = record else { return };
        let deadline =
            tokio::time::Instant::now() + self.connection_options.inventory_click_receipt_timeout;
        for (family, state) in [
            (
                Family::Click,
                super::lock_packet_state(&self.common_inventory_click)
                    .await
                    .as_ref()
                    .map(|r| (r.record.id.attempt(), r.record.send.after_sequence)),
            ),
            (
                Family::Transfer,
                super::lock_packet_state(&self.common_inventory_transfer)
                    .await
                    .as_ref()
                    .map(|r| (r.record.id.attempt(), r.record.send.after_sequence)),
            ),
            (
                Family::Crafting,
                super::lock_packet_state(&self.common_crafting_take)
                    .await
                    .as_ref()
                    .map(|r| (r.record.id.attempt(), r.record.send.after_sequence)),
            ),
            (
                Family::Swap,
                super::lock_packet_state(&self.common_inventory_swap)
                    .await
                    .as_ref()
                    .map(|r| (r.record.id.attempt(), r.record.send.after_sequence)),
            ),
        ] {
            if let Some((attempt, after)) =
                state.filter(|(_, after)| *after == execution.after_sequence)
            {
                let _ = after;
                self.finite_work
                    .lock()
                    .expect("finite work")
                    .expire_at(family, attempt, deadline);
            }
        }
        if execution.resync {
            self.finite_work.lock().expect("finite work").expire_at(
                Family::InventoryResync,
                execution.id.number(),
                deadline,
            );
        }
    }
    pub(super) async fn retain_window_prediction(&self, prediction: PendingClick) {
        let Some(execution) = lock_packet_state(&self.inventory_executor).await.record() else {
            return;
        };
        let mut history = self.window_predictions.lock().await;
        let bound = self.connection_options.max_cached_records.clamp(1, 4096);
        while history.len() >= bound {
            history.pop_front();
        }
        history.push_back((execution.id, prediction));
    }
    pub(super) async fn inventory_protocol_response(&self, reply: WindowTransaction) -> bool {
        let (session, source) = self.inventory_execution_scope().await;
        let mut executor = lock_packet_state(&self.inventory_executor).await;
        let matches = executor.record().is_some_and(|r| {
            r.id.session() == session
                && r.id.source() == source
                && r.window() == i32::from(reply.window_id)
                && r.legacy_action == Some(reply.action)
                && reply.packet_sequence > r.after_sequence
        });
        if matches {
            executor.response(session, source, reply.action, reply.packet_sequence);
        }
        matches
    }
}
impl api::adapter::InventorySendOps for Bot {
    async fn send_crafting_result(
        &self,
        mode: crate::client::GameMode,
        grid: &crate::client::crafting::ReceivedCrafting,
        destination: crate::client::crafting::CraftingResultDestination,
    ) -> Result<crate::client::inventory::InventorySendRecord> {
        Ok(self
            .start_crafting_take(mode, grid, destination, None, true)
            .await?
            .1)
    }

    async fn send_inventory_click(
        &self,
        mode: api::GameMode,
        source: InventorySource,
        slot: u16,
        button: inv::InventoryClickButton,
    ) -> Result<inv::InventorySendRecord> {
        Ok(self
            .start_click_inventory(mode, source, slot, button, true)
            .await?
            .1)
    }
    async fn send_inventory_transfer(
        &self,
        mode: api::GameMode,
        source: InventorySource,
        slot: u16,
    ) -> Result<inv::InventorySendRecord> {
        Ok(self
            .start_transfer_inventory(mode, source, slot, true)
            .await?
            .1)
    }
    async fn inventory_send_record(&self) -> Result<Option<inv::InventorySendRecord>> {
        Ok(lock_packet_state(&self.inventory_executor).await.record())
    }
    async fn resync_inventory(&self) -> Result<inv::InventorySendRecord> {
        let _gate = self.coherent_state_gate.lock().await;
        let (session, source) = self.inventory_execution_scope().await;
        let window = match source {
            InventorySource::Player => 0,
            InventorySource::Container { screen } => screen.window_id() as i8,
        };
        {
            let mut executor = lock_packet_state(&self.inventory_executor).await;
            if executor
                .record()
                .is_some_and(|r| r.status == inv::InventorySendStatus::Pending)
            {
                return Err(inv::unavailable(
                    "inventory still pending; await response or deadline",
                ));
            }
        }
        if let Some(previous) = lock_packet_state(&self.inventory_executor).await.record() {
            if let Some(action) = previous.legacy_action {
                let mut confirm = vec![previous.window() as u8];
                confirm.extend(action.to_be_bytes());
                confirm.push(1);
                self.connection
                    .dispatch_protocol_work(0x07, &confirm, None)
                    .await?;
            }
        }
        // Native PICKUP slot -1 returns Empty without changing any slot/cursor.
        // A deliberately unequal comparison requests full server resync. This
        // is a new no-effect probe, never the previous click's payload.
        let native =
            super::common_click::legacy_comparison_stack(&self.inventory_resync_comparison()?);
        let transaction = self
            .dispatch_window_ack_work_unlocked(
                self.operation_context(0),
                AcknowledgedOperation::WindowClick {
                    window_id: window,
                    slot: -1,
                    button: 0,
                    mode: ClickMode::Normal,
                    clicked: native,
                },
                None,
                true,
            )
            .await
            .map_err(|e| inv::unavailable(format!("inventory resync: {e}")))?;
        let action = transaction
            .window_action()
            .context("resync action unavailable")?;
        self.begin_inventory_execution(session, action, true)
            .await?;
        let execution = lock_packet_state(&self.inventory_executor)
            .await
            .record()
            .expect("retained resync");
        let work = self.begin_finite(
            crate::client::ledger::finite::Family::InventoryResync,
            execution.id.number(),
        );
        self.inventory_execution_written().await;
        self.spawn_finite_work(work, async move {
            let _ = transaction.wait().await;
        });
        Ok(lock_packet_state(&self.inventory_executor)
            .await
            .record()
            .expect("resync retained"))
    }
}

#[cfg(test)]
impl Bot {
    pub(super) async fn fixture_inventory_protocol_boundary(&self) {
        let Some(send) = lock_packet_state(&self.inventory_executor)
            .await
            .record()
            .filter(|r| r.status == inv::InventorySendStatus::Pending)
        else {
            return;
        };
        let actual = self.inventory().await;
        let Some(slots) = actual.windows.get(&(send.window() as i8)) else {
            return;
        };
        let mut full = vec![send.window() as u8];
        full.extend((slots.len() as i16).to_be_bytes());
        for slot in slots {
            crate::inventory::write_slot(&mut full, slot.as_ref());
        }
        Box::pin(self.apply_packet(0x14, full)).await.unwrap();
        let mut cursor = vec![255, 255, 255];
        crate::inventory::write_slot(&mut cursor, actual.cursor.as_ref());
        Box::pin(self.apply_packet(0x16, cursor)).await.unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::inventory::acceptance_tests::{self as script, Fixture};
    struct LegacyFixture {
        bot: Bot,
        packets: tokio::sync::mpsc::UnboundedReceiver<(i32, Vec<u8>)>,
    }
    fn native(v: &api::SlotKnowledge) -> Option<ItemStack> {
        super::super::common_click::legacy_comparison_stack(v)
    }
    impl Fixture for LegacyFixture {
        fn client(&self) -> crate::Client {
            crate::Client::from_java_1_16_1(self.bot.clone())
        }
        async fn inventory(&mut self, slots: &[api::SlotKnowledge], cursor: &api::SlotKnowledge) {
            let mut p = vec![0];
            p.extend((slots.len() as i16).to_be_bytes());
            for v in slots {
                crate::inventory::write_slot(&mut p, native(v).as_ref());
            }
            self.bot.apply_packet(0x14, p).await.unwrap();
            self.cursor(cursor).await;
        }
        async fn slot(&mut self, slot: u16, value: &api::SlotKnowledge) {
            let mut p = vec![0];
            p.extend((slot as i16).to_be_bytes());
            crate::inventory::write_slot(&mut p, native(value).as_ref());
            self.bot.apply_packet(0x16, p).await.unwrap();
        }
        async fn cursor(&mut self, value: &api::SlotKnowledge) {
            let mut p = vec![255, 255, 255];
            crate::inventory::write_slot(&mut p, native(value).as_ref());
            self.bot.apply_packet(0x16, p).await.unwrap();
        }
        async fn response(&mut self, r: &inv::InventorySendRecord, accepted: bool) {
            let mut p = vec![0];
            p.extend(r.legacy_action.unwrap().to_be_bytes());
            p.push(u8::from(accepted));
            self.bot.apply_packet(0x12, p).await.unwrap();
        }
        async fn click(&mut self) {
            loop {
                let (id, _) = tokio::time::timeout(Duration::from_secs(1), self.packets.recv())
                    .await
                    .unwrap()
                    .unwrap();
                if id == 0x09 {
                    break;
                }
            }
        }
    }
    #[tokio::test]
    async fn received_inventory_single_owner_pickup_transfer_crafting_matrix() {
        let (bot, packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        bot.apply_packet(0x3f, vec![0]).await.unwrap();
        let mut f = LegacyFixture { bot, packets };
        script::pickup_matrix(&mut f).await;
        script::missing_response_resync_and_late_response(&mut f).await;
        release.send(()).unwrap();
        let LegacyFixture { bot, packets } = f;
        drop(bot);
        server.await.unwrap();
        drop(packets);
    }
}
