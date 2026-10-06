//! One-shot common Creative flight commands. Sending never establishes grounding.
use super::{GameMode, PlayerObservation, SessionStamp};
use crate::{Result, client::inventory::unavailable};
pub(crate) type History = std::sync::Arc<std::sync::Mutex<Option<FlightRecord>>>;

/// Exact requested operation, retained before its owner starts I/O.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FlightCommand {
    /// Request an ability flag; not an acknowledgement of flight or grounding.
    SetFlying {
        /// Requested native flying bit; separate from received abilities.
        flying: bool,
    },
    /// Submit a bounded feet position without collision resolution.
    Move {
        /// Requested feet position, at most four blocks from current local position.
        position: [f64; 3],
        /// Requested yaw/pitch in native degrees.
        rotation: [f32; 2],
    },
}
/// Diagnostic lifecycle; an interrupted command is never automatically replayed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FlightStage {
    /// Connection-owned intent retained before writer admission.
    Prepared,
    /// Entire native frame sent, without a server acknowledgement.
    Submitted,
    /// Context or I/O failed; inspect the original attempt without replay.
    RequiresInspection,
}
/// Retained common flight dispatch. Saved data cannot authorize another write.
/// ```compile_fail
/// let record: voxrig::client::FlightRecord = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Clone, Debug, serde::Serialize)]
pub struct FlightRecord {
    /// Owning connection and world.
    pub session: SessionStamp,
    /// Monotonically increasing local attempt, never sent as a protocol sequence.
    pub attempt: u64,
    /// Exact requested operation.
    pub command: FlightCommand,
    /// Coherent observation before claiming the write.
    pub initial: PlayerObservation,
    /// Local requested flag before this command; separate from received abilities.
    pub previously_requested_flying: bool,
    /// Current dispatch lifecycle.
    pub stage: FlightStage,
    /// Entire frame dispatched; not a position, flight or rest acknowledgement.
    pub dispatched: bool,
    /// Latched failure; cancellation of the caller does not replay the owner.
    pub requires_inspection: Option<String>,
}
impl FlightRecord {
    pub(crate) fn unresolved(&self) -> bool {
        self.stage != FlightStage::Submitted
    }
    pub(crate) fn inspection(&mut self, reason: impl std::fmt::Display) {
        self.requires_inspection
            .get_or_insert_with(|| reason.to_string());
        self.stage = FlightStage::RequiresInspection;
    }
}
pub(crate) fn unresolved(history: &History) -> bool {
    history
        .lock()
        .expect("flight history")
        .as_ref()
        .is_some_and(FlightRecord::unresolved)
}
pub(crate) fn context(
    player: &PlayerObservation,
    command: FlightCommand,
    requested: bool,
) -> Result<()> {
    if player.game_mode != Some(GameMode::Creative)
        || !player.health.as_ref().is_some_and(|h| h.value.health > 0.0)
        || player.position.is_none()
        || player.received_pose.is_none()
    {
        return Err(unavailable(
            "flight requires healthy positioned received Creative context",
        ));
    }
    match command {
        FlightCommand::SetFlying { flying: true } if player.may_fly != Some(true) => {
            return Err(unavailable("server has not granted flight"));
        }
        FlightCommand::Move { position, rotation } => {
            super::operations::validate_rotation(rotation)?;
            if position
                .iter()
                .any(|v| !v.is_finite() || v.abs() > 30_000_000.0)
            {
                return Err(super::registry::invalid("invalid flight position"));
            }
            if player.may_fly != Some(true) || !requested {
                return Err(unavailable("flight must be permitted and requested"));
            }
            if player
                .position
                .as_ref()
                .unwrap()
                .value
                .iter()
                .zip(position)
                .map(|(a, b)| (a - b).powi(2))
                .sum::<f64>()
                > 16.0
            {
                return Err(super::registry::invalid("flight step exceeds four blocks"));
            }
        }
        _ => {}
    }
    Ok(())
}
pub(crate) fn prepare(
    player: PlayerObservation,
    command: FlightCommand,
    requested: bool,
    previous: Option<&FlightRecord>,
) -> Result<FlightRecord> {
    context(&player, command, requested)?;
    // Adapters already check mutation conflicts under their coherent lock.
    // Unacknowledged Creative inventory creation is independent of a flight write.
    if previous.is_some_and(FlightRecord::unresolved) {
        return Err(unavailable(
            "prior flight or other dispatch unresolved; inspect without replay",
        ));
    }
    let attempt = previous
        .map_or(Some(1), |r| r.attempt.checked_add(1))
        .ok_or_else(|| unavailable("flight attempts exhausted"))?;
    Ok(FlightRecord {
        session: player.session,
        attempt,
        command,
        initial: player,
        previously_requested_flying: requested,
        stage: FlightStage::Prepared,
        dispatched: false,
        requires_inspection: None,
    })
}
pub(crate) fn validate_before(
    record: &FlightRecord,
    player: &PlayerObservation,
    requested: bool,
) -> Result<()> {
    context(player, record.command, requested)?;
    if record.stage != FlightStage::Prepared
        || record.dispatched
        || record.requires_inspection.is_some()
        || player.session != record.session
        || player.dimension != record.initial.dimension
        || player.position != record.initial.position
        || player.received_pose != record.initial.received_pose
        || requested != record.previously_requested_flying
    {
        return Err(unavailable(
            "flight intent superseded or context changed; inspect without replay",
        ));
    }
    Ok(())
}
impl super::Client {
    /// Read the latest owned flight intent without waiting for a blocked writer.
    /// Completed records remain diagnostics after closure; no fresh receipt is invented.
    pub fn flight_record(&self) -> Option<FlightRecord> {
        let history = match &self.adapter {
            crate::connection::Adapter::Java1_16_1(bot) => &bot.flight_history,
            crate::connection::Adapter::Java1_21_11(bot) => &bot.flight_history,
        };
        history.lock().expect("flight history").clone()
    }
}
