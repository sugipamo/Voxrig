use super::*;
use crate::client::{
    DigCancelOutcome, DigOperation, DigOwnerExit, DigPhase, DigResult, DigWriteEvidence,
};
const TARGET: [i32; 3] = [9, 65, 8];
struct Fixture {
    bot: Bot,
    packets: tokio::sync::mpsc::UnboundedReceiver<(i32, Vec<u8>)>,
    _release: tokio::sync::oneshot::Sender<()>,
    server: tokio::task::JoinHandle<()>,
}
impl Fixture {
    async fn new() -> Self {
        let (bot, packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        {
            let mut r = bot.common_receipts.lock().await;
            r.inventory.window_id = Some(0);
            r.inventory.slots = (0..46)
                .map(|_| {
                    Some(crate::client::received(
                        crate::client::SlotKnowledge::Empty,
                        1,
                    ))
                })
                .collect();
            r.selected_hotbar = Some(crate::client::received(0, 1));
        }
        bot.world.lock().await.set_block_for_test(
            BlockPos {
                x: TARGET[0],
                y: TARGET[1],
                z: TARGET[2],
            },
            1,
        );
        Self {
            bot,
            packets,
            _release: release,
            server,
        }
    }
    fn client(&self) -> crate::Client {
        crate::Client::from_java_1_16_1(self.bot.clone())
    }
    async fn stop(self) {
        let _ = self.bot.revoke_connection();
        let _ = self.client().wait_shutdown().await;
        self.server.await.unwrap();
    }
    async fn target(&self, name: &str) {
        let id = crate::versions::java_1_16_1::state_id(&crate::NativeBlockState {
            name: format!("minecraft:{name}"),
            properties: Default::default(),
        })
        .unwrap();
        self.bot.world.lock().await.set_block_for_test(
            BlockPos {
                x: TARGET[0],
                y: TARGET[1],
                z: TARGET[2],
            },
            id,
        );
    }
}
async fn phase(op: &DigOperation, expected: DigPhase) {
    for _ in 0..20_000 {
        if op.snapshot().phase() == expected {
            return;
        }
        assert!(
            op.snapshot().finished().is_none(),
            "expected {expected:?}: {:?}",
            op.snapshot()
        );
        tokio::task::yield_now().await;
    }
    panic!("expected {expected:?}: {:?}", op.snapshot());
}
async fn statuses(control: &crate::client::ledger::testing::Control) -> Vec<u8> {
    let bytes = control.bytes();
    let mut input = bytes.as_slice();
    let mut statuses = Vec::new();
    while !input.is_empty() {
        let (id, payload) = read_packet(&mut input, None).await.unwrap();
        assert_eq!(id, 0x1b);
        statuses.push(payload[0]);
    }
    statuses
}

#[tokio::test]
async fn dig_handle_legacy_sync_exclusive_no_runtime_and_cancel_before_start() {
    let f = Fixture::new().await;
    let client = f.client();
    let outside = client.clone();
    assert_eq!(
        std::thread::spawn(move || outside
            .survival()
            .begin_dig(TARGET, crate::BlockFace::Up)
            .unwrap_err())
        .join()
        .unwrap()
        .kind(),
        crate::ErrorKind::State
    );
    let gate = f.bot.coherent_state_gate.lock().await;
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
    drop(op.cancel());
    drop(gate);
    let done = op.finished().await;
    assert_eq!(*done.cancel_outcome(), DigCancelOutcome::StartSuppressed);
    assert!(matches!(done.result(), DigResult::Cancelled));
    assert!(f.packets.is_empty());
    let _closed = f.bot.connection.ledger().cut(true);
    assert!(
        client
            .survival()
            .begin_dig(TARGET, crate::BlockFace::Up)
            .is_err()
    );
    f.stop().await;
}

#[tokio::test]
async fn dig_handle_legacy_start_committed_then_concurrent_cancels_one_abort() {
    let f = Fixture::new().await;
    let client = f.client();
    let control = crate::client::ledger::testing::Control::default();
    control.release(usize::MAX, false);
    f.bot
        .writer
        .lock()
        .await
        .inner
        .test_replace(control.writer());
    let op = client
        .survival()
        .begin_dig(TARGET, crate::BlockFace::Up)
        .unwrap();
    control.until(|c| c.progress().1 > 0).await;
    assert_eq!(*op.snapshot().dispatch().start(), DigWriteEvidence::Pending);
    for _ in 0..10 {
        op.clone().request_cancel();
    }
    control.release(usize::MAX, true);
    let (a, b) = tokio::join!(op.cancel(), op.finished());
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!(*a.cancel_outcome(), DigCancelOutcome::Aborted);
    assert!(matches!(a.result(), DigResult::Cancelled));
    assert_eq!(statuses(&control).await, [0, 1]);
    assert!(Arc::ptr_eq(&a, &op.cancel().await));
    f.stop().await;
}

#[tokio::test]
async fn dig_handle_legacy_stop_commit_boundary_and_faster_accepted_estimate() {
    for faster in [false, true] {
        for cancel_before in [true, false] {
            let f = Fixture::new().await;
            let client = f.client();
            if faster {
                f.bot.player.lock().await.on_ground = false;
            }
            let initial = f.bot.dig_snapshot(TARGET).await.unwrap();
            tokio::time::pause();
            let op = client
                .survival()
                .begin_dig(TARGET, crate::BlockFace::Up)
                .unwrap();
            phase(&op, DigPhase::Waiting).await;
            if faster {
                f.bot.player.lock().await.on_ground = true;
                assert!(
                    f.bot.dig_snapshot(TARGET).await.unwrap().estimate.ticks
                        < initial.estimate.ticks
                );
            }
            let control = crate::client::ledger::testing::Control::default();
            control.release(usize::MAX, cancel_before);
            f.bot
                .writer
                .lock()
                .await
                .inner
                .test_replace(control.writer());
            // Before: hold coherent validation. After: allow enqueue, hold flush.
            let mut gate = if cancel_before {
                Some(f.bot.coherent_state_gate.lock().await)
            } else {
                None
            };
            tokio::time::advance(Duration::from_millis((initial.estimate.ticks + 1) * 50 + 1))
                .await;
            phase(&op, DigPhase::Stopping).await;
            if cancel_before {
                assert_eq!(*op.snapshot().dispatch().stop(), DigWriteEvidence::NotSent);
                op.request_cancel();
                drop(gate.take());
            } else {
                control.until(|c| c.progress().1 > 0).await;
                assert_eq!(*op.snapshot().dispatch().stop(), DigWriteEvidence::Pending);
                op.request_cancel();
                control.release(usize::MAX, true);
                phase(&op, DigPhase::Observing).await;
                f.target("air").await;
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
                statuses(&control).await,
                if cancel_before { vec![1] } else { vec![2] }
            );
            if !cancel_before {
                assert!(
                    matches!(done.result(),DigResult::Finished(record) if record.estimate==initial.estimate && record.removed)
                );
            }
            drop(gate);
            tokio::time::resume();
            f.stop().await;
        }
    }
}

#[tokio::test]
async fn dig_handle_legacy_stop_refusal_and_cancel_share_cleanup() {
    let f = Fixture::new().await;
    let client = f.client();
    let initial = f.bot.dig_snapshot(TARGET).await.unwrap();
    tokio::time::pause();
    let op = client
        .survival()
        .begin_dig(TARGET, crate::BlockFace::Up)
        .unwrap();
    phase(&op, DigPhase::Waiting).await;
    f.bot
        .common_receipts
        .lock()
        .await
        .selected_hotbar
        .as_mut()
        .unwrap()
        .value = 1;
    let control = crate::client::ledger::testing::Control::default();
    control.release(usize::MAX, false);
    f.bot
        .writer
        .lock()
        .await
        .inner
        .test_replace(control.writer());
    tokio::time::advance(Duration::from_millis((initial.estimate.ticks + 1) * 50 + 1)).await;
    control.until(|c| c.progress().1 > 0).await;
    assert_eq!(op.snapshot().phase(), DigPhase::CleaningUp);
    op.request_cancel();
    control.release(usize::MAX, true);
    let done = op.finished().await;
    assert_eq!(*done.cancel_outcome(), DigCancelOutcome::Aborted);
    assert!(
        matches!(done.result(),DigResult::Failed { kind:crate::ErrorKind::State,message } if message.contains("selected hotbar"))
    );
    assert_eq!(statuses(&control).await, [1]);
    tokio::time::resume();
    f.stop().await;
}

#[tokio::test]
async fn dig_handle_legacy_delivery_unknown_abort_and_stop_never_confirmed() {
    for abort in [false, true] {
        for flush in [false, true] {
            let f = Fixture::new().await;
            let client = f.client();
            let initial = f.bot.dig_snapshot(TARGET).await.unwrap();
            tokio::time::pause();
            let op = client
                .survival()
                .begin_dig(TARGET, crate::BlockFace::Up)
                .unwrap();
            phase(&op, DigPhase::Waiting).await;
            let control = crate::client::ledger::testing::Control::default();
            control.release(usize::MAX, true);
            control.fail(if flush { None } else { Some(1) }, flush);
            f.bot
                .writer
                .lock()
                .await
                .inner
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
            if !abort {
                assert_eq!(*done.dispatch().abort(), DigWriteEvidence::NotSent);
            }
            tokio::time::resume();
            f.stop().await;
        }
    }
}

#[tokio::test]
async fn dig_handle_legacy_drop_instant_and_old_id_do_not_cancel_successor() {
    for instant in [false, true] {
        let f = Fixture::new().await;
        if instant {
            f.target("grass").await;
        }
        let client = f.client();
        let initial = f.bot.dig_snapshot(TARGET).await.unwrap();
        assert_eq!(initial.estimate.ticks == 0, instant);
        tokio::time::pause();
        let op = client
            .survival()
            .begin_dig(TARGET, crate::BlockFace::Up)
            .unwrap();
        let retained = op.clone();
        drop(op);
        if !instant {
            phase(&retained, DigPhase::Waiting).await;
            tokio::time::advance(Duration::from_millis((initial.estimate.ticks + 1) * 50 + 1))
                .await;
        }
        phase(&retained, DigPhase::Observing).await;
        f.target("air").await;
        tokio::time::advance(Duration::from_millis(21)).await;
        let done = retained.finished().await;
        assert!(matches!(done.result(), DigResult::Finished(_)));
        assert_eq!(
            *done.cancel_outcome(),
            if instant {
                DigCancelOutcome::InstantBreak
            } else {
                DigCancelOutcome::AlreadyStopped
            }
        );
        assert!(Arc::ptr_eq(&done, &retained.cancel().await));
        f.bot.common_receipts.lock().await.generation += 1;
        f.target("stone").await;
        let next = client
            .survival()
            .begin_dig(TARGET, crate::BlockFace::Up)
            .unwrap();
        assert_ne!(retained.id(), next.id());
        retained.request_cancel();
        phase(&next, DigPhase::Waiting).await;
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
async fn dig_handle_legacy_context_change_and_revoke_are_joined_without_replacement_abort() {
    for ending in ["context", "revoke", "disconnect"] {
        let f = Fixture::new().await;
        let client = f.client();
        let op = client
            .survival()
            .begin_dig(TARGET, crate::BlockFace::Up)
            .unwrap();
        phase(&op, DigPhase::Waiting).await;
        if ending == "revoke" {
            let _ = client.revoke_connection();
        } else if ending == "disconnect" {
            client.disconnect().await.unwrap();
        } else {
            f.bot.common_receipts.lock().await.generation += 1;
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
        } else {
            assert_eq!(*done.cancel_outcome(), DigCancelOutcome::ContextChanged);
        }
        f.stop().await;
    }
}

// The compatibility call uses the same handle state; the test-only observer
// lets these fixtures synchronize on actual boundaries, never elapsed sleeps.
async fn plain(
    f: &Fixture,
) -> (
    tokio::task::JoinHandle<crate::Result<crate::client::DigRecord>>,
    DigOperation,
) {
    let client = f.client();
    let task =
        tokio::spawn(async move { client.survival().dig(TARGET, crate::BlockFace::Up).await });
    loop {
        if let Some(op) = f.bot.operational_dig.testing_plain.lock().unwrap().clone() {
            return (task, op);
        }
        assert!(!task.is_finished());
        tokio::task::yield_now().await;
    }
}
async fn boundary(op: &DigOperation, phase: DigPhase, stage: crate::client::DigStage) {
    for _ in 0..20_000 {
        let s = op.snapshot();
        if s.phase() == phase && s.stage() == stage {
            return;
        }
        assert!(
            s.finished().is_none(),
            "expected {phase:?}/{stage:?}: {s:?}"
        );
        tokio::task::yield_now().await;
    }
    panic!("expected {phase:?}/{stage:?}: {:?}", op.snapshot());
}
fn deadline(error: &crate::Error) -> &crate::client::DigSnapshot {
    assert_eq!(error.kind(), crate::ErrorKind::OperationUnconfirmed);
    let s = error.dig_snapshot().expect("structured original snapshot");
    assert!(s.deadline_exceeded());
    s
}
#[tokio::test]
async fn plain_dig_legacy_snapshot_gate_and_state_stalls_are_bounded() {
    use crate::client::{DigStage as S, OperationOutcome, OperationUnknownReason};
    for gate_stall in [true, false] {
        let f = Fixture::new().await;
        let gate = if gate_stall {
            Some(f.bot.coherent_state_gate.lock().await)
        } else {
            None
        };
        let player = if !gate_stall {
            Some(f.bot.player.lock().await)
        } else {
            None
        };
        tokio::time::pause();
        let (task, op) = plain(&f).await;
        let stage = if gate_stall { S::Gate } else { S::Snapshot };
        boundary(&op, DigPhase::Preparing, stage).await;
        tokio::time::advance(crate::client::operational_dig::PLAIN_DIG_BUDGET).await;
        let error = task.await.unwrap().unwrap_err();
        let s = deadline(&error);
        assert_eq!(s.id(), op.id());
        assert_eq!(s.stage(), stage);
        assert_eq!(*s.dispatch().start(), DigWriteEvidence::NotSent);
        assert_eq!(
            op.finished().await.operation_outcome(),
            OperationOutcome::Unknown(OperationUnknownReason::Deadline)
        );
        drop(gate);
        drop(player);
        assert!(f.packets.is_empty());
        tokio::time::resume();
        f.stop().await;
    }
}
#[tokio::test]
async fn plain_dig_legacy_partial_start_deadline_keeps_evidence_gate_and_no_replay() {
    use crate::client::DigStage as S;
    for bytes in [0, 1, 3] {
        let f = Fixture::new().await;
        let control = crate::client::ledger::testing::Control::default();
        control.release(bytes, false);
        f.bot
            .writer
            .lock()
            .await
            .inner
            .test_replace(control.writer());
        tokio::time::pause();
        let (task, op) = plain(&f).await;
        control
            .until(|c| c.progress().0 > 0 && c.bytes().len() == bytes)
            .await;
        boundary(&op, DigPhase::Starting, S::WriterAcquired).await;
        tokio::time::advance(crate::client::operational_dig::PLAIN_DIG_BUDGET).await;
        let error = task.await.unwrap().unwrap_err();
        let s = deadline(&error);
        assert!(matches!(
            s.dispatch().start(),
            DigWriteEvidence::Pending | DigWriteEvidence::Unknown
        ));
        let done = op.finished().await;
        assert_eq!(*done.dispatch().start(), DigWriteEvidence::Unknown);
        assert_eq!(*done.dispatch().stop(), DigWriteEvidence::NotSent);
        assert_eq!(*done.dispatch().abort(), DigWriteEvidence::NotSent);
        assert_eq!(
            f.client().survival().dig_cleanup_record().unwrap().status,
            crate::client::DigCleanupStatus::Failed
        );
        // A queued legacy actor can finish its original write late. Its sealed
        // deadline completion stays Unknown; neither STOP nor ABORT is added.
        control.release(usize::MAX, true);
        control.until(|c| c.progress().2 == 1).await;
        assert_eq!(statuses(&control).await, [0]);
        let before = control.bytes();
        assert_eq!(
            f.client()
                .survival()
                .dig(TARGET, crate::BlockFace::Up)
                .await
                .unwrap_err()
                .kind(),
            crate::ErrorKind::OperationUnconfirmed
        );
        assert_eq!(control.bytes(), before);
        assert_eq!(
            *op.finished().await.dispatch().start(),
            DigWriteEvidence::Unknown
        );
        tokio::time::resume();
        f.stop().await;
    }
}
#[tokio::test]
async fn plain_dig_legacy_stop_gate_capture_and_abort_stalls_have_distinct_stages() {
    use crate::client::{DigStage as S, adapter::DigOps};
    for case in ["stop_gate", "capture", "abort"] {
        let f = Fixture::new().await;
        f.target("dirt").await;
        let ticks = f.bot.dig_snapshot(TARGET).await.unwrap().estimate.ticks;
        tokio::time::pause();
        let (task, op) = plain(&f).await;
        phase(&op, DigPhase::Waiting).await;
        let gate = if case == "stop_gate" {
            Some(f.bot.coherent_state_gate.lock().await)
        } else {
            None
        };
        let control = crate::client::ledger::testing::Control::default();
        if case != "stop_gate" {
            control.release(usize::MAX, false);
            f.bot
                .writer
                .lock()
                .await
                .inner
                .test_replace(control.writer());
        }
        if case == "abort" {
            op.request_cancel();
        } else {
            tokio::time::advance(Duration::from_millis((ticks + 1) * 50 + 1)).await;
        }
        let player = if case == "capture" {
            control.until(|c| c.progress().1 > 0).await;
            let player = f.bot.player.lock().await;
            control.release(usize::MAX, true);
            boundary(&op, DigPhase::Observing, S::Capture).await;
            Some(player)
        } else {
            boundary(
                &op,
                if case == "abort" {
                    DigPhase::CleaningUp
                } else {
                    DigPhase::Stopping
                },
                if case == "abort" {
                    S::WriterAcquired
                } else {
                    S::Gate
                },
            )
            .await;
            None
        };
        tokio::time::advance(crate::client::operational_dig::PLAIN_DIG_BUDGET).await;
        let error = task.await.unwrap().unwrap_err();
        let s = deadline(&error);
        assert!(matches!(
            s.dispatch().start(),
            DigWriteEvidence::Written { .. }
        ));
        assert!(s.estimate().is_some());
        assert!(s.stages().iter().any(|r| r.stage == S::Wait));
        assert_eq!(
            s.phase(),
            match case {
                "capture" => DigPhase::Observing,
                "abort" => DigPhase::CleaningUp,
                _ => DigPhase::Stopping,
            }
        );
        assert!(op.finished().await.snapshot().deadline_exceeded());
        drop(player);
        drop(gate);
        control.release(usize::MAX, true);
        tokio::time::resume();
        f.stop().await;
    }
}
#[tokio::test]
async fn plain_dig_legacy_entity_packet_flood_and_target_competition_finish_with_stages() {
    use crate::client::DigStage as S;
    let f = Fixture::new().await;
    f.target("dirt").await;
    tokio::time::pause();
    let (task, op) = plain(&f).await;
    phase(&op, DigPhase::Waiting).await;
    for id in 100..108 {
        let mut spawn = vec![id as u8];
        spawn.extend([id as u8; 16]);
        for coordinate in [8.5f64, 65., 8.5] {
            spawn.extend(coordinate.to_be_bytes());
        }
        spawn.extend([0, 0]);
        f.bot.apply_packet(0x04, spawn).await.unwrap();
    }
    // Actual decoded native relative-move packets, eight IDs and 1024 updates.
    // No packet backlog is taken as proof of the production incident's cause.
    for n in 0..1024 {
        let mut packet = vec![100 + (n % 8) as u8];
        packet.extend([0, 1, 0, 0, 0, 1, 1]);
        f.bot.apply_packet(0x28, packet).await.unwrap();
    }
    f.target("air").await;
    tokio::time::advance(Duration::from_secs(1)).await;
    let record = task.await.unwrap().unwrap();
    assert!(record.removed);
    let s = record.snapshot.unwrap();
    assert_eq!(s.id(), op.id());
    assert!(!s.deadline_exceeded());
    assert!(s.stages().iter().any(|r| r.stage == S::GateAcquired));
    assert!(s.stages().iter().any(|r| r.stage == S::Written));
    assert!(s.stages().iter().any(|r| r.stage == S::Waited));
    assert_eq!(*s.dispatch().stop(), DigWriteEvidence::NotSent);
    tokio::time::resume();
    f.stop().await;
}
