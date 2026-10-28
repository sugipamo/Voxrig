//! PLAYER_ACTION SWAP_ITEM_WITH_OFFHAND, independent of container clicks.
use super::*;
use crate::client::{
    self as api,
    inventory::{HandSwapRecord, hands as contract},
};

pub(in crate::versions::java_1_21_11::client) fn context_received(state: &mut State) {
    let Some(mut record) = state.common_hand_swap.take() else {
        return;
    };
    if record.unresolved() {
        if !state.ready
            || state.phase != Phase::Play
            || state.failure.is_some()
            || state.operations.inventory.unsupported_components
            || state.operations.inventory.pending_swap.is_some()
            || !state.operations.inventory.pending_creative.is_empty()
        {
            record.inspection("hand swap session or native inventory ownership unavailable");
        }
        match super::super::common_player_in_state(state, record.id.session().connection_id, false)
        {
            Ok(current) => {
                let registries = state
                    .registries
                    .capture(current.session, current.receive_sequence);
                contract::receive(&mut record, &current, &registries);
            }
            Err(error) => record.inspection(error),
        }
    }
    state.common_hand_swap = Some(record);
}

pub(super) async fn swap(ops: &Operations, mode: api::GameMode) -> Result<HandSwapRecord> {
    ops.bot
        .session
        .ledger
        .try_work()?
        .run_scoped(async {
            let mut state = ops.bot.session.state.lock().await;
            ops.mutable(&state)?;
            let inventory = &state.operations.inventory;
            if inventory.unsupported_components
                || inventory.pending_swap.is_some()
                || !inventory.pending_creative.is_empty()
            {
                return Err(unavailable("hand swap requires resolved native inventory"));
            }
            let initial = ops.common_player_unlocked(&state)?;
            let registries = state
                .registries
                .capture(initial.session, initial.receive_sequence);
            let attempt = state
                .common_hand_swap
                .as_ref()
                .map_or(Some(1), |r| r.id.attempt().checked_add(1))
                .ok_or_else(|| unavailable("hand swap attempts exhausted"))?;
            // Validate first, then allocate from the same sequence counter as dig/use/place.
            let mut record = contract::prepare(initial, registries, mode, attempt, 0)?;
            record.send.interaction_sequence = ops.next_sequence()?;
            let bytes = payload(record.send.interaction_sequence);
            let family = crate::client::ledger::finite::Family::HandSwap;
            let work = state
                .finite_work
                .begin(&ops.bot.session.ledger, family, attempt);
            let deadline = tokio::time::Instant::now() + super::drop::RESPONSE_DEADLINE;
            record.deadline = Some(deadline);
            state.common_hand_swap = Some(record);
            state.finite_work.expire_at(family, attempt, deadline);
            let result: Result<()> = work
                .run(async {
                    ops.bot
                        .session
                        .send(ids::play_serverbound::BLOCK_DIG, &bytes)
                        .await?;
                    state
                        .common_hand_swap
                        .as_mut()
                        .expect("retained hand swap")
                        .send
                        .dispatched = true;
                    // A refused exchange (spectator, dead or unloaded player) sends no
                    // slot update. The same ignored stale-revision PICKUP as selected
                    // drop asks for actual full contents after the exchange is handled.
                    ops.bot
                        .session
                        .send(
                            ids::play_serverbound::WINDOW_CLICK,
                            &super::drop::RESYNC_PAYLOAD,
                        )
                        .await?;
                    state
                        .common_hand_swap
                        .as_mut()
                        .expect("retained hand swap")
                        .resync_dispatched = true;
                    Ok(())
                })
                .await;
            let record = state.common_hand_swap.as_mut().expect("retained hand swap");
            match result {
                Ok(()) => Ok(record.clone()),
                Err(error) => {
                    record.inspection(format!("hand swap submission uncertain: {error}"));
                    Err(error)
                }
            }
        })
        .await
}

fn payload(sequence: i32) -> Vec<u8> {
    let mut bytes = vec![6]; // SWAP_ITEM_WITH_OFFHAND
    bytes.extend(0u64.to_be_bytes()); // BlockPos.ZERO
    bytes.push(0); // DOWN
    put_varint(&mut bytes, sequence);
    bytes
}

pub(super) async fn record(ops: &Operations) -> Result<Option<HandSwapRecord>> {
    let mut state = ops.bot.session.state.lock().await;
    context_received(&mut state);
    if ops.bot.session.stopped.load(Ordering::Acquire) || state.failure.is_some() {
        if let Some(record) = state.common_hand_swap.as_mut() {
            record.inspection("hand swap connection closed or uncertain");
        }
    }
    state.reconcile_finite();
    Ok(state.common_hand_swap.clone())
}

#[cfg(test)]
mod tests {
    #[test]
    fn hand_swap_payload_matches_original_native_codec() {
        let rows: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../../data/client_api/hand_swap_packets-1.21.11.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let sequence = row["sequence"].as_i64().unwrap().try_into().unwrap();
            assert_eq!(hex::encode(super::payload(sequence)), row["payload_hex"]);
        }
    }
}
