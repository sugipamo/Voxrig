//! Invocation-owned execution with bounded, observer-only waiting.
//!
//! A deadline ends observation with Unknown. It does not abort execution, release
//! domain gates, or certify a write seal. Late owner exit is retained separately.
use super::{Client, GenerationKey, SessionStamp, adapter::DrainOps};
use crate::{Error, ErrorKind, Result};
use std::{
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::{Notify, oneshot},
    time::Instant,
};

/// Bounds measured once at begin, including admission/state/writer/receipt waits.
#[derive(Clone, Copy, Debug)]
pub struct OperationLimits {
    /// Original execution budget; waiter replacement never renews it.
    pub run_budget: Duration,
    /// Additional observation/cleanup allowance. No forced task cancellation.
    pub cleanup_budget: Duration,
}
impl Default for OperationLimits {
    fn default() -> Self {
        Self {
            run_budget: Duration::from_secs(10),
            cleanup_budget: Duration::from_secs(2),
        }
    }
}
impl OperationLimits {
    fn deadline(self) -> Result<Instant> {
        let total = self
            .run_budget
            .checked_add(self.cleanup_budget)
            .filter(|total| !total.is_zero() && *total <= Duration::from_secs(3600))
            .ok_or_else(|| unavailable("operation budget must be 0 < total <= one hour"))?;
        Instant::now()
            .checked_add(total)
            .ok_or_else(|| unavailable("operation deadline overflow"))
    }
}
/// Process-local identity; never a replay or reconstruction capability.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct OwnedOperationId {
    /// Original immutable transport generation.
    pub generation: GenerationKey,
    /// Monotonic connection-local invocation number.
    pub ordinal: u64,
}
/// Exact original domain scope in addition to the transport in the ID.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum OperationScope {
    /// Monotonic controller owner ID; never selects the latest controller.
    Control {
        /// Selected original owner.
        session_id: u64,
    },
    /// Original world and opening. Player operations also require an explicit world.
    Inventory {
        /// Original connection/world.
        session: SessionStamp,
        /// Original source/opening.
        source: super::inventory::InventorySource,
    },
    /// Original grid lifetime, including submitted-close or table opening identity.
    Crafting {
        /// Original connection/world.
        session: SessionStamp,
        /// Original crafting basis.
        source: super::crafting::CraftingSource,
    },
    /// Original Body backend/world. The returned ActionId remains separate.
    Body(crate::body::BodyWorld),
}
/// Stable reason for a terminal uncertainty; diagnostic strings are supplementary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum OperationUnknownReason {
    /// Absolute execution plus cleanup allowance elapsed. Writes may still occur.
    Deadline,
    /// An invocation or receipt failed without proof of no future writes.
    Failed,
    /// Owner panic, shutdown cancellation, or lost runtime observer.
    OwnerLost,
    /// The original retained record was superseded; no successor was consumed.
    Superseded,
}
/// Terminal evidence vocabulary. None of these variants means server rollback.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum OperationOutcome {
    /// Owner exited before handing off any work, or a native sender proved its
    /// original attempt was dropped before writing. This invocation cannot write later.
    NotSent,
    /// Complete original write plus the operation's actual receipt predicates.
    /// Inspect the original record for effect/refusal; this is not blanket acceptance.
    SentReceived,
    /// Complete initial local write; server effect is not established by this result.
    SentUnconfirmed,
    /// Delivery/effect or future writes are unknown. Keep gates; never replay.
    Unknown(OperationUnknownReason),
}
/// Diagnostic failure; branch on outcome and kind, not message text.
#[derive(Clone, Debug)]
pub struct OperationFailure {
    /// Original API error category.
    pub kind: ErrorKind,
    /// Original diagnostic, not a stable protocol.
    pub message: String,
}
/// Immutable terminal observation. A deadline may precede the actual owner exit.
#[derive(Clone, Debug)]
pub struct OperationCompletion<T> {
    /// Exact invocation identity.
    pub id: OwnedOperationId,
    /// Original scope; cannot become a successor scope.
    pub scope: OperationScope,
    /// Terminal delivery/receipt disposition.
    pub outcome: OperationOutcome,
    /// Last original record, including its native ID, strict status and gate facts.
    /// Missing means the domain API has not returned its identity yet.
    pub record: Option<T>,
    /// Original diagnostic if available.
    pub failure: Option<OperationFailure>,
    /// Actual registered owner was joined. This does not certify descendant exit.
    pub owner_joined: bool,
}
/// Small synchronous snapshot, independent of domain locks and actor queues.
#[derive(Clone, Debug)]
pub struct OperationSnapshot<T> {
    /// Immutable terminal observation, once available.
    pub completion: Option<Arc<OperationCompletion<T>>>,
    /// Actual late owner exit, retained even after a Deadline completion.
    pub joined: Option<Arc<OperationCompletion<T>>>,
    /// Most recent original record while the owner observes receipts.
    pub record: Option<T>,
}
/// Waiting through an application deadline never cancels an invocation.
#[derive(Clone, Debug)]
pub enum OperationWait<T> {
    /// Fixed operation terminal result.
    Completed(Arc<OperationCompletion<T>>),
    /// Application wait expired earlier than the fixed operation deadline.
    Deadline(OperationSnapshot<T>),
}
/// Passive cloneable observer. Dropping handles/waiters does not cancel owners.
pub struct OwnedOperation<T> {
    state: Arc<OperationState<T>>,
}
impl<T> Clone for OwnedOperation<T> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
        }
    }
}
struct State<T> {
    handed_off: bool,
    terminal: Option<Arc<OperationCompletion<T>>>,
    joined: Option<Arc<OperationCompletion<T>>>,
    record: Option<T>,
}
pub(crate) struct OperationState<T> {
    id: OwnedOperationId,
    scope: OperationScope,
    deadline: Instant,
    state: Mutex<State<T>>,
    changed: Notify,
}
impl<T: Clone> OperationState<T> {
    fn lock(&self) -> std::sync::MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
    pub(crate) fn handoff(&self) {
        self.lock().handed_off = true;
    }
    pub(crate) fn retain(&self, record: T) {
        self.lock().record = Some(record);
    }
    fn unknown(&self, reason: OperationUnknownReason) {
        let mut state = self.lock();
        if state.terminal.is_none() {
            state.terminal = Some(Arc::new(OperationCompletion {
                id: self.id,
                scope: self.scope,
                outcome: OperationOutcome::Unknown(reason),
                record: state.record.clone(),
                failure: None,
                owner_joined: false,
            }));
        }
        drop(state);
        self.changed.notify_waiters();
    }
    fn joined(&self, output: std::result::Result<&OperationResult<T>, ()>) {
        if Instant::now() >= self.deadline {
            self.unknown(OperationUnknownReason::Deadline);
        }
        let mut state = self.lock();
        let (outcome, record, failure) = match output {
            Ok(output) => (
                output.outcome,
                output.record.clone().or_else(|| state.record.clone()),
                output.failure.clone(),
            ),
            Err(()) => (
                if state.handed_off {
                    OperationOutcome::Unknown(OperationUnknownReason::OwnerLost)
                } else {
                    OperationOutcome::NotSent
                },
                state.record.clone(),
                None,
            ),
        };
        let done = Arc::new(OperationCompletion {
            id: self.id,
            scope: self.scope,
            outcome,
            record,
            failure,
            owner_joined: true,
        });
        state.record = done.record.clone();
        state.joined = Some(done.clone());
        state.terminal.get_or_insert(done);
        drop(state);
        self.changed.notify_waiters();
    }
}
impl<T: Clone + Send + Sync + 'static> OwnedOperation<T> {
    /// Exact process-local invocation identity.
    pub fn id(&self) -> OwnedOperationId {
        self.state.id
    }
    /// Exact original domain lifetime.
    pub fn scope(&self) -> OperationScope {
        self.state.scope
    }
    /// Absolute operation deadline; all observers share this value.
    pub fn deadline(&self) -> Instant {
        self.state.deadline
    }
    /// Read cached evidence without I/O or domain state locks.
    pub fn snapshot(&self) -> OperationSnapshot<T> {
        if Instant::now() >= self.state.deadline {
            self.state.unknown(OperationUnknownReason::Deadline);
        }
        let state = self.state.lock();
        OperationSnapshot {
            completion: state.terminal.clone(),
            joined: state.joined.clone(),
            record: state.record.clone(),
        }
    }
    /// Bounded passive wait. Unknown never implies owner exit or no future writes.
    pub async fn finished(&self) -> Arc<OperationCompletion<T>> {
        loop {
            let changed = self.state.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if let Some(done) = self.snapshot().completion {
                return done;
            }
            let _ = tokio::time::timeout_at(self.state.deadline, changed).await;
        }
    }
    /// Use an earlier application deadline without extending the original budget.
    pub async fn wait_until(&self, deadline: Instant) -> OperationWait<T> {
        match tokio::time::timeout_at(deadline, self.finished()).await {
            Ok(done) => OperationWait::Completed(done),
            Err(_) => OperationWait::Deadline(self.snapshot()),
        }
    }
}
pub(crate) struct OperationResult<T> {
    outcome: OperationOutcome,
    record: Option<T>,
    failure: Option<OperationFailure>,
}
impl<T> OperationResult<T> {
    pub(crate) fn record(outcome: OperationOutcome, record: T) -> Self {
        Self {
            outcome,
            record: Some(record),
            failure: None,
        }
    }
    pub(crate) fn error(outcome: OperationOutcome, error: Error) -> Self {
        Self {
            outcome,
            record: None,
            failure: Some(OperationFailure {
                kind: error.kind(),
                message: error.to_string(),
            }),
        }
    }
}
/// Bounds live owners, including those whose deadline observation already ended.
#[derive(Default)]
pub(crate) struct OperationBook(Mutex<(u64, usize)>);
const MAX_OWNED_OPERATIONS: usize = 128;
struct Reservation(Arc<OperationBook>);
impl Drop for Reservation {
    fn drop(&mut self) {
        self.0.0.lock().unwrap_or_else(|p| p.into_inner()).1 -= 1;
    }
}
fn unavailable(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::State, anyhow::anyhow!(message.into()))
}
impl Client {
    pub(crate) fn begin_owned<T, F, Fut>(
        &self,
        scope: OperationScope,
        limits: OperationLimits,
        execute: F,
    ) -> Result<OwnedOperation<T>>
    where
        T: Clone + Send + Sync + 'static,
        F: FnOnce(Arc<OperationState<T>>) -> Fut + Send + 'static,
        Fut: Future<Output = OperationResult<T>> + Send + 'static,
    {
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| unavailable("begin requires a Tokio runtime"))?;
        let deadline = limits.deadline()?;
        let ledger = super::dispatch!(&self.adapter, a => DrainOps::drain_ledger(a));
        ledger.check_normal()?;
        let book = ledger.operation_book();
        let ordinal = {
            let mut book = book.0.lock().unwrap_or_else(|p| p.into_inner());
            if book.1 >= MAX_OWNED_OPERATIONS {
                return Err(Error::new(
                    ErrorKind::ResourceLimit,
                    anyhow::anyhow!("owned operation capacity"),
                ));
            }
            book.0 = book
                .0
                .checked_add(1)
                .ok_or_else(|| unavailable("owned operation IDs exhausted"))?;
            book.1 += 1;
            book.0
        };
        let reservation = Reservation(book);
        let state = Arc::new(OperationState {
            id: OwnedOperationId {
                generation: self.generation_key(),
                ordinal,
            },
            scope,
            deadline,
            state: Mutex::new(State {
                handed_off: false,
                terminal: None,
                joined: None,
                record: None,
            }),
            changed: Notify::new(),
        });
        let (registered, registration) = oneshot::channel();
        let owner = state.clone();
        let joined = state.clone();
        let work = ledger.try_work()?;
        ledger
            .tasks()
            .try_spawn_with_join(
                work.run(async move {
                    // Never pass a fallible registration until begin has committed.
                    if registration.await.is_err() {
                        return OperationResult {
                            outcome: OperationOutcome::NotSent,
                            record: None,
                            failure: None,
                        };
                    }
                    execute(owner).await
                }),
                move |result, _| {
                    joined.joined(result.as_ref().map_err(|_| ()));
                    drop(reservation);
                },
            )
            .map_err(|e| unavailable(format!("operation registration failed: {e}")))?;
        let watchdog = state.clone();
        runtime.spawn(async move {
            loop {
                let changed = watchdog.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if watchdog.lock().terminal.is_some() {
                    return;
                }
                if tokio::time::timeout_at(deadline, changed).await.is_err() {
                    watchdog.unknown(OperationUnknownReason::Deadline);
                    return;
                }
            }
        });
        let _ = registered.send(());
        Ok(OwnedOperation { state })
    }
}

/// Finite observation of irreversible connection retirement, independent of domain locks.
#[derive(Clone, Debug)]
pub enum ConnectionRetirement {
    /// Actual transport halves released and connection-owned tasks joined.
    Closed {
        /// Original revocation fact.
        revocation: super::ConnectionRevocation,
    },
    /// Locally revoked, but teardown could not be certified by the deadline.
    Unconfirmed {
        /// Original revocation fact; never reuse this connection.
        revocation: super::ConnectionRevocation,
    },
    /// Local shutdown completed with an explicit failure; never reuse this connection.
    Failed {
        /// Original revocation fact.
        revocation: super::ConnectionRevocation,
        /// Original shutdown error.
        error: super::ShutdownError,
    },
}
impl Client {
    /// Revoke synchronously before returning a passive finite shutdown observer.
    /// Dropping the future does not retract retirement. This is connection-wide,
    /// irreversible and does not undo already delivered server effects.
    pub fn retire_connection(
        &self,
        deadline: Instant,
    ) -> impl Future<Output = ConnectionRetirement> + Send + '_ {
        let revocation = self.revoke_connection();
        async move {
            match tokio::time::timeout_at(deadline, self.wait_shutdown()).await {
                Ok(Ok(())) => ConnectionRetirement::Closed { revocation },
                Ok(Err(error)) => ConnectionRetirement::Failed { revocation, error },
                Err(_) => ConnectionRetirement::Unconfirmed { revocation },
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) async fn registration_contract(client: Client) {
        let (release, _) = tokio::sync::watch::channel(false);
        let (started, mut starts) = tokio::sync::mpsc::unbounded_channel();
        let mut handles = Vec::new();
        for id in 0..MAX_OWNED_OPERATIONS {
            let mut gate = release.subscribe();
            let started = started.clone();
            handles.push(
                client
                    .begin_owned(
                        OperationScope::Control {
                            session_id: id as u64,
                        },
                        OperationLimits::default(),
                        move |state| async move {
                            state.handoff();
                            started.send(()).unwrap();
                            gate.wait_for(|open| *open).await.unwrap();
                            OperationResult::record(OperationOutcome::NotSent, id)
                        },
                    )
                    .unwrap(),
            );
        }
        for _ in 0..MAX_OWNED_OPERATIONS {
            starts.recv().await.unwrap();
        }
        let invocation = || {
            client.begin_owned(
                OperationScope::Control { session_id: 999 },
                OperationLimits::default(),
                |_| async { OperationResult::record(OperationOutcome::NotSent, 999usize) },
            )
        };
        assert!(matches!(invocation(), Err(error) if error.kind() == ErrorKind::ResourceLimit));
        drop(handles); // No handle owns capacity or execution; the actual owner does.
        assert!(matches!(invocation(), Err(error) if error.kind() == ErrorKind::ResourceLimit));
        let ledger = super::super::dispatch!(&client.adapter, a => DrainOps::drain_ledger(a));
        let book = ledger.operation_book();
        release.send_replace(true);
        tokio::time::timeout(Duration::from_secs(2), async {
            while book.0.lock().unwrap().1 != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let first = invocation().unwrap();
        assert_eq!(first.finished().await.outcome, OperationOutcome::NotSent);
        let key = first.id();
        let panic = client
            .begin_owned(
                OperationScope::Control { session_id: 999 },
                OperationLimits::default(),
                |state| async move {
                    state.handoff();
                    panic!("injected owned operation panic");
                    #[allow(unreachable_code)]
                    OperationResult::record(OperationOutcome::NotSent, 0usize)
                },
            )
            .unwrap();
        assert_ne!(key, panic.id());
        assert_eq!(
            panic.finished().await.outcome,
            OperationOutcome::Unknown(OperationUnknownReason::OwnerLost)
        );
        assert!(panic.snapshot().joined.unwrap().owner_joined);
        let without_runtime = client.clone();
        std::thread::spawn(move || {
            assert!(
                without_runtime
                    .player_control(super::super::GameMode::Survival)
                    .begin_stop_for(1, OperationLimits::default())
                    .is_err()
            );
        })
        .join()
        .unwrap();
        book.0.lock().unwrap().0 = u64::MAX;
        assert!(invocation().is_err());
        assert_eq!(book.0.lock().unwrap().1, 0);
        let _ = client.revoke_connection();
        assert!(invocation().is_err());
    }

    fn state() -> Arc<OperationState<u64>> {
        Arc::new(OperationState {
            id: OwnedOperationId {
                generation: GenerationKey::new(crate::MinecraftVersion::Java1_16_1, 91),
                ordinal: 1,
            },
            scope: OperationScope::Control { session_id: 10 },
            deadline: Instant::now() + Duration::from_secs(1),
            state: Mutex::new(State {
                handed_off: false,
                terminal: None,
                joined: None,
                record: None,
            }),
            changed: Notify::new(),
        })
    }
    #[tokio::test(start_paused = true)]
    async fn owned_deadline_is_terminal_and_late_result_remains_separate() {
        let state = state();
        let handle = OwnedOperation {
            state: state.clone(),
        };
        state.handoff();
        state.retain(11);
        tokio::time::advance(Duration::from_secs(1)).await;
        let deadline = handle.finished().await;
        assert_eq!(
            deadline.outcome,
            OperationOutcome::Unknown(OperationUnknownReason::Deadline)
        );
        assert!(!deadline.owner_joined);
        assert_eq!(deadline.record, Some(11));
        state.joined(Ok(&OperationResult::record(
            OperationOutcome::SentReceived,
            12,
        )));
        assert!(Arc::ptr_eq(&deadline, &handle.finished().await));
        let snapshot = handle.snapshot();
        let late = snapshot.joined.unwrap();
        assert!(late.owner_joined);
        assert_eq!(late.outcome, OperationOutcome::SentReceived);
        assert_eq!(late.record, Some(12));
        assert_eq!(late.id, deadline.id);
    }
    #[tokio::test(start_paused = true)]
    async fn owned_replaced_waiters_cannot_extend_budget_or_cancel_owner() {
        use std::future::Future;
        let state = state();
        let handle = OwnedOperation {
            state: state.clone(),
        };
        for _ in 0..4 {
            let mut waiter = Box::pin(handle.finished());
            std::future::poll_fn(|cx| {
                assert!(waiter.as_mut().poll(cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            drop(waiter);
            tokio::time::advance(Duration::from_millis(250)).await;
        }
        assert_eq!(
            handle.finished().await.outcome,
            OperationOutcome::Unknown(OperationUnknownReason::Deadline)
        );
        assert!(state.lock().joined.is_none());
    }
    #[tokio::test(start_paused = true)]
    async fn owned_application_deadline_does_not_end_original_operation() {
        let state = state();
        let handle = OwnedOperation {
            state: state.clone(),
        };
        assert!(matches!(
            handle.wait_until(Instant::now()).await,
            OperationWait::Deadline(_)
        ));
        assert!(handle.snapshot().completion.is_none());
        state.joined(Ok(&OperationResult::record(OperationOutcome::NotSent, 0)));
        assert_eq!(handle.finished().await.outcome, OperationOutcome::NotSent);
    }
    #[test]
    fn owned_owner_loss_before_handoff_is_the_only_generic_not_sent_proof() {
        let first = state();
        first.joined(Err(()));
        assert_eq!(
            first.lock().terminal.as_ref().unwrap().outcome,
            OperationOutcome::NotSent
        );
        let second = state();
        second.handoff();
        second.joined(Err(()));
        assert_eq!(
            second.lock().terminal.as_ref().unwrap().outcome,
            OperationOutcome::Unknown(OperationUnknownReason::OwnerLost)
        );
    }
}

// Native stop needs an invocation-specific write fact, including the legitimate
// no-op case (already stopped, old world, or no held wire keys). Only propagate
// to the selected stop child; ticks and unrelated release tasks must not inherit it.
pub(crate) type StopObserver = Arc<Mutex<OperationOutcome>>;
tokio::task_local! { pub(crate) static STOP_OBSERVER: StopObserver; }
pub(crate) fn capture_stop_observer() -> Option<StopObserver> {
    STOP_OBSERVER
        .try_with(|observer| {
            *observer.lock().unwrap_or_else(|p| p.into_inner()) =
                OperationOutcome::Unknown(OperationUnknownReason::Failed);
            observer.clone()
        })
        .ok()
}
pub(crate) async fn with_stop_observer<F: Future>(
    observer: Option<StopObserver>,
    future: F,
) -> F::Output {
    match observer {
        Some(observer) => STOP_OBSERVER.scope(observer, future).await,
        None => future.await,
    }
}
pub(crate) fn complete_stop(sent: bool) {
    let _ = STOP_OBSERVER.try_with(|observer| {
        *observer.lock().unwrap_or_else(|p| p.into_inner()) = if sent {
            OperationOutcome::SentUnconfirmed
        } else {
            OperationOutcome::NotSent
        };
    });
}
