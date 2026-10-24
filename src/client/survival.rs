//! Survival-mode operations and explicitly restricted additional contracts.
pub use super::Survival;
pub(crate) mod mining;
pub(crate) mod mining_tools;
pub use mining_tools::MiningEstimate;
pub(crate) mod model;
pub(crate) mod placement;
pub(crate) mod terrain;
pub use placement::{
    FluidSlabPlacementRecord, PlacementId, PlacementProcessing, PlacementRecord, PlacementSend,
    PlacementStage, ReplaceableCubePlacementRecord, SlabFluidState, SlabPlacementRecord,
    StairNeighborUpdate, StairPlacementRecord, StairShapeInput,
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
pub use target::{
    BlockTargetHit, BlockTargetObservation, BlockTargetPose, BlockTargetPoseSource,
    ContextualBlockTargetObservation,
};
#[cfg(test)]
mod oracle_tests;

/// Exact unmodeled state refused by the bounded dry walking model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainRefusal {
    /// Version whose collision/effect model was requested.
    pub version: crate::MinecraftVersion,
    /// Full received name and properties; no fallback shape was assumed.
    pub state: crate::NativeBlockState,
}
impl std::fmt::Display for TerrainRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unsupported dry terrain state {} {:?} ({:?})",
            self.state.name, self.state.properties, self.version
        )
    }
}
impl std::error::Error for TerrainRefusal {}
