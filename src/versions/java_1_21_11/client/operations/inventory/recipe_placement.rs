//! Owned native recipe requests with a short independently readable history lock.
use super::*;
use crate::client::{
    self as api,
    crafting::{self as contract, RecipePlacementRecord, RecipePlacementStage, dispatch},
    inventory::unavailable,
};
fn publish(state: &mut State) {
    let mut history = state
        .recipe_placement_history
        .lock()
        .expect("recipe placement history");
    if let (Some(record), Some(previous)) =
        (state.common_recipe_placement.as_mut(), history.as_ref())
    {
        if record.id == previous.id {
            if let Some(reason) = &previous.requires_inspection {
                record.inspection(reason);
            }
        }
    }
    *history = state.common_recipe_placement.clone();
}
fn capture(state: &State, connection: u64) -> Result<contract::ReceivedCraftingContext> {
    let player = super::super::common_player_in_state(state, connection, false)?;
    let table = state
        .operations
        .inventory
        .container
        .as_ref()
        .map(|s| s.capture(player.session));
    let registries = state
        .registries
        .capture(player.session, player.receive_sequence);
    let recipes =
        state
            .recipes
            .capture(player.session, player.receive_sequence, registries.clone())?;
    contract::ReceivedCraftingContext::capture(
        player.clone(),
        contract::take::screen_observation(&player, table),
        registries,
        recipes,
    )?
    .ok_or_else(|| unavailable("actual player/table crafting UI required"))
}
pub(in crate::versions::java_1_21_11::client) fn context_received(state: &mut State) {
    let Some(mut record) = state.common_recipe_placement.take() else {
        return;
    };
    if record.unresolved() {
        match capture(state, record.id.session().connection_id) {
            Ok(current) => match state
                .recipe_ghost
                .as_ref()
                .filter(|g| g.receive_sequence() > record.send.after_sequence)
                .map(|g| g.capture(record.id.session()))
                .transpose()
                .map(Option::flatten)
            {
                Ok(ghost) => dispatch::receive_with_ghost(&mut record, &current, ghost.as_ref()),
                Err(error) => record.inspection(error),
            },
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
            record
                .inspection("recipe placement native session/inventory/loading ownership changed");
        }
        if record.ready() {
            record.stage = if record.ghost.is_some() {
                RecipePlacementStage::ObservedGhost
            } else {
                RecipePlacementStage::ObservedPlaced
            };
        }
    }
    state.common_recipe_placement = Some(record);
    publish(state);
}
impl Operations {
    pub(crate) async fn common_place_recipe(
        &self,
        mode: api::GameMode,
        plan: &contract::RecipePlacementPlan,
    ) -> Result<RecipePlacementRecord> {
        let mut state = self.bot.session.state.lock().await;
        self.mutable(&state)?;
        if let contract::CraftingSource::Table { screen } = plan.layout().source() {
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
            return Err(unavailable(
                "native recipe placement inventory/data unresolved",
            ));
        }
        let current = capture(&state, self.bot.session.id)?;
        let attempt = state
            .common_recipe_placement
            .as_ref()
            .map_or(Some(1), |r| r.id.attempt().checked_add(1))
            .ok_or_else(|| unavailable("recipe placement attempts exhausted"))?;
        dispatch::validate_plan_history(plan, state.common_recipe_placement.as_ref())?;
        let record = dispatch::prepare(plan, &current, mode, attempt)?;
        dispatch::payload(&record)?;
        let id = record.id;
        state.common_recipe_placement = Some(record);
        publish(&mut state);
        let bot = self.bot.clone();
        let (reply, result) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = reply.send(bot.operations().recipe_placement_send_owned(id).await);
        });
        drop(state);
        result
            .await
            .map_err(|_| unavailable("recipe placement owner result unavailable"))?
    }
    async fn recipe_placement_send_owned(
        &self,
        id: contract::RecipePlacementId,
    ) -> Result<RecipePlacementRecord> {
        let mut state = self.bot.session.state.lock().await;
        let result = async {
            self.ready(&state)?;
            let record = state
                .common_recipe_placement
                .as_ref()
                .filter(|r| r.id == id)
                .cloned()
                .ok_or_else(|| unavailable("recipe placement intent superseded"))?;
            if record.requires_inspection.is_some() {
                return Err(unavailable("recipe placement interrupted before I/O"));
            }
            let current = capture(&state, id.session().connection_id)?;
            dispatch::validate_before(&record, &current)?;
            let inventory = &state.operations.inventory;
            if inventory.pending_swap.is_some()
                || inventory.unsupported_components
                || !inventory.pending_creative.is_empty()
            {
                return Err(unavailable(
                    "recipe placement native ownership changed before I/O",
                ));
            }
            state
                .common_recipe_placement
                .as_mut()
                .expect("retained")
                .send
                .after_sequence = current.receive_sequence();
            publish(&mut state);
            self.bot
                .session
                .send(
                    ids::play_serverbound::CRAFT_RECIPE_REQUEST,
                    &dispatch::payload(&record)?,
                )
                .await?;
            let record = state.common_recipe_placement.as_mut().expect("retained");
            record.send.dispatched = true;
            Ok(record.clone())
        }
        .await;
        if let Err(e) = &result {
            if let Some(record) = state.common_recipe_placement.as_mut() {
                record.inspection(format!("recipe placement submission uncertain: {e}"));
            }
        }
        publish(&mut state);
        result
    }
    pub(crate) async fn common_recipe_placement_record(
        &self,
    ) -> Result<Option<RecipePlacementRecord>> {
        if let Ok(mut state) = self.bot.session.state.try_lock() {
            context_received(&mut state);
        }
        let mut history = self
            .bot
            .recipe_placement_history
            .lock()
            .expect("recipe placement history");
        if self.bot.session.stopped.load(Ordering::Acquire) {
            if let Some(record) = history.as_mut() {
                record.inspection("recipe placement connection closed or uncertain");
            }
        }
        Ok(history.clone())
    }
}
