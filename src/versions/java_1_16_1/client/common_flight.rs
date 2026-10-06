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
    pub(crate) async fn common_flight(&self, command: FlightCommand) -> Result<FlightRecord> {
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
        let record = {
            let mut history = self.flight_history.lock().expect("flight history");
            let record = api::flight::prepare(player, command, requested, history.as_ref())?;
            *history = Some(record.clone());
            record
        };
        let bot = self.clone_internal();
        let attempt = record.attempt;
        let (reply, result) = tokio::sync::oneshot::channel();
        // Retention and spawn are synchronous: dropping the caller cannot lose the owner.
        tokio::spawn(async move {
            let _ = reply.send(bot.flight_send_owned(attempt).await);
        });
        drop(gate);
        result
            .await
            .map_err(|_| api::inventory::unavailable("flight owner result unavailable"))?
    }
    async fn flight_send_owned(&self, attempt: u64) -> Result<FlightRecord> {
        let record = self.flight_snapshot(attempt)?;
        let action = match record.command {
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
