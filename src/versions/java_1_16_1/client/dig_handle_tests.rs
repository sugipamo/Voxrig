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
