//! Retained acceptance and the independent completeness of late correction tracking.
use super::*;

/// Completeness of late correction tracking, scoped to one retained action.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum MonitoringStatus {
    /// No terminal Accepted/Unknown action with a sent-evidence baseline yet.
    NotMonitoring,
    /// All evidence after the baseline was inspected through this local ordinal.
    /// This is not a server tick, and says nothing about future corrections.
    CompleteThrough {
        /// Last completely inspected local evidence ordinal.
        cursor: u64,
    },
    /// Evidence was evicted before inspection. Permanent for this action.
    Gap {
        /// Last complete boundary, or the original baseline before the first refresh.
        after: u64,
        /// First retained evidence is immediately after this boundary.
        retained_from: u64,
    },
    /// The world lifetime changed. Permanent for this action.
    WorldChanged {
        /// Last complete boundary, or the original baseline.
        after: u64,
        /// World observed when loss was detected.
        world: BodyWorld,
    },
    /// The target chunk was replaced/unloaded. Permanent for this action.
    ChunkChanged {
        /// Last complete boundary, or the original baseline.
        after: u64,
        /// Original lifetime-ending evidence ordinal.
        at: u64,
        /// Target chunk coordinates.
        chunk: [i32; 2],
    },
    /// This refresh failed; the last complete boundary remains valid. Retryable.
    TemporarilyUnavailable {
        /// Last complete boundary, or the original baseline.
        after: u64,
        /// Diagnostic text; not a stable machine-readable error category.
        reason: String,
    },
    /// A first rollback was found. Tracking ends at this original evidence ordinal.
    CorrectionObserved {
        /// Original correcting evidence ordinal.
        at: u64,
    },
}
impl MonitoringStatus {
    pub(crate) fn ended(&self) -> bool {
        matches!(
            self,
            Self::Gap { .. }
                | Self::WorldChanged { .. }
                | Self::ChunkChanged { .. }
                | Self::CorrectionObserved { .. }
        )
    }
}
/// The original result and subsequent evidence are deliberately separate axes.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ActionObservation {
    /// Immutable once terminal; never overwritten by monitoring failure/rollback.
    pub acceptance: ActionStatus,
    /// Latest known rollback, or the original result if none was observed.
    pub outcome: ActionStatus,
    /// Evidence completeness, independent of acceptance.
    pub monitoring: MonitoringStatus,
}
/// Cursor for the dedicated bounded monitor log; distinct from BodyEvent cursors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MonitorCursor {
    /// HeadlessBody identity; zero is allowed only with ordinal zero.
    pub backend: u64,
    /// Last consumed monitor transition.
    pub ordinal: u64,
}
/// One change to a retained action's monitoring state.
#[derive(Clone, Debug, serde::Serialize)]
pub struct MonitorChange {
    /// Cursor after this transition.
    pub cursor: MonitorCursor,
    /// Affected action, including ordinary submit IDs.
    pub action: ActionId,
    /// New completeness (first permanent loss is retained on the action).
    pub monitoring: MonitoringStatus,
}
/// Monitor transitions, isolated from ordinary BodyEvent traffic.
#[derive(Clone, Debug, serde::Serialize)]
pub struct MonitorChanges {
    /// Advances only through the returned entries.
    pub cursor: MonitorCursor,
    /// This log evicted unconsumed entries. Re-read retained action observations.
    /// Expired action IDs return an explicit error; lost history is unrecoverable.
    pub gap: bool,
    /// Ordered retained transitions, at most the requested maximum.
    pub changes: Vec<MonitorChange>,
}
