//! Common caller-supplied inputs and model frames.
//!
//! These data types do not imply that every adapter implements every motion
//! contract. Physics, admission and scheduling belong to the selected adapter;
//! predicted frames never establish a received position or server tick.

/// Digital walking input for one predicted native game tick, without sprint/sneak.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub struct SurvivalInput {
    /// -1 backwards, 0 released, 1 forwards.
    pub forward: i8,
    /// -1 right, 0 released, 1 left.
    pub strafe: i8,
    /// Jump key state, including native repeat cooldown.
    pub jump: bool,
}
/// One native tick's heading and digital input. Route selection belongs to the caller.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct SurvivalControl {
    /// Native body yaw in degrees.
    pub yaw: f32,
    /// No sprint/sneak or implicit controls.
    pub input: SurvivalInput,
}
/// Bound on a single finite connection-owned control run.
pub const MAX_SURVIVAL_CONTROL_TICKS: usize = 120;
/// A simulated player frame, never a received pose or permission to build.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
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
#[derive(Clone, Debug, serde::Serialize)]
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
