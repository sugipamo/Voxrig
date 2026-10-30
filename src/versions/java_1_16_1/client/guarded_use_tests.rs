use super::*;
use crate::client::{
    self as api,
    guarded_use::{self as guarded, *},
};
use guarded::tests::Fixture as _;

struct Fixture {
    bot: Bot,
    packets: tokio::sync::mpsc::UnboundedReceiver<(i32, Vec<u8>)>,
}
impl guarded::tests::Fixture for Fixture {
    fn client(&self) -> crate::Client {
        crate::Client::from_java_1_16_1(self.bot.clone())
    }
    async fn mode(&mut self, mode: api::GameMode) {
        self.bot.survival.write().await.game_mode = Some(if mode == api::GameMode::Creative {
            1
        } else {
            0
        });
    }
    async fn slot(&mut self, slot: i16, item: Option<api::registry::RegistryId>) {
        let mut payload = vec![0];
        payload.extend(slot.to_be_bytes());
        if let Some(item) = item {
            payload.push(1);
            put_varint(&mut payload, item.value());
            payload.extend([1, 0]);
        } else {
            payload.push(0);
        }
        self.bot.apply_packet(0x16, payload).await.unwrap();
    }
    async fn packet(&mut self) -> (i32, Vec<u8>) {
        tokio::time::timeout(Duration::from_secs(1), self.packets.recv())
            .await
            .unwrap()
            .unwrap()
    }
    async fn silent(&mut self) {
        assert!(
            tokio::time::timeout(Duration::from_millis(15), self.packets.recv())
                .await
                .is_err()
        );
    }
}
async fn fixture() -> (Fixture, tokio::task::JoinHandle<()>) {
    let (bot, packets, release, server) = super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
    release.send(()).unwrap();
    bot.survival.write().await.game_mode = Some(0);
    bot.select_hotbar(0).await.unwrap();
    let mut f = Fixture { bot, packets };
    assert_eq!(f.packet().await.0, 0x24);
    (f, server)
}
#[tokio::test]
async fn guarded_use_contract() {
    let (mut f, server) = fixture().await;
    Box::pin(guarded::tests::contract(&mut f)).await;
    drop(f);
    server.await.unwrap();
}
#[tokio::test]
async fn guarded_use_fresh_cut_after_gate_and_readiness() {
    for block in [false, true] {
        let (mut f, server) = fixture().await;
        let client = f.client();
        let item = client.registry().item("minecraft:dirt").unwrap().id;
        // Hold readiness false and then change pose, selection and received slot.
        f.bot.player.lock().await.spawned = false;
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let ops = client.survival();
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
            assert!(cut.player.receive_sequence >= 1);
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
            tokio::time::timeout(Duration::from_millis(20), pending.as_mut())
                .await
                .is_err()
        );
        let gate = f.bot.coherent_state_gate.lock().await;
        f.bot.player.lock().await.x = 0.75;
        f.bot.inventory.write().await.selected_hotbar = 1;
        f.bot.common_receipts.lock().await.selected_hotbar = Some(api::received(1, 100));
        f.bot.player.lock().await.spawned = true;
        drop(gate);
        f.slot(37, Some(item)).await;
        stopped.store(true, Ordering::Release);
        f.bot.ready.notify_waiters();
        let result = pending.await.unwrap();
        assert!(matches!(
            result,
            GuardedUseResult::NotSent {
                reason: UseNotSentReason::GuardRejected,
                ..
            }
        ));
        f.silent().await;
        drop(ops);
        drop(client);
        drop(f);
        server.await.unwrap();
    }
}
#[tokio::test]
async fn guarded_use_gate_deadline_and_cancelled_actor_never_send() {
    let (mut f, server) = fixture().await;
    let client = f.client();
    let ops = client.survival();
    let gate = f.bot.coherent_state_gate.lock().await;
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
    // Cancel while actor is queued on writer. The moved gate survives waiter
    // cancellation; the atomic cancellation transition prevents a later write.
    let writer = f.bot.writer.clone().lock_owned().await;
    let bot = f.bot.clone();
    let pending = tokio::spawn(async move {
        bot.use_item_guarded(Hand::Main, None, |_| panic!("cancelled check"))
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if f.bot.coherent_state_gate.try_lock().is_err() {
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
    let result = ops
        .use_item_guarded(api::Hand::Main, None, |_| true)
        .await
        .unwrap();
    assert!(matches!(result, GuardedUseResult::Sent { .. }));
    assert_eq!(f.packet().await.0, 0x2e);
    drop(ops);
    drop(client);
    drop(f);
    server.await.unwrap();
}

#[tokio::test]
async fn guarded_use_checks_stop_after_writer_wait() {
    for block in [false, true] {
        let (mut f, server) = fixture().await;
        let writer = f.bot.writer.clone().lock_owned().await;
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
        tokio::time::timeout(Duration::from_secs(1), async {
            while f.bot.coherent_state_gate.try_lock().is_ok() {
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
        drop(f);
        server.await.unwrap();
    }
}
#[tokio::test]
async fn guarded_use_partial_write_is_uncertain_and_never_retried() {
    let (f, server) = fixture().await;
    let control = crate::client::ledger::testing::Control::default();
    control.release(usize::MAX, true);
    control.fail(Some(1), false);
    f.bot
        .writer
        .lock()
        .await
        .inner
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
    let client = f.client();
    assert!(
        client
            .survival()
            .use_item_guarded(api::Hand::Main, None, |_| true)
            .await
            .is_err()
    );
    assert_eq!(control.bytes().len(), 1);
    drop(client);
    drop(f);
    server.await.unwrap();
}
