//! Actor-owned ordinary QUICK_MOVE, retaining predictions separately from receipts.
use super::*;
use crate::client::{
    self as api,
    inventory::{self as contract, transfer},
};
use contract::{
    InventorySource, InventoryTransferId, InventoryTransferRecord, InventoryTransferStage,
};
#[derive(Clone)]
pub(super) struct NativeInventoryTransfer {
    pub(super) record: InventoryTransferRecord,
    pub(super) released: bool,
}
impl Bot {
    pub(crate) async fn common_transfer_inventory(
        &self,
        mode: api::GameMode,
        source: InventorySource,
        slot: u16,
    ) -> Result<InventoryTransferRecord> {
        let gate = self.coherent_state_gate.lock().await;
        self.common_motion_admission().await?;
        if let InventorySource::Container { screen } = source {
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
            .map_err(|e| contract::unavailable(format!("transfer admission: {e:?}")))?;
        if !self.inventory.read().await.pending_clicks.is_empty() {
            return Err(contract::unavailable("native inventory click unresolved"));
        }
        let attempt = self
            .common_inventory_transfer
            .lock()
            .await
            .as_ref()
            .map_or(Some(1), |p| p.record.id.attempt().checked_add(1))
            .ok_or_else(|| contract::unavailable("inventory transfer attempts exhausted"))?;
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
        let record =
            transfer::prepare_received((initial, registries), mode, source, slot, attempt, screen)?;
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
                    "legacy transfer received/cache UI basis disagrees",
                ));
            }
        }
        let id = record.id;
        *self.common_inventory_transfer.lock().await = Some(NativeInventoryTransfer {
            record,
            released: false,
        });
        let (reply, result) = tokio::sync::oneshot::channel();
        let bot = self.clone_internal();
        tokio::spawn(async move {
            let _ = reply.send(bot.common_transfer_send_owned(id, revision).await);
        });
        drop(gate);
        result
            .await
            .map_err(|_| contract::unavailable("transfer owner result unavailable"))?
    }
    async fn common_transfer_send_owned(
        &self,
        id: InventoryTransferId,
        revision: u64,
    ) -> Result<InventoryTransferRecord> {
        let _gate = self.coherent_state_gate.lock().await;
        let result = async {
            let record = self
                .common_inventory_transfer
                .lock()
                .await
                .as_ref()
                .filter(|s| s.record.id == id)
                .map(|s| s.record.clone())
                .ok_or_else(|| contract::unavailable("transfer intent superseded"))?;
            if record.requires_inspection.is_some() {
                return Err(contract::unavailable("transfer interrupted before I/O"));
            }
            let current = self.common_player_unlocked().await?;
            let screen = self
                .common_receipts
                .lock()
                .await
                .container
                .as_ref()
                .map(|s| s.capture(current.session));
            let registries = self
                .common_receipts
                .lock()
                .await
                .registries
                .capture(current.session, current.receive_sequence);
            transfer::validate_predecessors(&record, &current, screen.as_ref(), &registries)?;
            {
                let inventory = self.inventory.read().await;
                if !inventory.pending_clicks.is_empty()
                    || inventory
                        .open_window
                        .as_ref()
                        .map_or(0, |s| i32::from(s.id))
                        != record.window_id()
                    || current.inventory.cursor.as_ref().map(|v| &v.value)
                        != Some(&api::legacy_slot(inventory.cursor.as_ref())?)
                {
                    return Err(contract::unavailable(
                        "legacy transfer cache/owner changed before I/O",
                    ));
                }
            }
            // QUICK_MOVE may return Empty on a complete first round, but a
            // nonempty predecessor after any partial internal round. These
            // predictions are separately audited, never received outcomes.
            let comparison = if record
                .legacy_return_prediction
                .as_ref()
                .is_some_and(|v| v.value == api::SlotKnowledge::Empty)
            {
                record.before_slots[usize::from(record.source_slot)]
                    .as_ref()
                    .expect("validated source")
                    .value
                    .clone()
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
                .begin_window_transfer(id.attempt(), revision, record.window_id() as i8)
                .await
                .map_err(|e| contract::unavailable(format!("transfer reservation: {e:?}")))?;
            {
                let mut guard = self.common_inventory_transfer.lock().await;
                let r = &mut guard.as_mut().expect("retained").record;
                r.send.legacy_action = Some(action);
                r.send.legacy_comparison = Some(comparison);
                r.send.after_sequence = self.protocol_packet_sequence.load(Ordering::Acquire);
            }
            self.connection
                .bounded_inventory_transfer(id.attempt(), record.source_slot, 0, native)
                .await?;
            let mut guard = self.common_inventory_transfer.lock().await;
            let record = &mut guard.as_mut().expect("retained").record;
            record.send.dispatched = true;
            Ok(record.clone())
        }
        .await;
        if let Err(e) = &result {
            self.interrupt_common_inventory_transfer(format!("transfer submission uncertain: {e}"))
                .await;
        }
        result
    }
    pub(crate) async fn common_inventory_transfer_record(
        &self,
    ) -> Result<Option<InventoryTransferRecord>> {
        // A stalled writer must not hide the already-retained intent. Complete
        // reconciliation owns its continuation if the inspection caller cancels.
        if let Ok(gate) = self.coherent_state_gate.clone().try_lock_owned() {
            let bot = self.clone_internal();
            let (reply, result) = tokio::sync::oneshot::channel();
            tokio::spawn(async move {
                let _gate = gate;
                let _ = reply.send(bot.reconcile_common_inventory_transfer(true).await);
            });
            result
                .await
                .map_err(|_| contract::unavailable("transfer inspection owner unavailable"))??;
        }
        Ok(self
            .common_inventory_transfer
            .lock()
            .await
            .as_ref()
            .map(|s| s.record.clone()))
    }
    pub(super) async fn interrupt_common_inventory_transfer(&self, reason: impl std::fmt::Display) {
        if let Some(run) = self.common_inventory_transfer.lock().await.as_mut() {
            run.record.inspection(reason);
        }
    }
    pub(super) async fn common_transfer_context_received(&self) -> Result<()> {
        self.reconcile_common_inventory_transfer(false).await
    }
    pub(super) async fn common_transfer_reply_received(&self, reply: WindowTransaction) {
        if let Some(run) = self
            .common_inventory_transfer
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
    async fn reconcile_common_inventory_transfer(&self, confirm: bool) -> Result<()> {
        let Some(snapshot) = self.common_inventory_transfer.lock().await.clone() else {
            return Ok(());
        };
        if !snapshot.record.unresolved() {
            return Ok(());
        }
        if self.is_stopped() || self.connection_state() != ConnectionState::Ready {
            self.interrupt_common_inventory_transfer("transfer connection closed or uncertain")
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
            let mut guard = self.common_inventory_transfer.lock().await;
            let record = &mut guard.as_mut().expect("retained").record;
            let registries = self
                .common_receipts
                .lock()
                .await
                .registries
                .capture(current.session, current.receive_sequence);
            transfer::receive_with_registries(record, &current, screen.as_ref(), &registries);
            if native_conflict {
                record.inspection("legacy transfer cache/ownership context changed");
            }
            confirm && record.ready()
        };
        if complete {
            match self
                .connection
                .finish_inventory_transfer(snapshot.record.id.attempt())
                .await
            {
                Ok(()) => {
                    let mut guard = self.common_inventory_transfer.lock().await;
                    let run = guard.as_mut().expect("retained");
                    run.released = true;
                    run.record.stage = InventoryTransferStage::ObservedTransferred;
                }
                Err(e) => {
                    self.interrupt_common_inventory_transfer(format!(
                        "transfer completion gate: {e:?}"
                    ))
                    .await
                }
            }
        }
        Ok(())
    }
}
