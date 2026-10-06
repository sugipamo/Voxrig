//! Recipe placement owns the legacy normal-dispatch actor until actual conservation.
use super::*;
use crate::client::{
    self as api,
    crafting::{self as contract, RecipePlacementRecord, RecipePlacementStage, dispatch},
    inventory::unavailable,
};
#[derive(Clone)]
pub(super) struct NativeRecipePlacement {
    pub(super) record: RecipePlacementRecord,
    pub(super) released: bool,
}
impl Bot {
    async fn recipe_placement_capture(&self) -> Result<contract::ReceivedCraftingContext> {
        let player = self.common_player_unlocked().await?;
        let receipts = self.common_receipts.lock().await;
        let table = receipts
            .container
            .as_ref()
            .map(|s| s.capture(player.session));
        let registries = receipts
            .registries
            .capture(player.session, player.receive_sequence);
        let recipes = receipts.recipes.capture(
            player.session,
            player.receive_sequence,
            registries.clone(),
        )?;
        contract::ReceivedCraftingContext::capture(
            player.clone(),
            contract::take::screen_observation(&player, table),
            registries,
            recipes,
        )?
        .ok_or_else(|| unavailable("actual player/table crafting UI required"))
    }
    async fn recipe_placement_cache(&self, record: &RecipePlacementRecord) -> Result<()> {
        let inventory = self.inventory.read().await;
        if !inventory.pending_clicks.is_empty()
            || inventory
                .open_window
                .as_ref()
                .map_or(0, |s| i32::from(s.id))
                != record.window_id()
            || api::legacy_slot(inventory.cursor.as_ref())? != api::SlotKnowledge::Empty
        {
            return Err(unavailable(
                "recipe placement received/native ownership disagrees",
            ));
        }
        Ok(())
    }
    pub(crate) async fn common_place_recipe(
        &self,
        mode: api::GameMode,
        plan: &contract::RecipePlacementPlan,
    ) -> Result<RecipePlacementRecord> {
        let gate = self.coherent_state_gate.lock().await;
        self.common_motion_admission().await?;
        if let contract::CraftingSource::Table { screen } = plan.layout().source() {
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
            .map_err(|e| unavailable(format!("recipe placement admission: {e:?}")))?;
        let current = self.recipe_placement_capture().await?;
        let attempt = self
            .common_recipe_placement
            .lock()
            .await
            .as_ref()
            .map_or(Some(1), |r| r.record.id.attempt().checked_add(1))
            .ok_or_else(|| unavailable("recipe placement attempts exhausted"))?;
        let record = dispatch::prepare(plan, &current, mode, attempt)?;
        self.recipe_placement_cache(&record).await?;
        dispatch::payload(&record)?;
        let id = record.id;
        *self.common_recipe_placement.lock().await = Some(NativeRecipePlacement {
            record,
            released: false,
        });
        let (reply, result) = tokio::sync::oneshot::channel();
        let bot = self.clone_internal();
        tokio::spawn(async move {
            let _ = reply.send(bot.recipe_placement_send_owned(id, revision).await);
        });
        drop(gate);
        result
            .await
            .map_err(|_| unavailable("recipe placement owner result unavailable"))?
    }
    async fn recipe_placement_send_owned(
        &self,
        id: contract::RecipePlacementId,
        revision: u64,
    ) -> Result<RecipePlacementRecord> {
        let _gate = self.coherent_state_gate.lock().await;
        let result = async {
            let record = self
                .common_recipe_placement
                .lock()
                .await
                .as_ref()
                .filter(|r| r.record.id == id)
                .map(|r| r.record.clone())
                .ok_or_else(|| unavailable("recipe placement intent superseded"))?;
            if record.requires_inspection.is_some() {
                return Err(unavailable("recipe placement interrupted before I/O"));
            }
            let current = self.recipe_placement_capture().await?;
            dispatch::validate_before(&record, &current)?;
            self.recipe_placement_cache(&record).await?;
            self.common_recipe_placement
                .lock()
                .await
                .as_mut()
                .expect("retained")
                .record
                .send
                .after_sequence = current.receive_sequence();
            self.connection
                .bounded_recipe_placement(id.attempt(), revision, dispatch::payload(&record)?)
                .await?;
            let mut guard = self.common_recipe_placement.lock().await;
            let record = &mut guard.as_mut().expect("retained").record;
            record.send.dispatched = true;
            Ok(record.clone())
        }
        .await;
        if let Err(e) = &result {
            self.interrupt_common_recipe_placement(format!(
                "recipe placement submission uncertain: {e}"
            ))
            .await;
        }
        result
    }
    pub(super) async fn interrupt_common_recipe_placement(&self, reason: impl std::fmt::Display) {
        if let Some(r) = self.common_recipe_placement.lock().await.as_mut() {
            r.record.inspection(reason);
        }
    }
    pub(crate) async fn common_recipe_placement_record(
        &self,
    ) -> Result<Option<RecipePlacementRecord>> {
        if let Ok(gate) = self.coherent_state_gate.clone().try_lock_owned() {
            let bot = self.clone_internal();
            let (reply, result) = tokio::sync::oneshot::channel();
            tokio::spawn(async move {
                let _gate = gate;
                let _ = reply.send(bot.common_recipe_placement_context_received().await);
            });
            result
                .await
                .map_err(|_| unavailable("recipe placement inspection owner unavailable"))??;
        }
        if self.is_stopped() {
            self.interrupt_common_recipe_placement(
                "recipe placement connection closed or uncertain",
            )
            .await;
        }
        Ok(self
            .common_recipe_placement
            .lock()
            .await
            .as_ref()
            .map(|r| r.record.clone()))
    }
    pub(super) async fn common_recipe_placement_context_received(&self) -> Result<()> {
        let Some(snapshot) = self.common_recipe_placement.lock().await.clone() else {
            return Ok(());
        };
        if !snapshot.record.unresolved() {
            return Ok(());
        }
        if self.is_stopped() || self.connection_state() != ConnectionState::Ready {
            self.interrupt_common_recipe_placement(
                "recipe placement connection closed or uncertain",
            )
            .await;
            return Ok(());
        }
        let current = self.recipe_placement_capture().await;
        let cache = self.recipe_placement_cache(&snapshot.record).await;
        let complete = {
            let mut guard = self.common_recipe_placement.lock().await;
            let record = &mut guard.as_mut().expect("retained").record;
            match current {
                Ok(current) => dispatch::receive(record, &current),
                Err(e) => record.inspection(e),
            }
            if let Err(e) = cache {
                record.inspection(e);
            }
            record.ready()
        };
        if complete {
            match self
                .connection
                .finish_recipe_placement(snapshot.record.id.attempt())
                .await
            {
                Ok(()) => {
                    let mut guard = self.common_recipe_placement.lock().await;
                    let r = guard.as_mut().expect("retained");
                    r.released = true;
                    r.record.stage = RecipePlacementStage::ObservedPlaced;
                }
                Err(e) => {
                    self.interrupt_common_recipe_placement(format!(
                        "recipe placement release uncertain: {e:?}"
                    ))
                    .await
                }
            }
        }
        Ok(())
    }
}
