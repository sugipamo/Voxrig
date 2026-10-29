//! Drop-boundary characterizations retained from the mo8 audit.
use super::*;
use crate::body::{Action, Body, BodyConfig, HeadlessBody, ReleaseStatus};
use std::future::Future;

#[tokio::test]
async fn mo9_raw_native_dig_drop_leaves_start_without_abort_and_fails_drain() {
    let (bot, mut packets, release, server) =
        super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
    super::common_motion::tests::seed_motion(&bot).await;
    let target = BlockPos { x: 9, y: 65, z: 8 };
    bot.world.lock().await.set_block_for_test(target, 1);
    let mut waiter = Box::pin(bot.dig_block(target, BlockFace::Up));
    std::future::poll_fn(|cx| {
        assert!(waiter.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    let start = timeout(Duration::from_secs(1), packets.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!((start.0, start.1[0]), (0x1b, 0));
    drop(waiter);
    let mut drain = bot.connection.ledger().cut(false);
    assert!(drain.inspect().unwrap().is_err());
    assert!(packets.try_recv().is_err());
    assert!(!bot.operational_dig.active());
    let _ = bot.revoke_connection();
    crate::Client::from_java_1_16_1(bot.clone())
        .wait_shutdown()
        .await
        .unwrap();
    drop(release);
    server.await.unwrap();
}

fn body(bot: &Bot) -> Body<HeadlessBody> {
    Body::new(
        HeadlessBody::new(
            crate::Client::from_java_1_16_1(bot.clone()),
            BodyConfig {
                action_timeout: Duration::from_secs(3),
                settle_window: Duration::from_millis(100),
                ..BodyConfig::default()
            },
        )
        .unwrap(),
    )
}

#[tokio::test]
async fn mo9_body_dropped_send_waiter_keeps_owned_look() {
    let (bot, mut packets, release, server) =
        super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
    super::common_motion::tests::seed_motion(&bot).await;
    let body = body(&bot);
    let mut waiter = Box::pin(body.send_and_trust(Action::Look {
        yaw: 12.,
        pitch: 4.,
    }));
    std::future::poll_fn(|cx| {
        assert!(waiter.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(waiter);
    assert_eq!(
        timeout(Duration::from_secs(1), packets.recv())
            .await
            .unwrap()
            .unwrap()
            .0,
        0x13
    );
    assert_eq!(
        body.release_control().await.unwrap(),
        ReleaseStatus::Released
    );
    let _ = bot.revoke_connection();
    crate::Client::from_java_1_16_1(bot.clone())
        .wait_shutdown()
        .await
        .unwrap();
    drop(release);
    server.await.unwrap();
}

#[tokio::test]
async fn mo9_body_released_is_not_a_seal_of_actor_owned_look() {
    let (bot, _packets, release, server) =
        super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
    super::common_motion::tests::seed_motion(&bot).await;
    let control = crate::client::ledger::testing::Control::default();
    bot.writer.lock().await.inner.test_replace(control.writer());
    let body = body(&bot);
    let id = body
        .submit(Action::Look {
            yaw: 12.,
            pitch: 4.,
        })
        .await
        .unwrap();
    // Exact barrier: actor is inside the socket write, with zero bytes allowed.
    control.until(|c| c.progress().0 > 0).await;
    assert!(control.bytes().is_empty());
    body.cancel(id).await.unwrap();
    assert_eq!(
        body.release_control().await.unwrap(),
        ReleaseStatus::Released
    );
    assert!(control.bytes().is_empty());
    control.release(usize::MAX, true);
    control.until(|c| c.progress().2 == 1).await;
    let bytes = control.bytes();
    assert_eq!(
        read_packet(&mut bytes.as_slice(), None).await.unwrap().0,
        0x13
    );
    let _ = bot.revoke_connection();
    crate::Client::from_java_1_16_1(bot.clone())
        .wait_shutdown()
        .await
        .unwrap();
    drop(release);
    server.await.unwrap();
}

#[tokio::test]
async fn mo9_common_dig_drop_requests_abort_and_releases_exclusive_owner() {
    let (bot, mut packets, release, server) =
        super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
    super::common_motion::tests::seed_motion(&bot).await;
    let target = [9, 65, 8];
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
    bot.world
        .lock()
        .await
        .set_block_for_test(BlockPos { x: 9, y: 65, z: 8 }, 1);
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let survival = client.survival();
    let mut waiter = Box::pin(survival.dig(target, BlockFace::Up));
    std::future::poll_fn(|cx| {
        assert!(waiter.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    let start = timeout(Duration::from_secs(1), packets.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!((start.0, start.1[0]), (0x1b, 0));
    drop(waiter);
    let abort = timeout(Duration::from_secs(1), packets.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!((abort.0, abort.1[0]), (0x1b, 1));
    timeout(Duration::from_secs(1), async {
        while bot.operational_dig.active() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        survival.dig_cleanup_record().unwrap().status,
        crate::client::DigCleanupStatus::Aborted
    );
    // Production tracking mode preserves domain admission, while cancellation
    // leaves sticky finite-work failure for unissued drain receipts.
    assert!(
        bot.connection
            .ledger()
            .cut(false)
            .inspect()
            .unwrap()
            .is_err()
    );
    let next = survival.begin_dig(target, BlockFace::Up).unwrap();
    next.request_cancel();
    assert_eq!(
        *next.finished().await.cancel_outcome(),
        crate::client::DigCancelOutcome::StartSuppressed
    );
    let _ = bot.revoke_connection();
    client.wait_shutdown().await.unwrap();
    drop(release);
    server.await.unwrap();
}

#[tokio::test]
async fn mo9_registration_capacity_drop_panic_no_runtime_id_exhaustion_and_close() {
    let (bot, _packets, release, server) =
        super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
    super::common_motion::tests::seed_motion(&bot).await;
    let client = crate::Client::from_java_1_16_1(bot);
    crate::client::owned_operation::tests::registration_contract(client.clone()).await;
    // Panic is retained as a shutdown error; it cannot become a clean close proof.
    assert!(client.wait_shutdown().await.is_err());
    drop(release);
    server.await.unwrap();
}
