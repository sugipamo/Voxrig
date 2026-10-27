//! Version-owned PLAYER_ACTION DROP_ITEM / DROP_ALL_ITEMS, separate from clicks.
use super::*;
use crate::client::{
    self as api,
    inventory::{SelectedDropAmount, SelectedDropRecord, drop as contract},
};

pub(in crate::versions::java_1_21_11::client) fn context_received(state: &mut State) {
    let Some(mut record) = state.common_selected_drop.take() else {
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
            record.inspection("selected drop session or native inventory ownership unavailable");
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
    state.common_selected_drop = Some(record);
}

pub(super) async fn drop_selected(
    ops: &Operations,
    mode: api::GameMode,
    amount: SelectedDropAmount,
) -> Result<SelectedDropRecord> {
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
                return Err(unavailable(
                    "selected drop requires resolved native inventory",
                ));
            }
            let initial = ops.common_player_unlocked(&state)?;
            let registries = state
                .registries
                .capture(initial.session, initial.receive_sequence);
            let attempt = state
                .common_selected_drop
                .as_ref()
                .map_or(Some(1), |r| r.id.attempt().checked_add(1))
                .ok_or_else(|| unavailable("selected drop attempts exhausted"))?;
            // Validate first, then allocate from the same sequence counter as dig/use/place.
            let mut record = contract::prepare(initial, registries, mode, attempt, 0, amount)?;
            record.send.interaction_sequence = ops.next_sequence()?;
            let bytes = payload(amount, record.send.interaction_sequence);
            let family = crate::client::ledger::finite::Family::SelectedDrop;
            let work = state
                .finite_work
                .begin(&ops.bot.session.ledger, family, attempt);
            // Bounded like the inventory executor: an unanswered drop is unknown,
            // not refused, so the retained owner fails the connection at expiry.
            let deadline = tokio::time::Instant::now() + RESPONSE_DEADLINE;
            record.deadline = Some(deadline);
            state.common_selected_drop = Some(record);
            state.finite_work.expire_at(family, attempt, deadline);
            let result: Result<()> = work
                .run(async {
                    ops.bot
                        .session
                        .send(ids::play_serverbound::BLOCK_DIG, &bytes)
                        .await?;
                    state
                        .common_selected_drop
                        .as_mut()
                        .expect("retained selected drop")
                        .send
                        .dispatched = true;
                    // Native drop marks the reduced slot as already known by the client.
                    // An ignored PICKUP at -1 with an impossible native revision asks
                    // for actual full contents without moving a slot or held cursor.
                    ops.bot
                        .session
                        .send(ids::play_serverbound::WINDOW_CLICK, &RESYNC_PAYLOAD)
                        .await?;
                    state
                        .common_selected_drop
                        .as_mut()
                        .expect("retained selected drop")
                        .send
                        .resync_dispatched = true;
                    Ok(())
                })
                .await;
            let record = state
                .common_selected_drop
                .as_mut()
                .expect("retained selected drop");
            match result {
                Ok(()) => Ok(record.clone()),
                Err(error) => {
                    record.inspection(format!("selected drop submission uncertain: {error}"));
                    Err(error)
                }
            }
        })
        .await
}

/// Response deadline for a drop or hand swap and its resync, as the inventory executor.
pub(super) const RESPONSE_DEADLINE: Duration = Duration::from_secs(5);

// Player screen 0; revision -1 (native revisions are 0..32767); slot -1;
// left PICKUP; no changed slots; empty cursor hash as resync marker.
pub(super) const RESYNC_PAYLOAD: [u8; 12] = [0, 255, 255, 255, 255, 15, 255, 255, 0, 0, 0, 0];

fn payload(amount: SelectedDropAmount, sequence: i32) -> Vec<u8> {
    // Native enum ordinals: DROP_ITEM=4, DROP_ALL_ITEMS=3.
    const ACTIONS: [u8; 2] = [4, 3];
    let index = match amount {
        SelectedDropAmount::One => 0,
        SelectedDropAmount::All => 1,
    };
    let mut bytes = vec![ACTIONS[index]];
    bytes.extend(0u64.to_be_bytes()); // BlockPos.ZERO
    bytes.push(0); // DOWN
    put_varint(&mut bytes, sequence);
    bytes
}

pub(super) async fn record(ops: &Operations) -> Result<Option<SelectedDropRecord>> {
    let mut state = ops.bot.session.state.lock().await;
    context_received(&mut state);
    if ops.bot.session.stopped.load(Ordering::Acquire) || state.failure.is_some() {
        if let Some(record) = state.common_selected_drop.as_mut() {
            record.inspection("selected drop connection closed or uncertain");
        }
    }
    state.reconcile_finite();
    Ok(state.common_selected_drop.clone())
}

#[cfg(test)]
mod tests {
    #[test]
    fn selected_drop_payload_matches_original_native_codec() {
        let rows: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../../data/client_api/selected_drop_packets-1.21.11.json"
        ))
        .unwrap();
        let refresh: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../../data/client_api/selected_drop_resync-1.21.11.json"
        ))
        .unwrap();
        assert_eq!(hex::encode(super::RESYNC_PAYLOAD), refresh["payload_hex"]);
        for row in rows.as_array().unwrap() {
            let sequence = row["sequence"].as_i64().unwrap().try_into().unwrap();
            let amount = if row["amount"] == "one" {
                super::SelectedDropAmount::One
            } else {
                super::SelectedDropAmount::All
            };
            assert_eq!(
                hex::encode(super::payload(amount, sequence)),
                row["payload_hex"]
            );
        }
    }
}
