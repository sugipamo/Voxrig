//! Internal ledger backing public staged quiesce/drain for issue #131.
//!
//! Each transport owns one `Ledger`; guards and waiters retain its shared
//! state, never a Client/session reference (no ownership cycle). `QuiesceHandle`
//! and `DrainHandle` expose the staged boundary; guard/write-permit machinery
//! stays private. The first drain covers admitted work and ordinary-producer
//! parking; final sealing also closes stop and independent maintenance admission.
//! A successful final drain guarantees no later Voxrig writes, not server
//! acknowledgement or physical stillness. Joined connection shutdown and local
//! transport release are the separate `Client::wait_shutdown` contract.
//!
//! Admission and cut capture share a short synchronous gate. A cut C covers all
//! roots <= C and every descendant, including descendants created after the cut.
//! Moving a guard transfers ownership without retirement; `child` registers
//! before the parent can retire. Explicit completion relinquishes future write
//! authority. Unfinished Drop/panic is failure, never a successful decrement.
//!
//! Writers must acquire their existing serialization lock BEFORE `begin_write`,
//! then hold it through write_all + flush and `commit`. The ledger additionally
//! permits only one live attempt. W is a contiguous local completed-frame prefix,
//! not an ACK, remote read, or physical stillness. World reset must not reset W.
//! No ledger lock may be held across I/O, await, or acquiring writer/state locks.
//!
//! Watch is a wakeup hint: subscribe, inspect under the gate, await a change,
//! inspect again. Each waiter owns a receiver. Results are cached per waiter;
//! failure is sticky for every not-yet-issued receipt, even an older cut.
//!
//! Foundation admission has fixed bounds; production tracks existing adapter
//! admission without a new transport cap. Retired roots are removed immediately;
//! cumulative outcome counters saturate, and the first failure is
//! retained separately forever. No receipt/history registry is retained here.
//! See docs/internal/drain-ledger.md for provisional choices and integration debt.
#![allow(dead_code)] // Internal foundation/test diagnostics are not public APIs.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::watch;

use crate::MinecraftVersion;

pub(crate) mod finite;
mod staged;
pub(crate) mod stops;

const MAX_ROOTS: usize = 1024;
const MAX_GUARDS: usize = 4096;
const MAX_PRODUCERS: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GenerationKey {
    pub(crate) version: MinecraftVersion,
    pub(crate) connection_id: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Failure {
    OwnerLost,
    UncertainDispatch,
    TransactionUnconfirmed,
    TransactionExpired,
    TransactionOutcomeUnknown,
    RequiredReleaseFailed,
    Revoked,
    IdExhausted,
    Poisoned,
    Disconnected,
    WorkAborted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Outcome {
    Complete,
    NoWrite,
    Rejected,
    CancelledBeforeWrite,
    SuppressedBeforeWrite,
    Failed(Failure),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Outcomes {
    complete: u64,
    no_write: u64,
    rejected: u64,
    cancelled: u64,
    suppressed: u64,
    failed: u64,
}

impl Outcomes {
    fn record(&mut self, outcome: Outcome) {
        let count = match outcome {
            Outcome::Complete => &mut self.complete,
            Outcome::NoWrite => &mut self.no_write,
            Outcome::Rejected => &mut self.rejected,
            Outcome::CancelledBeforeWrite => &mut self.cancelled,
            Outcome::SuppressedBeforeWrite => &mut self.suppressed,
            Outcome::Failed(_) => &mut self.failed,
        };
        *count = count.saturating_add(1);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Terminal {
    root: Option<u64>,
    cause: Failure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AdmissionError {
    Closed,
    Capacity,
    WriterBusy,
    Terminal(Terminal),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scope {
    Snapshot,
    RootsClosedAndProducersParked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Cut {
    admitted_through: u64,
    scope: Scope,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Receipt {
    generation: GenerationKey,
    cut: Cut,
    written_through: u64,
    // Cumulative guard outcomes at issue time, not an exact per-cut history.
    outcomes: Outcomes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DrainFailure {
    generation: GenerationKey,
    cut: Cut,
    written_through: u64,
    terminal: Terminal,
}

struct State {
    // PR 2 observes existing transport admission. A failed drain cannot change
    // public send errors, and existing queued callers must not gain a new cap.
    track_existing: bool,
    closed: bool,
    quiescing: bool,
    sealed: bool,
    ready: bool,
    last_root: u64,
    last_producer: u64,
    last_write: u64,
    written_through: u64,
    writing: bool,
    roots: BTreeMap<u64, usize>,
    guards: usize,
    // Only unparked producers are retained; a park ACK is irreversible.
    producers: BTreeSet<u64>,
    terminal: Option<Terminal>,
    outcomes: Outcomes,
}

static NEXT_PROVENANCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

struct Shared {
    shutdown: super::shutdown::Registry,
    provenance: u64,
    generation: GenerationKey,
    state: Mutex<State>,
    changed: watch::Sender<()>,
    stops: Mutex<stops::StopBook>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poison) => {
                let mut state = poison.into_inner();
                state.fail(None, Failure::Poisoned);
                self.notify();
                state
            }
        }
    }

    fn notify(&self) {
        self.changed.send_replace(());
    }

    fn retire(&self, root: u64, outcome: Outcome) {
        let mut state = self.lock();
        if let Outcome::Failed(cause) = outcome {
            state.fail(Some(root), cause);
        }
        state.outcomes.record(outcome);
        let owners = state.roots.get_mut(&root).expect("live guard root");
        *owners -= 1;
        if *owners == 0 {
            state.roots.remove(&root);
        }
        state.guards -= 1;
        self.notify();
    }
}

impl State {
    fn fail(&mut self, root: Option<u64>, cause: Failure) {
        self.terminal.get_or_insert(Terminal { root, cause });
    }

    fn healthy(&self) -> Result<(), AdmissionError> {
        if self.track_existing {
            return Ok(());
        }
        self.terminal
            .map_or(Ok(()), |t| Err(AdmissionError::Terminal(t)))
    }

    fn next(&mut self, last: u64) -> Result<u64, AdmissionError> {
        last.checked_add(1).ok_or_else(|| {
            self.fail(None, Failure::IdExhausted);
            AdmissionError::Terminal(self.terminal.expect("overflow is terminal"))
        })
    }

    fn add_guard(&mut self, root: u64) -> Result<(), AdmissionError> {
        self.healthy()?;
        if !self.track_existing && self.guards >= MAX_GUARDS {
            return Err(AdmissionError::Capacity);
        }
        // A count represents actual live guards only, never retired history.
        *self.roots.get_mut(&root).expect("live parent root") += 1;
        self.guards += 1;
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct Ledger {
    shared: Arc<Shared>,
}

impl Ledger {
    pub(crate) fn tasks(&self) -> &super::shutdown::Registry {
        &self.shared.shutdown
    }
}

impl Ledger {
    #[allow(deprecated)] // fetch_update is required by MSRV 1.85.
    pub(crate) fn new(generation: GenerationKey) -> Self {
        let (changed, _) = watch::channel(());
        let provenance = NEXT_PROVENANCE
            .fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |n| n.checked_add(1),
            )
            .ok();
        Self {
            shared: Arc::new(Shared {
                shutdown: super::shutdown::Registry::new(super::GenerationKey::new(
                    generation.version,
                    generation.connection_id,
                )),
                provenance: provenance.unwrap_or(0),
                generation,
                changed,
                stops: Mutex::default(),
                state: Mutex::new(State {
                    track_existing: false,
                    closed: false,
                    quiescing: false,
                    sealed: false,
                    ready: false,
                    last_root: 0,
                    last_producer: 0,
                    last_write: 0,
                    written_through: 0,
                    writing: false,
                    roots: BTreeMap::new(),
                    guards: 0,
                    producers: BTreeSet::new(),
                    terminal: provenance.is_none().then_some(Terminal {
                        root: None,
                        cause: Failure::IdExhausted,
                    }),
                    outcomes: Outcomes::default(),
                }),
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn test_written_through(&self) -> u64 {
        self.shared.lock().written_through
    }

    #[cfg(test)]
    pub(crate) fn test_stop_debt(&self) -> (u64, usize) {
        let state = self.shared.lock();
        (state.last_root, state.roots.len())
    }
    #[cfg(test)]
    pub(crate) fn test_poison_stops(&self) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = self.shared.stops.lock().unwrap();
            panic!("injected stop-book poison");
        }));
    }
    // Track existing adapter admission with the same roots/children/permits.
    // Failure still rejects every unissued drain, without changing public send
    // errors or imposing a new queue capacity. Closing roots is still enforced.
    pub(crate) fn tracking(generation: GenerationKey) -> Self {
        let ledger = Self::new(generation);
        ledger.shared.lock().track_existing = true;
        ledger
    }

    pub(crate) fn admit(&self) -> Result<WorkGuard, AdmissionError> {
        let mut state = self.shared.lock();
        state.healthy()?;
        if state.closed {
            return Err(AdmissionError::Closed);
        }
        if !state.track_existing && (state.roots.len() >= MAX_ROOTS || state.guards >= MAX_GUARDS) {
            return Err(AdmissionError::Capacity);
        }
        let last = state.last_root;
        let id = state.next(last);
        self.shared.notify(); // Also publish allocation failure.
        let id = id?;
        state.last_root = id;
        state.roots.insert(id, 1);
        state.guards += 1;
        Ok(WorkGuard {
            shared: self.shared.clone(),
            root: id,
            retired: false,
            neutral_stop: false,
        })
    }

    pub(crate) fn register_producer(&self) -> Result<Producer, AdmissionError> {
        let mut state = self.shared.lock();
        state.healthy()?;
        if state.closed && !self.inherited() {
            return Err(AdmissionError::Closed);
        }
        if !state.track_existing && state.producers.len() >= MAX_PRODUCERS {
            return Err(AdmissionError::Capacity);
        }
        let last = state.last_producer;
        let id = state.next(last);
        self.shared.notify();
        let id = id?;
        state.last_producer = id;
        state.producers.insert(id);
        Ok(Producer {
            shared: self.shared.clone(),
            id,
            parked: false,
        })
    }

    pub(crate) fn cut(&self, close_roots: bool) -> Drain {
        // Subscribe BEFORE evaluating readiness, even for an already idle cut.
        let changed = self.shared.changed.subscribe();
        let mut state = self.shared.lock();
        if close_roots {
            state.closed = true;
            self.shared.notify();
        }
        let cut = Cut {
            admitted_through: state.last_root,
            scope: if close_roots {
                Scope::RootsClosedAndProducersParked
            } else {
                Scope::Snapshot
            },
        };
        Drain {
            shared: self.shared.clone(),
            changed,
            cut,
            result: None,
        }
    }

    pub(crate) fn fail(&self, cause: Failure) {
        self.shared.lock().fail(None, cause);
        self.shared.notify();
    }
}

pub(crate) struct WorkGuard {
    shared: Arc<Shared>,
    root: u64,
    retired: bool,
    neutral_stop: bool,
}

impl WorkGuard {
    pub(crate) fn tasks(&self) -> &super::shutdown::Registry {
        &self.shared.shutdown
    }
    pub(crate) fn child(&self) -> Result<Self, AdmissionError> {
        self.shared.lock().add_guard(self.root)?;
        Ok(Self {
            shared: self.shared.clone(),
            root: self.root,
            retired: false,
            neutral_stop: self.neutral_stop,
        })
    }

    // Consuming handoff moves precisely the same live ownership, even at capacity.
    fn transfer(self) -> Self {
        self
    }

    pub(crate) fn finish(mut self, outcome: Outcome) {
        self.shared.retire(self.root, outcome);
        self.retired = true;
    }

    fn begin_write(&self) -> Result<WritePermit, AdmissionError> {
        let mut state = self.shared.lock();
        state.healthy()?;
        if state.writing {
            return Err(AdmissionError::WriterBusy);
        }
        if !state.track_existing && state.guards >= MAX_GUARDS {
            return Err(AdmissionError::Capacity);
        }
        let last = state.last_write;
        let ordinal = state.next(last);
        self.shared.notify();
        let ordinal = ordinal?;
        state.add_guard(self.root)?;
        state.last_write = ordinal;
        state.writing = true;
        Ok(WritePermit {
            work: Some(Self {
                shared: self.shared.clone(),
                root: self.root,
                retired: false,
                neutral_stop: self.neutral_stop,
            }),
            ordinal,
        })
    }
}

impl Drop for WorkGuard {
    fn drop(&mut self) {
        if !self.retired {
            self.shared
                .retire(self.root, Outcome::Failed(Failure::OwnerLost));
        }
    }
}

struct WritePermit {
    work: Option<WorkGuard>,
    ordinal: u64,
}

impl WritePermit {
    fn commit(mut self) {
        let work = self.work.take().expect("live write permit");
        {
            let mut state = work.shared.lock();
            // Only one attempt exists. Tracking after a failure must never
            // advance the contiguous prefix over an incomplete frame.
            if state.written_through.checked_add(1) == Some(self.ordinal) {
                state.written_through = self.ordinal;
            } else {
                state.fail(Some(work.root), Failure::UncertainDispatch);
            }
            state.writing = false;
        }
        work.finish(Outcome::Complete);
    }
}

impl Drop for WritePermit {
    fn drop(&mut self) {
        if let Some(work) = self.work.take() {
            {
                let mut state = work.shared.lock();
                state.fail(Some(work.root), Failure::UncertainDispatch);
                state.writing = false;
            }
            work.finish(Outcome::Failed(Failure::UncertainDispatch));
        }
    }
}

pub(crate) struct Producer {
    shared: Arc<Shared>,
    id: u64,
    parked: bool,
}

impl Producer {
    pub(crate) fn park_requested(&self) -> bool {
        self.shared.lock().closed
    }

    // May only be called once no sample/queue can create another independent root.
    // Existing finite roots remain counted separately. No network stop is implied.
    pub(crate) fn acknowledge_park(mut self) {
        self.shared.lock().producers.remove(&self.id);
        self.parked = true;
        self.shared.notify();
    }
}

impl Drop for Producer {
    fn drop(&mut self) {
        if !self.parked {
            let mut state = self.shared.lock();
            state.producers.remove(&self.id);
            state.fail(None, Failure::OwnerLost);
            self.shared.notify();
        }
    }
}

pub(crate) struct Drain {
    shared: Arc<Shared>,
    changed: watch::Receiver<()>,
    cut: Cut,
    result: Option<Result<Receipt, DrainFailure>>,
}

impl Drain {
    pub(crate) fn inspect(&mut self) -> Option<Result<Receipt, DrainFailure>> {
        if self.result.is_some() {
            return self.result;
        }
        let state = self.shared.lock();
        self.result = if let Some(terminal) = state.terminal {
            Some(Err(DrainFailure {
                generation: self.shared.generation,
                cut: self.cut,
                written_through: state.written_through,
                terminal,
            }))
        } else if state
            .roots
            .range(..=self.cut.admitted_through)
            .next()
            .is_none()
            && (self.cut.scope == Scope::Snapshot || state.producers.is_empty())
        {
            Some(Ok(Receipt {
                generation: self.shared.generation,
                cut: self.cut,
                written_through: state.written_through,
                outcomes: state.outcomes,
            }))
        } else {
            None
        };
        self.result
    }

    pub(crate) async fn wait(&mut self) -> Result<Receipt, DrainFailure> {
        loop {
            if let Some(result) = self.inspect() {
                return result;
            }
            // The sender lives in shared state retained by this waiter.
            self.changed
                .changed()
                .await
                .expect("ledger sender retained");
        }
    }
}

#[cfg(test)]
mod tests;

// A scoped writer/queue owner has no retained authority after it leaves scope.
// A write attempt independently owns a child and records interruption first.
pub(crate) struct ScopedWork(Option<WorkGuard>);
impl ScopedWork {
    pub(crate) fn new(work: WorkGuard) -> Self {
        Self(Some(work))
    }
    pub(crate) fn guard(&self) -> &WorkGuard {
        self.0.as_ref().expect("live scope")
    }
}
impl Drop for ScopedWork {
    fn drop(&mut self) {
        if let Some(work) = self.0.take() {
            work.finish(if std::thread::panicking() {
                Outcome::Failed(Failure::OwnerLost)
            } else {
                Outcome::Complete
            });
        }
    }
}

// Shared only with a protocol transaction handle. Retiring the pending state
// releases its owner even if the public handle is never awaited or dropped.
pub(crate) struct PendingWork(Mutex<Option<WorkGuard>>);
impl PendingWork {
    pub(crate) fn new(work: WorkGuard) -> Self {
        Self(Mutex::new(Some(work)))
    }
    pub(crate) fn child(&self) -> Option<WorkGuard> {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .map(|work| work.child().expect("existing protocol owner"))
    }
    pub(crate) fn finish(&self, outcome: Outcome) {
        if let Some(work) = self.0.lock().unwrap_or_else(|p| p.into_inner()).take() {
            work.finish(outcome);
        }
    }
}

// The approved frame-completion policy: socket write plus flush. Publishing W
// never retires protocol pending/barrier owners or waits for server ACK.
enum CompletionPoint {
    SocketFlush,
}
const COMPLETION_POINT: CompletionPoint = CompletionPoint::SocketFlush;
fn publish_completion(permit: WritePermit) {
    match COMPLETION_POINT {
        CompletionPoint::SocketFlush => permit.commit(),
    }
}

// No AsyncWrite implementation or mutable stream accessor: production socket
// bytes can only be emitted by consuming a real permit here. Shutdown emits no
// packet and retains the adapters' existing teardown semantics.
pub(crate) struct SocketWriter<W> {
    inner: Option<W>,
    shutdown_error: Option<Arc<std::io::Error>>,
    own_actions: Option<Arc<crate::client::inventory::own_action::OwnActionSink>>,
    #[cfg(test)]
    replacement: Option<testing::BoundedWriter>,
}
impl<W: tokio::io::AsyncWrite + Unpin> SocketWriter<W> {
    pub(crate) fn new(inner: W) -> Self {
        Self {
            inner: Some(inner),
            shutdown_error: None,
            own_actions: None,
            #[cfg(test)]
            replacement: None,
        }
    }
    /// Record own play-phase item use/place/dig/drop/swap frames (I9). Attach
    /// only after login/configuration, whose packet IDs are a different space.
    pub(crate) fn observe_own_actions(
        &mut self,
        version: crate::MinecraftVersion,
        sink: Arc<crate::client::inventory::own_action::OwnActionSink>,
    ) {
        sink.attach(version);
        self.own_actions = Some(sink);
    }
    pub(crate) async fn send(
        &mut self,
        work: &WorkGuard,
        compression: Option<i32>,
        id: i32,
        payload: &[u8],
    ) -> anyhow::Result<()> {
        self.send_with_attempt(work, compression, id, payload, None)
            .await
    }
    pub(crate) async fn send_with_attempt(
        &mut self,
        work: &WorkGuard,
        compression: Option<i32>,
        id: i32,
        payload: &[u8],
        write_started: Option<&AtomicBool>,
    ) -> anyhow::Result<()> {
        work.observe_packet(id, payload)?;
        let permit = work
            .begin_write()
            .map_err(|e| anyhow::anyhow!("ledger write: {e:?}"))?;
        // Before the frame: a reply can be received before the write returns.
        if let Some(sink) = &self.own_actions {
            sink.observe(id, payload);
        }
        #[cfg(test)]
        if let Some(writer) = &mut self.replacement {
            if let Some(started) = write_started {
                started.store(true, Ordering::Release);
            }
            crate::protocol::write_packet(writer, compression, id, payload).await?;
            publish_completion(permit);
            return Ok(());
        }
        let writer = self.inner.as_mut().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotConnected, "writer released")
        })?;
        // Admission, predecessor checks and writer availability have succeeded.
        // From this boundary a failed write can have delivered a frame prefix.
        if let Some(started) = write_started {
            started.store(true, Ordering::Release);
        }
        crate::protocol::write_packet(writer, compression, id, payload).await?;
        publish_completion(permit);
        Ok(())
    }
    pub(crate) async fn shutdown(&mut self) -> std::io::Result<()> {
        if let Some(cause) = &self.shutdown_error {
            return Err(std::io::Error::other(cause.clone()));
        }
        if let Some(mut inner) = self.inner.take() {
            if let Err(cause) = tokio::io::AsyncWriteExt::shutdown(&mut inner).await {
                let cause = Arc::new(cause);
                self.shutdown_error = Some(cause.clone());
                return Err(std::io::Error::other(cause));
            }
        }
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn test_replace(&mut self, writer: testing::BoundedWriter) {
        self.replacement = Some(writer);
    }
    #[cfg(test)]
    pub(crate) fn is_released(&self) -> bool {
        self.inner.is_none()
    }
    #[cfg(test)]
    pub(crate) fn test_inner(&self) -> &W {
        self.inner.as_ref().expect("test writer open")
    }
}

#[cfg(test)]
pub(crate) mod testing;
