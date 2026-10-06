//! Survival-mode operations and explicitly restricted additional contracts.
pub use super::Survival;
pub mod checked;
pub(crate) mod mining;
pub(crate) mod mining_tools;
pub use mining_tools::MiningEstimate;
pub(crate) mod model;
pub(crate) mod placement;
pub(crate) mod terrain;
pub use placement::{
    PlacementId, PlacementProcessing, PlacementRecord, PlacementSend, PlacementStage,
};
mod motion;
mod recovery;
pub(crate) mod scene;
pub use recovery::{
    MiningProfileRecovery, MiningRecoveryAttempt, MiningRecoveryEvidence, MiningRecoveryMethod,
    MiningRecoveryTarget, RecoveredSurvivalClient,
};
pub use scene::{CapturedSurvivalScene, ScenePreview, SceneSource};
pub(crate) mod target;
pub use mining::{
    MiningAction, MiningId, MiningInventoryChange, MiningInventoryChangeKind,
    MiningProtocolObservation, MiningRecord, MiningSend, MiningStage, MiningTargetReceipt,
};
pub use motion::{
    MAX_SURVIVAL_CONTROL_TICKS, MotionPreview, MotionRecord, MotionStatus, PredictedMotionFrame,
    SurvivalControl, SurvivalInput, TerminalClearance,
};
pub use target::{BlockTargetHit, BlockTargetObservation};
