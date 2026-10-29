//! Result-only native PICKUP, with full-grid receipts and a short history lock.
use super::*;
use crate::client::{
    self as api,
    crafting::{
        self as contract, CraftingResultDestination, CraftingSource, CraftingTakeRecord, take,
    },
    inventory::unavailable,
};
fn publish(state: &mut State) {
    let mut history = state
        .crafting_take_history
        .lock()
        .expect("crafting history");
    if let (Some(record), Some(previous)) = (state.common_crafting_take.as_mut(), history.as_ref())
    {
        if record.id == previous.id {
            if let Some(reason) = &previous.requires_inspection {
                record.inspect_as(
                    previous.inspection_reason.unwrap_or(
                        contract::CraftingTakeInspectionReason::EvidenceOrDeliveryUncertain,
                    ),
                    reason,
                );
            }
        }
    }
    *history = state.common_crafting_take.clone();
}
pub(in crate::versions::java_1_21_11::client) fn context_received(state: &mut State) {
    let Some(mut record) = state.common_crafting_take.take() else {
        return;
    };
    if record.unresolved() {
        match super::super::common_player_in_state(state, record.id.session().connection_id, false)
        {
            Ok(current) => {
                let table = state.operations.inventory.screen(current.session);
                let registries = state
                    .registries
                    .capture(current.session, current.receive_sequence);
                take::receive(
                    &mut record,
                    &current,
                    &take::screen_observation(&current, table),
                    &registries,
                );
            }
            Err(e) => record.inspection(e),
        }
        let inventory = &state.operations.inventory;
        if !state.ready
            || !matches!(state.phase, Phase::Play)
            || state.failure.is_some()
            || inventory.pending_swap.is_some()
            || inventory.unsupported_components
            || !inventory.pending_creative.is_empty()
            || !state.loading.notification_dispatched()
        {
            record.inspection("crafting native session/inventory/loading ownership changed");
        }
        if record.ready() {
            record.stage = record.completed_stage();
        }
    }
    state.common_crafting_take = Some(record);
    publish(state);
}
pub(super) fn payload(record: &CraftingTakeRecord) -> Result<Vec<u8>> {
    let revision = record
        .send
        .sent_screen_revision
        .ok_or_else(|| unavailable("crafting command revision unavailable"))?;
    let mut bytes = Vec::new();
    put_varint(&mut bytes, record.window_id());
    put_varint(&mut bytes, revision);
    bytes.extend(0i16.to_be_bytes());
    bytes.extend([0, record.destination.click_mode(), 0]); // left, native mode, no predicted modified-slot map
    put_default_cursor_hash(&mut bytes, &InventorySlot::Empty)?;
    Ok(bytes)
}
impl Operations {
    fn start_crafting_deadline(&self, id: contract::CraftingTakeId) {
        let bot = self.bot.clone();
        let first =
            tokio::time::Instant::now() + bot.session.limits.inventory_click_receipt_timeout;
        self.bot.session.spawn_finite(Box::pin(async move {
            let second = bot.session.limits.crafting_take_resync_timeout;
            let mut deadline = first;
            let mut resynced = false;
            loop {
                tokio::time::sleep_until(
                    deadline.min(tokio::time::Instant::now() + Duration::from_millis(20)),
                )
                .await;
                {
                    let mut state = bot.session.state.lock().await;
                    context_received(&mut state);
                    if !state.common_crafting_take.as_ref().is_some_and(|r| {
                        r.id == id && r.stage == contract::CraftingTakeStage::Pending
                    }) {
                        break;
                    }
                    if tokio::time::Instant::now() < deadline {
                        continue;
                    }
                    if resynced {
                        state
                            .common_crafting_take
                            .as_mut()
                            .expect("retained")
                            .inspect_as(
                                contract::CraftingTakeInspectionReason::ReceiptDeadlineAfterResync,
                                "crafting receipt deadline after no-effect full resync",
                            );
                        publish(&mut state);
                        break;
                    }
                }
                match tokio::time::timeout(second, bot.operations().crafting_deadline_resync(id))
                    .await
                {
                    Ok(Ok(())) => {}
                    _ => {
                        let mut state = bot.session.state.lock().await;
                        if let Some(r) = state.common_crafting_take.as_mut().filter(|r| {
                            r.id == id && r.stage == contract::CraftingTakeStage::Pending
                        }) {
                            r.inspect_as(
                                contract::CraftingTakeInspectionReason::ResyncDeliveryUncertain,
                                "crafting no-effect full resync delivery uncertain",
                            );
                        }
                        publish(&mut state);
                        break;
                    }
                }
                resynced = true;
                deadline = tokio::time::Instant::now() + second;
            }
        }));
    }
    async fn crafting_deadline_resync(&self, id: contract::CraftingTakeId) -> Result<()> {
        let mut state = self.bot.session.state.lock().await;
        context_received(&mut state);
        let Some(record) = state
            .common_crafting_take
            .as_ref()
            .filter(|r| r.id == id && r.stage == contract::CraftingTakeStage::Pending)
        else {
            return Ok(());
        };
        let payload = super::recipe_placement::resync_payload(record.window_id());
        // Slot -1/revision -1 is a no-effect full-content request, not another result take.
        self.bot
            .session
            .send(ids::play_serverbound::WINDOW_CLICK, &payload)
            .await?;
        state
            .common_crafting_take
            .as_mut()
            .expect("retained")
            .deadline_resync_dispatched = true;
        publish(&mut state);
        Ok(())
    }
    async fn crafting_send_owned(
        &self,
        id: contract::CraftingTakeId,
        execution: crate::client::inventory::InventorySendId,
        submission: Option<Arc<take::Submission>>,
        operational: bool,
    ) -> Result<(
        CraftingTakeRecord,
        crate::client::inventory::InventorySendRecord,
    )> {
        let mut state = self.bot.session.state.lock().await;
        let result = async {
            self.ready(&state)?;
            let record = state
                .common_crafting_take
                .as_ref()
                .filter(|r| r.id == id)
                .cloned()
                .ok_or_else(|| unavailable("crafting intent superseded"))?;
            if record.requires_inspection.is_some() {
                return Err(unavailable("crafting interrupted before I/O"));
            }
            let current = self.common_player_unlocked(&state)?;
            let table = state.operations.inventory.screen(current.session);
            let registries = state
                .registries
                .capture(current.session, current.receive_sequence);
            take::validate_before(
                &record,
                &current,
                &take::screen_observation(&current, table),
                &registries,
            )?;
            let inventory = &state.operations.inventory;
            if inventory.pending_swap.is_some()
                || inventory.unsupported_components
                || !inventory.pending_creative.is_empty()
            {
                return Err(unavailable("crafting native ownership changed before I/O"));
            }
            state
                .common_crafting_take
                .as_mut()
                .expect("retained")
                .send
                .after_sequence = current.receive_sequence;
            publish(&mut state);
            if let Some(submission) = &submission {
                submission.retain(state.common_crafting_take.as_ref().expect("retained"));
            }
            state
                .inventory_executor
                .refresh_boundary(execution, current.receive_sequence)?;
            let basis_screen = state.operations.inventory.screen(current.session);
            state.inventory_executor.comparison(
                crate::client::inventory::outcome::Comparison::crafting(
                    execution,
                    &record,
                    &current,
                    basis_screen,
                    operational,
                ),
            );
            self.bot
                .session
                .send_with_attempt(
                    ids::play_serverbound::WINDOW_CLICK,
                    &payload(&record)?,
                    submission.as_ref().map(|s| s.write_started.as_ref()),
                )
                .await?;
            let record = state.common_crafting_take.as_mut().expect("retained");
            record.send.dispatched = true;
            self.start_crafting_deadline(id);
            let result = record.clone();
            self.send_inventory_fence(&mut state, execution).await?;
            Ok((
                result,
                state
                    .inventory_executor
                    .record()
                    .expect("retained original send"),
            ))
        }
        .await;
        if let Err(e) = &result {
            state.inventory_executor.invalidate_owned(execution);
            if let Some(record) = state.common_crafting_take.as_mut().filter(|r| r.id == id) {
                record.inspection(format!("crafting submission uncertain: {e}"));
                if let Some(submission) = &submission {
                    if record.id == id {
                        submission.retain(record);
                    }
                }
            }
        }
        publish(&mut state);
        result
    }
}

impl crate::client::adapter::CraftingTakeOps for Operations {
    async fn take_crafting_result_with_submission(
        &self,
        mode: api::GameMode,
        grid: &contract::ReceivedCrafting,
        destination: CraftingResultDestination,
        submission: Option<Arc<take::Submission>>,
    ) -> Result<CraftingTakeRecord> {
        self.start_crafting_take(mode, grid, destination, submission, false, false)
            .await
            .map(|(record, _)| record)
    }
    async fn crafting_take_record(&self) -> Result<Option<CraftingTakeRecord>> {
        if let Ok(mut state) = self.bot.session.state.try_lock() {
            context_received(&mut state);
        }
        let mut history = self
            .bot
            .crafting_take_history
            .lock()
            .expect("crafting history");
        if self.bot.session.stopped.load(Ordering::Acquire) {
            if let Some(record) = history.as_mut() {
                record.inspection("crafting connection closed or uncertain");
            }
        }
        Ok(history.clone())
    }
}

impl Operations {
    pub(super) async fn start_crafting_take(
        &self,
        mode: api::GameMode,
        grid: &contract::ReceivedCrafting,
        destination: CraftingResultDestination,
        submission: Option<Arc<take::Submission>>,
        acceptance: bool,
        operational: bool,
    ) -> Result<(
        CraftingTakeRecord,
        crate::client::inventory::InventorySendRecord,
    )> {
        self.bot
            .session
            .ledger
            .try_work()?
            .run_scoped(async {
                let mut state = self.bot.session.state.lock().await;
                self.mutable_in_domain(
                    &state,
                    true,
                    false,
                    None,
                    if acceptance {
                        api::domain::Domain::CurrentInventory
                    } else {
                        api::domain::Domain::Strict
                    },
                )?;
                if let CraftingSource::Table { screen } = grid.source() {
                    if state
                        .common_container_close
                        .as_ref()
                        .is_some_and(|r| r.id.screen() == screen)
                    {
                        return Err(unavailable("old table opening has retained close intent"));
                    }
                }
                let inventory = &state.operations.inventory;
                if inventory.pending_swap.is_some()
                    || inventory.unsupported_components
                    || !inventory.pending_creative.is_empty()
                {
                    return Err(unavailable("native inventory mutation/data unresolved"));
                }
                let initial = self.common_player_unlocked(&state)?;
                let table = inventory.screen(initial.session);
                let registries = state
                    .registries
                    .capture(initial.session, initial.receive_sequence);
                let screen = take::screen_observation(&initial, table);
                let attempt = state
                    .common_crafting_take
                    .as_ref()
                    .map_or(Some(1), |r| r.id.attempt().checked_add(1))
                    .ok_or_else(|| unavailable("crafting take attempts exhausted"))?;
                let record = match destination {
                    CraftingResultDestination::Cursor => {
                        take::prepare(initial, screen, registries, grid, mode, attempt)?
                    }
                    CraftingResultDestination::Inventory => {
                        take::prepare_transfer(initial, screen, registries, grid, mode, attempt)?
                    }
                };
                let id = record.id;
                let execution =
                    self.begin_inventory_execution(&mut state, record.initial.session, false)?;
                if let Some(submission) = &submission {
                    submission.retain(&record);
                }
                let work = state.finite_work.begin(
                    &self.bot.session.ledger,
                    crate::client::ledger::finite::Family::Crafting,
                    record.id.attempt(),
                );
                state.common_crafting_take = Some(record);
                publish(&mut state);
                let bot = self.bot.clone();
                let (reply, result) = tokio::sync::oneshot::channel();
                self.bot.session.spawn_finite_work(work, async move {
                    let _ = reply.send(crate::client::ledger::finite::checked(
                        bot.operations()
                            .crafting_send_owned(id, execution, submission, operational)
                            .await,
                    ));
                });
                drop(state);
                result
                    .await
                    .map_err(|_| unavailable("crafting owner result unavailable"))?
            })
            .await
    }
}
