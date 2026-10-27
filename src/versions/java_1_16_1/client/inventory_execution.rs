//! Packet-owned inventory execution, independent of strict evidence histories.
use super::*;
use crate::client::{
    self as api,
    inventory::{self as inv, InventorySource},
};
impl Bot {
    // Caller holds coherent_state_gate and the receipts lock. This is the one
    // legacy projection of local close authority onto received inventory. It
    // never changes the reducer's original window/opening or value provenance.
    pub(super) async fn received_inventory_projection(
        &self,
        receipts: &api::observation::LegacyReceipts,
        session: api::SessionStamp,
    ) -> api::InventoryObservation {
        let mut inventory = receipts.inventory.clone();
        inventory.player_screen = api::container::player_screen_access(
            session,
            inventory.window_id,
            receipts.container.as_ref().map(|s| s.id(session)),
            lock_packet_state(&self.common_container_close)
                .await
                .as_ref(),
        );
        inventory
    }
    async fn inventory_execution_snapshot(&self) -> inv::InventorySnapshot {
        let receipts = self.common_receipts.lock().await;
        let session = api::SessionStamp {
            version: crate::MinecraftVersion::Java1_16_1,
            connection_id: self.connection_id(),
            world_generation: receipts.generation,
        };
        let inventory = self.received_inventory_projection(&receipts, session).await;
        inv::InventorySnapshot::from_inventory(
            session,
            self.protocol_packet_sequence.load(Ordering::Acquire),
            &inventory,
            receipts.screen(session),
        )
    }
    pub(super) async fn inventory_changes_received(&self, packet: i32) {
        if !matches!(packet, 0x12 | 0x13 | 0x14 | 0x16 | 0x25 | 0x2e | 0x3a) {
            return;
        }
        let current = self.inventory_execution_snapshot().await;
        let crafting_complete = lock_packet_state(&self.common_crafting_take)
            .await
            .as_ref()
            .is_some_and(|r| {
                matches!(
                    r.record.stage,
                    api::crafting::CraftingTakeStage::ObservedTaken
                        | api::crafting::CraftingTakeStage::ObservedTransferred
                )
            });
        let mut executor = lock_packet_state(&self.inventory_executor).await;
        executor.receive(current, crafting_complete);
    }
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
        // InventorySnapshot owns Player/Container selection for admission and
        // comparison alike; do not reconstruct that decision from raw receipts.
        let current = self.inventory_execution_snapshot().await;
        (current.session, current.source)
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
            if !reply.accepted {
                executor.rejected();
            }
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
        async fn collect(&mut self, entity: i32, own: bool) {
            self.bot.player.lock().await.entity_id = Some(42);
            let mut p = Vec::new();
            crate::protocol::put_varint(&mut p, entity);
            crate::protocol::put_varint(&mut p, if own { 42 } else { 43 });
            crate::protocol::put_varint(&mut p, 1);
            self.bot.apply_packet(0x55, p).await.unwrap();
        }
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
            let mut p = vec![r.window() as u8];
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
    async fn submitted_close_missing_late_ack_and_reopen_require_resync() {
        let (bot, packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        bot.apply_packet(0x3f, vec![0]).await.unwrap();
        let mut f = LegacyFixture { bot, packets };
        let client = f.client();
        let stone = api::legacy_slot(Some(&ItemStack {
            item_id: crate::registry::item_id("stone").unwrap(),
            count: 7,
            nbt: None,
        }))
        .unwrap();
        let mut slots = vec![api::SlotKnowledge::Empty; 46];
        slots[9] = stone.clone();
        f.inventory(&slots, &api::SlotKnowledge::Empty).await;
        f.close_furnace(&stone).await;
        let old = client
            .survival()
            .send_inventory_click(InventorySource::Player, 9, inv::InventoryClickButton::Left)
            .await
            .unwrap();
        f.click().await;
        slots[9] = api::SlotKnowledge::Empty;
        f.inventory(&slots, &stone).await;
        assert!(client.survival().resync_inventory().await.is_err());
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(6)).await;
        assert_eq!(
            client
                .survival()
                .inventory_send_record()
                .await
                .unwrap()
                .unwrap()
                .status,
            inv::InventorySendStatus::ResyncRequired
        );
        tokio::time::resume();
        assert!(
            client
                .survival()
                .send_inventory_click(InventorySource::Player, 9, inv::InventoryClickButton::Left,)
                .await
                .is_err()
        );
        let sync = client.survival().resync_inventory().await.unwrap();
        f.click().await;
        f.response(&old, false).await; // late original ACK cannot acknowledge resync
        f.inventory(&slots, &stone).await;
        assert_eq!(
            client
                .survival()
                .inventory_send_record()
                .await
                .unwrap()
                .unwrap()
                .status,
            inv::InventorySendStatus::Pending
        );
        f.response(&sync, false).await;
        assert_eq!(
            client
                .survival()
                .inventory_send_record()
                .await
                .unwrap()
                .unwrap()
                .status,
            inv::InventorySendStatus::Diverged
        );
        assert!(
            client
                .inventory_divergence(old.id)
                .await
                .unwrap()
                .unwrap()
                .causes
                .contains(&inv::InventoryCause::TimedOutResync)
        );
        let next = client
            .survival()
            .send_inventory_click(InventorySource::Player, 9, inv::InventoryClickButton::Left)
            .await
            .unwrap();
        f.click().await;
        // A different received opening (even the same numeric window) retires the send.
        let first = f.furnace(&stone).await;
        assert_eq!(
            client
                .survival()
                .inventory_send_record()
                .await
                .unwrap()
                .unwrap()
                .status,
            inv::InventorySendStatus::ResyncRequired
        );
        let second = f.furnace(&stone).await;
        assert_ne!(first, second);
        f.response(&next, false).await;
        assert_eq!(
            client
                .survival()
                .inventory_send_record()
                .await
                .unwrap()
                .unwrap()
                .status,
            inv::InventorySendStatus::ResyncRequired
        );
        let reopened = client.survival().resync_inventory().await.unwrap();
        f.click().await;
        assert_eq!(
            reopened.id.source(),
            InventorySource::Container { screen: second }
        );
        f.response(&reopened, false).await;
        let mut full = vec![3, 0, 39];
        full.extend([0; 39]);
        f.bot.apply_packet(0x14, full).await.unwrap();
        f.cursor(&api::SlotKnowledge::Empty).await;
        assert_eq!(
            client
                .survival()
                .inventory_send_record()
                .await
                .unwrap()
                .unwrap()
                .status,
            inv::InventorySendStatus::Received
        );
        release.send(()).unwrap();
        drop(client);
        drop(f.bot);
        server.await.unwrap();
    }
    impl LegacyFixture {
        async fn furnace(&mut self, item: &api::SlotKnowledge) -> api::container::ScreenId {
            let menu =
                api::furnace::native_menu(crate::MinecraftVersion::Java1_16_1, "minecraft:furnace")
                    .unwrap();
            let mut open = vec![3];
            crate::protocol::put_varint(&mut open, menu.native_id);
            put_string(&mut open, "{}");
            self.bot.apply_packet(0x2e, open).await.unwrap();
            let mut full = vec![3, 0, 39];
            for slot in 0..39 {
                crate::inventory::write_slot(
                    &mut full,
                    if slot == 3 { native(item) } else { None }.as_ref(),
                );
            }
            self.bot.apply_packet(0x14, full).await.unwrap();
            self.cursor(&api::SlotKnowledge::Empty).await;
            self.client()
                .screen_state()
                .await
                .unwrap()
                .screen
                .unwrap()
                .id
        }
        async fn close_furnace(&mut self, item: &api::SlotKnowledge) {
            let screen = self.furnace(item).await;
            let close = self
                .client()
                .survival()
                .close_container(screen)
                .await
                .unwrap();
            assert!(close.dispatched);
            assert!(close.server_close_sequence.is_none());
            assert!(matches!(
                self.client()
                    .player_state()
                    .await
                    .unwrap()
                    .inventory
                    .player_screen,
                Some(api::container::PlayerScreenAccess::SubmittedClose { .. })
            ));
        }
        async fn full_player(&mut self, slots: &[api::SlotKnowledge]) {
            let mut full = vec![0];
            full.extend((slots.len() as i16).to_be_bytes());
            for slot in slots {
                crate::inventory::write_slot(&mut full, native(slot).as_ref());
            }
            self.bot.apply_packet(0x14, full).await.unwrap();
        }
    }
    #[tokio::test]
    async fn submitted_close_player_crafting_take_matches_unechoed_native_window() {
        let (bot, packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        bot.apply_packet(0x3f, vec![0]).await.unwrap();
        let mut f = LegacyFixture { bot, packets };
        let client = f.client();
        let item = |name: &str, count: i8| {
            api::legacy_slot(Some(&ItemStack {
                item_id: crate::registry::item_id(name).unwrap(),
                count,
                nbt: None,
            }))
            .unwrap()
        };
        let mut slots = vec![api::SlotKnowledge::Empty; 46];
        slots[9] = item("oak_planks", 2);
        f.inventory(&slots, &api::SlotKnowledge::Empty).await;
        // Vanilla does not echo this local close; the native cache keeps window 3.
        f.close_furnace(&slots[9].clone()).await;
        assert_eq!(f.bot.inventory().await.open_window.map(|w| w.id), Some(3));
        f.slot(1, &item("oak_planks", 1)).await;
        f.slot(3, &item("oak_planks", 1)).await;
        f.slot(0, &item("stick", 4)).await;
        let grid = client.received_crafting().await.unwrap().unwrap();
        let take = client.survival().take_crafting_result(&grid).await.unwrap();
        assert!(take.send.dispatched);
        f.click().await;
        drop(client);
        f.bot.disconnect().await.unwrap();
        drop(release);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn submitted_close_allows_taking_retained_player_crafting_result() {
        let (bot, packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        bot.apply_packet(0x3f, vec![0]).await.unwrap();
        let mut f = LegacyFixture { bot, packets };
        let client = f.client();
        let mut slots = vec![api::SlotKnowledge::Empty; 46];
        for (index, name) in [(0, "oak_button"), (1, "oak_planks")] {
            slots[index] = api::legacy_slot(Some(&ItemStack {
                item_id: crate::registry::item_id(name).unwrap(),
                count: 1,
                nbt: None,
            }))
            .unwrap();
        }
        f.inventory(&slots, &api::SlotKnowledge::Empty).await;
        f.close_furnace(&api::SlotKnowledge::Empty).await;
        // Native history still describes the furnace. Admission belongs to the
        // common received projection, whose exact submitted close grants access.
        assert_eq!(f.bot.inventory().await.open_window.unwrap().id, 3);
        let grid = client.received_crafting().await.unwrap().unwrap();
        assert_eq!(grid.input(0, 0).unwrap().unwrap().value(), &slots[1]);
        let take = client.survival().take_crafting_result(&grid).await.unwrap();
        assert!(take.send.dispatched);
        assert_eq!(take.window_id(), 0);
        f.click().await;
        let mut reply = vec![0];
        reply.extend(take.send.legacy_action.unwrap().to_be_bytes());
        reply.push(0); // original rejected-comparison/full-resync response
        f.bot.apply_packet(0x12, reply).await.unwrap();
        f.full_player(&vec![api::SlotKnowledge::Empty; 46]).await;
        f.cursor(&slots[0]).await;
        let completed = client
            .survival()
            .crafting_take_record()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            completed.stage,
            api::crafting::CraftingTakeStage::ObservedTaken
        );
        client.disconnect().await.unwrap();
        drop(release);
        drop(client);
        drop(f);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn submitted_close_pickaxe_send_settles_at_response_state_boundary() {
        let (bot, packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        bot.apply_packet(0x3f, vec![0]).await.unwrap();
        let mut f = LegacyFixture { bot, packets };
        let client = f.client();
        let pickaxe = api::legacy_slot(Some(&ItemStack {
            item_id: crate::registry::item_id("wooden_pickaxe").unwrap(),
            count: 1,
            nbt: None,
        }))
        .unwrap();
        // Every order of ACK, full contents, and cursor, including ACK last.
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            for external in [false, true] {
                let mut slots = vec![api::SlotKnowledge::Empty; 46];
                slots[9] = pickaxe.clone();
                f.inventory(&slots, &api::SlotKnowledge::Empty).await;
                f.close_furnace(&pickaxe).await;
                let sent = client
                    .survival()
                    .send_inventory_click(
                        InventorySource::Player,
                        9,
                        inv::InventoryClickButton::Left,
                    )
                    .await
                    .unwrap();
                f.click().await;
                slots[9] = api::SlotKnowledge::Empty;
                if external {
                    slots[19] = pickaxe.clone();
                }
                for (index, event) in order.into_iter().enumerate() {
                    match event {
                        0 => f.response(&sent, false).await,
                        1 => f.full_player(&slots).await,
                        _ => f.cursor(&pickaxe).await,
                    }
                    let done = client
                        .survival()
                        .inventory_send_record()
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(done.id, sent.id);
                    if index < 2 {
                        assert_eq!(done.status, inv::InventorySendStatus::Pending);
                    } else if external {
                        assert_eq!(done.status, inv::InventorySendStatus::Diverged);
                    } else {
                        // Fresh full contents can conservatively diverge, but
                        // must still settle this original send and release it.
                        assert!(
                            matches!(
                                done.status,
                                inv::InventorySendStatus::Received
                                    | inv::InventorySendStatus::Diverged
                            ),
                            "order={order:?}, status={:?}",
                            done.status
                        );
                    }
                }
                assert_eq!(
                    client
                        .survival()
                        .inventory_click_record()
                        .await
                        .unwrap()
                        .unwrap()
                        .stage,
                    inv::InventoryClickStage::ObservedClicked
                );
                let evidence = client.inventory_divergence(sent.id).await.unwrap();
                if external {
                    assert!(evidence.is_some());
                }
                if let Some(evidence) = evidence {
                    assert_eq!(evidence.before.source, InventorySource::Player);
                    assert_eq!(evidence.current.source, InventorySource::Player);
                    assert!(evidence.before.screen.is_none() && evidence.current.screen.is_none());
                    assert!(evidence.causes.contains(&inv::InventoryCause::Rejected));
                    assert!(evidence.causes.contains(&inv::InventoryCause::External));
                    assert!(evidence.current.receive_sequence > sent.after_sequence);
                    assert_eq!(
                        evidence.current.player_slots[9].as_ref().unwrap().value,
                        api::SlotKnowledge::Empty
                    );
                    assert_eq!(evidence.current.cursor.as_ref().unwrap().value, pickaxe);
                }
                let done = client
                    .survival()
                    .inventory_send_record()
                    .await
                    .unwrap()
                    .unwrap();
                let settled_evidence =
                    serde_json::to_value(client.inventory_divergence(sent.id).await.unwrap())
                        .unwrap();
                f.response(&sent, false).await; // duplicate cannot finish a second time
                f.cursor(&pickaxe).await;
                assert_eq!(
                    serde_json::to_value(&done).unwrap(),
                    serde_json::to_value(
                        client
                            .survival()
                            .inventory_send_record()
                            .await
                            .unwrap()
                            .unwrap()
                    )
                    .unwrap()
                );
                assert_eq!(
                    settled_evidence,
                    serde_json::to_value(client.inventory_divergence(sent.id).await.unwrap())
                        .unwrap()
                );
                // Admission is released: a new original click and explicit resync both work.
                let next = client
                    .survival()
                    .send_inventory_click(
                        InventorySource::Player,
                        9,
                        inv::InventoryClickButton::Left,
                    )
                    .await
                    .unwrap();
                f.click().await;
                slots[9] = pickaxe.clone();
                f.inventory(&slots, &api::SlotKnowledge::Empty).await;
                f.response(&next, false).await;
                assert_eq!(
                    client
                        .survival()
                        .inventory_send_record()
                        .await
                        .unwrap()
                        .unwrap()
                        .status,
                    inv::InventorySendStatus::Received
                );
                let resync = client.survival().resync_inventory().await.unwrap();
                f.click().await;
                f.response(&resync, false).await;
                f.inventory(&slots, &api::SlotKnowledge::Empty).await;
                assert_eq!(
                    client
                        .survival()
                        .inventory_send_record()
                        .await
                        .unwrap()
                        .unwrap()
                        .status,
                    inv::InventorySendStatus::Received
                );
            }
        }
        release.send(()).unwrap();
        drop(client);
        drop(f.bot);
        server.await.unwrap();
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
        script::collect_and_external_log(&mut f).await;
        script::crafting_result_outcomes(&mut f).await;
        release.send(()).unwrap();
        let LegacyFixture { bot, packets } = f;
        drop(bot);
        server.await.unwrap();
        drop(packets);
    }
}
