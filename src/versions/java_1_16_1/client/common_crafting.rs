//! Actor-owned result take; never mutates or predicts native crafting inputs.
use super::*;
use crate::client::{
    self as api,
    crafting::{
        self as contract, CraftingResultDestination, CraftingSource, CraftingTakeRecord, take,
    },
    inventory::unavailable,
};
#[derive(Clone)]
pub(super) struct NativeCraftingTake {
    pub(super) record: CraftingTakeRecord,
}
impl Bot {
    async fn common_crafting_send_owned(
        &self,
        id: contract::CraftingTakeId,
        revision: u64,
        _gate: tokio::sync::OwnedMutexGuard<()>,
        submission: Option<Arc<take::Submission>>,
    ) -> Result<(
        CraftingTakeRecord,
        crate::client::inventory::InventorySendRecord,
    )> {
        let result = async {
            let record = super::lock_packet_state(&self.common_crafting_take)
                .await
                .as_ref()
                .filter(|r| r.record.id == id)
                .map(|r| r.record.clone())
                .ok_or_else(|| unavailable("crafting intent superseded"))?;
            if record.requires_inspection.is_some() {
                return Err(unavailable("crafting interrupted before I/O"));
            }
            let current = self.common_player_unlocked().await?;
            let table = self.common_receipts.lock().await.screen(current.session);
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
            // validate_before owns source/cursor admission, including submitted
            // closes. The native projection can still show the historical menu.
            let comparison = record.legacy_full_comparison()?;
            let native = match &comparison {
                api::SlotKnowledge::Empty => None,
                api::SlotKnowledge::Item { item } => Some(ItemStack {
                    item_id: item.id.value(),
                    count: item.count as i8,
                    nbt: None,
                }),
                _ => unreachable!("validated full resync comparison"),
            };
            let action = match record.destination {
                CraftingResultDestination::Cursor => {
                    self.connection
                        .begin_crafting_result_take(
                            id.attempt(),
                            revision,
                            record.window_id() as i8,
                        )
                        .await
                }
                CraftingResultDestination::Inventory => {
                    self.connection
                        .begin_crafting_result_transfer(
                            id.attempt(),
                            revision,
                            record.window_id() as i8,
                        )
                        .await
                }
            }
            .map_err(|e| unavailable(format!("crafting reservation: {e:?}")))?;
            {
                let mut guard = super::lock_packet_state(&self.common_crafting_take).await;
                let record = &mut guard.as_mut().expect("retained").record;
                record.send.legacy_action = Some(action);
                record.send.legacy_comparison = Some(comparison);
                record.send.after_sequence = self.protocol_packet_sequence.load(Ordering::Acquire);
                if let Some(submission) = &submission {
                    submission.retain(record);
                }
            }
            let execution = self
                .begin_inventory_execution(record.initial.session, action, false)
                .await?;
            let basis = self.common_player_unlocked().await?;
            let basis_screen = self.common_receipts.lock().await.screen(basis.session);
            lock_packet_state(&self.inventory_executor)
                .await
                .comparison(crate::client::inventory::outcome::Comparison::crafting(
                    execution,
                    &record,
                    &basis,
                    basis_screen,
                ));
            self.connection
                .bounded_crafting_result_click(
                    id.attempt(),
                    native,
                    submission.as_ref().map(|s| s.write_started.clone()),
                )
                .await?;
            self.inventory_execution_written().await;
            let mut guard = super::lock_packet_state(&self.common_crafting_take).await;
            let record = &mut guard.as_mut().expect("retained").record;
            record.send.dispatched = true;
            self.start_crafting_deadline(id);
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
            let boundary = super::lock_packet_state(&self.common_crafting_take)
                .await
                .as_ref()
                .filter(|r| r.record.id == id)
                .map(|r| r.record.send.after_sequence);
            if let Some(after) = boundary {
                lock_packet_state(&self.inventory_executor)
                    .await
                    .cancel_unsent(id.session(), after);
            }
            self.interrupt_common_crafting_take(format!("crafting submission uncertain: {e}"))
                .await;
            if let Some(submission) = &submission {
                if let Some(retained) = super::lock_packet_state(&self.common_crafting_take)
                    .await
                    .as_ref()
                {
                    if retained.record.id == id {
                        submission.retain(&retained.record);
                    }
                }
            }
        }
        result
    }
    fn start_crafting_deadline(&self, id: contract::CraftingTakeId) {
        let bot = self.clone_internal();
        let first =
            tokio::time::Instant::now() + self.connection_options.inventory_click_receipt_timeout;
        self.spawn_finite(Box::pin(async move {
            let second = bot.connection_options.crafting_take_resync_timeout;
            let mut deadline = first;
            let mut resynced = false;
            loop {
                tokio::time::sleep_until(
                    deadline.min(tokio::time::Instant::now() + Duration::from_millis(20)),
                )
                .await;
                let pending = lock_packet_state(&bot.common_crafting_take)
                    .await
                    .as_ref()
                    .is_some_and(|r| {
                        r.record.id == id && r.record.stage == contract::CraftingTakeStage::Pending
                    });
                if !pending {
                    break;
                }
                if tokio::time::Instant::now() < deadline {
                    continue;
                }
                if resynced {
                    if let Some(r) = lock_packet_state(&bot.common_crafting_take)
                        .await
                        .as_mut()
                        .filter(|r| {
                            r.record.id == id
                                && r.record.stage == contract::CraftingTakeStage::Pending
                        })
                    {
                        r.record.inspect_as(
                            contract::CraftingTakeInspectionReason::ReceiptDeadlineAfterResync,
                            "crafting receipt deadline after no-effect full resync",
                        );
                    }
                    break;
                }
                // A stalled writer/lock is bounded too. No original click is ever replayed.
                match tokio::time::timeout(second, bot.crafting_deadline_resync(id)).await {
                    Ok(Ok(())) => {}
                    _ => {
                        if let Some(r) = lock_packet_state(&bot.common_crafting_take)
                            .await
                            .as_mut()
                            .filter(|r| {
                                r.record.id == id
                                    && r.record.stage == contract::CraftingTakeStage::Pending
                            })
                        {
                            r.record.inspect_as(
                                contract::CraftingTakeInspectionReason::ResyncDeliveryUncertain,
                                "crafting no-effect full resync delivery uncertain",
                            );
                        }
                        break;
                    }
                }
                resynced = true;
                deadline = tokio::time::Instant::now() + second;
            }
        }));
    }
    async fn crafting_deadline_resync(&self, id: contract::CraftingTakeId) -> Result<()> {
        let _gate = self.coherent_state_gate.lock().await;
        self.common_crafting_context_received().await?;
        let record = lock_packet_state(&self.common_crafting_take)
            .await
            .as_ref()
            .filter(|r| r.record.id == id && r.record.stage == contract::CraftingTakeStage::Pending)
            .map(|r| r.record.clone());
        let Some(record) = record else {
            return Ok(());
        };
        // Release a possibly withheld mismatch acknowledgement before probing.
        // This is protocol progress, never evidence that the original take failed.
        let mut confirm = vec![record.window_id() as u8];
        confirm.extend(
            record
                .send
                .legacy_action
                .expect("dispatched take action")
                .to_be_bytes(),
        );
        confirm.push(1);
        self.connection
            .dispatch_protocol_work(0x07, &confirm, None)
            .await?;
        let transaction = self
            .connection
            .dispatch_acknowledged_with_diagnostic(
                self.operation_context(0),
                OperationClass::Normal,
                AcknowledgedOperation::WindowClick {
                    window_id: record.window_id() as i8,
                    slot: -1,
                    button: 0,
                    mode: ClickMode::Normal,
                    clicked: super::common_click::legacy_comparison_stack(
                        &self.inventory_resync_comparison()?,
                    ),
                },
                None,
            )
            .await
            .map_err(|e| unavailable(format!("crafting no-effect resync: {e}")))?;
        // The protocol actor owns this probe's reply; preserve the ORIGINAL
        // inventory executor identity, comparison, and take transaction reply.
        if let Some(r) = lock_packet_state(&self.common_crafting_take)
            .await
            .as_mut()
            .filter(|r| r.record.id == id)
        {
            r.record.deadline_resync_dispatched = transaction.was_dispatched();
            r.record.deadline_resync_action = transaction.window_action();
        }
        if !transaction.was_dispatched() {
            return Err(unavailable("crafting resync not dispatched"));
        }
        Ok(())
    }
    pub(super) async fn interrupt_common_crafting_take(&self, reason: impl std::fmt::Display) {
        if let Some(r) = super::lock_packet_state(&self.common_crafting_take)
            .await
            .as_mut()
        {
            r.record.inspection(reason);
        }
    }
    pub(super) async fn common_crafting_reply_received(&self, reply: WindowTransaction) {
        if let Some(r) = lock_packet_state(&self.common_crafting_take)
            .await
            .as_mut()
            .filter(|r| {
                r.record.deadline_resync_action == Some(reply.action)
                    && r.record.window_id() == i32::from(reply.window_id)
                    && reply.packet_sequence > r.record.send.after_sequence
            })
        {
            r.record.deadline_resync_reply.get_or_insert(
                api::inventory::InventoryTransactionReply {
                    window_id: reply.window_id,
                    action: reply.action,
                    accepted: reply.accepted,
                    receive_sequence: reply.packet_sequence,
                },
            );
        }

        if let Some(r) = super::lock_packet_state(&self.common_crafting_take)
            .await
            .as_mut()
            .filter(|r| {
                r.record.unresolved()
                    && r.record.send.legacy_action == Some(reply.action)
                    && r.record.window_id() == i32::from(reply.window_id)
                    && reply.packet_sequence > r.record.send.after_sequence
            })
        {
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
    pub(super) async fn common_crafting_context_received(&self) -> Result<()> {
        self.with_finite(crate::client::ledger::finite::Family::Crafting, async {
            let Some(snapshot) = super::lock_packet_state(&self.common_crafting_take)
                .await
                .clone()
            else {
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
            let table = self.common_receipts.lock().await.screen(current.session);
            let registries = self
                .common_receipts
                .lock()
                .await
                .registries
                .capture(current.session, current.receive_sequence);
            let complete = {
                let mut guard = super::lock_packet_state(&self.common_crafting_take).await;
                let record = &mut guard.as_mut().expect("retained").record;
                take::receive(
                    record,
                    &current,
                    &take::screen_observation(&current, table),
                    &registries,
                );
                record.ready()
            };
            if complete {
                let mut guard = super::lock_packet_state(&self.common_crafting_take).await;
                let run = guard.as_mut().expect("retained");
                run.record.stage = run.record.completed_stage();
            }
            Ok(())
        })
        .await
    }
}

impl crate::client::adapter::CraftingTakeOps for Bot {
    async fn take_crafting_result_with_submission(
        &self,
        mode: api::GameMode,
        grid: &contract::ReceivedCrafting,
        destination: CraftingResultDestination,
        submission: Option<Arc<take::Submission>>,
    ) -> Result<CraftingTakeRecord> {
        self.start_crafting_take(mode, grid, destination, submission, false)
            .await
            .map(|(record, _)| record)
    }
    async fn crafting_take_record(&self) -> Result<Option<CraftingTakeRecord>> {
        if let Ok(gate) = self.coherent_state_gate.clone().try_lock_owned() {
            let bot = self.clone_internal();
            let (reply, result) = tokio::sync::oneshot::channel();
            self.spawn_inspection(
                crate::client::ledger::finite::Family::Crafting,
                async move {
                    let _gate = gate;
                    let _ = reply.send(crate::client::ledger::finite::checked(
                        bot.common_crafting_context_received().await,
                    ));
                },
            )
            .await;
            result
                .await
                .map_err(|_| unavailable("crafting inspection owner unavailable"))??;
        }
        Ok(super::lock_packet_state(&self.common_crafting_take)
            .await
            .as_ref()
            .map(|r| r.record.clone()))
    }
}

impl Bot {
    pub(super) async fn start_crafting_take(
        &self,
        mode: api::GameMode,
        grid: &contract::ReceivedCrafting,
        destination: CraftingResultDestination,
        submission: Option<Arc<take::Submission>>,
        acceptance: bool,
    ) -> Result<(
        CraftingTakeRecord,
        crate::client::inventory::InventorySendRecord,
    )> {
        self.connection
            .ledger()
            .try_work()?
            .run_scoped(async {
                let gate = self.coherent_state_gate.clone().lock_owned().await;
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
                if let CraftingSource::Table { screen } = grid.source() {
                    if super::lock_packet_state(&self.common_container_close)
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
                if !self.inventory().await.pending_clicks.is_empty() {
                    return Err(unavailable("native inventory click unresolved"));
                }
                let attempt = super::lock_packet_state(&self.common_crafting_take)
                    .await
                    .as_ref()
                    .map_or(Some(1), |r| r.record.id.attempt().checked_add(1))
                    .ok_or_else(|| unavailable("crafting take attempts exhausted"))?;
                let initial = self.common_player_unlocked().await?;
                let table = self.common_receipts.lock().await.screen(initial.session);
                let registries = self
                    .common_receipts
                    .lock()
                    .await
                    .registries
                    .capture(initial.session, initial.receive_sequence);
                let screen = take::screen_observation(&initial, table);
                let record = match destination {
                    CraftingResultDestination::Cursor => {
                        take::prepare(initial, screen, registries, grid, mode, attempt)?
                    }
                    CraftingResultDestination::Inventory => {
                        take::prepare_transfer(initial, screen, registries, grid, mode, attempt)?
                    }
                };
                // prepare owns received source/cursor admission; reserve below
                // independently excludes unresolved inventory execution.
                self.reserve_inventory_execution(record.initial.session)
                    .await?;
                let id = record.id;
                if let Some(submission) = &submission {
                    submission.retain(&record);
                }
                let work = self.begin_finite(
                    crate::client::ledger::finite::Family::Crafting,
                    id.attempt(),
                );
                *super::lock_packet_state(&self.common_crafting_take).await =
                    Some(NativeCraftingTake { record });
                let (reply, result) = tokio::sync::oneshot::channel();
                let bot = self.clone_internal();
                self.spawn_finite_work(work, async move {
                    // Transfer the original admission lock to the connection owner:
                    // queued receipts cannot change the predecessor between reservation and I/O.
                    let _ = reply.send(crate::client::ledger::finite::checked(
                        bot.common_crafting_send_owned(id, revision, gate, submission)
                            .await,
                    ));
                });
                result
                    .await
                    .map_err(|_| unavailable("crafting owner result unavailable"))?
            })
            .await
    }
}
