use super::*;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWrite};

#[tokio::test]
async fn common_dismount_requires_receipt_before_release_and_never_replays() {
    use crate::client::{
        GameMode,
        vehicle::{DismountStage, VehicleRelation},
    };
    for mode in [GameMode::Survival, GameMode::Creative] {
        let (session, api, mut peer) = common_ground_fixture(mode).await;
        let client = crate::Client::from_java_1_21_11(api.bot.clone());
        session
            .state
            .lock()
            .await
            .receive(ids::play_clientbound::SET_PASSENGERS, &[10, 1, 42], 256)
            .unwrap();
        let VehicleRelation::Mounted { mount } = client
            .vehicle_state()
            .await
            .unwrap()
            .relation
            .unwrap()
            .value
        else {
            panic!()
        };
        let wrong = if mode == GameMode::Survival {
            client.creative().dismount(mount).await
        } else {
            client.survival().dismount(mount).await
        };
        assert!(wrong.is_err());
        let record = if mode == GameMode::Survival {
            client.survival().dismount(mount).await.unwrap()
        } else {
            client.creative().dismount(mount).await.unwrap()
        };
        assert_eq!(record.stage, DismountStage::Submitted);
        assert_eq!(
            read_packet(&mut peer, None).await.unwrap(),
            (0x2a, vec![32])
        );
        let early = if mode == GameMode::Survival {
            client.survival().complete_dismount(record.id).await
        } else {
            client.creative().complete_dismount(record.id).await
        };
        assert!(early.is_err());
        assert!(
            !client
                .dismount_record()
                .await
                .unwrap()
                .unwrap()
                .release_claimed
        );
        let duplicate = if mode == GameMode::Survival {
            client.survival().dismount(mount).await
        } else {
            client.creative().dismount(mount).await
        };
        assert!(duplicate.is_err());
        session
            .state
            .lock()
            .await
            .receive(ids::play_clientbound::SET_PASSENGERS, &[10, 2, 43, 42], 256)
            .unwrap();
        assert_eq!(
            client.dismount_record().await.unwrap().unwrap().stage,
            DismountStage::Submitted
        );
        session
            .state
            .lock()
            .await
            .receive(ids::play_clientbound::SET_PASSENGERS, &[11, 0], 256)
            .unwrap();
        assert_eq!(
            client.dismount_record().await.unwrap().unwrap().stage,
            DismountStage::Submitted
        );
        session
            .state
            .lock()
            .await
            .receive(ids::play_clientbound::SET_PASSENGERS, &[10, 1, 43], 256)
            .unwrap();
        assert_eq!(
            client.dismount_record().await.unwrap().unwrap().stage,
            DismountStage::ObservedUnmounted
        );
        let done = if mode == GameMode::Survival {
            client
                .survival()
                .complete_dismount(record.id)
                .await
                .unwrap()
        } else {
            client
                .creative()
                .complete_dismount(record.id)
                .await
                .unwrap()
        };
        assert_eq!(done.stage, DismountStage::Completed);
        assert_eq!(read_packet(&mut peer, None).await.unwrap(), (0x2a, vec![0]));
        let duplicate = if mode == GameMode::Survival {
            client.survival().complete_dismount(record.id).await
        } else {
            client.creative().complete_dismount(record.id).await
        };
        assert!(duplicate.is_err());
        session.stop();
        assert_eq!(
            client.dismount_record().await.unwrap().unwrap().stage,
            DismountStage::Completed
        );
        assert!(
            timeout(Duration::from_millis(30), read_packet(&mut peer, None))
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn cancelled_dismount_caller_keeps_one_owner_and_readable_intent() {
    use crate::client::{
        GameMode,
        vehicle::{DismountStage, VehicleRelation},
    };
    let (session, api, mut peer) = common_ground_fixture(GameMode::Survival).await;
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    session
        .state
        .lock()
        .await
        .receive(ids::play_clientbound::SET_PASSENGERS, &[10, 1, 42], 256)
        .unwrap();
    let VehicleRelation::Mounted { mount } = client
        .vehicle_state()
        .await
        .unwrap()
        .relation
        .unwrap()
        .value
    else {
        panic!()
    };
    let writer = session.writer.lock().await;
    let ops = client.survival();
    let mut attempt = Box::pin(ops.dismount(mount));
    assert!(
        timeout(Duration::from_millis(20), attempt.as_mut())
            .await
            .is_err()
    );
    drop(attempt);
    let record = timeout(Duration::from_millis(50), client.dismount_record())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(record.stage, DismountStage::Prepared);
    assert!(!record.request_dispatched);
    drop(writer);
    assert_eq!(
        read_packet(&mut peer, None).await.unwrap(),
        (0x2a, vec![32])
    );
    assert!(client.survival().dismount(mount).await.is_err());
    assert!(
        timeout(Duration::from_millis(30), read_packet(&mut peer, None))
            .await
            .is_err()
    );
    session
        .state
        .lock()
        .await
        .receive(ids::play_clientbound::SET_PASSENGERS, &[10, 0], 256)
        .unwrap();
    client
        .survival()
        .complete_dismount(record.id)
        .await
        .unwrap();
    assert_eq!(read_packet(&mut peer, None).await.unwrap(), (0x2a, vec![0]));
}

async fn fixture() -> (Arc<Session>, OwnedReadHalf, TcpStream) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (peer, _) = listener.accept().await.unwrap();
    let (reader, writer) = client.into_split();
    let session = Arc::new(Session {
        id: 42,
        started: Instant::now(),
        writer: Mutex::new(Writer {
            stream: writer,
            compression: None,
        }),
        state: Mutex::new(State::default()),
        changed: Notify::new(),
        cancel: Notify::new(),
        stopped: AtomicBool::new(false),
        interrupted_packet: AtomicI32::new(-1),
        limits: crate::client::ClientLimits::default(),
        interaction_sequence: AtomicI32::new(0),
    });
    (session, reader, peer)
}

async fn pending<F: Future>(mut future: Pin<&mut F>) {
    std::future::poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}

fn operations(session: &Arc<Session>) -> operations::Operations {
    operations::Operations {
        bot: Bot {
            crafting_take_history: {
                let state = session.state.try_lock().expect("new session");
                state.crafting_take_history.clone()
            },
            flight_history: {
                let state = session.state.try_lock().expect("new session");
                state.flight_history.clone()
            },
            dismount_history: {
                let state = session.state.try_lock().expect("new session");
                state.dismount_history.clone()
            },
            close_history: session
                .state
                .try_lock()
                .expect("new session")
                .close_history
                .clone(),
            session: session.clone(),
            _lease: Arc::new(Lease(Arc::downgrade(session))),
        },
    }
}

#[tokio::test]
async fn completed_frames_decode_with_single_byte_fragmentation_and_compression() {
    let (session, _, _) = fixture().await;
    for compression in [None, Some(0), Some(256)] {
        let (mut writer, mut reader) = tokio::io::duplex(1);
        let sender = async {
            for id in [1, 2] {
                session
                    .write_frame(&mut writer, compression, id, &[42; 512])
                    .await
                    .unwrap();
            }
        };
        let receiver = async {
            for id in [1, 2] {
                let result = read_packet(&mut reader, compression).await.unwrap();
                assert_eq!(result, (id, vec![42; 512]));
            }
        };
        timeout(Duration::from_secs(2), async {
            tokio::join!(sender, receiver);
        })
        .await
        .unwrap();
        assert!(!session.stopped.load(Ordering::Acquire));
        assert_eq!(session.interrupted_packet.load(Ordering::Acquire), -1);
    }
}

#[tokio::test]
async fn cancelling_lock_wait_does_not_poison_or_send() {
    let (session, _, mut peer) = fixture().await;
    let locked = session.writer.lock().await;
    let mut first = Box::pin(session.send(1, &[2]));
    pending(first.as_mut()).await;
    drop(first);
    assert!(!session.stopped.load(Ordering::Acquire));
    assert_eq!(session.interrupted_packet.load(Ordering::Acquire), -1);
    drop(locked);
    session.send(3, &[4]).await.unwrap();
    assert_eq!(read_packet(&mut peer, None).await.unwrap(), (3, vec![4]));
}

#[tokio::test]
async fn cancelled_partial_frame_prevents_queued_requests_and_automatic_response_and_closes_tcp() {
    let (session, reader, mut peer) = fixture().await;
    let running = session.clone();
    let receiver = tokio::spawn(async move {
        running.run_receiver(reader).await;
    });
    write_packet(
        &mut peer,
        None,
        ids::configuration_clientbound::PING,
        &[0; 4],
    )
    .await
    .unwrap();
    assert_eq!(
        read_packet(&mut peer, None).await.unwrap().0,
        ids::configuration_serverbound::PONG
    );
    // Keep the real writer lock, but deterministically interrupt its common
    // framing helper on a one-byte duplex stream instead of saturating TCP.
    let locked = session.writer.lock().await;
    let (mut limited, mut received) = tokio::io::duplex(1);
    let mut attempt = Box::pin(session.write_frame(&mut limited, None, 5, &[0; 100]));
    pending(attempt.as_mut()).await;
    let mut queued = Box::pin(session.send(6, &[7]));
    pending(queued.as_mut()).await;
    write_packet(
        &mut peer,
        None,
        ids::configuration_clientbound::KEEP_ALIVE,
        &[0; 8],
    )
    .await
    .unwrap();
    // Receiving the request proves the automatic response will use the same
    // writer queue. It must not append bytes after the interrupted helper.
    timeout(Duration::from_secs(1), async {
        loop {
            if session.state.lock().await.sequence == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(attempt);
    assert_eq!(session.interrupted_packet.load(Ordering::Acquire), 5);
    drop(locked);
    assert_eq!(
        queued.await.unwrap_err().kind(),
        ErrorKind::UncertainDispatch
    );
    assert_eq!(
        session.send(8, &[]).await.unwrap_err().kind(),
        ErrorKind::UncertainDispatch
    );
    timeout(Duration::from_secs(1), receiver)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        timeout(Duration::from_secs(1), peer.read_u8())
            .await
            .unwrap()
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::UnexpectedEof
    );
    drop(limited);
    let mut bytes = vec![];
    received.read_to_end(&mut bytes).await.unwrap();
    assert_eq!(bytes, vec![101]); // original frame prefix only; no next packet
}

struct FailingWriter {
    bytes: Vec<u8>,
    limit: usize,
    fail_flush: bool,
}
impl AsyncWrite for FailingWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        input: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.bytes.len() == self.limit {
            return Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into()));
        }
        let count = input.len().min(self.limit - self.bytes.len());
        self.bytes.extend_from_slice(&input[..count]);
        Poll::Ready(Ok(count))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(if self.fail_flush {
            Err(std::io::ErrorKind::BrokenPipe.into())
        } else {
            Ok(())
        })
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn write_or_flush_error_preserves_first_uncertainty_and_never_appends() {
    for (limit, fail_flush) in [(0, false), (2, false), (100, true)] {
        let (session, _, _) = fixture().await;
        let mut writer = FailingWriter {
            bytes: vec![],
            limit,
            fail_flush,
        };
        assert_eq!(
            session
                .write_frame(&mut writer, None, 9, &[10; 8])
                .await
                .unwrap_err()
                .kind(),
            ErrorKind::UncertainDispatch
        );
        let before = writer.bytes.clone();
        assert_eq!(
            session
                .write_frame(&mut writer, None, 11, &[])
                .await
                .unwrap_err()
                .kind(),
            ErrorKind::UncertainDispatch
        );
        assert_eq!(writer.bytes, before);
        assert_eq!(session.interrupted_packet.load(Ordering::Acquire), 9);
        assert!(session.stopped.load(Ordering::Acquire));
    }
}

#[tokio::test]
async fn explicit_disconnect_interrupts_a_blocked_frame_and_retains_inventory_history() {
    let (session, reader, mut peer) = fixture().await;
    let api = operations(&session);
    {
        let mut state = session.state.lock().await;
        state.ready = true;
        state.sequence = 3;
        state.loading = loading::InteractionLoading::completed_fixture();
        state.operations.reset_world(0).unwrap();
        let mut full = vec![0, 1, 46];
        for slot in 0..46 {
            if slot == 9 {
                full.push(1);
                put_varint(
                    &mut full,
                    operations::default_item("stone", 1).unwrap().item_id,
                );
                full.extend([0, 0]);
            } else {
                full.push(0);
            }
        }
        full.push(0);
        operations::receive(&mut state, ids::play_clientbound::WINDOW_ITEMS, &full).unwrap();
    }
    let running = session.clone();
    let receiver = tokio::spawn(async move {
        running.run_receiver(reader).await;
    });
    let locked = session.writer.lock().await;
    let mut swap_attempt = Box::pin(api.swap_player_hotbar(9, 0));
    pending(swap_attempt.as_mut()).await;
    drop(swap_attempt);
    let swap = api
        .operation_history()
        .await
        .pending_inventory_swap
        .unwrap();
    let (mut limited, _limited_reader) = tokio::io::duplex(1);
    let mut attempt = Box::pin(session.write_frame(
        &mut limited,
        None,
        ids::play_serverbound::WINDOW_CLICK,
        &[0; 100],
    ));
    pending(attempt.as_mut()).await;
    session.stop();
    assert_eq!(
        attempt.await.unwrap_err().kind(),
        ErrorKind::UncertainDispatch
    );
    drop(locked);
    timeout(Duration::from_secs(1), receiver)
        .await
        .unwrap()
        .unwrap();
    assert!(peer.read_u8().await.is_err());
    assert_eq!(
        api.player_state().await.unwrap_err().kind(),
        ErrorKind::UncertainDispatch
    );
    let history = api.operation_history().await;
    assert!(history.connection_closed);
    assert_eq!(
        history.interrupted_packet_id,
        Some(ids::play_serverbound::WINDOW_CLICK)
    );
    assert_eq!(history.pending_inventory_swap, Some(swap));
    assert!(api.select_hotbar(0).await.is_err());
    assert!(
        api.operation_history()
            .await
            .pending_inventory_swap
            .is_some()
    );
}

#[tokio::test]
async fn creative_intent_is_retained_when_submission_is_cancelled_before_writer_acquisition() {
    let (session, _, _) = fixture().await;
    let api = operations(&session);
    {
        let mut state = session.state.lock().await;
        state.ready = true;
        state.operations.reset_world(1).unwrap();
        state.loading = loading::InteractionLoading::completed_fixture();
    }
    let locked = session.writer.lock().await;
    let mut attempt = Box::pin(api.set_creative_hotbar(0, Some(("stone", 1))));
    pending(attempt.as_mut()).await;
    drop(attempt);
    drop(locked);
    session.stop();
    let history = api.operation_history().await;
    assert!(history.connection_closed);
    assert_eq!(history.interrupted_packet_id, None);
    assert_eq!(history.pending_creative_slots, vec![0]);
}

#[tokio::test]
async fn position_attempt_retains_receipt_and_cancelled_dispatch_blocks_next_mutation() {
    use operations::{PositionBasis, ReceivedPose};
    let (session, _, mut peer) = fixture().await;
    let api = operations(&session);
    {
        let mut s = session.state.lock().await;
        s.phase = Phase::Play;
        s.ready = true;
        s.loading = loading::InteractionLoading::completed_fixture();
        s.position = Some([0.5, 1.0, 0.5]);
        s.operations.reset_world(1).unwrap();
        let mut abilities = vec![4];
        abilities.extend(0.05f32.to_be_bytes());
        abilities.extend(0.1f32.to_be_bytes());
        operations::receive(&mut s, ids::play_clientbound::ABILITIES, &abilities).unwrap();
        let generation = s.loading.generation;
        s.motion.receive(ReceivedPose {
            generation,
            receive_sequence: 5,
            position: [0.5, 1.0, 0.5],
            rotation: [0.0; 2],
            velocity: Some([0.0; 3]),
        });
    }
    api.set_flying(true).await.unwrap();
    assert_eq!(
        read_packet(&mut peer, None).await.unwrap().0,
        ids::play_serverbound::ABILITIES
    );
    api.move_flying([1.5, 1.0, 0.5], [0.0; 2]).await.unwrap();
    assert_eq!(
        read_packet(&mut peer, None).await.unwrap().0,
        ids::play_serverbound::POSITION_LOOK
    );
    let sent = api.player_state().await.unwrap();
    assert!(!sent.position_from_server);
    assert_eq!(sent.motion.position_basis, PositionBasis::Submitted);
    assert_eq!(sent.motion.received_pose.unwrap().position, [0.5, 1.0, 0.5]);
    assert!(sent.motion.last_submission.unwrap().dispatched);
    let locked = session.writer.lock().await;
    let mut request = Box::pin(api.move_flying([2.5, 1.0, 0.5], [0.0; 2]));
    pending(request.as_mut()).await;
    drop(request);
    drop(locked);
    let history = api.operation_history().await;
    assert_eq!(
        history.motion.position_basis,
        PositionBasis::PendingSubmission
    );
    assert!(!history.motion.last_submission.as_ref().unwrap().dispatched);
    assert_eq!(history.motion.last_submission.unwrap().attempt_id, 2);
    assert!(api.select_hotbar(1).await.is_err());
    assert!(api.move_flying([2.5, 1.0, 0.5], [0.0; 2]).await.is_err());
    assert!(
        timeout(Duration::from_millis(10), read_packet(&mut peer, None))
            .await
            .is_err()
    );
    {
        let mut s = session.state.lock().await;
        let generation = s.loading.generation;
        s.position = Some([0.5, 1.0, 0.5]);
        s.motion.receive(ReceivedPose {
            generation,
            receive_sequence: 9,
            position: [0.5, 1.0, 0.5],
            rotation: [0.0; 2],
            velocity: Some([0.0; 3]),
        });
    }
    let restored = api.player_state().await.unwrap();
    assert!(restored.position_from_server);
    assert_eq!(
        restored.motion.last_submission.unwrap().superseded_at,
        Some(9)
    );
    {
        let mut s = session.state.lock().await;
        s.loading.reset(10);
        s.motion.invalidate(10, "world generation changed");
    }
    let reset = api.player_state().await.unwrap();
    assert!(!reset.position_from_server);
    assert_eq!(reset.motion.received_pose.unwrap().receive_sequence, 9);
    assert_eq!(reset.motion.position_basis, PositionBasis::Unavailable);
}

#[tokio::test]
async fn both_common_modes_target_every_audited_native_storage_state_without_dispatch() {
    let (session, _, mut peer) = fixture().await;
    let api = operations(&session);
    {
        let mut state = session.state.lock().await;
        state.identity = Some(LoginIdentity {
            uuid: [1; 16],
            name: "StorageTargetProbe".into(),
            server: crate::Server::default(),
        });
        state.phase = Phase::Play;
        state.ready = true;
        state.sequence = 10;
        state.loading = loading::InteractionLoading::completed_fixture();
        state.operations.reset_world(0).unwrap();
        state.operations.local_player = operations::LocalPlayerState::spawned(42);
        state.operations.local_player.velocity = Some(operations::VelocitySample {
            value: [0.0; 3],
            receive_sequence: 10,
        });
        state.operations.local_player.health = Some(operations::PlayerHealth {
            health: 20.0,
            food: 20,
            saturation: 5.0,
            receive_sequence: 10,
        });
        state.world.select_dimension(
            "minecraft:overworld".into(),
            Dimension::new(-64, 384).unwrap(),
        );
        state.position = Some([8.5, 65.0, 8.5]);
        let generation = state.loading.generation;
        state.motion.receive(operations::ReceivedPose {
            generation,
            receive_sequence: 10,
            position: [8.5, 65.0, 8.5],
            rotation: [0.0; 2],
            velocity: Some([0.0; 3]),
        });
        for x in 0..16 {
            for y in 63..72 {
                for z in 0..16 {
                    state
                        .world
                        .seed_replay_cell([x, y, z], if y == 64 { 1 } else { 0 });
                }
            }
        }
    }
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    for (mode, value) in [
        (crate::client::GameMode::Survival, 0.0f32),
        (crate::client::GameMode::Creative, 1.0f32),
    ] {
        let mut packet = vec![3];
        packet.extend(value.to_be_bytes());
        {
            let mut state = session.state.lock().await;
            operations::receive(
                &mut state,
                ids::play_clientbound::GAME_STATE_CHANGE,
                &packet,
            )
            .unwrap();
        }
        for state in
            crate::client::tests::common_storage_target_states(crate::MinecraftVersion::Java1_21_11)
        {
            let id = crate::versions::java_1_21_11::state_id(&state).unwrap();
            session
                .state
                .lock()
                .await
                .world
                .seed_replay_cell([8, 66, 11], id);
            crate::client::tests::common_storage_target_scenario(&client, mode, &state).await;
        }
    }
    assert!(
        timeout(Duration::from_millis(30), read_packet(&mut peer, None))
            .await
            .is_err()
    );
    let mut abilities = vec![6];
    abilities.extend(0.05f32.to_be_bytes());
    abilities.extend(0.1f32.to_be_bytes());
    operations::receive(
        &mut *session.state.lock().await,
        ids::play_clientbound::ABILITIES,
        &abilities,
    )
    .unwrap();
    assert!(client.creative().target_block(4.5).await.is_err());
}

#[tokio::test]
async fn common_motion_uses_modern_rules_and_retained_connection_owned_dispatch() {
    let (session, api, mut peer) = common_ground_fixture(crate::client::GameMode::Survival).await;
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    crate::client::tests::common_motion_preview_scenario(&client).await;
    crate::client::tests::common_target_scenario(&client).await;
    let (id, bytes) = read_packet(&mut peer, None).await.unwrap();
    assert_eq!(id, ids::play_serverbound::LOOK);
    assert_eq!(bytes.len(), 9);
    assert!(
        timeout(Duration::from_millis(30), read_packet(&mut peer, None))
            .await
            .is_err()
    );
    crate::client::tests::common_motion_dispatch_scenario(&client).await;
    for _ in 0..37 {
        let (input_id, input) = read_packet(&mut peer, None).await.unwrap();
        assert_eq!(input_id, ids::play_serverbound::PLAYER_INPUT);
        assert_eq!(input.len(), 1);
        let (pose_id, pose) = read_packet(&mut peer, None).await.unwrap();
        assert_eq!(pose_id, ids::play_serverbound::POSITION_LOOK);
        assert_eq!(pose.len(), 33);
    }
    let prior = client.survival().motion_record().await.unwrap().unwrap();
    api.start_predicted_survival_path(
        &[crate::client::survival::SurvivalControl {
            yaw: 0.0,
            input: Default::default(),
        }; 2],
    )
    .await
    .unwrap();
    let retired = client.survival().motion_record().await.unwrap().unwrap();
    assert_eq!(retired.run_id, prior.run_id);
    assert_eq!(retired.dispatched_ticks, prior.dispatched_ticks);
    assert_eq!(
        retired.status,
        crate::client::survival::MotionStatus::RequiresInspection
    );
    assert!(retired.problem.unwrap().contains("superseded"));
    for _ in 0..2 {
        read_packet(&mut peer, None).await.unwrap();
        read_packet(&mut peer, None).await.unwrap();
    }
    {
        let mut state = session.state.lock().await;
        let mut mode = vec![3];
        mode.extend(1.0f32.to_be_bytes());
        state.sequence += 1;
        operations::receive(&mut state, ids::play_clientbound::GAME_STATE_CHANGE, &mode).unwrap();
    }
    assert!(
        client
            .survival()
            .preview_path(&[crate::client::survival::SurvivalControl {
                yaw: 0.0,
                input: Default::default(),
            }])
            .await
            .is_err()
    );
    assert_eq!(
        client
            .survival()
            .motion_record()
            .await
            .unwrap()
            .unwrap()
            .status,
        crate::client::survival::MotionStatus::RequiresInspection
    );
}

#[tokio::test]
async fn common_creative_contract_dispatches_modern_packets_without_inventory_echo() {
    let (session, _, mut peer) = fixture().await;
    let api = operations(&session);
    {
        let mut state = session.state.lock().await;
        state.phase = Phase::Play;
        state.ready = true;
        state.loading = loading::InteractionLoading::completed_fixture();
        state.operations.reset_world(1).unwrap();
        state.world.select_dimension(
            "minecraft:overworld".into(),
            Dimension::new(-64, 384).unwrap(),
        );
        state.world.seed_replay_cell([0, 0, 1], 1);
        state.position = Some([0.5, 1.0, 0.5]);
        state.motion.receive(operations::ReceivedPose {
            generation: 1,
            receive_sequence: 1,
            position: [0.5, 1.0, 0.5],
            rotation: [0.0; 2],
            velocity: Some([0.0; 3]),
        });
        let mut health = 20.0f32.to_be_bytes().to_vec();
        put_varint(&mut health, 20);
        health.extend(5.0f32.to_be_bytes());
        operations::receive(&mut state, ids::play_clientbound::UPDATE_HEALTH, &health).unwrap();
        let mut abilities = vec![4];
        abilities.extend(0.05f32.to_be_bytes());
        abilities.extend(0.1f32.to_be_bytes());
        operations::receive(&mut state, ids::play_clientbound::ABILITIES, &abilities).unwrap();
    }
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    crate::client::tests::common_creative_scenario(&client).await;
    let mut emitted = Vec::new();
    for _ in 0..7 {
        emitted.push(
            tokio::time::timeout(Duration::from_secs(1), read_packet(&mut peer, None))
                .await
                .unwrap()
                .unwrap(),
        );
    }
    assert_eq!(
        emitted.iter().map(|p| p.0).collect::<Vec<_>>(),
        [
            ids::play_serverbound::SET_CREATIVE_SLOT,
            ids::play_serverbound::LOOK,
            ids::play_serverbound::HELD_ITEM_SLOT,
            ids::play_serverbound::ABILITIES,
            ids::play_serverbound::POSITION_LOOK,
            ids::play_serverbound::BLOCK_DIG,
            ids::play_serverbound::BLOCK_PLACE
        ]
    );
    assert_eq!(&emitted[0].1[..2], &36i16.to_be_bytes());
    assert_eq!(emitted[3].1, [2]);
    let mut slot = vec![0, 1];
    put_varint(
        &mut slot,
        operations::default_item("stone", 1).unwrap().item_id,
    );
    slot.extend([0, 0]);
    {
        let mut state = session.state.lock().await;
        state.sequence = 8;
        operations::receive(
            &mut state,
            ids::play_clientbound::SET_PLAYER_INVENTORY,
            &slot,
        )
        .unwrap();
    }
    let state = client.player_state().await.unwrap();
    assert!(
        matches!(&state.inventory.slots[36], Some(value) if matches!(&value.value,
        crate::client::SlotKnowledge::Item { item } if item.name == "minecraft:stone"))
    );
    assert!(!state.pending_dispatch);
}

#[tokio::test]
async fn common_cursor_provenance_does_not_advance_on_unrelated_slot_packets() {
    let (session, _, _) = fixture().await;
    let api = operations(&session);
    {
        let mut state = session.state.lock().await;
        state.sequence = 10;
        operations::receive(&mut state, ids::play_clientbound::SET_CURSOR_ITEM, &[0]).unwrap();
        state.sequence = 20;
        operations::receive(
            &mut state,
            ids::play_clientbound::SET_PLAYER_INVENTORY,
            &[0, 0],
        )
        .unwrap();
    }
    let state = api.common_player_state().await.unwrap();
    assert_eq!(
        state.inventory.cursor.unwrap().source,
        crate::client::ValueSource::Received { sequence: 10 }
    );
    assert_eq!(
        state.inventory.slots[36].as_ref().unwrap().source,
        crate::client::ValueSource::Received { sequence: 20 }
    );
}

async fn common_ground_fixture(
    mode: crate::client::GameMode,
) -> (Arc<Session>, operations::Operations, TcpStream) {
    let (session, _, peer) = fixture().await;
    let api = operations(&session);
    {
        let mut state = session.state.lock().await;
        state.identity = Some(LoginIdentity {
            uuid: [1; 16],
            name: "CommonProbe".into(),
            server: crate::Server::default(),
        });
        state.phase = Phase::Play;
        state.sequence = 10;
        state.ready = true;
        state.loading = loading::InteractionLoading::completed_fixture();
        state.operations.reset_world(0).unwrap();
        let mut packet = vec![3];
        packet.extend(
            (if mode == crate::client::GameMode::Creative {
                1f32
            } else {
                0f32
            })
            .to_be_bytes(),
        );
        operations::receive(
            &mut state,
            ids::play_clientbound::GAME_STATE_CHANGE,
            &packet,
        )
        .unwrap();
        state.operations.local_player = operations::LocalPlayerState::spawned(42);
        state.operations.local_player.velocity = Some(operations::VelocitySample {
            value: [0.0; 3],
            receive_sequence: 10,
        });
        state.operations.local_player.health = Some(operations::PlayerHealth {
            health: 20.0,
            food: 20,
            saturation: 5.0,
            receive_sequence: 10,
        });
        state.world.select_dimension(
            "minecraft:overworld".into(),
            Dimension::new(-64, 384).unwrap(),
        );
        state.position = Some([8.5, 65.0, 8.5]);
        let generation = state.loading.generation;
        state.motion.receive(operations::ReceivedPose {
            generation,
            receive_sequence: 10,
            position: [8.5, 65.0, 8.5],
            rotation: [0.0; 2],
            velocity: Some([0.0; 3]),
        });
        for x in 0..16 {
            for y in 63..72 {
                for z in 0..16 {
                    state
                        .world
                        .seed_replay_cell([x, y, z], if y == 64 { 1 } else { 0 });
                }
            }
        }
    }
    (session, api, peer)
}

#[tokio::test]
async fn creative_ground_motion_keeps_mode_and_prediction_contract() {
    use crate::client::{
        GameMode,
        survival::{MotionStatus, SurvivalControl},
    };
    let (session, api, mut peer) = common_ground_fixture(GameMode::Creative).await;
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    let controls = [SurvivalControl {
        yaw: 0.0,
        input: Default::default(),
    }; 2];
    assert!(client.survival().preview_path(&controls).await.is_err());
    assert!(
        client
            .survival()
            .start_predicted_path(&controls)
            .await
            .is_err()
    );
    assert!(api.start_predicted_survival_path(&controls).await.is_err());
    let preview = client.creative().preview_path(&controls).await.unwrap();
    assert_eq!(preview.initial.game_mode, Some(GameMode::Creative));
    let sent = client
        .creative()
        .start_predicted_path(&controls)
        .await
        .unwrap();
    for _ in 0..2 {
        assert_eq!(
            read_packet(&mut peer, None).await.unwrap().0,
            ids::play_serverbound::PLAYER_INPUT
        );
        assert_eq!(
            read_packet(&mut peer, None).await.unwrap().0,
            ids::play_serverbound::POSITION_LOOK
        );
    }
    let completed = timeout(Duration::from_secs(2), async {
        loop {
            let r = client.creative().motion_record().await.unwrap().unwrap();
            if r.status != MotionStatus::Running {
                break r;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(completed.run_id, sent.run_id);
    assert_eq!(completed.status, MotionStatus::Predicted);
    assert_eq!(
        completed.preview.initial.received_pose,
        preview.initial.received_pose
    );
    {
        let mut packet = vec![3];
        packet.extend(0f32.to_be_bytes());
        operations::receive(
            &mut *session.state.lock().await,
            ids::play_clientbound::GAME_STATE_CHANGE,
            &packet,
        )
        .unwrap();
    }
    assert_eq!(
        client
            .creative()
            .motion_record()
            .await
            .unwrap()
            .unwrap()
            .status,
        MotionStatus::RequiresInspection
    );
    {
        let mut packet = vec![3];
        packet.extend(1f32.to_be_bytes());
        operations::receive(
            &mut *session.state.lock().await,
            ids::play_clientbound::GAME_STATE_CHANGE,
            &packet,
        )
        .unwrap();
    }
    assert!(
        client
            .creative()
            .start_predicted_path(&controls)
            .await
            .is_err()
    );
    assert!(
        timeout(Duration::from_millis(30), read_packet(&mut peer, None))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn common_entity_lifetime_rejects_reused_id_and_wrong_mode() {
    use crate::client::{GameMode, Hand};
    for mode in [GameMode::Survival, GameMode::Creative] {
        let (session, api, mut peer) = common_ground_fixture(mode).await;
        let client = crate::Client::from_java_1_21_11(api.bot.clone());
        let mut spawn = vec![43];
        spawn.extend([7; 16]);
        put_varint(
            &mut spawn,
            client
                .registry()
                .builtin_id("minecraft:entity_type", "minecraft:sheep")
                .unwrap()
                .value(),
        );
        for v in [9.0f64, 65.0, 8.5] {
            spawn.extend(v.to_be_bytes());
        }
        spawn.extend([0; 5]); // zero compact velocity, three angles, zero object data.
        session
            .state
            .lock()
            .await
            .receive(ids::play_clientbound::SPAWN_ENTITY, &spawn, 256)
            .unwrap();
        let identity = client.connection_identity().await.unwrap();
        assert_eq!(identity.uuid, [1; 16]);
        assert_eq!(identity.name, "CommonProbe");
        let observed = client.entity_spawns().await.unwrap();
        let target = observed.entities[0].id;
        assert_eq!(identity.session, observed.session);
        assert_eq!(
            observed.entities[0].type_name.as_deref(),
            Some("minecraft:sheep")
        );
        let (ops, wrong) = match mode {
            GameMode::Survival => (
                crate::client::entity::EntityAction::Attack { sneaking: false },
                client.creative().attack_entity(target, false).await,
            ),
            _ => (
                crate::client::entity::EntityAction::Attack { sneaking: false },
                client.survival().attack_entity(target, false).await,
            ),
        };
        assert!(wrong.is_err());
        client
            .execute(mode, crate::client::operations::Action::Entity(target, ops))
            .await
            .unwrap();
        assert_eq!(
            read_packet(&mut peer, None).await.unwrap(),
            (ids::play_serverbound::USE_ENTITY, vec![43, 1, 0])
        );
        session
            .state
            .lock()
            .await
            .receive(ids::play_clientbound::ENTITY_DESTROY, &[1, 43], 256)
            .unwrap();
        assert!(client.entity_spawns().await.unwrap().entities.is_empty());
        assert!(
            client
                .execute(mode, crate::client::operations::Action::Entity(target, ops))
                .await
                .is_err()
        );
        session
            .state
            .lock()
            .await
            .receive(ids::play_clientbound::SPAWN_ENTITY, &spawn, 256)
            .unwrap();
        let next = client.entity_spawns().await.unwrap().entities[0].id;
        assert_ne!(next, target);
        assert!(
            client
                .execute(mode, crate::client::operations::Action::Entity(target, ops))
                .await
                .is_err()
        );
        match mode {
            GameMode::Survival => {
                client
                    .survival()
                    .interact_entity(next, Hand::Off, true)
                    .await
                    .unwrap();
            }
            _ => {
                client
                    .creative()
                    .interact_entity(next, Hand::Off, true)
                    .await
                    .unwrap();
            }
        }
        assert_eq!(
            read_packet(&mut peer, None).await.unwrap(),
            (ids::play_serverbound::USE_ENTITY, vec![43, 0, 1, 1])
        );
        // Cancellation before the writer is acquired cannot emit a packet.
        let writer = session.writer.lock().await;
        let mut attempt =
            Box::pin(client.execute(mode, crate::client::operations::Action::Entity(next, ops)));
        pending(attempt.as_mut()).await;
        drop(attempt);
        drop(writer);
        assert_eq!(session.interrupted_packet.load(Ordering::Acquire), -1);
        assert!(
            timeout(Duration::from_millis(10), peer.read_u8())
                .await
                .is_err()
        );
        // A world reset retires all common targets even if raw IDs reappear.
        session.state.lock().await.entities.clear();
        assert!(
            client
                .execute(mode, crate::client::operations::Action::Entity(next, ops))
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn common_vehicle_receipts_refuse_stale_ground_authority() {
    use crate::client::{GameMode, survival::SurvivalControl, vehicle::VehicleRelation};
    for mode in [GameMode::Survival, GameMode::Creative] {
        let (session, api, mut peer) = common_ground_fixture(mode).await;
        let client = crate::Client::from_java_1_21_11(api.bot.clone());
        let controls = [SurvivalControl {
            yaw: 0.0,
            input: Default::default(),
        }];
        client.preview_motion_path(mode, &controls).await.unwrap();
        assert!(client.vehicle_state().await.unwrap().relation.is_none());
        session
            .state
            .lock()
            .await
            .receive(ids::play_clientbound::SET_PASSENGERS, &[10, 1, 42], 256)
            .unwrap();
        let mounted = client.vehicle_state().await.unwrap();
        let VehicleRelation::Mounted { mount } = mounted.relation.as_ref().unwrap().value else {
            panic!()
        };
        assert_eq!(mount.session(), mounted.session);
        assert_eq!(mount.native_vehicle_id(), 10);
        assert!(mount.vehicle().is_none());
        assert!(client.preview_motion_path(mode, &controls).await.is_err());
        session
            .state
            .lock()
            .await
            .receive(ids::play_clientbound::SET_PASSENGERS, &[11, 0], 256)
            .unwrap();
        assert_eq!(
            client
                .vehicle_state()
                .await
                .unwrap()
                .relation
                .unwrap()
                .value,
            VehicleRelation::Mounted { mount }
        );
        session
            .state
            .lock()
            .await
            .receive(ids::play_clientbound::SET_PASSENGERS, &[10, 0], 256)
            .unwrap();
        let unmounted = client.vehicle_state().await.unwrap();
        assert_eq!(
            unmounted.relation.unwrap().value,
            VehicleRelation::Unmounted {
                previous_mount: mount
            }
        );
        assert!(unmounted.passengers.unwrap().value.is_empty());
        assert!(client.preview_motion_path(mode, &controls).await.is_err());
        session
            .state
            .lock()
            .await
            .receive(ids::play_clientbound::ENTITY_DESTROY, &[1, 10], 256)
            .unwrap();
        assert!(client.vehicle_state().await.unwrap().relation.is_none());
        assert!(client.preview_motion_path(mode, &controls).await.is_err());
        assert!(
            timeout(Duration::from_millis(30), read_packet(&mut peer, None))
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn malformed_passenger_frame_preserves_relation_and_terminates_receive() {
    use crate::client::{GameMode, vehicle::VehicleRelation};
    for payload in [&[10, 0, 0][..], &[10, 2, 42, 42]] {
        let (session, api, _) = common_ground_fixture(GameMode::Survival).await;
        let client = crate::Client::from_java_1_21_11(api.bot.clone());
        session
            .state
            .lock()
            .await
            .receive(ids::play_clientbound::SET_PASSENGERS, &[10, 1, 42], 256)
            .unwrap();
        let before = client.vehicle_state().await.unwrap();
        let VehicleRelation::Mounted { mount } = before.relation.as_ref().unwrap().value else {
            panic!()
        };
        let mut state = session.state.lock().await;
        assert!(
            state
                .receive(ids::play_clientbound::SET_PASSENGERS, payload, 256)
                .is_err()
        );
        assert!(!state.ready);
        assert!(state.failure.is_some());
        let retained =
            state
                .vehicles
                .capture(before.session, state.sequence, Some(42), &state.entities);
        assert_eq!(
            retained.relation.as_ref().unwrap().value,
            VehicleRelation::Mounted { mount }
        );
        assert_eq!(
            retained.relation.unwrap().source,
            before.relation.unwrap().source
        );
        assert!(
            state
                .receive(ids::play_clientbound::SET_PASSENGERS, &[10, 0], 256)
                .is_err()
        );
        drop(state);
        assert!(client.vehicle_state().await.is_err());
    }
}

#[tokio::test]
async fn cancelled_common_flight_wait_keeps_one_owner_and_nonblocking_record() {
    use crate::client::{FlightCommand, FlightStage, GameMode};
    let (session, api, mut peer) = common_ground_fixture(GameMode::Creative).await;
    {
        let mut state = session.state.lock().await;
        let mut abilities = vec![4];
        abilities.extend(0.05f32.to_be_bytes());
        abilities.extend(0.1f32.to_be_bytes());
        operations::receive(&mut state, ids::play_clientbound::ABILITIES, &abilities).unwrap();
    }
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    let writer = session.writer.lock().await;
    let ops = client.creative();
    let mut waiter = Box::pin(ops.set_flying(true));
    assert!(
        timeout(Duration::from_millis(20), waiter.as_mut())
            .await
            .is_err()
    );
    drop(waiter);
    let pending = client.flight_record().unwrap();
    assert_eq!(pending.stage, FlightStage::Prepared);
    assert_eq!(pending.command, FlightCommand::SetFlying { flying: true });
    assert!(!pending.dispatched);
    // Reading retained intent never needs the state/writer lock held by its owner.
    assert_eq!(client.flight_record().unwrap().attempt, pending.attempt);
    drop(writer);
    assert_eq!(
        read_packet(&mut peer, None).await.unwrap(),
        (ids::play_serverbound::ABILITIES, vec![2])
    );
    let completed = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let r = client.flight_record().unwrap();
            if r.stage == FlightStage::Submitted {
                break r;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(completed.dispatched);
    assert_eq!(completed.attempt, pending.attempt);
    assert!(client.player_state().await.unwrap().received_pose.is_some());
    ops.move_flying([8.5, 67.0, 8.5], [10.0, 0.0])
        .await
        .unwrap();
    assert_eq!(
        read_packet(&mut peer, None).await.unwrap().0,
        ids::play_serverbound::POSITION_LOOK
    );
    assert_eq!(client.flight_record().unwrap().attempt, pending.attempt + 1);
    assert!(
        timeout(Duration::from_millis(20), read_packet(&mut peer, None))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn common_landing_retains_received_flight_flag_and_ground_continuation() {
    let (session, api, mut peer) = common_ground_fixture(crate::client::GameMode::Creative).await;
    let mut abilities = vec![6];
    abilities.extend(0.05f32.to_be_bytes());
    abilities.extend(0.1f32.to_be_bytes());
    {
        let mut state = session.state.lock().await;
        state.sequence += 1;
        operations::receive(&mut state, ids::play_clientbound::ABILITIES, &abilities).unwrap();
    }
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    crate::client::tests::common_creative_landing_scenario(&client).await;
    let mut frames = Vec::new();
    while let Ok(Ok(frame)) = timeout(Duration::from_millis(20), read_packet(&mut peer, None)).await
    {
        frames.push(frame);
    }
    assert_eq!(
        frames
            .iter()
            .filter(|f| f.0 == ids::play_serverbound::ABILITIES)
            .map(|f| f.1.clone())
            .collect::<Vec<_>>(),
        vec![vec![2], vec![0]]
    );
    assert_eq!(
        frames
            .iter()
            .filter(|f| f.0 == ids::play_serverbound::POSITION_LOOK)
            .count(),
        6
    );
    assert_eq!(
        frames
            .iter()
            .filter(|f| f.0 == ids::play_serverbound::PLAYER_INPUT)
            .map(|f| f.1.clone())
            .collect::<Vec<_>>(),
        vec![vec![0]; 5]
    );
    {
        let state = session.state.lock().await;
        assert_eq!(state.operations.abilities_receipt().unwrap().value, 6);
        assert_eq!(
            state.operations.local_player.velocity.unwrap().value,
            [0.; 3]
        );
    }
    assert!(!api.player_state().await.unwrap().requested_flying);
    // A later abilities packet can supersede the submitted stop even with identical flags.
    {
        let mut state = session.state.lock().await;
        state.sequence += 1;
        operations::receive(&mut state, ids::play_clientbound::ABILITIES, &abilities).unwrap();
    }
    assert!(
        client
            .creative()
            .preview_path(&[crate::client::survival::SurvivalControl {
                yaw: 0.,
                input: Default::default()
            }])
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancelled_common_landing_wait_retains_disable_neutral_and_two_ground_ticks() {
    let (session, api, mut peer) = common_ground_fixture(crate::client::GameMode::Creative).await;
    let mut abilities = vec![4];
    abilities.extend(0.05f32.to_be_bytes());
    abilities.extend(0.1f32.to_be_bytes());
    operations::receive(
        &mut *session.state.lock().await,
        ids::play_clientbound::ABILITIES,
        &abilities,
    )
    .unwrap();
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    let ops = client.creative();
    let position = client.player_state().await.unwrap().position.unwrap().value;
    ops.set_flying(true).await.unwrap();
    ops.move_flying(position, [0.; 2]).await.unwrap();
    read_packet(&mut peer, None).await.unwrap();
    read_packet(&mut peer, None).await.unwrap();
    let writer = session.writer.lock().await;
    let mut wait = Box::pin(ops.land());
    assert!(
        timeout(Duration::from_millis(20), wait.as_mut())
            .await
            .is_err()
    );
    drop(wait);
    let pending = client.flight_record().unwrap();
    assert_eq!(pending.command, crate::client::FlightCommand::Land);
    assert_eq!(pending.stage, crate::client::FlightStage::Prepared);
    assert!(!pending.landing.as_ref().unwrap().disable_dispatched);
    drop(writer);
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let r = client.flight_record().unwrap();
            assert!(r.requires_inspection.is_none());
            if r.stage == crate::client::FlightStage::Submitted {
                let l = r.landing.unwrap();
                assert!(l.disable_dispatched && l.neutral_dispatched);
                assert_eq!(l.motion.dispatched_ticks, 2);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut frames = Vec::new();
    while let Ok(Ok(f)) = timeout(Duration::from_millis(20), read_packet(&mut peer, None)).await {
        frames.push(f);
    }
    assert_eq!(
        frames
            .iter()
            .filter(|f| f.0 == ids::play_serverbound::ABILITIES)
            .map(|f| f.1.clone())
            .collect::<Vec<_>>(),
        vec![vec![0]]
    );
    assert_eq!(
        frames
            .iter()
            .filter(|f| f.0 == ids::play_serverbound::POSITION_LOOK)
            .count(),
        2
    );
    assert!(ops.land().await.is_err());
}
