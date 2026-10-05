//! Actor-owned ordinary PICKUP, retaining predictions separately from receipts.
use super::*;
use crate::client::{
    self as api,
    inventory::{self as contract, click},
};
use contract::{
    InventoryClickButton, InventoryClickId, InventoryClickRecord, InventoryClickSource,
    InventoryClickStage,
};
#[derive(Clone)]
pub(super) struct NativeInventoryClick {
    pub(super) record: InventoryClickRecord,
    pub(super) released: bool,
}
impl Bot {
    pub(crate) async fn common_click_inventory(
        &self,
        mode: api::GameMode,
        source: InventoryClickSource,
        slot: u16,
        button: InventoryClickButton,
    ) -> Result<InventoryClickRecord> {
        let gate = self.coherent_state_gate.lock().await;
        self.common_motion_admission().await?;
        if let InventoryClickSource::Container { screen } = source {
            if self
                .common_container_close
                .lock()
                .await
                .as_ref()
                .is_some_and(|r| r.id.screen() == screen)
            {
                return Err(contract::unavailable(
                    "old opening has a retained close intent",
                ));
            }
        }
        let revision = self
            .connection
            .motion_admission_revision()
            .await
            .map_err(|e| contract::unavailable(format!("click admission: {e:?}")))?;
        if !self.inventory.read().await.pending_clicks.is_empty() {
            return Err(contract::unavailable("native inventory click unresolved"));
        }
        let attempt = self
            .common_inventory_click
            .lock()
            .await
            .as_ref()
            .map_or(Some(1), |p| p.record.id.attempt().checked_add(1))
            .ok_or_else(|| contract::unavailable("inventory click attempts exhausted"))?;
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
        let record = click::prepare_received(
            (initial, registries),
            mode,
            source,
            slot,
            button,
            attempt,
            screen,
        )?;
        {
            let inventory = self.inventory.read().await;
            if inventory
                .open_window
                .as_ref()
                .map_or(0, |s| i32::from(s.id))
                != record.window_id()
                || api::legacy_slot(inventory.cursor.as_ref())? != record.cursor_before.value
            {
                return Err(contract::unavailable(
                    "legacy click received/cache UI basis disagrees",
                ));
            }
        }
        let id = record.id;
        *self.common_inventory_click.lock().await = Some(NativeInventoryClick {
            record,
            released: false,
        });
        let (reply, result) = tokio::sync::oneshot::channel();
        let bot = self.clone_internal();
        tokio::spawn(async move {
            let _ = reply.send(bot.common_click_send_owned(id, revision).await);
        });
        drop(gate);
        result
            .await
            .map_err(|_| contract::unavailable("click owner result unavailable"))?
    }
    async fn common_click_send_owned(
        &self,
        id: InventoryClickId,
        revision: u64,
    ) -> Result<InventoryClickRecord> {
        let _gate = self.coherent_state_gate.lock().await;
        let result = async {
            let record = self
                .common_inventory_click
                .lock()
                .await
                .as_ref()
                .filter(|s| s.record.id == id)
                .map(|s| s.record.clone())
                .ok_or_else(|| contract::unavailable("click intent superseded"))?;
            if record.requires_inspection.is_some() {
                return Err(contract::unavailable("click interrupted before I/O"));
            }
            let current = self.common_player_unlocked().await?;
            let screen = self
                .common_receipts
                .lock()
                .await
                .container
                .as_ref()
                .map(|s| s.capture(current.session));
            let slots = click::source_slots(&record, &current, screen.as_ref())?;
            if !contract::same_received_value(
                slots
                    .get(usize::from(record.source_slot))
                    .and_then(Option::as_ref),
                &record.source_before,
            ) || !contract::same_received_value(
                current.inventory.cursor.as_ref(),
                &record.cursor_before,
            ) {
                return Err(contract::unavailable("click capture changed before I/O"));
            }
            {
                let inventory = self.inventory.read().await;
                if !inventory.pending_clicks.is_empty()
                    || inventory
                        .open_window
                        .as_ref()
                        .map_or(0, |s| i32::from(s.id))
                        != record.window_id()
                    || api::legacy_slot(inventory.cursor.as_ref())? != record.cursor_before.value
                {
                    return Err(contract::unavailable(
                        "legacy click cache/owner changed before I/O",
                    ));
                }
            }
            // Original native PICKUP returns source_before. Deliberately use
            // the other nonempty predecessor (or Empty) for actual full resync.
            let comparison = if matches!(record.source_before.value, api::SlotKnowledge::Empty) {
                record.cursor_before.value.clone()
            } else {
                api::SlotKnowledge::Empty
            };
            let native = match &comparison {
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
                api::SlotKnowledge::Unavailable => unreachable!("validated predecessor"),
            };
            let action = self
                .connection
                .begin_window_click(id.attempt(), revision, record.window_id() as i8)
                .await
                .map_err(|e| contract::unavailable(format!("click reservation: {e:?}")))?;
            {
                let mut guard = self.common_inventory_click.lock().await;
                let r = &mut guard.as_mut().expect("retained").record;
                r.send.legacy_action = Some(action);
                r.send.legacy_comparison = Some(comparison);
                r.send.after_sequence = self.protocol_packet_sequence.load(Ordering::Acquire);
            }
            self.connection
                .bounded_inventory_click(
                    id.attempt(),
                    record.source_slot,
                    record.button.native(),
                    native,
                )
                .await?;
            let mut guard = self.common_inventory_click.lock().await;
            let record = &mut guard.as_mut().expect("retained").record;
            record.send.dispatched = true;
            Ok(record.clone())
        }
        .await;
        if let Err(e) = &result {
            self.interrupt_common_inventory_click(format!("click submission uncertain: {e}"))
                .await;
        }
        result
    }
    pub(crate) async fn common_inventory_click_record(
        &self,
    ) -> Result<Option<InventoryClickRecord>> {
        // A stalled writer must not hide the already-retained intent. Complete
        // reconciliation owns its continuation if the inspection caller cancels.
        if let Ok(gate) = self.coherent_state_gate.clone().try_lock_owned() {
            let bot = self.clone_internal();
            let (reply, result) = tokio::sync::oneshot::channel();
            tokio::spawn(async move {
                let _gate = gate;
                let _ = reply.send(bot.reconcile_common_inventory_click(true).await);
            });
            result
                .await
                .map_err(|_| contract::unavailable("click inspection owner unavailable"))??;
        }
        Ok(self
            .common_inventory_click
            .lock()
            .await
            .as_ref()
            .map(|s| s.record.clone()))
    }
    pub(super) async fn interrupt_common_inventory_click(&self, reason: impl std::fmt::Display) {
        if let Some(run) = self.common_inventory_click.lock().await.as_mut() {
            run.record.inspection(reason);
        }
    }
    pub(super) async fn common_click_context_received(&self) -> Result<()> {
        self.reconcile_common_inventory_click(false).await
    }
    pub(super) async fn common_click_reply_received(&self, reply: WindowTransaction) {
        if let Some(run) = self
            .common_inventory_click
            .lock()
            .await
            .as_mut()
            .filter(|s| {
                s.record.unresolved()
                    && s.record.send.legacy_action == Some(reply.action)
                    && s.record.window_id() == i32::from(reply.window_id)
                    && reply.packet_sequence > s.record.send.after_sequence
            })
        {
            run.record
                .legacy_reply
                .get_or_insert(contract::InventoryTransactionReply {
                    window_id: reply.window_id,
                    action: reply.action,
                    accepted: reply.accepted,
                    receive_sequence: reply.packet_sequence,
                });
        }
    }
    async fn reconcile_common_inventory_click(&self, confirm: bool) -> Result<()> {
        let Some(snapshot) = self.common_inventory_click.lock().await.clone() else {
            return Ok(());
        };
        if !snapshot.record.unresolved() {
            return Ok(());
        }
        if self.is_stopped() || self.connection_state() != ConnectionState::Ready {
            self.interrupt_common_inventory_click("click connection closed or uncertain")
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
        let cache_cursor = api::legacy_slot(inventory.cursor.as_ref())?;
        let native_conflict = !inventory.pending_clicks.is_empty()
            || inventory
                .open_window
                .as_ref()
                .map_or(0, |s| i32::from(s.id))
                != snapshot.record.window_id()
            || current
                .inventory
                .cursor
                .as_ref()
                .is_none_or(|c| c.value != cache_cursor);
        drop(inventory);
        let complete = {
            let mut guard = self.common_inventory_click.lock().await;
            let record = &mut guard.as_mut().expect("retained").record;
            let registries = self
                .common_receipts
                .lock()
                .await
                .registries
                .capture(current.session, current.receive_sequence);
            click::receive_with_registries(record, &current, screen.as_ref(), &registries);
            if native_conflict {
                record.inspection("legacy click cache/ownership context changed");
            }
            confirm && record.ready()
        };
        if complete {
            match self
                .connection
                .finish_inventory_click(snapshot.record.id.attempt())
                .await
            {
                Ok(()) => {
                    let mut guard = self.common_inventory_click.lock().await;
                    let run = guard.as_mut().expect("retained");
                    run.released = true;
                    run.record.stage = InventoryClickStage::ObservedClicked;
                }
                Err(e) => {
                    self.interrupt_common_inventory_click(format!("click completion gate: {e:?}"))
                        .await
                }
            }
        }
        Ok(())
    }
}
