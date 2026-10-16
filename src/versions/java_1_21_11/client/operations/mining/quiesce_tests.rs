// Public staged API regressions on a real receiver/writer and native clone.
fn quiesce_client(f: &Fixture) -> crate::Client { crate::Client::from_java_1_21_11(f.api.bot.clone()) }

#[tokio::test]
async fn quiesce_modern_item_stop_is_once_only_and_final_seals_receiver_responses() {
    use crate::client::{DrainScope, StopTargetKind};
    let mut f = Fixture::new().await;
    let client = quiesce_client(&f);
    client.survival().use_item(crate::client::Hand::Main).await.unwrap();
    assert_eq!(read_packet(&mut f.peer, None).await.unwrap().0, ids::play_serverbound::USE_ITEM);
    let mut q = client.begin_quiesce(client.generation_key()).unwrap();
    let first = q.admitted_drain();
    assert_eq!(first.wait().await.unwrap().scope(), DrainScope::ProducersParked);
    assert_eq!(f.api.select_hotbar(1).await.unwrap_err().kind(), ErrorKind::State);
    assert_eq!(f.session.send(ids::play_serverbound::PLAYER_INPUT, &[1]).await.unwrap_err().kind(), ErrorKind::State);
    assert_eq!(client.player_control(GameMode::Survival).start_control().await.unwrap_err().kind(), ErrorKind::State);
    let target = q.stop_targets().await.unwrap().into_iter().find(|t| t.kind() == StopTargetKind::ItemUse).unwrap();
    assert_eq!(target.item_use_hand(), Some(crate::client::Hand::Main));
    {
        let mut permit = q.stop_permit().unwrap();
        let receipt = permit.neutralize(target).await.unwrap();
        assert_eq!(receipt.target(), target);
        assert_eq!(permit.neutralize(target).await.unwrap(), receipt);
    }
    let (id, payload) = read_packet(&mut f.peer, None).await.unwrap();
    assert_eq!(id, ids::play_serverbound::BLOCK_DIG); assert_eq!(payload[0], 5);
    let final_drain = q.seal_for_fence().unwrap();
    let receipt = final_drain.wait().await.unwrap();
    assert_eq!(receipt.scope(), DrainScope::SealedForFence);
    assert_eq!(q.seal_for_fence().unwrap().wait().await.unwrap(), receipt);
    assert_eq!(q.stop_permit().err().unwrap().kind(), crate::client::DrainErrorKind::Sealed);
    let w = f.session.ledger.test_written_through();
    // Post-seal keepalive and teleport are applied read-only, without any reply.
    f.receive(ids::play_clientbound::KEEP_ALIVE, &123i64.to_be_bytes()).await;
    let mut position = vec![0];
    for v in [8., 65., 8., 0., 0., 0.] { position.extend(f64::to_be_bytes(v)); }
    position.extend(0f32.to_be_bytes()); position.extend(0f32.to_be_bytes()); position.extend(0u32.to_be_bytes());
    f.receive(ids::play_clientbound::POSITION, &position).await;
    assert_eq!(f.session.ledger.test_written_through(), w);
    assert!(timeout(Duration::from_millis(30), read_packet(&mut f.peer, None)).await.is_err());
    let _ = client.revoke_connection();
    assert_eq!(final_drain.wait().await.unwrap(), receipt);
    f.stop().await;
}

#[tokio::test]
async fn quiesce_modern_queued_ground_jump_retires_on_one_tick_before_park() {
    use crate::client::adapter::ControlOps;
    use crate::client::control::GroundJumpStatus;
    let mut f = Fixture::new().await;
    let client = quiesce_client(&f);
    let record = ControlOps::start_control(&f.api, GameMode::Survival).await.unwrap();
    // Acquire state before the new ticker can sample. Request and quiesce run in
    // this same task without yielding after the request has queued its raw owner.
    ControlOps::request_ground_jump(&f.api, GameMode::Survival, record.session_id).await.unwrap();
    let mut q = client.begin_quiesce(client.generation_key()).unwrap();
    assert!(!q.admitted_drain().is_drained().unwrap());
    let first = timeout(Duration::from_secs(1), q.admitted_drain().wait()).await.unwrap().unwrap();
    let (id, _) = read_packet(&mut f.peer, None).await.unwrap();
    assert_eq!(id, ids::play_serverbound::PLAYER_INPUT);
    assert_eq!(read_packet(&mut f.peer, None).await.unwrap().0, ids::play_serverbound::POSITION_LOOK);
    let record = ControlOps::control_record(&f.api).await.unwrap().unwrap();
    assert_eq!(record.dispatched_ticks, 1);
    assert!(matches!(record.ground_jump.unwrap().status, GroundJumpStatus::Evaluated { dispatched: true, .. }));
    assert!(first.written_through().get() > 0);
    let target = q.stop_targets().await.unwrap().into_iter().find(|t| t.control_session_id() == Some(record.session_id)).unwrap();
    q.stop_permit().unwrap().neutralize(target).await.unwrap();
    q.seal_for_fence().unwrap().wait().await.unwrap();
    f.stop().await;
}

#[tokio::test]
async fn quiesce_modern_intentional_abort_waits_for_cleanup_and_can_drain() {
    use crate::client::StopTargetKind;
    let mut f = Fixture::new().await;
    f.start().await;
    let client = quiesce_client(&f);
    let mut q = client.begin_quiesce(client.generation_key()).unwrap();
    q.admitted_drain().wait().await.unwrap();
    let target = q.stop_targets().await.unwrap().into_iter().find(|t| t.kind() == StopTargetKind::Digging).unwrap();
    let writer = f.session.writer.lock().await;
    let mut permit = q.stop_permit().unwrap();
    let mut stop = Box::pin(permit.neutralize(target));
    assert!(timeout(Duration::from_millis(10), &mut stop).await.is_err());
    // Dropping a waiter after admission leaves the accepted worker alive.
    drop(stop); drop(permit);
    let final_drain = q.seal_for_fence().unwrap();
    assert!(!final_drain.is_drained().unwrap());
    drop(writer);
    let (id, payload) = read_packet(&mut f.peer, None).await.unwrap();
    assert_eq!(id, ids::play_serverbound::BLOCK_DIG); assert_eq!(payload[0], 1);
    assert!(!final_drain.is_drained().unwrap());
    let mut rest = &payload[10..]; let sequence = crate::protocol::get_varint(&mut rest).unwrap();
    let mut ack = Vec::new(); put_varint(&mut ack, sequence);
    f.receive(ids::play_clientbound::ACKNOWLEDGE_PLAYER_DIGGING, &ack).await;
    timeout(Duration::from_secs(1), final_drain.wait()).await.unwrap().unwrap();
    assert!(timeout(Duration::from_millis(20), read_packet(&mut f.peer, None)).await.is_err());
    f.stop().await;
}

#[tokio::test]
async fn quiesce_modern_final_seal_waits_for_stalled_teleport_pair() {
    let mut f = Fixture::new().await;
    let client = quiesce_client(&f);
    let mut q = client.begin_quiesce(client.generation_key()).unwrap(); q.admitted_drain().wait().await.unwrap();
    let writer = f.session.writer.lock().await;
    let mut position = vec![0];
    for v in [8f64, 65., 8., 0., 0., 0.] { position.extend(v.to_be_bytes()); }
    position.extend(0f32.to_be_bytes()); position.extend(0f32.to_be_bytes()); position.extend(0u32.to_be_bytes());
    write_packet(&mut f.peer, None, ids::play_clientbound::POSITION, &position).await.unwrap();
    timeout(Duration::from_secs(1), async {
        while f.session.state.try_lock().is_ok() { tokio::task::yield_now().await; }
    }).await.unwrap();
    let final_drain = q.seal_for_fence().unwrap(); assert!(!final_drain.is_drained().unwrap());
    drop(writer);
    assert_eq!(read_packet(&mut f.peer, None).await.unwrap().0, ids::play_serverbound::TELEPORT_CONFIRM);
    assert_eq!(read_packet(&mut f.peer, None).await.unwrap().0, ids::play_serverbound::POSITION_LOOK);
    let receipt = timeout(Duration::from_secs(1), final_drain.wait()).await.unwrap().unwrap();
    f.receive(ids::play_clientbound::POSITION, &position).await;
    assert_eq!(f.session.ledger.test_written_through(), receipt.written_through().get());
    assert!(timeout(Duration::from_millis(20), read_packet(&mut f.peer, None)).await.is_err());
    f.stop().await;
}
#[tokio::test]
async fn quiesce_modern_stale_target_mode_round_trip_latches_required_release_failure() {
    let mut f = Fixture::new().await;
    let client = quiesce_client(&f); client.survival().use_item(crate::client::Hand::Main).await.unwrap(); read_packet(&mut f.peer, None).await.unwrap();
    let mut q = client.begin_quiesce(client.generation_key()).unwrap(); q.admitted_drain().wait().await.unwrap();
    let target = q.stop_targets().await.unwrap().into_iter().find(|t| t.kind() == crate::client::StopTargetKind::ItemUse).unwrap();
    for mode in [1f32, 0.] { let mut p = vec![3]; p.extend(mode.to_be_bytes()); f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &p).await; }
    assert!(matches!(q.stop_permit().unwrap().neutralize(target).await, Err(crate::client::StopError::ContextChanged { .. })));
    assert_eq!(q.seal_for_fence().err().unwrap().kind(), crate::client::DrainErrorKind::RequiredReleaseFailed);
    assert!(timeout(Duration::from_millis(20), read_packet(&mut f.peer, None)).await.is_err());
    f.stop().await;
}
#[tokio::test]
async fn quiesce_modern_vehicle_neutral_preserves_mount_without_predicted_position() {
    let mut f = Fixture::new().await;
    f.receive(ids::play_clientbound::SET_PASSENGERS, &[12, 1, 42]).await;
    f.session.send(ids::play_serverbound::PLAYER_INPUT, &[1]).await.unwrap(); read_packet(&mut f.peer, None).await.unwrap();
    let client = quiesce_client(&f); let mut q = client.begin_quiesce(client.generation_key()).unwrap(); q.admitted_drain().wait().await.unwrap();
    let target = q.stop_targets().await.unwrap().into_iter().find(|t| t.kind() == crate::client::StopTargetKind::VehicleInput).unwrap();
    assert_eq!(target.mount().unwrap().native_vehicle_id(), 12);
    q.stop_permit().unwrap().neutralize(target).await.unwrap();
    assert_eq!(read_packet(&mut f.peer, None).await.unwrap(), (ids::play_serverbound::PLAYER_INPUT, vec![0]));
    assert_eq!(read_packet(&mut f.peer, None).await.unwrap(), (ids::play_serverbound::STEER_BOAT, vec![0, 0]));
    q.seal_for_fence().unwrap().wait().await.unwrap();
    assert!(timeout(Duration::from_millis(20), read_packet(&mut f.peer, None)).await.is_err());
    f.stop().await;
}

#[tokio::test]
async fn quiesce_modern_jump_waiting_for_producer_lock_loses_before_admission() {
    use crate::client::adapter::ControlOps;
    let mut f = Fixture::new().await;
    let client = quiesce_client(&f);
    let control = ControlOps::start_control(&f.api, GameMode::Survival).await.unwrap();
    let key = client.generation_key();
    let state = f.session.state.lock().await;
    let mut request = Box::pin(ControlOps::request_ground_jump(&f.api, GameMode::Survival, control.session_id));
    std::future::poll_fn(|cx| { assert!(request.as_mut().poll(cx).is_pending()); std::task::Poll::Ready(()) }).await;
    let mut q = client.begin_quiesce(key).unwrap();
    drop(state);
    assert_eq!(request.await.unwrap_err().kind(), ErrorKind::State);
    timeout(Duration::from_secs(1), q.admitted_drain().wait()).await.unwrap().unwrap();
    q.seal_for_fence().unwrap().wait().await.unwrap();
    assert!(timeout(Duration::from_millis(20), read_packet(&mut f.peer, None)).await.is_err());
    f.stop().await;
}

#[tokio::test]
async fn quiesce_modern_completed_common_owner_is_retired() {
    use crate::client::adapter::ControlOps;
    let f = Fixture::new().await;
    let record = ControlOps::start_control(&f.api, GameMode::Survival).await.unwrap();
    ControlOps::stop_control_for(&f.api, record.session_id).await.unwrap();
    let client = quiesce_client(&f); let mut q = client.begin_quiesce(client.generation_key()).unwrap();
    q.admitted_drain().wait().await.unwrap();
    assert!(q.stop_targets().await.unwrap().iter().all(|t| t.control_session_id() != Some(record.session_id)));
    q.seal_for_fence().unwrap().wait().await.unwrap();
    f.stop().await;
}

#[tokio::test]
async fn quiesce_modern_movement_mount_round_trip_cannot_stop_replacement_context() {
    use crate::client::adapter::ControlOps;
    let mut f = Fixture::new().await;
    let record = ControlOps::start_control(&f.api, GameMode::Survival).await.unwrap();
    let client = quiesce_client(&f); let mut q = client.begin_quiesce(client.generation_key()).unwrap(); q.admitted_drain().wait().await.unwrap();
    let target = q.stop_targets().await.unwrap().into_iter().find(|t| t.control_session_id() == Some(record.session_id)).unwrap();
    f.receive(ids::play_clientbound::SET_PASSENGERS, &[12, 1, 42]).await;
    f.receive(ids::play_clientbound::SET_PASSENGERS, &[12, 0]).await;
    assert!(matches!(q.stop_permit().unwrap().neutralize(target).await, Err(crate::client::StopError::ContextChanged { .. })));
    assert_eq!(q.seal_for_fence().err().unwrap().kind(), crate::client::DrainErrorKind::RequiredReleaseFailed);
    assert!(timeout(Duration::from_millis(20), read_packet(&mut f.peer, None)).await.is_err());
    f.stop().await;
}
