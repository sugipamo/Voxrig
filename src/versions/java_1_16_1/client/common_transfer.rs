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
}
impl Bot {
    async fn common_transfer_send_owned(
        &self,
        id: InventoryTransferId,
        revision: u64,
    ) -> Result<(
        InventoryTransferRecord,
        crate::client::inventory::InventorySendRecord,
    )> {
        let _gate = self.coherent_state_gate.lock().await;
        let result = async {
            let record = super::lock_packet_state(&self.common_inventory_transfer)
                .await
                .as_ref()
                .filter(|s| s.record.id == id)
                .map(|s| s.record.clone())
                .ok_or_else(|| contract::unavailable("transfer intent superseded"))?;
            if record.requires_inspection.is_some() {
                return Err(contract::unavailable("transfer interrupted before I/O"));
            }
            let current = self.common_player_unlocked().await?;
            let screen = self.common_receipts.lock().await.screen(current.session);
            let registries = self
                .common_receipts
                .lock()
                .await
                .registries
                .capture(current.session, current.receive_sequence);
            transfer::validate_predecessors(&record, &current, screen.as_ref(), &registries)?;
            // QUICK_MOVE may return Empty on a complete first round, but a
            // nonempty predecessor after any partial internal round. These
            // predictions are separately audited, never received outcomes.
            let comparison = self.inventory_resync_comparison()?;
            let native = super::common_click::legacy_comparison_stack(&comparison);
            let action = self
                .connection
                .begin_window_transfer(id.attempt(), revision, record.window_id() as i8)
                .await
                .map_err(|e| contract::unavailable(format!("transfer reservation: {e:?}")))?;
            {
                let mut guard = super::lock_packet_state(&self.common_inventory_transfer).await;
                let r = &mut guard.as_mut().expect("retained").record;
                r.send.legacy_action = Some(action);
                r.send.legacy_comparison = Some(comparison);
                r.send.after_sequence = self.protocol_packet_sequence.load(Ordering::Acquire);
            }
            let execution = self
                .begin_inventory_execution(record.initial.session, action, false)
                .await?;
            let basis = self.common_player_unlocked().await?;
            let basis_screen = self.common_receipts.lock().await.screen(basis.session);
            lock_packet_state(&self.inventory_executor)
                .await
                .comparison(crate::client::inventory::outcome::Comparison::transfer(
                    execution,
                    &record,
                    &basis,
                    basis_screen,
                ));
            self.connection
                .bounded_inventory_transfer(id.attempt(), record.source_slot, 0, native)
                .await?;
            self.inventory_execution_written().await;
            let mut guard = super::lock_packet_state(&self.common_inventory_transfer).await;
            let record = &mut guard.as_mut().expect("retained").record;
            record.send.dispatched = true;
            Ok((
                record.clone(),
                lock_packet_state(&self.inventory_executor)
                    .await
                    .record()
                    .expect("retained original send"),
            ))
        }
        .await;
        if let Err(e) = &result {
            let boundary = super::lock_packet_state(&self.common_inventory_transfer)
                .await
                .as_ref()
                .filter(|r| r.record.id == id)
                .map(|r| r.record.send.after_sequence);
            if let Some(after) = boundary {
                lock_packet_state(&self.inventory_executor)
                    .await
                    .cancel_unsent(id.session(), after);
            }
            self.interrupt_common_inventory_transfer(format!("transfer submission uncertain: {e}"))
                .await;
        }
        result
    }
    pub(super) async fn interrupt_common_inventory_transfer(&self, reason: impl std::fmt::Display) {
        if let Some(run) = super::lock_packet_state(&self.common_inventory_transfer)
            .await
            .as_mut()
        {
            run.record.inspection(reason);
        }
    }
    pub(super) async fn common_transfer_context_received(&self) -> Result<()> {
        self.reconcile_common_inventory_transfer().await
    }
    pub(super) async fn common_transfer_reply_received(&self, reply: WindowTransaction) {
        if let Some(run) = super::lock_packet_state(&self.common_inventory_transfer)
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
    async fn reconcile_common_inventory_transfer(&self) -> Result<()> {
        self.with_finite(crate::client::ledger::finite::Family::Transfer, async {
            let Some(snapshot) = super::lock_packet_state(&self.common_inventory_transfer)
                .await
                .clone()
            else {
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
            let screen = self.common_receipts.lock().await.screen(current.session);
            let inventory = self.inventory().await;
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
            // Read before the record lock; never nest the executor inside it.
            let unchanged = lock_packet_state(&self.inventory_executor)
                .await
                .unchanged_boundary();
            let complete = {
                let mut guard = super::lock_packet_state(&self.common_inventory_transfer).await;
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
                // After absorbing receipts: a fresh changed receipt wins.
                record.observe_unchanged(unchanged);
                record.ready()
            };
            if complete {
                let mut guard = super::lock_packet_state(&self.common_inventory_transfer).await;
                let run = guard.as_mut().expect("retained");
                run.record.stage = InventoryTransferStage::ObservedTransferred;
            }
            Ok(())
        })
        .await
    }
}

impl crate::client::adapter::InventoryTransferOps for Bot {
    async fn transfer_inventory(
        &self,
        mode: api::GameMode,
        source: InventorySource,
        slot: u16,
    ) -> Result<InventoryTransferRecord> {
        self.start_transfer_inventory(mode, source, slot, false)
            .await
            .map(|(record, _)| record)
    }
    async fn inventory_transfer_record(&self) -> Result<Option<InventoryTransferRecord>> {
        // A stalled writer must not hide the already-retained intent. Complete
        // reconciliation owns its continuation if the inspection caller cancels.
        if let Ok(gate) = self.coherent_state_gate.clone().try_lock_owned() {
            let bot = self.clone_internal();
            let (reply, result) = tokio::sync::oneshot::channel();
            self.spawn_inspection(
                crate::client::ledger::finite::Family::Transfer,
                async move {
                    let _gate = gate;
                    let _ = reply.send(crate::client::ledger::finite::checked(
                        bot.reconcile_common_inventory_transfer().await,
                    ));
                },
            )
            .await;
            result
                .await
                .map_err(|_| contract::unavailable("transfer inspection owner unavailable"))??;
        }
        Ok(super::lock_packet_state(&self.common_inventory_transfer)
            .await
            .as_ref()
            .map(|s| s.record.clone()))
    }
}

impl Bot {
    pub(super) async fn start_transfer_inventory(
        &self,
        mode: api::GameMode,
        source: InventorySource,
        slot: u16,
        acceptance: bool,
    ) -> Result<(
        InventoryTransferRecord,
        crate::client::inventory::InventorySendRecord,
    )> {
        self.connection
            .ledger()
            .try_work()?
            .run_scoped(async {
                let gate = self.coherent_state_gate.lock().await;
                self.common_domain_admission(
                    false,
                    None,
                    if acceptance {
                        api::domain::Domain::CurrentInventory
                    } else {
                        api::domain::Domain::Strict
                    },
                )
                .await?;
                if let InventorySource::Container { screen } = source {
                    if super::lock_packet_state(&self.common_container_close)
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
                if !self.inventory().await.pending_clicks.is_empty() {
                    return Err(contract::unavailable("native inventory click unresolved"));
                }
                let attempt = super::lock_packet_state(&self.common_inventory_transfer)
                    .await
                    .as_ref()
                    .map_or(Some(1), |p| p.record.id.attempt().checked_add(1))
                    .ok_or_else(|| {
                        contract::unavailable("inventory transfer attempts exhausted")
                    })?;
                let initial = self.common_player_unlocked().await?;
                let screen = self.common_receipts.lock().await.screen(initial.session);
                let registries = self
                    .common_receipts
                    .lock()
                    .await
                    .registries
                    .capture(initial.session, initial.receive_sequence);
                let record = transfer::prepare_received(
                    (initial, registries, None),
                    mode,
                    source,
                    slot,
                    attempt,
                    screen,
                )?;
                {
                    let inventory = self.inventory().await;
                    if inventory
                        .open_window
                        .as_ref()
                        .map_or(0, |s| i32::from(s.id))
                        != record.window_id()
                        || api::legacy_slot(inventory.cursor.as_ref())?
                            != record.cursor_before.value
                    {
                        return Err(contract::unavailable(
                            "legacy transfer received/cache UI basis disagrees",
                        ));
                    }
                }
                self.reserve_inventory_execution(record.initial.session)
                    .await?;
                let id = record.id;
                let work = self.begin_finite(
                    crate::client::ledger::finite::Family::Transfer,
                    id.attempt(),
                );
                *super::lock_packet_state(&self.common_inventory_transfer).await =
                    Some(NativeInventoryTransfer { record });
                let (reply, result) = tokio::sync::oneshot::channel();
                let bot = self.clone_internal();
                self.spawn_finite_work(work, async move {
                    let _ = reply.send(crate::client::ledger::finite::checked(
                        bot.common_transfer_send_owned(id, revision).await,
                    ));
                });
                drop(gate);
                result
                    .await
                    .map_err(|_| contract::unavailable("transfer owner result unavailable"))?
            })
            .await
    }
}
