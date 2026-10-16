use super::*;
use crate::client::{DrainScope, StopTargetKind, adapter::ControlOps};
use std::future::Future;
async fn fixture() -> (
    Bot,
    tokio::sync::mpsc::UnboundedReceiver<(i32, Vec<u8>)>,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let (bot, packets, release, server) =
        super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
    super::super::common_motion::tests::seed_motion(&bot).await;
    (bot, packets, release, server)
}
#[tokio::test]
async fn quiesce_legacy_void_setters_are_noops_and_native_clones_reject_actions() {
    let (bot, mut packets, release, server) = fixture().await;
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let key = client.generation_key();
    let clone = bot.clone();
    let mut q = client.begin_quiesce(key).unwrap();
    assert!(
        matches!(client.clone().begin_quiesce(key), Err(e) if e.kind() == crate::client::DrainErrorKind::AlreadyQuiescing)
    );
    q.admitted_drain().wait().await.unwrap();
    let state = clone.control_snapshot().await;
    let barrier = clone.teleport_barrier_ticks.load(Ordering::Acquire);
    clone
        .set_control(ControlState {
            forward: true,
            ..Default::default()
        })
        .await;
    clone.clear_control().await;
    clone.suspend_movement_for(Duration::from_secs(30));
    assert_eq!(clone.control_snapshot().await.value, state.value);
    assert_eq!(clone.control_snapshot().await.revision, state.revision);
    assert_eq!(
        clone.teleport_barrier_ticks.load(Ordering::Acquire),
        barrier
    );
    assert_eq!(
        clone.swing_arm(Hand::Main).await.unwrap_err().kind(),
        crate::ErrorKind::State
    );
    assert_eq!(
        clone
            .unstable()
            .move_relative(1., 0.)
            .await
            .unwrap_err()
            .kind(),
        crate::ErrorKind::State
    );
    assert_eq!(
        client
            .survival()
            .dig([8, 64, 8], crate::BlockFace::Up)
            .await
            .unwrap_err()
            .kind(),
        crate::ErrorKind::State
    );
    assert!(q.stop_targets().await.unwrap().is_empty());
    let final_drain = q.seal_for_fence().unwrap();
    let receipt = final_drain.wait().await.unwrap();
    assert_eq!(receipt.scope(), DrainScope::SealedForFence);
    let w = bot.connection.ledger().test_written_through();
    bot.apply_packet(0x20, 99i64.to_be_bytes().to_vec())
        .await
        .unwrap();
    let mut position = Vec::new();
    for v in [8f64, 65., 8.] {
        position.extend(v.to_be_bytes());
    }
    position.extend(0f32.to_be_bytes());
    position.extend(0f32.to_be_bytes());
    position.push(0);
    put_varint(&mut position, 2);
    bot.apply_packet(0x35, position).await.unwrap();
    assert_eq!(bot.connection.ledger().test_written_through(), w);
    assert!(packets.try_recv().is_err());
    let _ = client.revoke_connection();
    assert_eq!(final_drain.wait().await.unwrap(), receipt);
    release.send(()).unwrap();
    drop(clone);
    drop(bot);
    drop(client);
    drop(q);
    server.await.unwrap();
}
#[tokio::test]
async fn quiesce_legacy_queued_jump_writes_once_then_parks_and_selected_movement_releases() {
    let (bot, mut packets, release, server) = fixture().await;
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let record = ControlOps::start_control(&bot, crate::client::GameMode::Survival)
        .await
        .unwrap();
    ControlOps::request_ground_jump(&bot, crate::client::GameMode::Survival, record.session_id)
        .await
        .unwrap();
    let mut q = client.begin_quiesce(client.generation_key()).unwrap();
    assert!(!q.admitted_drain().is_drained().unwrap());
    timeout(Duration::from_secs(1), q.admitted_drain().wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(packets.recv().await.unwrap().0, 0x13);
    let record = ControlOps::control_record(&bot).await.unwrap().unwrap();
    assert_eq!(record.dispatched_ticks, 1);
    assert!(matches!(
        record.ground_jump.unwrap().status,
        crate::client::control::GroundJumpStatus::Evaluated {
            dispatched: true,
            ..
        }
    ));
    let target = q
        .stop_targets()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.control_session_id() == Some(record.session_id))
        .unwrap();
    q.stop_permit().unwrap().neutralize(target).await.unwrap();
    q.seal_for_fence().unwrap().wait().await.unwrap();
    assert!(packets.try_recv().is_err());
    release.send(()).unwrap();
    drop(client);
    drop(q);
    drop(bot);
    server.await.unwrap();
}
#[tokio::test]
async fn quiesce_legacy_item_release_is_explicit_once_only_with_side_effects_allowed() {
    let (bot, mut packets, release, server) = fixture().await;
    bot.use_item(Hand::Main).await.unwrap();
    assert_eq!(packets.recv().await.unwrap().0, 0x2e);
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let mut q = client.begin_quiesce(client.generation_key()).unwrap();
    q.admitted_drain().wait().await.unwrap();
    let targets = q.stop_targets().await.unwrap();
    let target = targets
        .into_iter()
        .find(|t| t.kind() == StopTargetKind::ItemUse)
        .unwrap();
    assert!(packets.try_recv().is_err());
    let mut permit = q.stop_permit().unwrap();
    let receipt = permit.neutralize(target).await.unwrap();
    assert_eq!(permit.neutralize(target).await.unwrap(), receipt);
    let (id, payload) = packets.recv().await.unwrap();
    assert_eq!(id, 0x1b);
    assert_eq!(payload[0], 5);
    q.seal_for_fence().unwrap().wait().await.unwrap();
    assert!(packets.try_recv().is_err());
    release.send(()).unwrap();
    drop(client);
    drop(q);
    drop(bot);
    server.await.unwrap();
}
#[tokio::test]
async fn quiesce_legacy_final_seal_waits_for_stalled_teleport_pair() {
    let (bot, mut packets, release, server) = fixture().await;
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let mut q = client.begin_quiesce(client.generation_key()).unwrap();
    q.admitted_drain().wait().await.unwrap();
    let writer = bot.writer.lock().await;
    let mut position = Vec::new();
    for v in [8f64, 65., 8.] {
        position.extend(v.to_be_bytes());
    }
    position.extend(0f32.to_be_bytes());
    position.extend(0f32.to_be_bytes());
    position.push(0);
    put_varint(&mut position, 2);
    let owner = bot.clone();
    let packet = tokio::spawn(async move { owner.apply_packet(0x35, position).await });
    timeout(Duration::from_secs(1), async {
        while bot.coherent_state_gate.try_lock().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let final_drain = q.seal_for_fence().unwrap();
    assert!(!final_drain.is_drained().unwrap());
    drop(writer);
    packet.await.unwrap().unwrap();
    assert_eq!(packets.recv().await.unwrap().0, 0x00);
    assert_eq!(packets.recv().await.unwrap().0, 0x13);
    let receipt = final_drain.wait().await.unwrap();
    assert_eq!(
        receipt.written_through().get(),
        bot.connection.ledger().test_written_through()
    );
    assert!(packets.try_recv().is_err());
    release.send(()).unwrap();
    drop(client);
    drop(q);
    drop(bot);
    server.await.unwrap();
}
#[tokio::test]
async fn quiesce_legacy_intentional_abort_is_clean_only_after_confirmation() {
    let (bot, mut packets, release, server) = fixture().await;
    let position = BlockPos { x: 9, y: 65, z: 8 };
    bot.unstable()
        .send_digging(DiggingStatus::Started, position, BlockFace::Up)
        .await
        .unwrap();
    assert_eq!(packets.recv().await.unwrap().1[0], 0);
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let mut q = client.begin_quiesce(client.generation_key()).unwrap();
    q.admitted_drain().wait().await.unwrap();
    let target = q
        .stop_targets()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.kind() == StopTargetKind::Digging)
        .unwrap();
    let mut permit = q.stop_permit().unwrap();
    let mut stop = Box::pin(permit.neutralize(target));
    assert!(timeout(Duration::from_millis(10), &mut stop).await.is_err());
    let (id, abort) = packets.recv().await.unwrap();
    assert_eq!(id, 0x1b);
    assert_eq!(abort[0], 1);
    assert!(timeout(Duration::from_millis(10), &mut stop).await.is_err());
    let mut ack = position.packed().to_be_bytes().to_vec();
    put_varint(&mut ack, 1);
    put_varint(&mut ack, 1);
    ack.push(1);
    bot.apply_packet(0x07, ack).await.unwrap();
    let receipt = stop.await.unwrap();
    assert_eq!(receipt.target(), target);
    q.seal_for_fence().unwrap().wait().await.unwrap();
    assert!(packets.try_recv().is_err());
    release.send(()).unwrap();
    drop(client);
    drop(q);
    drop(bot);
    server.await.unwrap();
}
#[tokio::test]
async fn quiesce_legacy_native_jump_remains_finite_until_its_tick() {
    let (bot, mut packets, release, server) = fixture().await;
    bot.jump().await.unwrap();
    bot.teleport_barrier_ticks.store(0, Ordering::Release);
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let mut q = client.begin_quiesce(client.generation_key()).unwrap();
    assert!(!q.admitted_drain().is_drained().unwrap());
    timeout(Duration::from_secs(1), q.admitted_drain().wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(packets.recv().await.unwrap().0, 0x13);
    assert!(!bot.jump_requested.load(Ordering::Acquire));
    q.seal_for_fence().unwrap().wait().await.unwrap();
    assert!(packets.try_recv().is_err());
    release.send(()).unwrap();
    drop(client);
    drop(q);
    drop(bot);
    server.await.unwrap();
}
#[tokio::test]
async fn quiesce_legacy_stale_target_cannot_be_restored_by_mode_round_trip() {
    let (bot, mut packets, release, server) = fixture().await;
    bot.use_item(Hand::Main).await.unwrap();
    packets.recv().await.unwrap();
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let mut q = client.begin_quiesce(client.generation_key()).unwrap();
    q.admitted_drain().wait().await.unwrap();
    let target = q
        .stop_targets()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.kind() == StopTargetKind::ItemUse)
        .unwrap();
    for mode in [1f32, 0.] {
        let mut p = vec![3];
        p.extend(mode.to_be_bytes());
        bot.apply_packet(0x1e, p).await.unwrap();
    }
    assert!(matches!(
        q.stop_permit().unwrap().neutralize(target).await,
        Err(StopError::ContextChanged { .. })
    ));
    assert_eq!(
        q.seal_for_fence().err().unwrap().kind(),
        crate::client::DrainErrorKind::RequiredReleaseFailed
    );
    assert!(packets.try_recv().is_err());
    release.send(()).unwrap();
    drop(client);
    drop(q);
    drop(bot);
    server.await.unwrap();
}
#[tokio::test]
async fn quiesce_legacy_vehicle_neutral_keeps_original_mount_and_emits_no_position() {
    let (bot, mut packets, release, server) = fixture().await;
    bot.player.lock().await.entity_id = Some(7);
    bot.apply_packet(0x4b, vec![12, 1, 7]).await.unwrap();
    bot.set_vehicle_control(VehicleControl {
        forward: 1.,
        sideways: 0.,
        jump: false,
        dismount: false,
    })
    .await
    .unwrap();
    assert_eq!(packets.recv().await.unwrap().0, 0x1d);
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let mut q = client.begin_quiesce(client.generation_key()).unwrap();
    q.admitted_drain().wait().await.unwrap();
    let target = q
        .stop_targets()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.kind() == StopTargetKind::VehicleInput)
        .unwrap();
    assert_eq!(target.mount().unwrap().native_vehicle_id(), 12);
    q.stop_permit().unwrap().neutralize(target).await.unwrap();
    assert_eq!(packets.recv().await.unwrap(), (0x1d, vec![0; 9]));
    assert_eq!(packets.recv().await.unwrap(), (0x16, vec![0, 0]));
    q.seal_for_fence().unwrap().wait().await.unwrap();
    assert!(packets.try_recv().is_err());
    release.send(()).unwrap();
    drop(client);
    drop(q);
    drop(bot);
    server.await.unwrap();
}

#[tokio::test]
async fn quiesce_legacy_abort_cleanup_timeout_is_permanent_transaction_unconfirmed() {
    let (mut bot, mut packets, release, server) = fixture().await;
    bot.connection_options.protocol_ack_timeout = Duration::from_millis(20);
    bot.unstable()
        .send_digging(
            DiggingStatus::Started,
            BlockPos { x: 9, y: 65, z: 8 },
            BlockFace::Up,
        )
        .await
        .unwrap();
    packets.recv().await.unwrap();
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let mut q = client.begin_quiesce(client.generation_key()).unwrap();
    q.admitted_drain().wait().await.unwrap();
    let target = q
        .stop_targets()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.kind() == StopTargetKind::Digging)
        .unwrap();
    let error = q
        .stop_permit()
        .unwrap()
        .neutralize(target)
        .await
        .unwrap_err();
    assert!(
        matches!(error, StopError::RequiredReleaseFailed { drain, .. } if drain.kind() == crate::client::DrainErrorKind::TransactionExpired)
    );
    assert_eq!(packets.recv().await.unwrap().1[0], 1);
    assert_eq!(
        q.seal_for_fence().err().unwrap().kind(),
        crate::client::DrainErrorKind::TransactionExpired
    );
    assert!(packets.try_recv().is_err());
    release.send(()).unwrap();
    drop(client);
    drop(q);
    drop(bot);
    server.await.unwrap();
}

#[tokio::test]
async fn quiesce_legacy_jump_waiting_for_producer_lock_loses_before_admission() {
    for common in [false, true] {
        let (bot, mut packets, release, server) = fixture().await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let control = if common {
            Some(
                ControlOps::start_control(&bot, crate::client::GameMode::Survival)
                    .await
                    .unwrap(),
            )
        } else {
            None
        };
        let gate = bot.coherent_state_gate.lock().await;
        let source = &bot;
        let mut request: std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + '_>> =
            if let Some(control) = control {
                Box::pin(async move {
                    ControlOps::request_ground_jump(
                        source,
                        crate::client::GameMode::Survival,
                        control.session_id,
                    )
                    .await
                    .map(|_| ())
                })
            } else {
                Box::pin(bot.jump())
            };
        std::future::poll_fn(|cx| {
            assert!(request.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        let mut q = client.begin_quiesce(client.generation_key()).unwrap();
        drop(gate);
        assert_eq!(request.await.unwrap_err().kind(), crate::ErrorKind::State);
        timeout(Duration::from_secs(1), q.admitted_drain().wait())
            .await
            .unwrap()
            .unwrap();
        q.seal_for_fence().unwrap().wait().await.unwrap();
        assert!(packets.try_recv().is_err());
        release.send(()).unwrap();
        drop(q);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
}

#[tokio::test]
async fn quiesce_legacy_completed_common_owner_is_retired() {
    let (bot, _packets, release, server) = fixture().await;
    let record = ControlOps::start_control(&bot, crate::client::GameMode::Survival)
        .await
        .unwrap();
    ControlOps::stop_control_for(&bot, record.session_id)
        .await
        .unwrap();
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let mut q = client.begin_quiesce(client.generation_key()).unwrap();
    q.admitted_drain().wait().await.unwrap();
    assert!(
        q.stop_targets()
            .await
            .unwrap()
            .iter()
            .all(|t| t.control_session_id() != Some(record.session_id))
    );
    q.seal_for_fence().unwrap().wait().await.unwrap();
    release.send(()).unwrap();
    drop(client);
    drop(q);
    drop(bot);
    server.await.unwrap();
}

#[tokio::test]
async fn quiesce_legacy_native_clear_preserves_common_and_selected_owners_are_independent() {
    let (bot, mut packets, release, server) = fixture().await;
    bot.player.lock().await.entity_id = Some(7);
    let record = ControlOps::start_control(&bot, crate::client::GameMode::Survival)
        .await
        .unwrap();
    bot.clear_control().await;
    assert!(
        bot.connection
            .ledger()
            .stop_targets()
            .unwrap()
            .iter()
            .any(|t| t.control_session_id() == Some(record.session_id))
    );
    bot.set_control(ControlState {
        forward: true,
        sprint: true,
        ..Default::default()
    })
    .await;
    assert!(bot.control().await.forward);
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let mut q = client.begin_quiesce(client.generation_key()).unwrap();
    q.admitted_drain().wait().await.unwrap();
    while packets.try_recv().is_ok() {}
    let targets = q.stop_targets().await.unwrap();
    let native = *targets
        .iter()
        .find(|t| t.kind() == StopTargetKind::Movement && t.control_session_id().is_none())
        .unwrap();
    let common = *targets
        .iter()
        .find(|t| t.control_session_id() == Some(record.session_id))
        .unwrap();
    q.stop_permit().unwrap().neutralize(native).await.unwrap();
    assert_eq!(bot.control().await, ControlState::default());
    assert!(packets.try_recv().is_err());
    assert!(q.stop_targets().await.unwrap().contains(&common));
    q.stop_permit().unwrap().neutralize(common).await.unwrap();
    for action in [4, 1] {
        assert_eq!(packets.recv().await.unwrap(), (0x1c, vec![7, action, 0]));
    }
    q.seal_for_fence().unwrap().wait().await.unwrap();
    assert!(packets.try_recv().is_err());
    release.send(()).unwrap();
    drop(client);
    drop(q);
    drop(bot);
    server.await.unwrap();
}

#[tokio::test]
async fn quiesce_legacy_movement_mount_round_trip_cannot_stop_replacement_context() {
    let (bot, mut packets, release, server) = fixture().await;
    bot.player.lock().await.entity_id = Some(7);
    let record = ControlOps::start_control(&bot, crate::client::GameMode::Survival)
        .await
        .unwrap();
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let mut q = client.begin_quiesce(client.generation_key()).unwrap();
    q.admitted_drain().wait().await.unwrap();
    let target = q
        .stop_targets()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.control_session_id() == Some(record.session_id))
        .unwrap();
    while packets.try_recv().is_ok() {}
    bot.apply_packet(0x4b, vec![12, 1, 7]).await.unwrap();
    bot.apply_packet(0x4b, vec![12, 0]).await.unwrap();
    assert!(matches!(
        q.stop_permit().unwrap().neutralize(target).await,
        Err(StopError::ContextChanged { .. })
    ));
    assert_eq!(
        q.seal_for_fence().err().unwrap().kind(),
        crate::client::DrainErrorKind::RequiredReleaseFailed
    );
    assert!(packets.try_recv().is_err());
    release.send(()).unwrap();
    drop(client);
    drop(q);
    drop(bot);
    server.await.unwrap();
}

#[tokio::test]
async fn quiesce_legacy_native_clear_keeps_stop_until_wire_posture_is_neutral() {
    let (bot, mut packets, release, server) = fixture().await;
    bot.player.lock().await.entity_id = Some(7);
    bot.set_control(ControlState {
        forward: true,
        sprint: true,
        ..Default::default()
    })
    .await;
    let first = bot.connection.ledger().native_target().unwrap();
    bot.set_control(ControlState {
        sneak: true,
        ..Default::default()
    })
    .await;
    let latest = bot.connection.ledger().native_target().unwrap();
    assert_ne!(first, latest);
    // A tick for the prior sample must not retire replacement input.
    bot.connection.ledger().clear_target(first);
    assert!(bot.connection.ledger().stop_valid(latest));
    bot.clear_control().await;
    assert_eq!(bot.control().await, ControlState::default());
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let mut q = client.begin_quiesce(client.generation_key()).unwrap();
    q.admitted_drain().wait().await.unwrap();
    assert!(q.stop_targets().await.unwrap().contains(&latest));
    q.stop_permit().unwrap().neutralize(latest).await.unwrap();
    for action in [4, 1] {
        assert_eq!(packets.recv().await.unwrap(), (0x1c, vec![7, action, 0]));
    }
    q.seal_for_fence().unwrap().wait().await.unwrap();
    assert!(packets.try_recv().is_err());
    release.send(()).unwrap();
    drop(client);
    drop(q);
    drop(bot);
    server.await.unwrap();
}

#[tokio::test]
async fn shutdown_waiters_are_passive_cancel_safe_concurrent_and_cached() {
    let (bot, _packets, release, server) = fixture().await;
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let writer = bot.writer.lock().await;
    let mut cancelled = Box::pin(client.wait_shutdown());
    std::future::poll_fn(|cx| {
        assert!(cancelled.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    let _fence = client.revoke_connection();
    drop(cancelled);
    let mut waiting = Box::pin(client.wait_shutdown());
    std::future::poll_fn(|cx| {
        assert!(waiting.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(waiting);
    drop(writer);
    let clone = client.clone();
    let (first, second) = timeout(Duration::from_secs(2), async {
        tokio::join!(client.wait_shutdown(), clone.wait_shutdown())
    })
    .await
    .unwrap();
    first.unwrap();
    second.unwrap();
    client.wait_shutdown().await.unwrap();
    // The peer and idle native/common clones survive successful local completion.
    release.send(()).unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn shutdown_success_cannot_repair_a_failed_drain() {
    let (bot, _packets, release, server) = fixture().await;
    let client = crate::Client::from_java_1_16_1(bot.clone());
    let work = bot.connection.ledger().work();
    let owner = client.begin_quiesce(client.generation_key()).unwrap();
    drop(work);
    let failed = owner.admitted_drain().wait().await.unwrap_err();
    assert_eq!(failed.kind(), crate::client::DrainErrorKind::OwnerLost);
    let _fence = client.revoke_connection();
    timeout(Duration::from_secs(2), client.wait_shutdown())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(owner.admitted_drain().wait().await.unwrap_err(), failed);
    release.send(()).unwrap();
    server.await.unwrap();
}
