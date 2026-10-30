use super::*;
use crate::client::{
    self as api,
    guarded_use::{self as guarded, *},
};
use guarded::tests::Fixture as _;

impl guarded::tests::Fixture for Fixture {
    fn client(&self) -> crate::Client {
        crate::Client::from_java_1_21_11(self.api.bot.clone())
    }
    async fn mode(&mut self, mode: api::GameMode) {
        self.session.state.lock().await.operations.game_mode = Some(mode);
    }
    async fn slot(&mut self, slot: i16, item: Option<api::registry::RegistryId>) {
        let mut payload = vec![0, 2];
        payload.extend(slot.to_be_bytes());
        if let Some(item) = item {
            payload.push(1);
            put_varint(&mut payload, item.value());
            payload.extend([0, 0]);
        } else {
            payload.push(0);
        }
        self.receive(ids::play_clientbound::SET_SLOT, &payload)
            .await;
    }
    async fn packet(&mut self) -> (i32, Vec<u8>) {
        timeout(Duration::from_secs(1), read_packet(&mut self.peer, None))
            .await
            .unwrap()
            .unwrap()
    }
    async fn silent(&mut self) {
        assert!(
            timeout(Duration::from_millis(15), read_packet(&mut self.peer, None))
                .await
                .is_err()
        );
    }
}
#[tokio::test]
async fn guarded_use_contract() {
    let mut f = Fixture::new().await;
    Box::pin(guarded::tests::contract(&mut f)).await;
    f.stop().await;
}
#[tokio::test]
async fn guarded_use_fresh_cut_after_gate_and_readiness() {
    for block in [false, true] {
        let mut f = Fixture::new().await;
        let client = f.client();
        let ops = client.survival();
        let item = client.registry().item("minecraft:dirt").unwrap().id;
        f.session.state.lock().await.ready = false;
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let target = BlockUseTarget {
            position: [1, 0, 0],
            face: crate::BlockFace::Up,
            cursor: [0.5; 3],
        };
        let check: guarded::Check = Box::new(move |cut| {
            assert_eq!(cut.player.position.as_ref().unwrap().value[0], 0.75);
            assert_eq!(cut.player.selected_hotbar.as_ref().unwrap().value, 1);
            assert_eq!(cut.slot, Some(37));
            assert!(
                matches!(&cut.held.as_ref().unwrap().value, api::SlotKnowledge::Item { item: i } if i.id == item)
            );
            assert!(cut.player.receive_sequence > 10);
            !stop.load(Ordering::Acquire)
        });
        let mut pending = Box::pin(async {
            if block {
                ops.use_on_block_guarded(target, api::Hand::Main, None, check)
                    .await
            } else {
                ops.use_item_guarded(api::Hand::Main, None, check).await
            }
        });
        assert!(
            timeout(Duration::from_millis(20), pending.as_mut())
                .await
                .is_err()
        );
        f.slot(37, Some(item)).await;
        f.receive(ids::play_clientbound::HELD_ITEM_SLOT, &[1]).await;
        let mut state = f.session.state.lock().await;
        state.position = Some([0.75, 1.0, 0.5]);
        state.ready = true;
        drop(state);
        stopped.store(true, Ordering::Release);
        f.session.changed.notify_waiters();
        assert!(matches!(
            pending.await.unwrap(),
            GuardedUseResult::NotSent {
                reason: UseNotSentReason::GuardRejected,
                ..
            }
        ));
        f.silent().await;
        f.stop().await;
    }
}
#[tokio::test]
async fn guarded_use_gate_deadline_and_cancelled_writer_never_send() {
    let mut f = Fixture::new().await;
    let client = f.client();
    let ops = client.survival();
    let gate = f.session.state.lock().await;
    tokio::time::pause();
    let mut pending = Box::pin(ops.use_item_guarded(api::Hand::Main, None, |_| panic!("deadline")));
    std::future::poll_fn(|cx| {
        assert!(std::future::Future::poll(pending.as_mut(), cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    tokio::time::advance(Duration::from_secs(6)).await;
    assert!(matches!(
        pending.await.unwrap(),
        GuardedUseResult::NotSent {
            reason: UseNotSentReason::AdmissionDeadline,
            ..
        }
    ));
    tokio::time::resume();
    drop(gate);
    f.silent().await;
    let writer = f.session.writer.clone().lock_owned().await;
    let api = f.api.clone();
    let pending = tokio::spawn(async move {
        api.use_item_guarded(api::Hand::Main, None, |_| panic!("cancelled check"))
            .await
    });
    timeout(Duration::from_secs(1), async {
        loop {
            if f.session.state.try_lock().is_err() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    drop(writer);
    f.silent().await;
    assert!(matches!(
        ops.use_item_guarded(api::Hand::Main, None, |_| true)
            .await
            .unwrap(),
        GuardedUseResult::Sent { .. }
    ));
    assert_eq!(f.packet().await.0, ids::play_serverbound::USE_ITEM);
    f.stop().await;
}

#[tokio::test]
async fn guarded_use_checks_stop_after_writer_wait() {
    for block in [false, true] {
        let mut f = Fixture::new().await;
        let writer = f.session.writer.clone().lock_owned().await;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = calls.clone();
        let client = f.client();
        let pending = tokio::spawn(async move {
            let check: guarded::Check = Box::new(move |_| {
                count.fetch_add(1, Ordering::SeqCst);
                !stopped.load(Ordering::Acquire)
            });
            let ops = client.survival();
            if block {
                ops.use_on_block_guarded(
                    BlockUseTarget {
                        position: [1, 0, 0],
                        face: crate::BlockFace::Up,
                        cursor: [0.5; 3],
                    },
                    api::Hand::Main,
                    None,
                    check,
                )
                .await
            } else {
                ops.use_item_guarded(api::Hand::Main, None, check).await
            }
        });
        timeout(Duration::from_secs(1), async {
            while f.session.state.try_lock().is_ok() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        stop.store(true, Ordering::Release);
        drop(writer);
        assert!(matches!(
            pending.await.unwrap().unwrap(),
            GuardedUseResult::NotSent {
                reason: UseNotSentReason::GuardRejected,
                ..
            }
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        f.silent().await;
        f.stop().await;
    }
}
#[tokio::test]
async fn guarded_use_partial_write_is_uncertain_and_never_retried() {
    let f = Fixture::new().await;
    let control = crate::client::ledger::testing::Control::default();
    control.release(usize::MAX, true);
    control.fail(Some(1), false);
    f.session
        .writer
        .lock()
        .await
        .stream
        .test_replace(control.writer());
    let result = f
        .client()
        .survival()
        .use_item_guarded(api::Hand::Main, None, |_| true)
        .await;
    assert_eq!(
        result.unwrap_err().kind(),
        crate::ErrorKind::UncertainDispatch
    );
    assert_eq!(control.bytes().len(), 1);
    assert!(
        f.client()
            .survival()
            .use_item_guarded(api::Hand::Main, None, |_| true)
            .await
            .is_err()
    );
    assert_eq!(control.bytes().len(), 1);
    f.stop().await;
}
