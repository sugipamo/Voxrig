use crate::client::ledger::testing::Control as WriterControl;

#[tokio::test]
async fn ledger_legacy_all_writer_routes_share_one_flushed_prefix() {
    for compression in [None, Some(0), Some(256)] {
        let (actor, mut peer, writer) = actor_fixture_with_writer().await;
        actor.mark_ready().await;
        writer.lock().await.compression = compression;
        let context = OperationContext {
            generation: actor.generation(),
            source_observation_sequence: 0,
        };
        actor
            .dispatch(context, OperationClass::Normal, 1, &[7])
            .await
            .unwrap();
        actor
            .dispatch_operation(context, OperationClass::Normal, 2, &[8])
            .await
            .unwrap();
        actor
            .dispatch_interaction_batch(
                context,
                OperationClass::Normal,
                None,
                vec![(3, vec![9]), (4, vec![10])],
            )
            .await
            .unwrap();
        actor.dispatch_protocol(5, &[11]).await.unwrap();
        let revision = actor.motion_admission_revision().await.unwrap();
        actor.begin_bounded_motion(1, revision).await.unwrap();
        actor.bounded_position(1, vec![0; 33]).await.unwrap();
        actor.finish_bounded_motion(1).await.unwrap();
        for (id, payload) in [
            (1, vec![7]),
            (2, vec![8]),
            (3, vec![9]),
            (4, vec![10]),
            (5, vec![11]),
            (0x13, vec![0; 33]),
        ] {
            assert_eq!(
                read_packet(&mut peer, compression).await.unwrap(),
                (id, payload)
            );
        }
        assert_eq!(actor.ledger.test_written_through(), 6);
        assert!(actor.ledger.cut(false).wait().await.is_ok());
    }
}

#[tokio::test]
async fn ledger_legacy_actor_stalls_header_body_flush_and_covers_cancelled_waiter() {
    for compression in [None, Some(0), Some(256)] {
        let (actor, _, writer) = actor_fixture_with_writer().await;
        actor.mark_ready().await;
        let control = WriterControl::default();
        {
            let mut writer = writer.lock().await;
            writer.compression = compression;
            writer.inner.test_replace(control.writer());
        }
        let mut send = Box::pin(actor.dispatch_protocol(9, &[42; 32]));
        std::future::poll_fn(|cx| {
            assert!(send.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        control.until(|c| c.progress().0 > 0).await;
        let mut drain = actor.ledger.cut(false);
        assert!(drain.inspect().is_none());
        control.release(1, false);
        control.until(|c| c.bytes().len() == 1).await;
        assert_eq!(actor.ledger.test_written_through(), 0);
        control.release(usize::MAX, false);
        control.until(|c| c.progress().1 > 0).await;
        assert_eq!(actor.ledger.test_written_through(), 0);
        assert!(drain.inspect().is_none());
        drop(send); // Queue/actor owns this write; caller cancellation is not retirement.
        assert!(drain.inspect().is_none());
        control.release(usize::MAX, true);
        drain.wait().await.unwrap();
        assert_eq!(actor.ledger.test_written_through(), 1);
        let bytes = control.bytes();
        assert_eq!(
            read_packet(&mut bytes.as_slice(), compression)
                .await
                .unwrap(),
            (9, vec![42; 32])
        );
    }
}

#[tokio::test]
async fn ledger_legacy_partial_errors_and_batch_midway_failure_do_not_skip_ordinals() {
    for compression in [None, Some(0), Some(256)] {
        for (after, flush) in [
            (Some(0), false),
            (Some(1), false),
            (Some(2), false),
            (None, true),
        ] {
            let (actor, _, writer) = actor_fixture_with_writer().await;
            actor.mark_ready().await;
            let control = WriterControl::default();
            control.release(usize::MAX, true);
            control.fail(after, flush);
            {
                let mut writer = writer.lock().await;
                writer.compression = compression;
                writer.inner.test_replace(control.writer());
            }
            assert!(actor.dispatch_protocol(9, &[42; 32]).await.is_err());
            assert_eq!(actor.ledger.test_written_through(), 0);
            assert!(actor.ledger.cut(false).inspect().unwrap().is_err());
        }
        // Measure the exact first frame, then fail one byte into the second.
        let expected = WriterControl::default();
        expected.release(usize::MAX, true);
        crate::protocol::write_packet(&mut expected.writer(), compression, 1, &[7])
            .await
            .unwrap();
        let first = expected.bytes();
        let (actor, _, writer) = actor_fixture_with_writer().await;
        actor.mark_ready().await;
        let control = WriterControl::default();
        control.release(usize::MAX, true);
        control.fail(Some(first.len() + 1), false);
        {
            let mut writer = writer.lock().await;
            writer.compression = compression;
            writer.inner.test_replace(control.writer());
        }
        let context = OperationContext {
            generation: actor.generation(),
            source_observation_sequence: 0,
        };
        assert_eq!(
            actor
                .dispatch_interaction_batch(
                    context,
                    OperationClass::Normal,
                    None,
                    vec![(1, vec![7]), (2, vec![8]), (3, vec![9])]
                )
                .await
                .unwrap(),
            crate::DispatchOutcome::DeliveryUnknown
        );
        assert_eq!(actor.ledger.test_written_through(), 1);
        assert_eq!(control.bytes().len(), first.len() + 1);
        assert_eq!(&control.bytes()[..first.len()], &first);
        assert_eq!(control.progress().2, 1);
        assert!(actor.ledger.cut(false).inspect().unwrap().is_err());
    }
}

#[tokio::test]
async fn ledger_legacy_ack_waits_for_barrier_cleanup_and_expiry_stays_failed() {
    for expire in [false, true] {
        let (actor, mut peer, _) = actor_fixture_with_writer().await;
        actor.mark_ready().await;
        let context = OperationContext {
            generation: actor.generation(),
            source_observation_sequence: 0,
        };
        let transaction = actor
            .dispatch_acknowledged(
                context,
                OperationClass::Normal,
                crate::AcknowledgedOperation::WindowClick {
                    window_id: 0,
                    slot: 9,
                    button: 0,
                    mode: crate::ClickMode::Normal,
                    clicked: None,
                },
            )
            .await
            .unwrap();
        read_packet(&mut peer, None).await.unwrap();
        let action = transaction.window_action().unwrap();
        let barrier = transaction.barrier_work().unwrap();
        let mut drain = actor.ledger.cut(false);
        assert_eq!(actor.ledger.test_written_through(), 1);
        assert!(drain.inspect().is_none());
        if expire {
            actor
                .commands
                .send(Command::ExpireTransaction(
                    TransactionIdentity::WindowClick {
                        window_id: 0,
                        action,
                    },
                ))
                .await
                .unwrap();
            assert_eq!(
                transaction.wait().await,
                crate::DispatchOutcome::DeliveryUnknown
            );
            assert!(drain.inspect().unwrap().is_err());
            drop(barrier);
            // Preserve existing public behavior: expiry retires this identity,
            // not the connection or unrelated packet dispatch.
            actor
                .dispatch(context, OperationClass::Normal, 8, &[9])
                .await
                .unwrap();
            assert_eq!(read_packet(&mut peer, None).await.unwrap(), (8, vec![9]));
            assert!(actor.ledger.cut(false).inspect().unwrap().is_err());
        } else {
            assert!(actor.observe_window_confirmation(0, action, true).await);
            assert!(drain.inspect().is_none());
            drop(barrier); // exact local reconciliation precedes actor commit
            assert!(drain.inspect().is_none());
            assert!(actor.commit_window_barrier(0, action).await);
            assert_eq!(
                transaction.wait().await,
                crate::DispatchOutcome::Acknowledged
            );
            drain.wait().await.unwrap();
        }
    }
}

#[tokio::test]
async fn ledger_legacy_rejected_ack_response_inherits_transaction_root_after_cut() {
    let (actor, mut peer, _) = actor_fixture_with_writer().await;
    actor.mark_ready().await;
    let context = OperationContext {
        generation: actor.generation(),
        source_observation_sequence: 0,
    };
    let transaction = actor
        .dispatch_acknowledged(
            context,
            OperationClass::Normal,
            crate::AcknowledgedOperation::WindowClick {
                window_id: 0,
                slot: 9,
                button: 0,
                mode: crate::ClickMode::Normal,
                clicked: None,
            },
        )
        .await
        .unwrap();
    read_packet(&mut peer, None).await.unwrap();
    let mut drain = actor.ledger.cut(false);
    let (observed, response) = actor
        .observe_window_confirmation_work(0, transaction.window_action().unwrap(), false)
        .await;
    assert!(observed);
    assert_eq!(transaction.wait().await, crate::DispatchOutcome::Rejected);
    assert!(drain.inspect().is_none());
    actor
        .dispatch_protocol_work(7, &[0, 0, 1, 1], response)
        .await
        .unwrap();
    assert_eq!(
        read_packet(&mut peer, None).await.unwrap(),
        (7, vec![0, 0, 1, 1])
    );
    drain.wait().await.unwrap();
    assert_eq!(actor.ledger.test_written_through(), 2);
}

#[tokio::test]
async fn ledger_legacy_pending_expiry_survives_root_closure() {
    let (actor, mut peer, _) = actor_fixture_with_writer().await;
    actor.mark_ready().await;
    let context = OperationContext {
        generation: actor.generation(),
        source_observation_sequence: 0,
    };
    let transaction = actor
        .dispatch_acknowledged(
            context,
            OperationClass::Normal,
            crate::AcknowledgedOperation::DigFinish {
                position: crate::BlockPos { x: 3, y: 4, z: 5 },
                face: crate::BlockFace::Up,
            },
        )
        .await
        .unwrap();
    read_packet(&mut peer, None).await.unwrap();
    let mut drain = actor.ledger.cut(true);
    assert!(drain.inspect().is_none());
    actor
        .commands
        .send(Command::ExpireTransaction(transaction.identity))
        .await
        .unwrap();
    assert_eq!(
        transaction.wait().await,
        crate::DispatchOutcome::DeliveryUnknown
    );
    assert!(drain.inspect().unwrap().is_err());
}
