//! One ordinary PICKUP; predictions are never applied to received inventory.
use super::*;
use crate::client::{
    self as api,
    inventory::{self as contract, click},
};
use contract::{
    InventoryClickButton, InventoryClickRecord, InventoryClickSource, InventoryClickStage,
};
pub(in crate::versions::java_1_21_11::client) fn context_received(state: &mut State) {
    let Some(mut record) = state.common_inventory_click.take() else {
        return;
    };
    if record.unresolved() {
        let inventory = &state.operations.inventory;
        let slots = inventory
            .slots
            .iter()
            .zip(&inventory.slot_sequences)
            .map(|(v, s)| {
                s.and_then(|s| {
                    super::super::common_slot(v)
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
        let mut current = record.initial.clone();
        current.session.world_generation = state.loading.generation;
        current.game_mode = state.operations.game_mode;
        current.receive_sequence = state.sequence;
        current.selected_hotbar =
            state
                .operations
                .selected_hotbar
                .as_ref()
                .map(|s| api::ObservedValue {
                    value: s.slot,
                    source: if s.from_server {
                        api::ValueSource::Received {
                            sequence: s.sequence,
                        }
                    } else {
                        api::ValueSource::Submitted
                    },
                });
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
        click::receive(&mut record, &current, screen.as_ref());
        if inventory.pending_swap.is_some()
            || inventory.unsupported_components
            || !inventory.pending_creative.is_empty()
        {
            record.inspection("native click ownership/item data context changed");
        }
        if !state.ready || !matches!(state.phase, Phase::Play) || state.failure.is_some() {
            record.inspection("click session closed or unavailable");
        }
    }
    state.common_inventory_click = Some(record);
}
pub(super) fn payload(record: &InventoryClickRecord) -> Result<Vec<u8>> {
    let revision = record
        .send
        .screen_revision
        .ok_or_else(|| contract::unavailable("actual click screen revision unavailable"))?;
    let mut payload = Vec::new();
    put_varint(&mut payload, record.window_id());
    put_varint(&mut payload, revision);
    payload.extend((record.source_slot as i16).to_be_bytes());
    payload.extend([record.button.native(), 0, 0]); // button, PICKUP, empty changed map
    // Send actual cursor predecessor, not predicted after-click hash. This
    // retains native before values, forcing actual changed source/cursor receipts.
    let cursor = match &record.cursor_before.value {
        api::SlotKnowledge::Empty => InventorySlot::Empty,
        api::SlotKnowledge::Item { item } => InventorySlot::Item {
            item: PlainItem {
                name: item.name.clone(),
                item_id: item.id.value(),
                count: item.count as i32,
            },
        },
        api::SlotKnowledge::Unavailable => {
            return Err(contract::unavailable("actual cursor unavailable"));
        }
    };
    put_default_cursor_hash(&mut payload, &cursor)?;
    Ok(payload)
}
impl Operations {
    pub(crate) async fn common_click_inventory(
        &self,
        mode: api::GameMode,
        source: InventoryClickSource,
        slot: u16,
        button: InventoryClickButton,
    ) -> Result<InventoryClickRecord> {
        let mut state = self.bot.session.state.lock().await;
        self.mutable(&state)?;
        if let InventoryClickSource::Container { screen } = source {
            if state
                .common_container_close
                .as_ref()
                .is_some_and(|r| r.id.screen() == screen)
            {
                return Err(contract::unavailable(
                    "old opening has a retained close intent",
                ));
            }
        }
        let inventory = &state.operations.inventory;
        if inventory.pending_swap.is_some()
            || inventory.unsupported_components
            || !inventory.pending_creative.is_empty()
        {
            return Err(contract::unavailable(
                "native inventory mutation/data unresolved",
            ));
        }
        let initial = self.common_player_unlocked(&state)?;
        let screen = inventory
            .container
            .as_ref()
            .map(|s| s.capture(initial.session));
        let attempt = state
            .common_inventory_click
            .as_ref()
            .map_or(Some(1), |s| s.id.attempt().checked_add(1))
            .ok_or_else(|| contract::unavailable("inventory click attempts exhausted"))?;
        let record = click::prepare(initial, mode, source, slot, button, attempt, screen)?;
        let bytes = payload(&record)?;
        state.common_inventory_click = Some(record);
        let result = self
            .bot
            .session
            .send(ids::play_serverbound::WINDOW_CLICK, &bytes)
            .await;
        let record = state.common_inventory_click.as_mut().expect("retained");
        match result {
            Ok(()) => {
                record.send.dispatched = true;
                Ok(record.clone())
            }
            Err(e) => {
                record.inspection(format!("click submission uncertain: {e}"));
                Err(e)
            }
        }
    }
    pub(crate) async fn common_inventory_click_record(
        &self,
    ) -> Result<Option<InventoryClickRecord>> {
        let mut state = self.bot.session.state.lock().await;
        context_received(&mut state);
        if self.bot.session.stopped.load(Ordering::Acquire) || state.failure.is_some() {
            if let Some(record) = state.common_inventory_click.as_mut() {
                record.inspection("click connection closed or uncertain");
            }
        }
        if let Some(record) = state.common_inventory_click.as_mut() {
            if record.ready() {
                record.stage = InventoryClickStage::ObservedClicked;
            }
        }
        Ok(state.common_inventory_click.clone())
    }
}
