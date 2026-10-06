//! Connection-owned, explicitly staged empty-hand mining diagnostics.
use crate::NativeBlockState;
use crate::client::{BlockFace, ObservedValue, PlayerObservation, SessionStamp, SlotKnowledge};

/// Opaque identity of one retained attempt; inspection JSON cannot recreate it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct MiningId {
    session: SessionStamp,
    attempt: u64,
}
impl MiningId {
    pub(crate) fn new(session: SessionStamp, attempt: u64) -> Self {
        Self { session, attempt }
    }
    /// Original connection/world; not a live operation permit.
    pub fn session(self) -> SessionStamp {
        self.session
    }
    /// Owning adapter's attempt identity, not a server tick.
    pub fn attempt(self) -> u64 {
        self.attempt
    }
}
/// Explicit native mining command. No elapsed timer automatically sends these.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum MiningAction {
    /// Start destroying the original block.
    Start = 0,
    /// Abort once; not proof that delayed FINISH was cleared.
    Abort = 1,
    /// Finish once; an early finish may schedule a delayed server break.
    Finish = 2,
}
/// Before-I/O command record; a complete write is not acceptance.
#[derive(Clone, Debug, serde::Serialize)]
pub struct MiningSend {
    /// Applied receive boundary before the attempted command.
    pub after_sequence: u64,
    /// Native global interaction sequence; absent on 1.16.1.
    pub interaction_sequence: Option<i32>,
    /// Complete command frame was dispatched.
    pub dispatched: bool,
}
/// Fresh exact-target packet, separate from unrelated world/cache revisions.
#[derive(Clone, Debug, serde::Serialize)]
pub struct MiningTargetReceipt {
    /// Complete version-native received state.
    pub state: NativeBlockState,
    /// Actual packet ordinal.
    pub receive_sequence: u64,
}
/// Native protocol facts are not equivalent across versions or result authority.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MiningProtocolObservation {
    /// Legacy response names an action and target, not a global interaction sequence.
    LegacyReply {
        /// Actual native status (0 START, 1 ABORT, 2 FINISH).
        action: i32,
        /// Native success flag; not independent proof of removal or safe continuation.
        accepted: bool,
        /// Complete state carried by that response.
        state: NativeBlockState,
        /// Response packet ordinal.
        receive_sequence: u64,
    },
    /// Modern acknowledgement says the interaction sequence was processed.
    ModernProcessing {
        /// Highest acknowledged sequence.
        sequence: i32,
        /// Ordinal of the packet that established this value.
        receive_sequence: u64,
    },
}
/// First received inventory prerequisite that changed during an attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MiningInventoryChangeKind {
    /// Selected hand or its ordered provenance changed.
    SelectionChanged,
    /// Player screen or cursor changed/became unavailable.
    PlayerScreenChanged,
    /// Unsupported data or unresolved inventory operations appeared.
    InventoryUnavailable,
    /// Original selected slot stopped being received empty.
    SelectedHandChanged,
}
/// Latched first interruption, preserved even if later packets restore an empty hand.
#[derive(Clone, Debug, serde::Serialize)]
pub struct MiningInventoryChange {
    /// Which prerequisite changed first.
    pub kind: MiningInventoryChangeKind,
    /// Actual receive boundary of the first change.
    pub receive_sequence: u64,
    /// Selection and its receive/submission provenance at that boundary.
    pub selection: Option<ObservedValue<u8>>,
    /// Contents of the original selected slot; missing data remains Unavailable.
    pub original_hand: SlotKnowledge,
    /// Slot-specific receive boundary, when available.
    pub hand_receive_sequence: Option<u64>,
    /// Received active screen.
    pub window_id: Option<i32>,
    /// Received carried stack, never an inferred empty default.
    pub cursor: SlotKnowledge,
    /// Adapter reports unsupported inventory data.
    pub unsupported_components: bool,
    /// No other cause has been latched before or after this interruption.
    pub sole_cause: bool,
}
/// Retained state of the explicit attempt; no variant grants the next mutation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MiningStage {
    /// START retained/attempted, without FINISH or a confirmed result.
    Mining,
    /// FINISH retained/attempted; still unresolved even after ABORT/ack.
    PendingAfterFinish,
    /// Fresh exact-target air was reconciled; actor attribution is unknown.
    ObservedRemoved,
    /// Changed context, uncertain failure or conflicting packets need inspection.
    RequiresInspection,
}
/// Snapshot of the connection's retained attempt, also readable after closure.
#[derive(Clone, Debug, serde::Serialize)]
pub struct MiningRecord {
    /// Immutable owning attempt identity.
    pub id: MiningId,
    /// Coherent player/inventory capture before possible START I/O.
    pub initial: PlayerObservation,
    /// Original removal cell.
    pub target: [i32; 3],
    /// Original native first-outline face.
    pub face: BlockFace,
    /// Complete received predecessor.
    pub baseline: NativeBlockState,
    /// Local scheduling estimate; not elapsed server ticks or an acceptance fence.
    pub estimated_wait_ms: u64,
    /// Retained before possible START dispatch.
    pub start: MiningSend,
    /// At most one explicit FINISH attempt, retained before I/O.
    pub finish: Option<MiningSend>,
    /// At most one explicit ABORT attempt; never releases delayed mining.
    pub abort: Option<MiningSend>,
    /// Latest exact-target packet; not a current-world snapshot.
    pub target_receipt: Option<MiningTargetReceipt>,
    /// Latest relevant version-native protocol receipt, when available.
    pub protocol: Option<MiningProtocolObservation>,
    /// First received inventory interruption.
    pub inventory_change: Option<MiningInventoryChange>,
    /// First latched failure/context reason; later air cannot clear it.
    pub requires_inspection: Option<String>,
    /// Current retained result state.
    pub stage: MiningStage,
    /// Original connection's once-only before-I/O recovery claim.
    pub recovery_attempt: Option<super::MiningRecoveryAttempt>,
    /// Always false on the original connection, including after fresh recovery.
    pub continuation_validated: bool,
}
pub(crate) fn material(state: &NativeBlockState) -> crate::Result<()> {
    if !matches!(state.name.as_str(), "minecraft:dirt" | "minecraft:stone")
        || !state.properties.is_empty()
    {
        return Err(crate::Error::new(
            crate::ErrorKind::Unsupported,
            anyhow::anyhow!("empty-hand mining currently admits dirt and stone only"),
        ));
    }
    Ok(())
}
pub(crate) fn unavailable(reason: impl std::fmt::Display) -> crate::Error {
    crate::Error::new(crate::ErrorKind::State, anyhow::anyhow!("{reason}"))
}
