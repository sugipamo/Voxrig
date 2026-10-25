use super::*;
use std::time::Duration;
use tokio::net::TcpListener;

async fn fixture() -> (Bot, OwnedReadHalf, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let stream = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (peer, _) = listener.accept().await.unwrap();
    let (reader, writer) = stream.into_split();
    let dimension = Dimension::new(-64, 384).unwrap();
    let mut state = State {
        phase: Phase::Play,
        dimensions: vec![dimension],
        sequence: 1,
        ..State::default()
    };
    state
        .world
        .select_dimension("minecraft:overworld".into(), dimension);
    state.loading.reset(1);
    let id = 900;
    let session = Arc::new(Session {
        operational_dig: Arc::default(),
        id,
        ledger: Ledger::tracking(GenerationKey {
            version: MinecraftVersion::Java1_21_11,
            connection_id: id,
        }),
        started: Instant::now(),
        writer: Arc::new(Mutex::new(Writer {
            stream: SocketWriter::new(writer),
            compression: None,
        })),
        state: Mutex::new(state),
        changed: Notify::new(),
        cancel: Notify::new(),
        stopped: AtomicBool::new(false),
        revoked: AtomicBool::new(false),
        receiver_abort: std::sync::OnceLock::new(),
        interrupted_packet: AtomicI32::new(-1),
        limits: crate::client::ClientLimits::default(),
        interaction_sequence: AtomicI32::new(0),
    });
    let bot = Bot {
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
        _lease: Arc::new(Lease(Arc::downgrade(&session))),
    };
    (bot, reader, peer)
}
async fn receive(peer: &mut TcpStream, id: i32, bytes: &[u8]) {
    write_packet(peer, None, id, bytes).await.unwrap();
}
fn chunk() -> Vec<u8> {
    let mut bytes = vec![0; 9];
    let mut sections = Vec::new();
    for _ in 0..24 {
        sections.extend([0; 6]);
    }
    put_varint(&mut bytes, sections.len() as i32);
    bytes.extend(sections);
    bytes.extend([0; 7]);
    bytes
}
async fn position(peer: &mut TcpStream) {
    receive(peer, ids::play_clientbound::POSITION, &[0; 61]).await;
    assert_eq!(
        read_packet(peer, None).await.unwrap(),
        (ids::play_serverbound::TELEPORT_CONFIRM, vec![0])
    );
    assert_eq!(
        read_packet(peer, None).await.unwrap().0,
        ids::play_serverbound::POSITION_LOOK
    );
}
async fn no_packet(peer: &mut TcpStream) {
    assert!(
        timeout(Duration::from_millis(15), read_packet(peer, None))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn received_terrain_and_initial_chunks_gate_ordered_notification_once_per_world() {
    let (bot, reader, mut peer) = fixture().await;
    let receiver = {
        let s = bot.session.clone();
        tokio::spawn(async move { s.run_receiver(reader).await })
    };
    receive(
        &mut peer,
        ids::play_clientbound::GAME_STATE_CHANGE,
        &[13, 0, 0, 0, 0],
    )
    .await;
    position(&mut peer).await;
    no_packet(&mut peer).await;
    assert!(
        timeout(Duration::from_millis(15), bot.wait_until_ready())
            .await
            .is_err()
    );
    assert!(bot.operations().select_hotbar(0).await.is_err());
    assert!(
        bot.operations()
            .operation_history()
            .await
            .interaction_loading
            .attempt
            .is_none()
    );
    receive(&mut peer, ids::play_clientbound::MAP_CHUNK, &chunk()).await;
    assert_eq!(
        read_packet(&mut peer, None).await.unwrap(),
        (ids::play_serverbound::PLAYER_LOADED, vec![])
    );
    bot.wait_until_ready().await.unwrap();
    let first = bot
        .operations()
        .operation_history()
        .await
        .interaction_loading;
    assert!(first.notification_dispatched());
    bot.operations().select_hotbar(0).await.unwrap();
    assert_eq!(
        read_packet(&mut peer, None).await.unwrap().0,
        ids::play_serverbound::HELD_ITEM_SLOT
    );
    receive(
        &mut peer,
        ids::play_clientbound::GAME_STATE_CHANGE,
        &[13, 0, 0, 0, 0],
    )
    .await;
    receive(&mut peer, ids::play_clientbound::MAP_CHUNK, &chunk()).await;
    no_packet(&mut peer).await;

    let mut respawn = vec![0];
    put_string(&mut respawn, "minecraft:overworld");
    respawn.extend([0; 8]);
    respawn.extend([0, 255, 0, 1, 0, 0, 63, 0]);
    receive(&mut peer, ids::play_clientbound::RESPAWN, &respawn).await;
    // Reasserted held input is ordered after RESPawn and before the new pose;
    // its dispatch does not make the world ready or invent a held-slot receipt.
    assert_eq!(
        read_packet(&mut peer, None).await.unwrap(),
        (ids::play_serverbound::HELD_ITEM_SLOT, vec![0, 0])
    );
    {
        let state = bot.session.state.lock().await;
        let selection = state.operations.selected_hotbar.as_ref().unwrap();
        assert!(selection.dispatched && !selection.from_server);
        assert!(!state.ready);
    }
    position(&mut peer).await;
    receive(&mut peer, ids::play_clientbound::MAP_CHUNK, &chunk()).await;
    no_packet(&mut peer).await;
    assert!(bot.operations().select_hotbar(0).await.is_err());
    receive(
        &mut peer,
        ids::play_clientbound::GAME_STATE_CHANGE,
        &[13, 0, 0, 0, 0],
    )
    .await;
    assert_eq!(
        read_packet(&mut peer, None).await.unwrap().0,
        ids::play_serverbound::PLAYER_LOADED
    );
    bot.wait_until_ready().await.unwrap();
    let next = bot
        .operations()
        .operation_history()
        .await
        .interaction_loading;
    assert!(next.generation > first.generation);
    assert_eq!(next.previous_attempt.unwrap().generation, first.generation);
    bot.disconnect().await.unwrap();
    receiver.await.unwrap();
}

#[tokio::test]
async fn cancelled_notification_retains_attempt_and_configuration_invalidates_its_authority() {
    let (bot, _, mut peer) = fixture().await;
    {
        let mut state = bot.session.state.lock().await;
        state.ready = true;
        state.position = Some([0.0; 3]);
        state.world.seed_replay_cell([0; 3], 0);
        state.loading.initial_chunks_sequence = Some(2);
    }
    let writer = bot.session.writer.lock().await;
    assert!(
        timeout(Duration::from_millis(15), async {
            let mut state = bot.session.state.lock().await;
            bot.session.complete_loading(&mut state).await
        })
        .await
        .is_err()
    );
    drop(writer);
    let history = bot.operations().operation_history().await;
    assert!(!history.interaction_loading.attempt.unwrap().dispatched);
    assert!(!history.connection_closed); // Cancelled before acquiring the sender.
    assert!(bot.operations().select_hotbar(0).await.is_err());
    {
        let mut state = bot.session.state.lock().await;
        bot.session.complete_loading(&mut state).await.unwrap();
        state
            .receive(ids::play_clientbound::START_CONFIGURATION, &[], 64)
            .unwrap();
        assert!(state.loading.attempt.is_none());
        assert!(!state.loading.previous_attempt.as_ref().unwrap().dispatched);
        bot.session.complete_loading(&mut state).await.unwrap();
    }
    no_packet(&mut peer).await;
    bot.disconnect().await.unwrap();
    assert!(
        !bot.operations()
            .operation_history()
            .await
            .interaction_loading
            .notification_dispatched()
    );
}

#[tokio::test]
async fn common_state_waits_cover_present_racing_and_closed_state() {
    let (bot, _reader, _peer) = fixture().await;
    tokio::time::pause();
    let position = [8, 64, 8];
    {
        let mut state = bot.session.state.lock().await;
        state.world.seed_replay_cell(position, 1);
        state
            .receive(
                ids::play_clientbound::SYSTEM_CHAT,
                &[8, 0, 4, b'w', b'a', b'i', b't', 0],
                256,
            )
            .unwrap();
    }
    let client = crate::Client::from_java_1_21_11(bot.clone());
    let dirt = super::super::super::state_id(&crate::NativeBlockState {
        name: "minecraft:dirt".into(),
        properties: Default::default(),
    })
    .unwrap();
    crate::Client::assert_common_state_waits(&client, position, || {
        let mut state = bot.session.state.try_lock().unwrap();
        state.world.seed_replay_cell(position, dirt);
        state.sequence += 1;
        bot.session.changed.notify_waiters();
    })
    .await;
}

#[tokio::test]
async fn common_waits_complete_pending_and_observation_registration_races() {
    let (bot, _reader, _peer) = fixture().await;
    tokio::time::pause();
    let client = crate::Client::from_java_1_21_11(bot.clone());
    crate::Client::assert_common_wait_progress(&client, |position| {
        let bot = bot.clone();
        async move {
            let mut state = bot.session.state.lock().await;
            if let Some(position) = position {
                state.world.seed_replay_cell(position, 1);
                state.sequence += 1;
            } else {
                state
                    .receive(
                        ids::play_clientbound::SYSTEM_CHAT,
                        &[8, 0, 8, b'p', b'r', b'o', b'g', b'r', b'e', b's', b's', 0],
                        256,
                    )
                    .unwrap();
            }
            bot.session.changed.notify_waiters();
        }
    })
    .await;
}
