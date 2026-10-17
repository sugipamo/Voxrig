//! Passive completion and private connection task ownership.
use super::GenerationKey;
use std::future::Future;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot, watch};

/// Failure to certify local transport teardown. Diagnostic text is unstable.
#[derive(Clone, Debug)]
pub struct ShutdownError {
    generation: GenerationKey,
    kind: ShutdownErrorKind,
    cause: Arc<dyn std::error::Error + Send + Sync>,
}
impl ShutdownError {
    /// Exact original transport generation.
    pub fn generation(&self) -> GenerationKey {
        self.generation
    }
    /// Stable failure category.
    pub fn kind(&self) -> ShutdownErrorKind {
        self.kind
    }
}
impl std::fmt::Display for ShutdownError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "shutdown {:?} for {:?}: {}",
            self.kind, self.generation, self.cause
        )
    }
}
impl std::error::Error for ShutdownError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.cause.as_ref())
    }
}
/// Local shutdown failure; every error withholds a successful completion certificate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ShutdownErrorKind {
    /// The local writer shutdown failed; remaining tasks were still joined.
    WriterShutdownFailed,
    /// A connection task panicked or was unexpectedly cancelled.
    TaskFailed,
    /// Completion bookkeeping could not be trusted.
    Internal,
}
pub(crate) struct OnExit<F: FnOnce()>(pub(crate) Option<F>);
impl<F: FnOnce()> Drop for OnExit<F> {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f();
        }
    }
}

type Result = std::result::Result<(), ShutdownError>;
type Teardown = std::pin::Pin<Box<dyn Future<Output = std::io::Result<()>> + Send>>;
struct TaskFailure {
    cause: Arc<tokio::task::JoinError>,
    expected: bool,
}
type Joined = std::pin::Pin<Box<dyn Future<Output = Option<TaskFailure>> + Send>>;
enum Command {
    Task {
        abort: tokio::task::AbortHandle,
        join: Joined,
    },
    Stop(Teardown),
}
/// A caller-owned output future. No detached task is needed to observe an output.
/// The registry exclusively owns and awaits the actual Tokio JoinHandle.
pub(crate) struct Task<T> {
    receive: oneshot::Receiver<std::result::Result<T, TaskError>>,
    abort: Option<tokio::task::AbortHandle>,
}
impl<T> Task<T> {
    pub(crate) fn abort_handle(&self) -> Option<tokio::task::AbortHandle> {
        self.abort.clone()
    }
}
impl<T> Future for Task<T> {
    type Output = std::result::Result<T, TaskError>;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        match std::pin::Pin::new(&mut self.get_mut().receive).poll(cx) {
            std::task::Poll::Ready(Ok(result)) => std::task::Poll::Ready(result),
            std::task::Poll::Ready(Err(_)) => std::task::Poll::Ready(Err(TaskError(Arc::new(
                std::io::Error::other("connection task output unavailable"),
            )))),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}
#[derive(Debug)]
pub(crate) struct TaskError(Arc<dyn std::error::Error + Send + Sync>);
impl std::fmt::Display for TaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self.0.as_ref(), f)
    }
}
impl std::error::Error for TaskError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}
struct State {
    sender: Option<mpsc::UnboundedSender<Command>>,
    closing: bool,
}
/// Admission and closing share this gate, independently of finite ledger debt.
/// The supervisor owns actual JoinHandles; returned futures observe task outputs.
pub(crate) struct Registry {
    generation: GenerationKey,
    runtime: Option<tokio::runtime::Handle>,
    termination_requested: Arc<std::sync::atomic::AtomicBool>,
    state: Mutex<State>,
    result: watch::Sender<Option<Result>>,
}
impl Registry {
    pub(crate) fn new(generation: GenerationKey) -> Self {
        Self {
            generation,
            runtime: tokio::runtime::Handle::try_current().ok(),
            termination_requested: Arc::default(),
            state: Mutex::new(State {
                sender: None,
                closing: false,
            }),
            result: watch::channel(None).0,
        }
    }
    fn internal(&self, message: &'static str) {
        self.result.send_if_modified(|result| {
            if result.is_some() {
                return false;
            }
            *result = Some(Err(error(
                self.generation,
                ShutdownErrorKind::Internal,
                std::io::Error::other(message),
            )));
            true
        });
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poison| {
            self.internal("poisoned shutdown registry");
            let mut state = poison.into_inner();
            state.closing = true;
            state
        })
    }
    fn sender(&self, state: &mut State) -> mpsc::UnboundedSender<Command> {
        state
            .sender
            .get_or_insert_with(|| {
                let (sender, receiver) = mpsc::unbounded_channel();
                self.runtime
                    .clone()
                    .unwrap_or_else(tokio::runtime::Handle::current)
                    .spawn(supervise(self.generation, receiver, self.result.clone()));
                sender
            })
            .clone()
    }
    pub(crate) fn spawn<F>(&self, future: F) -> Task<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.try_spawn(future).unwrap_or_else(|_| {
            let (_, receive) = oneshot::channel();
            Task {
                receive,
                abort: None,
            }
        })
    }
    /// Successful submission and close are serialized by the registry gate.
    /// Never destroy a rejected future/command under that gate: destructors may
    /// retire ledger debt or re-enter shutdown (including last-owner Drop).
    pub(crate) fn try_spawn<F>(&self, future: F) -> std::result::Result<Task<F::Output>, TaskError>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let mut state = self.lock();
        if state.closing {
            drop(state);
            drop(future);
            return Err(TaskError(Arc::new(std::io::Error::other(
                "task registry closed",
            ))));
        }
        let runtime = self
            .runtime
            .clone()
            .or_else(|| tokio::runtime::Handle::try_current().ok());
        let Some(runtime) = runtime else {
            drop(state);
            drop(future);
            return Err(TaskError(Arc::new(std::io::Error::other(
                "no Tokio runtime for task submission",
            ))));
        };
        let sender = self.sender(&mut state);
        let (send, receive) = oneshot::channel();
        let task = runtime.spawn(future);
        let abort = task.abort_handle();
        let termination_requested = self.termination_requested.clone();
        let join = Box::pin(async move {
            match task.await {
                Ok(output) => {
                    let _ = send.send(Ok(output));
                    None
                }
                Err(cause) => {
                    let cause = Arc::new(cause);
                    let expected = cause.is_cancelled()
                        && termination_requested.load(std::sync::atomic::Ordering::Acquire);
                    let _ = send.send(Err(TaskError(cause.clone())));
                    Some(TaskFailure { cause, expected })
                }
            }
        });
        if let Err(rejected) = sender.send(Command::Task {
            abort: abort.clone(),
            join,
        }) {
            state.closing = true;
            abort.abort();
            self.internal("shutdown task observer lost");
            drop(state);
            drop(rejected);
            return Err(TaskError(Arc::new(std::io::Error::other(
                "shutdown task observer lost",
            ))));
        }
        Ok(Task {
            receive,
            abort: Some(abort),
        })
    }
    pub(crate) fn start(
        &self,
        teardown: impl Future<Output = std::io::Result<()>> + Send + 'static,
    ) {
        let mut state = self.lock();
        if state.closing {
            return;
        }
        state.closing = true;
        self.termination_requested
            .store(true, std::sync::atomic::Ordering::Release);
        let submitted = self
            .sender(&mut state)
            .send(Command::Stop(Box::pin(teardown)));
        if submitted.is_err() {
            self.internal("shutdown coordinator lost");
        }
        drop(state);
        drop(submitted); // Teardown destructors must also run outside the registry.
    }
    pub(crate) async fn wait(&self) -> Result {
        let mut result = self.result.subscribe();
        loop {
            if let Some(result) = result.borrow_and_update().clone() {
                return result;
            }
            if result.changed().await.is_err() {
                return Err(error(
                    self.generation,
                    ShutdownErrorKind::Internal,
                    std::io::Error::other("shutdown publisher lost"),
                ));
            }
        }
    }
}
fn error(
    generation: GenerationKey,
    kind: ShutdownErrorKind,
    cause: impl std::error::Error + Send + Sync + 'static,
) -> ShutdownError {
    ShutdownError {
        generation,
        kind,
        cause: Arc::new(cause),
    }
}
async fn supervise(
    generation: GenerationKey,
    mut commands: mpsc::UnboundedReceiver<Command>,
    result: watch::Sender<Option<Result>>,
) {
    let mut tasks = tokio::task::JoinSet::new();
    let mut first_error = None;
    let mut aborts = std::collections::HashMap::new();
    let teardown = loop {
        tokio::select! {
            biased;
            command = commands.recv() => match command {
                Some(Command::Task { abort, join }) => {
                    let watcher = tasks.spawn(join);
                    aborts.insert(watcher.id(), abort);
                }
                Some(Command::Stop(teardown)) => break teardown,
                None => {
                    first_error.get_or_insert_with(|| error(generation, ShutdownErrorKind::Internal, std::io::Error::other("shutdown authority lost")));
                    break Box::pin(async { Ok(()) }) as Teardown;
                }
            },
            Some(joined) = tasks.join_next_with_id(), if !tasks.is_empty() => {
                match joined {
                    Ok((id, outcome)) => {
                        aborts.remove(&id);
                        if let Some(failure) = outcome { if !failure.expected { first_error.get_or_insert_with(|| ShutdownError { generation, kind: ShutdownErrorKind::TaskFailed, cause: failure.cause }); } }
                    },
                    Err(cause) => { first_error.get_or_insert_with(|| error(generation, ShutdownErrorKind::Internal, cause)); }
                }
            }
        }
    };
    for abort in aborts.values() {
        abort.abort();
    }
    while let Some(joined) = tasks.join_next_with_id().await {
        match joined {
            Ok((id, outcome)) => {
                aborts.remove(&id);
                if let Some(failure) = outcome {
                    if !failure.expected {
                        first_error.get_or_insert_with(|| ShutdownError {
                            generation,
                            kind: ShutdownErrorKind::TaskFailed,
                            cause: failure.cause,
                        });
                    }
                }
            }
            Err(cause) => {
                first_error
                    .get_or_insert_with(|| error(generation, ShutdownErrorKind::Internal, cause));
            }
        }
    }
    if let Err(cause) = teardown.await {
        first_error.get_or_insert_with(|| {
            error(generation, ShutdownErrorKind::WriterShutdownFailed, cause)
        });
    }
    let mut terminal = Some(first_error.map_or(Ok(()), Err));
    result.send_if_modified(|result| {
        if result.is_some() {
            return false;
        }
        *result = terminal.take();
        true
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    fn registry() -> Registry {
        Registry::new(GenerationKey::new(crate::MinecraftVersion::Java1_16_1, 42))
    }
    #[test]
    fn shutdown_public_contracts_and_future_construction_need_no_runtime() {
        fn traits<T: Send + Sync + Unpin + 'static>() {}
        traits::<ShutdownError>();
        traits::<ShutdownErrorKind>();
        let registry = registry();
        let future = registry.wait();
        fn send(_: impl Future<Output = Result> + Send) {}
        send(future);
    }
    #[tokio::test]
    async fn shutdown_cancelled_observation_cannot_cancel_owned_teardown() {
        let registry = registry();
        let (release, blocked) = oneshot::channel();
        registry.start(async move {
            blocked.await.unwrap();
            Ok(())
        });
        let mut wait = Box::pin(registry.wait());
        std::future::poll_fn(|cx| {
            assert!(wait.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        drop(wait);
        release.send(()).unwrap();
        // There is no active waiter while the owner completes and caches its result.
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while registry.result.borrow().is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        registry.wait().await.unwrap();
    }
    #[tokio::test]
    async fn shutdown_poison_fails_closed_with_cached_internal_error() {
        let registry = registry();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _gate = registry.state.lock().unwrap();
            panic!("injected registry invariant failure");
        }));
        registry.start(async { Ok(()) });
        let first = registry.wait().await.unwrap_err();
        let later = registry.wait().await.unwrap_err();
        assert_eq!(first.kind(), ShutdownErrorKind::Internal);
        assert!(Arc::ptr_eq(&first.cause, &later.cause));
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_joins_actual_task_after_abort_including_blocked_destructor() {
        let registry = registry();
        let (started, start) = oneshot::channel();
        let (dropping, drop_started) = oneshot::channel();
        let release = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let gate = release.clone();
        registry.spawn(async move {
            let _cleanup = OnExit(Some(move || {
                dropping.send(()).unwrap();
                let (lock, wake) = &*gate;
                let _guard = wake
                    .wait_while(lock.lock().unwrap(), |ready| !*ready)
                    .unwrap();
            }));
            started.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        start.await.unwrap();
        let torn_down = Arc::new(AtomicBool::new(false));
        let done = torn_down.clone();
        registry.start(async move {
            done.store(true, Ordering::Release);
            Ok(())
        });
        drop_started.await.unwrap();
        assert!(!torn_down.load(Ordering::Acquire));
        let mut wait = Box::pin(registry.wait());
        std::future::poll_fn(|cx| {
            assert!(wait.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        *release.0.lock().unwrap() = true;
        release.1.notify_one();
        wait.await.unwrap();
        assert!(torn_down.load(Ordering::Acquire));
    }
    #[tokio::test]
    async fn shutdown_preserves_writer_error_for_all_waiters_and_later_calls() {
        let registry = registry();
        registry.start(async { Err(std::io::Error::other("writer teardown failed")) });
        let (a, b) = tokio::join!(registry.wait(), registry.wait());
        let a = a.unwrap_err();
        let b = b.unwrap_err();
        let later = registry.wait().await.unwrap_err();
        assert_eq!(a.kind(), ShutdownErrorKind::WriterShutdownFailed);
        assert_eq!(a.generation(), registry.generation);
        assert!(Arc::ptr_eq(&a.cause, &b.cause));
        assert!(Arc::ptr_eq(&a.cause, &later.cause));
        assert!(std::error::Error::source(&a).is_some());
    }
    #[tokio::test]
    async fn shutdown_unexpected_task_cancel_is_not_hidden_by_later_shutdown() {
        let registry = registry();
        let task = registry.spawn(std::future::pending::<()>());
        task.abort_handle().unwrap().abort();
        assert!(task.await.is_err());
        registry.start(async { Ok(()) });
        assert_eq!(
            registry.wait().await.unwrap_err().kind(),
            ShutdownErrorKind::TaskFailed
        );
    }
    #[tokio::test]
    async fn shutdown_task_panic_retains_source_and_still_tears_down_writer() {
        let registry = registry();
        let panic = registry.spawn(async { panic!("connection worker panic") });
        assert!(panic.await.is_err());
        let cleaned = Arc::new(AtomicBool::new(false));
        let done = cleaned.clone();
        registry.start(async move {
            done.store(true, Ordering::Release);
            Ok(())
        });
        let error = registry.wait().await.unwrap_err();
        assert_eq!(error.kind(), ShutdownErrorKind::TaskFailed);
        assert!(
            std::error::Error::source(&error)
                .unwrap()
                .downcast_ref::<tokio::task::JoinError>()
                .unwrap()
                .is_panic()
        );
        assert!(cleaned.load(Ordering::Acquire));
    }
    #[tokio::test]
    async fn shutdown_rejects_late_child_and_keeps_generations_separate() {
        let old = Arc::new(registry());
        let fresh = registry();
        let (accepted, parent_started) = oneshot::channel();
        let source = old.clone();
        old.spawn(async move {
            source.spawn(std::future::pending::<()>());
            accepted.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        parent_started.await.unwrap();
        old.start(async { Ok(()) });
        let ran = Arc::new(AtomicBool::new(false));
        let flag = ran.clone();
        let rejected = old.spawn(async move {
            flag.store(true, Ordering::Release);
        });
        assert!(rejected.await.is_err());
        old.wait().await.unwrap();
        assert!(!ran.load(Ordering::Acquire));
        let mut wait = Box::pin(fresh.wait());
        std::future::poll_fn(|cx| {
            assert!(wait.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        fresh.start(async { Ok(()) });
        wait.await.unwrap();
    }
    #[tokio::test]
    async fn checked_submission_rejection_drops_outside_registry_and_retires_work() {
        for version in [
            crate::MinecraftVersion::Java1_16_1,
            crate::MinecraftVersion::Java1_21_11,
        ] {
            for poisoned in [false, true] {
                let ledger =
                    crate::client::ledger::Ledger::tracking(crate::client::ledger::GenerationKey {
                        version,
                        connection_id: 42,
                    });
                let registry = ledger.tasks();
                if poisoned {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let _lock = registry.state.lock().unwrap();
                        panic!("injected registry poison");
                    }));
                } else {
                    registry.start(async { Ok(()) });
                }
                let work = ledger.work();
                let dropped = Arc::new(AtomicBool::new(false));
                let flag = dropped.clone();
                let source = ledger.clone();
                let cleanup = OnExit(Some(move || {
                    // A rejected unpolled future may re-enter shutdown on Drop.
                    source.tasks().start(async { Ok(()) });
                    drop(work);
                    flag.store(true, Ordering::Release);
                }));
                let rejected = registry.try_spawn(async move {
                    let _cleanup = cleanup;
                    std::future::pending::<()>().await;
                });
                assert!(rejected.is_err());
                assert!(dropped.load(Ordering::Acquire));
                assert_eq!(ledger.test_stop_debt().1, 0);
                assert_eq!(
                    ledger.check_terminal().unwrap_err().kind(),
                    crate::client::DrainErrorKind::OwnerLost
                );
                if poisoned {
                    assert_eq!(
                        registry.wait().await.unwrap_err().kind(),
                        ShutdownErrorKind::Internal
                    );
                } else {
                    registry.wait().await.unwrap();
                }
            }
        }
    }

    #[tokio::test]
    async fn checked_submission_and_shutdown_race_is_registered_or_rejected() {
        for _ in 0..32 {
            let registry = Arc::new(registry());
            let gate = Arc::new(std::sync::Barrier::new(3));
            let dropped = Arc::new(AtomicBool::new(false));
            let flag = dropped.clone();
            let cleanup = OnExit(Some(move || {
                flag.store(true, Ordering::Release);
            }));
            let submitter = {
                let registry = registry.clone();
                let gate = gate.clone();
                std::thread::spawn(move || {
                    gate.wait();
                    registry.try_spawn(async move {
                        let _cleanup = cleanup;
                        std::future::pending::<()>().await;
                    })
                })
            };
            let closer = {
                let registry = registry.clone();
                let gate = gate.clone();
                std::thread::spawn(move || {
                    gate.wait();
                    registry.start(async { Ok(()) });
                })
            };
            gate.wait();
            let submission = submitter.join().unwrap();
            closer.join().unwrap();
            if let Ok(task) = submission {
                assert!(task.abort_handle().is_some());
                assert!(task.await.is_err());
            }
            registry.wait().await.unwrap();
            assert!(dropped.load(Ordering::Acquire));
        }
    }

    #[tokio::test]
    async fn checked_submission_detects_lost_supervisor() {
        let registry = registry();
        let (sender, receiver) = mpsc::unbounded_channel();
        drop(receiver);
        registry.lock().sender = Some(sender);
        assert!(registry.try_spawn(std::future::pending::<()>()).is_err());
        assert!(registry.lock().closing);
        assert_eq!(
            registry.wait().await.unwrap_err().kind(),
            ShutdownErrorKind::Internal
        );
    }
}
