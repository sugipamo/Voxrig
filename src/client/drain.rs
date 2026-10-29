//! Irreversible, transport-bound staged quiesce and explicit neutral stops.
// The approved inline StopError schema and explicit Send RPIT signatures are
// intentional public contracts; boxing errors or hiding Send changes that API.
#![allow(clippy::result_large_err, clippy::manual_async_fn)]
use super::adapter::DrainOps;
use super::{Client, GameMode, Hand, MountId, SessionStamp, ledger};
use std::future::Future;
use std::sync::{Arc, Mutex};

/// One process-local transport identity, unchanged by world resets.
/// Observation only; this key grants no admission or stop authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct GenerationKey {
    version: crate::MinecraftVersion,
    connection_id: u64,
}
impl GenerationKey {
    pub(crate) const fn new(version: crate::MinecraftVersion, connection_id: u64) -> Self {
        Self {
            version,
            connection_id,
        }
    }
    /// Exact adapter version; no I/O or errors.
    pub const fn version(self) -> crate::MinecraftVersion {
        self.version
    }
    /// Transport ID; meaningful only with its version and owning process/run.
    pub const fn connection_id(self) -> u64 {
        self.connection_id
    }
}

/// Transport-local admitted-root boundary, including all late descendants.
/// Zero denotes an empty prefix. Observation grants no authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, serde::Serialize)]
pub struct AdmissionId(u64);
impl AdmissionId {
    /// Numeric prefix; compare only within the same transport and process/run.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Contiguous socket-write-plus-flush prefix, never a server acknowledgement.
/// Zero denotes an empty prefix. Observation grants no authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, serde::Serialize)]
pub struct WriteOrdinal(u64);
impl WriteOrdinal {
    /// Numeric prefix; compare only within the same transport and process/run.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// The guarantee of one drain stage; neither scope proves physical stillness.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum DrainScope {
    /// Ordinary producers parked and work through the first cut completed; maintenance may still write.
    ProducersParked,
    /// All admission sealed and remaining write authority relinquished; no later Voxrig writes.
    SealedForFence,
}
/// Immutable historical completion fact, issued atomically with the write prefix.
/// Cannot authorize writes; remains valid after later failure or revocation.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct DrainReceipt {
    generation: GenerationKey,
    admitted_through: AdmissionId,
    written_through: WriteOrdinal,
    scope: DrainScope,
}
impl DrainReceipt {
    /// Owning transport, independent of world generation.
    pub const fn generation(&self) -> GenerationKey {
        self.generation
    }
    /// All roots through this cut and their late descendants completed cleanly.
    pub const fn admitted_through(&self) -> AdmissionId {
        self.admitted_through
    }
    /// Contiguous socket-write-plus-flush prefix at issue; never server acceptance.
    pub const fn written_through(&self) -> WriteOrdinal {
        self.written_through
    }
    /// First-stage or persistent final no-write guarantee.
    pub const fn scope(&self) -> DrainScope {
        self.scope
    }
}
/// Stable drain failure category. Terminal failures are sticky for unissued drains.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum DrainErrorKind {
    /// Supplied key differs from this transport; no write.
    WrongGeneration,
    /// First quiesce requires the initial Ready boundary.
    NotReady,
    /// A successful begin already owns this irreversible quiesce.
    AlreadyQuiescing,
    /// Stop authority or final seal requested before the first receipt.
    FirstDrainPending,
    /// Stop authority requested after the final seal.
    Sealed,
    /// Revocation won the ordering before receipt issue.
    RevokedBeforeDrain,
    /// Interrupted or partial frame; delivery is unknown.
    UncertainDispatch,
    /// Covered work lost its owner through drop, panic or task abort.
    OwnerLost,
    /// An existing required protocol or cleanup deadline was observed to expire.
    TransactionExpired,
    /// Concrete outcome, context or provenance loss prevents required confirmation.
    TransactionOutcomeUnknown,
    /// Required transaction or cleanup is unconfirmed without a more specific cause.
    TransactionUnconfirmed,
    /// A selected required neutral release failed or was refused.
    RequiredReleaseFailed,
    /// Covered work was aborted outside an intentional confirmed permit stop.
    WorkAborted,
    /// Transport closure preceded receipt issue; closure is not drain.
    Disconnected,
    /// Checked identity exhaustion or required registration resource failure.
    ResourceExhausted,
    /// Poisoned state or an invariant prevents reliable completion.
    Internal,
}
/// Structured failure diagnostics; diagnostic Display text is unstable.
/// The first terminal session cause persists; this is never a completion receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrainError {
    kind: DrainErrorKind,
    generation: GenerationKey,
    requested_generation: Option<GenerationKey>,
    admitted_through: Option<AdmissionId>,
    written_through: WriteOrdinal,
    failed_admission: Option<AdmissionId>,
}
impl DrainError {
    pub(crate) fn new(
        kind: DrainErrorKind,
        generation: GenerationKey,
        requested_generation: Option<GenerationKey>,
        cut: Option<u64>,
        written: u64,
        failed: Option<u64>,
    ) -> Self {
        Self {
            kind,
            generation,
            requested_generation,
            admitted_through: cut.map(AdmissionId),
            written_through: WriteOrdinal(written),
            failed_admission: failed.map(AdmissionId),
        }
    }
    /// Stable failure category.
    pub const fn kind(&self) -> DrainErrorKind {
        self.kind
    }
    /// Actual owning transport, also for a wrong-key request.
    pub const fn generation(&self) -> GenerationKey {
        self.generation
    }
    /// Caller-supplied key for WrongGeneration, otherwise None.
    pub const fn requested_generation(&self) -> Option<GenerationKey> {
        self.requested_generation
    }
    /// Fixed cut for stage failures; None before a cut exists.
    pub const fn admitted_through(&self) -> Option<AdmissionId> {
        self.admitted_through
    }
    /// Last certain contiguous prefix sampled for this error.
    pub const fn written_through(&self) -> WriteOrdinal {
        self.written_through
    }
    /// Attributable failed root, if known; absence does not imply success.
    pub const fn failed_admission(&self) -> Option<AdmissionId> {
        self.failed_admission
    }
}
impl std::fmt::Display for DrainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "drain {:?} on {:?}", self.kind, self.generation)
    }
}
impl std::error::Error for DrainError {}

/// Caller-selected held input family; inspection never automatically selects stops.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum StopTargetKind {
    /// Ground, flight, glide or explicitly selected native/passive movement input.
    Movement,
    /// Original item-use lifetime; release may cause gameplay effects, including firing a bow.
    ItemUse,
    /// Original mounted input, without dismount or predicted vehicle movement.
    VehicleInput,
    /// Original in-progress dig; intentional confirmed permit ABORT is clean completion.
    Digging,
}
/// Opaque original held-input binding. Copies grant no stop authority.
/// Stale world, mode or owner bindings never affect a replacement owner.
#[derive(Clone, Copy, Eq, PartialEq, serde::Serialize)]
pub struct StopTarget {
    #[serde(skip)]
    provenance: u64,
    kind: StopTargetKind,
    session: SessionStamp,
    mode: GameMode,
    binding_id: u64,
    control_session_id: Option<u64>,
    item_use_hand: Option<Hand>,
    mount: Option<MountId>,
    mining_id: Option<super::survival::MiningId>,
}
impl std::fmt::Debug for StopTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StopTarget")
            .field("kind", &self.kind)
            .field("session", &self.session)
            .field("mode", &self.mode)
            .field("binding_id", &self.binding_id)
            .field("control_session_id", &self.control_session_id)
            .field("item_use_hand", &self.item_use_hand)
            .field("mount", &self.mount)
            .field("mining_id", &self.mining_id)
            .finish_non_exhaustive()
    }
}

impl StopTarget {
    pub(crate) fn set_mining(&mut self, id: super::survival::MiningId) {
        self.mining_id = Some(id);
    }
    pub(crate) fn set_binding(&mut self, id: u64, provenance: u64) {
        self.binding_id = id;
        self.provenance = provenance;
    }
    pub(crate) fn belongs_to(&self, provenance: u64) -> bool {
        self.provenance == provenance
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        kind: StopTargetKind,
        session: SessionStamp,
        mode: GameMode,
        binding_id: u64,
        control_session_id: Option<u64>,
        item_use_hand: Option<Hand>,
        mount: Option<MountId>,
        mining_id: Option<super::survival::MiningId>,
    ) -> Self {
        Self {
            provenance: 0,
            kind,
            session,
            mode,
            binding_id,
            control_session_id,
            item_use_hand,
            mount,
            mining_id,
        }
    }
    /// Exact held-input family.
    pub const fn kind(self) -> StopTargetKind {
        self.kind
    }
    /// Original transport and world, rechecked before stop writes.
    pub const fn session(self) -> SessionStamp {
        self.session
    }
    /// Original expected received mode; switching is not authorized.
    pub const fn mode(self) -> GameMode {
        self.mode
    }
    /// Stable domain lifetime identity within this transport; never a ledger root.
    pub const fn binding_id(self) -> u64 {
        self.binding_id
    }
    /// Common movement session ID, otherwise None.
    pub const fn control_session_id(self) -> Option<u64> {
        self.control_session_id
    }
    /// Original hand for ItemUse, otherwise None.
    pub const fn item_use_hand(self) -> Option<Hand> {
        self.item_use_hand
    }
    /// Original mount for VehicleInput, otherwise None.
    pub const fn mount(self) -> Option<MountId> {
        self.mount
    }
    /// Common staged mining identity when applicable.
    pub const fn mining_id(self) -> Option<super::survival::MiningId> {
        self.mining_id
    }
}
/// Local completion of an explicitly selected neutral transaction and cleanup.
/// Historical after issue; never server acceptance or final connection drain.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct StopReceipt {
    target: StopTarget,
    written_through: WriteOrdinal,
}
impl StopReceipt {
    pub(crate) fn new(target: StopTarget, written: u64) -> Self {
        Self {
            target,
            written_through: WriteOrdinal(written),
        }
    }
    /// Exact original target, never rebound to a replacement.
    pub const fn target(&self) -> StopTarget {
        self.target
    }
    /// Contiguous socket-write-plus-flush prefix at local stop completion.
    pub const fn written_through(&self) -> WriteOrdinal {
        self.written_through
    }
}
/// How this enqueue relates to connection-owned work; neither case certifies completion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopAdmission {
    /// This call registered a new ledger root and submitted its worker.
    NewlyAdmitted,
    /// This call joined an existing pending or cached result without new work.
    JoinedExisting,
}

/// Owned, passive observation of an already submitted selected stop.
/// Dropping this value (or its wait future) never cancels accepted work or debt.
#[must_use = "accepted stop work continues even if completion is not observed"]
pub struct StopCompletion {
    ledger: ledger::Ledger,
    admission: StopAdmission,
    receiver: tokio::sync::watch::Receiver<Option<Result<StopReceipt, StopError>>>,
}
impl StopCompletion {
    pub(crate) fn new(
        ledger: ledger::Ledger,
        admission: StopAdmission,
        receiver: tokio::sync::watch::Receiver<Option<Result<StopReceipt, StopError>>>,
    ) -> Self {
        Self {
            ledger,
            admission,
            receiver,
        }
    }
    /// Synchronous disposition fixed by enqueue; a join may refer to historical work.
    pub fn admission(&self) -> StopAdmission {
        self.admission
    }
    /// Observe the cached result or await completion. Performs no admission.
    /// A caller timeout only cancels observation, never the worker or drain debt.
    /// Release caller-held admission/policy gates before awaiting this future.
    /// Progress requires the connection's Tokio runtime to remain running.
    pub fn wait(mut self) -> impl Future<Output = Result<StopReceipt, StopError>> + Send + 'static {
        async move {
            loop {
                if let Some(result) = self.receiver.borrow_and_update().clone() {
                    return result;
                }
                if self.receiver.changed().await.is_err() {
                    return Err(StopError::Drain(
                        self.ledger.error(DrainErrorKind::OwnerLost),
                    ));
                }
            }
        }
    }
}

/// A selected stop could not supply clean completion.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum StopError {
    /// Phase, provenance inspection, transport or sticky ledger failure.
    Drain(DrainError),
    /// A new root was registered, but worker submission could not be certified.
    /// Work may have been admitted or started. This is never a no-work refusal;
    /// the ledger remains failed and a retry returns this cached error.
    AdmissionInterrupted {
        /// Original selected binding.
        target: StopTarget,
        /// Sticky underlying cause; commonly OwnerLost on registry rejection.
        drain: DrainError,
    },
    /// Target was not issued for this original transport/owner binding.
    WrongTarget {
        /// Original selected binding.
        target: StopTarget,
    },
    /// Original world, mode, mount, hand or owner was invalidated, even if restored.
    ContextChanged {
        /// Original selected binding.
        target: StopTarget,
    },
    /// No qualified neutral protocol action exists for this target.
    Unsupported {
        /// Original selected binding.
        target: StopTarget,
    },
    /// Selected release failed or was refused; drain retains the underlying failure.
    RequiredReleaseFailed {
        /// Original selected binding.
        target: StopTarget,
        /// Sticky underlying cause.
        drain: DrainError,
    },
    /// Covered abort outside intentional confirmed neutralization remains a failure.
    WorkAborted {
        /// Original selected binding.
        target: StopTarget,
        /// Sticky underlying cause.
        drain: DrainError,
    },
}
impl std::fmt::Display for StopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "neutral stop failed: {self:?}")
    }
}
impl std::error::Error for StopError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Drain(e)
            | Self::AdmissionInterrupted { drain: e, .. }
            | Self::RequiredReleaseFailed { drain: e, .. }
            | Self::WorkAborted { drain: e, .. } => Some(e),
            _ => None,
        }
    }
}

/// Sole authority to advance a transport through irreversible staged quiesce.
/// Not Clone. Drop never resumes, cancels accepted work or automatically stops inputs.
/// Retain this owner through final seal; lost authority requires explicit source revocation.
/// ```compile_fail
/// fn overlapping(q: &mut voxrig::client::QuiesceHandle) {
///     let mut permit = q.stop_permit().unwrap();
///     let _final_drain = q.seal_for_fence();
///     drop(permit);
/// }
/// ```
#[must_use = "quiesce is irreversible; retain this handle through stop and final seal"]
pub struct QuiesceHandle {
    client: Client,
    ledger: ledger::Ledger,
    first: DrainHandle,
    final_drain: Option<DrainHandle>,
}
impl QuiesceHandle {
    /// Original transport; no I/O or errors.
    pub fn generation(&self) -> GenerationKey {
        self.ledger.generation()
    }
    /// Observe the fixed first cut and ordinary-producer park acknowledgements.
    /// Repeat calls share the same stage cache. Observer drop cancels no work.
    pub fn admitted_drain(&self) -> DrainHandle {
        self.first.clone()
    }
    fn first_complete(&self) -> Result<(), DrainError> {
        if self.first.is_drained()? {
            Ok(())
        } else {
            Err(self.ledger.error(DrainErrorKind::FirstDrainPending))
        }
    }
    /// Inspect original held-input bindings after the first drain; selects/stops nothing.
    /// Cancellation has no write effect. Phase/failure/untrackable provenance returns Drain.
    pub fn stop_targets(
        &self,
    ) -> impl Future<Output = Result<Vec<StopTarget>, StopError>> + Send + '_ {
        async move {
            self.first_complete().map_err(StopError::Drain)?;
            super::dispatch!(&self.client.adapter, a => DrainOps::stop_targets(a).await)
        }
    }
    /// Exclusively borrow stop authority after the first receipt and before final seal.
    /// Pending, sealed or terminal sessions return an error; unused permit drop sends nothing.
    pub fn stop_permit(&mut self) -> Result<StopPermit<'_>, DrainError> {
        self.first_complete()?;
        if self.final_drain.is_some() {
            return Err(self.ledger.error(DrainErrorKind::Sealed));
        }
        self.ledger.check_terminal()?;
        Ok(StopPermit { quiesce: self })
    }
    /// Seal stop and independent maintenance admission, fixing the final cut.
    /// Requires first completion. Repeats share one observer/cache. Accepted stops remain debt.
    pub fn seal_for_fence(&mut self) -> Result<DrainHandle, DrainError> {
        self.first_complete()?;
        if let Some(drain) = &self.final_drain {
            drain.is_drained()?;
            return Ok(drain.clone());
        }
        let drain = DrainHandle::new(
            self.ledger.clone(),
            self.ledger.seal()?,
            DrainScope::SealedForFence,
        );
        self.final_drain = Some(drain.clone());
        Ok(drain)
    }
}

/// Cloneable observer for one fixed stage, with no stop or resume authority.
/// Synchronous polls and independent waits share one cached result. This is not a Future.
/// ```compile_fail
/// async fn direct_await(drain: voxrig::client::DrainHandle) {
///     drain.await;
/// }
/// ```
#[derive(Clone)]
#[must_use = "observe drain completion or handle its failure before reattach"]
pub struct DrainHandle {
    ledger: ledger::Ledger,
    cut: AdmissionId,
    scope: DrainScope,
    stage: Arc<Mutex<ledger::Drain>>,
}
impl DrainHandle {
    fn new(ledger: ledger::Ledger, stage: ledger::Drain, scope: DrainScope) -> Self {
        Self {
            ledger,
            cut: AdmissionId(stage.admitted_through()),
            scope,
            stage: Arc::new(Mutex::new(stage)),
        }
    }
    /// Original transport; no I/O or errors.
    pub fn generation(&self) -> GenerationKey {
        self.ledger.generation()
    }
    /// Fixed admitted-root cut, including late descendants.
    pub fn admitted_through(&self) -> AdmissionId {
        self.cut
    }
    /// Promised stage scope; this getter alone is not completion evidence.
    pub fn scope(&self) -> DrainScope {
        self.scope
    }
    fn inspect(&self) -> Result<Option<DrainReceipt>, DrainError> {
        let result = self
            .stage
            .lock()
            .map_err(|_| self.ledger.error(DrainErrorKind::Internal))?
            .inspect();
        result
            .map(|r| {
                r.map(|r| DrainReceipt {
                    generation: self.generation(),
                    admitted_through: self.cut,
                    written_through: WriteOrdinal(r.written_through()),
                    scope: self.scope,
                })
                .map_err(|e| e.public())
            })
            .transpose()
    }
    /// Cheap synchronous poll: false pending, true issued/cached, Err failed.
    /// No I/O, await, runtime, reconciliation scan or work progression. Uses short locks
    /// and a live-root prefix query. Callable from a `Fn() -> Result<bool, E> + Send + Sync`.
    /// Issued historical completion survives later session failure or revocation.
    pub fn is_drained(&self) -> Result<bool, DrainError> {
        self.inspect().map(|r| r.is_some())
    }
    /// Wait for the shared receipt or sticky failure, with independent wake registration.
    /// Repeat/concurrent waits are supported. Drop/timeout cancels only this wait.
    pub fn wait(&self) -> impl Future<Output = Result<DrainReceipt, DrainError>> + Send + '_ {
        async move {
            let mut changed = self.ledger.subscribe();
            loop {
                if let Some(receipt) = self.inspect()? {
                    return Ok(receipt);
                }
                changed.changed().await.expect("ledger sender retained");
            }
        }
    }
}

/// One exclusive borrowed capability for caller-selected neutral transactions.
/// Not Clone/Copy; cannot send arbitrary packets or resume normal input.
#[must_use = "only explicit selected neutral transactions are allowed through this permit"]
pub struct StopPermit<'a> {
    quiesce: &'a mut QuiesceHandle,
}
impl StopPermit<'_> {
    /// Synchronously admit this exact target's once-only neutral action and submit
    /// its connection-owned worker, or join its pending/cached result.
    /// Before `Ok` returns, ledger registration and task-registry submission are
    /// complete. Workers can run immediately, even before this method returns.
    /// [`StopAdmission::NewlyAdmitted`] identifies new work; [`StopAdmission::JoinedExisting`]
    /// adds no root or worker. Admission is not delivery or successful completion.
    ///
    /// `Err(StopError::AdmissionInterrupted { .. })` means a root was registered
    /// but registry acceptance failed; work may have started and drain stays failed.
    /// Other errors create no new work in this call (earlier work may still exist).
    /// Wrong/stale targets retain their sticky required-release failure. Execution,
    /// reset, revoke and worker-loss errors after successful enqueue surface through
    /// [`StopCompletion::wait`]; a historical cached success remains historical.
    ///
    /// This method uses only ordinary synchronous mutexes: stop-book -> ledger
    /// state (released) -> task registry. It never holds ledger state across registry
    /// submission, awaits, blocks on async locks, or performs I/O. Mutex contention
    /// can block the calling thread; this is not a hard real-time bound. Callers
    /// must not re-enter Voxrig from within any caller-held gate in a way that
    /// conflicts with this lock order, and must release their gate before awaiting
    /// completion. Native state/writer locks and I/O are used only by the worker.
    ///
    /// A live Tokio runtime is required for worker progress. Submission uses the
    /// connection's captured runtime, or the current runtime if none was captured;
    /// a missing runtime yields `AdmissionInterrupted` after root registration.
    /// Keep that runtime running through completion/shutdown.
    /// The SDK enforces no caller deadline: check it in the caller's hook before
    /// enqueue. A timeout around `wait()` cancels observation only.
    /// Native action effects and original-owner validation are as documented by
    /// [`Self::neutralize`], including release-use side effects.
    ///
    /// ```no_run
    /// # use voxrig::client::{QuiesceHandle, StopTarget, StopError};
    /// # async fn stop(q: &mut QuiesceHandle, target: StopTarget) -> Result<(), StopError> {
    /// let completion = q.stop_permit().map_err(StopError::Drain)?
    ///     .enqueue_neutralize(target)?;
    /// let admission = completion.admission(); // Fixed before wait() is constructed.
    /// let final_drain = q.seal_for_fence().map_err(StopError::Drain)?;
    /// completion.wait().await?;
    /// final_drain.wait().await.map_err(StopError::Drain)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn enqueue_neutralize(&mut self, target: StopTarget) -> Result<StopCompletion, StopError> {
        super::dispatch!(&self.quiesce.client.adapter, a => DrainOps::enqueue_neutralize(a, target))
    }

    /// Admit this exact target's neutral action at first poll, at most once per binding.
    /// Construction/drop before first poll has no admission or sticky-error effects.
    /// On first poll this calls [`Self::enqueue_neutralize`], then observes completion.
    /// Accepted cleanup is connection-owned and survives waiter drop; retries join/cache it.
    /// Common and legacy native movement are distinct owners. A dormant native stop
    /// clears its local input; the selected common stop releases common wire posture.
    /// For an [`ItemUse`](StopTargetKind::ItemUse) target, neutralize sends the native
    /// release-use packet. Releasing a drawn bow may fire its arrow, and releasing
    /// other items may also affect gameplay. The [`StopReceipt`] confirms local
    /// dispatch and cleanup, not server acceptance or cancellation of those effects.
    /// Intentional digging ABORT is clean only when cleanup is confirmed;
    /// timeout/unknown outcome remain sticky errors.
    /// Stale, unsupported and refused requests fail closed without touching a replacement.
    pub fn neutralize(
        &mut self,
        target: StopTarget,
    ) -> impl Future<Output = Result<StopReceipt, StopError>> + Send + '_ {
        async move { self.enqueue_neutralize(target)?.wait().await }
    }
}
impl Client {
    /// Observe shutdown of this exact Client's original transport generation,
    /// identified by `generation_key()`, shared with all common/native clones.
    /// This passive observer does not initiate quiesce, cleanup or shutdown and
    /// remains pending until transport termination begins. Concurrent and repeated
    /// waiters all observe the same cached result; after completion this returns
    /// that result immediately, including the original error on failed shutdown.
    /// Dropping or timing out this future cancels observation only and cannot
    /// affect shutdown progress or another waiter.
    ///
    /// `Ok(())` certifies release of the local transport halves and actual exit
    /// of connection-owned tasks (including timers and workers). It does not prove
    /// server acknowledgement, game effects or physical stillness. Manager event
    /// relays and caller-owned tasks/futures are outside this scope: callers must
    /// cancel/join their own source tasks. A caller holding writer serialization
    /// can delay completion. No bounded completion time or timeout is provided.
    ///
    /// `Client::retire_connection` combines revocation with bounded observation
    /// of this completion. Its Unconfirmed result certifies revocation only.
    /// Golemkit must stop authorizing work with old evidence; policies requiring
    /// confirmed shutdown need this method returning `Ok(())`. A drain or stop
    /// receipt alone does not supply that fact. Mining, acknowledgement-identity,
    /// entity and ledger-owner retirement have separate domain-specific meanings.
    pub fn wait_shutdown(
        &self,
    ) -> impl Future<Output = Result<(), super::ShutdownError>> + Send + '_ {
        let ledger = super::dispatch!(&self.adapter, a => DrainOps::drain_ledger(a));
        async move { ledger.tasks().wait().await }
    }

    /// Immutable process-local transport identity shared by all common/native clones.
    /// Synchronous observation, before or after revocation; no I/O, runtime or errors.
    pub fn generation_key(&self) -> GenerationKey {
        super::dispatch!(&self.adapter, a => DrainOps::drain_ledger(a).generation())
    }
    /// Irreversibly close normal admission, request producer park and fix the first cut.
    /// Synchronous, with no I/O/await. Requires the exact key and Ready state; succeeds once.
    /// Existing Result-returning mutations/native/raw sends then fail with ErrorKind::State.
    /// Legacy void control setters become no-ops. Drop never resumes or selects stops.
    /// Neither stage proves server acceptance; the final stage guarantees no later Voxrig writes.
    ///
    /// Before calling, serialize application source admission, settle inventory
    /// operations and explicitly close containers using their original ScreenId.
    /// End item use before quiesce.
    /// Keep this serialization through begin: clones must not reopen a container
    /// or restart item use. Quiesce does not automatically close screens or release
    /// items, and a new ordinary container close is rejected after begin.
    pub fn begin_quiesce(&self, generation: GenerationKey) -> Result<QuiesceHandle, DrainError> {
        let (ledger, ready) = super::dispatch!(&self.adapter, a => (DrainOps::drain_ledger(a), DrainOps::drain_ready(a)));
        let stage = ledger.begin(generation, ready)?;
        let first = DrainHandle::new(ledger.clone(), stage, DrainScope::ProducersParked);
        Ok(QuiesceHandle {
            client: self.clone(),
            ledger,
            first,
            final_drain: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ledger() -> ledger::Ledger {
        ledger::Ledger::tracking(ledger::GenerationKey {
            version: crate::MinecraftVersion::Java1_16_1,
            connection_id: 1,
        })
    }
    #[test]
    fn quiesce_poll_is_sync_cached_and_fn_send_sync_without_runtime() {
        fn traits<T: Send + Sync + Unpin + 'static>() {}
        traits::<DrainHandle>();
        traits::<QuiesceHandle>();
        traits::<DrainReceipt>();
        traits::<StopTarget>();
        traits::<StopError>();
        traits::<StopAdmission>();
        traits::<StopCompletion>();
        fn permit_traits<T: Send + Sync + Unpin>() {}
        permit_traits::<StopPermit<'_>>();
        let ledger = ledger();
        let work = ledger.work();
        let stage = DrainHandle::new(
            ledger.clone(),
            ledger.begin(ledger.generation(), true).unwrap(),
            DrainScope::ProducersParked,
        );
        let poll: Box<dyn Fn() -> Result<bool, DrainError> + Send + Sync> = Box::new({
            let stage = stage.clone();
            move || stage.is_drained()
        });
        assert!(!poll().unwrap());
        work.finish(ledger::Outcome::Complete);
        assert!(poll().unwrap());
        ledger.fail(ledger::Failure::Revoked);
        assert!(poll().unwrap());
        assert!(stage.clone().is_drained().unwrap());
        assert!(ledger.begin(ledger.generation(), true).is_err());
    }
    #[tokio::test]
    async fn quiesce_shared_waiters_cancel_independently_and_revoke_orders_issue() {
        for issue in [false, true] {
            let ledger = ledger();
            let work = ledger.work();
            let stage = DrainHandle::new(
                ledger.clone(),
                ledger.begin(ledger.generation(), true).unwrap(),
                DrainScope::ProducersParked,
            );
            let cancelled = tokio::spawn({
                let stage = stage.clone();
                async move { stage.wait().await }
            });
            cancelled.abort();
            let _ = cancelled.await;
            work.finish(ledger::Outcome::Complete);
            if issue {
                assert!(stage.is_drained().unwrap());
            }
            ledger.fail(ledger::Failure::Revoked);
            let (one, two) = tokio::join!(stage.wait(), stage.wait());
            assert_eq!(one.is_ok(), issue);
            assert_eq!(one, two);
            if !issue {
                assert_eq!(one.unwrap_err().kind(), DrainErrorKind::RevokedBeforeDrain);
            }
        }
    }
    #[test]
    fn quiesce_key_phase_and_independent_maintenance_seal() {
        let ledger = ledger();
        let wrong = GenerationKey::new(crate::MinecraftVersion::Java1_21_11, 1);
        let e = ledger.begin(wrong, true).err().unwrap();
        assert_eq!(e.kind(), DrainErrorKind::WrongGeneration);
        assert_eq!(e.requested_generation(), Some(wrong));
        assert_eq!(
            ledger
                .begin(ledger.generation(), false)
                .err()
                .unwrap()
                .kind(),
            DrainErrorKind::NotReady
        );
        let first = DrainHandle::new(
            ledger.clone(),
            ledger.begin(ledger.generation(), true).unwrap(),
            DrainScope::ProducersParked,
        );
        assert_eq!(
            ledger
                .begin(ledger.generation(), true)
                .err()
                .unwrap()
                .kind(),
            DrainErrorKind::AlreadyQuiescing
        );
        assert!(first.is_drained().unwrap());
        assert!(ledger.try_work().is_err());
        let maintenance = ledger.maintenance().unwrap();
        let final_drain = DrainHandle::new(
            ledger.clone(),
            ledger.seal().unwrap(),
            DrainScope::SealedForFence,
        );
        assert!(!final_drain.is_drained().unwrap());
        assert!(ledger.maintenance().is_err());
        assert!(ledger.stop_work().is_err());
        let late = maintenance.child().unwrap();
        maintenance.finish(ledger::Outcome::Complete);
        assert!(!final_drain.is_drained().unwrap());
        late.finish(ledger::Outcome::Complete);
        assert!(final_drain.is_drained().unwrap());
        assert_eq!(final_drain.scope(), DrainScope::SealedForFence);
    }
}
