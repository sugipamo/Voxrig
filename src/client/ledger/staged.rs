//! Staged admission classes share the receipt/revocation gate.
#![allow(clippy::result_large_err)] // The approved StopError carries inline diagnostics.
use super::*;
use crate::client::drain::{DrainError, DrainErrorKind, GenerationKey as PublicKey};

impl Ledger {
    pub(crate) fn generation(&self) -> PublicKey {
        PublicKey::new(
            self.shared.generation.version,
            self.shared.generation.connection_id,
        )
    }
    pub(crate) fn set_ready(&self, ready: bool) {
        self.shared.lock().ready = ready;
    }
    pub(crate) fn ready(&self) -> bool {
        self.shared.lock().ready
    }
    pub(crate) fn quiescing(&self) -> bool {
        self.shared.lock().quiescing
    }
    pub(crate) fn while_open(&self, action: impl FnOnce()) {
        let state = self.shared.lock();
        if !state.closed {
            action();
        }
    }
    pub(crate) fn normal_open(&self) -> bool {
        !self.shared.lock().closed
    }
    pub(crate) fn check_normal(&self) -> crate::Result<()> {
        if self.normal_open() || self.inherited() {
            Ok(())
        } else {
            Err(finite::admission_error(AdmissionError::Closed))
        }
    }
    pub(crate) fn check_terminal(&self) -> Result<(), DrainError> {
        let state = self.shared.lock();
        if state.terminal.is_some() {
            Err(self.error_locked(&state, DrainErrorKind::Internal, None))
        } else {
            Ok(())
        }
    }
    pub(crate) fn error(&self, kind: DrainErrorKind) -> DrainError {
        let state = self.shared.lock();
        self.error_locked(&state, kind, None)
    }
    fn error_locked(
        &self,
        state: &State,
        fallback: DrainErrorKind,
        cut: Option<u64>,
    ) -> DrainError {
        let (kind, root) = state
            .terminal
            .map_or((fallback, None), |t| (failure_kind(t.cause), t.root));
        DrainError::new(
            kind,
            self.generation(),
            None,
            cut,
            state.written_through,
            root,
        )
    }
    pub(crate) fn begin(&self, key: PublicKey, ready: bool) -> Result<Drain, DrainError> {
        let changed = self.shared.changed.subscribe();
        let mut state = self.shared.lock();
        if state.terminal.is_some() {
            return Err(self.error_locked(&state, DrainErrorKind::Internal, None));
        }
        if key != self.generation() {
            return Err(DrainError::new(
                DrainErrorKind::WrongGeneration,
                self.generation(),
                Some(key),
                None,
                state.written_through,
                None,
            ));
        }
        if state.quiescing {
            return Err(self.error_locked(&state, DrainErrorKind::AlreadyQuiescing, None));
        }
        if !ready || self.shared.generation.version == MinecraftVersion::Java1_21_11 && !state.ready
        {
            return Err(self.error_locked(&state, DrainErrorKind::NotReady, None));
        }
        state.closed = true;
        state.quiescing = true;
        let cut = Cut {
            admitted_through: state.last_root,
            scope: Scope::RootsClosedAndProducersParked,
        };
        self.shared.notify();
        Ok(Drain {
            shared: self.shared.clone(),
            changed,
            cut,
            result: None,
        })
    }
    pub(crate) fn maintenance(&self) -> Result<WorkGuard, AdmissionError> {
        self.special(false)
    }
    pub(crate) fn stop_work(&self) -> Result<WorkGuard, AdmissionError> {
        self.special(true)
    }
    fn special(&self, stop: bool) -> Result<WorkGuard, AdmissionError> {
        let mut state = self.shared.lock();
        state.healthy()?;
        if stop {
            if let Some(terminal) = state.terminal {
                return Err(AdmissionError::Terminal(terminal));
            }
        }
        if state.sealed || (stop && !state.quiescing) || (state.closed && !state.quiescing) {
            return Err(AdmissionError::Closed);
        }
        let last = state.last_root;
        let id = state.next(last)?;
        state.last_root = id;
        state.roots.insert(id, 1);
        state.guards += 1;
        self.shared.notify();
        Ok(WorkGuard {
            shared: self.shared.clone(),
            root: id,
            retired: false,
            neutral_stop: stop,
        })
    }
    pub(crate) fn seal(&self) -> Result<Drain, DrainError> {
        let changed = self.shared.changed.subscribe();
        let mut state = self.shared.lock();
        if state.terminal.is_some() {
            return Err(self.error_locked(&state, DrainErrorKind::Internal, None));
        }
        state.sealed = true;
        state.closed = true;
        let cut = Cut {
            admitted_through: state.last_root,
            scope: Scope::RootsClosedAndProducersParked,
        };
        self.shared.notify();
        Ok(Drain {
            shared: self.shared.clone(),
            changed,
            cut,
            result: None,
        })
    }
    pub(crate) fn issue_stop(
        &self,
        target: crate::client::StopTarget,
    ) -> Result<crate::client::StopReceipt, crate::client::StopError> {
        let state = self.shared.lock();
        if state.terminal.is_some() {
            return Err(crate::client::StopError::Drain(self.error_locked(
                &state,
                DrainErrorKind::Internal,
                None,
            )));
        }
        Ok(crate::client::StopReceipt::new(
            target,
            state.written_through,
        ))
    }
    pub(crate) fn written_through(&self) -> u64 {
        self.shared.lock().written_through
    }
    pub(crate) fn subscribe(&self) -> watch::Receiver<()> {
        self.shared.changed.subscribe()
    }
}
impl Drain {
    pub(crate) fn admitted_through(&self) -> u64 {
        self.cut.admitted_through
    }
}
impl Receipt {
    pub(crate) fn written_through(&self) -> u64 {
        self.written_through
    }
}
impl DrainFailure {
    pub(crate) fn public(&self) -> DrainError {
        DrainError::new(
            failure_kind(self.terminal.cause),
            PublicKey::new(self.generation.version, self.generation.connection_id),
            None,
            Some(self.cut.admitted_through),
            self.written_through,
            self.terminal.root,
        )
    }
}
fn failure_kind(failure: Failure) -> DrainErrorKind {
    match failure {
        Failure::OwnerLost => DrainErrorKind::OwnerLost,
        Failure::UncertainDispatch => DrainErrorKind::UncertainDispatch,
        Failure::TransactionUnconfirmed => DrainErrorKind::TransactionUnconfirmed,
        Failure::TransactionExpired => DrainErrorKind::TransactionExpired,
        Failure::TransactionOutcomeUnknown => DrainErrorKind::TransactionOutcomeUnknown,
        Failure::RequiredReleaseFailed => DrainErrorKind::RequiredReleaseFailed,
        Failure::Revoked => DrainErrorKind::RevokedBeforeDrain,
        Failure::IdExhausted => DrainErrorKind::ResourceExhausted,
        Failure::Poisoned => DrainErrorKind::Internal,
        Failure::Disconnected => DrainErrorKind::Disconnected,
        Failure::WorkAborted => DrainErrorKind::WorkAborted,
    }
}
