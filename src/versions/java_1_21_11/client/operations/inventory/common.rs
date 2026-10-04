//! Common exchange record retains native intent, cancellation and actual receipts.
use super::*;
use crate::client::{self as api, inventory as contract};
use contract::{InventorySwapRecord, InventorySwapStage};
#[derive(Clone)]
pub(in crate::versions::java_1_21_11::client) struct CommonSwap {
    pub(in crate::versions::java_1_21_11::client) record: InventorySwapRecord,
    submission: Option<InventorySwap>,
}
pub(in crate::versions::java_1_21_11::client) fn context_received(state: &mut State) {
    let Some(mut common) = state.common_inventory_swap.take() else {
        return;
    };
    if common.record.stage != InventorySwapStage::ObservedSwapped {
        // Pure projection under the same session lock. No independent capture or prediction.
        let inventory = &state.operations.inventory;
        let slots = inventory
            .slots
            .iter()
            .zip(&inventory.slot_sequences)
            .map(|(value, sequence)| {
                if matches!(value, InventorySlot::Unavailable) {
                    return None;
                }
                sequence.and_then(|s| {
                    super::super::common_slot(value)
                        .ok()
                        .map(|v| api::received(v, s))
                })
            })
            .collect();
        let cursor = inventory.cursor_sequence.and_then(|s| {
            super::super::common_slot(&inventory.cursor)
                .ok()
                .map(|v| api::received(v, s))
        });
        let mut current = common.record.initial.clone();
        current.session.world_generation = state.loading.generation;
        current.game_mode = state.operations.game_mode;
        current.receive_sequence = state.sequence;
        current.inventory = api::InventoryObservation {
            slots,
            cursor,
            window_id: inventory.window_id,
            player_screen: api::container::player_screen_access(
                current.session,
                inventory.window_id,
                inventory
                    .container
                    .as_ref()
                    .map(|s| s.capture(current.session).id),
                state.common_container_close.as_ref(),
            ),
            screen_revision: inventory.screen_revision,
            player_screen_revision: inventory.player_revision.clone(),
            local_cache: None,
        };
        let screen = inventory
            .container
            .as_ref()
            .map(|s| s.capture(current.session));
        contract::receive(&mut common.record, &current, screen.as_ref());
        if inventory.pending_swap.as_ref() != common.submission.as_ref()
            || inventory.unsupported_components
            || !inventory.pending_creative.is_empty()
        {
            contract::inspection(
                &mut common.record,
                "native swap ownership/item data context changed",
            );
        }
        if !state.ready || !matches!(state.phase, Phase::Play) || state.failure.is_some() {
            contract::inspection(
                &mut common.record,
                "inventory session closed or unavailable",
            );
        }
    }
    state.common_inventory_swap = Some(common);
}
impl Operations {
    pub(crate) async fn common_swap_hotbar(
        &self,
        mode: api::GameMode,
        main: u8,
        hotbar: u8,
    ) -> Result<InventorySwapRecord> {
        let mut state = self.bot.session.state.lock().await;
        self.mutable(&state)?;
        let initial = self.common_player_unlocked(&state)?;
        let attempt = state
            .common_inventory_swap
            .as_ref()
            .map_or(Some(1), |s| s.record.id.attempt().checked_add(1))
            .ok_or_else(|| contract::unavailable("inventory attempts exhausted"))?;
        let record = contract::prepare(initial, mode, main, hotbar, attempt)?;
        let after_close = match record.initial.inventory.player_screen {
            Some(api::container::PlayerScreenAccess::SubmittedClose { .. }) => {
                record.send.screen_revision
            }
            _ => None,
        };
        let (submission, payload) = prepare_with_player_revision(
            &state.operations.inventory,
            self.bot.session.id,
            state.sequence,
            main,
            hotbar,
            after_close,
        )?;
        state.operations.inventory.pending_swap = Some(submission.clone());
        state.common_inventory_swap = Some(CommonSwap {
            record,
            submission: Some(submission),
        });
        self.bot
            .session
            .send(ids::play_serverbound::WINDOW_CLICK, &payload)
            .await?;
        let common = state.common_inventory_swap.as_mut().expect("retained");
        common.record.send.dispatched = true;
        Ok(common.record.clone())
    }
    pub(crate) async fn common_swap_container_hotbar(
        &self,
        mode: api::GameMode,
        screen: api::container::ScreenId,
        slot: u16,
        hotbar: u8,
    ) -> Result<InventorySwapRecord> {
        let mut state = self.bot.session.state.lock().await;
        self.mutable(&state)?;
        if state
            .common_container_close
            .as_ref()
            .is_some_and(|r| r.id.screen() == screen)
        {
            return Err(contract::unavailable(
                "container has a retained close intent; old opening cannot be clicked",
            ));
        }
        let initial = self.common_player_unlocked(&state)?;
        if state.operations.inventory.unsupported_components
            || state.operations.inventory.pending_swap.is_some()
            || !state.operations.inventory.pending_creative.is_empty()
        {
            return Err(contract::unavailable(
                "native inventory data/mutation unresolved",
            ));
        }
        let captured = state
            .operations
            .inventory
            .container
            .as_ref()
            .map(|s| s.capture(initial.session));
        let attempt = state
            .common_inventory_swap
            .as_ref()
            .map_or(Some(1), |s| s.record.id.attempt().checked_add(1))
            .ok_or_else(|| contract::unavailable("inventory attempts exhausted"))?;
        let record = contract::prepare_source(
            initial,
            mode,
            contract::InventorySwapSource::Container { screen },
            slot,
            hotbar,
            attempt,
            captured,
        )?;
        let revision = record
            .send
            .screen_revision
            .ok_or_else(|| contract::unavailable("native screen revision unavailable"))?;
        let mut payload = Vec::new();
        put_varint(&mut payload, record.window_id());
        put_varint(&mut payload, revision);
        payload.extend((slot as i16).to_be_bytes());
        payload.extend([hotbar, 2, 0, 0]);
        state.common_inventory_swap = Some(CommonSwap {
            record,
            submission: None,
        });
        self.bot
            .session
            .send(ids::play_serverbound::WINDOW_CLICK, &payload)
            .await?;
        let common = state.common_inventory_swap.as_mut().expect("retained");
        common.record.send.dispatched = true;
        Ok(common.record.clone())
    }
    pub(crate) async fn common_inventory_swap_record(&self) -> Result<Option<InventorySwapRecord>> {
        let mut state = self.bot.session.state.lock().await;
        context_received(&mut state);
        if self.bot.session.stopped.load(Ordering::Acquire) || state.failure.is_some() {
            if let Some(s) = state.common_inventory_swap.as_mut() {
                contract::inspection(&mut s.record, "inventory connection closed or uncertain");
            }
        }
        let ready = state.common_inventory_swap.as_ref().is_some_and(|common| {
            contract::destinations_ready(&common.record)
                && common.submission.as_ref().is_none_or(|submission| {
                    observed(&state.operations.inventory, submission, state.sequence).is_some()
                        || matches!(
                            common.record.initial.inventory.player_screen,
                            Some(api::container::PlayerScreenAccess::SubmittedClose { .. })
                        )
                })
        });
        if ready {
            let common = state.common_inventory_swap.as_mut().expect("retained");
            common.record.stage = InventorySwapStage::ObservedSwapped;
            state.operations.inventory.pending_swap = None;
        }
        Ok(state
            .common_inventory_swap
            .as_ref()
            .map(|s| s.record.clone()))
    }
}
