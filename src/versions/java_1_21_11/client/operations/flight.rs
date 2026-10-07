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
                FlightCommand::Land => {
                    return Err(unavailable("landing needs its retained finite owner"));
                }
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

impl Operations {
    async fn landing_send_owned(
        &self,
        attempt: u64,
        run: SurvivalMotionRecord,
    ) -> Result<FlightRecord> {
        let result = async {
            let mut state = self.bot.session.state.lock().await;
            self.mutable_with_flight_owner(&state, true, true)?;
            let record = self.flight_snapshot(attempt)?;
            let player = self.common_player_unlocked(&state)?;
            api::flight::validate_before(&record, &player, state.operations.requested_flying)?;
            let abilities = state.operations.abilities_receipt();
            if abilities != record.received_abilities {
                return Err(unavailable("landing abilities changed before I/O"));
            }
            let checked = movement::landing_plan(
                &mut state,
                self.bot.session.id,
                self.bot.session.started.elapsed().as_millis() as u64 / 50,
                player,
                record.landing.as_ref().unwrap().flight_attempt,
            )?;
            if checked.preview.initial.world_revision != run.preview.initial.world_revision
                || checked.preview.initial.player != run.preview.initial.player
            {
                return Err(unavailable("landing context changed after intent"));
            }
            self.bot
                .session
                .send(ids::play_serverbound::ABILITIES, &[0])
                .await?;
            state.operations.requested_flying = false;
            self.bot
                .flight_history
                .lock()
                .expect("flight history")
                .as_mut()
                .unwrap()
                .landing
                .as_mut()
                .unwrap()
                .disable_dispatched = true;
            self.bot
                .session
                .send(ids::play_serverbound::PLAYER_INPUT, &[0])
                .await?;
            self.bot
                .flight_history
                .lock()
                .expect("flight history")
                .as_mut()
                .unwrap()
                .landing
                .as_mut()
                .unwrap()
                .neutral_dispatched = true;
            let uuid = state
                .identity
                .as_ref()
                .ok_or_else(|| unavailable("landing identity unavailable"))?
                .uuid;
            state.survival_motion = Some(run.clone());
            drop(state);
            self.run_survival_motion(&run, None, uuid).await
        }
        .await;
        if let Err(e) = &result {
            let mut state = self.bot.session.state.lock().await;
            if let Some(r) = state
                .survival_motion
                .as_mut()
                .filter(|r| r.run_id == run.run_id)
            {
                r.status = SurvivalMotionStatus::RequiresInspection;
                r.problem.get_or_insert_with(|| e.to_string());
                if let Ok(r) = movement::landing_common_record(r) {
                    api::flight::sync_motion(&self.bot.flight_history, &r);
                }
            }
        }
        let mut history = self.bot.flight_history.lock().expect("flight history");
        let record = history.as_mut().unwrap();
        match &result {
            Ok(()) => {
                record.dispatched = true;
                record.stage = FlightStage::Submitted;
            }
            Err(e) => record.inspection(e),
        }
        result?;
        Ok(record.clone())
    }
}

impl crate::client::adapter::FlightOps for Operations {
    async fn flight(&self, command: FlightCommand) -> Result<FlightRecord> {
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
        let previous_attempt = self
            .bot
            .flight_history
            .lock()
            .expect("flight history")
            .as_ref()
            .map_or(0, |r| r.attempt);
        let landing_run = if command == FlightCommand::Land {
            let mut state = state;
            let run = movement::landing_plan(
                &mut state,
                self.bot.session.id,
                self.bot.session.started.elapsed().as_millis() as u64 / 50,
                player.clone(),
                previous_attempt,
            )?;
            (state, Some(run))
        } else {
            (state, None)
        };
        let (state, landing_run) = landing_run;
        let record = {
            let mut history = self.bot.flight_history.lock().expect("flight history");
            let mut record = api::flight::prepare(
                player,
                command,
                state.operations.requested_flying,
                history.as_ref(),
            )?;
            record.received_abilities = state.operations.abilities_receipt();
            if let Some(run) = &landing_run {
                record.landing = Some(api::FlightLanding {
                    flight_attempt: previous_attempt,
                    declared_controller_velocity: [0.; 3],
                    disable_dispatched: false,
                    neutral_dispatched: false,
                    motion: movement::landing_common_record(run)?,
                });
            }
            *history = Some(record.clone());
            record
        };
        let bot = self.bot.clone();
        let attempt = record.attempt;
        let (reply, result) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let result = if let Some(run) = landing_run {
                bot.operations().landing_send_owned(attempt, run).await
            } else {
                bot.operations().flight_send_owned(attempt).await
            };
            let _ = reply.send(result);
        });
        drop(state);
        result
            .await
            .map_err(|_| unavailable("flight owner result unavailable"))?
    }
}
