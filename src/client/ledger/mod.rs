//! Internal, unwired admission ledger foundation for issue #131 PR 1.
//!
//! One future session owns one `Ledger`; guards and waiters retain its shared
//! state, never a Client/session reference (no ownership cycle). No production
//! operation uses this module yet, so its receipts establish no public guarantee.
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
//! Live roots, guards and producers have fixed bounds. Retired roots are removed
//! immediately; cumulative outcome counters saturate, and the first failure is
//! retained separately forever. No receipt/history registry is retained here.
//! See docs/internal/drain-ledger.md for provisional choices and integration debt.
#![allow(dead_code)] // Deliberately unwired until subsequent ledger integration PRs.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::watch;

use crate::MinecraftVersion;

const MAX_ROOTS: usize = 1024;
const MAX_GUARDS: usize = 4096;
const MAX_PRODUCERS: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GenerationKey {
    version: MinecraftVersion,
    connection_id: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Failure {
    OwnerLost,
    UncertainDispatch,
    TransactionUnconfirmed,
    RequiredReleaseFailed,
    Revoked,
    IdExhausted,
    Poisoned,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Outcome {
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
struct Terminal {
    root: Option<u64>,
    cause: Failure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AdmissionError {
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
struct Receipt {
    generation: GenerationKey,
    cut: Cut,
    written_through: u64,
    // Cumulative guard outcomes at issue time, not an exact per-cut history.
    outcomes: Outcomes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DrainFailure {
    generation: GenerationKey,
    cut: Cut,
    written_through: u64,
    terminal: Terminal,
}

struct State {
    closed: bool,
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

struct Shared {
    generation: GenerationKey,
    state: Mutex<State>,
    changed: watch::Sender<()>,
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
        if self.guards >= MAX_GUARDS {
            return Err(AdmissionError::Capacity);
        }
        // Per-root counts cannot exceed the globally bounded guard count.
        *self.roots.get_mut(&root).expect("live parent root") += 1;
        self.guards += 1;
        Ok(())
    }
}

struct Ledger {
    shared: Arc<Shared>,
}

impl Ledger {
    fn new(generation: GenerationKey) -> Self {
        let (changed, _) = watch::channel(());
        Self {
            shared: Arc::new(Shared {
                generation,
                changed,
                state: Mutex::new(State {
                    closed: false,
                    last_root: 0,
                    last_producer: 0,
                    last_write: 0,
                    written_through: 0,
                    writing: false,
                    roots: BTreeMap::new(),
                    guards: 0,
                    producers: BTreeSet::new(),
                    terminal: None,
                    outcomes: Outcomes::default(),
                }),
            }),
        }
    }

    fn admit(&self) -> Result<WorkGuard, AdmissionError> {
        let mut state = self.shared.lock();
        state.healthy()?;
        if state.closed {
            return Err(AdmissionError::Closed);
        }
        if state.roots.len() >= MAX_ROOTS || state.guards >= MAX_GUARDS {
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
        })
    }

    fn register_producer(&self) -> Result<Producer, AdmissionError> {
        let mut state = self.shared.lock();
        state.healthy()?;
        if state.closed {
            return Err(AdmissionError::Closed);
        }
        if state.producers.len() >= MAX_PRODUCERS {
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

    fn cut(&self, close_roots: bool) -> Drain {
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

    fn fail(&self, cause: Failure) {
        self.shared.lock().fail(None, cause);
        self.shared.notify();
    }
}

struct WorkGuard {
    shared: Arc<Shared>,
    root: u64,
    retired: bool,
}

impl WorkGuard {
    fn child(&self) -> Result<Self, AdmissionError> {
        self.shared.lock().add_guard(self.root)?;
        Ok(Self {
            shared: self.shared.clone(),
            root: self.root,
            retired: false,
        })
    }

    // Consuming handoff moves precisely the same live ownership, even at capacity.
    fn transfer(self) -> Self {
        self
    }

    fn finish(mut self, outcome: Outcome) {
        self.shared.retire(self.root, outcome);
        self.retired = true;
    }

    fn begin_write(&self) -> Result<WritePermit, AdmissionError> {
        let mut state = self.shared.lock();
        state.healthy()?;
        if state.writing {
            return Err(AdmissionError::WriterBusy);
        }
        if state.guards >= MAX_GUARDS {
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
            // Only one attempt exists, and allocation stops at the first failure.
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

struct Producer {
    shared: Arc<Shared>,
    id: u64,
    parked: bool,
}

impl Producer {
    fn park_requested(&self) -> bool {
        self.shared.lock().closed
    }

    // May only be called once no sample/queue can create another independent root.
    // Existing finite roots remain counted separately. No network stop is implied.
    fn acknowledge_park(mut self) {
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

struct Drain {
    shared: Arc<Shared>,
    changed: watch::Receiver<()>,
    cut: Cut,
    result: Option<Result<Receipt, DrainFailure>>,
}

impl Drain {
    fn inspect(&mut self) -> Option<Result<Receipt, DrainFailure>> {
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

    async fn wait(&mut self) -> Result<Receipt, DrainFailure> {
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
