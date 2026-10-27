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
pub(super) fn legacy_comparison_stack(comparison: &api::SlotKnowledge) -> Option<ItemStack> {
    match comparison {
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
    }
}
#[derive(Clone)]
pub(super) struct NativeInventoryClick {
    pub(super) record: InventoryClickRecord,
    receipt_deadline: Option<tokio::time::Instant>,
}
impl Bot {
    pub(super) async fn common_click_posture_changed(&self, pose: i32, receipt: u64) {
        if let Some(run) = super::lock_packet_state(&self.common_inventory_click)
            .await
            .as_mut()
        {
            if run.record.unresolved() {
                if let Some(predicate) =
                    click::Dependency::for_click(&run.record).posture_failure(pose)
                {
                    run.record.invalidation.get_or_insert(click::Invalidation {
                        predicate: predicate.into(),
                        change: click::StateChange::PostureChanged { pose },
                        receipt,
                    });
                    run.record.inspection(predicate);
                }
            }
        }
    }

    async fn common_click_send_owned(
        &self,
        id: InventoryClickId,
        revision: u64,
    ) -> Result<(
        InventoryClickRecord,
        crate::client::inventory::InventorySendRecord,
    )> {
        let _gate = self.coherent_state_gate.lock().await;
        let result = async {
            let record = super::lock_packet_state(&self.common_inventory_click)
                .await
                .as_ref()
                .filter(|s| s.record.id == id)
                .map(|s| s.record.clone())
                .ok_or_else(|| contract::unavailable("click intent superseded"))?;
            if record.requires_inspection.is_some() {
                return Err(contract::unavailable("click interrupted before I/O"));
            }
            if let Some(predicate) = click::Dependency::for_click(&record)
                .posture_failure(self.local_pose.lock().await.unwrap_or(-1))
            {
                return Err(contract::unavailable(predicate));
            }
            let current = self.common_player_unlocked().await?;
            let screen = self.common_receipts.lock().await.screen(current.session);
            let slots = click::source_slots(&record, &current, screen.as_ref())?;
            if let Some(context) = &record.item_context {
                let registries = self
                    .common_receipts
                    .lock()
                    .await
                    .registries
                    .capture(current.session, current.receive_sequence);
                context.classify(
                    &record.source_before,
                    &record.source_before,
                    &record.prediction.source.value,
                    &registries,
                )?;
            }
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
            // Original native PICKUP returns source_before. Deliberately use
            // the other nonempty predecessor (or Empty) for actual full resync.
            let processing_resync = false;
            let comparison = self.inventory_resync_comparison()?;
            let native = legacy_comparison_stack(&comparison);
            let action = self
                .connection
                .begin_window_click(id.attempt(), revision, record.window_id() as i8)
                .await
                .map_err(|e| contract::unavailable(format!("click reservation: {e:?}")))?;
            {
                let mut guard = super::lock_packet_state(&self.common_inventory_click).await;
                let r = &mut guard.as_mut().expect("retained").record;
                r.send.legacy_action = Some(action);
                r.send.legacy_comparison = Some(comparison);
                r.send.request_full_resync = processing_resync;
                r.send.after_sequence = self.protocol_packet_sequence.load(Ordering::Acquire);
            }
            let execution = self
                .begin_inventory_execution(record.initial.session, action, false)
                .await?;
            let basis = self.common_player_unlocked().await?;
            let basis_screen = self.common_receipts.lock().await.screen(basis.session);
            lock_packet_state(&self.inventory_executor)
                .await
                .comparison(crate::client::inventory::outcome::Comparison::click(
                    execution,
                    &record,
                    &basis,
                    basis_screen,
                ));
            if processing_resync {
                self.connection
                    .bounded_processing_click(
                        id.attempt(),
                        record.source_slot,
                        record.button.native(),
                        native,
                    )
                    .await?;
            } else {
                self.connection
                    .bounded_inventory_click(
                        id.attempt(),
                        record.source_slot,
                        record.button.native(),
                        native,
                    )
                    .await?;
            }
            self.inventory_execution_written().await;
            let mut guard = super::lock_packet_state(&self.common_inventory_click).await;
            let run = guard.as_mut().expect("retained");
            run.receipt_deadline = Some(
                tokio::time::Instant::now()
                    + self.connection_options.inventory_click_receipt_timeout,
            );
            self.finite_work.lock().expect("finite work").expire_at(
                crate::client::ledger::finite::Family::Click,
                id.attempt(),
                run.receipt_deadline.expect("deadline"),
            );
            run.record.send.dispatched = true;
            Ok((
                run.record.clone(),
                lock_packet_state(&self.inventory_executor)
                    .await
                    .record()
                    .expect("retained original send"),
            ))
        }
        .await;
        if let Err(e) = &result {
            let boundary = super::lock_packet_state(&self.common_inventory_click)
                .await
                .as_ref()
                .filter(|r| r.record.id == id)
                .map(|r| r.record.send.after_sequence);
            if let Some(after) = boundary {
                lock_packet_state(&self.inventory_executor)
                    .await
                    .cancel_unsent(id.session(), after);
            }
            self.interrupt_common_inventory_click(format!("click submission uncertain: {e}"))
                .await;
        }
        result
    }
    pub(super) async fn interrupt_common_inventory_click(&self, reason: impl std::fmt::Display) {
        if let Some(run) = super::lock_packet_state(&self.common_inventory_click)
            .await
            .as_mut()
        {
            run.record.inspection(reason);
        }
    }
    pub(super) async fn common_click_context_received(&self) -> Result<()> {
        self.reconcile_common_inventory_click().await
    }
    pub(super) async fn common_click_reply_received(&self, reply: WindowTransaction) {
        if let Some(run) = super::lock_packet_state(&self.common_inventory_click)
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
    async fn reconcile_common_inventory_click(&self) -> Result<()> {
        self.with_finite(crate::client::ledger::finite::Family::Click, async {
            let Some(snapshot) = super::lock_packet_state(&self.common_inventory_click)
                .await
                .clone()
            else {
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
            let screen = self.common_receipts.lock().await.screen(current.session);
            // Read before the record lock; never nest the executor inside it.
            let unchanged = lock_packet_state(&self.inventory_executor)
                .await
                .unchanged_boundary();
            let complete = {
                let mut guard = super::lock_packet_state(&self.common_inventory_click).await;
                let record = &mut guard.as_mut().expect("retained").record;
                let registries = self
                    .common_receipts
                    .lock()
                    .await
                    .registries
                    .capture(current.session, current.receive_sequence);
                if !record.ready()
                    && snapshot
                        .receipt_deadline
                        .is_some_and(|deadline| tokio::time::Instant::now() >= deadline)
                {
                    record.inspection("click receipt deadline elapsed; inspect without replay");
                }
                click::receive_with_registries(record, &current, screen.as_ref(), &registries);
                // After absorbing receipts: a fresh changed receipt wins.
                record.observe_unchanged(unchanged);
                // Complete at the receive boundary, before a later independent
                // inventory update can overwrite this already-observed outcome.
                // Inspection timing must not decide whether exact receipts count.
                record.ready()
            };
            if complete {
                let mut guard = super::lock_packet_state(&self.common_inventory_click).await;
                let run = guard.as_mut().expect("retained");
                run.record.stage = InventoryClickStage::ObservedClicked;
            }
            Ok(())
        })
        .await
    }
}

impl crate::client::adapter::InventoryClickOps for Bot {
    async fn click_inventory(
        &self,
        mode: api::GameMode,
        source: InventoryClickSource,
        slot: u16,
        button: InventoryClickButton,
    ) -> Result<InventoryClickRecord> {
        self.start_click_inventory(mode, source, slot, button, false)
            .await
            .map(|(record, _)| record)
    }
    async fn inventory_click_record(&self) -> Result<Option<InventoryClickRecord>> {
        // A stalled writer must not hide the already-retained intent. Complete
        // reconciliation owns its continuation if the inspection caller cancels.
        if let Ok(gate) = self.coherent_state_gate.clone().try_lock_owned() {
            let bot = self.clone_internal();
            let (reply, result) = tokio::sync::oneshot::channel();
            self.spawn_inspection(crate::client::ledger::finite::Family::Click, async move {
                let _gate = gate;
                let _ = reply.send(crate::client::ledger::finite::checked(
                    bot.reconcile_common_inventory_click().await,
                ));
            })
            .await;
            result
                .await
                .map_err(|_| contract::unavailable("click inspection owner unavailable"))??;
        }
        Ok(super::lock_packet_state(&self.common_inventory_click)
            .await
            .as_ref()
            .map(|s| s.record.clone()))
    }
}

impl Bot {
    pub(super) async fn start_click_inventory(
        &self,
        mode: api::GameMode,
        source: InventoryClickSource,
        slot: u16,
        button: InventoryClickButton,
        acceptance: bool,
    ) -> Result<(
        InventoryClickRecord,
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
                if let InventoryClickSource::Container { screen } = source {
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
                    .map_err(|e| contract::unavailable(format!("click admission: {e:?}")))?;
                if !self.inventory().await.pending_clicks.is_empty() {
                    return Err(contract::unavailable("native inventory click unresolved"));
                }
                let attempt = super::lock_packet_state(&self.common_inventory_click)
                    .await
                    .as_ref()
                    .map_or(Some(1), |p| p.record.id.attempt().checked_add(1))
                    .ok_or_else(|| contract::unavailable("inventory click attempts exhausted"))?;
                let initial = self.common_player_unlocked().await?;
                let screen = self.common_receipts.lock().await.screen(initial.session);
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
                if let Some(predicate) = click::Dependency::for_click(&record)
                    .posture_failure(self.local_pose.lock().await.unwrap_or(-1))
                {
                    return Err(contract::unavailable(predicate));
                }
                self.reserve_inventory_execution(record.initial.session)
                    .await?;
                let id = record.id;
                let work =
                    self.begin_finite(crate::client::ledger::finite::Family::Click, id.attempt());
                *super::lock_packet_state(&self.common_inventory_click).await =
                    Some(NativeInventoryClick {
                        record,
                        receipt_deadline: None,
                    });
                let (reply, result) = tokio::sync::oneshot::channel();
                let bot = self.clone_internal();
                self.spawn_finite_work(work, async move {
                    let _ = reply.send(crate::client::ledger::finite::checked(
                        bot.common_click_send_owned(id, revision).await,
                    ));
                });
                drop(gate);
                result
                    .await
                    .map_err(|_| contract::unavailable("click owner result unavailable"))?
            })
            .await
    }
}
