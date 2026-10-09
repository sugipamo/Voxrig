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
                record.inspection(reason);
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
                let table = state
                    .operations
                    .inventory
                    .container
                    .as_ref()
                    .map(|s| s.capture(current.session));
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
    async fn crafting_send_owned(
        &self,
        id: contract::CraftingTakeId,
    ) -> Result<CraftingTakeRecord> {
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
            let table = state
                .operations
                .inventory
                .container
                .as_ref()
                .map(|s| s.capture(current.session));
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
            self.bot
                .session
                .send(ids::play_serverbound::WINDOW_CLICK, &payload(&record)?)
                .await?;
            let record = state.common_crafting_take.as_mut().expect("retained");
            record.send.dispatched = true;
            Ok(record.clone())
        }
        .await;
        if let Err(e) = &result {
            if let Some(record) = state.common_crafting_take.as_mut() {
                record.inspection(format!("crafting submission uncertain: {e}"));
            }
        }
        publish(&mut state);
        result
    }
}

impl crate::client::adapter::CraftingTakeOps for Operations {
    async fn take_crafting_result(
        &self,
        mode: api::GameMode,
        grid: &contract::ReceivedCrafting,
        destination: CraftingResultDestination,
    ) -> Result<CraftingTakeRecord> {
        let mut state = self.bot.session.state.lock().await;
        self.mutable(&state)?;
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
        let table = inventory
            .container
            .as_ref()
            .map(|s| s.capture(initial.session));
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
        state.common_crafting_take = Some(record);
        publish(&mut state);
        let bot = self.bot.clone();
        let (reply, result) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = reply.send(bot.operations().crafting_send_owned(id).await);
        });
        drop(state);
        result
            .await
            .map_err(|_| unavailable("crafting owner result unavailable"))?
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
