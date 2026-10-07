//! Connection-owned common flight writes; caller cancellation only stops waiting.
use super::*;
use crate::client::{
    self as api,
    flight::{FlightCommand, FlightRecord, FlightStage},
    operations::Action,
};
impl Bot {
    pub(super) fn flight_snapshot(&self, attempt: u64) -> Result<FlightRecord> {
        self.flight_history
            .lock()
            .expect("flight history")
            .as_ref()
            .filter(|r| r.attempt == attempt)
            .cloned()
            .ok_or_else(|| api::inventory::unavailable("flight intent superseded"))
    }
    async fn flight_send_owned(&self, attempt: u64) -> Result<FlightRecord> {
        let record = self.flight_snapshot(attempt)?;
        let action = match record.command {
            FlightCommand::Land => {
                return Err(api::inventory::unavailable(
                    "landing needs its retained finite owner",
                ));
            }
            FlightCommand::SetFlying { flying } => Action::SetFlying(flying),
            FlightCommand::Move { position, rotation } => Action::MoveFlying(position, rotation),
        };
        let result = self
            .execute_common_inner(api::GameMode::Creative, action, Some(attempt))
            .await;
        if result.is_ok() && matches!(record.command, FlightCommand::SetFlying { flying: true }) {
            let gate = self.coherent_state_gate.lock().await;
            if let Some(previous) = self.common_motion.lock().await.take() {
                let mut retired = previous.record;
                retired.status = api::survival::MotionStatus::RequiresInspection;
                retired.problem.get_or_insert_with(|| {
                    "ground motion superseded by owned Creative flight".into()
                });
                *self.retired_common_motion.lock().await = Some(retired);
            }
            drop(gate);
        }
        {
            let mut history = self.flight_history.lock().expect("flight history");
            let record = history.as_mut().expect("retained flight");
            match &result {
                Ok(_) => {
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

impl Bot {
    async fn landing_send_owned(
        &self,
        attempt: u64,
        run: super::common_motion::NativeMotionRun,
    ) -> Result<FlightRecord> {
        let result = async {
            let gate = self.coherent_state_gate.lock().await;
            self.common_motion_admission_inner(true).await?;
            let record = self.flight_snapshot(attempt)?;
            let player = self.common_player_unlocked().await?;
            let receipts = self.common_receipts.lock().await;
            api::flight::validate_before(&record, &player, receipts.requested_flying)?;
            if receipts.abilities != record.received_abilities
                || self.connection_state() != ConnectionState::Ready
            {
                return Err(api::inventory::unavailable(
                    "landing abilities or connection changed before I/O",
                ));
            }
            drop(receipts);
            if self.control().await != ControlState::default()
                || self.motion.lock().await.revision() != record.native_motion_revision.unwrap()
            {
                return Err(api::inventory::unavailable(
                    "landing motion owner changed before I/O",
                ));
            }
            let checked = self.landing_plan(player).await?;
            if checked.record.preview.world_revision != run.record.preview.world_revision {
                return Err(api::inventory::unavailable(
                    "landing geometry changed after intent",
                ));
            }
            self.send(0x1a, &[0]).await?;
            self.common_receipts.lock().await.requested_flying = false;
            self.flight_history
                .lock()
                .expect("flight history")
                .as_mut()
                .unwrap()
                .landing
                .as_mut()
                .unwrap()
                .disable_dispatched = true;
            self.send(0x1d, &[0; 9]).await?;
            self.flight_history
                .lock()
                .expect("flight history")
                .as_mut()
                .unwrap()
                .landing
                .as_mut()
                .unwrap()
                .neutral_dispatched = true;
            let revision = self
                .connection
                .motion_admission_revision()
                .await
                .map_err(|e| {
                    api::inventory::unavailable(format!(
                        "landing bounded admission rejected: {e:?}"
                    ))
                })?;
            let run = self.install_landing_model(run).await;
            drop(gate);
            self.run_common_path(run, revision).await
        }
        .await;
        if let Err(e) = &result {
            let gate = self.coherent_state_gate.lock().await;
            if let Some(run) = self.common_motion.lock().await.as_mut() {
                run.record.status = api::survival::MotionStatus::RequiresInspection;
                run.record.problem.get_or_insert_with(|| e.to_string());
                api::flight::sync_motion(&self.flight_history, &run.record);
            }
            drop(gate);
        }
        let mut history = self.flight_history.lock().expect("flight history");
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

impl crate::client::adapter::FlightOps for Bot {
    async fn flight(&self, command: FlightCommand) -> Result<FlightRecord> {
        let gate = self.coherent_state_gate.lock().await;
        self.common_motion_admission().await?;
        if self.connection_state() != ConnectionState::Ready
            || self.control().await != ControlState::default()
            || self.common_receipts.lock().await.pending_dispatch
            || self
                .common_receipts
                .lock()
                .await
                .vehicles
                .motion_interrupted()
        {
            return Err(api::inventory::unavailable(
                "flight requires ready released unmounted context",
            ));
        }
        let player = self.common_player_unlocked().await?;
        let requested = self.common_receipts.lock().await.requested_flying;
        let abilities = self.common_receipts.lock().await.abilities.clone();
        let revision = self.motion.lock().await.revision();
        let landing_run = if command == FlightCommand::Land {
            let previous = self
                .flight_history
                .lock()
                .expect("flight history")
                .clone()
                .ok_or_else(|| api::inventory::unavailable("landing requires owned flight step"))?;
            if previous.native_motion_revision != Some(revision) {
                return Err(api::inventory::unavailable(
                    "landing local motion interrupted after flight step",
                ));
            }
            Some(self.landing_plan(player.clone()).await?)
        } else {
            None
        };
        let record = {
            let mut history = self.flight_history.lock().expect("flight history");
            let previous_attempt = history.as_ref().map_or(0, |r| r.attempt);
            let mut record = api::flight::prepare(player, command, requested, history.as_ref())?;
            record.received_abilities = abilities;
            record.native_motion_revision = Some(revision);
            if let Some(run) = &landing_run {
                record.landing = Some(api::FlightLanding {
                    flight_attempt: previous_attempt,
                    declared_controller_velocity: [0.; 3],
                    disable_dispatched: false,
                    neutral_dispatched: false,
                    motion: run.record.clone(),
                });
            }
            *history = Some(record.clone());
            record
        };
        let bot = self.clone_internal();
        let attempt = record.attempt;
        let (reply, result) = tokio::sync::oneshot::channel();
        // Retention and spawn are synchronous: dropping the caller cannot lose the owner.
        tokio::spawn(async move {
            let result = if let Some(run) = landing_run {
                bot.landing_send_owned(attempt, run).await
            } else {
                bot.flight_send_owned(attempt).await
            };
            let _ = reply.send(result);
        });
        drop(gate);
        result
            .await
            .map_err(|_| api::inventory::unavailable("flight owner result unavailable"))?
    }
}
