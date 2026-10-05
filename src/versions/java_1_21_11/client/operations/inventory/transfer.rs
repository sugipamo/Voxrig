//! One ordinary QUICK_MOVE; predictions are never applied to received inventory.
use super::*;
use crate::client::{
    self as api,
    inventory::{self as contract, transfer},
};
use contract::{InventorySource, InventoryTransferRecord, InventoryTransferStage};
pub(in crate::versions::java_1_21_11::client) fn context_received(state: &mut State) {
    let Some(mut record) = state.common_inventory_transfer.take() else {
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
        let registries = state
            .registries
            .capture(current.session, current.receive_sequence);
        transfer::receive_with_registries(&mut record, &current, screen.as_ref(), &registries);
        if inventory.pending_swap.is_some()
            || inventory.unsupported_components
            || !inventory.pending_creative.is_empty()
        {
            record.inspection("native transfer ownership/item data context changed");
        }
        if !state.ready || !matches!(state.phase, Phase::Play) || state.failure.is_some() {
            record.inspection("transfer session closed or unavailable");
        }
    }
    state.common_inventory_transfer = Some(record);
}
pub(super) fn payload(record: &InventoryTransferRecord) -> Result<Vec<u8>> {
    let revision = record
        .send
        .sent_screen_revision
        .ok_or_else(|| contract::unavailable("actual transfer screen revision unavailable"))?;
    let mut payload = Vec::new();
    put_varint(&mut payload, record.window_id());
    put_varint(&mut payload, revision);
    payload.extend((record.source_slot as i16).to_be_bytes());
    payload.extend([0, 1, 0]); // button, QUICK_MOVE, empty changed map
    // Leave native remote slots at their received predecessors with an empty changed map.
    // QUICK_MOVE preserves the received cursor. For data-bearing cursors use
    // an explicit different revision to obtain real full contents/cursor; this
    // Empty comparison marker is neither a cursor value nor a native data hash.
    if record.send.request_full_resync {
        put_default_cursor_hash(&mut payload, &InventorySlot::Empty)?;
        return Ok(payload);
    }
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
    pub(crate) async fn common_transfer_inventory(
        &self,
        mode: api::GameMode,
        source: InventorySource,
        slot: u16,
    ) -> Result<InventoryTransferRecord> {
        let mut state = self.bot.session.state.lock().await;
        self.mutable(&state)?;
        if let InventorySource::Container { screen } = source {
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
            .common_inventory_transfer
            .as_ref()
            .map_or(Some(1), |s| s.id.attempt().checked_add(1))
            .ok_or_else(|| contract::unavailable("inventory transfer attempts exhausted"))?;
        let registries = state
            .registries
            .capture(initial.session, initial.receive_sequence);
        let mut record =
            transfer::prepare_received((initial, registries), mode, source, slot, attempt, screen)?;
        if matches!(&record.cursor_before.value, api::SlotKnowledge::Item { item } if item.data != api::ItemData::Default)
        {
            record.send.request_full_resync = true;
            record.send.sent_screen_revision = record
                .send
                .screen_revision
                .map(|v| if v == 0 { 1 } else { 0 });
        }
        let bytes = payload(&record)?;
        state.common_inventory_transfer = Some(record);
        let result = self
            .bot
            .session
            .send(ids::play_serverbound::WINDOW_CLICK, &bytes)
            .await;
        let record = state.common_inventory_transfer.as_mut().expect("retained");
        match result {
            Ok(()) => {
                record.send.dispatched = true;
                Ok(record.clone())
            }
            Err(e) => {
                record.inspection(format!("transfer submission uncertain: {e}"));
                Err(e)
            }
        }
    }
    pub(crate) async fn common_inventory_transfer_record(
        &self,
    ) -> Result<Option<InventoryTransferRecord>> {
        let mut state = self.bot.session.state.lock().await;
        context_received(&mut state);
        if self.bot.session.stopped.load(Ordering::Acquire) || state.failure.is_some() {
            if let Some(record) = state.common_inventory_transfer.as_mut() {
                record.inspection("transfer connection closed or uncertain");
            }
        }
        if let Some(record) = state.common_inventory_transfer.as_mut() {
            if record.ready() {
                record.stage = InventoryTransferStage::ObservedTransferred;
            }
        }
        Ok(state.common_inventory_transfer.clone())
    }
}
