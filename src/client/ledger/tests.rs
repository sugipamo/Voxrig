use super::*;
use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};

fn ledger() -> Ledger {
    Ledger::new(GenerationKey {
        version: MinecraftVersion::Java1_16_1,
        connection_id: 7,
    })
}

fn cause(drain: &mut Drain) -> Failure {
    drain.inspect().unwrap().unwrap_err().terminal.cause
}

#[test]
fn admission_and_cut_have_both_atomic_orderings() {
    // Explicit channel handshakes enumerate both gate orderings without sleeps.
    for admission_first in [false, true] {
        let ledger = Arc::new(ledger());
        let (send, recv) = std::sync::mpsc::channel();
        let other = ledger.clone();
        let thread = std::thread::spawn(move || {
            if admission_first {
                let work = other.admit().unwrap();
                send.send(Some(work)).unwrap();
            } else {
                send.send(None).unwrap();
            }
        });
        let work = recv.recv().unwrap();
        let mut cut = ledger.cut(true);
        if let Some(work) = work {
            assert_eq!(cut.cut.admitted_through, work.root);
            assert!(cut.inspect().is_none());
            work.finish(Outcome::NoWrite);
        } else {
            assert_eq!(cut.cut.admitted_through, 0);
        }
        assert!(matches!(ledger.admit(), Err(AdmissionError::Closed)));
        assert!(cut.inspect().unwrap().is_ok());
        thread.join().unwrap();
    }
}

#[test]
fn cut_includes_late_children_and_zero_gap_transfer() {
    let ledger = ledger();
    let parent = ledger.admit().unwrap();
    let mut cut = ledger.cut(true);
    let child = parent.child().unwrap();
    parent.finish(Outcome::Complete);
    assert!(cut.inspect().is_none());
    let retained = child.transfer();
    assert!(cut.inspect().is_none());
    let receiver = retained.child().unwrap();
    retained.finish(Outcome::Complete);
    assert!(cut.inspect().is_none());
    receiver.finish(Outcome::Complete);
    assert!(cut.inspect().unwrap().is_ok());
}

#[test]
fn snapshot_excludes_new_roots_but_not_descendants() {
    let ledger = ledger();
    let first = ledger.admit().unwrap();
    let mut cut = ledger.cut(false);
    let later = ledger.admit().unwrap();
    first.finish(Outcome::Complete);
    assert_eq!(cut.inspect().unwrap().unwrap().cut.admitted_through, 1);
    assert_eq!(ledger.shared.lock().roots.len(), 1);
    later.finish(Outcome::Complete);
}

#[test]
fn owner_drop_and_panic_are_sticky_failures() {
    for panic in [false, true] {
        let ledger = ledger();
        let work = ledger.admit().unwrap();
        let mut cut = ledger.cut(false);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _work = work;
            if panic {
                panic!("owner panicked");
            }
        }));
        assert_eq!(result.is_err(), panic);
        assert_eq!(cause(&mut cut), Failure::OwnerLost);
        assert_eq!(cause(&mut ledger.cut(false)), Failure::OwnerLost);
        assert!(ledger.shared.lock().roots.is_empty());
    }
}

#[test]
fn explicit_prewrite_outcomes_are_retained_without_failure() {
    let ledger = ledger();
    for outcome in [
        Outcome::Complete,
        Outcome::NoWrite,
        Outcome::Rejected,
        Outcome::CancelledBeforeWrite,
        Outcome::SuppressedBeforeWrite,
    ] {
        ledger.admit().unwrap().finish(outcome);
    }
    assert_eq!(
        ledger.cut(false).inspect().unwrap().unwrap().outcomes,
        Outcomes {
            complete: 1,
            no_write: 1,
            rejected: 1,
            cancelled: 1,
            suppressed: 1,
            failed: 0
        }
    );
}

#[test]
fn all_uncertain_outcomes_fail_even_after_all_debt_retires() {
    for failure in [
        Failure::UncertainDispatch,
        Failure::TransactionUnconfirmed,
        Failure::RequiredReleaseFailed,
        Failure::Revoked,
    ] {
        let ledger = ledger();
        let work = ledger.admit().unwrap();
        let mut cut = ledger.cut(false);
        work.finish(Outcome::Failed(failure));
        ledger.fail(Failure::OwnerLost);
        assert_eq!(cause(&mut cut), failure);
        for _ in 0..100 {
            assert_eq!(cause(&mut ledger.cut(false)), failure);
        }
        assert!(matches!(ledger.admit(), Err(AdmissionError::Terminal(_))));
    }
}

#[test]
fn issued_receipt_is_historical_but_pending_older_cut_fails() {
    let ledger = ledger();
    let mut issued = ledger.cut(false);
    let saved = issued.inspect().unwrap().unwrap();
    let mut pending = ledger.cut(false);
    ledger.fail(Failure::Revoked);
    assert_eq!(issued.inspect(), Some(Ok(saved)));
    assert_eq!(cause(&mut pending), Failure::Revoked);
}

#[test]
fn write_permit_keeps_root_live_and_publishes_contiguous_prefix() {
    let ledger = ledger();
    let parent = ledger.admit().unwrap();
    let write = parent.begin_write().unwrap();
    assert_eq!(write.ordinal, 1);
    assert!(matches!(
        parent.begin_write(),
        Err(AdmissionError::WriterBusy)
    ));
    let mut cut = ledger.cut(false);
    parent.finish(Outcome::Complete);
    assert!(cut.inspect().is_none());
    write.commit();
    assert_eq!(cut.inspect().unwrap().unwrap().written_through, 1);
    let next = ledger.admit().unwrap();
    next.begin_write().unwrap().commit();
    next.finish(Outcome::Complete);
    assert_eq!(
        ledger
            .cut(false)
            .inspect()
            .unwrap()
            .unwrap()
            .written_through,
        2
    );
}

#[test]
fn partial_write_never_advances_prefix_or_disappears() {
    let ledger = ledger();
    let work = ledger.admit().unwrap();
    work.begin_write().unwrap().commit();
    let partial = work.begin_write().unwrap();
    drop(partial);
    assert!(matches!(
        work.begin_write(),
        Err(AdmissionError::Terminal(_))
    ));
    work.finish(Outcome::Complete);
    let error = ledger.cut(false).inspect().unwrap().unwrap_err();
    assert_eq!(error.written_through, 1);
    assert_eq!(error.terminal.cause, Failure::UncertainDispatch);
    assert_eq!(cause(&mut ledger.cut(false)), Failure::UncertainDispatch);
}

#[test]
fn producer_park_is_separate_from_finite_debt_and_network_stop() {
    let ledger = ledger();
    let producer = ledger.register_producer().unwrap();
    let work = ledger.admit().unwrap();
    assert!(!producer.park_requested());
    let mut cut = ledger.cut(true);
    assert!(producer.park_requested());
    assert!(matches!(
        ledger.register_producer(),
        Err(AdmissionError::Closed)
    ));
    producer.acknowledge_park();
    assert!(cut.inspect().is_none());
    work.finish(Outcome::SuppressedBeforeWrite);
    assert_eq!(cut.inspect().unwrap().unwrap().written_through, 0);
}

#[test]
fn idle_does_not_substitute_for_park_ack_and_producer_loss_fails() {
    let ledger = ledger();
    let producer = ledger.register_producer().unwrap();
    assert!(ledger.cut(false).inspect().unwrap().is_ok());
    let mut cut = ledger.cut(true);
    assert!(cut.inspect().is_none());
    drop(producer);
    assert_eq!(cause(&mut cut), Failure::OwnerLost);
}

#[test]
fn identifiers_exhaust_without_wrapping_and_notify_waiters() {
    for allocator in 0..3 {
        let ledger = ledger();
        let work = ledger.admit().unwrap();
        let mut cut = ledger.cut(false);
        {
            let mut state = ledger.shared.lock();
            match allocator {
                0 => state.last_root = u64::MAX,
                1 => state.last_producer = u64::MAX,
                _ => state.last_write = u64::MAX,
            }
        }
        let failed = match allocator {
            0 => matches!(ledger.admit(), Err(AdmissionError::Terminal(_))),
            1 => matches!(ledger.register_producer(), Err(AdmissionError::Terminal(_))),
            _ => matches!(work.begin_write(), Err(AdmissionError::Terminal(_))),
        };
        assert!(failed);
        assert!(cut.changed.has_changed().unwrap());
        work.finish(Outcome::NoWrite);
        assert_eq!(cause(&mut cut), Failure::IdExhausted);
    }
}

#[test]
fn final_id_is_usable_then_exhaustion_is_terminal() {
    let ledger = ledger();
    ledger.shared.lock().last_root = u64::MAX - 1;
    let work = ledger.admit().unwrap();
    assert_eq!(work.root, u64::MAX);
    work.finish(Outcome::NoWrite);
    assert!(ledger.cut(false).inspect().unwrap().is_ok());
    assert!(matches!(ledger.admit(), Err(AdmissionError::Terminal(_))));
}

#[test]
fn live_bounds_and_retirement_keep_memory_bounded() {
    let ledger = ledger();
    let mut roots = Vec::new();
    for _ in 0..MAX_ROOTS {
        roots.push(ledger.admit().unwrap());
    }
    assert!(matches!(ledger.admit(), Err(AdmissionError::Capacity)));
    for root in roots {
        root.finish(Outcome::Complete);
    }
    let parent = ledger.admit().unwrap();
    let children: Vec<_> = (1..MAX_GUARDS).map(|_| parent.child().unwrap()).collect();
    assert!(matches!(parent.child(), Err(AdmissionError::Capacity)));
    assert!(matches!(
        parent.begin_write(),
        Err(AdmissionError::Capacity)
    ));
    let parent = parent.transfer();
    for child in children {
        child.finish(Outcome::NoWrite);
    }
    parent.finish(Outcome::Complete);
    let producers: Vec<_> = (0..MAX_PRODUCERS)
        .map(|_| ledger.register_producer().unwrap())
        .collect();
    assert!(matches!(
        ledger.register_producer(),
        Err(AdmissionError::Capacity)
    ));
    for producer in producers {
        producer.acknowledge_park();
    }
    for _ in 0..10_000 {
        ledger.admit().unwrap().finish(Outcome::NoWrite);
        assert!(ledger.cut(false).inspect().unwrap().is_ok());
    }
    let state = ledger.shared.lock();
    assert!(state.roots.is_empty());
    assert!(state.producers.is_empty());
    assert_eq!(state.guards, 0);
    assert!(state.terminal.is_none());
}

#[test]
fn diagnostic_saturation_cannot_erase_failure() {
    let ledger = ledger();
    ledger.shared.lock().outcomes.failed = u64::MAX;
    ledger
        .admit()
        .unwrap()
        .finish(Outcome::Failed(Failure::TransactionUnconfirmed));
    assert_eq!(ledger.shared.lock().outcomes.failed, u64::MAX);
    assert_eq!(
        cause(&mut ledger.cut(false)),
        Failure::TransactionUnconfirmed
    );
}

#[test]
fn poisoned_gate_fails_closed_without_panicking_during_guard_drop() {
    let ledger = ledger();
    let work = ledger.admit().unwrap();
    let shared = ledger.shared.clone();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _gate = shared.state.lock().unwrap();
            panic!("poison gate");
        }))
        .is_err()
    );
    drop(work);
    assert_eq!(cause(&mut ledger.cut(false)), Failure::Poisoned);
}

struct Counter(AtomicUsize);
impl Wake for Counter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
fn counter() -> (Arc<Counter>, Waker) {
    let counter = Arc::new(Counter(AtomicUsize::new(0)));
    let waker = Waker::from(counter.clone());
    (counter, waker)
}

#[test]
fn multiple_waiters_each_receive_a_wakeup_and_the_same_boundary() {
    let ledger = ledger();
    let work = ledger.admit().unwrap();
    let mut first = ledger.cut(false);
    let mut second = ledger.cut(false);
    let (count1, waker1) = counter();
    let (count2, waker2) = counter();
    let mut future1 = pin!(first.wait());
    let mut future2 = pin!(second.wait());
    assert!(
        future1
            .as_mut()
            .poll(&mut Context::from_waker(&waker1))
            .is_pending()
    );
    assert!(
        future2
            .as_mut()
            .poll(&mut Context::from_waker(&waker2))
            .is_pending()
    );
    work.finish(Outcome::Complete);
    assert!(count1.0.load(Ordering::SeqCst) > 0);
    assert!(count2.0.load(Ordering::SeqCst) > 0);
    let Poll::Ready(Ok(a)) = future1.as_mut().poll(&mut Context::from_waker(&waker1)) else {
        panic!("first waiter pending")
    };
    let Poll::Ready(Ok(b)) = future2.as_mut().poll(&mut Context::from_waker(&waker2)) else {
        panic!("second waiter pending")
    };
    assert_eq!(a, b);
}

#[test]
fn completion_between_inspection_and_change_registration_is_not_lost() {
    let ledger = ledger();
    let work = ledger.admit().unwrap();
    let mut cut = ledger.cut(false); // Already subscribed.
    assert!(cut.inspect().is_none());
    work.finish(Outcome::Complete); // Before changed() is even constructed/polled.
    let (_, waker) = counter();
    {
        let mut changed = pin!(cut.changed.changed());
        assert!(
            changed
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_ready()
        );
    }
    assert!(cut.inspect().unwrap().is_ok());
}

#[test]
fn waiter_cancellation_does_not_retire_work_or_reopen_gate() {
    let ledger = ledger();
    let work = ledger.admit().unwrap();
    let mut cut = ledger.cut(true);
    let (_, waker) = counter();
    {
        let mut future = pin!(cut.wait());
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
    }
    drop(cut);
    assert!(matches!(ledger.admit(), Err(AdmissionError::Closed)));
    let mut replacement = ledger.cut(true);
    assert!(replacement.inspect().is_none());
    work.finish(Outcome::NoWrite);
    let mut future = pin!(replacement.wait());
    assert!(matches!(
        future.as_mut().poll(&mut Context::from_waker(&waker)),
        Poll::Ready(Ok(_))
    ));
}

#[test]
fn generation_is_transport_bound_and_ledger_instances_never_mix() {
    let first = ledger();
    let second = Ledger::new(GenerationKey {
        version: MinecraftVersion::Java1_21_11,
        connection_id: 7,
    });
    let work = first.admit().unwrap();
    let receipt = second.cut(false).inspect().unwrap().unwrap();
    assert_ne!(receipt.generation, first.shared.generation);
    assert!(first.cut(false).inspect().is_none());
    work.finish(Outcome::Complete);
}

#[test]
fn producer_and_write_final_ids_are_usable_without_wrapping() {
    let ledger = ledger();
    ledger.shared.lock().last_producer = u64::MAX - 1;
    let producer = ledger.register_producer().unwrap();
    assert_eq!(producer.id, u64::MAX);
    producer.acknowledge_park();
    assert!(matches!(
        ledger.register_producer(),
        Err(AdmissionError::Terminal(_))
    ));
    assert_eq!(ledger.shared.lock().last_producer, u64::MAX);

    let ledger = Ledger::new(GenerationKey {
        version: MinecraftVersion::Java1_16_1,
        connection_id: 8,
    });
    let work = ledger.admit().unwrap();
    {
        let mut state = ledger.shared.lock();
        state.last_write = u64::MAX - 1;
        state.written_through = u64::MAX - 1;
    }
    let write = work.begin_write().unwrap();
    assert_eq!(write.ordinal, u64::MAX);
    write.commit();
    assert_eq!(ledger.shared.lock().written_through, u64::MAX);
    assert!(matches!(
        work.begin_write(),
        Err(AdmissionError::Terminal(_))
    ));
    work.finish(Outcome::Complete);
    assert_eq!(ledger.shared.lock().last_write, u64::MAX);
    assert_eq!(cause(&mut ledger.cut(false)), Failure::IdExhausted);
}

#[test]
fn terminal_failure_wakes_every_registered_waiter() {
    let ledger = ledger();
    let work = ledger.admit().unwrap();
    let mut first = ledger.cut(false);
    let mut second = ledger.cut(false);
    let (count1, waker1) = counter();
    let (count2, waker2) = counter();
    let mut future1 = pin!(first.wait());
    let mut future2 = pin!(second.wait());
    assert!(
        future1
            .as_mut()
            .poll(&mut Context::from_waker(&waker1))
            .is_pending()
    );
    assert!(
        future2
            .as_mut()
            .poll(&mut Context::from_waker(&waker2))
            .is_pending()
    );
    ledger.fail(Failure::Revoked);
    assert!(count1.0.load(Ordering::SeqCst) > 0);
    assert!(count2.0.load(Ordering::SeqCst) > 0);
    for (mut future, waker) in [(future1.as_mut(), &waker1), (future2.as_mut(), &waker2)] {
        let Poll::Ready(Err(error)) = future.as_mut().poll(&mut Context::from_waker(waker)) else {
            panic!("failure did not wake waiter")
        };
        assert_eq!(error.terminal.cause, Failure::Revoked);
    }
    work.finish(Outcome::NoWrite);
}

#[test]
fn guards_and_waiters_survive_session_ledger_owner_drop() {
    let ledger = ledger();
    let work = ledger.admit().unwrap();
    let mut cut = ledger.cut(true);
    drop(ledger);
    assert!(cut.inspect().is_none());
    work.finish(Outcome::Complete);
    let receipt = cut.inspect().unwrap().unwrap();
    let (_, waker) = counter();
    for _ in 0..2 {
        let mut future = pin!(cut.wait());
        assert_eq!(
            future.as_mut().poll(&mut Context::from_waker(&waker)),
            Poll::Ready(Ok(receipt))
        );
    }
}
