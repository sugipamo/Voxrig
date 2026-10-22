//! Finite connection owners and retained receive/inspection dependencies.
//! These are ordinary ledger roots/children; task-local propagation only hands
//! the existing root to nested sends on the SAME transport. It grants no bypass.
use super::*;
use std::future::Future;

pub(crate) fn admission_error(error: AdmissionError) -> crate::Error {
    crate::Error::new(
        crate::ErrorKind::State,
        anyhow::anyhow!("connection admission closed: {error:?}"),
    )
}

tokio::task_local! {
    static CURRENT: ScopedWork;
}

impl Ledger {
    pub(crate) fn inherited(&self) -> bool {
        CURRENT
            .try_with(|parent| Arc::ptr_eq(&self.shared, &parent.guard().shared))
            .unwrap_or(false)
    }
    pub(crate) fn try_work(&self) -> crate::Result<WorkGuard> {
        CURRENT
            .try_with(|parent| {
                Arc::ptr_eq(&self.shared, &parent.guard().shared)
                    .then(|| parent.guard().child().expect("live finite owner"))
            })
            .ok()
            .flatten()
            .map_or_else(|| self.admit().map_err(admission_error), Ok)
    }
    pub(crate) fn work(&self) -> WorkGuard {
        self.try_work().expect("existing finite admission")
    }
}
impl WorkGuard {
    pub(crate) fn spawn<F>(self, future: F) -> crate::client::shutdown::Task<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let shared = self.shared.clone();
        shared.shutdown.spawn(self.run(future))
    }

    // Box at construction, rather than retaining F inside an async helper's
    // frame. Native composite inventory helpers nest several such boundaries;
    // their future size must not multiply on the caller's thread stack.
    pub(crate) fn run_scoped<F: Future>(self, future: F) -> impl Future<Output = F::Output> {
        CURRENT.scope(ScopedWork::new(self), Box::pin(future))
    }
    pub(crate) fn run_result<T, E>(
        self,
        future: impl Future<Output = Result<T, E>>,
    ) -> impl Future<Output = Result<T, E>> {
        let future = Box::pin(future);
        async move {
            let child = self.child().expect("finite child");
            let result = CURRENT.scope(ScopedWork::new(child), future).await;
            self.finish(if result.is_err() {
                Outcome::Failed(Failure::TransactionUnconfirmed)
            } else {
                Outcome::Complete
            });
            result
        }
    }

    pub(crate) fn run<F: Future>(self, future: F) -> impl Future<Output = F::Output> {
        let future = Box::pin(future);
        async move {
            let child = self.child().expect("finite child");
            let output = CURRENT.scope(ScopedWork::new(child), future).await;
            self.finish(Outcome::Complete);
            output
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Family {
    Swap,
    HandSwap,
    SelectedDrop,
    Click,
    Transfer,
    Crafting,
    Recipe,
    Placement,
    Mining,
    ContainerOpen,
    Creative(u8),
}

#[derive(Default)]
pub(crate) struct Retained {
    entries: BTreeMap<Family, (u64, Arc<PendingWork>)>,
}
impl Retained {
    pub(crate) fn child(&self, family: Family) -> Option<WorkGuard> {
        self.entries.get(&family).and_then(|(_, work)| work.child())
    }

    pub(crate) fn begin(&mut self, ledger: &Ledger, family: Family, id: u64) -> WorkGuard {
        // Every accepted call obtains its own admission (or an existing scoped
        // parent's child), even when a native pending marker coalesces updates.
        let work = ledger.work();
        if let Some((previous, pending)) = self.entries.get(&family) {
            if *previous == id {
                if let Some(child) = pending.child() {
                    child.finish(Outcome::NoWrite);
                    // The older pending owner already covers every cut that
                    // includes this newer call. Keep it until received cleanup;
                    // the new raw owner covers this call's complete send.
                    return work;
                }
            }
        }
        let child = work.child().expect("retained continuation child");
        // Replacing unresolved authority fails through WorkGuard::drop.
        self.entries
            .insert(family, (id, Arc::new(PendingWork::new(work))));
        child
    }
    // An expiry task has no write authority and cannot prolong a completed
    // record. It only classifies the EXISTING deadline; public state is untouched.
    pub(crate) fn expire_at(&self, family: Family, id: u64, deadline: tokio::time::Instant) {
        if let Some((current, work)) = self.entries.get(&family) {
            if *current == id {
                let work = Arc::downgrade(work);
                let shared = &self.entries.get(&family).expect("retained work").1;
                let Some(shared) = shared.child() else {
                    return;
                };
                let registry = shared.shared.clone();
                shared.finish(Outcome::NoWrite);
                registry.shutdown.spawn(async move {
                    tokio::time::sleep_until(deadline).await;
                    if let Some(work) = work.upgrade() {
                        work.finish(Outcome::Failed(Failure::TransactionExpired));
                    }
                });
            }
        }
    }
    pub(crate) fn reconcile(&mut self, family: Family, state: Option<(u64, bool, bool)>) {
        let Some((id, _)) = self.entries.get(&family) else {
            return;
        };
        let outcome = match state {
            Some((current, complete, failed)) if current == *id => {
                if failed {
                    Some(Outcome::Failed(Failure::TransactionUnconfirmed))
                } else if complete {
                    Some(Outcome::Complete)
                } else {
                    None
                }
            }
            _ => Some(Outcome::Failed(Failure::OwnerLost)),
        };
        if let Some(outcome) = outcome {
            self.entries
                .remove(&family)
                .expect("retained owner")
                .1
                .finish(outcome);
        }
    }
}

// Preserve the public outcome while permanently classifying failed finite work.
pub(crate) fn checked<T, E>(result: Result<T, E>) -> Result<T, E> {
    if result.is_err() {
        let _ = CURRENT.try_with(|owner| {
            owner
                .guard()
                .child()
                .expect("live failed owner")
                .finish(Outcome::Failed(Failure::TransactionUnconfirmed));
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ledger(id: u64) -> Ledger {
        Ledger::tracking(GenerationKey {
            version: MinecraftVersion::Java1_16_1,
            connection_id: id,
        })
    }
    #[tokio::test]
    async fn finite_generic_failure_keeps_unconfirmed_fallback() {
        let ledger = ledger(1);
        let result = ledger.work().run_result(async { Err::<(), _>(()) }).await;
        assert!(result.is_err());
        assert_eq!(
            ledger.cut(false).wait().await.unwrap_err().public().kind(),
            crate::client::DrainErrorKind::TransactionUnconfirmed
        );
    }
    #[tokio::test]
    async fn finite_observer_timeout_is_not_transaction_expiry() {
        let ledger = ledger(1);
        let work = ledger.work();
        let mut drain = ledger.cut(false);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(1), drain.wait())
                .await
                .is_err()
        );
        work.finish(Outcome::Complete);
        drain.wait().await.unwrap();
    }
    #[tokio::test]
    async fn ledger_finite_late_writes_inherit_the_cut_after_root_closure() {
        let ledger = ledger(1);
        let work = ledger.work();
        let mut drain = ledger.cut(true);
        work.run(async {
            let late = ledger.work();
            assert!(drain.inspect().is_none());
            late.begin_write().unwrap().commit();
            late.finish(Outcome::Complete);
            assert!(drain.inspect().is_none());
        })
        .await;
        assert_eq!(drain.wait().await.unwrap().written_through, 1);
    }
    #[tokio::test]
    async fn ledger_finite_dropped_caller_cannot_cancel_transferred_continuation() {
        let ledger = ledger(1);
        let (started, start) = tokio::sync::oneshot::channel();
        let (release, wait) = tokio::sync::oneshot::channel();
        let source = ledger.clone();
        let caller = tokio::spawn(ledger.work().run_scoped(async move {
            let work = source.work();
            tokio::spawn(work.run(async move {
                wait.await.unwrap();
            }));
            started.send(()).unwrap();
            std::future::pending::<()>().await;
        }));
        start.await.unwrap();
        let mut drain = ledger.cut(false);
        caller.abort();
        let _ = caller.await;
        assert!(drain.inspect().is_none());
        release.send(()).unwrap();
        drain.wait().await.unwrap();
    }
    #[tokio::test]
    async fn ledger_finite_caller_owned_timer_drop_is_sticky_without_a_release_write() {
        let ledger = ledger(1);
        let (started, wait) = tokio::sync::oneshot::channel();
        let source = ledger.clone();
        let caller = tokio::spawn(ledger.work().run_result(async move {
            let send = source.work();
            send.begin_write().unwrap().commit();
            send.finish(Outcome::Complete);
            started.send(()).unwrap();
            std::future::pending::<()>().await;
            Ok::<(), ()>(())
        }));
        wait.await.unwrap();
        let mut drain = ledger.cut(false);
        assert!(drain.inspect().is_none());
        caller.abort();
        let _ = caller.await;
        assert!(drain.wait().await.is_err());
        assert_eq!(ledger.test_written_through(), 1);
    }
    #[test]
    fn ledger_finite_retained_records_are_debt_after_the_send_worker_exits() {
        let ledger = ledger(1);
        let mut retained = Retained::default();
        retained
            .begin(&ledger, Family::Crafting, 7)
            .finish(Outcome::Complete);
        let mut drain = ledger.cut(false);
        retained.reconcile(Family::Crafting, Some((7, false, false)));
        assert!(drain.inspect().is_none());
        retained.reconcile(Family::Crafting, Some((7, true, false)));
        assert!(drain.inspect().unwrap().is_ok());
    }
    #[tokio::test]
    async fn ledger_finite_retained_expiry_is_sticky_and_completion_disarms_expiry() {
        for complete in [false, true] {
            let ledger = ledger(1);
            let mut retained = Retained::default();
            retained
                .begin(&ledger, Family::Click, 1)
                .finish(Outcome::Complete);
            retained.expire_at(Family::Click, 1, tokio::time::Instant::now());
            let mut drain = ledger.cut(false);
            if complete {
                retained.reconcile(Family::Click, Some((1, true, false)));
            }
            if !complete {
                assert_eq!(
                    drain.wait().await.unwrap_err().public().kind(),
                    crate::client::DrainErrorKind::TransactionExpired
                );
            } else {
                drain.wait().await.unwrap();
            }
            retained.reconcile(Family::Click, Some((1, true, false)));
            assert_eq!(ledger.cut(false).wait().await.is_ok(), complete);
        }
    }
    #[test]
    fn ledger_finite_repeated_pending_marker_cannot_bypass_new_root_closure() {
        let ledger = ledger(1);
        let mut retained = Retained::default();
        retained
            .begin(&ledger, Family::Creative(0), 1)
            .finish(Outcome::Complete);
        let mut drain = ledger.cut(true);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                retained.begin(&ledger, Family::Creative(0), 1)
            }))
            .is_err()
        );
        assert!(drain.inspect().is_none());
        retained.reconcile(Family::Creative(0), Some((1, true, false)));
        assert!(drain.inspect().unwrap().is_ok());
    }
    #[test]
    fn ledger_finite_stale_reconciliation_never_releases_a_new_owner() {
        let ledger = ledger(1);
        let mut retained = Retained::default();
        retained
            .begin(&ledger, Family::Placement, 1)
            .finish(Outcome::Complete);
        retained
            .begin(&ledger, Family::Placement, 2)
            .finish(Outcome::Complete);
        retained.reconcile(Family::Placement, Some((1, true, false)));
        assert!(ledger.cut(false).inspect().unwrap().is_err());
    }
    #[tokio::test]
    async fn ledger_finite_recovery_and_same_key_instances_do_not_cross_ledgers() {
        let original = ledger(1);
        let replacement = ledger(1);
        original
            .work()
            .run(async {
                let fresh = replacement.work();
                let mut replacement_drain = replacement.cut(false);
                assert!(replacement_drain.inspect().is_none());
                assert!(original.cut(false).inspect().is_none());
                fresh.begin_write().unwrap().commit();
                fresh.finish(Outcome::Complete);
                assert_eq!(replacement_drain.wait().await.unwrap().written_through, 1);
            })
            .await;
        assert_eq!(original.cut(false).wait().await.unwrap().written_through, 0);
    }
}
