//! Common Creative commands retained before spawning their one-shot writer owner.
use super::*;
use crate::client::{
    self as api,
    flight::{FlightCommand, FlightRecord, FlightStage},
    inventory::unavailable,
};
impl Operations {
    fn flight_snapshot(&self, attempt: u64) -> Result<FlightRecord> {
        self.bot
            .flight_history
            .lock()
            .expect("flight history")
            .as_ref()
            .filter(|r| r.attempt == attempt)
            .cloned()
            .ok_or_else(|| unavailable("flight intent superseded"))
    }
    pub(super) async fn common_flight(&self, command: FlightCommand) -> Result<FlightRecord> {
        let state = self.bot.session.state.lock().await;
        self.mutable(&state)?;
        if state.operations.inventory.pending_swap.is_some()
            || state.operations.local_player.motion_interruption.is_some()
            || state.vehicles.motion_interrupted()
            || !movement::flight_can_retire(&state)
        {
            return Err(unavailable(
                "flight requires unmounted resolved common motion context",
            ));
        }
        let player = self.common_player_unlocked(&state)?;
        let record = {
            let mut history = self.bot.flight_history.lock().expect("flight history");
            let record = api::flight::prepare(
                player,
                command,
                state.operations.requested_flying,
                history.as_ref(),
            )?;
            *history = Some(record.clone());
            record
        };
        let bot = self.bot.clone();
        let attempt = record.attempt;
        let (reply, result) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = reply.send(bot.operations().flight_send_owned(attempt).await);
        });
        drop(state);
        result
            .await
            .map_err(|_| unavailable("flight owner result unavailable"))?
    }
    async fn flight_send_owned(&self, attempt: u64) -> Result<FlightRecord> {
        let mut state = self.bot.session.state.lock().await;
        let result = async {
            self.mutable_with_flight_owner(&state, true, true)?;
            if state.operations.inventory.pending_swap.is_some()
                || state.operations.local_player.motion_interruption.is_some()
                || state.vehicles.motion_interrupted()
                || !movement::flight_can_retire(&state)
            {
                return Err(unavailable("flight motion context changed before dispatch"));
            }
            let record = self.flight_snapshot(attempt)?;
            let player = self.common_player_unlocked(&state)?;
            api::flight::validate_before(&record, &player, state.operations.requested_flying)?;
            match record.command {
                FlightCommand::SetFlying { flying } => {
                    self.bot
                        .session
                        .send(
                            ids::play_serverbound::ABILITIES,
                            &[if flying { 2 } else { 0 }],
                        )
                        .await?;
                    state.operations.requested_flying = flying;
                    if flying {
                        movement::retire_common_for_flight(&mut state)?;
                    }
                }
                FlightCommand::Move { position, rotation } => {
                    let mut payload = Vec::with_capacity(33);
                    for v in position {
                        payload.extend(v.to_be_bytes());
                    }
                    for v in rotation {
                        payload.extend(v.to_be_bytes());
                    }
                    payload.push(0);
                    let generation = state.loading.generation;
                    let sequence = state.sequence;
                    state
                        .motion
                        .begin(generation, sequence, position, rotation)?;
                    self.bot
                        .session
                        .send(ids::play_serverbound::POSITION_LOOK, &payload)
                        .await?;
                    state.position = Some(position);
                    state.rotation = rotation;
                    state.motion.dispatched();
                }
            }
            Ok(())
        }
        .await;
        {
            let mut history = self.bot.flight_history.lock().expect("flight history");
            let record = history.as_mut().expect("retained flight");
            match &result {
                Ok(()) => {
                    record.dispatched = true;
                    record.stage = FlightStage::Submitted;
                }
                Err(e) => record.inspection(e),
            }
        }
        result?;
        self.flight_snapshot(attempt)
    }
}
