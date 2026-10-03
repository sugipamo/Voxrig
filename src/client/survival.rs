//! Survival-mode operations and explicitly restricted additional contracts.
pub use super::Survival;
pub mod checked;
mod motion;
pub use motion::{
    MAX_SURVIVAL_CONTROL_TICKS, PredictedMotionFrame, SurvivalControl, SurvivalInput,
    TerminalClearance,
};
