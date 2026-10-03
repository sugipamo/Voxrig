//! Survival-mode operations and explicitly restricted additional contracts.
pub use super::Survival;
pub mod checked;
pub(crate) mod model;
mod motion;
pub use motion::{
    MAX_SURVIVAL_CONTROL_TICKS, MotionPreview, MotionRecord, MotionStatus, PredictedMotionFrame,
    SurvivalControl, SurvivalInput, TerminalClearance,
};
