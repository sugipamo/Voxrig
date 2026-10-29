use super::*;
use crate::client::{
    OperationLimits, OperationOutcome, OperationUnknownReason,
    inventory::{InventoryClickButton as Button, InventorySource as Source},
};
use std::future::Future;
fn limits() -> OperationLimits {
    OperationLimits {
        run_budget: Duration::from_millis(30),
        cleanup_budget: Duration::ZERO,
    }
}
async fn assert_no_packet(f: &mut CommonFixture) {
    let mut read = Box::pin(read_packet(&mut f.peer, None));
    std::future::poll_fn(|cx| {
        assert!(read.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
}
#[tokio::test]
async fn owned_modern_writer_and_state_stall_returns_terminal_then_retains_late_receipt() {
    let mut f = CommonFixture::new().await;
    let client = f.client();
    let stamp = client.player_state().await.unwrap().session;
    let session = f.api.bot.session.clone();
    let writer = session.writer.lock().await;
    let operation = client
        .survival()
        .begin_inventory_click(stamp, Source::Player, 9, Button::Left, limits())
        .unwrap();
    // Coherent State is held by the original sender while waiting on writer.
    timeout(Duration::from_secs(1), async {
        while session.state.try_lock().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut waiter = Box::pin(operation.finished());
    std::future::poll_fn(|cx| {
        assert!(waiter.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(waiter);
    let done = timeout(Duration::from_secs(1), operation.finished())
        .await
        .unwrap();
    assert_eq!(done.id, operation.id());
    assert_eq!(
        done.outcome,
        OperationOutcome::Unknown(OperationUnknownReason::Deadline)
    );
    assert!(!done.owner_joined);
    assert!(session.state.try_lock().is_err());
    assert!(operation.snapshot().joined.is_none());
    drop(writer);
    assert_eq!(
        read_packet(&mut f.peer, None).await.unwrap().0,
        ids::play_serverbound::WINDOW_CLICK
    );
    // Send itself finishes, receipt owner and gate remain live.
    let retained = client
        .survival()
        .inventory_click_record()
        .await
        .unwrap()
        .unwrap();
    assert!(retained.send.dispatched);
    assert!(
        client
            .survival()
            .click_inventory(Source::Player, 10, Button::Left)
            .await
            .is_err()
    );
    f.slot(9, InventorySlot::Empty).await;
    let mut cursor = vec![];
    put_slot(&mut cursor, &plain("stone", 32));
    f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &cursor)
        .await;
    timeout(Duration::from_secs(1), async {
        while operation.snapshot().joined.is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let late = operation.snapshot().joined.unwrap();
    assert_eq!(late.id, done.id);
    assert_eq!(late.record.as_ref().unwrap().id, retained.id);
    assert_eq!(late.outcome, OperationOutcome::SentReceived);
    assert!(Arc::ptr_eq(&done, &operation.finished().await));
    f.stop().await;
}
#[tokio::test]
async fn owned_modern_stale_world_or_connection_never_selects_successor() {
    let mut f = CommonFixture::new().await;
    let client = f.client();
    let stamp = client.player_state().await.unwrap().session;
    let mut wrong = stamp;
    wrong.connection_id += 1;
    assert!(
        client
            .survival()
            .begin_inventory_click(wrong, Source::Player, 9, Button::Left, limits())
            .is_err()
    );
    // Hold the coherent boundary until the operation has its original scope.
    let session = f.api.bot.session.clone();
    let mut state = session.state.lock().await;
    let operation = client
        .survival()
        .begin_inventory_click(
            stamp,
            Source::Player,
            9,
            Button::Left,
            OperationLimits::default(),
        )
        .unwrap();
    state.loading.generation += 1;
    drop(state);
    let done = timeout(Duration::from_secs(1), operation.finished())
        .await
        .unwrap();
    assert_eq!(
        done.scope,
        crate::client::OperationScope::Inventory {
            session: stamp,
            source: Source::Player
        }
    );
    assert_eq!(done.outcome, OperationOutcome::NotSent);
    assert!(done.failure.is_some());
    assert!(session.state.lock().await.common_inventory_click.is_none());
    assert_no_packet(&mut f).await;
    f.stop().await;
}
#[tokio::test]
async fn owned_modern_retirement_is_finite_even_while_writer_is_held() {
    let f = CommonFixture::new().await;
    let client = f.client();
    let session = f.api.bot.session.clone();
    let writer = session.writer.lock().await;
    let retire = client.retire_connection(tokio::time::Instant::now());
    // Revocation is synchronous, even before polling this future.
    assert!(session.revoked.load(Ordering::Acquire));
    assert!(matches!(
        retire.await,
        crate::client::ConnectionRetirement::Unconfirmed { .. }
    ));
    drop(writer);
    assert!(matches!(
        client
            .retire_connection(tokio::time::Instant::now() + Duration::from_secs(2))
            .await,
        crate::client::ConnectionRetirement::Closed { .. }
    ));
    f.stop().await;
}

#[tokio::test]
async fn owned_modern_waiter_drop_at_header_body_and_flush_preserves_exactly_one_click() {
    let f = CommonFixture::new().await;
    let client = f.client();
    let stamp = client.player_state().await.unwrap().session;
    let control = crate::client::ledger::testing::Control::default();
    f.api
        .bot
        .session
        .writer
        .lock()
        .await
        .stream
        .test_replace(control.writer());
    let operation = client
        .survival()
        .begin_inventory_click(
            stamp,
            Source::Player,
            9,
            Button::Left,
            OperationLimits::default(),
        )
        .unwrap();
    control.until(|c| c.progress().0 > 0).await;
    for boundary in [0, 1, 3] {
        if boundary != 0 {
            control.release(boundary, false);
            control.until(|c| c.bytes().len() == boundary).await;
        }
        let mut waiter = Box::pin(operation.finished());
        std::future::poll_fn(|cx| {
            assert!(waiter.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        drop(waiter);
        assert!(operation.snapshot().completion.is_none());
    }
    control.release(usize::MAX, false);
    control.until(|c| c.progress().1 > 0).await;
    let mut waiter = Box::pin(operation.finished());
    std::future::poll_fn(|cx| {
        assert!(waiter.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(waiter);
    assert_eq!(control.progress().2, 0);
    control.release(usize::MAX, true);
    control.until(|c| c.progress().2 >= 2).await; // click + native response fence
    let bytes = control.bytes();
    let mut bytes = bytes.as_slice();
    assert_eq!(
        read_packet(&mut bytes, None).await.unwrap().0,
        ids::play_serverbound::WINDOW_CLICK
    );
    assert_eq!(
        read_packet(&mut bytes, None).await.unwrap().0,
        ids::play_serverbound::TAB_COMPLETE
    );
    assert!(bytes.is_empty());
    assert!(operation.snapshot().completion.is_none()); // receipt owner still needed
    let _ = client.revoke_connection();
    client.wait_shutdown().await.unwrap();
    let done = operation.finished().await;
    assert_ne!(done.outcome, OperationOutcome::NotSent);
    f.stop().await;
}

#[tokio::test]
async fn original_modern_click_and_transfer_dropped_during_frame_keep_inspection_and_close() {
    for transfer in [false, true] {
        for boundary in [0, 1, 3, usize::MAX] {
            let f = CommonFixture::new().await;
            let client = f.client();
            let session = f.api.bot.session.clone();
            let control = crate::client::ledger::testing::Control::default();
            session
                .writer
                .lock()
                .await
                .stream
                .test_replace(control.writer());
            control.release(boundary, false);
            let survival = client.survival();
            let mut send = Box::pin(async {
                if transfer {
                    survival
                        .transfer_inventory(Source::Player, 9)
                        .await
                        .map(|_| ())
                } else {
                    survival
                        .click_inventory(Source::Player, 9, Button::Left)
                        .await
                        .map(|_| ())
                }
            });
            std::future::poll_fn(|cx| {
                assert!(send.as_mut().poll(cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            assert!(control.progress().0 > 0);
            if boundary == usize::MAX {
                assert!(control.progress().1 > 0);
            } else {
                assert_eq!(control.bytes().len(), boundary);
            }
            drop(send);
            assert!(session.stopped.load(Ordering::Acquire));
            let mut state = session.state.lock().await;
            assert_eq!(
                state.inventory_executor.record().unwrap().status,
                crate::client::inventory::InventorySendStatus::ResyncRequired
            );
            if transfer {
                assert_eq!(
                    state.common_inventory_transfer.as_ref().unwrap().stage,
                    crate::client::inventory::InventoryTransferStage::RequiresInspection
                );
            } else {
                assert_eq!(
                    state.common_inventory_click.as_ref().unwrap().stage,
                    crate::client::inventory::InventoryClickStage::RequiresInspection
                );
            }
            drop(state);
            assert!(
                survival
                    .send_inventory_click(Source::Player, 10, Button::Left)
                    .await
                    .is_err()
            );
            f.stop().await;
        }
    }
}

#[tokio::test]
async fn owned_modern_original_opening_is_not_rebound_to_reused_window_id() {
    let mut f = CommonFixture::new().await;
    let client = f.client();
    let original = f.open_swap_container().await;
    let successor = f.open_swap_container().await;
    assert_ne!(original, successor);
    assert_eq!(original.window_id(), successor.window_id());
    let operation = client
        .survival()
        .begin_inventory_click(
            original.session(),
            Source::Container { screen: original },
            0,
            Button::Left,
            OperationLimits::default(),
        )
        .unwrap();
    let done = operation.finished().await;
    assert_eq!(
        done.scope,
        crate::client::OperationScope::Inventory {
            session: original.session(),
            source: Source::Container { screen: original }
        }
    );
    assert!(done.failure.is_some());
    assert_eq!(
        client.screen_state().await.unwrap().screen.unwrap().id,
        successor
    );
    assert!(
        f.api
            .bot
            .session
            .state
            .lock()
            .await
            .common_inventory_click
            .is_none()
    );
    assert_no_packet(&mut f).await;
    f.stop().await;
}
