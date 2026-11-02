//! One-shot native item admission. A guard runs after readiness and writer waits.
//!
//! Guards must be short, synchronous and non-reentrant: inspect only the supplied
//! cut and independent caller-owned data (for example atomic lease/stop flags).
//! Do not call Client/native APIs, block, acquire locks shared with Voxrig callers,
//! or enter a runtime from a guard. No Client capability is passed to the guard.
//! Unwinding panics are caught and reported as NotSent; panic=abort still aborts.
//!
//! `*_with_reads` variants also freeze, for at most [`MAX_GUARDED_READS`] caller
//! cells, the receive ordinal of the last applied packet that set each cell's
//! block state: block change, multi-block change, 1.16.1 digging acknowledgement
//! and explosion blocks (even when the state is unchanged), and chunk data for
//! the whole column. Light-only updates, block-entity data, entity, time and all
//! other packets do not count. A column that is not loaded (never received,
//! unloaded, evicted, reset or dimension change) or a Y outside the dimension
//! reads `None`; callers must fail closed. Ordinals can be later than the actual
//! last change (a reload or a busy section counts as touching every cell) but
//! never earlier.
use super::{
    DispatchReceipt, GameMode, Hand, PlayerObservation, SlotKnowledge, registry::RegistryId,
};
use crate::{Client, Result};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
use tokio::time::{Duration, Instant};

pub(crate) mod cells;

/// Maximum cells in one guarded read set; longer lists are rejected before any
/// wait or cut, never truncated.
pub const MAX_GUARDED_READS: usize = 64;

/// Selection and received held identity. Count and components are not compared.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct HeldItemExpectation {
    /// Required selected hotbar index, including for offhand use.
    pub selected_hotbar: u8,
    /// None requires explicit received Empty, never missing/unavailable knowledge.
    pub item: Option<RegistryId>,
}
/// Explicit block-local hit, independent of the view ray.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct BlockUseTarget {
    /// Absolute block position.
    pub position: [i32; 3],
    /// Clicked face.
    pub face: crate::BlockFace,
    /// Each coordinate must be within 0..1.
    pub cursor: [f32; 3],
}
/// Immutable physical input frozen by the native state gate through frame flush.
/// The receive high-water mark is not per-field freshness; inspect value sources.
#[derive(Clone, Debug, serde::Serialize)]
pub struct PhysicalCut {
    /// Coherent pose, selection, inventory, session and receive sequence.
    pub player: PlayerObservation,
    /// Requested hand.
    pub hand: Hand,
    /// Actual player slot selected by this hand; None if selection is unknown.
    pub slot: Option<usize>,
    /// Value and provenance for that slot; None means unknown.
    pub held: Option<super::ObservedValue<SlotKnowledge>>,
    /// Requested read cells in request order, each with the receive ordinal that
    /// last touched it under this same gate. None is unknown/unloaded: fail closed.
    /// Empty unless a `*_with_reads` API supplied cells.
    pub cells: Vec<([i32; 3], Option<u64>)>,
}
impl PhysicalCut {
    pub(crate) fn new(
        player: PlayerObservation,
        hand: Hand,
        cells: Vec<([i32; 3], Option<u64>)>,
    ) -> Self {
        let slot = match hand {
            Hand::Main => player
                .selected_hotbar
                .as_ref()
                .filter(|s| s.value <= 8)
                .map(|s| 36 + usize::from(s.value)),
            Hand::Off => Some(45),
        };
        let held = slot
            .and_then(|i| player.inventory.slots.get(i))
            .cloned()
            .flatten();
        Self {
            player,
            hand,
            slot,
            held,
            cells,
        }
    }
    pub(crate) fn matches(&self, expected: HeldItemExpectation) -> bool {
        let selected = self
            .player
            .selected_hotbar
            .as_ref()
            .filter(|s| {
                matches!(
                    s.source,
                    super::ValueSource::Received { .. } | super::ValueSource::Submitted
                )
            })
            .map(|s| s.value);
        selected == Some(expected.selected_hotbar)
            && self.held.as_ref().is_some_and(|s| {
                matches!(s.source, super::ValueSource::Received { .. })
                    && match (&s.value, expected.item) {
                        (SlotKnowledge::Empty, None) => true,
                        (SlotKnowledge::Item { item }, Some(id)) => item.id == id,
                        _ => false,
                    }
            })
    }
}
/// Proven refusal before any frame write or surviving send authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum UseNotSentReason {
    /// Caller guard returned false.
    GuardRejected,
    /// Caller guard unwound; the connection and its locks remain usable.
    GuardPanicked,
    /// Required selection/held identity was missing or different.
    ExpectationMismatch,
    /// The fixed five-second budget elapsed before commit.
    AdmissionDeadline,
}
/// Native dispatch evidence, independent of the eventual game result.
#[derive(Clone, Debug, serde::Serialize)]
#[non_exhaustive]
pub enum GuardedUseResult {
    /// Definitely no use packet, and no later send from this invocation.
    NotSent {
        /// Stable typed refusal.
        reason: UseNotSentReason,
        /// Cut inspected if available. Deadline can precede a cut.
        cut: Option<Box<PhysicalCut>>,
    },
    /// Complete frame flush only. Observe existing receipt/Body paths for effects.
    Sent {
        /// Same dispatch-only receipt as the existing common operation.
        receipt: DispatchReceipt,
        /// Exact guard input, retained for correlation.
        cut: Box<PhysicalCut>,
    },
}
impl GuardedUseResult {
    /// Uses mo9's evidence vocabulary without changing OwnedOperation semantics.
    pub fn outcome(&self) -> super::OperationOutcome {
        match self {
            Self::NotSent { .. } => super::OperationOutcome::NotSent,
            Self::Sent { .. } => super::OperationOutcome::SentUnconfirmed,
        }
    }
    pub(crate) fn refused(reason: UseNotSentReason, cut: Option<PhysicalCut>) -> Self {
        Self::NotSent {
            reason,
            cut: cut.map(Box::new),
        }
    }
}
pub(crate) type Check = Box<dyn FnOnce(&PhysicalCut) -> bool + Send + 'static>;
#[derive(Clone, Copy)]
pub(crate) struct Request {
    pub hand: Hand,
    pub target: Option<BlockUseTarget>,
    pub expected: Option<HeldItemExpectation>,
}
impl Request {
    pub(crate) fn validate(self, version: crate::MinecraftVersion) -> Result<()> {
        if let Some(e) = self.expected {
            if e.selected_hotbar > 8 {
                return Err(crate::error::invalid_input(
                    "expected hotbar slot must be 0..8",
                ));
            }
            if let Some(item) = e.item {
                super::registry::Registry::for_version(version).item_definition(item)?;
            }
        }
        if let Some(t) = self.target {
            super::item_use::validate_cursor(t.cursor)?;
        }
        Ok(())
    }
    pub(crate) fn check(
        self,
        cut: &PhysicalCut,
        check: Check,
    ) -> std::result::Result<(), UseNotSentReason> {
        if self.expected.is_some_and(|e| !cut.matches(e)) {
            return Err(UseNotSentReason::ExpectationMismatch);
        }
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(cut))) {
            Ok(true) => Ok(()),
            Ok(false) => Err(UseNotSentReason::GuardRejected),
            Err(_) => Err(UseNotSentReason::GuardPanicked),
        }
    }
}
// Pending -> committed/cancelled is a single shared transition. A dropped waiter
// cannot leave a queued actor command authorized to start writing later.
#[derive(Clone)]
pub(crate) struct Attempt(Arc<AttemptState>);
struct AttemptState {
    phase: AtomicU8,
    frozen: std::sync::Mutex<Option<tokio::sync::OwnedMutexGuard<()>>>,
}
impl Attempt {
    pub(crate) fn new() -> Self {
        Self(Arc::new(AttemptState {
            phase: AtomicU8::new(0),
            frozen: std::sync::Mutex::new(None),
        }))
    }
    pub(crate) fn commit(&self) -> bool {
        self.0
            .phase
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    fn cancel(&self) -> bool {
        let cancelled = self
            .0
            .phase
            .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
            || self.0.phase.load(Ordering::Acquire) == 2;
        if cancelled {
            self.0.frozen.lock().expect("guarded frozen gate").take();
        }
        cancelled
    }
    pub(crate) fn freeze(&self, gate: tokio::sync::OwnedMutexGuard<()>) {
        let mut frozen = self.0.frozen.lock().expect("guarded frozen gate");
        if !self.cancelled() {
            *frozen = Some(gate);
        }
    }
    pub(crate) fn take_gate(&self) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        self.0.frozen.lock().expect("guarded frozen gate").take()
    }
    pub(crate) fn cancelled(&self) -> bool {
        self.0.phase.load(Ordering::Acquire) == 2
    }
}
struct CancelOnDrop(Attempt);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

impl Client {
    pub(crate) async fn guarded_use(
        &self,
        mode: Option<GameMode>,
        request: Request,
        reads: &[[i32; 3]],
        check: Check,
    ) -> Result<GuardedUseResult> {
        request.validate(self.version())?;
        if reads.len() > MAX_GUARDED_READS {
            return Err(crate::error::invalid_input(
                "guarded read set exceeds MAX_GUARDED_READS",
            ));
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        let attempt = Attempt::new();
        let _cancel = CancelOnDrop(attempt.clone());
        let execution = async {
            match &self.adapter {
                crate::connection::Adapter::Java1_16_1(bot) => {
                    bot.guarded_use(mode, request, reads, check, deadline, attempt.clone())
                        .await
                }
                crate::connection::Adapter::Java1_21_11(bot) => {
                    bot.operations()
                        .guarded_use(mode, request, reads, check, deadline, attempt.clone())
                        .await
                }
            }
        };
        match tokio::time::timeout_at(deadline, execution).await {
            Ok(result) => result,
            Err(_) if attempt.cancel() => Ok(GuardedUseResult::refused(
                UseNotSentReason::AdmissionDeadline,
                None,
            )),
            Err(_) => Err(crate::Error::new(
                crate::ErrorKind::UncertainDispatch,
                anyhow::anyhow!("guarded use deadline after commit; inspect without retry"),
            )),
        }
    }
}

macro_rules! guarded_api {
    ($ty:ty, $mode:expr) => {
        impl $ty {
            /// One-shot use after a fresh native cut and synchronous guard. All readiness,
            /// state and writer waits precede the guard. See `guarded_use` module for the
            /// non-reentrant callback contract. Fixed total budget: five seconds. No retry.
            /// False/panic/mismatch is typed NotSent. Errors after commit can be uncertain;
            /// Sent proves complete flush only and does not establish server acceptance.
            pub async fn use_item_guarded<F>(
                &self,
                hand: Hand,
                expected: Option<HeldItemExpectation>,
                check: F,
            ) -> Result<GuardedUseResult>
            where
                F: FnOnce(&PhysicalCut) -> bool + Send + 'static,
            {
                self.client
                    .guarded_use(
                        Some($mode),
                        Request {
                            hand,
                            target: None,
                            expected,
                        },
                        &[],
                        Box::new(check),
                    )
                    .await
            }
            /// Guarded explicit block use, including received empty-hand expectations.
            /// Has the same guard/deadline/evidence contract as `use_item_guarded` and
            /// the same server-range and cursor rules as existing explicit block use.
            pub async fn use_on_block_guarded<F>(
                &self,
                target: BlockUseTarget,
                hand: Hand,
                expected: Option<HeldItemExpectation>,
                check: F,
            ) -> Result<GuardedUseResult>
            where
                F: FnOnce(&PhysicalCut) -> bool + Send + 'static,
            {
                self.client
                    .guarded_use(
                        Some($mode),
                        Request {
                            hand,
                            target: Some(target),
                            expected,
                        },
                        &[],
                        Box::new(check),
                    )
                    .await
            }
            /// `use_item_guarded` whose cut also carries `PhysicalCut::cells` for
            /// `reads` (at most `MAX_GUARDED_READS`, else InvalidInput without any
            /// wait or send). See the `guarded_use` module for what touches a cell.
            pub async fn use_item_guarded_with_reads<F>(
                &self,
                hand: Hand,
                expected: Option<HeldItemExpectation>,
                reads: &[[i32; 3]],
                check: F,
            ) -> Result<GuardedUseResult>
            where
                F: FnOnce(&PhysicalCut) -> bool + Send + 'static,
            {
                self.client
                    .guarded_use(
                        Some($mode),
                        Request {
                            hand,
                            target: None,
                            expected,
                        },
                        reads,
                        Box::new(check),
                    )
                    .await
            }
            /// `use_on_block_guarded` with the read cells of `use_item_guarded_with_reads`.
            pub async fn use_on_block_guarded_with_reads<F>(
                &self,
                target: BlockUseTarget,
                hand: Hand,
                expected: Option<HeldItemExpectation>,
                reads: &[[i32; 3]],
                check: F,
            ) -> Result<GuardedUseResult>
            where
                F: FnOnce(&PhysicalCut) -> bool + Send + 'static,
            {
                self.client
                    .guarded_use(
                        Some($mode),
                        Request {
                            hand,
                            target: Some(target),
                            expected,
                        },
                        reads,
                        Box::new(check),
                    )
                    .await
            }
            /// Additive expect API: None requires a received empty slot. Unknown slots
            /// never count as empty. Uses the guarded boundary with an always-true guard.
            pub async fn use_on_block_expect_slot(
                &self,
                target: BlockUseTarget,
                hand: Hand,
                expected: HeldItemExpectation,
            ) -> Result<GuardedUseResult> {
                self.use_on_block_guarded(target, hand, Some(expected), |_| true)
                    .await
            }
        }
    };
}
guarded_api!(super::Survival, GameMode::Survival);
guarded_api!(super::Creative, GameMode::Creative);

#[cfg(test)]
pub(crate) mod tests;
