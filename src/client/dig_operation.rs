//! Invocation-bound cancellation and joined completion for operational digging.
#![allow(clippy::manual_async_fn)] // The explicit Send guarantee is part of this API.
use super::{DigRecord, GenerationKey, SessionStamp, operational_dig::Action};
use crate::{ErrorKind, Result};
use std::{
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::Notify;

/// Opaque process-local identity. No lookup, reconstruction or persistence is supported.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct DigOperationId {
    generation: GenerationKey,
    ordinal: u64,
}
impl std::hash::Hash for DigOperationId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.generation.version().hash(state);
        self.generation.connection_id().hash(state);
        self.ordinal.hash(state);
    }
}
impl DigOperationId {
    pub(crate) fn new(generation: GenerationKey, ordinal: u64) -> Self {
        Self {
            generation,
            ordinal,
        }
    }
}

/// Local write evidence, never a server acknowledgement or rollback proof.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum DigWriteEvidence {
    /// Not admitted; in a completion this is sealed against later dispatch.
    NotSent,
    /// Committed, with its write outcome not yet known. Snapshots only.
    Pending,
    /// Complete local write (including flush on 1.21.11).
    Written {
        /// Modern interaction sequence; absent on 1.16.1.
        interaction_sequence: Option<i32>,
    },
    /// May have been partially or fully written; never retry automatically.
    Unknown,
}
/// Independent evidence for the three possible writes.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct DigDispatchEvidence {
    start: DigWriteEvidence,
    stop: DigWriteEvidence,
    abort: DigWriteEvidence,
}
impl DigDispatchEvidence {
    /// START evidence.
    pub fn start(&self) -> &DigWriteEvidence {
        &self.start
    }
    /// STOP evidence.
    pub fn stop(&self) -> &DigWriteEvidence {
        &self.stop
    }
    /// ABORT evidence.
    pub fn abort(&self) -> &DigWriteEvidence {
        &self.abort
    }
    fn get_mut(&mut self, action: Action) -> &mut DigWriteEvidence {
        match action {
            Action::Start => &mut self.start,
            Action::Finish { .. } => &mut self.stop,
            Action::Abort => &mut self.abort,
        }
    }
}
/// Current local owner phase; snapshots do not certify task exit.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum DigPhase {
    /// Preparing the original snapshot.
    Preparing,
    /// Validating or writing START.
    Starting,
    /// Waiting the original estimate.
    Waiting,
    /// Validating or writing STOP.
    Stopping,
    /// Attempting the single original-context ABORT.
    CleaningUp,
    /// Observing the target after dispatch.
    Observing,
    /// The owner has actually been joined.
    Finished,
}
/// Last local await boundary or completed milestone, interpreted with DigPhase.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum DigStage {
    /// Waiting for the initial native snapshot or readiness.
    Snapshot,
    /// Waiting for native send queue capacity.
    Capacity,
    /// Waiting for the coherence/state gate.
    Gate,
    /// Coherence/state gate acquired.
    GateAcquired,
    /// Waiting for the native writer.
    Writer,
    /// Native writer acquired.
    WriterAcquired,
    /// Reading and validating original prerequisites.
    Validate,
    /// At the transport commitment boundary; consult dispatch evidence for admission.
    Write,
    /// A complete local write was proved.
    Written,
    /// Original estimate is available in the snapshot.
    Estimate,
    /// Waiting the estimate before STOP.
    Wait,
    /// Estimate wait completed.
    Waited,
    /// Awaiting a target capture.
    Capture,
    /// Target capture completed.
    Captured,
    /// Reading processed-boundary rejection evidence.
    Rejection,
}
/// First occurrence of a phase/stage pair. Repeated capture polling is coalesced.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct DigStageRecord {
    /// Owner phase at this boundary.
    pub phase: DigPhase,
    /// Boundary or milestone.
    pub stage: DigStage,
    /// Monotonic elapsed time since this invocation was registered.
    pub elapsed: std::time::Duration,
}
/// Terminal disposition, independent of result and owner exit.
///
/// Proved STOP, instant START, and eligible ABORT take precedence over failures;
/// then proved START suppression, context change, disconnect, and uncertainty.
/// Diagnostic strings may change: branch on typed variants and write evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DigCancelOutcome {
    /// START definitely was not sent, including admission failures.
    StartSuppressed,
    /// ABORT Written and STOP NotSent. This does not undo server effects.
    /// On 1.16.1 ABORT cannot clear a delayed break scheduled by an earlier STOP.
    Aborted,
    /// STOP Written, even when cancellation was requested before it finished.
    AlreadyStopped,
    /// START Written with ticks == 0; no STOP or cancellation ABORT.
    InstantBreak,
    /// Target changed before STOP; no claim about who changed it.
    Completed,
    /// A known failure with no stronger disposition.
    Failed {
        /// Stable error category.
        kind: ErrorKind,
        /// Unstable diagnostic text.
        message: String,
    },
    /// Original world/mode/session changed; no ABORT into the replacement.
    ContextChanged,
    /// The transport disconnected. Earlier write evidence remains meaningful.
    Disconnected,
    /// Unresolved writes or abnormal task exit.
    Unknown {
        /// Unstable diagnostic text.
        reason: String,
    },
}
/// Owner result, separate from local packet disposition.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DigResult {
    /// Normal observation completed, including after a late cancellation request.
    Finished(DigRecord),
    /// Cooperative cancellation was observed; inspect disposition and owner exit.
    Cancelled,
    /// Original failure; cleanup failures are retained separately on the completion.
    Failed {
        /// Stable category.
        kind: ErrorKind,
        /// Unstable diagnostic text.
        message: String,
    },
}
/// How the actual owner task exited, reported only after joining it.
/// New variants may be added; downstream matches must include a wildcard.
///
/// ```compile_fail
/// use voxrig::client::DigOwnerExit;
/// fn exhaustive(exit: DigOwnerExit) {
///     match exit {
///         DigOwnerExit::Returned => {},
///         DigOwnerExit::Cancelled { .. } => {},
///         DigOwnerExit::Panicked => {},
///     }
/// }
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DigOwnerExit {
    /// The owner returned normally (possibly with an error).
    Returned,
    /// Tokio cancelled the owner.
    Cancelled {
        /// Cancellation was requested as part of connection teardown.
        connection_teardown: bool,
    },
    /// The owner panicked.
    Panicked,
}
/// Immutable, invocation-bound receipt of the owner's joined exit.
///
/// `Finished` results require `Returned`; `Cancelled` results require cooperative
/// cancellation and `Returned`. Abnormal exits have `Failed` results, but retain
/// proved write dispositions. No terminal evidence is `Pending`. This is not a
/// shutdown receipt: use `Client::wait_shutdown` for connection retirement.
#[derive(Clone, Debug, PartialEq)]
pub struct DigCompletion {
    id: DigOperationId,
    session: Option<SessionStamp>,
    dispatch: DigDispatchEvidence,
    cancel_outcome: DigCancelOutcome,
    result: DigResult,
    owner_exit: DigOwnerExit,
    cancel_requested: bool,
    cleanup_failure: Option<(ErrorKind, String)>,
    progress: DigSnapshot,
}
impl DigCompletion {
    /// Frozen progress immediately before joined publication, including deadline evidence.
    pub fn snapshot(&self) -> &DigSnapshot {
        &self.progress
    }

    /// Invocation identity.
    pub fn id(&self) -> DigOperationId {
        self.id
    }
    /// Original session, if snapshot preparation completed.
    pub fn session(&self) -> Option<SessionStamp> {
        self.session
    }
    /// Sealed write evidence.
    pub fn dispatch(&self) -> &DigDispatchEvidence {
        &self.dispatch
    }
    /// Packet disposition; not proof of unchanged server state.
    pub fn cancel_outcome(&self) -> &DigCancelOutcome {
        &self.cancel_outcome
    }
    /// Original operation result.
    pub fn result(&self) -> &DigResult {
        &self.result
    }
    /// Actual joined task exit.
    pub fn owner_exit(&self) -> &DigOwnerExit {
        &self.owner_exit
    }
    /// Whether cancellation was requested before publication.
    pub fn cancel_requested(&self) -> bool {
        self.cancel_requested
    }
    /// Cleanup failure category, separate from the original result.
    pub fn cleanup_error_kind(&self) -> Option<ErrorKind> {
        self.cleanup_failure.as_ref().map(|e| e.0)
    }
    /// Unstable cleanup diagnostic; do not branch on its wording.
    pub fn cleanup_message(&self) -> Option<&str> {
        self.cleanup_failure.as_ref().map(|e| e.1.as_str())
    }
}
/// Point-in-time state. Only `finished()` supplies a joined receipt.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct DigSnapshot {
    target: Option<[i32; 3]>,
    face: Option<super::BlockFace>,
    id: DigOperationId,
    session: Option<SessionStamp>,
    phase: DigPhase,
    cancel_requested: bool,
    dispatch: DigDispatchEvidence,
    #[serde(skip)]
    finished: Option<Arc<DigCompletion>>,
    stage: DigStage,
    stages: Vec<DigStageRecord>,
    estimate: Option<super::DigEstimate>,
    deadline_exceeded: bool,
}
impl std::fmt::Display for DigSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "dig {:?} {:?} {:?} at {:?}/{:?}, deadline_exceeded={}, dispatch={:?}",
            self.id,
            self.target,
            self.face,
            self.phase,
            self.stage,
            self.deadline_exceeded,
            self.dispatch
        )
    }
}
impl std::error::Error for DigSnapshot {}
impl DigSnapshot {
    /// Original target, available before any native state lock is acquired.
    pub fn target(&self) -> Option<[i32; 3]> {
        self.target
    }
    /// Original face, never rederived from a later scene.
    pub fn face(&self) -> Option<super::BlockFace> {
        self.face
    }
    /// Most recent boundary, including a blocked await.
    pub fn stage(&self) -> DigStage {
        self.stage
    }
    /// Bounded history of first phase/stage occurrences.
    pub fn stages(&self) -> &[DigStageRecord] {
        &self.stages
    }
    /// Original timing estimate, if preparation completed.
    pub fn estimate(&self) -> Option<&super::DigEstimate> {
        self.estimate.as_ref()
    }
    /// The plain dig's immutable internal deadline expired; never success evidence.
    pub fn deadline_exceeded(&self) -> bool {
        self.deadline_exceeded
    }

    /// Invocation identity.
    pub fn id(&self) -> DigOperationId {
        self.id
    }
    /// Original session, if captured.
    pub fn session(&self) -> Option<SessionStamp> {
        self.session
    }
    /// Local phase.
    pub fn phase(&self) -> DigPhase {
        self.phase
    }
    /// Latched cancellation request.
    pub fn cancel_requested(&self) -> bool {
        self.cancel_requested
    }
    /// Provisional write evidence.
    pub fn dispatch(&self) -> &DigDispatchEvidence {
        &self.dispatch
    }
    /// Shared joined receipt, absent until the registry joins the actual owner.
    pub fn finished(&self) -> Option<&Arc<DigCompletion>> {
        self.finished.as_ref()
    }
}

/// Explicit control of one dig, without retaining the live client/connection.
///
/// **Dropping all handles or any waiter futures does NOT cancel: digging continues.**
/// Call `request_cancel` explicitly. Callers own cancellation deadlines; the SDK adds no cancellation timeout
/// or retries. Cancellation before STOP commitment prevents STOP and requests one
/// eligible original-context ABORT. After commitment STOP is allowed to finish;
/// `AlreadyStopped` means complete local STOP, never rollback. Even ABORT Written
/// cannot clear a 1.16.1 server's already-scheduled delayed break.
#[derive(Clone, Debug)]
pub struct DigOperation {
    pub(crate) state: Arc<OperationState>,
}
impl DigOperation {
    /// Identity of this invocation, never reused on its transport.
    pub fn id(&self) -> DigOperationId {
        self.state.id
    }
    /// Latch cancellation and wake the owner; no adapter locks or I/O.
    pub fn request_cancel(&self) {
        self.state.request_cancel();
    }
    /// Read current per-invocation evidence synchronously.
    pub fn snapshot(&self) -> DigSnapshot {
        self.state.snapshot()
    }
    /// Request cancellation **before returning the future**, then await joined exit.
    /// Dropping or never polling this future does not retract the request.
    pub fn cancel(&self) -> impl Future<Output = Arc<DigCompletion>> + Send + '_ {
        self.request_cancel();
        self.finished()
    }
    /// Passively await the same immutable joined receipt for every reader.
    /// Dropping/timing out this future does not cancel the dig. This is not a
    /// connection shutdown receipt; use `Client::wait_shutdown` separately.
    pub fn finished(&self) -> impl Future<Output = Arc<DigCompletion>> + Send + '_ {
        async move {
            loop {
                let changed = self.state.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if let Some(done) = self.state.snapshot().finished {
                    return done;
                }
                changed.await;
            }
        }
    }
}

#[derive(Debug)]
struct State {
    target: Option<[i32; 3]>,
    face: Option<super::BlockFace>,
    session: Option<SessionStamp>,
    phase: DigPhase,
    dispatch: DigDispatchEvidence,
    instant: bool,
    attempted: [bool; 3],
    cleanup_started: bool,
    cooperative: bool,
    context_changed: bool,
    cleanup_failure: Option<(ErrorKind, String)>,
    finished: Option<Arc<DigCompletion>>,
    stage: DigStage,
    stages: Vec<DigStageRecord>,
    estimate: Option<super::DigEstimate>,
    deadline_exceeded: bool,
    plain: bool,
}
#[derive(Debug)]
pub(crate) struct OperationState {
    id: DigOperationId,
    pub(crate) cancelled: Arc<AtomicBool>,
    inner: Mutex<State>,
    changed: Notify,
    started: tokio::time::Instant,
}
impl OperationState {
    pub(crate) fn new(id: DigOperationId) -> Arc<Self> {
        Arc::new(Self {
            id,
            cancelled: Arc::new(AtomicBool::new(false)),
            changed: Notify::new(),
            started: tokio::time::Instant::now(),
            inner: Mutex::new(State {
                target: None,
                face: None,
                session: None,
                phase: DigPhase::Preparing,
                dispatch: DigDispatchEvidence {
                    start: DigWriteEvidence::NotSent,
                    stop: DigWriteEvidence::NotSent,
                    abort: DigWriteEvidence::NotSent,
                },
                instant: false,
                attempted: [false; 3],
                cleanup_started: false,
                cooperative: false,
                context_changed: false,
                cleanup_failure: None,
                finished: None,
                stage: DigStage::Snapshot,
                stages: Vec::new(),
                estimate: None,
                deadline_exceeded: false,
                plain: false,
            }),
        })
    }
    pub(crate) fn plain(id: DigOperationId) -> Arc<Self> {
        let operation = Self::new(id);
        operation.lock().plain = true;
        operation
    }
    pub(crate) fn is_plain(&self) -> bool {
        self.lock().plain
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub(crate) fn request_cancel(&self) {
        let state = self.lock();
        if state.finished.is_none() {
            self.cancelled.store(true, Ordering::Release);
        }
        drop(state);
        self.changed.notify_waiters();
    }
    pub(crate) async fn cancellation(&self) {
        loop {
            let wake = self.changed.notified();
            tokio::pin!(wake);
            wake.as_mut().enable();
            if self.cancelled.load(Ordering::Acquire) {
                return;
            }
            wake.await;
        }
    }
    pub(crate) fn snapshot(&self) -> DigSnapshot {
        self.snapshot_locked(&self.lock())
    }
    fn snapshot_locked(&self, state: &State) -> DigSnapshot {
        DigSnapshot {
            target: state.target,
            face: state.face,
            id: self.id,
            session: state.session,
            phase: state.phase,
            cancel_requested: self.cancelled.load(Ordering::Acquire),
            dispatch: state.dispatch.clone(),
            finished: state.finished.clone(),
            stage: state.stage,
            stages: state.stages.clone(),
            estimate: state.estimate.clone(),
            deadline_exceeded: state.deadline_exceeded,
        }
    }
    pub(crate) fn stage(&self, stage: DigStage) {
        let mut state = self.lock();
        if state.finished.is_some() {
            return;
        }
        let phase = state.phase;
        state.stage = stage;
        let elapsed = self.started.elapsed();
        if state
            .stages
            .iter()
            .any(|r| r.phase == phase && r.stage == stage)
        {
            return;
        }
        state.stages.push(DigStageRecord {
            phase,
            stage,
            elapsed,
        });
        let dispatch = state.dispatch.clone();
        drop(state);
        crate::lifecycle::emit_dig_lifecycle(|| {
            serde_json::json!({
                "stage": "operational_stage", "operation": self.id,
                "phase": phase, "boundary": stage, "elapsed_ms": elapsed.as_millis(),
                "dispatch": dispatch,
            })
        });
    }
    pub(crate) fn estimate(&self, estimate: &super::DigEstimate) {
        self.lock().estimate = Some(estimate.clone());
        self.stage(DigStage::Estimate);
        crate::lifecycle::emit_dig_lifecycle(|| {
            serde_json::json!({
                "stage": "operational_estimate", "operation": self.id, "estimate": estimate,
            })
        });
    }
    pub(crate) fn deadline(&self) -> DigSnapshot {
        self.lock().deadline_exceeded = true;
        let snapshot = self.snapshot();
        crate::lifecycle::emit_dig_lifecycle(|| {
            serde_json::json!({
                "stage": "operational_deadline", "snapshot": snapshot,
            })
        });
        snapshot
    }
    pub(crate) fn target(&self, target: [i32; 3], face: super::BlockFace) {
        let mut state = self.lock();
        state.target = Some(target);
        state.face = Some(face);
    }
    pub(crate) fn prepare(&self, session: SessionStamp, instant: bool) {
        let mut state = self.lock();
        state.session = Some(session);
        state.instant = instant;
    }
    pub(crate) fn phase(&self, phase: DigPhase) {
        self.lock().phase = phase;
    }
    pub(crate) fn begin_cleanup(&self) -> bool {
        let mut state = self.lock();
        if state.cleanup_started {
            return false;
        }
        state.cleanup_started = true;
        state.phase = DigPhase::CleaningUp;
        true
    }
    pub(crate) fn cooperative(&self) {
        self.lock().cooperative = true;
    }
    pub(crate) fn context_changed(&self) {
        self.lock().context_changed = true;
    }
    pub(crate) fn cleanup_failure(&self, error: &crate::Error) {
        self.lock().cleanup_failure = Some((error.kind(), error.to_string()));
    }
    /// Pamo's approved Q2 boundary is STOP committed, not validation or future
    /// construction. This single gate orders cancellation against BOTH START and
    /// STOP. Adapters call it immediately before enqueue/write, under their send
    /// serialization, with no intervening await before the non-retractable hand-off.
    /// ABORT uses the same gate to allow at most one attempt, only with STOP NotSent.
    pub(crate) fn try_commit(&self, action: Action) -> bool {
        let mut state = self.lock();
        if state.finished.is_some() {
            return false;
        }
        if matches!(action, Action::Abort) {
            if state.dispatch.stop != DigWriteEvidence::NotSent
                || !written(&state.dispatch.start)
                || state.instant
            {
                return false;
            }
        } else if self.cancelled.load(Ordering::Acquire) {
            state.cooperative = true;
            return false;
        }
        let index = action.status() as usize;
        if state.attempted[index] {
            return false;
        }
        state.attempted[index] = true;
        let evidence = state.dispatch.get_mut(action);
        if *evidence != DigWriteEvidence::NotSent {
            return false;
        }
        *evidence = DigWriteEvidence::Pending;
        true
    }
    pub(crate) fn evidence(&self, action: Action) -> DigWriteEvidence {
        self.lock().dispatch.get_mut(action).clone()
    }
    pub(crate) fn settle(&self, action: Action, evidence: DigWriteEvidence) {
        let mut state = self.lock();
        if state.finished.is_none() && *state.dispatch.get_mut(action) == DigWriteEvidence::Pending
        {
            let complete = matches!(evidence, DigWriteEvidence::Written { .. });
            *state.dispatch.get_mut(action) = evidence;
            drop(state);
            if complete {
                self.stage(DigStage::Written);
            }
        }
    }
    pub(crate) fn publish(
        &self,
        joined: &std::result::Result<Result<DigRecord>, tokio::task::JoinError>,
        teardown: bool,
    ) {
        let mut state = self.lock();
        if state.finished.is_some() {
            return;
        }
        for action in [
            Action::Start,
            Action::Finish {
                elapsed: std::time::Duration::ZERO,
            },
            Action::Abort,
        ] {
            let evidence = state.dispatch.get_mut(action);
            if *evidence == DigWriteEvidence::Pending {
                *evidence = DigWriteEvidence::Unknown;
            }
        }
        let (result, exit) = match joined {
            Ok(Ok(record)) => (DigResult::Finished(record.clone()), DigOwnerExit::Returned),
            Ok(Err(_))
                if state.cooperative
                    && state.cleanup_failure.is_none()
                    && !state.context_changed
                    && (state.dispatch.start == DigWriteEvidence::NotSent
                        || (written(&state.dispatch.abort)
                            && state.dispatch.stop == DigWriteEvidence::NotSent)) =>
            {
                (DigResult::Cancelled, DigOwnerExit::Returned)
            }
            Ok(Err(error)) => (
                DigResult::Failed {
                    kind: error.kind(),
                    message: error.to_string(),
                },
                DigOwnerExit::Returned,
            ),
            Err(error) => (
                DigResult::Failed {
                    kind: ErrorKind::OperationUnconfirmed,
                    message: error.to_string(),
                },
                if error.is_panic() {
                    DigOwnerExit::Panicked
                } else {
                    DigOwnerExit::Cancelled {
                        connection_teardown: teardown,
                    }
                },
            ),
        };
        let outcome = classify(&state, &result, &exit);
        state.finished = Some(Arc::new(DigCompletion {
            id: self.id,
            session: state.session,
            dispatch: state.dispatch.clone(),
            cancel_outcome: outcome,
            result,
            owner_exit: exit,
            cancel_requested: self.cancelled.load(Ordering::Acquire),
            cleanup_failure: state.cleanup_failure.clone(),
            progress: self.snapshot_locked(&state),
        }));
        state.phase = DigPhase::Finished;
        drop(state);
        self.changed.notify_waiters();
    }
}
fn written(evidence: &DigWriteEvidence) -> bool {
    matches!(evidence, DigWriteEvidence::Written { .. })
}
fn classify(state: &State, result: &DigResult, exit: &DigOwnerExit) -> DigCancelOutcome {
    if written(&state.dispatch.stop) {
        return DigCancelOutcome::AlreadyStopped;
    }
    if state.instant && written(&state.dispatch.start) {
        return DigCancelOutcome::InstantBreak;
    }
    if written(&state.dispatch.start)
        && !state.instant
        && written(&state.dispatch.abort)
        && state.dispatch.stop == DigWriteEvidence::NotSent
    {
        return DigCancelOutcome::Aborted;
    }
    if state.dispatch.start == DigWriteEvidence::NotSent {
        return DigCancelOutcome::StartSuppressed;
    }
    if state.context_changed {
        return DigCancelOutcome::ContextChanged;
    }
    if matches!(
        result,
        DigResult::Failed {
            kind: ErrorKind::Disconnected | ErrorKind::Connection,
            ..
        }
    ) || matches!(
        state.cleanup_failure,
        Some((ErrorKind::Disconnected | ErrorKind::Connection, _))
    ) || matches!(
        exit,
        DigOwnerExit::Cancelled {
            connection_teardown: true
        }
    ) {
        return DigCancelOutcome::Disconnected;
    }
    if [
        state.dispatch.start(),
        state.dispatch.stop(),
        state.dispatch.abort(),
    ]
    .contains(&&DigWriteEvidence::Unknown)
        || *exit != DigOwnerExit::Returned
    {
        return DigCancelOutcome::Unknown {
            reason: "write or owner exit unresolved".into(),
        };
    }
    match result {
        DigResult::Finished(_) => DigCancelOutcome::Completed,
        DigResult::Failed { kind, message } => DigCancelOutcome::Failed {
            kind: *kind,
            message: message.clone(),
        },
        DigResult::Cancelled => DigCancelOutcome::Unknown {
            reason: "cancellation without confirmed interruption".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{BlockFace, Survival, shutdown::Registry};
    use std::{future::Future, time::Duration};

    fn operation() -> DigOperation {
        DigOperation {
            state: OperationState::new(DigOperationId::new(
                GenerationKey::new(crate::MinecraftVersion::Java1_16_1, 7),
                1,
            )),
        }
    }
    const STOP: Action = Action::Finish {
        elapsed: Duration::ZERO,
    };
    fn write(state: &OperationState, action: Action) {
        assert!(state.try_commit(action));
        state.settle(
            action,
            DigWriteEvidence::Written {
                interaction_sequence: None,
            },
        );
    }
    #[test]
    fn dig_handle_public_traits_and_futures_are_send_without_native() {
        fn traits<T: Send + Sync + Clone + std::fmt::Debug>() {}
        traits::<DigOperation>();
        traits::<DigOperationId>();
        traits::<DigSnapshot>();
        traits::<DigCompletion>();
        traits::<DigWriteEvidence>();
        traits::<DigDispatchEvidence>();
        traits::<DigPhase>();
        traits::<DigCancelOutcome>();
        traits::<DigResult>();
        traits::<DigOwnerExit>();
        fn send(_: impl Future + Send) {}
        let op = operation();
        send(op.finished());
        send(op.cancel());
        fn signatures(survival: &Survival) {
            let _: crate::Result<crate::client::prelude::DigOperation> =
                survival.begin_dig([0; 3], BlockFace::Up);
        }
        let _ = signatures;
    }
    #[test]
    fn dig_start_and_stop_commit_cancel_orders_are_atomic() {
        for action in [Action::Start, STOP] {
            let op = operation();
            op.request_cancel();
            assert!(!op.state.try_commit(action));
            assert_eq!(op.state.evidence(action), DigWriteEvidence::NotSent);
            let op = operation();
            assert!(op.state.try_commit(action));
            op.request_cancel();
            assert_eq!(op.state.evidence(action), DigWriteEvidence::Pending);
            assert!(!op.state.try_commit(action));
        }
    }
    #[test]
    fn dig_cancel_and_commit_barrier_race_has_only_two_outcomes() {
        for action in [Action::Start, STOP] {
            for _ in 0..16 {
                let op = operation();
                let gate = Arc::new(std::sync::Barrier::new(2));
                let other = op.clone();
                let ready = gate.clone();
                let thread = std::thread::spawn(move || {
                    ready.wait();
                    other.request_cancel();
                });
                gate.wait();
                let committed = op.state.try_commit(action);
                thread.join().unwrap();
                assert_eq!(
                    op.state.evidence(action),
                    if committed {
                        DigWriteEvidence::Pending
                    } else {
                        DigWriteEvidence::NotSent
                    }
                );
                assert!(op.snapshot().cancel_requested());
            }
        }
    }
    #[test]
    fn dig_concurrent_cancels_share_a_single_cleanup_latch() {
        let op = operation();
        write(&op.state, Action::Start);
        let barrier = Arc::new(std::sync::Barrier::new(5));
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let cloned = op.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    cloned.request_cancel();
                })
            })
            .collect();
        barrier.wait();
        for thread in threads {
            thread.join().unwrap();
        }
        assert!(!op.state.try_commit(STOP));
        assert!(op.state.begin_cleanup());
        assert!(!op.state.begin_cleanup());
        write(&op.state, Action::Abort);
        assert!(!op.state.try_commit(Action::Abort));
        op.state
            .publish(&Ok(Err(crate::error::state_error("cancelled"))), false);
        let done = op.snapshot().finished.unwrap();
        assert!(matches!(done.result(), DigResult::Cancelled));
        assert_eq!(*done.cancel_outcome(), DigCancelOutcome::Aborted);
    }

    #[test]
    fn dig_outcome_precedence_preserves_evidence_and_original_failure() {
        for (actions, instant, expected) in [
            (vec![], false, DigCancelOutcome::StartSuppressed),
            (vec![Action::Start], true, DigCancelOutcome::InstantBreak),
            (
                vec![Action::Start, STOP],
                false,
                DigCancelOutcome::AlreadyStopped,
            ),
            (
                vec![Action::Start, Action::Abort],
                false,
                DigCancelOutcome::Aborted,
            ),
        ] {
            let op = operation();
            op.state.lock().instant = instant;
            for action in actions {
                write(&op.state, action);
            }
            op.state.context_changed();
            op.state.publish(
                &Ok(Err(crate::error::state_error("original refusal"))),
                false,
            );
            let done = op.snapshot().finished.unwrap();
            assert_eq!(*done.cancel_outcome(), expected);
            assert!(matches!(
                done.result(),
                DigResult::Failed {
                    kind: ErrorKind::State,
                    ..
                }
            ));
            assert_eq!(*done.owner_exit(), DigOwnerExit::Returned);
        }
        let op = operation();
        write(&op.state, Action::Start);
        assert!(op.state.try_commit(STOP));
        assert!(!op.state.try_commit(Action::Abort));
        op.state
            .publish(&Ok(Err(crate::error::state_error("reply lost"))), false);
        assert_eq!(*op.snapshot().dispatch().stop(), DigWriteEvidence::Unknown);
        assert!(matches!(
            op.snapshot().finished.unwrap().cancel_outcome(),
            DigCancelOutcome::Unknown { .. }
        ));
        // A later actor reply cannot mutate frozen completion or snapshots.
        op.state.settle(
            STOP,
            DigWriteEvidence::Written {
                interaction_sequence: None,
            },
        );
        assert_eq!(*op.snapshot().dispatch().stop(), DigWriteEvidence::Unknown);
    }
    #[test]
    fn dig_abort_requires_stop_definitely_not_sent_and_one_attempt() {
        let op = operation();
        write(&op.state, Action::Start);
        assert!(op.state.try_commit(Action::Abort));
        assert!(!op.state.try_commit(Action::Abort));
        op.state.settle(Action::Abort, DigWriteEvidence::Unknown);
        op.state.publish(
            &Ok(Err(crate::error::state_error("cleanup uncertain"))),
            false,
        );
        assert!(matches!(
            op.snapshot().finished.unwrap().cancel_outcome(),
            DigCancelOutcome::Unknown { .. }
        ));
        let op = operation();
        write(&op.state, Action::Start);
        op.state.context_changed();
        op.state
            .publish(&Ok(Err(crate::error::state_error("world changed"))), false);
        assert_eq!(
            *op.snapshot().finished.unwrap().cancel_outcome(),
            DigCancelOutcome::ContextChanged
        );
    }
    #[test]
    fn dig_cleanup_failure_is_separate_and_late_cancel_cannot_hide_original_refusal() {
        let op = operation();
        write(&op.state, Action::Start);
        let original = crate::Error::new(
            ErrorKind::Rejected,
            anyhow::anyhow!("original STOP refusal"),
        );
        let cleanup = crate::Error::new(
            ErrorKind::UncertainDispatch,
            anyhow::anyhow!("cleanup flush failed"),
        );
        assert!(op.state.begin_cleanup());
        assert!(op.state.try_commit(Action::Abort));
        op.state.settle(Action::Abort, DigWriteEvidence::Unknown);
        op.state.cleanup_failure(&cleanup);
        op.request_cancel();
        op.state.publish(&Ok(Err(original)), false);
        let done = op.snapshot().finished.unwrap();
        assert!(
            matches!(done.result(), DigResult::Failed { kind: ErrorKind::Rejected, message } if message == "original STOP refusal")
        );
        assert_eq!(
            done.cleanup_error_kind(),
            Some(ErrorKind::UncertainDispatch)
        );
        assert_eq!(done.cleanup_message(), Some("cleanup flush failed"));
        assert!(matches!(
            done.cancel_outcome(),
            DigCancelOutcome::Unknown { .. }
        ));
    }

    #[tokio::test]
    async fn dig_cancel_is_eager_waiters_passive_and_receipt_is_shared() {
        let op = operation();
        let mut passive = Box::pin(op.finished());
        std::future::poll_fn(|cx| {
            assert!(passive.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        drop(passive);
        assert!(!op.snapshot().cancel_requested());
        drop(op.cancel());
        assert!(op.snapshot().cancel_requested());
        assert!(op.snapshot().finished().is_none());
        op.state.cooperative();
        op.state
            .publish(&Ok(Err(crate::error::state_error("cancelled"))), false);
        let (a, b) = tokio::join!(op.cancel(), op.finished());
        assert!(Arc::ptr_eq(&a, &b));
        assert!(Arc::ptr_eq(&a, &op.cancel().await));
        assert!(matches!(a.result(), DigResult::Cancelled));
    }
    #[tokio::test]
    async fn dig_registry_publishes_only_after_join_on_return_panic_and_teardown() {
        for exit in [0, 1, 2] {
            let op = operation();
            let registry =
                Registry::new(GenerationKey::new(crate::MinecraftVersion::Java1_16_1, 7));
            let (release, wait) = tokio::sync::oneshot::channel::<()>();
            let state = op.state.clone();
            let task = registry
                .try_spawn_with_join(
                    async move {
                        wait.await.unwrap();
                        if exit == 1 {
                            panic!("injected dig panic");
                        }
                        if exit == 2 {
                            std::future::pending::<()>().await;
                        }
                        Err(crate::error::state_error("injected original failure"))
                    },
                    move |joined, teardown| state.publish(joined, teardown),
                )
                .unwrap();
            assert!(op.snapshot().finished().is_none());
            if exit == 2 {
                registry.start(async { Ok(()) });
            } else {
                release.send(()).unwrap();
            }
            let _ = task.await;
            let done = op.finished().await;
            assert_eq!(
                *done.owner_exit(),
                match exit {
                    0 => DigOwnerExit::Returned,
                    1 => DigOwnerExit::Panicked,
                    _ => DigOwnerExit::Cancelled {
                        connection_teardown: true
                    },
                }
            );
            assert!(matches!(done.result(), DigResult::Failed { .. }));
            if exit != 2 {
                registry.start(async { Ok(()) });
            }
            let result = registry.wait().await;
            assert_eq!(result.is_ok(), exit != 1);
        }
    }
    #[tokio::test]
    async fn dig_proved_abort_survives_panicked_owner_and_refused_abort_cannot_retry() {
        let op = operation();
        write(&op.state, Action::Start);
        write(&op.state, Action::Abort);
        let registry = Registry::new(GenerationKey::new(crate::MinecraftVersion::Java1_16_1, 7));
        let state = op.state.clone();
        let task = registry
            .try_spawn_with_join(
                async { panic!("after ABORT write") },
                move |joined, teardown| state.publish(joined, teardown),
            )
            .unwrap();
        assert!(task.await.is_err());
        let done = op.finished().await;
        assert_eq!(*done.cancel_outcome(), DigCancelOutcome::Aborted);
        assert_eq!(*done.owner_exit(), DigOwnerExit::Panicked);
        assert!(matches!(done.result(), DigResult::Failed { .. }));
        registry.start(async { Ok(()) });
        assert!(registry.wait().await.is_err());
        let op = operation();
        write(&op.state, Action::Start);
        assert!(op.state.begin_cleanup());
        assert!(!op.state.begin_cleanup());
        assert!(op.state.try_commit(Action::Abort));
        op.state.settle(Action::Abort, DigWriteEvidence::NotSent);
        assert!(!op.state.try_commit(Action::Abort));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dig_completion_absent_until_owner_destructor_and_finite_work_finalize() {
        let op = operation();
        let ledger =
            crate::client::ledger::Ledger::tracking(crate::client::ledger::GenerationKey {
                version: crate::MinecraftVersion::Java1_16_1,
                connection_id: 7,
            });
        let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let (dropping, dropped) = tokio::sync::oneshot::channel();
        let release = gate.clone();
        let guard = crate::client::shutdown::OnExit(Some(move || {
            dropping.send(()).unwrap();
            let _held = release
                .1
                .wait_while(release.0.lock().unwrap(), |ready| !*ready)
                .unwrap();
        }));
        let state = op.state.clone();
        let task = ledger
            .tasks()
            .try_spawn_with_join(
                ledger.work().run_result(async move {
                    let _guard = guard;
                    Err(crate::error::state_error("done"))
                }),
                move |joined, teardown| state.publish(joined, teardown),
            )
            .unwrap();
        dropped.await.unwrap();
        assert!(op.snapshot().finished().is_none());
        assert!(ledger.cut(false).inspect().is_none());
        *gate.0.lock().unwrap() = true;
        gate.1.notify_one();
        let _ = task.await;
        assert!(op.snapshot().finished().is_some());
        assert!(ledger.cut(false).inspect().is_some());
        ledger.tasks().start(async { Ok(()) });
        ledger.tasks().wait().await.unwrap();
    }
    #[tokio::test(start_paused = true)]
    async fn dig_bounded_wait_preserves_owner_and_projects_terminal_vocabulary() {
        use crate::client::{OperationOutcome as O, OperationUnknownReason as U};
        let op = operation();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        let DigWait::Unknown(snapshot) = op.finished_until(deadline).await else {
            panic!("owner must remain live");
        };
        assert_eq!(snapshot.id(), op.id());
        assert_eq!(snapshot.id().generation().connection_id(), 7);
        assert_eq!(snapshot.id().ordinal(), 1);
        assert!(!snapshot.cancel_requested());
        assert!(snapshot.finished().is_none());
        op.state
            .publish(&Ok(Err(crate::error::state_error("not admitted"))), false);
        let DigWait::Completed(done) = op.finished_until(tokio::time::Instant::now()).await else {
            panic!("joined result must be immediately available");
        };
        assert_eq!(done.operation_outcome(), O::NotSent);
        let op = operation();
        write(&op.state, Action::Start);
        op.state
            .publish(&Ok(Err(crate::error::state_error("no receipt"))), false);
        assert_eq!(op.finished().await.operation_outcome(), O::SentUnconfirmed);
        let op = operation();
        assert!(op.state.try_commit(Action::Start));
        op.state
            .publish(&Ok(Err(crate::error::state_error("interrupted"))), false);
        assert_eq!(
            op.finished().await.operation_outcome(),
            O::Unknown(U::Failed)
        );
    }
}

/// Finite observation using the existing DigOperation identity and owner.
#[derive(Clone, Debug)]
pub enum DigWait {
    /// Original joined completion, including independent START/STOP/ABORT evidence.
    Completed(Arc<DigCompletion>),
    /// Deadline elapsed; no cancellation, join, seal or server-effect proof.
    Unknown(DigSnapshot),
}
impl DigOperationId {
    /// Original immutable connection generation.
    pub fn generation(self) -> GenerationKey {
        self.generation
    }
    /// Monotonic invocation number within that generation.
    pub fn ordinal(self) -> u64 {
        self.ordinal
    }
}
impl DigOperation {
    /// Bounded passive observation. Use request_cancel explicitly, and retire the
    /// connection if cleanup remains unconfirmed. Does not create a second owner.
    pub async fn finished_until(&self, deadline: tokio::time::Instant) -> DigWait {
        match tokio::time::timeout_at(deadline, self.finished()).await {
            Ok(done) => DigWait::Completed(done),
            Err(_) => DigWait::Unknown(self.snapshot()),
        }
    }
}
impl DigCompletion {
    /// Same terminal vocabulary as owned operations; detailed per-packet
    /// evidence and the original strict DigResult remain authoritative.
    pub fn operation_outcome(&self) -> super::OperationOutcome {
        use super::{OperationOutcome as O, OperationUnknownReason as U};
        if self.progress.deadline_exceeded {
            return O::Unknown(U::Deadline);
        }
        let writes = [
            &self.dispatch.start,
            &self.dispatch.stop,
            &self.dispatch.abort,
        ];
        if writes.iter().all(|w| **w == DigWriteEvidence::NotSent) {
            return O::NotSent;
        }
        if writes
            .iter()
            .any(|w| matches!(w, DigWriteEvidence::Unknown | DigWriteEvidence::Pending))
        {
            return O::Unknown(U::Failed);
        }
        match &self.result {
            DigResult::Finished(record)
                if !matches!(record.outcome, super::DigOutcome::Unconfirmed) =>
            {
                O::SentReceived
            }
            _ => O::SentUnconfirmed,
        }
    }
}
