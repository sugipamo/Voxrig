//! Live held-input provenance and once-only, connection-owned selected stops.
#![allow(clippy::result_large_err)] // Preserve the approved public StopError schema.
use super::*;
use crate::client::{
    GameMode, SessionStamp, StopAdmission, StopCompletion, StopError, StopReceipt, StopTarget,
    StopTargetKind,
};
use std::future::Future;

#[derive(Clone, Copy)]
pub(crate) enum StopAction {
    CommonMovement(u64),
    NativeMovement,
    ItemUse,
    Vehicle,
    Dig {
        position: [i32; 3],
        face: crate::BlockFace,
    },
}
type StopReceiver = watch::Receiver<Option<Result<StopReceipt, StopError>>>;

struct Binding {
    target: StopTarget,
    action: StopAction,
    valid: bool,
    neutralized: bool,
    // Err caches a root whose registry submission was never certified.
    result: Option<Result<StopReceiver, StopError>>,
}
#[derive(Default)]
pub(super) struct StopBook {
    next: u64,
    using: Option<(u64, Option<crate::client::Hand>)>,
    hand: Option<(Option<u8>, Option<crate::client::SlotKnowledge>)>,
    context: Option<(SessionStamp, GameMode, Option<crate::client::MountId>)>,
    bindings: Vec<Binding>,
}
impl Ledger {
    fn lock_stops(&self) -> std::sync::MutexGuard<'_, StopBook> {
        self.shared.stops.lock().unwrap_or_else(|poison| {
            self.fail(Failure::Poisoned);
            let mut book = poison.into_inner();
            for binding in &mut book.bindings {
                binding.valid = false;
                binding.neutralized = false;
            }
            book
        })
    }
    pub(crate) fn bind_stop(
        &self,
        mut target: StopTarget,
        action: StopAction,
    ) -> crate::Result<()> {
        let mut book = self.lock_stops();
        if self.shared.stops.is_poisoned() {
            return Err(finite::admission_error(AdmissionError::Closed));
        }
        let Some(id) = book.next.checked_add(1) else {
            self.fail(Failure::IdExhausted);
            return Err(finite::admission_error(AdmissionError::Closed));
        };
        book.next = id;
        target.set_binding(id, self.shared.provenance);
        book.bindings.retain(|b| {
            b.target.kind() != target.kind()
                || target.kind() == StopTargetKind::Movement
                    && b.target.control_session_id().is_some()
                        != target.control_session_id().is_some()
        });
        book.bindings.push(Binding {
            target,
            action,
            valid: true,
            neutralized: false,
            result: None,
        });
        Ok(())
    }
    pub(crate) fn clear_stop(&self, kind: StopTargetKind) {
        let mut book = self.lock_stops();
        for b in &mut book.bindings {
            if b.target.kind() == kind {
                b.valid = false;
                b.neutralized = true;
            }
        }
    }
    pub(crate) fn clear_target(&self, target: StopTarget) {
        let mut book = self.lock_stops();
        if let Some(b) = book.bindings.iter_mut().find(|b| b.target == target) {
            b.valid = false;
            b.neutralized = true;
        }
    }
    pub(crate) fn retire_common_control(&self, id: u64) {
        let mut book = self.lock_stops();
        for b in &mut book.bindings {
            if b.target.kind() == StopTargetKind::Movement
                && b.target.control_session_id() == Some(id)
            {
                b.valid = false;
                b.neutralized = true;
            }
        }
    }
    pub(crate) fn native_target(&self) -> Option<StopTarget> {
        self.lock_stops()
            .bindings
            .iter()
            .find(|b| {
                b.target.kind() == StopTargetKind::Movement
                    && b.target.control_session_id().is_none()
                    && !b.neutralized
            })
            .map(|b| b.target)
    }
    pub(crate) fn observe_stop_context(&self, session: SessionStamp, mode: Option<GameMode>) {
        let mut book = self.lock_stops();
        for b in &mut book.bindings {
            if b.target.session() != session {
                b.valid = false;
                b.neutralized = b.target.kind() == StopTargetKind::Movement;
            } else if Some(b.target.mode()) != mode {
                b.valid = false;
                b.neutralized = false;
            }
        }
    }
    pub(crate) fn stop_targets(&self) -> Result<Vec<StopTarget>, StopError> {
        self.check_terminal().map_err(StopError::Drain)?;
        let book = self.lock_stops();
        self.check_terminal().map_err(StopError::Drain)?;
        if book.bindings.iter().any(|b| {
            b.target.kind() == StopTargetKind::Movement
                && !b.valid
                && !b.neutralized
                && b.result.is_none()
        }) {
            self.fail(Failure::TransactionOutcomeUnknown);
            return Err(StopError::Drain(
                self.error(crate::client::DrainErrorKind::TransactionOutcomeUnknown),
            ));
        }
        Ok(book
            .bindings
            .iter()
            .filter(|b| b.valid)
            .map(|b| b.target)
            .collect())
    }

    pub(crate) fn stop_valid(&self, target: StopTarget) -> bool {
        self.lock_stops()
            .bindings
            .iter()
            .any(|b| b.valid && b.target == target)
    }
    pub(crate) fn enqueue_neutralize<F, Fut>(
        &self,
        target: StopTarget,
        execute: F,
    ) -> Result<StopCompletion, StopError>
    where
        F: FnOnce(StopAction) -> Fut,
        Fut: Future<Output = crate::Result<()>> + Send + 'static,
    {
        let (admission, receiver) = {
            let mut book = self.lock_stops();
            if self.shared.stops.is_poisoned() || self.shared.state.is_poisoned() {
                return Err(StopError::Drain(
                    self.error(crate::client::DrainErrorKind::Internal),
                ));
            }
            let Some(binding) = book.bindings.iter_mut().find(|b| b.target == target) else {
                self.fail(Failure::RequiredReleaseFailed);
                return Err(
                    if !target.belongs_to(self.shared.provenance)
                        || target.session().connection_id != self.generation().connection_id()
                        || target.session().version != self.generation().version()
                    {
                        StopError::WrongTarget { target }
                    } else {
                        StopError::ContextChanged { target }
                    },
                );
            };
            if let Some(result) = &binding.result {
                (StopAdmission::JoinedExisting, result.clone()?)
            } else {
                self.check_terminal().map_err(StopError::Drain)?;
                if !binding.valid {
                    self.fail(Failure::RequiredReleaseFailed);
                    return Err(StopError::ContextChanged { target });
                }
                let work = self.stop_work().map_err(|_| {
                    StopError::Drain(self.error(crate::client::DrainErrorKind::Sealed))
                })?;
                let (sender, receiver) = watch::channel(None);
                binding.result = Some(Ok(receiver.clone()));
                let future = execute(binding.action);
                let ledger = self.clone();
                // The raw owner spans the whole worker. Panic/task abort is OwnerLost;
                // observer cancellation cannot abort this task or its accepted cleanup.
                let submitted = self.tasks().try_spawn(work.run(async move {
                    let result = future.await;
                    let result = match result {
                        Ok(()) => ledger.issue_stop(target),
                        Err(error) => {
                            ledger.fail(match error.kind() {
                                crate::ErrorKind::UncertainDispatch => Failure::UncertainDispatch,
                                crate::ErrorKind::Timeout => Failure::TransactionUnconfirmed,
                                _ => Failure::RequiredReleaseFailed,
                            });
                            if !ledger.stop_valid(target) && error.kind() == crate::ErrorKind::State
                            {
                                Err(StopError::ContextChanged { target })
                            } else {
                                Err(StopError::RequiredReleaseFailed {
                                    target,
                                    drain: ledger.error(
                                        crate::client::DrainErrorKind::RequiredReleaseFailed,
                                    ),
                                })
                            }
                        }
                    };
                    sender.send_replace(Some(result));
                }));
                if submitted.is_err() {
                    // WorkGuard may already have dropped and latched OwnerLost.
                    // No ledger-state guard spans registry submission or rejection.
                    self.fail(Failure::RequiredReleaseFailed);
                    let error = StopError::AdmissionInterrupted {
                        target,
                        drain: self.error(crate::client::DrainErrorKind::RequiredReleaseFailed),
                    };
                    binding.result = Some(Err(error.clone()));
                    return Err(error);
                }
                (StopAdmission::NewlyAdmitted, receiver)
            }
        };
        Ok(StopCompletion::new(self.clone(), admission, receiver))
    }
}
impl Ledger {
    pub(crate) fn stop_context(
        &self,
        session: SessionStamp,
        mode: Option<GameMode>,
        mount: Option<crate::client::MountId>,
    ) {
        self.observe_stop_context(session, mode);
        let mut book = self.lock_stops();
        let mount_changed = book.context.is_some_and(|(_, _, old)| old != mount);
        for b in &mut book.bindings {
            if (b.target.kind() == StopTargetKind::Movement && mount_changed)
                || (b.target.kind() == StopTargetKind::VehicleInput && b.target.mount() != mount)
            {
                b.valid = false;
            }
        }
        book.context = mode.map(|mode| (session, mode, mount));
    }
    pub(crate) fn bind_movement(
        &self,
        session: SessionStamp,
        mode: GameMode,
        id: Option<u64>,
    ) -> crate::Result<()> {
        self.bind_stop(
            StopTarget::new(
                StopTargetKind::Movement,
                session,
                mode,
                0,
                id,
                None,
                None,
                None,
            ),
            id.map_or(StopAction::NativeMovement, StopAction::CommonMovement),
        )
    }
    pub(crate) fn attach_mining(&self, id: crate::client::survival::MiningId) {
        let mut book = self.lock_stops();
        if let Some(binding) = book
            .bindings
            .iter_mut()
            .find(|b| b.target.kind() == StopTargetKind::Digging && b.valid)
        {
            binding.target.set_mining(id);
        }
    }
}
impl WorkGuard {
    pub(super) fn observe_packet(&self, id: i32, payload: &[u8]) -> crate::Result<()> {
        let ledger = Ledger {
            shared: self.shared.clone(),
        };
        let Some((session, mode, mount)) = ledger.lock_stops().context else {
            return Ok(());
        };
        let version = session.version;
        let (use_id, dig_id, vehicle_id, boat_id) = match version {
            MinecraftVersion::Java1_16_1 => (0x2e, 0x1b, 0x1d, 0x16),
            MinecraftVersion::Java1_21_11 => (0x40, 0x28, 0x2a, 0x22),
        };
        let target =
            |kind, hand, mount| StopTarget::new(kind, session, mode, 0, None, hand, mount, None);
        let mut rest = payload;
        if id == use_id {
            let hand = match crate::protocol::get_varint(&mut rest).ok() {
                Some(0) => crate::client::Hand::Main,
                Some(1) => crate::client::Hand::Off,
                _ => return Ok(()),
            };
            ledger.bind_stop(
                target(StopTargetKind::ItemUse, Some(hand), None),
                StopAction::ItemUse,
            )?;
        } else if id == dig_id {
            let Ok(action) = crate::protocol::get_varint(&mut rest) else {
                return Ok(());
            };
            if action == 5 {
                ledger.clear_stop(StopTargetKind::ItemUse);
            } else if action == 0 && rest.len() >= 9 {
                let packed = i64::from_be_bytes(rest[..8].try_into().expect("position length"));
                let position = [
                    (packed >> 38) as i32,
                    (packed << 52 >> 52) as i32,
                    (packed << 26 >> 38) as i32,
                ];
                let face = match rest[8] {
                    0 => crate::BlockFace::Down,
                    1 => crate::BlockFace::Up,
                    2 => crate::BlockFace::North,
                    3 => crate::BlockFace::South,
                    4 => crate::BlockFace::West,
                    5 => crate::BlockFace::East,
                    _ => return Ok(()),
                };
                ledger.bind_stop(
                    target(StopTargetKind::Digging, None, None),
                    StopAction::Dig { position, face },
                )?;
            } else if action == 1 || action == 2 {
                if action == 1 && !self.neutral_stop {
                    ledger.fail(Failure::WorkAborted);
                }
                ledger.clear_stop(StopTargetKind::Digging);
            }
        } else if (id == vehicle_id || id == boat_id) && mount.is_some() {
            let neutral = payload.iter().all(|b| *b == 0);
            if neutral {
                ledger.clear_stop(StopTargetKind::VehicleInput);
            } else {
                ledger.bind_stop(
                    target(StopTargetKind::VehicleInput, None, mount),
                    StopAction::Vehicle,
                )?;
            }
        }
        Ok(())
    }
}
impl Ledger {
    pub(crate) fn bind_native_input(&self, held: bool) -> crate::Result<()> {
        if !held {
            // Local clear precedes the producer's wire posture reconciliation.
            // Retain the native lifetime until that tick or an explicit stop.
            return Ok(());
        }
        let context = self.lock_stops().context;
        if let Some((session, mode, _)) = context {
            self.bind_movement(session, mode, None)
        } else {
            self.fail(Failure::TransactionOutcomeUnknown);
            Ok(())
        }
    }
    pub(crate) fn known_item_stop(&self, hand: crate::client::Hand) -> bool {
        self.lock_stops().bindings.iter().any(|b| {
            b.target.kind() == StopTargetKind::ItemUse
                && b.target.item_use_hand() == Some(hand)
                && (b.valid || b.neutralized)
        })
    }
    pub(crate) fn observe_hand(
        &self,
        using: Option<(u64, Option<crate::client::Hand>)>,
        hand: Option<(Option<u8>, Option<crate::client::SlotKnowledge>)>,
    ) {
        let mut book = self.lock_stops();
        let flags_changed = using != book.using;
        let stack_changed = book.hand.is_some() && book.hand != hand;
        for b in &mut book.bindings {
            if b.target.kind() == StopTargetKind::ItemUse
                && (stack_changed || flags_changed && using.is_some_and(|(_, hand)| hand.is_some()))
            {
                b.neutralized = false;
            }
            if b.target.kind() == StopTargetKind::ItemUse
                && (stack_changed
                    || flags_changed
                        && using.is_some_and(|(_, hand)| hand != b.target.item_use_hand()))
            {
                b.valid = false;
            }
            if b.target.kind() == StopTargetKind::Digging && stack_changed {
                b.valid = false;
            }
        }
        book.using = using;
        book.hand = hand;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> Ledger {
        source_version(MinecraftVersion::Java1_16_1)
    }
    fn source_version(version: MinecraftVersion) -> Ledger {
        let ledger = Ledger::tracking(GenerationKey {
            version,
            connection_id: 1,
        });
        ledger.set_ready(true);
        ledger
    }
    fn item(ledger: &Ledger) -> StopTarget {
        ledger
            .bind_stop(
                StopTarget::new(
                    StopTargetKind::ItemUse,
                    SessionStamp {
                        version: ledger.generation().version(),
                        connection_id: 1,
                        world_generation: 1,
                    },
                    GameMode::Survival,
                    0,
                    None,
                    Some(crate::client::Hand::Main),
                    None,
                    None,
                ),
                StopAction::ItemUse,
            )
            .unwrap();
        ledger.stop_targets().unwrap()[0]
    }
    #[test]
    fn quiesce_concrete_movement_provenance_loss_is_outcome_unknown() {
        let ledger = source();
        let session = SessionStamp {
            version: MinecraftVersion::Java1_16_1,
            connection_id: ledger.generation().connection_id(),
            world_generation: 1,
        };
        ledger
            .bind_movement(session, GameMode::Survival, Some(1))
            .unwrap();
        ledger.observe_stop_context(session, Some(GameMode::Creative));
        assert!(
            matches!(ledger.stop_targets(), Err(StopError::Drain(error)) if error.kind() == crate::client::DrainErrorKind::TransactionOutcomeUnknown)
        );
        assert_eq!(
            ledger
                .cut(false)
                .inspect()
                .unwrap()
                .unwrap_err()
                .public()
                .kind(),
            crate::client::DrainErrorKind::TransactionOutcomeUnknown
        );
    }
    #[test]
    fn quiesce_normal_item_release_retains_knowledge_until_server_flags_catch_up() {
        let ledger = source();
        ledger.observe_hand(Some((9, Some(crate::client::Hand::Main))), None);
        let target = item(&ledger);
        ledger.clear_stop(StopTargetKind::ItemUse);
        ledger.observe_hand(Some((9, Some(crate::client::Hand::Main))), None);
        assert!(!ledger.stop_valid(target));
        assert!(ledger.known_item_stop(crate::client::Hand::Main));
        assert!(ledger.stop_targets().unwrap().is_empty());
        ledger.observe_hand(Some((10, Some(crate::client::Hand::Main))), None);
        assert!(!ledger.known_item_stop(crate::client::Hand::Main));
    }
    #[tokio::test]
    async fn quiesce_stop_timeout_and_unknown_delivery_stay_distinct_and_sticky() {
        for (kind, expected) in [
            (
                crate::ErrorKind::Timeout,
                crate::client::DrainErrorKind::TransactionUnconfirmed,
            ),
            (
                crate::ErrorKind::UncertainDispatch,
                crate::client::DrainErrorKind::UncertainDispatch,
            ),
        ] {
            let ledger = source();
            let target = item(&ledger);
            ledger
                .begin(ledger.generation(), true)
                .unwrap()
                .wait()
                .await
                .unwrap();
            let error = ledger
                .enqueue_neutralize(target, move |_| async move {
                    Err(crate::Error::new(kind, anyhow::anyhow!("cleanup failed")))
                })
                .unwrap()
                .wait()
                .await
                .unwrap_err();
            assert!(
                matches!(error, StopError::RequiredReleaseFailed { drain, .. } if drain.kind() == expected)
            );
            assert_eq!(ledger.seal().err().unwrap().kind(), expected);
            assert_eq!(
                ledger.cut(false).wait().await.unwrap_err().public().kind(),
                expected
            );
        }
    }
    #[tokio::test]
    async fn quiesce_stop_provenance_never_crosses_same_key_ledger_instances() {
        let first = source();
        let second = source();
        let target = item(&first);
        let second_target = item(&second);
        assert_ne!(target, second_target);
        assert!(!format!("{target:?}").contains("provenance"));
        assert!(
            serde_json::to_value(target)
                .unwrap()
                .get("provenance")
                .is_none()
        );
        second
            .begin(second.generation(), true)
            .unwrap()
            .wait()
            .await
            .unwrap();
        assert!(matches!(
            second.enqueue_neutralize(target, |_| async {
                panic!("foreign stop must never execute");
                #[allow(unreachable_code)]
                Ok(())
            }),
            Err(StopError::WrongTarget { .. })
        ));
        assert_eq!(
            second.seal().err().unwrap().kind(),
            crate::client::DrainErrorKind::RequiredReleaseFailed
        );
        assert!(first.cut(false).wait().await.is_ok());
    }
    #[tokio::test]
    async fn quiesce_stop_worker_owner_loss_fails_without_hiding_accepted_debt() {
        let ledger = source();
        let target = item(&ledger);
        ledger
            .begin(ledger.generation(), true)
            .unwrap()
            .wait()
            .await
            .unwrap();
        let error = ledger
            .enqueue_neutralize(target, |_| async {
                panic!("worker failure");
                #[allow(unreachable_code)]
                Ok(())
            })
            .unwrap()
            .wait()
            .await
            .unwrap_err();
        assert!(
            matches!(error, StopError::Drain(e) if e.kind() == crate::client::DrainErrorKind::OwnerLost)
        );
        assert_eq!(
            ledger.seal().err().unwrap().kind(),
            crate::client::DrainErrorKind::OwnerLost
        );
    }
    #[tokio::test]
    async fn quiesce_stop_revocation_before_receipt_issue_cannot_become_success() {
        let ledger = source();
        let target = item(&ledger);
        ledger
            .begin(ledger.generation(), true)
            .unwrap()
            .wait()
            .await
            .unwrap();
        let (started, start) = tokio::sync::oneshot::channel();
        let (release, wait) = tokio::sync::oneshot::channel();
        let caller = tokio::spawn({
            let ledger = ledger.clone();
            async move {
                ledger
                    .enqueue_neutralize(target, |_| async move {
                        started.send(()).unwrap();
                        wait.await.unwrap();
                        Ok(())
                    })
                    .unwrap()
                    .wait()
                    .await
            }
        });
        start.await.unwrap();
        ledger.fail(Failure::Revoked);
        release.send(()).unwrap();
        assert!(
            matches!(caller.await.unwrap(), Err(StopError::Drain(e)) if e.kind() == crate::client::DrainErrorKind::RevokedBeforeDrain)
        );
    }
    const VERSIONS: [MinecraftVersion; 2] =
        [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11];

    #[tokio::test]
    async fn enqueue_registers_debt_immediately_drop_is_passive_and_duplicates_join() {
        for version in VERSIONS {
            let ledger = source_version(version);
            let target = item(&ledger);
            ledger
                .begin(ledger.generation(), true)
                .unwrap()
                .wait()
                .await
                .unwrap();
            let before = ledger.test_stop_debt().0;
            let (release, blocked) = tokio::sync::oneshot::channel();
            let completion = ledger
                .enqueue_neutralize(target, |_| async move {
                    blocked.await.unwrap();
                    Ok(())
                })
                .unwrap();
            assert_eq!(completion.admission(), StopAdmission::NewlyAdmitted);
            assert_eq!(ledger.test_stop_debt(), (before + 1, 1));
            let joined = ledger
                .enqueue_neutralize(target, |_| async {
                    panic!("duplicate must not execute");
                    #[allow(unreachable_code)]
                    Ok(())
                })
                .unwrap();
            assert_eq!(joined.admission(), StopAdmission::JoinedExisting);
            assert_eq!(ledger.test_stop_debt(), (before + 1, 1));
            drop(completion);
            let mut final_drain = ledger.seal().unwrap();
            assert!(final_drain.inspect().is_none());
            // No completion future has been polled. Dropping the first observer
            // neither releases debt nor prevents the worker from completing.
            release.send(()).unwrap();
            let receipt = joined.wait().await.unwrap();
            final_drain.wait().await.unwrap();
            let cached = ledger
                .enqueue_neutralize(target, |_| async { panic!() })
                .unwrap();
            assert_eq!(cached.admission(), StopAdmission::JoinedExisting);
            assert_eq!(cached.wait().await.unwrap(), receipt);
            assert_eq!(ledger.test_stop_debt(), (before + 1, 0));
            ledger.fail(Failure::Revoked);
            let historical = ledger
                .enqueue_neutralize(target, |_| async { panic!() })
                .unwrap();
            assert_eq!(historical.wait().await.unwrap(), receipt);
        }
    }

    #[tokio::test]
    async fn enqueue_registry_close_between_root_and_submission_is_explicit_and_cached() {
        for version in VERSIONS {
            let ledger = source_version(version);
            let target = item(&ledger);
            ledger
                .begin(ledger.generation(), true)
                .unwrap()
                .wait()
                .await
                .unwrap();
            let before = ledger.test_stop_debt().0;
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let worker_ledger = ledger.clone();
            let worker_barrier = barrier.clone();
            let enqueue = std::thread::spawn(move || {
                worker_ledger.enqueue_neutralize(target, |_| {
                    // Test-only factory barrier at the exact root/submission gap.
                    worker_barrier.wait();
                    worker_barrier.wait();
                    async { panic!("closed registry must not run this worker") }
                })
            });
            barrier.wait();
            assert_eq!(ledger.test_stop_debt(), (before + 1, 1));
            ledger.tasks().start(async { Ok(()) });
            barrier.wait();
            let error = enqueue.join().unwrap().err().unwrap();
            assert!(
                matches!(&error, StopError::AdmissionInterrupted { drain, .. }
                if drain.kind() == crate::client::DrainErrorKind::OwnerLost)
            );
            assert_eq!(ledger.test_stop_debt(), (before + 1, 0));
            let cached = ledger
                .enqueue_neutralize(target, |_| async { panic!() })
                .err()
                .unwrap();
            assert_eq!(cached, error); // No Ok without a certified registry submission.
            assert_eq!(
                ledger.seal().err().unwrap().kind(),
                crate::client::DrainErrorKind::OwnerLost
            );
            ledger.tasks().wait().await.unwrap();
        }
    }

    #[tokio::test]
    async fn enqueue_revoke_before_and_after_root_have_ordered_thread_barriers() {
        for version in VERSIONS {
            for revoke_first in [true, false] {
                let ledger = source_version(version);
                let target = item(&ledger);
                ledger
                    .begin(ledger.generation(), true)
                    .unwrap()
                    .wait()
                    .await
                    .unwrap();
                let before = ledger.test_stop_debt().0;
                let barrier = Arc::new(std::sync::Barrier::new(2));
                let entrant = ledger.clone();
                let gate = barrier.clone();
                let (release, blocked) = tokio::sync::oneshot::channel();
                let enqueue = std::thread::spawn(move || {
                    if revoke_first {
                        gate.wait();
                    }
                    entrant.enqueue_neutralize(target, |_| {
                        if !revoke_first {
                            gate.wait();
                            gate.wait();
                        }
                        async move {
                            blocked.await.unwrap();
                            Ok(())
                        }
                    })
                });
                if !revoke_first {
                    barrier.wait();
                    assert_eq!(ledger.test_stop_debt(), (before + 1, 1));
                }
                ledger.fail(Failure::Revoked);
                barrier.wait();
                let result = enqueue.join().unwrap();
                if revoke_first {
                    assert!(matches!(result, Err(StopError::Drain(e))
                        if e.kind() == crate::client::DrainErrorKind::RevokedBeforeDrain));
                    assert_eq!(ledger.test_stop_debt(), (before, 0));
                } else {
                    let completion = result.unwrap();
                    assert_eq!(completion.admission(), StopAdmission::NewlyAdmitted);
                    release.send(()).unwrap();
                    assert!(matches!(completion.wait().await, Err(StopError::Drain(e))
                        if e.kind() == crate::client::DrainErrorKind::RevokedBeforeDrain));
                }
            }
        }
    }

    #[tokio::test]
    async fn enqueue_poison_fails_closed_even_for_cached_success() {
        for version in VERSIONS {
            for poison_book in [true, false] {
                let ledger = source_version(version);
                let target = item(&ledger);
                ledger
                    .begin(ledger.generation(), true)
                    .unwrap()
                    .wait()
                    .await
                    .unwrap();
                ledger
                    .enqueue_neutralize(target, |_| async { Ok(()) })
                    .unwrap()
                    .wait()
                    .await
                    .unwrap();
                let before = ledger.test_stop_debt().0;
                if poison_book {
                    ledger.test_poison_stops();
                } else {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let _state = ledger.shared.state.lock().unwrap();
                        panic!("injected ledger poison");
                    }));
                }
                assert!(
                    matches!(ledger.enqueue_neutralize(target, |_| async { panic!() }),
                    Err(StopError::Drain(e)) if e.kind() == crate::client::DrainErrorKind::Internal)
                );
                assert!(matches!(ledger.stop_targets(), Err(StopError::Drain(e))
                    if e.kind() == crate::client::DrainErrorKind::Internal));
                assert_eq!(ledger.test_stop_debt().0, before);
                assert_eq!(
                    ledger.seal().err().unwrap().kind(),
                    crate::client::DrainErrorKind::Internal
                );
            }
        }
    }

    #[test]
    fn enqueue_missing_runtime_reports_post_admission_failure_without_panic() {
        let ledger = source();
        let target = item(&ledger);
        ledger.begin(ledger.generation(), true).unwrap();
        assert!(matches!(
            ledger.enqueue_neutralize(target, |_| async { Ok(()) }),
            Err(StopError::AdmissionInterrupted { .. })
        ));
        assert_eq!(ledger.test_stop_debt(), (1, 0));
    }

    #[tokio::test]
    async fn enqueue_stale_foreign_and_sealed_refusals_add_no_root() {
        for version in VERSIONS {
            for reason in 0..3 {
                let ledger = source_version(version);
                let target = item(&ledger);
                ledger
                    .begin(ledger.generation(), true)
                    .unwrap()
                    .wait()
                    .await
                    .unwrap();
                let target = match reason {
                    0 => {
                        ledger.clear_target(target);
                        target
                    }
                    1 => item(&source_version(version)),
                    _ => {
                        ledger.seal().unwrap();
                        target
                    }
                };
                let before = ledger.test_stop_debt();
                let error = ledger
                    .enqueue_neutralize(target, |_| async { panic!() })
                    .err()
                    .unwrap();
                match reason {
                    0 => assert!(matches!(error, StopError::ContextChanged { .. })),
                    1 => assert!(matches!(error, StopError::WrongTarget { .. })),
                    _ => assert!(
                        matches!(error, StopError::Drain(e) if e.kind() == crate::client::DrainErrorKind::Sealed)
                    ),
                }
                assert_eq!(ledger.test_stop_debt(), before);
                if reason < 2 {
                    assert_eq!(
                        ledger.seal().err().unwrap().kind(),
                        crate::client::DrainErrorKind::RequiredReleaseFailed
                    );
                }
            }
        }
    }
}
