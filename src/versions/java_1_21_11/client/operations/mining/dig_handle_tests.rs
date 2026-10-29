// Included in the TCP mining fixture module; all race decisions use explicit
// owner phases, held locks, bounded writer progress or paused time.
use crate::client::{
    DigCancelOutcome, DigOperation, DigOwnerExit, DigPhase, DigResult, DigWriteEvidence,
};

async fn dig_phase(op: &DigOperation, phase: DigPhase) {
    for _ in 0..20_000 {
        if op.snapshot().phase() == phase {
            return;
        }
        assert!(
            op.snapshot().finished().is_none(),
            "expected {phase:?}: {:?}",
            op.snapshot()
        );
        tokio::task::yield_now().await;
    }
    panic!("expected {phase:?}: {:?}", op.snapshot());
}
async fn dig_statuses(control: &crate::client::ledger::testing::Control) -> Vec<u8> {
    let bytes = control.bytes();
    let mut input = bytes.as_slice();
    let mut statuses = Vec::new();
    while !input.is_empty() {
        let (id, payload) = read_packet(&mut input, None).await.unwrap();
        assert_eq!(id, ids::play_serverbound::BLOCK_DIG);
        statuses.push(payload[0]);
    }
    statuses
}

#[tokio::test]
async fn dig_handle_begin_is_sync_exclusive_no_runtime_and_setup_error_sends_nothing() {
    let f = Fixture::new().await;
    let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
    let outside = client.clone();
    let error = std::thread::spawn(move || {
        outside
            .survival()
            .begin_dig(TARGET, crate::BlockFace::Up)
            .unwrap_err()
    })
    .join()
    .unwrap();
    assert_eq!(error.kind(), ErrorKind::State);
    let state = f.session.state.lock().await;
    let op = client
        .survival()
        .begin_dig(TARGET, crate::BlockFace::Up)
        .unwrap();
    assert!(
        client
            .survival()
            .begin_dig(TARGET, crate::BlockFace::Up)
            .is_err()
    );
    assert!(
        client
            .survival()
            .dig(TARGET, crate::BlockFace::Up)
            .await
            .is_err()
    );
    op.request_cancel();
    drop(state);
    assert_eq!(
        *op.finished().await.cancel_outcome(),
        DigCancelOutcome::StartSuppressed
    );
    let _closed = f.session.ledger.cut(true);
    assert!(
        client
            .survival()
            .begin_dig(TARGET, crate::BlockFace::Up)
            .is_err()
    );
    f.stop().await;
}

#[tokio::test]
async fn dig_handle_rejected_registration_releases_slot_and_never_sends_start() {
    let f = Fixture::new().await;
    let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
    let control = crate::client::ledger::testing::Control::default();
    control.release(usize::MAX, true);
    f.session
        .writer
        .lock()
        .await
        .stream
        .test_replace(control.writer());
    let writer = f.session.writer.clone();
    f.session
        .ledger
        .tasks()
        .start(async move { writer.lock().await.stream.shutdown().await });
    assert!(
        client
            .survival()
            .begin_dig(TARGET, crate::BlockFace::Up)
            .is_err()
    );
    assert!(!f.session.operational_dig.active());
    assert!(control.bytes().is_empty());
    f.stop().await;
}

#[tokio::test]
async fn dig_handle_cancel_before_start_under_writer_lock_suppresses_all_bytes() {
    let f = Fixture::new().await;
    let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
    let control = crate::client::ledger::testing::Control::default();
    control.release(usize::MAX, true);
    let mut writer = f.session.writer.lock().await;
    writer.stream.test_replace(control.writer());
    let op = client
        .survival()
        .begin_dig(TARGET, crate::BlockFace::Up)
        .unwrap();
    dig_phase(&op, DigPhase::Starting).await;
    assert_eq!(*op.snapshot().dispatch().start(), DigWriteEvidence::NotSent);
    drop(op.cancel()); // eager even without polling
    drop(writer);
    let done = op.finished().await;
    assert_eq!(*done.cancel_outcome(), DigCancelOutcome::StartSuppressed);
    assert!(matches!(done.result(), DigResult::Cancelled));
    assert_eq!(done.dispatch().start(), &DigWriteEvidence::NotSent);
    assert!(control.bytes().is_empty());
    f.stop().await;
}

#[tokio::test]
async fn dig_handle_start_committed_then_cancel_finishes_start_and_one_abort() {
    let mut f = Fixture::new().await;
    f.change(TARGET, &native("dirt")).await;
    let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
    let control = crate::client::ledger::testing::Control::default();
    control.release(usize::MAX, false);
    f.session
        .writer
        .lock()
        .await
        .stream
        .test_replace(control.writer());
    let op = client
        .survival()
        .begin_dig(TARGET, crate::BlockFace::Up)
        .unwrap();
    control.until(|c| c.progress().1 > 0).await;
    assert_eq!(*op.snapshot().dispatch().start(), DigWriteEvidence::Pending);
    for _ in 0..8 {
        op.clone().request_cancel();
    }
    control.release(usize::MAX, true);
    let (a, b) = tokio::join!(op.finished(), op.cancel());
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!(*a.cancel_outcome(), DigCancelOutcome::Aborted);
    assert!(matches!(a.result(), DigResult::Cancelled));
    assert_eq!(*a.dispatch().stop(), DigWriteEvidence::NotSent);
    assert_eq!(dig_statuses(&control).await, [0, 1]);
    f.stop().await;
}

#[tokio::test]
async fn dig_handle_stop_commit_boundary_including_faster_accepted_estimate() {
    use crate::client::adapter::DigOps;
    for faster in [false, true] {
        for cancel_before in [true, false] {
            let mut f = Fixture::new().await;
            f.change(TARGET, &native("dirt")).await;
            if faster {
                f.change([0, 0, 0], &native("air")).await;
            }
            let initial = f.api.dig_snapshot(TARGET).await.unwrap();
            let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
            tokio::time::pause();
            let op = client
                .survival()
                .begin_dig(TARGET, crate::BlockFace::Up)
                .unwrap();
            dig_phase(&op, DigPhase::Waiting).await;
            if faster {
                f.change([0, 0, 0], &native("stone")).await;
                assert!(
                    f.api.dig_snapshot(TARGET).await.unwrap().estimate.ticks
                        < initial.estimate.ticks
                );
            }
            let control = crate::client::ledger::testing::Control::default();
            control.release(usize::MAX, cancel_before);
            let mut writer = f.session.writer.lock().await;
            writer.stream.test_replace(control.writer());
            tokio::time::advance(Duration::from_millis((initial.estimate.ticks + 1) * 50 + 1))
                .await;
            dig_phase(&op, DigPhase::Stopping).await;
            if cancel_before {
                assert_eq!(*op.snapshot().dispatch().stop(), DigWriteEvidence::NotSent);
                op.request_cancel();
                drop(writer);
            } else {
                drop(writer);
                control.until(|c| c.progress().1 > 0).await;
                assert_eq!(*op.snapshot().dispatch().stop(), DigWriteEvidence::Pending);
                op.request_cancel();
                control.release(usize::MAX, true);
            }
            if !cancel_before {
                dig_phase(&op, DigPhase::Observing).await;
                f.change(TARGET, &native("air")).await;
                tokio::time::advance(Duration::from_millis(21)).await;
            }
            let done = op.finished().await;
            assert_eq!(
                *done.cancel_outcome(),
                if cancel_before {
                    DigCancelOutcome::Aborted
                } else {
                    DigCancelOutcome::AlreadyStopped
                }
            );
            assert_eq!(
                dig_statuses(&control).await,
                if cancel_before { vec![1] } else { vec![2] }
            );
            if !cancel_before {
                assert!(
                    matches!(done.result(), DigResult::Finished(record) if record.estimate == initial.estimate && record.removed)
                );
            }
            tokio::time::resume();
            f.stop().await;
        }
    }
}

#[tokio::test]
async fn dig_handle_stop_refusal_and_cancel_share_one_cleanup_and_preserve_cause() {
    use crate::client::adapter::DigOps;
    let mut f = Fixture::new().await;
    f.change(TARGET, &native("dirt")).await;
    let initial = f.api.dig_snapshot(TARGET).await.unwrap();
    let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
    tokio::time::pause();
    let op = client
        .survival()
        .begin_dig(TARGET, crate::BlockFace::Up)
        .unwrap();
    dig_phase(&op, DigPhase::Waiting).await;
    f.session
        .state
        .lock()
        .await
        .operations
        .selected_hotbar
        .as_mut()
        .unwrap()
        .slot = 1;
    let control = crate::client::ledger::testing::Control::default();
    control.release(usize::MAX, false);
    f.session
        .writer
        .lock()
        .await
        .stream
        .test_replace(control.writer());
    tokio::time::advance(Duration::from_millis((initial.estimate.ticks + 1) * 50 + 1)).await;
    control.until(|c| c.progress().1 > 0).await;
    assert_eq!(op.snapshot().phase(), DigPhase::CleaningUp);
    for _ in 0..10 {
        op.request_cancel();
    }
    control.release(usize::MAX, true);
    let done = op.finished().await;
    assert_eq!(*done.cancel_outcome(), DigCancelOutcome::Aborted);
    assert!(
        matches!(done.result(), DigResult::Failed { kind: ErrorKind::State, message } if message.contains("selected hotbar"))
    );
    assert_eq!(dig_statuses(&control).await, [1]);
    tokio::time::resume();
    f.stop().await;
}

#[tokio::test]
async fn dig_handle_partial_and_failed_flush_never_confirm_abort_or_stop() {
    use crate::client::adapter::DigOps;
    for abort in [false, true] {
        for flush in [false, true] {
            let mut f = Fixture::new().await;
            f.change(TARGET, &native("dirt")).await;
            let initial = f.api.dig_snapshot(TARGET).await.unwrap();
            let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
            tokio::time::pause();
            let op = client
                .survival()
                .begin_dig(TARGET, crate::BlockFace::Up)
                .unwrap();
            dig_phase(&op, DigPhase::Waiting).await;
            let control = crate::client::ledger::testing::Control::default();
            control.release(usize::MAX, true);
            control.fail(if flush { None } else { Some(1) }, flush);
            f.session
                .writer
                .lock()
                .await
                .stream
                .test_replace(control.writer());
            if abort {
                op.request_cancel();
            } else {
                tokio::time::advance(Duration::from_millis((initial.estimate.ticks + 1) * 50 + 1))
                    .await;
            }
            let done = op.finished().await;
            assert!(matches!(
                done.cancel_outcome(),
                DigCancelOutcome::Unknown { .. } | DigCancelOutcome::Disconnected
            ));
            assert_eq!(
                *if abort {
                    done.dispatch().abort()
                } else {
                    done.dispatch().stop()
                },
                DigWriteEvidence::Unknown
            );
            assert!(!matches!(
                done.cancel_outcome(),
                DigCancelOutcome::Aborted | DigCancelOutcome::AlreadyStopped
            ));
            if !abort {
                assert_eq!(*done.dispatch().abort(), DigWriteEvidence::NotSent);
            }
            tokio::time::resume();
            f.stop().await;
        }
    }
}

#[tokio::test]
async fn dig_handle_drop_keeps_running_instant_late_cancel_and_id_isolation() {
    for instant in [false, true] {
        let mut f = Fixture::new().await;
        f.change(
            TARGET,
            &native(if instant { "short_grass" } else { "dirt" }),
        )
        .await;
        let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
        tokio::time::pause();
        let op = client
            .survival()
            .begin_dig(TARGET, crate::BlockFace::Up)
            .unwrap();
        let retained = op.clone();
        drop(op);
        if !instant {
            dig_phase(&retained, DigPhase::Waiting).await;
            tokio::time::advance(Duration::from_secs(1)).await;
        }
        dig_phase(&retained, DigPhase::Observing).await;
        f.change(TARGET, &native("air")).await;
        tokio::time::advance(Duration::from_millis(21)).await;
        let done = retained.finished().await;
        assert_eq!(
            *done.cancel_outcome(),
            if instant {
                DigCancelOutcome::InstantBreak
            } else {
                DigCancelOutcome::AlreadyStopped
            }
        );
        assert!(matches!(done.result(), DigResult::Finished(_)));
        assert!(Arc::ptr_eq(&done, &retained.cancel().await));
        {
            let mut state = f.session.state.lock().await;
            state.loading.generation += 1;
            state.loading.attempt.as_mut().unwrap().generation = state.loading.generation;
            let generation = state.loading.generation;
            let receive_sequence = state.sequence;
            let position = state.position.unwrap();
            let rotation = state.rotation;
            state.motion.receive(ReceivedPose {
                generation,
                receive_sequence,
                position,
                rotation,
                velocity: Some([0.0; 3]),
            });
        }
        f.change(TARGET, &native("dirt")).await;
        let next = client
            .survival()
            .begin_dig(TARGET, crate::BlockFace::Up)
            .unwrap();
        assert_ne!(retained.id(), next.id());
        retained.request_cancel();
        dig_phase(&next, DigPhase::Waiting).await;
        assert!(!next.snapshot().cancel_requested());
        next.request_cancel();
        assert_eq!(
            *next.finished().await.cancel_outcome(),
            DigCancelOutcome::Aborted
        );
        tokio::time::resume();
        f.stop().await;
    }
}

#[tokio::test]
async fn dig_handle_context_change_has_no_abort_and_revoke_still_joins() {
    for ending in ["context", "revoke", "disconnect"] {
        let f = Fixture::new().await;
        let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
        let op = client
            .survival()
            .begin_dig(TARGET, crate::BlockFace::Up)
            .unwrap();
        dig_phase(&op, DigPhase::Waiting).await;
        if ending == "revoke" {
            let _ = client.revoke_connection();
        } else if ending == "disconnect" {
            client.disconnect().await.unwrap();
        } else {
            f.session.state.lock().await.loading.generation += 1;
            op.request_cancel();
        }
        let done = op.finished().await;
        assert_eq!(*done.dispatch().abort(), DigWriteEvidence::NotSent);
        if ending != "context" {
            assert!(matches!(
                done.cancel_outcome(),
                DigCancelOutcome::Disconnected | DigCancelOutcome::Unknown { .. }
            ));
            assert_eq!(
                *done.owner_exit(),
                DigOwnerExit::Cancelled {
                    connection_teardown: true
                }
            );
            client.wait_shutdown().await.unwrap();
        } else {
            assert_eq!(*done.cancel_outcome(), DigCancelOutcome::ContextChanged);
        }
        f.stop().await;
    }
}

async fn plain_modern(f: &Fixture) -> (tokio::task::JoinHandle<crate::Result<crate::client::DigRecord>>, DigOperation) {
    let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
    let task = tokio::spawn(async move { client.survival().dig(TARGET, crate::BlockFace::Up).await });
    loop {
        if let Some(op) = f.session.operational_dig.testing_plain.lock().unwrap().clone() { return (task, op); }
        assert!(!task.is_finished());
        tokio::task::yield_now().await;
    }
}
async fn modern_boundary(op: &DigOperation, phase: DigPhase, stage: crate::client::DigStage) {
    for _ in 0..20_000 {
        let s = op.snapshot();
        if s.phase() == phase && s.stage() == stage { return; }
        assert!(s.finished().is_none(), "expected {phase:?}/{stage:?}: {s:?}");
        tokio::task::yield_now().await;
    }
    panic!("expected {phase:?}/{stage:?}: {:?}", op.snapshot());
}
#[tokio::test]
async fn plain_dig_modern_state_and_writer_lock_stalls_are_bounded() {
    use crate::client::{DigStage as S, OperationOutcome as O, OperationUnknownReason as U};
    for state_stall in [true, false] {
        let f = Fixture::new().await;
        let state = if state_stall { Some(f.session.state.lock().await) } else { None };
        let writer = if !state_stall { Some(f.session.writer.lock().await) } else { None };
        tokio::time::pause();
        let (task, op) = plain_modern(&f).await;
        modern_boundary(&op, if state_stall { DigPhase::Preparing } else { DigPhase::Starting },
            if state_stall { S::Gate } else { S::Writer }).await;
        tokio::time::advance(crate::client::operational_dig::PLAIN_DIG_BUDGET).await;
        let error = task.await.unwrap().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::OperationUnconfirmed);
        let s = error.dig_snapshot().unwrap();
        assert!(s.deadline_exceeded());
        assert_eq!(*s.dispatch().start(), DigWriteEvidence::NotSent);
        assert_eq!(s.id(), op.id());
        assert_eq!(op.finished().await.operation_outcome(), O::Unknown(U::Deadline));
        drop(state); drop(writer);
        tokio::time::resume();
        f.stop().await;
    }
}
#[tokio::test]
async fn plain_dig_modern_partial_start_and_flush_deadlines_never_become_success() {
    use crate::client::{DigStage as S, OperationOutcome as O, OperationUnknownReason as U};
    for bytes in [Some(0), Some(1), Some(3), None] {
        let f = Fixture::new().await;
        let control = crate::client::ledger::testing::Control::default();
        control.release(bytes.unwrap_or(usize::MAX), false);
        f.session.writer.lock().await.stream.test_replace(control.writer());
        tokio::time::pause();
        let (task, op) = plain_modern(&f).await;
        control.until(|c| if let Some(bytes) = bytes { c.progress().0 > 0 && c.bytes().len() == bytes } else { c.progress().1 > 0 }).await;
        // Native transport commitment is separate from holding the writer lock.
        modern_boundary(&op, DigPhase::Starting, S::Write).await;
        tokio::time::advance(crate::client::operational_dig::PLAIN_DIG_BUDGET).await;
        let error = task.await.unwrap().unwrap_err();
        assert!(error.dig_snapshot().unwrap().deadline_exceeded());
        let done = op.finished().await;
        assert_eq!(done.operation_outcome(), O::Unknown(U::Deadline));
        assert_eq!(*done.dispatch().start(), DigWriteEvidence::Unknown);
        assert_eq!(*done.dispatch().stop(), DigWriteEvidence::NotSent);
        assert_eq!(*done.dispatch().abort(), DigWriteEvidence::NotSent);
        let before = control.bytes();
        control.release(usize::MAX, true);
        assert_eq!(control.bytes(), before);
        assert_eq!(*op.finished().await.dispatch().start(), DigWriteEvidence::Unknown);
        tokio::time::resume();
        f.stop().await;
    }
}
#[tokio::test]
async fn plain_dig_modern_long_estimate_deadline_retains_start_and_gate() {
    use crate::client::{DigStage as S, OperationOutcome as O, OperationUnknownReason as U};
    let mut f = Fixture::new().await;
    f.change(TARGET, &native("obsidian")).await;
    tokio::time::pause();
    let (task, op) = plain_modern(&f).await;
    modern_boundary(&op, DigPhase::Waiting, S::Wait).await;
    assert!(op.snapshot().estimate().unwrap().ticks > 600);
    tokio::time::advance(crate::client::operational_dig::PLAIN_DIG_BUDGET).await;
    let error = task.await.unwrap().unwrap_err();
    let s = error.dig_snapshot().unwrap();
    assert_eq!(s.stage(), S::Wait);
    assert!(matches!(s.dispatch().start(), DigWriteEvidence::Written { .. }));
    assert_eq!(*s.dispatch().stop(), DigWriteEvidence::NotSent);
    assert_eq!(*s.dispatch().abort(), DigWriteEvidence::NotSent);
    assert_eq!(op.finished().await.operation_outcome(), O::Unknown(U::Deadline));
    let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
    assert_eq!(client.survival().dig(TARGET, crate::BlockFace::Up).await.unwrap_err().kind(), ErrorKind::OperationUnconfirmed);
    tokio::time::resume();
    f.stop().await;
}
