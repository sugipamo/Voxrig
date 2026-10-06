//! Actor-owned result take; never mutates or predicts native crafting inputs.
use super::*;
use crate::client::{
    self as api,
    crafting::{self as contract, CraftingSource, CraftingTakeRecord, CraftingTakeStage, take},
    inventory::unavailable,
};
#[derive(Clone)]
pub(super) struct NativeCraftingTake {
    pub(super) record: CraftingTakeRecord,
    pub(super) released: bool,
}
impl Bot {
    pub(crate) async fn common_take_crafting_result(
        &self,
        mode: api::GameMode,
        grid: &contract::ReceivedCrafting,
    ) -> Result<CraftingTakeRecord> {
        let gate = self.coherent_state_gate.lock().await;
        self.common_motion_admission().await?;
        if let CraftingSource::Table { screen } = grid.source() {
            if self
                .common_container_close
                .lock()
                .await
                .as_ref()
                .is_some_and(|r| r.id.screen() == screen)
            {
                return Err(unavailable("old table opening has retained close intent"));
            }
        }
        let revision = self
            .connection
            .motion_admission_revision()
            .await
            .map_err(|e| unavailable(format!("crafting admission: {e:?}")))?;
        if !self.inventory.read().await.pending_clicks.is_empty() {
            return Err(unavailable("native inventory click unresolved"));
        }
        let attempt = self
            .common_crafting_take
            .lock()
            .await
            .as_ref()
            .map_or(Some(1), |r| r.record.id.attempt().checked_add(1))
            .ok_or_else(|| unavailable("crafting take attempts exhausted"))?;
        let initial = self.common_player_unlocked().await?;
        let table = self
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
        let screen = take::screen_observation(&initial, table);
        let record = take::prepare(initial, screen, registries, grid, mode, attempt)?;
        self.crafting_cache(&record).await?;
        let id = record.id;
        *self.common_crafting_take.lock().await = Some(NativeCraftingTake {
            record,
            released: false,
        });
        let (reply, result) = tokio::sync::oneshot::channel();
        let bot = self.clone_internal();
        tokio::spawn(async move {
            let _ = reply.send(bot.common_crafting_send_owned(id, revision).await);
        });
        drop(gate);
        result
            .await
            .map_err(|_| unavailable("crafting owner result unavailable"))?
    }
    async fn crafting_cache(&self, record: &CraftingTakeRecord) -> Result<()> {
        let inventory = self.inventory.read().await;
        if !inventory.pending_clicks.is_empty()
            || inventory
                .open_window
                .as_ref()
                .map_or(0, |s| i32::from(s.id))
                != record.window_id()
            || api::legacy_slot(inventory.cursor.as_ref())? != record.cursor_before.value
        {
            return Err(unavailable(
                "crafting received/native cache ownership disagrees",
            ));
        }
        Ok(())
    }
    async fn common_crafting_send_owned(
        &self,
        id: contract::CraftingTakeId,
        revision: u64,
    ) -> Result<CraftingTakeRecord> {
        let _gate = self.coherent_state_gate.lock().await;
        let result = async {
            let record = self
                .common_crafting_take
                .lock()
                .await
                .as_ref()
                .filter(|r| r.record.id == id)
                .map(|r| r.record.clone())
                .ok_or_else(|| unavailable("crafting intent superseded"))?;
            if record.requires_inspection.is_some() {
                return Err(unavailable("crafting interrupted before I/O"));
            }
            let current = self.common_player_unlocked().await?;
            let table = self
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
            take::validate_before(
                &record,
                &current,
                &take::screen_observation(&current, table),
                &registries,
            )?;
            self.crafting_cache(&record).await?;
            // Original result PICKUP returns the nonempty output predecessor.
            // Empty comparison deliberately requests actual native full resync.
            let action = self
                .connection
                .begin_crafting_result_take(id.attempt(), revision, record.window_id() as i8)
                .await
                .map_err(|e| unavailable(format!("crafting reservation: {e:?}")))?;
            {
                let mut guard = self.common_crafting_take.lock().await;
                let record = &mut guard.as_mut().expect("retained").record;
                record.send.legacy_action = Some(action);
                record.send.legacy_comparison = Some(api::SlotKnowledge::Empty);
                record.send.after_sequence = self.protocol_packet_sequence.load(Ordering::Acquire);
            }
            self.connection
                .bounded_inventory_click(id.attempt(), 0, 0, None)
                .await?;
            let mut guard = self.common_crafting_take.lock().await;
            let record = &mut guard.as_mut().expect("retained").record;
            record.send.dispatched = true;
            Ok(record.clone())
        }
        .await;
        if let Err(e) = &result {
            self.interrupt_common_crafting_take(format!("crafting submission uncertain: {e}"))
                .await;
        }
        result
    }
    pub(super) async fn interrupt_common_crafting_take(&self, reason: impl std::fmt::Display) {
        if let Some(r) = self.common_crafting_take.lock().await.as_mut() {
            r.record.inspection(reason);
        }
    }
    pub(crate) async fn common_crafting_take_record(&self) -> Result<Option<CraftingTakeRecord>> {
        if let Ok(gate) = self.coherent_state_gate.clone().try_lock_owned() {
            let bot = self.clone_internal();
            let (reply, result) = tokio::sync::oneshot::channel();
            tokio::spawn(async move {
                let _gate = gate;
                let _ = reply.send(bot.common_crafting_context_received(true).await);
            });
            result
                .await
                .map_err(|_| unavailable("crafting inspection owner unavailable"))??;
        }
        Ok(self
            .common_crafting_take
            .lock()
            .await
            .as_ref()
            .map(|r| r.record.clone()))
    }
    pub(super) async fn common_crafting_reply_received(&self, reply: WindowTransaction) {
        if let Some(r) = self.common_crafting_take.lock().await.as_mut().filter(|r| {
            r.record.unresolved()
                && r.record.send.legacy_action == Some(reply.action)
                && r.record.window_id() == i32::from(reply.window_id)
                && reply.packet_sequence > r.record.send.after_sequence
        }) {
            r.record
                .legacy_reply
                .get_or_insert(api::inventory::InventoryTransactionReply {
                    window_id: reply.window_id,
                    action: reply.action,
                    accepted: reply.accepted,
                    receive_sequence: reply.packet_sequence,
                });
        }
    }
    pub(super) async fn common_crafting_context_received(&self, confirm: bool) -> Result<()> {
        let Some(snapshot) = self.common_crafting_take.lock().await.clone() else {
            return Ok(());
        };
        if !snapshot.record.unresolved() {
            return Ok(());
        }
        if self.is_stopped() || self.connection_state() != ConnectionState::Ready {
            self.interrupt_common_crafting_take("crafting connection closed or uncertain")
                .await;
            return Ok(());
        }
        let current = self.common_player_unlocked().await?;
        let table = self
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
        let inventory = self.inventory.read().await;
        let cache_conflict = !inventory.pending_clicks.is_empty()
            || inventory
                .open_window
                .as_ref()
                .map_or(0, |s| i32::from(s.id))
                != snapshot.record.window_id()
            || current.inventory.cursor.as_ref().is_none_or(|c| {
                api::legacy_slot(inventory.cursor.as_ref()).map_or(true, |v| c.value != v)
            });
        drop(inventory);
        let complete = {
            let mut guard = self.common_crafting_take.lock().await;
            let record = &mut guard.as_mut().expect("retained").record;
            take::receive(
                record,
                &current,
                &take::screen_observation(&current, table),
                &registries,
            );
            if cache_conflict {
                record.inspection("crafting cache/ownership context changed");
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
                    let mut guard = self.common_crafting_take.lock().await;
                    let r = guard.as_mut().expect("retained");
                    r.released = true;
                    r.record.stage = CraftingTakeStage::ObservedTaken;
                }
                Err(e) => {
                    self.interrupt_common_crafting_take(format!("crafting completion gate: {e:?}"))
                        .await
                }
            }
        }
        Ok(())
    }
}
