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
    if state.operations.inventory.pending_swap.is_none() {
        if let Some(work) = state.pending_swap_work.take() {
            work.finish(Outcome::Failed(Failure::TransactionUnconfirmed));
        }
    }
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
        let registries = state
            .registries
            .capture(current.session, current.receive_sequence);
        contract::receive_with_registries(
            &mut common.record,
            &current,
            screen.as_ref(),
            &registries,
        );
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
    complete_received(state);
    state.reconcile_finite();
}

fn complete_received(state: &mut State) {
    let ready = state.common_inventory_swap.as_ref().is_some_and(|common| {
        contract::destinations_ready(&common.record)
            && common.submission.as_ref().is_none_or(|submission| {
                common.record.item_data.is_some()
                    || observed(&state.operations.inventory, submission, state.sequence).is_some()
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
        if let Some(work) = state.pending_swap_work.take() {
            work.finish(Outcome::Complete);
        }
    }
}

impl crate::client::adapter::InventorySwapOps for Operations {
    async fn swap_hotbar(
        &self,
        mode: api::GameMode,
        main: u8,
        hotbar: u8,
    ) -> Result<InventorySwapRecord> {
        self.bot
            .session
            .ledger
            .try_work()?
            .run_scoped(async {
                let mut state = self.bot.session.state.lock().await;
                self.mutable(&state)?;
                let initial = self.common_player_unlocked(&state)?;
                let attempt = state
                    .common_inventory_swap
                    .as_ref()
                    .map_or(Some(1), |s| s.record.id.attempt().checked_add(1))
                    .ok_or_else(|| contract::unavailable("inventory attempts exhausted"))?;
                let registries = state
                    .registries
                    .capture(initial.session, initial.receive_sequence);
                let record = contract::prepare_received_source(
                    (initial, registries),
                    mode,
                    contract::InventorySwapSource::PlayerMain,
                    u16::from(main),
                    hotbar,
                    attempt,
                    None,
                )?;
                let after_close = match record.initial.inventory.player_screen {
                    Some(api::container::PlayerScreenAccess::SubmittedClose { .. }) => {
                        record.send.screen_revision
                    }
                    _ => None,
                };
                let (submission, payload) = prepare_with_item_data(
                    &state.operations.inventory,
                    self.bot.session.id,
                    state.sequence,
                    main,
                    hotbar,
                    after_close,
                    true,
                )?;
                let work = self.bot.session.ledger.work();
                state.pending_swap_work = Some(PendingWork::new(work));
                state.operations.inventory.pending_swap = Some(submission.clone());
                state.common_inventory_swap = Some(CommonSwap {
                    record,
                    submission: Some(submission),
                });
                let send_work = ScopedWork::new(
                    state
                        .pending_swap_work
                        .as_ref()
                        .expect("pending swap owner")
                        .child()
                        .expect("pending swap write child"),
                );
                self.bot
                    .session
                    .send_work(
                        ids::play_serverbound::WINDOW_CLICK,
                        &payload,
                        send_work.guard(),
                    )
                    .await?;
                let common = state.common_inventory_swap.as_mut().expect("retained");
                common.record.send.dispatched = true;
                Ok(common.record.clone())
            })
            .await
    }
    async fn swap_container_hotbar(
        &self,
        mode: api::GameMode,
        screen: api::container::ScreenId,
        slot: u16,
        hotbar: u8,
    ) -> Result<InventorySwapRecord> {
        self.bot
            .session
            .ledger
            .try_work()?
            .run_scoped(async {
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
                let registries = state
                    .registries
                    .capture(initial.session, initial.receive_sequence);
                let record = contract::prepare_received_source(
                    (initial, registries),
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
                payload.extend([hotbar, 2, 0]);
                put_default_cursor_hash(&mut payload, &InventorySlot::Empty)?;
                let work = state.finite_work.begin(
                    &self.bot.session.ledger,
                    crate::client::ledger::finite::Family::Swap,
                    record.id.attempt(),
                );
                state.common_inventory_swap = Some(CommonSwap {
                    record,
                    submission: None,
                });
                work.run(
                    self.bot
                        .session
                        .send(ids::play_serverbound::WINDOW_CLICK, &payload),
                )
                .await?;
                let common = state.common_inventory_swap.as_mut().expect("retained");
                common.record.send.dispatched = true;
                Ok(common.record.clone())
            })
            .await
    }
    async fn inventory_swap_record(&self) -> Result<Option<InventorySwapRecord>> {
        let mut state = self.bot.session.state.lock().await;
        context_received(&mut state);
        if self.bot.session.stopped.load(Ordering::Acquire) || state.failure.is_some() {
            if let Some(s) = state.common_inventory_swap.as_mut() {
                contract::inspection(&mut s.record, "inventory connection closed or uncertain");
            }
        }
        complete_received(&mut state);
        state.reconcile_finite();
        Ok(state
            .common_inventory_swap
            .as_ref()
            .map(|s| s.record.clone()))
    }
}

pub(in crate::versions::java_1_21_11::client) fn swap_ledger_state(
    state: &State,
) -> Option<(u64, bool, bool)> {
    state.common_inventory_swap.as_ref().map(|s| {
        (
            s.record.id.attempt(),
            s.record.stage == InventorySwapStage::ObservedSwapped,
            s.record.requires_inspection.is_some(),
        )
    })
}
