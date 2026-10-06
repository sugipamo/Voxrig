//! Common caller-supplied inputs and model frames.
//!
//! These data types do not imply that every adapter implements every motion
//! contract. Physics, admission and scheduling belong to the selected adapter;
//! predicted frames never establish a received position or server tick.

/// Digital walking input for one predicted native game tick, without sprint/sneak.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SurvivalInput {
    /// -1 backwards, 0 released, 1 forwards.
    pub forward: i8,
    /// -1 right, 0 released, 1 left.
    pub strafe: i8,
    /// Jump key state, including native repeat cooldown.
    pub jump: bool,
}
/// One native tick's heading and digital input. Route selection belongs to the caller.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SurvivalControl {
    /// Native body yaw in degrees.
    pub yaw: f32,
    /// No sprint/sneak or implicit controls.
    pub input: SurvivalInput,
}
/// Bound on a single finite connection-owned control run.
pub const MAX_SURVIVAL_CONTROL_TICKS: usize = 120;
/// Read-only dry full-cube forecast under the selected adapter's native defaults.
/// All initial values share one capture boundary. No packet is sent and no
/// reusable action authority, server tick or physical error bound is created.
#[derive(Clone, Debug, serde::Serialize)]
pub struct MotionPreview {
    /// Initial player/inventory capture, preserving received and local origins.
    pub initial: crate::client::PlayerObservation,
    /// Received world cache revision at that same adapter boundary.
    pub world_revision: u64,
    /// Model seed, separate from the received pose retained by initial.
    pub initial_frame: PredictedMotionFrame,
    /// Caller-supplied finite headings and digital controls.
    pub controls: Vec<SurvivalControl>,
    /// Per-control model frames; never received positions.
    pub frames: Vec<PredictedMotionFrame>,
    /// Model assessment for released rest on known dry support.
    pub terminal_clearance: TerminalClearance,
}
/// A simulated player frame, never a received pose or permission to build.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PredictedMotionFrame {
    /// Tick count from the preview's initial context, not server time.
    pub tick: u16,
    /// Predicted feet position.
    pub position: [f64; 3],
    /// Simulated next-tick velocity, including gravity while resting on a floor.
    pub velocity: [f64; 3],
    /// Predicted downward collision.
    pub on_ground: bool,
    /// Predicted X/Z obstruction.
    pub horizontal_collision: bool,
    /// No displacement with released controls and predicted floor contact.
    pub resting: bool,
}
/// An adapter model's endpoint assessment, separate from received outcomes.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TerminalClearance {
    /// The model reports resting support and a margin from solid walls.
    Admitted {
        /// Per-axis model-space planning reserve. Not a physical error bound for predicted continuation.
        horizontal_margin: f64,
    },
    /// The model requires new controls before dispatching the preview.
    RequiresReplan {
        /// Specific rest/support/geometry issue.
        reason: String,
    },
}

/// No phase means server-confirmed stopped motion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionStatus {
    /// The bounded input sequence is being dispatched at native tick spacing.
    Running,
    /// Locally settled; awaiting a same-instance position observation.
    AwaitingObservation,
    /// Prediction and observation agree, subject to fresh standing geometry checks.
    Observed,
    /// Fully dispatched and locally settled under the explicit prediction contract.
    /// Fresh native standing/geometry checks are still required before interaction.
    Predicted,
    /// Failure, correction, changed context or missing observation. Never auto-replay.
    RequiresInspection,
}
impl MotionStatus {
    /// A candidate for fresh standing admission, not authority by itself.
    pub fn is_continuation_candidate(self) -> bool {
        matches!(self, Self::Observed | Self::Predicted)
    }
}

/// Retained diagnostic of a finite prediction-based path, never reusable authority.
/// Elapsed time or complete dispatch is not a received/server-confirmed endpoint.
#[derive(Clone, Debug, serde::Serialize)]
pub struct MotionRecord {
    /// Owning connection/world and native registry version.
    pub session: crate::client::SessionStamp,
    /// Connection-local monotonically increasing identity.
    pub run_id: u64,
    /// Coherent initial capture and native-model forecast made before any send.
    pub preview: MotionPreview,
    /// Before-I/O intent for the latest native tick.
    pub attempted_tick: u16,
    /// Latest fully dispatched native tick, without implied acceptance.
    pub dispatched_ticks: u16,
    /// Retained phase; failure never automatically replays controls.
    pub status: MotionStatus,
    /// First retained interruption/failure, including after transport closure.
    pub problem: Option<String>,
}
