use super::*;

fn plain(name: &str, count: u8) -> InventorySlot {
    InventorySlot::Item {
        item: default_item(name, count).unwrap(),
    }
}
fn put_slot(payload: &mut Vec<u8>, value: &InventorySlot) {
    match value {
        InventorySlot::Empty => payload.push(0),
        InventorySlot::Item { item } => {
            put_varint(payload, item.count);
            put_varint(payload, item.item_id);
            payload.extend([0, 0]);
        }
        InventorySlot::Unavailable => panic!("cannot encode unknown inventory"),
    }
}
fn full(inventory: &Inventory, revision: i32) -> Vec<u8> {
    let mut payload = vec![0];
    put_varint(&mut payload, revision);
    payload.push(46);
    for value in &inventory.slots {
        put_slot(&mut payload, value);
    }
    put_slot(&mut payload, &inventory.cursor);
    payload
}
fn slot_update(index: u16, value: &InventorySlot, revision: i32) -> Vec<u8> {
    let mut payload = vec![0];
    put_varint(&mut payload, revision);
    payload.extend(index.to_be_bytes());
    put_slot(&mut payload, value);
    payload
}
fn baseline() -> State {
    let mut state = State {
        sequence: 10,
        ready: true,
        loading: loading::InteractionLoading::completed_fixture(),
        ..State::default()
    };
    state.operations.game_mode = Some(GameMode::Survival);
    let mut inventory = Inventory {
        cursor: InventorySlot::Empty,
        ..Default::default()
    };
    inventory.slots.fill(InventorySlot::Empty);
    inventory.slots[9] = plain("stone", 32);
    inventory.slots[36] = plain("dirt", 12);
    super::super::receive(
        &mut state,
        ids::play_clientbound::WINDOW_ITEMS,
        &full(&inventory, 128),
    )
    .unwrap();
    state
}

struct CommonFixture {
    api: Operations,
    peer: tokio::net::TcpStream,
    receiver: tokio::task::JoinHandle<()>,
}
impl CommonFixture {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let stream = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (peer, _) = listener.accept().await.unwrap();
        let (reader, writer) = stream.into_split();
        let mut state = baseline();
        state.phase = Phase::Play;
        let session = Arc::new(Session {
            id: 42,
            started: Instant::now(),
            writer: Mutex::new(Writer {
                stream: writer,
                compression: None,
            }),
            state: Mutex::new(state),
            changed: Notify::new(),
            cancel: Notify::new(),
            stopped: AtomicBool::new(false),
            interrupted_packet: AtomicI32::new(-1),
            limits: crate::client::ClientLimits::default(),
            interaction_sequence: AtomicI32::new(0),
        });
        let api = Operations {
            bot: Bot {
                session: session.clone(),
                _lease: Arc::new(Lease(Arc::downgrade(&session))),
            },
        };
        let receiver = tokio::spawn(async move {
            session.run_receiver(reader).await;
        });
        Self {
            api,
            peer,
            receiver,
        }
    }
    fn client(&self) -> crate::Client {
        crate::Client::from_java_1_21_11(self.api.bot.clone())
    }
    async fn receive(&mut self, id: i32, payload: &[u8]) {
        let before = self.api.bot.session.state.lock().await.sequence;
        write_packet(&mut self.peer, None, id, payload)
            .await
            .unwrap();
        timeout(Duration::from_secs(1), async {
            while self.api.bot.session.state.lock().await.sequence == before {
                tokio::task::yield_now().await
            }
        })
        .await
        .unwrap();
    }
    async fn slot(&mut self, index: u16, value: InventorySlot) {
        self.receive(
            ids::play_clientbound::SET_SLOT,
            &slot_update(index, &value, 130),
        )
        .await;
    }
    async fn stop(self) {
        self.api.bot.disconnect().await.unwrap();
        self.receiver.await.unwrap();
    }
}
#[tokio::test]
async fn common_swap_same_consumer_receives_both_destinations_then_next_creative_empty_swap() {
    use crate::client::{GameMode as Mode, inventory::InventorySwapStage};
    let mut f = CommonFixture::new().await;
    let client = f.client();
    let record =
        crate::client::tests::common_swap_start_scenario(&client, Mode::Survival, 9, 0).await;
    assert_eq!(
        read_packet(&mut f.peer, None).await.unwrap(),
        (
            ids::play_serverbound::WINDOW_CLICK,
            vec![0, 128, 1, 0, 9, 0, 2, 0, 0]
        )
    );
    assert!(record.send.legacy_action.is_none() && record.legacy_reply.is_none());
    assert_eq!(record.send.screen_revision, Some(128));
    let submission = f
        .api
        .bot
        .session
        .state
        .lock()
        .await
        .operations
        .inventory
        .pending_swap
        .clone()
        .unwrap();
    assert!(
        f.api
            .wait_inventory_swap(&submission, Duration::from_millis(1))
            .await
            .is_err()
    );
    f.slot(9, submission.hotbar_before.clone()).await;
    crate::client::tests::common_swap_pending_scenario(&client).await;
    // Direct raw-player hotbar update maps to canonical player-screen slot 36.
    let mut raw = vec![0];
    put_slot(&mut raw, &submission.main_before);
    f.receive(ids::play_clientbound::SET_PLAYER_INVENTORY, &raw)
        .await;
    let complete = crate::client::tests::common_swap_completed_scenario(&client, record.id).await;
    assert!(complete.legacy_reply.is_none());
    f.slot(9, InventorySlot::Empty).await;
    assert_eq!(
        client
            .survival()
            .inventory_swap_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        InventorySwapStage::ObservedSwapped
    );
    let mut mode = vec![3];
    mode.extend(1f32.to_be_bytes());
    f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &mode)
        .await;
    let next =
        crate::client::tests::common_swap_start_scenario(&client, Mode::Creative, 9, 0).await;
    assert_ne!(next.id, record.id);
    read_packet(&mut f.peer, None).await.unwrap();
    let next_submission = f
        .api
        .bot
        .session
        .state
        .lock()
        .await
        .operations
        .inventory
        .pending_swap
        .clone()
        .unwrap();
    f.slot(9, next_submission.hotbar_before).await;
    f.slot(36, next_submission.main_before).await;
    crate::client::tests::common_swap_completed_scenario(&client, next.id).await;
    f.api.bot.disconnect().await.unwrap();
    assert_eq!(
        client
            .creative()
            .inventory_swap_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        InventorySwapStage::ObservedSwapped
    );
    f.receiver.await.unwrap();
}
#[tokio::test]
async fn common_swap_transient_destination_and_screen_conflicts_stay_latched() {
    use crate::client::inventory::InventorySwapStage;
    let mut f = CommonFixture::new().await;
    let client = f.client();
    client.survival().swap_hotbar(9, 0).await.unwrap();
    read_packet(&mut f.peer, None).await.unwrap();
    let submission = f
        .api
        .bot
        .session
        .state
        .lock()
        .await
        .operations
        .inventory
        .pending_swap
        .clone()
        .unwrap();
    f.slot(9, submission.hotbar_before.clone()).await;
    f.slot(9, submission.main_before.clone()).await;
    f.slot(9, submission.hotbar_before.clone()).await;
    f.slot(36, submission.main_before.clone()).await;
    let result = client
        .survival()
        .inventory_swap_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.stage, InventorySwapStage::RequiresInspection);
    assert!(
        result
            .requires_inspection
            .unwrap()
            .contains("changed again")
    );
    assert!(
        f.api
            .wait_inventory_swap(&submission, Duration::from_millis(1))
            .await
            .is_err()
    );
    assert!(f.api.select_hotbar(0).await.is_err());
    f.stop().await;
}
#[tokio::test]
async fn common_swap_cancelled_before_writer_cannot_confirm_from_other_actor_updates() {
    use crate::client::inventory::InventorySwapStage;
    let mut f = CommonFixture::new().await;
    let client = f.client();
    let session = f.api.bot.session.clone();
    let writer = session.writer.lock().await;
    let ops = client.survival();
    let waiter = tokio::spawn(async move { ops.swap_hotbar(9, 0).await });
    timeout(Duration::from_secs(1), async {
        while session.state.try_lock().is_ok() {
            tokio::task::yield_now().await
        }
    })
    .await
    .unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    drop(writer);
    let submission = session
        .state
        .lock()
        .await
        .operations
        .inventory
        .pending_swap
        .clone()
        .unwrap();
    assert!(
        !session
            .state
            .lock()
            .await
            .common_inventory_swap
            .as_ref()
            .unwrap()
            .record
            .send
            .dispatched
    );
    f.slot(9, submission.hotbar_before).await;
    f.slot(36, submission.main_before).await;
    let result = client
        .survival()
        .inventory_swap_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.stage, InventorySwapStage::Pending);
    assert!(!result.send.dispatched);
    assert!(
        timeout(Duration::from_millis(10), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    assert!(client.survival().swap_hotbar(9, 0).await.is_err());
    f.api.bot.disconnect().await.unwrap();
    assert_eq!(
        client
            .survival()
            .inventory_swap_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        InventorySwapStage::RequiresInspection
    );
    f.receiver.await.unwrap();
}
#[tokio::test]
async fn common_swap_requires_actual_empty_cursor_and_latches_transient_mode_change() {
    use crate::client::inventory::InventorySwapStage;
    let mut f = CommonFixture::new().await;
    let client = f.client();
    f.api
        .bot
        .session
        .state
        .lock()
        .await
        .operations
        .inventory
        .cursor_sequence = None;
    assert!(client.survival().swap_hotbar(9, 0).await.is_err());
    assert!(
        f.api
            .bot
            .session
            .state
            .lock()
            .await
            .common_inventory_swap
            .is_none()
    );
    assert!(
        timeout(Duration::from_millis(10), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &[0])
        .await;
    client.survival().swap_hotbar(9, 0).await.unwrap();
    read_packet(&mut f.peer, None).await.unwrap();
    let mut mode = vec![3];
    mode.extend(1f32.to_be_bytes());
    f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &mode)
        .await;
    mode[1..].copy_from_slice(&0f32.to_be_bytes());
    f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &mode)
        .await;
    let submission = f
        .api
        .bot
        .session
        .state
        .lock()
        .await
        .operations
        .inventory
        .pending_swap
        .clone()
        .unwrap();
    f.slot(9, submission.hotbar_before).await;
    f.slot(36, submission.main_before).await;
    assert_eq!(
        client
            .survival()
            .inventory_swap_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        InventorySwapStage::RequiresInspection
    );
    f.stop().await;
}

#[test]
fn player_cursor_revision_and_slot_updates_preserve_atomic_receive() {
    let mut state = baseline();
    assert_eq!(state.operations.inventory.window_id, Some(0));
    assert_eq!(state.operations.inventory.screen_revision, Some(128));
    assert_eq!(state.operations.inventory.cursor, InventorySlot::Empty);
    let update = slot_update(9, &plain("glass", 7), 129);
    for length in 0..update.len() {
        let before = serde_json::to_value(&state.operations.inventory).unwrap();
        assert!(
            super::super::receive(
                &mut state,
                ids::play_clientbound::SET_SLOT,
                &update[..length]
            )
            .is_err()
        );
        assert_eq!(
            serde_json::to_value(&state.operations.inventory).unwrap(),
            before
        );
    }
    state.sequence = 11;
    super::super::receive(&mut state, ids::play_clientbound::SET_SLOT, &update).unwrap();
    assert_eq!(state.operations.inventory.screen_revision, Some(129));
    assert_eq!(state.operations.inventory.slot_sequences[9], Some(11));
    let mut cursor = Vec::new();
    put_slot(&mut cursor, &plain("stone", 1));
    super::super::receive(&mut state, ids::play_clientbound::SET_CURSOR_ITEM, &cursor).unwrap();
    assert!(prepare(&state.operations.inventory, 1, 11, 9, 0).is_err());
    for malformed in [
        vec![0, 255, 255, 255, 255, 15, 46], // negative screen revision
        vec![0, 0, 129, 8],                  // excessive inventory count
        vec![0, 0, 45],                      // incorrect player slot count
        vec![0, 0, 255, 255, 0],             // negative slot
        vec![0, 0, 0, 46, 0],                // slot outside player inventory
    ] {
        let id = if malformed.len() == 5 {
            ids::play_clientbound::SET_SLOT
        } else {
            ids::play_clientbound::WINDOW_ITEMS
        };
        let before = serde_json::to_value(&state.operations.inventory).unwrap();
        assert!(super::super::receive(&mut state, id, &malformed).is_err());
        assert_eq!(
            serde_json::to_value(&state.operations.inventory).unwrap(),
            before
        );
    }
}

#[test]
fn swap_admission_never_turns_unknown_or_pending_state_into_permission() {
    let state = baseline();
    let baseline = state.operations.inventory;
    let (submission, payload) = prepare(&baseline, 42, 10, 9, 0).unwrap();
    assert_eq!(payload, [0, 128, 1, 0, 9, 0, 2, 0, 0]);
    let cases: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../data/java_1_21_11/inventory_swap_packets.json"
    ))
    .unwrap();
    for case in cases.as_array().unwrap() {
        let mut received = baseline.clone();
        received.screen_revision = Some(case["revision"].as_i64().unwrap() as i32);
        let main = case["main_slot"].as_u64().unwrap() as u8;
        let hotbar = case["hotbar"].as_u64().unwrap() as u8;
        received.slots[usize::from(main)] = plain("stone", 32);
        received.slots[36 + usize::from(hotbar)] = InventorySlot::Empty;
        assert_eq!(
            prepare(&received, 42, 10, main, hotbar).unwrap().1,
            hex::decode(case["payload_hex"].as_str().unwrap()).unwrap()
        );
    }
    assert_eq!(baseline.slots[9], plain("stone", 32));
    for (main, hotbar) in [(8, 0), (36, 0), (9, 9)] {
        assert_eq!(
            prepare(&baseline, 42, 10, main, hotbar).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
    }
    assert!(prepare(&baseline, 42, 10, 10, 1).is_err()); // two empty slots
    let mut changed = baseline.clone();
    changed.pending_swap = Some(submission);
    assert!(prepare(&changed, 42, 10, 9, 1).is_err());
    changed = baseline.clone();
    changed.screen_revision = None;
    assert!(prepare(&changed, 42, 10, 9, 0).is_err());
    changed = baseline.clone();
    changed.slots[9] = InventorySlot::Unavailable;
    assert!(prepare(&changed, 42, 10, 9, 0).is_err());
    changed = baseline.clone();
    changed.cursor = InventorySlot::Unavailable;
    assert!(prepare(&changed, 42, 10, 9, 0).is_err());
    changed = baseline.clone();
    if let InventorySlot::Item { item } = &mut changed.slots[9] {
        item.count = 96;
    }
    assert_eq!(
        prepare(&changed, 42, 10, 9, 0).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    changed = baseline;
    changed.unsupported_components = true;
    assert!(prepare(&changed, 42, 10, 9, 0).is_err());
}

#[test]
fn confirmation_requires_both_post_submission_destinations() {
    let mut state = baseline();
    let (submission, _) = prepare(&state.operations.inventory, 42, 10, 9, 0).unwrap();
    state.operations.inventory.pending_swap = Some(submission.clone());
    // Matching values without new per-slot packets cannot confirm an action.
    state.operations.inventory.slots.swap(9, 36);
    assert!(observed(&state.operations.inventory, &submission, 100).is_none());
    state.sequence = 11;
    super::super::receive(
        &mut state,
        ids::play_clientbound::SET_SLOT,
        &slot_update(9, &submission.hotbar_before, 129),
    )
    .unwrap();
    assert!(observed(&state.operations.inventory, &submission, 11).is_none());
    state.sequence = 12;
    super::super::receive(
        &mut state,
        ids::play_clientbound::SET_SLOT,
        &slot_update(36, &submission.main_before, 130),
    )
    .unwrap();
    let proof = observed(&state.operations.inventory, &submission, 12).unwrap();
    assert_eq!((proof.main_sequence, proof.hotbar_sequence), (11, 12));
    assert_eq!(proof.submission.connection_id, 42);
    // A foreign container invalidates the cursor/revision and keeps uncertainty.
    super::super::receive(&mut state, ids::play_clientbound::WINDOW_ITEMS, &[1, 0, 0]).unwrap();
    assert_eq!(
        state.operations.inventory.pending_swap.as_ref(),
        Some(&submission)
    );
    assert!(observed(&state.operations.inventory, &submission, 13).is_none());
    assert!(prepare(&state.operations.inventory, 42, 13, 9, 0).is_err());
}

#[test]
fn window_changes_and_unsupported_cursor_cannot_restore_stale_swap_authority() {
    let mut state = baseline();
    let full_player = full(&state.operations.inventory, 129);
    let open = [1, 0, 10, 0]; // container 1, type 0, empty title NBT
    super::super::receive(&mut state, ids::play_clientbound::OPEN_WINDOW, &open).unwrap();
    assert_eq!(state.operations.inventory.window_id, Some(1));
    super::super::receive(
        &mut state,
        ids::play_clientbound::WINDOW_ITEMS,
        &full_player,
    )
    .unwrap();
    assert_eq!(state.operations.inventory.window_id, Some(1));
    assert!(prepare(&state.operations.inventory, 42, 10, 9, 0).is_err());
    super::super::receive(&mut state, ids::play_clientbound::CLOSE_WINDOW, &[1]).unwrap();
    assert_eq!(state.operations.inventory.window_id, None);
    assert!(prepare(&state.operations.inventory, 42, 10, 9, 0).is_err());
    super::super::receive(
        &mut state,
        ids::play_clientbound::WINDOW_ITEMS,
        &full_player,
    )
    .unwrap();
    assert!(prepare(&state.operations.inventory, 42, 10, 9, 0).is_ok());
    let mut cursor = vec![1];
    put_varint(&mut cursor, default_item("stone", 1).unwrap().item_id);
    cursor.extend([1, 0]); // component payload remains deliberately unsupported
    super::super::receive(&mut state, ids::play_clientbound::SET_CURSOR_ITEM, &cursor).unwrap();
    assert_eq!(
        state.operations.inventory.cursor,
        InventorySlot::Unavailable
    );
    assert!(prepare(&state.operations.inventory, 42, 10, 9, 0).is_err());
}

#[tokio::test]
async fn ordinary_click_uses_real_transport_and_timeout_never_resubmits() {
    use super::super::super::{Lease, Phase, Session, Writer};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let connection = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (mut peer, _) = listener.accept().await.unwrap();
    let (reader, writer) = connection.into_split();
    let mut state = baseline();
    state.phase = Phase::Play;
    let session = Arc::new(Session {
        id: 42,
        started: Instant::now(),
        writer: Mutex::new(Writer {
            stream: writer,
            compression: None,
        }),
        state: Mutex::new(state),
        changed: Notify::new(),
        cancel: Notify::new(),
        stopped: AtomicBool::new(false),
        interrupted_packet: AtomicI32::new(-1),
        limits: crate::client::ClientLimits::default(),
        interaction_sequence: AtomicI32::new(0),
    });
    let operations = Operations {
        bot: Bot {
            session: session.clone(),
            _lease: Arc::new(Lease(Arc::downgrade(&session))),
        },
    };
    let running = session.clone();
    let receiver = tokio::spawn(async move { running.receive_loop(reader).await });
    let submission = operations.swap_player_hotbar(9, 0).await.unwrap();
    let (id, payload) = read_packet(&mut peer, None).await.unwrap();
    assert_eq!(id, ids::play_serverbound::WINDOW_CLICK);
    assert_eq!(payload, [0, 128, 1, 0, 9, 0, 2, 0, 0]);
    assert_eq!(
        operations.player_state().await.unwrap().inventory.slots[9],
        submission.main_before
    );
    assert_eq!(
        operations
            .swap_player_hotbar(9, 1)
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::State
    );
    assert_eq!(
        operations.select_hotbar(1).await.unwrap_err().kind(),
        ErrorKind::State
    );
    assert_eq!(
        operations
            .use_on_block([0, 0, 0], crate::BlockFace::Up, [0.5; 3])
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::State
    );
    assert_eq!(
        operations
            .wait_inventory_swap(&submission, Duration::from_millis(5))
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::Timeout
    );
    assert!(
        operations
            .player_state()
            .await
            .unwrap()
            .inventory
            .pending_swap
            .is_some()
    );
    write_packet(
        &mut peer,
        None,
        ids::play_clientbound::SET_SLOT,
        &slot_update(9, &submission.hotbar_before, 129),
    )
    .await
    .unwrap();
    assert_eq!(
        operations
            .wait_inventory_swap(&submission, Duration::from_millis(10))
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::Timeout
    );
    write_packet(
        &mut peer,
        None,
        ids::play_clientbound::SET_SLOT,
        &slot_update(36, &submission.main_before, 130),
    )
    .await
    .unwrap();
    let proof = operations
        .wait_inventory_swap(&submission, Duration::from_secs(1))
        .await
        .unwrap();
    assert!(proof.hotbar_sequence > proof.main_sequence);
    assert!(
        operations
            .player_state()
            .await
            .unwrap()
            .inventory
            .pending_swap
            .is_none()
    );
    assert!(
        timeout(Duration::from_millis(10), read_packet(&mut peer, None))
            .await
            .is_err()
    );
    let reverse = operations.swap_player_hotbar(9, 0).await.unwrap();
    assert_eq!(
        read_packet(&mut peer, None).await.unwrap().0,
        ids::play_serverbound::WINDOW_CLICK
    );
    write_packet(
        &mut peer,
        None,
        ids::play_clientbound::SET_SLOT,
        &slot_update(36, &reverse.main_before, 131),
    )
    .await
    .unwrap();
    write_packet(
        &mut peer,
        None,
        ids::play_clientbound::SET_SLOT,
        &slot_update(9, &reverse.hotbar_before, 132),
    )
    .await
    .unwrap();
    operations
        .wait_inventory_swap(&reverse, Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(
        operations.player_state().await.unwrap().inventory.slots[9],
        submission.main_before
    );
    let mut foreign = reverse;
    foreign.connection_id = 99;
    assert_eq!(
        operations
            .wait_inventory_swap(&foreign, Duration::from_millis(1))
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::State
    );
    operations.bot.disconnect().await.unwrap();
    receiver.await.unwrap().unwrap();
}
