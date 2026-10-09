use super::*;

#[tokio::test]
async fn player_context_keeps_received_fields_readable_after_transport_revocation() {
    let (session, api, _peer) = common_ground_fixture(crate::client::GameMode::Survival).await;
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    let mut xp = 0.25f32.to_be_bytes().to_vec();
    xp.extend([7, 20]);
    session
        .state
        .lock()
        .await
        .receive(ids::play_clientbound::EXPERIENCE, &xp, 256)
        .unwrap();
    let before = client.player_context().await.unwrap();
    let _ = client.revoke_connection();
    let closed = client.player_context().await.unwrap();
    assert_eq!(closed.experience, before.experience);
    assert_eq!(closed.session, before.session);
}
use crate::client::adapter::{CoreOps, StandingQueryOps};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWrite};

#[tokio::test]
async fn basic_client_actions_cover_received_modes_and_keep_admission_checks() {
    use crate::client::GameMode;
    for mode in [
        GameMode::Survival,
        GameMode::Creative,
        GameMode::Adventure,
        GameMode::Spectator,
    ] {
        let (session, ops, mut peer) = common_ground_fixture(mode).await;
        let client = crate::Client::from_java_1_21_11(ops.bot.clone());
        crate::client::tests::common_basic_actions_scenario(&client, mode).await;
        assert_eq!(
            read_packet(&mut peer, None).await.unwrap().0,
            ids::play_serverbound::LOOK
        );
        if mode != GameMode::Spectator {
            assert_eq!(
                read_packet(&mut peer, None).await.unwrap(),
                (
                    ids::play_serverbound::HELD_ITEM_SLOT,
                    8_i16.to_be_bytes().to_vec()
                )
            );
        }
        session.state.lock().await.motion.position_basis =
            super::super::motion::PositionBasis::PendingSubmission;
        assert!(client.look([0.0, 0.0]).await.is_err());
        assert!(client.select_hotbar(0).await.is_err());
        {
            let mut state = session.state.lock().await;
            state.motion.position_basis = super::super::motion::PositionBasis::Received;
            let mut changed = vec![3];
            changed.extend(
                (if mode == GameMode::Survival {
                    1f32
                } else {
                    0f32
                })
                .to_be_bytes(),
            );
            operations::receive(
                &mut state,
                ids::play_clientbound::GAME_STATE_CHANGE,
                &changed,
            )
            .unwrap();
        }
        assert!(
            client
                .execute(mode, crate::client::operations::Action::Look([0.0, 0.0]))
                .await
                .is_err()
        );
        session
            .state
            .lock()
            .await
            .operations
            .reset_configuration(10);
        assert!(client.look([0.0, 0.0]).await.is_err());
        assert!(client.select_hotbar(0).await.is_err());
        assert!(
            timeout(Duration::from_millis(30), read_packet(&mut peer, None))
                .await
                .is_err()
        );
        let _ = client.revoke_connection();
        assert!(client.look([0.0, 0.0]).await.is_err());
        assert!(client.select_hotbar(0).await.is_err());
    }
}

#[tokio::test]
async fn common_long_raycast_retains_shapes_unloaded_and_bounds() {
    let (session, ops, _peer) = common_ground_fixture(crate::client::GameMode::Survival).await;
    {
        let mut state = session.state.lock().await;
        for x in 8..64 {
            for z in 8..10 {
                state.world.seed_replay_cell([x, 66, z], 0);
            }
        }
        state.world.seed_replay_cell([48, 66, 8], 1);
    }
    let client = crate::Client::from_java_1_21_11(ops.bot.clone());
    crate::client::tests::common_long_raycast_scenario(&client).await;
}

#[tokio::test]
async fn long_raycast_rejects_world_switch_between_height_and_capture() {
    let (session, api, mut peer) = common_ground_fixture(crate::client::GameMode::Survival).await;
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    let guard = session.state.lock().await;
    let mut ray = Box::pin(client.raycast_blocks([8.5, 66.0, 8.5], [1.0, 0.0, 0.0], 48.0));
    assert!(
        timeout(Duration::from_millis(20), ray.as_mut())
            .await
            .is_err()
    );
    let reset_session = session.clone();
    let mut reset = tokio::spawn(async move {
        let mut state = reset_session.state.lock().await;
        state.loading.reset(12);
        state.motion.invalidate(12, "world reset");
        state.world.reset();
        state.world.select_dimension(
            "minecraft:the_nether".into(),
            Dimension::new(0, 256).unwrap(),
        );
    });
    assert!(
        timeout(Duration::from_millis(20), &mut reset)
            .await
            .is_err()
    );
    drop(guard);
    let error = timeout(Duration::from_secs(2), ray)
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::State);
    assert!(error.to_string().contains("world changed during raycast"));
    reset.await.unwrap();
    assert!(
        timeout(Duration::from_millis(30), read_packet(&mut peer, None))
            .await
            .is_err()
    );
    session.stop();
}

#[tokio::test]
async fn control_restart_keeps_model_momentum_and_aim_after_decoded_velocity() {
    let (session, api, _peer) = common_ground_fixture(crate::client::GameMode::Survival).await;
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    client.look([37.0, -12.0]).await.unwrap();
    // Modern packed vector, decoded by the actual receive path. Y/Z are zero.
    let packed = 1u64 | (17694u64 << 3) | (16383u64 << 18) | (16383u64 << 33);
    let mut velocity = vec![42, packed as u8, (packed >> 8) as u8];
    velocity.extend(((packed >> 16) as u32).to_be_bytes());
    session
        .state
        .lock()
        .await
        .receive(ids::play_clientbound::ENTITY_VELOCITY, &velocity, 256)
        .unwrap();
    let survival = client.survival();
    let start = survival.start_control().await.unwrap();
    assert_eq!([start.controls.yaw, start.controls.pitch], [37.0, -12.0]);
    timeout(Duration::from_secs(2), async {
        while survival
            .control_record()
            .await
            .unwrap()
            .unwrap()
            .dispatched_ticks
            < 3
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let stopped = survival.stop_control().await.unwrap().unwrap();
    let previous = stopped.frame.unwrap();
    assert!(previous.velocity[0] > 0.0 && previous.velocity[0] < 0.08);
    let start = survival.start_control().await.unwrap();
    assert_ne!(start.session_id, stopped.session_id);
    assert_eq!([start.controls.yaw, start.controls.pitch], [37.0, -12.0]);
    assert_eq!((start.controls.forward, start.controls.strafe), (0, 0));
    timeout(Duration::from_secs(2), async {
        while survival
            .control_record()
            .await
            .unwrap()
            .unwrap()
            .dispatched_ticks
            == 0
        {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    let resumed = survival.stop_control().await.unwrap().unwrap();
    let frame = resumed.frame.unwrap();
    assert_eq!((resumed.corrections, resumed.velocity_updates), (0, 0));
    assert!(frame.position[0] > previous.position[0]);
    assert!(frame.velocity[0] > 0.0 && frame.velocity[0] < previous.velocity[0]);
    assert_eq!(client.player_state().await.unwrap().rotation, [37.0, -12.0]);
    // Another local movement send can end at the same coordinates. Position
    // equality alone cannot authorize reusing the stopped controller's model.
    {
        let mut state = session.state.lock().await;
        let position = state.position.unwrap();
        let rotation = state.rotation;
        let generation = state.loading.generation;
        let sequence = state.sequence;
        state
            .motion
            .begin(generation, sequence, position, rotation)
            .unwrap();
        state.motion.dispatched();
    }
    assert!(survival.start_control().await.is_err());
    session.stop();
}

#[tokio::test]
async fn common_player_control_checks_all_received_modes_and_pending_dispatch() {
    use crate::client::GameMode;
    for mode in [
        GameMode::Survival,
        GameMode::Creative,
        GameMode::Adventure,
        GameMode::Spectator,
    ] {
        let (session, api, mut peer) = common_ground_fixture(mode).await;
        let client = crate::Client::from_java_1_21_11(api.bot.clone());
        let input = client.player_control(mode);
        crate::client::tests::common_player_control_scenario(&client, mode).await;
        for pitch in [-12.0f32, -90.0, 90.0] {
            let mut expected = [37.0f32.to_be_bytes(), pitch.to_be_bytes()].concat();
            expected.push(u8::from(matches!(
                mode,
                GameMode::Survival | GameMode::Adventure
            )));
            assert_eq!(
                read_packet(&mut peer, None).await.unwrap(),
                (ids::play_serverbound::LOOK, expected)
            );
        }
        if mode != GameMode::Spectator {
            for slot in [0i16, 8] {
                assert_eq!(
                    read_packet(&mut peer, None).await.unwrap(),
                    (
                        ids::play_serverbound::HELD_ITEM_SLOT,
                        slot.to_be_bytes().to_vec()
                    )
                );
            }
        }
        let other = if mode == GameMode::Survival {
            1f32
        } else {
            0f32
        };
        let mut change = vec![3];
        change.extend(other.to_be_bytes());
        session
            .state
            .lock()
            .await
            .receive(ids::play_clientbound::GAME_STATE_CHANGE, &change, 256)
            .unwrap();
        assert_eq!(
            input.look([0.; 2]).await.unwrap_err().kind(),
            ErrorKind::State
        );
        assert_eq!(
            input.select_hotbar(1).await.unwrap_err().kind(),
            ErrorKind::State
        );
        session.state.lock().await.motion.position_basis =
            operations::PositionBasis::PendingSubmission;
        let input = client.player_control(if other == 0. {
            GameMode::Survival
        } else {
            GameMode::Creative
        });
        assert_eq!(
            input.look([0.; 2]).await.unwrap_err().kind(),
            ErrorKind::State
        );
        assert_eq!(
            input.select_hotbar(1).await.unwrap_err().kind(),
            ErrorKind::State
        );
        assert!(
            timeout(Duration::from_millis(30), read_packet(&mut peer, None))
                .await
                .is_err()
        );
        session.stop();
    }
}

#[tokio::test]
async fn cancelled_basic_hotbar_selection_blocks_later_basic_inputs_without_replay() {
    let (session, api, mut peer) = common_ground_fixture(crate::client::GameMode::Adventure).await;
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    let input = client.player_control(crate::client::GameMode::Adventure);
    let writer = session.writer.lock().await;
    let mut selection = Box::pin(input.select_hotbar(4));
    assert!(
        timeout(Duration::from_millis(20), selection.as_mut())
            .await
            .is_err()
    );
    drop(selection);
    drop(writer);
    assert!(client.player_state().await.unwrap().pending_dispatch);
    assert!(
        client
            .player_state()
            .await
            .unwrap()
            .selected_hotbar
            .is_none()
    );
    assert_eq!(
        input.look([10., 0.]).await.unwrap_err().kind(),
        ErrorKind::State
    );
    assert_eq!(
        input.select_hotbar(1).await.unwrap_err().kind(),
        ErrorKind::State
    );
    assert!(
        timeout(Duration::from_millis(30), read_packet(&mut peer, None))
            .await
            .is_err()
    );
    session.stop();
}

#[tokio::test]
async fn basic_input_waiting_for_state_rechecks_received_mode_and_missing_geometry() {
    let (session, api, mut peer) = common_ground_fixture(crate::client::GameMode::Adventure).await;
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    let input = client.player_control(crate::client::GameMode::Adventure);
    let mut state = session.state.lock().await;
    let mut look = Box::pin(input.look([17., 20.]));
    assert!(
        timeout(Duration::from_millis(20), look.as_mut())
            .await
            .is_err()
    );
    let mut change = vec![3];
    change.extend(0f32.to_be_bytes());
    state
        .receive(ids::play_clientbound::GAME_STATE_CHANGE, &change, 256)
        .unwrap();
    drop(state);
    assert_eq!(look.await.unwrap_err().kind(), ErrorKind::State);
    let input = client.player_control(crate::client::GameMode::Survival);
    session.state.lock().await.operations.local_player.velocity = None;
    assert!(input.look([17., 20.]).await.is_err());
    assert_eq!(client.player_state().await.unwrap().rotation, [0.; 2]);
    session.state.lock().await.operations = Default::default();
    assert_eq!(
        input.look([17., 20.]).await.unwrap_err().kind(),
        ErrorKind::State
    );
    assert_eq!(
        input.select_hotbar(4).await.unwrap_err().kind(),
        ErrorKind::State
    );
    assert!(
        timeout(Duration::from_millis(30), read_packet(&mut peer, None))
            .await
            .is_err()
    );
    session.stop();
}

#[tokio::test]
async fn common_long_raycast_preserves_missing_cells_and_world_identity() {
    let (session, api, mut peer) = common_ground_fixture(crate::client::GameMode::Survival).await;
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    crate::client::tests::common_unloaded_long_raycast_scenario(&client).await;
    let before = client
        .raycast_blocks([8.5, 66.0, 8.5], [1.0, 0.0, 0.0], 48.0)
        .await
        .unwrap();
    {
        let mut state = session.state.lock().await;
        state.loading.reset(12);
        state.motion.invalidate(12, "world reset");
        state.world.reset();
        state.world.select_dimension(
            "minecraft:overworld".into(),
            Dimension::new(-64, 384).unwrap(),
        );
    }
    let after = client
        .raycast_blocks([8.5, 66.0, 8.5], [1.0, 0.0, 0.0], 48.0)
        .await
        .unwrap();
    assert_ne!(
        before.session.world_generation,
        after.session.world_generation
    );
    assert_eq!(before.session.connection_id, after.session.connection_id);
    assert!(matches!(
        after.result,
        crate::client::BlockRaycast::Unloaded { .. }
    ));
    assert!(
        timeout(Duration::from_millis(30), read_packet(&mut peer, None))
            .await
            .is_err()
    );
    session.stop();
}

#[tokio::test]
async fn player_capture_ground_and_rotation_keep_independent_origins_and_reset() {
    use crate::client::{GameMode, ValueSource};
    let (session, ops, _peer) = common_ground_fixture(GameMode::Survival).await;
    let client = crate::Client::from_java_1_21_11(ops.bot.clone());
    let initial = client.player_state().await.unwrap();
    assert!(initial.on_ground.is_none());
    assert!(matches!(
        initial.rotation_source,
        Some(ValueSource::Received { .. })
    ));
    ops.look(initial.rotation).await.unwrap();
    let looked = client.player_state().await.unwrap();
    assert_eq!(looked.rotation, initial.rotation);
    assert_eq!(looked.rotation_source, Some(ValueSource::Submitted));
    assert_eq!(looked.position, initial.position);
    crate::client::tests::common_control_capture_scenario(&client).await;
    {
        let mut state = session.state.lock().await;
        state.motion.on_ground = Some(false);
    }
    let airborne = client.player_state().await.unwrap();
    assert_eq!(
        airborne.on_ground.unwrap(),
        crate::client::ObservedValue {
            value: false,
            source: ValueSource::Predicted
        }
    );
    {
        let mut state = session.state.lock().await;
        let mut pose = state.motion.received_pose.clone().unwrap();
        pose.receive_sequence += 1;
        state.motion.receive(pose);
    }
    assert!(client.player_state().await.unwrap().on_ground.is_none());
    {
        let mut state = session.state.lock().await;
        state.loading.reset(12);
        state.motion.invalidate(12, "world reset");
    }
    let reset = client.player_state().await.unwrap();
    assert!(reset.on_ground.is_none() && reset.rotation_source.is_none());
    assert_ne!(
        reset.session.world_generation,
        initial.session.world_generation
    );
    session.stop();
    assert!(client.player_state().await.is_err());
}

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
        id: crate::connection::next_connection_id(),
        started: Instant::now(),
        writer: Mutex::new(Writer {
            stream: writer,
            compression: None,
        }),
        state: Mutex::new(State::default()),
        changed: Notify::new(),
        cancel: Notify::new(),
        stopped: AtomicBool::new(false),
        revoked: AtomicBool::new(false),
        receiver_abort: std::sync::OnceLock::new(),
        runtime: tokio::runtime::Handle::current(),
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

#[tokio::test]
async fn common_revocation_fences_clones_with_capture_and_writer_locked() {
    let (session, reader, mut peer) = fixture().await;
    let api = operations(&session);
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    let capture = session.state.lock().await;
    let writer = session.writer.lock().await;
    let receiving = session.clone();
    let receiver = tokio::spawn(async move { receiving.run_receiver(reader).await });
    session.receiver_abort.set(receiver.abort_handle()).unwrap();
    let mut queued = Vec::new();
    for _ in 0..32 {
        let mut sender = Box::pin(session.send(9, &[1]));
        pending(sender.as_mut()).await;
        queued.push(sender);
    }
    // Invoke outside the runtime with the immutable actual connection identity.
    let outside = client.clone();
    let connection_id = session.id;
    std::thread::spawn(move || {
        crate::client::tests::common_revocation_scenario(
            &outside,
            MinecraftVersion::Java1_21_11,
            connection_id,
        )
    })
    .join()
    .unwrap();
    for sender in queued {
        assert_eq!(
            timeout(Duration::from_secs(1), sender)
                .await
                .unwrap()
                .unwrap_err()
                .kind(),
            ErrorKind::Disconnected
        );
    }
    assert_eq!(session.interrupted_packet.load(Ordering::Acquire), -1);
    assert!(
        timeout(Duration::from_secs(1), receiver)
            .await
            .unwrap()
            .unwrap_err()
            .is_cancelled()
    );
    drop(capture);
    drop(writer);
    assert!(client.survival().select_hotbar(0).await.is_err());
    assert!(
        timeout(Duration::from_secs(1), peer.read_u8())
            .await
            .unwrap()
            .is_err()
    );
    assert!(api.operation_history().await.connection_closed);
}

#[tokio::test]
async fn common_revocation_keeps_partial_write_unknown_and_other_connection_live() {
    let (session, _reader, _peer) = fixture().await;
    let api = operations(&session);
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    let (other, _reader, mut peer) = fixture().await;
    let other_client = crate::Client::from_java_1_21_11(operations(&other).bot);
    assert_ne!(session.id, other.id);
    let (mut limited, mut prefix) = tokio::io::duplex(1);
    let mut attempt = Box::pin(session.write_frame(&mut limited, None, 9, &[0; 100]));
    pending(attempt.as_mut()).await;
    crate::client::tests::common_revocation_scenario(
        &client,
        MinecraftVersion::Java1_21_11,
        session.id,
    );
    assert_eq!(
        attempt.await.unwrap_err().kind(),
        ErrorKind::UncertainDispatch
    );
    assert_eq!(prefix.read_u8().await.unwrap(), 101);
    assert_eq!(api.operation_history().await.interrupted_packet_id, Some(9));
    assert_eq!(
        session.send(10, &[2]).await.unwrap_err().kind(),
        ErrorKind::UncertainDispatch
    );
    other.send(11, &[3]).await.unwrap();
    assert_eq!(read_packet(&mut peer, None).await.unwrap(), (11, vec![3]));
    assert_ne!(other_client.revoke_connection(), client.revoke_connection());
}

fn operations(session: &Arc<Session>) -> operations::Operations {
    operations::Operations {
        bot: Bot {
            respawn_history: {
                let state = session.state.try_lock().expect("new session");
                state.respawn_history.clone()
            },
            recipe_placement_history: {
                let state = session.state.try_lock().expect("new session");
                state.recipe_placement_history.clone()
            },
            crafting_take_history: {
                let state = session.state.try_lock().expect("new session");
                state.crafting_take_history.clone()
            },
            flight_history: {
                let state = session.state.try_lock().expect("new session");
                state.flight_history.clone()
            },
            vehicle_control_history: {
                let state = session.state.try_lock().expect("new session");
                state.vehicle_control_history.clone()
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
    for _ in 0..10 {
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
            ids::play_serverbound::BLOCK_PLACE,
            ids::play_serverbound::USE_ITEM,
            ids::play_serverbound::BLOCK_DIG,
            ids::play_serverbound::ARM_ANIMATION
        ]
    );
    assert_eq!(&emitted[0].1[..2], &36i16.to_be_bytes());
    assert_eq!(emitted[3].1, [2]);
    // Off hand, target, face UP, cursor, not inside, not border, interaction sequence 2.
    let mut place = vec![1];
    place.extend((1i64 << 12).to_be_bytes()); // packed x 0, y 0, z 1
    place.push(1);
    for v in [0.5f32; 3] {
        place.extend(v.to_be_bytes());
    }
    place.extend([0, 0, 2]);
    assert_eq!(emitted[6].1, place);
    // Off hand, sequence 3, then the current rotation for the server's rotation snap.
    let mut use_item = vec![1, 3];
    use_item.extend(10.0f32.to_be_bytes());
    use_item.extend(0.0f32.to_be_bytes());
    assert_eq!(emitted[7].1, use_item);
    // RELEASE_USE_ITEM, BlockPos.ZERO, Direction.DOWN, sequence 0.
    assert_eq!(emitted[8].1, [5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(emitted[9].1, [0]);
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
    let state = CoreOps::player_state(&api).await.unwrap();
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
    let (session, api, peer, _) = common_ground_transport(mode).await;
    (session, api, peer)
}

async fn common_ground_transport(
    mode: crate::client::GameMode,
) -> (
    Arc<Session>,
    operations::Operations,
    TcpStream,
    OwnedReadHalf,
) {
    let (session, reader, peer) = fixture().await;
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
            (match mode {
                crate::client::GameMode::Survival => 0f32,
                crate::client::GameMode::Creative => 1f32,
                crate::client::GameMode::Adventure => 2f32,
                crate::client::GameMode::Spectator => 3f32,
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
    (session, api, peer, reader)
}

#[tokio::test]
async fn restart_does_not_replay_modern_received_tcp_velocity_or_reset_rotation() {
    use crate::client::adapter::ControlOps;
    let (session, api, mut peer, reader) =
        common_ground_transport(crate::client::GameMode::Survival).await;
    {
        let mut state = session.state.lock().await;
        state.rotation = [37., -12.];
    }
    let receiving = session.clone();
    let receiver = tokio::spawn(async move { receiving.run_receiver(reader).await });
    // Native 1.21.11 packed vector, with a small positive X impulse.
    let packed = 1u64 | (17694u64 << 3) | (16383u64 << 18) | (16383u64 << 33);
    let mut velocity = vec![42, packed as u8, (packed >> 8) as u8];
    velocity.extend(((packed >> 16) as u32).to_be_bytes());
    write_packet(
        &mut peer,
        None,
        ids::play_clientbound::ENTITY_VELOCITY,
        &velocity,
    )
    .await
    .unwrap();
    timeout(Duration::from_secs(2), async {
        while session.state.lock().await.sequence == 10 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    let start = api
        .start_control(crate::client::GameMode::Survival)
        .await
        .unwrap();
    assert_eq!([start.controls.yaw, start.controls.pitch], [37., -12.]);
    timeout(Duration::from_secs(2), async {
        while api
            .control_record()
            .await
            .unwrap()
            .unwrap()
            .dispatched_ticks
            < 3
        {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    let frame = api.stop_control().await.unwrap().unwrap().frame.unwrap();
    assert!(frame.velocity[0] > 0. && frame.velocity[0] < 0.08);
    let start = api
        .start_control(crate::client::GameMode::Survival)
        .await
        .unwrap();
    assert_eq!([start.controls.yaw, start.controls.pitch], [37., -12.]);
    timeout(Duration::from_secs(2), async {
        loop {
            let record = api.control_record().await.unwrap().unwrap();
            if record.dispatched_ticks > 0 {
                assert_eq!(record.dispatched_ticks, 1);
                assert_eq!((record.corrections, record.velocity_updates), (0, 0));
                assert!(
                    (record.frame.unwrap().position[0] - frame.position[0] - frame.velocity[0])
                        .abs()
                        < 1e-12
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    api.stop_control().await.unwrap();
    // A new impulse while stopped is consumed exactly once by the new session.
    let before = session.state.lock().await.position.unwrap();
    let sequence = session.state.lock().await.sequence;
    let packed = 1u64 | (15728u64 << 3) | (16383u64 << 18) | (16383u64 << 33);
    let mut velocity = vec![42, packed as u8, (packed >> 8) as u8];
    velocity.extend(((packed >> 16) as u32).to_be_bytes());
    write_packet(
        &mut peer,
        None,
        ids::play_clientbound::ENTITY_VELOCITY,
        &velocity,
    )
    .await
    .unwrap();
    timeout(Duration::from_secs(2), async {
        while session.state.lock().await.sequence == sequence {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    let expected = session
        .state
        .lock()
        .await
        .operations
        .local_player
        .velocity
        .unwrap()
        .value[0];
    api.start_control(crate::client::GameMode::Survival)
        .await
        .unwrap();
    timeout(Duration::from_secs(2), async {
        loop {
            let record = api.control_record().await.unwrap().unwrap();
            if record.dispatched_ticks > 0 {
                assert_eq!(record.dispatched_ticks, 1);
                assert!((record.frame.unwrap().position[0] - before[0] - expected).abs() < 1e-12);
                assert_eq!(record.velocity_updates, 0);
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    api.stop_control().await.unwrap();
    // A real own-position correction while stopped also supplies the next view.
    let sequence = session.state.lock().await.sequence;
    let mut pose = vec![1];
    for v in [9.5f64, 68., 9.5, 0., 0., 0.] {
        pose.extend(v.to_be_bytes());
    }
    for v in [151f32, -23.] {
        pose.extend(v.to_be_bytes());
    }
    pose.extend(0u32.to_be_bytes());
    write_packet(&mut peer, None, ids::play_clientbound::POSITION, &pose)
        .await
        .unwrap();
    timeout(Duration::from_secs(2), async {
        while session.state.lock().await.sequence == sequence {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    let start = api
        .start_control(crate::client::GameMode::Survival)
        .await
        .unwrap();
    assert_eq!([start.controls.yaw, start.controls.pitch], [151., -23.]);
    api.stop_control().await.unwrap();
    session.stop();
    receiver.await.unwrap();
}

#[tokio::test]
async fn ground_request_admission_rechecks_modern_tcp_mode_and_revocation_after_waiting() {
    let (session, api, mut peer, reader) =
        common_ground_transport(crate::client::GameMode::Survival).await;
    let receiving = session.clone();
    let receiver = tokio::spawn(async move { receiving.run_receiver(reader).await });
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    let survival = client.survival();
    assert!(survival.request_ground_jump(1).await.is_err());
    let started = survival.start_control().await.unwrap();
    assert!(
        survival
            .request_ground_jump(started.session_id + 1)
            .await
            .is_err()
    );
    let queued = survival
        .request_ground_jump(started.session_id)
        .await
        .unwrap();
    let guard = session.state.lock().await;
    let mut cancelled = Box::pin(survival.request_ground_jump(started.session_id));
    assert!(
        timeout(Duration::from_millis(10), cancelled.as_mut())
            .await
            .is_err()
    );
    drop(cancelled);
    drop(guard);
    assert_eq!(
        survival
            .control_record()
            .await
            .unwrap()
            .unwrap()
            .ground_jump
            .unwrap()
            .request_id,
        queued.request_id
    );
    for mode in [1f32, 0.] {
        let sequence = session.state.lock().await.sequence;
        let mut packet = vec![3];
        packet.extend(mode.to_be_bytes());
        write_packet(
            &mut peer,
            None,
            ids::play_clientbound::GAME_STATE_CHANGE,
            &packet,
        )
        .await
        .unwrap();
        timeout(Duration::from_secs(2), async {
            while session.state.lock().await.sequence == sequence {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        if mode == 1. {
            assert!(
                survival
                    .request_ground_jump(started.session_id)
                    .await
                    .is_err()
            );
            timeout(Duration::from_secs(2), async {
                while !matches!(
                    survival.control_record().await.unwrap().unwrap().status,
                    crate::client::control::ControlStatus::Stopped { .. }
                ) {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert!(
                survival
                    .request_ground_jump(started.session_id)
                    .await
                    .is_err()
            );
            assert!(
                survival
                    .control_record()
                    .await
                    .unwrap()
                    .unwrap()
                    .ground_jump
                    .unwrap()
                    .status
                    != crate::client::control::GroundJumpStatus::Queued
            );
        }
    }
    let replacement = survival.start_control().await.unwrap();
    assert!(
        survival
            .request_ground_jump(started.session_id)
            .await
            .is_err()
    );
    let guard = session.state.lock().await;
    let mut waiting = Box::pin(survival.request_ground_jump(replacement.session_id));
    assert!(
        timeout(Duration::from_millis(10), waiting.as_mut())
            .await
            .is_err()
    );
    let _ = client.revoke_connection();
    drop(guard);
    assert!(waiting.await.is_err());
    assert!(
        survival
            .control_record()
            .await
            .unwrap()
            .unwrap()
            .ground_jump
            .is_none()
    );
    receiver.await.unwrap();
}

#[tokio::test]
async fn selected_control_rejects_replacement_and_old_world_without_writes() {
    use crate::client::control::{ControlSession, ControlStatus, Controls, Output, Received};
    let (session, api, mut peer) = common_ground_fixture(crate::client::GameMode::Survival).await;
    let received = Received {
        environment: crate::client::physics::Environment::defaults(
            crate::MinecraftVersion::Java1_21_11,
        ),
        pose: Some((10, [8.5, 65.0, 8.5], Some([0.0; 3]))),
        velocity: None,
        using_item: None,
    };
    let mut control = ControlSession::new(
        crate::MinecraftVersion::Java1_21_11,
        2,
        [8.5, 65.0, 8.5],
        &received,
    );
    control.dispatched(&Output {
        sneak: Some(true),
        sprint: Some(true),
        input: Some(32),
        position: [8.5, 65.0, 8.5],
        rotation: [0.0; 2],
        on_ground: true,
        horizontal_collision: false,
    });
    {
        let mut state = session.state.lock().await;
        let generation = state.loading.generation;
        state.control.seed_session(control, generation);
    }
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    let survival = client.survival();
    let keys = Controls {
        forward: 1,
        ..Default::default()
    };
    assert!(survival.set_controls_for(1, keys).await.is_err());
    assert!(survival.stop_control_for(1).await.is_err());
    let replacement = survival.control_record().await.unwrap().unwrap();
    assert_eq!(replacement.session_id, 2);
    assert_eq!(replacement.status, ControlStatus::Running);
    assert_eq!(replacement.controls, Controls::default());
    assert_eq!(
        survival.set_controls_for(2, keys).await.unwrap().controls,
        keys
    );
    session.state.lock().await.loading.generation += 1;
    assert!(
        survival
            .set_controls_for(2, Controls::default())
            .await
            .is_err()
    );
    let stopped = survival.stop_control_for(2).await.unwrap();
    assert_eq!(stopped.controls, keys);
    assert!(
        matches!(stopped.status, ControlStatus::Stopped { ref reason } if reason.contains("world changed"))
    );
    assert_eq!(survival.stop_control_for(2).await.unwrap(), stopped);
    assert!(
        timeout(Duration::from_millis(30), read_packet(&mut peer, None))
            .await
            .is_err()
    );
    session.stop();
}

#[tokio::test]
async fn cancelling_control_stop_wait_still_releases_keys_and_retains_record() {
    use crate::client::adapter::ControlOps;
    use crate::client::control::{ControlSession, ControlStatus, Output, Received};
    let (session, api, mut peer) = common_ground_fixture(crate::client::GameMode::Survival).await;
    let received = Received {
        environment: crate::client::physics::Environment::defaults(
            crate::MinecraftVersion::Java1_21_11,
        ),
        pose: Some((10, [8.5, 65.0, 8.5], Some([0.0; 3]))),
        velocity: None,
        using_item: None,
    };
    let mut control = ControlSession::new(
        crate::MinecraftVersion::Java1_21_11,
        1,
        [8.5, 65.0, 8.5],
        &received,
    );
    control.dispatched(&Output {
        sneak: Some(true),
        sprint: Some(true),
        input: Some(32),
        position: [8.5, 65.0, 8.5],
        rotation: [0.0; 2],
        on_ground: true,
        horizontal_collision: false,
    });
    {
        let mut state = session.state.lock().await;
        let generation = state.loading.generation;
        state.control.seed_session(control, generation);
    }
    let writer = session.writer.lock().await;
    let mut wait = Box::pin(api.stop_control_for(1));
    assert!(
        timeout(Duration::from_millis(30), wait.as_mut())
            .await
            .is_err()
    );
    drop(wait);
    drop(writer);
    assert_eq!(
        timeout(Duration::from_secs(1), read_packet(&mut peer, None))
            .await
            .unwrap()
            .unwrap(),
        (ids::play_serverbound::PLAYER_INPUT, vec![0])
    );
    assert_eq!(
        timeout(Duration::from_secs(1), read_packet(&mut peer, None))
            .await
            .unwrap()
            .unwrap(),
        (ids::play_serverbound::ENTITY_ACTION, vec![42, 2, 0])
    );
    let record = api.control_record().await.unwrap().unwrap();
    assert_eq!(record.session_id, 1);
    assert!(matches!(record.status, ControlStatus::Stopped { .. }));
    assert_eq!(api.stop_control_for(1).await.unwrap(), record);
    assert!(
        timeout(Duration::from_millis(30), read_packet(&mut peer, None))
            .await
            .is_err()
    );
    session.stop();
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
        let initial = client.entity_motion(target).await.unwrap();
        assert!(initial.on_ground.is_none());
        assert_eq!(initial.velocity.as_ref().unwrap().value, [0.0; 3]);
        let mut relative = vec![43];
        for value in [4096i16, 0, 0] {
            relative.extend(value.to_be_bytes());
        }
        relative.push(1);
        session
            .state
            .lock()
            .await
            .receive(ids::play_clientbound::REL_ENTITY_MOVE, &relative, 256)
            .unwrap();
        let moved = client.entity_motion(target).await.unwrap();
        assert_eq!(
            moved.position.as_ref().unwrap().value.position,
            [10.0, 65.0, 8.5]
        );
        assert_eq!(moved.entity.spawn_position, initial.entity.spawn_position);
        assert_eq!(moved.velocity, initial.velocity);
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
        assert!(client.entity_motion(target).await.is_err());
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
        // Malformed receipt is terminal on this adapter. Check the retained
        // ledger directly, and require all public live observations to fail.
        session
            .state
            .lock()
            .await
            .receive(ids::play_clientbound::SPAWN_ENTITY, &spawn, 256)
            .unwrap();
        let fresh = client.entity_spawns().await.unwrap().entities[0].id;
        let before_failure = client.entity_motion(fresh).await.unwrap();
        relative.push(0);
        let mut state = session.state.lock().await;
        assert!(
            state
                .receive(ids::play_clientbound::REL_ENTITY_MOVE, &relative, 256)
                .is_err()
        );
        assert_eq!(
            state
                .entities
                .capture_motion(observed.session, fresh, state.sequence)
                .unwrap()
                .position,
            before_failure.position
        );
        drop(state);
        assert!(client.entity_motion(fresh).await.is_err());
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
        StandingQueryOps::preview_path(
            &client.java_1_21_11().unwrap().operations(),
            mode,
            &controls,
        )
        .await
        .unwrap();
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
        assert!(
            StandingQueryOps::preview_path(
                &client.java_1_21_11().unwrap().operations(),
                mode,
                &controls
            )
            .await
            .is_err()
        );
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
        assert!(
            StandingQueryOps::preview_path(
                &client.java_1_21_11().unwrap().operations(),
                mode,
                &controls
            )
            .await
            .is_err()
        );
        session
            .state
            .lock()
            .await
            .receive(ids::play_clientbound::ENTITY_DESTROY, &[1, 10], 256)
            .unwrap();
        assert!(client.vehicle_state().await.unwrap().relation.is_none());
        assert!(
            StandingQueryOps::preview_path(
                &client.java_1_21_11().unwrap().operations(),
                mode,
                &controls
            )
            .await
            .is_err()
        );
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

#[tokio::test]
async fn cancelled_vehicle_control_waiter_keeps_finite_owner_and_final_neutral() {
    use crate::client::{GameMode, VehicleControlStage, VehicleInput, VehicleRelation};
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
        let inputs = [
            VehicleInput {
                forward: 1,
                ..Default::default()
            },
            VehicleInput {
                forward: 1,
                ..Default::default()
            },
            VehicleInput::default(),
        ];
        let writer = session.writer.lock().await;
        let ops = client.survival();
        let creative = client.creative();
        let mut attempt = Box::pin(async {
            if mode == GameMode::Survival {
                ops.start_vehicle_control(mount, &inputs).await
            } else {
                creative.start_vehicle_control(mount, &inputs).await
            }
        });
        assert!(
            timeout(Duration::from_millis(20), attempt.as_mut())
                .await
                .is_err()
        );
        drop(attempt);
        let pending = timeout(Duration::from_millis(50), client.vehicle_control_record())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(pending.stage, VehicleControlStage::Running);
        assert_eq!(pending.attempted_tick, 1);
        assert_eq!(pending.dispatched_ticks, 0);
        drop(writer);
        assert_eq!(read_packet(&mut peer, None).await.unwrap(), (0x2a, vec![1]));
        let busy = if mode == GameMode::Survival {
            client.survival().dismount(mount).await
        } else {
            client.creative().dismount(mount).await
        };
        assert!(busy.is_err());
        assert_eq!(read_packet(&mut peer, None).await.unwrap(), (0x2a, vec![1]));
        assert_eq!(read_packet(&mut peer, None).await.unwrap(), (0x2a, vec![0]));
        let final_record = timeout(Duration::from_secs(1), async {
            loop {
                let r = client.vehicle_control_record().await.unwrap().unwrap();
                if r.stage == VehicleControlStage::Submitted {
                    break r;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(final_record.id, pending.id);
        assert_eq!(
            (final_record.attempted_tick, final_record.dispatched_ticks),
            (3, 3)
        );
        assert!(
            timeout(Duration::from_millis(20), read_packet(&mut peer, None))
                .await
                .is_err()
        );
        let _ = client.revoke_connection();
        assert_eq!(
            client
                .vehicle_control_record()
                .await
                .unwrap()
                .unwrap()
                .stage,
            VehicleControlStage::Submitted
        );
    }
}
#[tokio::test]
async fn vehicle_control_cannot_retire_partially_sent_ground_run() {
    use crate::client::{
        GameMode, VehicleInput, VehicleRelation,
        survival::{MotionStatus, SurvivalControl},
    };
    let (session, api, mut peer) = common_ground_fixture(GameMode::Survival).await;
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    let controls = [SurvivalControl {
        yaw: 0.0,
        input: Default::default(),
    }; 20];
    let started = client
        .survival()
        .start_predicted_path(&controls)
        .await
        .unwrap();
    assert_eq!(
        read_packet(&mut peer, None).await.unwrap().0,
        ids::play_serverbound::PLAYER_INPUT
    );
    assert_eq!(
        read_packet(&mut peer, None).await.unwrap().0,
        ids::play_serverbound::POSITION_LOOK
    );
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
    assert!(
        client
            .survival()
            .start_vehicle_control(mount, &[VehicleInput::default()])
            .await
            .is_err()
    );
    let retained = timeout(Duration::from_secs(1), async {
        loop {
            let record = client.survival().motion_record().await.unwrap().unwrap();
            if record.status != MotionStatus::Running {
                break record;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(retained.run_id, started.run_id);
    assert_eq!(retained.dispatched_ticks, 1);
    assert_eq!(retained.status, MotionStatus::RequiresInspection);
    assert!(client.vehicle_control_record().await.unwrap().is_none());
    assert!(
        timeout(Duration::from_millis(20), read_packet(&mut peer, None))
            .await
            .is_err()
    );
    let _ = client.revoke_connection();
}
#[tokio::test]
async fn vehicle_control_revocation_while_writer_waits_preserves_first_failure() {
    use crate::client::{GameMode, VehicleControlStage, VehicleInput, VehicleRelation};
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
    let inputs = [VehicleInput::default()];
    let mut attempt = Box::pin(ops.start_vehicle_control(mount, &inputs));
    assert!(
        timeout(Duration::from_millis(20), attempt.as_mut())
            .await
            .is_err()
    );
    let before = timeout(Duration::from_millis(50), client.vehicle_control_record())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!((before.attempted_tick, before.dispatched_ticks), (1, 0));
    let _ = client.revoke_connection();
    let revoked = timeout(Duration::from_millis(50), client.vehicle_control_record())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(revoked.stage, VehicleControlStage::RequiresInspection);
    let first = revoked.requires_inspection.clone();
    assert!(first.is_some());
    drop(writer);
    assert!(
        timeout(Duration::from_secs(1), attempt.as_mut())
            .await
            .unwrap()
            .is_err()
    );
    let after = client.vehicle_control_record().await.unwrap().unwrap();
    assert_eq!(after.id, before.id);
    assert_eq!(after.stage, VehicleControlStage::RequiresInspection);
    assert_eq!(after.requires_inspection, first);
    assert_eq!(after.dispatched_ticks, 0);
    assert!(matches!(
        timeout(Duration::from_millis(20), read_packet(&mut peer, None)).await,
        Err(_) | Ok(Err(_))
    ));
}
#[tokio::test]
async fn vehicle_control_latches_unmount_before_same_numeric_vehicle_reappears() {
    use crate::client::{GameMode, VehicleControlStage, VehicleInput, VehicleRelation};
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
    let ops = client.survival();
    let waiter = tokio::spawn(async move {
        ops.start_vehicle_control(
            mount,
            &[
                VehicleInput {
                    forward: 1,
                    ..Default::default()
                },
                VehicleInput {
                    forward: 1,
                    ..Default::default()
                },
                VehicleInput::default(),
            ],
        )
        .await
    });
    assert_eq!(read_packet(&mut peer, None).await.unwrap(), (0x2a, vec![1]));
    {
        let mut state = session.state.lock().await;
        state
            .receive(ids::play_clientbound::SET_PASSENGERS, &[10, 0], 256)
            .unwrap();
        state
            .receive(ids::play_clientbound::SET_PASSENGERS, &[10, 1, 42], 256)
            .unwrap();
    }
    assert!(waiter.await.unwrap().is_err());
    let record = client.vehicle_control_record().await.unwrap().unwrap();
    assert_eq!(record.stage, VehicleControlStage::RequiresInspection);
    assert_eq!(record.dispatched_ticks, 1);
    assert!(
        client
            .survival()
            .start_vehicle_control(mount, &[VehicleInput::default()])
            .await
            .is_err()
    );
    assert!(client.survival().dismount(mount).await.is_err());
    assert!(
        timeout(Duration::from_millis(50), read_packet(&mut peer, None))
            .await
            .is_err()
    );
    let _ = client.revoke_connection();
    assert_eq!(
        client
            .vehicle_control_record()
            .await
            .unwrap()
            .unwrap()
            .requires_inspection,
        record.requires_inspection
    );
}

#[tokio::test]
async fn received_vehicle_correction_stops_remaining_mounted_inputs() {
    use crate::client::{GameMode, VehicleControlStage, VehicleInput, VehicleRelation};
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
    let inputs = vec![VehicleInput::default(); 20];
    let ops = client.survival();
    let attempt = tokio::spawn(async move { ops.start_vehicle_control(mount, &inputs).await });
    timeout(Duration::from_secs(1), read_packet(&mut peer, None))
        .await
        .unwrap()
        .unwrap();
    session
        .state
        .lock()
        .await
        .receive(ids::play_clientbound::VEHICLE_MOVE, &[0; 32], 256)
        .unwrap();
    assert!(attempt.await.unwrap().is_err());
    let stopped = client.vehicle_control_record().await.unwrap().unwrap();
    assert_eq!(stopped.stage, VehicleControlStage::RequiresInspection);
    assert!(stopped.dispatched_ticks < 20);
    assert!(
        stopped
            .requires_inspection
            .as_deref()
            .unwrap()
            .contains("correction")
    );
    assert!(
        client
            .vehicle_state()
            .await
            .unwrap()
            .motion_correction_sequence
            .is_some()
    );
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(
        client
            .vehicle_control_record()
            .await
            .unwrap()
            .unwrap()
            .dispatched_ticks,
        stopped.dispatched_ticks
    );
}

#[tokio::test]
async fn common_dismount_ground_keeps_unknown_received_velocity_and_continues_both_modes() {
    use crate::client::{GameMode, VehicleRelation};
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
            panic!();
        };
        let record = if mode == GameMode::Survival {
            client.survival().dismount(mount).await.unwrap()
        } else {
            client.creative().dismount(mount).await.unwrap()
        };
        assert_eq!(
            read_packet(&mut peer, None).await.unwrap(),
            (0x2a, vec![32])
        );
        let mut position = Vec::new();
        put_varint(&mut position, 99);
        for value in [8.5f64, 65., 8.5, 0., 0., 0.] {
            position.extend(value.to_be_bytes());
        }
        position.extend([0; 8]);
        position.extend(0x1f8u32.to_be_bytes());
        session
            .state
            .lock()
            .await
            .receive(ids::play_clientbound::POSITION, &position, 256)
            .unwrap();
        assert!(client.survival().resume_ground(record.id).await.is_err());
        session
            .state
            .lock()
            .await
            .receive(ids::play_clientbound::SET_PASSENGERS, &[10, 0], 256)
            .unwrap();
        if mode == GameMode::Survival {
            client
                .survival()
                .complete_dismount(record.id)
                .await
                .unwrap();
        } else {
            client
                .creative()
                .complete_dismount(record.id)
                .await
                .unwrap();
        }
        assert_eq!(read_packet(&mut peer, None).await.unwrap(), (0x2a, vec![0]));
        let before = session.state.lock().await.operations.local_player.clone();
        assert!(before.velocity.is_none());
        assert!(before.motion_interruption.is_some());
        crate::client::tests::common_dismount_ground_scenario(&client, record.id).await;
        session
            .state
            .lock()
            .await
            .receive(ids::play_clientbound::ENTITY_DESTROY, &[1, 10], 256)
            .unwrap();
        assert!(client.vehicle_state().await.unwrap().relation.is_none());
        let preview = if mode == GameMode::Survival {
            client
                .survival()
                .preview_path(&[crate::client::survival::SurvivalControl {
                    yaw: 0.,
                    input: Default::default(),
                }])
                .await
        } else {
            client
                .creative()
                .preview_path(&[crate::client::survival::SurvivalControl {
                    yaw: 0.,
                    input: Default::default(),
                }])
                .await
        };
        assert!(
            preview.is_ok(),
            "ground stop must survive retirement of an already unmounted vehicle: {preview:?}"
        );
        crate::client::tests::common_retired_vehicle_ground_scenario(&client).await;
        for _ in 0..6 {
            assert_eq!(
                read_packet(&mut peer, None).await.unwrap(),
                (ids::play_serverbound::PLAYER_INPUT, vec![0])
            );
            assert_eq!(
                read_packet(&mut peer, None).await.unwrap().0,
                ids::play_serverbound::POSITION_LOOK
            );
        }
        let after = session.state.lock().await.operations.local_player.clone();
        assert_eq!(after.velocity, before.velocity);
        assert_eq!(after.motion_interruption, before.motion_interruption);
        assert!(
            timeout(Duration::from_millis(20), read_packet(&mut peer, None))
                .await
                .is_err()
        );
        session.stop();
    }
}

#[tokio::test]
async fn common_dismount_ground_owned_cancel_remount_and_revoke_preserve_history() {
    use crate::client::{GameMode, VehicleRelation, survival::MotionStatus};
    for kind in ["cancel", "remount", "retire", "revoke"] {
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
            panic!();
        };
        let record = client.survival().dismount(mount).await.unwrap();
        let mut position = Vec::new();
        put_varint(&mut position, 99);
        for value in [8.5f64, 65., 8.5, 0., 0., 0.] {
            position.extend(value.to_be_bytes());
        }
        position.extend([0; 8]);
        position.extend(0x1f8u32.to_be_bytes());
        session
            .state
            .lock()
            .await
            .receive(ids::play_clientbound::POSITION, &position, 256)
            .unwrap();
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
        for _ in 0..2 {
            read_packet(&mut peer, None).await.unwrap();
        }
        if matches!(kind, "remount" | "retire") {
            let owned = client.clone();
            let waiter =
                tokio::spawn(async move { owned.survival().resume_ground(record.id).await });
            assert_eq!(
                read_packet(&mut peer, None).await.unwrap().0,
                ids::play_serverbound::PLAYER_INPUT
            );
            assert_eq!(
                read_packet(&mut peer, None).await.unwrap().0,
                ids::play_serverbound::POSITION_LOOK
            );
            if kind == "remount" {
                session
                    .state
                    .lock()
                    .await
                    .receive(ids::play_clientbound::SET_PASSENGERS, &[10, 1, 42], 256)
                    .unwrap();
            } else {
                session
                    .state
                    .lock()
                    .await
                    .receive(ids::play_clientbound::ENTITY_DESTROY, &[1, 10], 256)
                    .unwrap();
            }
            assert!(waiter.await.unwrap().is_err());
            let failed = client
                .dismount_record()
                .await
                .unwrap()
                .unwrap()
                .grounding
                .unwrap()
                .motion;
            assert_eq!(failed.status, MotionStatus::RequiresInspection);
            assert_eq!((failed.attempted_tick, failed.dispatched_ticks), (1, 1));
            let _ = client.revoke_connection();
            assert_eq!(
                client
                    .dismount_record()
                    .await
                    .unwrap()
                    .unwrap()
                    .grounding
                    .unwrap()
                    .motion
                    .problem,
                failed.problem
            );
        } else {
            let writer = session.writer.lock().await;
            let ops = client.survival();
            let mut waiter = Box::pin(ops.resume_ground(record.id));
            assert!(
                timeout(Duration::from_millis(20), waiter.as_mut())
                    .await
                    .is_err()
            );
            drop(waiter);
            let intent = timeout(Duration::from_millis(50), client.dismount_record())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(intent.id, record.id);
            assert_eq!(intent.grounding.unwrap().motion.dispatched_ticks, 0);
            if kind == "revoke" {
                let _ = client.revoke_connection();
                let failed = client
                    .dismount_record()
                    .await
                    .unwrap()
                    .unwrap()
                    .grounding
                    .unwrap()
                    .motion;
                assert_eq!(failed.status, MotionStatus::RequiresInspection);
                drop(writer);
                tokio::time::sleep(Duration::from_millis(30)).await;
                let again = client
                    .dismount_record()
                    .await
                    .unwrap()
                    .unwrap()
                    .grounding
                    .unwrap()
                    .motion;
                assert_eq!(again.dispatched_ticks, 0);
                assert_eq!(again.problem, failed.problem);
            } else {
                drop(writer);
                for _ in 0..2 {
                    assert_eq!(
                        read_packet(&mut peer, None).await.unwrap(),
                        (ids::play_serverbound::PLAYER_INPUT, vec![0])
                    );
                    assert_eq!(
                        read_packet(&mut peer, None).await.unwrap().0,
                        ids::play_serverbound::POSITION_LOOK
                    );
                }
                timeout(Duration::from_secs(2), async {
                    loop {
                        let g = client
                            .dismount_record()
                            .await
                            .unwrap()
                            .unwrap()
                            .grounding
                            .unwrap();
                        assert!(g.motion.problem.is_none(), "{:?}", g.motion.problem);
                        if g.motion.status == MotionStatus::Predicted {
                            assert_eq!(g.motion.dispatched_ticks, 2);
                            break;
                        }
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
            }
        }
        assert!(client.survival().resume_ground(record.id).await.is_err());
        assert!(!matches!(
            timeout(Duration::from_millis(20), read_packet(&mut peer, None)).await,
            Ok(Ok(_))
        ));
        session.stop();
    }
}

#[tokio::test]
async fn common_boss_bar_transport_preserves_fields_and_actual_removal() {
    let (session, api, _peer) = common_ground_fixture(crate::client::GameMode::Survival).await;
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../data/client_api/boss_bar_packets.json"
    ))
    .unwrap();
    let rows = fixtures["versions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["version"] == "1.21.11")
        .unwrap();
    let decode = |r: &serde_json::Value| {
        r["payload_hex"]
            .as_str()
            .unwrap()
            .as_bytes()
            .chunks_exact(2)
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect::<Vec<_>>()
    };
    assert!(
        client
            .boss_bars()
            .await
            .unwrap()
            .last_update_sequence
            .is_none()
    );
    session
        .state
        .lock()
        .await
        .receive(
            ids::play_clientbound::BOSS_BAR,
            &decode(&rows["packets"][0]),
            256,
        )
        .unwrap();
    let before = client.boss_bars().await.unwrap();
    session
        .state
        .lock()
        .await
        .receive(
            ids::play_clientbound::BOSS_BAR,
            &decode(&rows["packets"][2]),
            256,
        )
        .unwrap();
    let updated = client.boss_bars().await.unwrap();
    assert_eq!(before.bars[0].title, updated.bars[0].title);
    assert_ne!(
        before.bars[0].progress.source,
        updated.bars[0].progress.source
    );
    session
        .state
        .lock()
        .await
        .receive(
            ids::play_clientbound::BOSS_BAR,
            &decode(&rows["packets"][1]),
            256,
        )
        .unwrap();
    let removed = client.boss_bars().await.unwrap();
    assert!(removed.bars.is_empty());
    assert!(removed.last_update_sequence > updated.last_update_sequence);
    client.disconnect().await.unwrap();
    assert!(client.boss_bars().await.is_err());
}

#[tokio::test]
async fn common_display_receipts_clear_reset_and_atomic_tab_match_native_packets() {
    let (session, api, _peer) = common_ground_fixture(crate::client::GameMode::Creative).await;
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    let samples: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../data/client_api/display_packets.json"
    ))
    .unwrap();
    let rows = &samples["versions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["version"] == "1.21.11")
        .unwrap()["packets"];
    let decode = |r: &serde_json::Value| {
        r["payload_hex"]
            .as_str()
            .unwrap()
            .as_bytes()
            .chunks_exact(2)
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect::<Vec<_>>()
    };
    assert!(client.titles().await.unwrap().timing.is_none());
    assert!(client.tab_list().await.unwrap().text.is_none());
    assert!(client.world_border().await.unwrap().size.is_none());
    for row in rows.as_array().unwrap() {
        session
            .state
            .lock()
            .await
            .receive(row["packet_id"].as_i64().unwrap() as i32, &decode(row), 256)
            .unwrap();
    }
    let titles = client.titles().await.unwrap();
    assert!(titles.title.unwrap().value.is_none());
    assert!(titles.action_bar.is_some());
    assert!(titles.clear.unwrap().value);
    assert_eq!(
        titles.timing.unwrap().value,
        crate::client::ui::TitleTiming::ResetToDefaults
    );
    let border = client.world_border().await.unwrap();
    assert_eq!(border.warning_delay.unwrap().value, 17);
    assert_eq!(border.warning_distance.unwrap().value, 3);
    let tab = client.tab_list().await.unwrap();
    assert!(matches!(
        tab.text.as_ref().unwrap().value.header,
        crate::client::ui::UiText::NativeNbt { .. }
    ));
    session.state.lock().await.loading.generation += 1;
    let border = client.world_border().await.unwrap();
    assert!(border.center.is_none() && border.size.is_none() && border.warning_delay.is_none());
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["group"] == "tab")
        .unwrap();
    let mut truncated = decode(row);
    truncated.pop();
    assert!(
        session
            .state
            .lock()
            .await
            .receive(0x78, &truncated, 256)
            .is_err()
    );
    // A malformed native packet makes the session fail. The ledger must still
    // retain its last complete receipt, without making public observations usable.
    let state = session.state.lock().await;
    assert_eq!(
        state.display.tab_list(tab.session, state.sequence).text,
        tab.text
    );
    drop(state);
    assert!(client.tab_list().await.is_err());
    client.disconnect().await.unwrap();
    assert!(
        client.titles().await.is_err()
            && client.tab_list().await.is_err()
            && client.world_border().await.is_err()
    );
}

#[tokio::test]
async fn social_common_bridge_applies_every_original_team_and_player_info_packet() {
    use crate::client::ui::social_tests::{assert_bridge, bridge_bytes, bridge_cases};
    let (session, api, _peer) = common_ground_fixture(crate::client::GameMode::Creative).await;
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    assert!(client.teams().await.unwrap().last_update_sequence.is_none());
    assert!(
        client
            .player_list()
            .await
            .unwrap()
            .last_update_sequence
            .is_none()
    );
    for row in bridge_cases(crate::MinecraftVersion::Java1_21_11) {
        session
            .state
            .lock()
            .await
            .receive(
                row["packet_id"].as_i64().unwrap() as i32,
                &bridge_bytes(&row),
                256,
            )
            .unwrap();
        assert_bridge(&client, &row).await;
    }
    client.disconnect().await.unwrap();
    assert!(client.teams().await.is_err() && client.player_list().await.is_err());
}

#[tokio::test]
async fn common_reconfiguration_resets_ui_and_requires_fresh_registration() {
    use crate::client::{
        GameMode,
        ui::social_tests::{bridge_bytes, bridge_cases},
    };
    for mode in [GameMode::Survival, GameMode::Creative] {
        let (session, api, _peer) = common_ground_fixture(mode).await;
        let client = crate::Client::from_java_1_21_11(api.bot.clone());
        let mut selected = vec![];
        for dataset in [
            include_str!("../../../../../data/client_api/display_packets.json"),
            include_str!("../../../../../data/client_api/boss_bar_packets.json"),
        ] {
            let value: serde_json::Value = serde_json::from_str(dataset).unwrap();
            let version = value["versions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["version"] == "1.21.11")
                .unwrap();
            for row in version["packets"].as_array().unwrap() {
                if row["group"] == "tab"
                    || row["group"] == "title" && row["operation"].as_u64().is_some_and(|v| v < 3)
                    || row.get("group").is_none() && row["operation"] == 0
                {
                    selected.push(row.clone());
                }
            }
        }
        let social = bridge_cases(MinecraftVersion::Java1_21_11);
        selected.push(
            social
                .iter()
                .find(|r| r["group"] == "team" && r["operation"] == 0)
                .unwrap()
                .clone(),
        );
        selected.push(
            social
                .iter()
                .find(|r| r["group"] != "team" && r["flags"] == 255)
                .unwrap()
                .clone(),
        );
        {
            let mut state = session.state.lock().await;
            for row in &selected {
                let id = row["packet_id"]
                    .as_i64()
                    .map_or(ids::play_clientbound::BOSS_BAR, |v| v as i32);
                state.receive(id, &bridge_bytes(row), 256).unwrap();
            }
            let mut objective = vec![];
            put_string(&mut objective, "old_context");
            objective.push(0);
            objective.extend([8, 0, 3, b'O', b'l', b'd']);
            put_varint(&mut objective, 0);
            objective.push(0);
            state
                .receive(ids::play_clientbound::SCOREBOARD_OBJECTIVE, &objective, 256)
                .unwrap();
        }
        let before = client.teams().await.unwrap();
        let old_roster = client.player_list().await.unwrap();
        let old_titles = client.titles().await.unwrap();
        assert!(!before.teams.is_empty() && !old_roster.entries.is_empty());
        assert!(old_titles.title.is_some() && old_titles.action_bar.is_some());
        assert!(client.tab_list().await.unwrap().text.is_some());
        assert!(!client.boss_bars().await.unwrap().bars.is_empty());
        assert!(
            !client
                .scoreboard_state()
                .await
                .unwrap()
                .objectives
                .is_empty()
        );
        let reset = {
            let mut state = session.state.lock().await;
            state
                .receive(ids::play_clientbound::START_CONFIGURATION, &[], 256)
                .unwrap();
            assert_eq!(state.phase, Phase::Configuration);
            state.sequence
        };
        // Read-only captures expose missing facts during configuration;
        // only mutations require the completed play/loading baseline.
        assert!(client.player_state().await.unwrap().position.is_none());
        assert!(client.teams().await.unwrap().teams.is_empty());
        assert!(client.player_list().await.unwrap().entries.is_empty());
        assert!(client.survival().select_hotbar(0).await.is_err());
        assert!(client.creative().select_hotbar(0).await.is_err());
        // Re-establish the fixture's own-player prerequisites, without seeding any UI.
        {
            let mut state = session.state.lock().await;
            state.phase = Phase::Play;
            state.ready = true;
            state.position = Some([0.0, 65.0, 0.0]);
            state.world.select_dimension(
                "minecraft:overworld".into(),
                Dimension::new(-64, 384).unwrap(),
            );
            state.loading = loading::InteractionLoading::completed_fixture();
            state.loading.generation = reset;
            state.loading.attempt.as_mut().unwrap().generation = reset;
            state
                .operations
                .reset_world(if mode == GameMode::Survival { 0 } else { 1 })
                .unwrap();
            state.operations.local_player = operations::LocalPlayerState::spawned(42);
        }
        let teams = client.teams().await.unwrap();
        let roster = client.player_list().await.unwrap();
        assert!(teams.teams.is_empty() && roster.entries.is_empty());
        assert_eq!(teams.context_reset_sequence, Some(reset));
        assert_eq!(roster.context_reset_sequence, Some(reset));
        assert!(teams.last_update_sequence.is_none() && roster.last_update_sequence.is_none());
        let titles = client.titles().await.unwrap();
        assert!(
            titles.title.is_none()
                && titles.subtitle.is_none()
                && titles.clear.is_none()
                && titles.timing.is_none()
        );
        assert_eq!(titles.action_bar, old_titles.action_bar);
        assert_eq!(titles.context_reset_sequence, Some(reset));
        assert!(client.tab_list().await.unwrap().text.is_none());
        let bars = client.boss_bars().await.unwrap();
        assert!(bars.bars.is_empty());
        assert_eq!(bars.context_reset_sequence, Some(reset));
        let board = client.scoreboard_state().await.unwrap();
        assert!(board.objectives.is_empty());
        assert_eq!(board.context_reset_sequence, Some(reset));
        // A partial update from the old registration cannot recreate either record.
        let changes = [
            social
                .iter()
                .find(|r| r["group"] == "team" && r["operation"] == 2)
                .unwrap(),
            social
                .iter()
                .find(|r| r["group"] != "team" && r["flags"] == 4)
                .unwrap(),
        ];
        {
            let mut state = session.state.lock().await;
            for row in changes {
                state
                    .receive(
                        row["packet_id"].as_i64().unwrap() as i32,
                        &bridge_bytes(row),
                        256,
                    )
                    .unwrap();
            }
        }
        assert!(
            client.teams().await.unwrap().teams.is_empty()
                && client.player_list().await.unwrap().entries.is_empty()
        );
        {
            let mut state = session.state.lock().await;
            for row in selected
                .iter()
                .filter(|r| r["group"] == "team" || r["flags"] == 255)
            {
                state
                    .receive(
                        row["packet_id"].as_i64().unwrap() as i32,
                        &bridge_bytes(row),
                        256,
                    )
                    .unwrap();
            }
        }
        assert!(
            !client.teams().await.unwrap().teams.is_empty()
                && !client.player_list().await.unwrap().entries.is_empty()
        );
        assert_eq!(
            client.teams().await.unwrap().context_reset_sequence,
            Some(reset)
        );
        assert_eq!(before.context_reset_sequence, None); // Detached observations remain unchanged.
        client.disconnect().await.unwrap();
    }
}

#[tokio::test]
async fn common_respawn_uses_received_death_once_and_retains_actual_new_world() {
    use crate::client::{GameMode, RespawnStage};
    for mode in [GameMode::Survival, GameMode::Creative] {
        let (session, api, mut peer) = common_ground_fixture(mode).await;
        let client = crate::Client::from_java_1_21_11(api.bot.clone());
        assert!(client.respawn().await.is_err());
        assert!(client.respawn_record().is_none());
        let mut health = 0f32.to_be_bytes().to_vec();
        health.push(20);
        health.extend(5f32.to_be_bytes());
        session
            .state
            .lock()
            .await
            .receive(ids::play_clientbound::UPDATE_HEALTH, &health, 256)
            .unwrap();
        let sent = client.respawn().await.unwrap();
        assert!(sent.dispatched);
        assert_eq!(sent.stage, RespawnStage::Submitted);
        assert_eq!(
            read_packet(&mut peer, None).await.unwrap(),
            (ids::play_serverbound::CLIENT_COMMAND, vec![0])
        );
        assert!(client.clone().respawn().await.is_err());
        let mut spawn = vec![0];
        put_string(&mut spawn, "minecraft:overworld");
        spawn.extend([0; 8]);
        spawn.extend([
            if mode == GameMode::Creative { 1 } else { 0 },
            255,
            0,
            1,
            0,
            0,
            63,
            0,
        ]);
        let received_sequence = {
            let mut state = session.state.lock().await;
            state.dimensions = vec![Dimension::new(-64, 384).unwrap()];
            state
                .receive(ids::play_clientbound::RESPAWN, &spawn, 256)
                .unwrap();
            state.sequence
        };
        let fresh = client.respawn_record().unwrap();
        assert_eq!(fresh.stage, RespawnStage::RespawnReceived);
        let received = fresh.received_spawn.unwrap();
        assert_eq!(received.value.connection_id, sent.session.connection_id);
        assert_eq!(received.value.world_generation, received_sequence);
        assert_eq!(
            received.source,
            crate::client::ValueSource::Received {
                sequence: received_sequence
            }
        );
        assert!(sent.received_spawn.is_none());
        assert!(client.respawn().await.is_err()); // No fresh dead health or ready baseline.
        session.stop();
        assert!(client.respawn_record().unwrap().dispatched);
        assert!(
            tokio::time::timeout(Duration::from_millis(30), read_packet(&mut peer, None))
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn common_respawn_cancelled_waiter_keeps_owned_send_and_readable_history() {
    use crate::client::{GameMode, RespawnStage};
    let (session, api, mut peer) = common_ground_fixture(GameMode::Survival).await;
    let client = crate::Client::from_java_1_21_11(api.bot);
    let mut health = 0f32.to_be_bytes().to_vec();
    health.push(20);
    health.extend(5f32.to_be_bytes());
    session
        .state
        .lock()
        .await
        .receive(ids::play_clientbound::UPDATE_HEALTH, &health, 256)
        .unwrap();
    let writer = session.writer.lock().await;
    let waiter = tokio::spawn({
        let client = client.clone();
        async move { client.respawn().await }
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while client.respawn_record().is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        client.respawn_record().unwrap().stage,
        RespawnStage::Prepared
    );
    assert!(!client.respawn_record().unwrap().dispatched);
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    drop(writer);
    assert_eq!(
        read_packet(&mut peer, None).await.unwrap(),
        (ids::play_serverbound::CLIENT_COMMAND, vec![0])
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while !client.respawn_record().unwrap().dispatched {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(client.respawn().await.is_err());
    session.stop();
    assert_eq!(
        client.respawn_record().unwrap().stage,
        RespawnStage::Submitted
    );
}

#[tokio::test]
async fn common_respawn_settling_requires_owned_world_and_only_released_inputs() {
    use crate::client::prelude::{GameMode, SurvivalControl, SurvivalInput};
    for mode in [GameMode::Survival, GameMode::Creative] {
        let (session, api, mut peer) = common_ground_fixture(mode).await;
        let client = crate::Client::from_java_1_21_11(api.bot.clone());
        let controls = vec![
            SurvivalControl {
                yaw: 0.0,
                input: SurvivalInput::default()
            };
            4
        ];
        let mut health = 0f32.to_be_bytes().to_vec();
        health.push(20);
        health.extend(5f32.to_be_bytes());
        session
            .state
            .lock()
            .await
            .receive(ids::play_clientbound::UPDATE_HEALTH, &health, 256)
            .unwrap();
        client.respawn().await.unwrap();
        read_packet(&mut peer, None).await.unwrap();
        {
            let mut state = session.state.lock().await;
            let floor = std::mem::take(&mut state.world);
            state.dimensions = vec![Dimension::new(-64, 384).unwrap()];
            let mut spawn = vec![0];
            put_string(&mut spawn, "minecraft:overworld");
            spawn.extend([0; 8]);
            spawn.extend([
                if mode == GameMode::Creative { 1 } else { 0 },
                255,
                0,
                1,
                0,
                0,
                63,
                0,
            ]);
            state
                .receive(ids::play_clientbound::RESPAWN, &spawn, 256)
                .unwrap();
            let generation = state.loading.generation;
            state.world = floor; // Unit fixture's received static floor for the new world.
            let mut pose = vec![3];
            for v in [8.5f64, 65.1, 8.5, 0., 0., 0.] {
                pose.extend(v.to_be_bytes());
            }
            for v in [0f32, 0.] {
                pose.extend(v.to_be_bytes());
            }
            pose.extend(0i32.to_be_bytes());
            state
                .receive(ids::play_clientbound::POSITION, &pose, 256)
                .unwrap();
            health[..4].copy_from_slice(&20f32.to_be_bytes());
            state
                .receive(ids::play_clientbound::UPDATE_HEALTH, &health, 256)
                .unwrap();
            state.ready = true;
            state.loading = loading::InteractionLoading::completed_fixture();
            state.loading.generation = generation;
        }
        let preview = api.preview_path(mode, &controls).await.unwrap();
        assert!(!preview.initial_frame.on_ground);
        assert_eq!(
            preview.initial.received_pose.as_ref().unwrap().position,
            [8.5, 65.1, 8.5]
        );
        let last = preview.frames.last().unwrap();
        assert_eq!(last.position, [8.5, 65.0, 8.5]);
        assert!(last.on_ground && last.resting);
        assert_eq!(
            client.player_state().await.unwrap().received_pose,
            preview.initial.received_pose
        );
        let active = [SurvivalControl {
            yaw: 0.,
            input: SurvivalInput {
                forward: 1,
                ..Default::default()
            },
        }; 4];
        assert!(api.preview_path(mode, &active).await.is_err());
        assert!(client.survival().target_block(4.5).await.is_err()); // No landing authority from a preview.
        let history = api.bot.respawn_history.clone();
        let saved = history.lock().unwrap().take().unwrap();
        assert!(api.preview_path(mode, &controls).await.is_err());
        *history.lock().unwrap() = Some(saved);
        {
            let mut state = session.state.lock().await;
            state
                .operations
                .local_player
                .health
                .as_mut()
                .unwrap()
                .health = f32::NAN;
        }
        assert!(api.preview_path(mode, &controls).await.is_err());
        session
            .state
            .lock()
            .await
            .operations
            .local_player
            .health
            .as_mut()
            .unwrap()
            .health = 20.;
        session
            .state
            .lock()
            .await
            .operations
            .local_player
            .velocity
            .as_mut()
            .unwrap()
            .value = [0.1, 0., 0.];
        assert!(api.preview_path(mode, &controls).await.is_err());
    }
}

#[tokio::test]
async fn common_entity_history_freezes_native_packets_and_reports_retention() {
    use crate::client::{
        EntityHistoryKind as K, MAX_ENTITY_HISTORY_READ, MAX_ENTITY_HISTORY_RECORDS,
    };
    let (session, api, _peer) = common_ground_fixture(crate::client::GameMode::Survival).await;
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    macro_rules! receive {
        ($id:expr,$body:expr) => {
            session
                .state
                .lock()
                .await
                .receive($id, &$body, 256)
                .unwrap()
        };
    }
    let baseline = client
        .entity_history_after(None, 1)
        .await
        .unwrap()
        .latest_cursor;
    let make_spawn = |id: u8, name: &str, living: bool| {
        let mut p = vec![id];
        p.extend([id; 16]);
        put_varint(
            &mut p,
            client
                .registry()
                .builtin_id("minecraft:entity_type", name)
                .unwrap()
                .value(),
        );
        for v in [2f64, 65., 0.5] {
            p.extend(v.to_be_bytes());
        }
        let _ = living;
        p.extend([0; 5]);
        p
    };
    let sheep = make_spawn(42, "minecraft:sheep", true);
    let fireball = make_spawn(43, "minecraft:fireball", false);
    receive!(ids::play_clientbound::SPAWN_ENTITY, sheep.clone());
    receive!(ids::play_clientbound::SPAWN_ENTITY, fireball.clone());
    let mut move_look = vec![42];
    for d in [4096i16, 0, 0] {
        move_look.extend(d.to_be_bytes());
    }
    move_look.extend([32, 16, 1]);
    receive!(ids::play_clientbound::ENTITY_MOVE_LOOK, move_look);
    let mut velocity = vec![43];
    let packed = 1u64 | (32766u64 << 3) | (32766u64 << 18) | (32766u64 << 33);
    velocity.extend([packed as u8, (packed >> 8) as u8]);
    velocity.extend(((packed >> 16) as u32).to_be_bytes());
    receive!(ids::play_clientbound::ENTITY_VELOCITY, velocity);
    let zero_velocity = vec![43, 0];
    receive!(
        ids::play_clientbound::ENTITY_VELOCITY,
        zero_velocity.clone()
    );
    let mut status = 42i32.to_be_bytes().to_vec();
    status.push(2);
    receive!(ids::play_clientbound::ENTITY_STATUS, status);
    receive!(ids::play_clientbound::ANIMATION, [42, 1]);
    receive!(ids::play_clientbound::ENTITY_DESTROY, [2, 42, 43]);
    let page = client
        .entity_history_after(Some(baseline), 1024)
        .await
        .unwrap();
    assert!(page.gap.is_none());
    assert!(!page.has_more);
    assert_eq!(page.records.len(), 9);
    crate::client::tests::history_quiet_read_clock_scenario(&client).await;
    assert!(page.records.windows(2).all(|r| r[0].ordinal < r[1].ordinal
        && r[0].receive_sequence <= r[1].receive_sequence
        && r[0].applied_after <= r[1].applied_after));
    let K::Spawn(sheep_sample) = &page.records[0].kind else {
        panic!()
    };
    let K::Spawn(projectile) = &page.records[1].kind else {
        panic!()
    };
    assert_eq!(
        sheep_sample.entity.type_name.as_deref(),
        Some("minecraft:sheep")
    );
    assert_eq!(
        projectile.entity.type_name.as_deref(),
        Some("minecraft:fireball")
    );
    let K::Motion(moved) = &page.records[2].kind else {
        panic!()
    };
    assert_eq!(
        moved.position.as_ref().unwrap().value.position,
        [3., 65., 0.5]
    );
    assert_eq!(moved.rotation.as_ref().unwrap().value, [45., 22.5]);
    assert_eq!(
        sheep_sample.position.as_ref().unwrap().value.position,
        [2., 65., 0.5]
    );
    let K::Motion(first_velocity) = &page.records[3].kind else {
        panic!()
    };
    let K::Motion(last_velocity) = &page.records[4].kind else {
        panic!()
    };
    assert_eq!(first_velocity.velocity.as_ref().unwrap().value, [1.; 3]);
    assert_eq!(last_velocity.velocity.as_ref().unwrap().value, [0.; 3]);
    assert_ne!(
        first_velocity.velocity.as_ref().unwrap().source,
        last_velocity.velocity.as_ref().unwrap().source
    );
    assert!(matches!(page.records[5].kind, K::Status { status: 2, .. }));
    assert!(matches!(
        page.records[6].kind,
        K::Animation { animation: 1, .. }
    ));
    assert!(matches!(
        page.records[7].kind,
        K::Removed { native_id: 42, .. }
    ));
    assert!(matches!(
        page.records[8].kind,
        K::Removed { native_id: 43, .. }
    ));
    assert_eq!(
        page.records[7].receive_sequence,
        page.records[8].receive_sequence
    );
    let original = projectile.entity.id;
    receive!(ids::play_clientbound::SPAWN_ENTITY, fireball.clone());
    let reused = client.entity_spawns().await.unwrap().entities[0].id;
    assert_ne!(original, reused);
    let mut respawn = vec![0];
    put_string(&mut respawn, "minecraft:overworld");
    respawn.extend([0; 8]);
    respawn.extend([0, 255, 0, 1, 0, 0, 63, 0]);
    session.state.lock().await.dimensions = vec![Dimension::new(-64, 384).unwrap()];
    receive!(ids::play_clientbound::RESPAWN, respawn);
    receive!(ids::play_clientbound::SPAWN_ENTITY, fireball);
    let fresh = client.entity_spawns().await.unwrap().entities[0].id;
    assert_ne!(
        reused.session().world_generation,
        fresh.session().world_generation
    );
    let worlds = client
        .entity_history_after(Some(page.next_cursor), 1024)
        .await
        .unwrap();
    assert!(
        worlds
            .records
            .iter()
            .any(|r| matches!(r.kind, K::WorldChanged { .. }))
    );
    assert!(
        worlds
            .records
            .iter()
            .any(|r| r.session.world_generation == reused.session().world_generation)
    );
    assert!(
        worlds
            .records
            .iter()
            .any(|r| r.session.world_generation == fresh.session().world_generation)
    );
    assert!(client.entity_history_after(None, 0).await.is_err());
    assert!(
        client
            .entity_history_after(None, MAX_ENTITY_HISTORY_READ + 1)
            .await
            .is_err()
    );
    for _ in 0..MAX_ENTITY_HISTORY_RECORDS + 100 {
        receive!(
            ids::play_clientbound::ENTITY_VELOCITY,
            zero_velocity.clone()
        );
    }
    let mut page = client
        .entity_history_after(Some(baseline), MAX_ENTITY_HISTORY_READ)
        .await
        .unwrap();
    assert!(page.gap.is_some());
    assert!(page.has_more);
    assert_eq!(page.records.len(), 1024);
    let dropped = page.gap.unwrap().dropped_through;
    assert_eq!(page.records[0].ordinal, dropped + 1);
    let mut count = page.records.len();
    while page.has_more {
        page = client
            .entity_history_after(Some(page.next_cursor), MAX_ENTITY_HISTORY_READ)
            .await
            .unwrap();
        assert!(page.gap.is_none());
        count += page.records.len();
    }
    assert_eq!(count, MAX_ENTITY_HISTORY_RECORDS);
    let tail = page.next_cursor;
    let _ = client.revoke_connection();
    let closed = client
        .entity_history_after(None, MAX_ENTITY_HISTORY_READ)
        .await
        .unwrap();
    assert!(closed.gap.is_some());
    assert!(
        client
            .entity_history_after(Some(tail), 1)
            .await
            .unwrap()
            .records
            .is_empty()
    );
}
