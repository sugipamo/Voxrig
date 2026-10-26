use super::*;
use crate::versions::java_1_21_11::{state_id, world::Dimension};
use tokio::{
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
const SUPPORT: [i32; 3] = [2, 2, 0];
const TARGET: [i32; 3] = [1, 2, 0];

#[tokio::test]
async fn captured_placement_uses_the_live_native_cursor_and_target_checks() {
    let f = Fixture::new().await;
    let scene = f
        .api
        .capture_survival_scene(crate::Region {
            min: [-2, 0, -2],
            max: [4, 6, 2],
        })
        .await
        .unwrap();
    let imagined = scene
        .scenario()
        .preview_cube_placement(SUPPORT, crate::BlockFace::West, [-90.0, 3.0], "dirt")
        .unwrap();
    let mut state = f.api.bot.session.state.lock().await;
    let actual = prepare(
        &mut state,
        f.api.bot.session.id,
        0,
        SUPPORT,
        crate::BlockFace::West as u8,
        false,
    )
    .unwrap();
    assert_eq!(imagined.cursor, actual.cursor);
    assert_eq!(imagined.edit.position, actual.target);
    assert_eq!(imagined.edit.before, actual.before);
    assert_eq!(imagined.edit.after, actual.expected);
    assert_eq!(imagined.face_id, actual.face_id);
    drop(state);
    f.stop().await;
}
fn native(name: &str) -> crate::NativeBlockState {
    crate::NativeBlockState {
        name: format!("minecraft:{name}"),
        properties: Default::default(),
    }
}
fn fixture_state() -> State {
    let mut state = State {
        phase: Phase::Play,
        ready: true,
        position: Some([0.5, 1.0, 0.5]),
        rotation: [-90.0, 3.0],
        sequence: 10,
        loading: loading::InteractionLoading::completed_fixture(),
        ..State::default()
    };
    state.operations.reset_world(0).unwrap();
    state.operations.local_player = LocalPlayerState::spawned(42);
    state.operations.local_player.velocity = Some(VelocitySample {
        value: [0.0; 3],
        receive_sequence: 10,
    });
    state.operations.local_player.health = Some(PlayerHealth {
        health: 20.0,
        food: 20,
        saturation: 5.0,
        receive_sequence: 10,
    });
    state.motion.receive(ReceivedPose {
        generation: state.loading.generation,
        receive_sequence: state.sequence,
        position: state.position.unwrap(),
        rotation: state.rotation,
        velocity: Some([0.0; 3]),
    });
    state.world.select_dimension(
        "minecraft:overworld".into(),
        Dimension::new(-64, 384).unwrap(),
    );
    for x in -2..=4 {
        for z in -2..=2 {
            state.world.seed_replay_cell([x, 0, z], 1);
        }
    }
    state
        .world
        .seed_replay_cell(SUPPORT, state_id(&native("stone")).unwrap());
    let mut slots = vec![0, 1, 46];
    for slot in 0..46 {
        if slot == 36 {
            slots.push(5);
            put_varint(&mut slots, default_item("dirt", 5).unwrap().item_id);
            slots.extend([0, 0]);
        } else {
            slots.push(0);
        }
    }
    slots.push(0);
    super::super::receive(&mut state, ids::play_clientbound::WINDOW_ITEMS, &slots).unwrap();
    state.operations.selected_hotbar = Some(HotbarSelection {
        slot: 0,
        sequence: 10,
        dispatched: true,
        from_server: true,
    });
    state
}
struct Fixture {
    api: Operations,
    peer: TcpStream,
    receiver: JoinHandle<()>,
}
impl Fixture {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (peer, _) = listener.accept().await.unwrap();
        let (reader, writer) = stream.into_split();
        let id = 72;
        let state = fixture_state();
        let health_log = state.health_log.clone();
        let session = Arc::new(Session {
            health_log: Arc::clone(&health_log),
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
        let api = Operations {
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
    async fn receive(&mut self, id: i32, payload: &[u8]) {
        let before = self.api.bot.session.state.lock().await.sequence;
        write_packet(&mut self.peer, None, id, payload)
            .await
            .unwrap();
        timeout(Duration::from_secs(1), async {
            while self.api.bot.session.state.lock().await.sequence == before {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
    async fn expected_packet(&mut self, context: &str) -> (i32, Vec<u8>) {
        timeout(Duration::from_secs(1), read_packet(&mut self.peer, None))
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {context}"))
            .unwrap_or_else(|error| panic!("failed to read {context}: {error}"))
    }
    async fn start(&mut self) -> PlacementIntent {
        let intent = self
            .api
            .place_survival_cube(SUPPORT, crate::BlockFace::West)
            .await
            .unwrap();
        assert_eq!(intent.target, TARGET);
        let (id, p) = self
            .expected_packet("BLOCK_PLACE after native placement")
            .await;
        assert_eq!(id, ids::play_serverbound::BLOCK_PLACE);
        let mut r = Reader::new(&p);
        assert_eq!(r.varint().unwrap(), 0);
        assert_eq!(
            super::super::super::super::wire::unpack_position(r.u64().unwrap()),
            SUPPORT
        );
        assert_eq!(r.varint().unwrap(), 4);
        assert_eq!(r.f32().unwrap(), 0.0);
        assert!((0.0..1.0).contains(&r.f32().unwrap()));
        assert_eq!(r.f32().unwrap(), 0.5);
        assert!(!r.bool().unwrap());
        assert!(!r.bool().unwrap());
        assert_eq!(r.varint().unwrap(), intent.sequence);
        r.end().unwrap();
        intent
    }
    async fn block(&mut self, p: [i32; 3], name: &str) {
        let mut payload = pack_position(p).to_be_bytes().to_vec();
        put_varint(&mut payload, state_id(&native(name)).unwrap());
        self.receive(ids::play_clientbound::BLOCK_CHANGE, &payload)
            .await;
    }
    async fn stack(&mut self, count: i32) {
        let mut p = vec![0, 2];
        p.extend(36i16.to_be_bytes());
        put_varint(&mut p, count);
        if count > 0 {
            put_varint(&mut p, default_item("dirt", 1).unwrap().item_id);
            p.extend([0, 0]);
        }
        self.receive(ids::play_clientbound::SET_SLOT, &p).await;
    }
    async fn ack(&mut self, sequence: i32) {
        let mut p = Vec::new();
        put_varint(&mut p, sequence);
        self.receive(ids::play_clientbound::ACKNOWLEDGE_PLAYER_DIGGING, &p)
            .await;
    }
    async fn stop(self) {
        self.api.bot.disconnect().await.unwrap();
        timeout(Duration::from_secs(1), self.receiver)
            .await
            .unwrap()
            .unwrap();
    }
}
#[tokio::test]
async fn placement_requires_both_receipts_and_processed_sequence_in_either_order() {
    for inventory_first in [false, true] {
        let mut f = Fixture::new().await;
        let i = f.start().await;
        assert!(f.api.select_hotbar(1).await.is_err());
        assert!(
            f.api
                .use_on_block(SUPPORT, crate::BlockFace::West, [0.5; 3])
                .await
                .is_err()
        );
        assert!(
            f.api
                .place_survival_cube(SUPPORT, crate::BlockFace::West)
                .await
                .is_err()
        );
        f.ack(i.sequence).await;
        assert!(matches!(
            f.api
                .wait_survival_placement(&i, Duration::from_millis(5))
                .await
                .unwrap(),
            PlacementStatus::Pending { .. }
        ));
        if inventory_first {
            f.stack(4).await;
        } else {
            f.block(TARGET, "dirt").await;
        }
        assert!(matches!(
            f.api.observe_survival_placement(&i).await.unwrap(),
            PlacementStatus::Pending { .. }
        ));
        if inventory_first {
            f.block(TARGET, "dirt").await;
        } else {
            f.stack(4).await;
        }
        let PlacementStatus::ObservedPlaced { observation } = f
            .api
            .wait_survival_placement(&i, Duration::from_secs(1))
            .await
            .unwrap()
        else {
            panic!("missing both receipts");
        };
        assert_eq!(
            observation.held_after,
            InventorySlot::Item {
                item: default_item("dirt", 4).unwrap()
            }
        );
        f.api.select_hotbar(1).await.unwrap();
        assert_eq!(
            f.expected_packet("HELD_ITEM_SLOT after releasing placement")
                .await
                .0,
            ids::play_serverbound::HELD_ITEM_SLOT
        );
        f.stop().await;
    }
}
#[tokio::test]
async fn missing_ack_last_item_and_conflicting_receipts_do_not_silently_release() {
    let mut f = Fixture::new().await;
    f.stack(1).await;
    let i = f.start().await;
    f.block(TARGET, "dirt").await;
    f.stack(0).await;
    assert!(matches!(
        f.api.observe_survival_placement(&i).await.unwrap(),
        PlacementStatus::Pending { .. }
    ));
    f.ack(i.sequence).await;
    assert!(matches!(
        f.api.observe_survival_placement(&i).await.unwrap(),
        PlacementStatus::ObservedPlaced { .. }
    ));
    f.stop().await;
    for inventory_conflict in [false, true] {
        let mut f = Fixture::new().await;
        let i = f.start().await;
        if inventory_conflict {
            f.stack(3).await;
            f.stack(4).await;
        } else {
            f.block(TARGET, "dirt").await;
            f.block(TARGET, "air").await;
            f.block(TARGET, "dirt").await;
        }
        f.ack(i.sequence).await;
        assert!(matches!(
            f.api.observe_survival_placement(&i).await.unwrap(),
            PlacementStatus::RequiresInspection { .. }
        ));
        assert!(f.api.select_hotbar(1).await.is_err());
        f.stop().await;
    }
}
#[tokio::test]
async fn cancelled_send_retains_intent_without_replay_and_closed_history_survives() {
    let f = Fixture::new().await;
    let writer = f.api.bot.session.writer.lock().await;
    assert!(
        timeout(
            Duration::from_millis(10),
            f.api.place_survival_cube(SUPPORT, crate::BlockFace::West)
        )
        .await
        .is_err()
    );
    drop(writer);
    let h = f.api.operation_history().await;
    assert!(!h.placement.unwrap().dispatched);
    assert!(
        f.api
            .place_survival_cube(SUPPORT, crate::BlockFace::West)
            .await
            .is_err()
    );
    f.api.bot.disconnect().await.unwrap();
    assert!(f.api.operation_history().await.placement.is_some());
    f.stop().await;
}
#[test]
fn admission_rejects_body_unknown_target_wrong_hit_empty_or_unsupported_material() {
    let mut s = fixture_state();
    assert!(prepare(&mut s, 1, 0, SUPPORT, 4, false).is_ok());
    assert!(prepare(&mut s, 1, 0, SUPPORT, 1, false).is_err());
    s.world.seed_replay_cell(TARGET, 1);
    assert!(prepare(&mut s, 1, 0, SUPPORT, 4, false).is_err());
    let mut s = fixture_state();
    s.operations.inventory.slots[36] = InventorySlot::Empty;
    assert!(prepare(&mut s, 1, 0, SUPPORT, 4, false).is_err());
    s.operations.inventory.slots[36] = InventorySlot::Item {
        item: default_item("sand", 5).unwrap(),
    };
    assert!(prepare(&mut s, 1, 0, SUPPORT, 4, false).is_err());
    assert!(survival::standing_intersects([0.5, 1.0, 0.5], [0, 2, 0]));
    assert!(!survival::standing_intersects([0.5, 1.0, 0.5], TARGET));
}

#[test]
fn packet_and_admitted_materials_match_native_oracle() {
    let oracle: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../data/java_1_21_11/survival_placement.json"
    ))
    .unwrap();
    let names: Vec<_> = oracle["materials"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        survival::DRY_CUBES
            .iter()
            .copied()
            .filter(|n| *n != "minecraft:grass_block")
            .collect::<Vec<_>>()
    );
    let mut s = fixture_state();
    for m in oracle["materials"].as_array().unwrap() {
        let item = default_item(m["name"].as_str().unwrap(), 5).unwrap();
        assert_eq!(i64::from(item.item_id), m["item_id"].as_i64().unwrap());
        s.operations.inventory.slots[36] = InventorySlot::Item { item };
        let i = prepare(&mut s, 1, 0, SUPPORT, 4, false).unwrap();
        assert_eq!(
            i64::from(state_id(&i.expected).unwrap()),
            m["state_id"].as_i64().unwrap()
        );
    }
    let mut i = prepare(&mut s, 1, 0, SUPPORT, 4, false).unwrap();
    i.support = [-2, -61, 3];
    i.sequence = 17;
    for p in oracle["packets"].as_array().unwrap() {
        i.face_id = p["face"].as_u64().unwrap() as u8;
        i.cursor = offset(i.face_id).map(|d| 0.5 + d as f32 * 0.5);
        assert_eq!(hex::encode(packet(&i)), p["hex"].as_str().unwrap());
    }
}
#[tokio::test]
async fn chunk_world_and_restored_material_conflicts_remain_latched() {
    for kind in 0..3 {
        let mut f = Fixture::new().await;
        let i = f.start().await;
        if kind == 0 {
            f.stack(4).await;
            f.stack(5).await;
            f.stack(4).await;
        } else if kind == 1 {
            f.receive(ids::play_clientbound::UNLOAD_CHUNK, &[0; 8])
                .await;
        } else {
            f.receive(
                ids::play_clientbound::GAME_STATE_CHANGE,
                &[3, 0x3f, 0x80, 0, 0],
            )
            .await;
            f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &[3, 0, 0, 0, 0])
                .await;
        }
        assert!(matches!(
            f.api.observe_survival_placement(&i).await.unwrap(),
            PlacementStatus::RequiresInspection { .. }
        ));
        assert!(f.api.select_hotbar(1).await.is_err());
        f.stop().await;
    }
}

#[tokio::test]
async fn common_placement_same_consumer_requires_real_processing_and_retains_native_guards() {
    let mut f = Fixture::new().await;
    let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
    let record = crate::client::tests::common_placement_start_scenario(
        &client,
        SUPPORT,
        crate::BlockFace::West,
        TARGET,
    )
    .await;
    assert!(record.send.interaction_sequence.is_some());
    assert_eq!(
        f.expected_packet("BLOCK_PLACE after common placement")
            .await
            .0,
        ids::play_serverbound::BLOCK_PLACE
    );
    f.block(TARGET, "dirt").await;
    f.stack(4).await;
    crate::client::tests::common_placement_pending_scenario(&client).await;
    f.ack(record.send.interaction_sequence.unwrap()).await;
    let completed =
        crate::client::tests::common_placement_completed_scenario(&client, record.id).await;
    f.expected_packet("HELD_ITEM_SLOT after completed placement")
        .await;
    assert!(completed.processing.as_ref().unwrap().receive_sequence > record.send.after_sequence);
    f.block(TARGET, "air").await;
    assert_eq!(
        client
            .survival()
            .placement_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        crate::client::survival::PlacementStage::ObservedPlaced
    );
    f.block([-2, 2, 0], "stone").await;
    client.survival().look([90.0, 3.0]).await.unwrap();
    f.expected_packet("LOOK before next placement").await;
    let next = client
        .survival()
        .place_cube([-2, 2, 0], crate::BlockFace::East)
        .await
        .unwrap();
    assert_ne!(next.id, record.id);
    assert_eq!(next.held_before.count, 4);
    f.expected_packet("BLOCK_PLACE for next placement").await;
    f.stop().await;
}
#[tokio::test]
async fn common_placement_after_no_echo_close_keeps_opening_history_and_reopen_conflicts() {
    use crate::client::{container::PlayerScreenAccess, survival::PlacementStage};
    for reopen in [false, true] {
        let mut f = Fixture::new().await;
        f.receive(ids::play_clientbound::OPEN_WINDOW, &[3, 2, 10, 0])
            .await;
        let mut full = vec![3, 5, 63];
        for slot in 0..63 {
            if slot == 54 {
                full.push(5);
                put_varint(&mut full, default_item("dirt", 5).unwrap().item_id);
                full.extend([0, 0]);
            } else {
                full.push(0);
            }
        }
        full.push(0);
        f.receive(ids::play_clientbound::WINDOW_ITEMS, &full).await;
        let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
        let screen = client.screen_state().await.unwrap().screen.unwrap().id;
        assert!(
            client
                .survival()
                .place_cube(SUPPORT, crate::BlockFace::West)
                .await
                .is_err()
        );
        let close = client.survival().close_container(screen).await.unwrap();
        assert_eq!(
            f.expected_packet("CLOSE_WINDOW after common close").await,
            (ids::play_serverbound::CLOSE_WINDOW, vec![3])
        );
        crate::client::tests::common_closed_player_screen_scenario(&client, close.id).await;
        // The native-only checked API still requires its original received-player-screen contract.
        assert!(
            f.api
                .place_survival_cube(SUPPORT, crate::BlockFace::West)
                .await
                .is_err()
        );
        let record = client
            .survival()
            .place_cube(SUPPORT, crate::BlockFace::West)
            .await
            .unwrap();
        assert_eq!(
            f.expected_packet("BLOCK_PLACE after submitted close")
                .await
                .0,
            ids::play_serverbound::BLOCK_PLACE
        );
        assert_eq!(record.initial.inventory.window_id, Some(3));
        assert_eq!(
            record.initial.inventory.player_screen,
            Some(PlayerScreenAccess::SubmittedClose { close: close.id })
        );
        if reopen {
            f.receive(ids::play_clientbound::OPEN_WINDOW, &[3, 2, 10, 0])
                .await;
            assert!(
                client
                    .player_state()
                    .await
                    .unwrap()
                    .inventory
                    .player_screen
                    .is_none()
            );
        }
        f.block(TARGET, "dirt").await;
        f.stack(4).await;
        f.ack(record.send.interaction_sequence.unwrap()).await;
        let final_record = client.survival().placement_record().await.unwrap().unwrap();
        assert_eq!(
            final_record.stage,
            if reopen {
                PlacementStage::RequiresInspection
            } else {
                PlacementStage::ObservedPlaced
            }
        );
        if reopen {
            assert!(client.survival().select_hotbar(1).await.is_err());
        }
        f.stop().await;
    }
}
#[tokio::test]
async fn common_placement_cancelled_before_writer_retains_intent_without_inventing_dispatch_or_release()
 {
    let mut f = Fixture::new().await;
    let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
    let guard = f.api.bot.session.writer.lock().await;
    let ops = client.survival();
    let mut submit = Box::pin(ops.place_cube(SUPPORT, crate::BlockFace::West));
    std::future::poll_fn(|cx| {
        assert!(submit.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(submit);
    drop(guard);
    let record = client.survival().placement_record().await.unwrap().unwrap();
    assert!(!record.send.dispatched);
    assert_eq!(
        record.stage,
        crate::client::survival::PlacementStage::Pending
    );
    f.block(TARGET, "dirt").await;
    f.stack(4).await;
    f.ack(record.send.interaction_sequence.unwrap()).await;
    assert_ne!(
        client
            .survival()
            .placement_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        crate::client::survival::PlacementStage::ObservedPlaced
    );
    assert!(
        client
            .survival()
            .place_cube(SUPPORT, crate::BlockFace::West)
            .await
            .is_err()
    );
    f.stop().await;
    assert_eq!(
        client
            .survival()
            .placement_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        crate::client::survival::PlacementStage::RequiresInspection
    );
}
#[tokio::test]
async fn common_placement_transient_support_loss_and_missing_cursor_cannot_be_erased() {
    let mut f = Fixture::new().await;
    let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
    f.api
        .bot
        .session
        .state
        .lock()
        .await
        .operations
        .inventory
        .cursor_sequence = None;
    assert!(
        client
            .survival()
            .place_cube(SUPPORT, crate::BlockFace::West)
            .await
            .is_err()
    );
    assert!(f.api.bot.session.state.lock().await.placement.is_none());
    f.api
        .bot
        .session
        .state
        .lock()
        .await
        .operations
        .inventory
        .cursor_sequence = Some(10);
    let record = client
        .survival()
        .place_cube(SUPPORT, crate::BlockFace::West)
        .await
        .unwrap();
    f.expected_packet("BLOCK_PLACE before support loss").await;
    f.block([0, 0, 0], "air").await;
    f.block([0, 0, 0], "stone").await;
    f.block(TARGET, "dirt").await;
    f.stack(4).await;
    f.ack(record.send.interaction_sequence.unwrap()).await;
    let conflict = client.survival().placement_record().await.unwrap().unwrap();
    assert_eq!(
        conflict.stage,
        crate::client::survival::PlacementStage::RequiresInspection
    );
    assert!(
        conflict
            .requires_inspection
            .unwrap()
            .contains("standing prerequisites")
    );
    f.stop().await;
}

#[tokio::test]
async fn common_placement_requires_fresh_ack_and_preserves_completed_common_history_on_native_only_switch()
 {
    let mut f = Fixture::new().await;
    let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
    {
        let mut s = f.api.bot.session.state.lock().await;
        s.operations.ack = Some(1);
        s.operations.ack_receive_sequence = Some(1);
    }
    let record = client
        .survival()
        .place_cube(SUPPORT, crate::BlockFace::West)
        .await
        .unwrap();
    f.expected_packet("BLOCK_PLACE before fresh acknowledgement")
        .await;
    f.block(TARGET, "dirt").await;
    f.stack(4).await;
    crate::client::tests::common_placement_pending_scenario(&client).await;
    assert!(
        client
            .survival()
            .placement_record()
            .await
            .unwrap()
            .unwrap()
            .processing
            .is_none()
    );
    f.ack(record.send.interaction_sequence.unwrap()).await;
    let completed =
        crate::client::tests::common_placement_completed_scenario(&client, record.id).await;
    f.expected_packet("HELD_ITEM_SLOT after fresh acknowledgement")
        .await;
    f.block([-2, 2, 0], "stone").await;
    client.survival().look([90.0, 3.0]).await.unwrap();
    f.expected_packet("LOOK before native-only placement").await;
    let native = f
        .api
        .place_survival_cube([-2, 2, 0], crate::BlockFace::East)
        .await
        .unwrap();
    f.expected_packet("BLOCK_PLACE after native-only placement")
        .await;
    assert_ne!(Some(native.sequence), completed.send.interaction_sequence);
    let historical = client.survival().placement_record().await.unwrap().unwrap();
    assert_eq!(historical.id, completed.id);
    assert_eq!(
        historical.stage,
        crate::client::survival::PlacementStage::ObservedPlaced
    );
    assert_eq!(
        historical.initial.received_pose,
        completed.initial.received_pose
    );
    f.stop().await;
}

fn replacement_fixture_state(name: &str) -> crate::NativeBlockState {
    let blocks: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../data/java_1_21_11/blocks.json"
    ))
    .unwrap();
    let id = blocks
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap()["minStateId"]
        .as_i64()
        .unwrap();
    crate::versions::java_1_21_11::native_state(id as i32).unwrap()
}
async fn replacement_block(f: &mut Fixture, target: [i32; 3], block: &crate::NativeBlockState) {
    let mut payload = pack_position(target).to_be_bytes().to_vec();
    put_varint(&mut payload, state_id(block).unwrap());
    f.receive(ids::play_clientbound::BLOCK_CHANGE, &payload)
        .await;
}
async fn replacement_start(
    f: &mut Fixture,
    clicked: bool,
    count: i32,
) -> crate::client::survival::ReplaceableCubePlacementRecord {
    use crate::client::adapter::PlacementOps;
    f.stack(count).await;
    let destination = if clicked { SUPPORT } else { TARGET };
    f.block(destination, "short_grass").await;
    {
        let mut state = f.api.bot.session.state.lock().await;
        // Adjacent case looks past the plant's inset outline to the cube face.
        state.rotation = [-90.0, if clicked { 10.0 } else { 3.0 }];
        state.position = Some([0.5, 1.0, if clicked { 0.5 } else { 0.05 }]);
        let pose = ReceivedPose {
            generation: state.loading.generation,
            receive_sequence: state.sequence,
            position: state.position.unwrap(),
            rotation: state.rotation,
            velocity: Some([0.0; 3]),
        };
        state.motion.receive(pose);
    }
    let record = f
        .api
        .place_cube_replacing(
            SUPPORT,
            crate::BlockFace::West,
            destination,
            native("short_grass"),
        )
        .await
        .unwrap();
    assert_eq!(record.target, destination);
    assert_eq!(record.replaces_clicked, clicked);
    assert_eq!(record.first_outline.position, SUPPORT);
    assert_eq!(record.first_outline.state, record.support_state);
    assert_eq!(
        record.stage,
        crate::client::survival::PlacementStage::Pending
    );
    let (id, bytes) = f.expected_packet("replacement BLOCK_PLACE").await;
    assert_eq!(id, ids::play_serverbound::BLOCK_PLACE);
    let mut reader = Reader::new(&bytes);
    assert_eq!(reader.varint().unwrap(), 0);
    assert_eq!(
        reader.take(8).unwrap(),
        pack_position(SUPPORT).to_be_bytes()
    );
    assert_eq!(reader.varint().unwrap(), crate::BlockFace::West as i32);
    for coordinate in record.cursor {
        assert_eq!(reader.f32().unwrap(), coordinate);
    }
    assert!(!reader.bool().unwrap());
    assert!(!reader.bool().unwrap());
    assert_eq!(
        reader.varint().unwrap(),
        record.send.interaction_sequence.unwrap()
    );
    reader.end().unwrap();
    record
}

#[tokio::test]
async fn replacement_has_separate_block_material_ack_receipts_and_frozen_history() {
    use crate::client::{adapter::PlacementOps, survival::PlacementStage};
    for clicked in [false, true] {
        for count in [1, 5] {
            let mut f = Fixture::new().await;
            let sent = replacement_start(&mut f, clicked, count).await;
            assert!(
                f.api
                    .place_cube(SUPPORT, crate::BlockFace::West)
                    .await
                    .is_err()
            );
            assert!(f.api.select_hotbar(1).await.is_err());
            assert!(f.api.placement_record().await.unwrap().is_none());
            f.ack(sent.send.interaction_sequence.unwrap()).await;
            f.stack(count - 1).await;
            let partial = f
                .api
                .replaceable_cube_placement_record()
                .await
                .unwrap()
                .unwrap();
            assert_eq!(partial.stage, PlacementStage::Pending);
            assert!(partial.material_receipt.is_some() && partial.processing.is_some());
            assert!(partial.target_receipt.is_none());
            f.block(sent.target, "dirt").await;
            let complete = f
                .api
                .replaceable_cube_placement_record()
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                complete.stage,
                PlacementStage::ObservedPlaced,
                "{:?}",
                complete.requires_inspection
            );
            let frozen = serde_json::to_value(&complete).unwrap();
            // Later unrelated receipts cannot rewrite the completed attempt.
            f.block(sent.target, "stone").await;
            f.stack(12).await;
            f.ack(sent.send.interaction_sequence.unwrap() + 1).await;
            assert_eq!(
                serde_json::to_value(
                    f.api
                        .replaceable_cube_placement_record()
                        .await
                        .unwrap()
                        .unwrap()
                )
                .unwrap(),
                frozen
            );
            assert!(f.api.placement_record().await.unwrap().is_none());
            f.stop().await;
        }
    }
}

#[tokio::test]
async fn replacement_rejects_wrong_predecessor_destination_nonplants_and_preserves_air_api() {
    use crate::client::adapter::PlacementOps;
    for before in [
        "stone",
        "water",
        "snow",
        "vine",
        "air",
        "short_grass",
        "fern",
        "dead_bush",
    ] {
        let mut f = Fixture::new().await;
        let predecessor = replacement_fixture_state(before);
        replacement_block(&mut f, TARGET, &predecessor).await;
        assert!(
            f.api
                .place_cube_replacing(SUPPORT, crate::BlockFace::West, TARGET, native("dead_bush"))
                .await
                .is_err()
                || before == "dead_bush"
        );
        if before != "dead_bush" {
            assert!(
                f.api
                    .place_cube_replacing(
                        SUPPORT,
                        crate::BlockFace::West,
                        [1, 3, 0],
                        predecessor.clone()
                    )
                    .await
                    .is_err()
            );
            let mut properties = predecessor;
            properties
                .properties
                .insert("unsupported".into(), "true".into());
            assert!(
                f.api
                    .place_cube_replacing(SUPPORT, crate::BlockFace::West, TARGET, properties)
                    .await
                    .is_err()
            );
            if before != "air" {
                assert!(
                    f.api
                        .place_cube(SUPPORT, crate::BlockFace::West)
                        .await
                        .is_err()
                );
            }
        }
        f.stop().await;
    }
}

#[tokio::test]
async fn replacement_rejects_exact_nonplant_predecessors_without_dispatch() {
    use crate::client::adapter::PlacementOps;
    for name in ["stone", "water", "snow", "vine", "air"] {
        let mut f = Fixture::new().await;
        let predecessor = replacement_fixture_state(name);
        replacement_block(&mut f, TARGET, &predecessor).await;
        // Use the actual received state: a mismatched predecessor would reject
        // even if the audited destination restriction were accidentally removed.
        for clicked in [SUPPORT, TARGET] {
            assert!(
                f.api
                    .place_cube_replacing(
                        clicked,
                        crate::BlockFace::West,
                        TARGET,
                        predecessor.clone(),
                    )
                    .await
                    .is_err(),
                "{name} at {clicked:?}"
            );
        }
        let state = f.api.bot.session.state.lock().await;
        assert!(state.placement.is_none(), "{name}");
        assert!(state.common_placement.is_none(), "{name}");
        drop(state);
        assert!(
            f.api
                .replaceable_cube_placement_record()
                .await
                .unwrap()
                .is_none()
        );
        let mut byte = [0];
        assert_eq!(
            f.peer.try_read(&mut byte).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "unexpected dispatch for {name}"
        );
        f.stop().await;
    }
}

#[tokio::test]
async fn replacement_latches_intermediate_conflicts_and_missing_receipts_never_complete() {
    use crate::client::{adapter::PlacementOps, survival::PlacementStage};
    for conflict in [
        "target",
        "material",
        "clicked_support",
        "chunk",
        "selection",
    ] {
        let mut f = Fixture::new().await;
        let sent = replacement_start(&mut f, false, 1).await;
        match conflict {
            "target" => {
                f.block(TARGET, "air").await;
                f.block(TARGET, "short_grass").await;
            }
            "material" => {
                f.stack(2).await;
                f.stack(1).await;
            }
            "clicked_support" => {
                f.block(SUPPORT, "dirt").await;
                f.block(SUPPORT, "stone").await;
            }
            "chunk" => placement_chunk_changed(&mut *f.api.bot.session.state.lock().await, [0, 0]),
            _ => {
                f.api
                    .bot
                    .session
                    .state
                    .lock()
                    .await
                    .operations
                    .selected_hotbar
                    .as_mut()
                    .unwrap()
                    .slot = 1;
            }
        }
        f.block(TARGET, "dirt").await;
        f.stack(0).await;
        f.ack(sent.send.interaction_sequence.unwrap()).await;
        let r = f
            .api
            .replaceable_cube_placement_record()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r.stage, PlacementStage::RequiresInspection, "{conflict}");
        f.stop().await;
    }
    for missing in ["block", "material", "ack"] {
        let mut f = Fixture::new().await;
        let sent = replacement_start(&mut f, true, 1).await;
        if missing != "block" {
            f.block(sent.target, "dirt").await;
        }
        if missing != "material" {
            f.stack(0).await;
        }
        if missing != "ack" {
            f.ack(sent.send.interaction_sequence.unwrap()).await;
        }
        assert_eq!(
            f.api
                .replaceable_cube_placement_record()
                .await
                .unwrap()
                .unwrap()
                .stage,
            PlacementStage::Pending
        );
        f.stop().await;
    }
}

#[test]
fn original_replacement_context_fixture_qualifies_only_the_passive_cube_lane() {
    use std::io::Read;
    let mut bytes = String::new();
    flate2::read::GzDecoder::new(
        include_bytes!("../../../../../../data/client_api/replaceable_placement.json.gz")
            .as_slice(),
    )
    .read_to_string(&mut bytes)
    .unwrap();
    let rows: serde_json::Value = serde_json::from_str(&bytes).unwrap();
    let mut checked = 0;
    let mut nonreplaceable = 0;
    for row in rows.as_array().unwrap() {
        let name = row["before"].as_str().unwrap();
        let material = format!("minecraft:{}", row["material"].as_str().unwrap());
        if name == "stone" && survival::DRY_CUBES.contains(&material.as_str()) {
            assert_eq!(row["can_replace"], false);
            assert_eq!(row["replace_clicked"], false);
            let direction = offset(row["face"].as_u64().unwrap() as u8);
            let destination: [i32; 3] = std::array::from_fn(|i| SUPPORT[i] + direction[i]);
            assert_eq!(row["destination"], serde_json::json!(destination));
            nonreplaceable += 1;
        }
        if replaceable_plant(&native(name)) && row["material"] == name {
            assert_eq!(row["can_replace"], false); // Same plant item is context-dependent and excluded.
            assert_eq!(row["replace_clicked"], false);
        }
        if !replaceable_plant(&native(name)) || !survival::DRY_CUBES.contains(&material.as_str()) {
            continue;
        }
        assert_eq!(row["replace_owner"], "eog");
        assert_eq!(
            row["outline"],
            serde_json::to_value(plant_outline(&native(name)).unwrap()).unwrap()
        );
        assert_eq!(row["auxiliary"], serde_json::json!([]));
        assert_eq!(row["collision_empty"], true);
        assert_eq!(row["liquid"], false);
        assert_eq!(row["can_replace"], true);
        assert_eq!(row["can_place"], true);
        let adjacent = row["adjacent"].as_bool().unwrap();
        assert_eq!(row["replace_clicked"], !adjacent);
        let direction = offset(row["face"].as_u64().unwrap() as u8);
        let destination = if adjacent {
            std::array::from_fn(|i| SUPPORT[i] + direction[i])
        } else {
            SUPPORT
        };
        assert_eq!(row["destination"], serde_json::json!(destination));
        assert_eq!(
            row["placed_state"].as_i64().unwrap(),
            i64::from(state_id(&native(row["material"].as_str().unwrap())).unwrap())
        );
        checked += 1;
    }
    assert_eq!(checked, 2376); // 3 plants × 11 cubes × 6 faces × 2 secondary × 2 cells × 3 hit heights.
    assert_eq!(nonreplaceable, 864); // 12 entries (stone repeated) × 6 faces × 2 secondary × 2 cells × 3 heights.
}

#[tokio::test]
async fn replacement_cancelled_before_writer_retains_unsent_intent_without_replay() {
    use crate::client::{adapter::PlacementOps, survival::PlacementStage};
    let mut f = Fixture::new().await;
    f.block(SUPPORT, "short_grass").await;
    f.api.bot.session.state.lock().await.rotation = [-90.0, 10.0];
    let writer = f.api.bot.session.writer.lock().await;
    let mut future = Box::pin(f.api.place_cube_replacing(
        SUPPORT,
        crate::BlockFace::West,
        SUPPORT,
        native("short_grass"),
    ));
    std::future::poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(future);
    drop(writer);
    let sent = f
        .api
        .replaceable_cube_placement_record()
        .await
        .unwrap()
        .unwrap();
    assert!(!sent.send.dispatched);
    assert_eq!(sent.stage, PlacementStage::Pending);
    f.block(SUPPORT, "dirt").await;
    f.stack(4).await;
    f.ack(sent.send.interaction_sequence.unwrap()).await;
    assert_ne!(
        f.api
            .replaceable_cube_placement_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        PlacementStage::ObservedPlaced
    );
    assert!(
        f.api
            .place_cube_replacing(
                SUPPORT,
                crate::BlockFace::West,
                TARGET,
                native("short_grass")
            )
            .await
            .is_err()
    );
    let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
    f.stop().await;
    assert_eq!(
        client
            .survival()
            .replaceable_cube_placement_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        PlacementStage::RequiresInspection
    );
}

#[test]
fn replacement_first_outlines_match_original_native_clip_at_edges_inside_and_far_coordinates() {
    use std::io::Read;
    let mut text = String::new();
    flate2::read::GzDecoder::new(
        include_bytes!("../../../../../../data/client_api/replaceable_placement_geometry.json.gz")
            .as_slice(),
    )
    .read_to_string(&mut text)
    .unwrap();
    let rows: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 384);
    for row in rows.as_array().unwrap() {
        let start = serde_json::from_value::<[f64; 3]>(row["start"].clone()).unwrap();
        let end = serde_json::from_value::<[f64; 3]>(row["end"].clone()).unwrap();
        let cell = serde_json::from_value::<[i32; 3]>(row["cell"].clone()).unwrap();
        let block = crate::versions::java_1_21_11::native_state(
            row["state_index"].as_i64().unwrap() as i32,
        )
        .unwrap();
        let actual = super::super::super::raycast::replacement_cast_in(start, end, |p| {
            Ok(if p == cell {
                block.clone()
            } else {
                native("air")
            })
        })
        .unwrap();
        assert_eq!(actual.is_none(), row["hit"].is_null(), "{row}");
        if let Some(actual) = actual {
            assert_eq!(actual.position, cell);
            assert_eq!(
                format!("{:?}", actual.face).to_lowercase(),
                row["hit"]["face"].as_str().unwrap()
            );
            let point = serde_json::from_value::<[f64; 3]>(row["hit"]["point"].clone()).unwrap();
            for (a, b) in actual.point.into_iter().zip(point) {
                assert!((a - b).abs() < 1e-10, "{row}");
            }
        }
    }
}

#[tokio::test]
async fn replacement_stale_ack_and_old_new_history_remain_separate() {
    use crate::client::{adapter::PlacementOps, survival::PlacementStage};
    let mut f = Fixture::new().await;
    let old = f
        .api
        .place_cube(SUPPORT, crate::BlockFace::West)
        .await
        .unwrap();
    f.expected_packet("air placement").await;
    f.block(TARGET, "dirt").await;
    f.stack(4).await;
    f.ack(old.send.interaction_sequence.unwrap()).await;
    let old = f.api.placement_record().await.unwrap().unwrap();
    assert_eq!(old.stage, PlacementStage::ObservedPlaced);
    let old_json = serde_json::to_value(&old).unwrap();
    // The ACK sequence is high enough numerically, but predates the new boundary.
    f.ack(99).await;
    let sent = replacement_start(&mut f, false, 1).await;
    f.block(sent.target, "dirt").await;
    f.stack(0).await;
    let partial = f
        .api
        .replaceable_cube_placement_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(partial.stage, PlacementStage::Pending);
    assert!(partial.processing.is_none());
    assert_eq!(
        serde_json::to_value(f.api.placement_record().await.unwrap().unwrap()).unwrap(),
        old_json
    );
    f.ack(100).await;
    let complete = f
        .api
        .replaceable_cube_placement_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(complete.stage, PlacementStage::ObservedPlaced);
    let replacement_json = serde_json::to_value(&complete).unwrap();
    // Re-establish air at a resolved site, then start another ordinary placement.
    f.block(TARGET, "air").await;
    f.stack(3).await;
    let next = f
        .api
        .place_cube(SUPPORT, crate::BlockFace::West)
        .await
        .unwrap();
    f.expected_packet("next air placement").await;
    assert_ne!(next.id, old.id);
    assert_eq!(
        serde_json::to_value(
            f.api
                .replaceable_cube_placement_record()
                .await
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        replacement_json
    );
    f.stop().await;
}

fn oak(kind: &str) -> crate::NativeBlockState {
    crate::NativeBlockState {
        name: "minecraft:oak_slab".into(),
        properties: [
            ("type".into(), kind.into()),
            ("waterlogged".into(), "false".into()),
        ]
        .into(),
    }
}
async fn slab_stack(f: &mut Fixture, count: i32) {
    let mut bytes = vec![0, 2];
    bytes.extend(36i16.to_be_bytes());
    put_varint(&mut bytes, count);
    if count > 0 {
        put_varint(&mut bytes, default_item("oak_slab", 1).unwrap().item_id);
        bytes.extend([0, 0]);
    }
    f.receive(ids::play_clientbound::SET_SLOT, &bytes).await;
}
async fn slab_start(
    f: &mut Fixture,
    merge: bool,
    count: i32,
) -> crate::client::survival::SlabPlacementRecord {
    use crate::client::adapter::PlacementOps;
    slab_stack(f, count).await;
    if merge {
        // The exposed upper face of a bottom slab merges in the same cell.
        replacement_block(f, SUPPORT, &oak("bottom")).await;
        let mut state = f.api.bot.session.state.lock().await;
        state.rotation = [-90., 3.0];
    }
    let before = if merge { oak("bottom") } else { native("air") };
    let target = if merge { SUPPORT } else { TARGET };
    let face = if merge {
        crate::BlockFace::Up
    } else {
        crate::BlockFace::West
    };
    let sent = f
        .api
        .place_slab(SUPPORT, face, target, before)
        .await
        .unwrap();
    assert_eq!(sent.merges_clicked, merge);
    assert_eq!(sent.expected, oak(if merge { "double" } else { "top" }));
    assert_eq!(sent.first_outline.position, SUPPORT);
    let (id, bytes) = f.expected_packet("slab BLOCK_PLACE").await;
    assert_eq!(id, ids::play_serverbound::BLOCK_PLACE);
    let mut r = Reader::new(&bytes);
    assert_eq!(r.varint().unwrap(), 0);
    assert_eq!(r.take(8).unwrap(), pack_position(SUPPORT).to_be_bytes());
    assert_eq!(r.varint().unwrap(), face as i32);
    for coordinate in sent.cursor {
        assert_eq!(r.f32().unwrap(), coordinate);
    }
    assert!(!r.bool().unwrap());
    assert!(!r.bool().unwrap());
    assert_eq!(r.varint().unwrap(), sent.send.interaction_sequence.unwrap());
    r.end().unwrap();
    sent
}
#[tokio::test]
async fn slab_independent_receipts_full_state_last_item_and_frozen_separate_history() {
    use crate::client::{adapter::PlacementOps, survival::PlacementStage};
    for merge in [false, true] {
        for count in [1, 5] {
            for inventory_first in [false, true] {
                let mut f = Fixture::new().await;
                let sent = slab_start(&mut f, merge, count).await;
                assert!(f.api.placement_record().await.unwrap().is_none());
                assert!(
                    f.api
                        .replaceable_cube_placement_record()
                        .await
                        .unwrap()
                        .is_none()
                );
                assert!(
                    f.api
                        .place_cube(SUPPORT, crate::BlockFace::West)
                        .await
                        .is_err()
                );
                f.ack(sent.send.interaction_sequence.unwrap()).await;
                if inventory_first {
                    slab_stack(&mut f, count - 1).await;
                } else {
                    replacement_block(&mut f, sent.target, &sent.expected).await;
                }
                assert_eq!(
                    f.api.slab_placement_record().await.unwrap().unwrap().stage,
                    PlacementStage::Pending
                );
                if inventory_first {
                    replacement_block(&mut f, sent.target, &sent.expected).await;
                } else {
                    slab_stack(&mut f, count - 1).await;
                }
                let complete = f.api.slab_placement_record().await.unwrap().unwrap();
                assert_eq!(
                    complete.stage,
                    PlacementStage::ObservedPlaced,
                    "{:?}",
                    complete.requires_inspection
                );
                assert_eq!(
                    complete.target_receipt.as_ref().unwrap().value,
                    sent.expected
                );
                let frozen = serde_json::to_value(&complete).unwrap();
                f.block(sent.target, "stone").await;
                f.stack(12).await;
                f.ack(99).await;
                assert_eq!(
                    serde_json::to_value(f.api.slab_placement_record().await.unwrap().unwrap())
                        .unwrap(),
                    frozen
                );
                f.stop().await;
            }
        }
    }
}
#[tokio::test]
async fn slab_rejects_wrong_target_predecessor_foreign_wet_and_double_without_send() {
    use crate::client::adapter::PlacementOps;
    for case in [
        "target",
        "predecessor",
        "spruce",
        "wet",
        "double",
        "material",
    ] {
        let mut f = Fixture::new().await;
        slab_stack(&mut f, 1).await;
        let mut before = native("air");
        let mut target = TARGET;
        match case {
            "target" => target = SUPPORT,
            "predecessor" => before = native("stone"),
            "spruce" => {
                before = oak("bottom");
                before.name = "minecraft:spruce_slab".into();
            }
            "wet" => {
                before = oak("bottom");
                before
                    .properties
                    .insert("waterlogged".into(), "true".into());
            }
            "double" => before = oak("double"),
            "material" => f.stack(2).await,
            _ => unreachable!(),
        }
        if matches!(case, "spruce" | "wet" | "double") {
            replacement_block(&mut f, TARGET, &before).await;
        }
        assert!(
            f.api
                .place_slab(SUPPORT, crate::BlockFace::West, target, before)
                .await
                .is_err(),
            "{case}"
        );
        assert!(f.api.slab_placement_record().await.unwrap().is_none());
        assert!(
            timeout(Duration::from_millis(10), read_packet(&mut f.peer, None))
                .await
                .is_err()
        );
        f.stop().await;
    }
}
#[tokio::test]
async fn slab_conflicts_latch_and_missing_or_stale_receipts_never_confirm() {
    use crate::client::{adapter::PlacementOps, survival::PlacementStage};
    for case in [
        "block",
        "material",
        "ack",
        "stale_ack",
        "wrong_type",
        "wrong_waterlogged",
        "support",
        "restore",
        "slot_restore",
        "chunk",
    ] {
        let mut f = Fixture::new().await;
        if case == "stale_ack" {
            f.ack(100).await;
        }
        let sent = slab_start(&mut f, true, 1).await;
        if case == "wrong_type" {
            replacement_block(&mut f, sent.target, &oak("top")).await;
        }
        if case == "wrong_waterlogged" {
            let mut wet = sent.expected.clone();
            wet.properties.insert("waterlogged".into(), "true".into());
            replacement_block(&mut f, sent.target, &wet).await;
        }
        if case == "support" {
            f.block(sent.support, "stone").await;
        }
        if case == "chunk" {
            placement_chunk_changed(&mut *f.api.bot.session.state.lock().await, [0, 0]);
        }
        if case != "block" {
            replacement_block(&mut f, sent.target, &sent.expected).await;
        }
        if case == "restore" {
            replacement_block(&mut f, sent.target, &sent.before).await;
            replacement_block(&mut f, sent.target, &sent.expected).await;
        }
        if case != "material" {
            slab_stack(&mut f, 0).await;
        }
        if case == "slot_restore" {
            slab_stack(&mut f, 1).await;
            slab_stack(&mut f, 0).await;
        }
        if !matches!(case, "ack" | "stale_ack") {
            f.ack(sent.send.interaction_sequence.unwrap()).await;
        }
        let stage = f.api.slab_placement_record().await.unwrap().unwrap().stage;
        assert_eq!(
            stage,
            if matches!(case, "block" | "material" | "ack" | "stale_ack") {
                PlacementStage::Pending
            } else {
                PlacementStage::RequiresInspection
            },
            "{case}"
        );
        f.stop().await;
    }
}
#[tokio::test]
async fn slab_cancelled_writer_keeps_unsent_intent_and_never_replays() {
    use crate::client::{adapter::PlacementOps, survival::PlacementStage};
    let mut f = Fixture::new().await;
    slab_stack(&mut f, 1).await;
    let session = f.api.bot.session.clone();
    let writer = session.writer.lock().await;
    let future = f
        .api
        .place_slab(SUPPORT, crate::BlockFace::West, TARGET, native("air"));
    assert!(timeout(Duration::from_millis(30), future).await.is_err());
    drop(writer);
    let intent = f.api.slab_placement_record().await.unwrap().unwrap();
    assert!(!intent.send.dispatched);
    replacement_block(&mut f, intent.target, &intent.expected).await;
    slab_stack(&mut f, 0).await;
    f.ack(intent.send.interaction_sequence.unwrap()).await;
    assert_ne!(
        f.api.slab_placement_record().await.unwrap().unwrap().stage,
        PlacementStage::ObservedPlaced
    );
    assert!(
        timeout(Duration::from_millis(10), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    f.stop().await;
}

async fn slab_fixed_entity(f: &mut Fixture, name: &str, position: [f64; 3]) {
    let type_id = crate::client::registry::Registry::for_version(MinecraftVersion::Java1_21_11)
        .builtin_id("minecraft:entity_type", &format!("minecraft:{name}"))
        .unwrap()
        .value();
    let mut bytes = vec![55];
    bytes.extend([55; 16]);
    put_varint(&mut bytes, type_id);
    for coordinate in position {
        bytes.extend(coordinate.to_be_bytes());
    }
    bytes.extend([0, 0, 0, 0, 0]);
    f.receive(ids::play_clientbound::SPAWN_ENTITY, &bytes).await;
}
#[tokio::test]
async fn slab_obstruction_uses_actual_half_for_own_and_received_entity_boxes() {
    let mut f = Fixture::new().await;
    let own = [1.2, 2.5, 0.2, 1.8, 4.3, 0.8];
    {
        let state = f.api.bot.session.state.lock().await;
        assert!(slab::unobstructed(&state, 72, own, TARGET, &oak("bottom")).is_ok());
        assert!(slab::unobstructed(&state, 72, own, TARGET, &oak("top")).is_err());
        assert!(slab::unobstructed(&state, 72, own, TARGET, &oak("double")).is_err());
    }
    // TNT's box is wholly in the top half or above it; the bottom remains clear.
    slab_fixed_entity(&mut f, "tnt", [1.5, 2.5, 0.5]).await;
    {
        let state = f.api.bot.session.state.lock().await;
        let own = [0., 1., 0., 0.8, 2.8, 0.8];
        assert!(slab::unobstructed(&state, 72, own, TARGET, &oak("bottom")).is_ok());
        assert!(slab::unobstructed(&state, 72, own, TARGET, &oak("top")).is_err());
        assert!(slab::unobstructed(&state, 72, own, TARGET, &oak("double")).is_err());
    }
    use crate::client::adapter::PlacementOps;
    slab_stack(&mut f, 1).await;
    assert!(
        f.api
            .place_slab(SUPPORT, crate::BlockFace::West, TARGET, native("air"))
            .await
            .is_err()
    );
    assert!(f.api.slab_placement_record().await.unwrap().is_none());
    f.stop().await;
}
#[tokio::test]
async fn slab_bottom_side_and_top_underside_have_exact_native_expected_states() {
    use crate::client::adapter::PlacementOps;
    for underside in [false, true] {
        let mut f = Fixture::new().await;
        slab_stack(&mut f, 1).await;
        let clicked = if underside { [2, 3, 0] } else { SUPPORT };
        if underside {
            f.block(SUPPORT, "air").await;
            replacement_block(&mut f, clicked, &oak("top")).await;
        }
        f.api.bot.session.state.lock().await.rotation = [-90., if underside { -30. } else { 10. }];
        let target = if underside { clicked } else { TARGET };
        let before = if underside { oak("top") } else { native("air") };
        let face = if underside {
            crate::BlockFace::Down
        } else {
            crate::BlockFace::West
        };
        let sent = f
            .api
            .place_slab(clicked, face, target, before)
            .await
            .unwrap();
        assert_eq!(
            sent.expected,
            oak(if underside { "double" } else { "bottom" })
        );
        f.expected_packet("slab other half").await;
        replacement_block(&mut f, target, &sent.expected).await;
        slab_stack(&mut f, 0).await;
        f.ack(sent.send.interaction_sequence.unwrap()).await;
        assert_eq!(
            f.api.slab_placement_record().await.unwrap().unwrap().stage,
            crate::client::survival::PlacementStage::ObservedPlaced
        );
        f.stop().await;
    }
}

#[tokio::test]
async fn slab_history_survives_next_cube_and_connection_closure() {
    use crate::client::{adapter::PlacementOps, survival::PlacementStage};
    let mut f = Fixture::new().await;
    let sent = slab_start(&mut f, false, 1).await;
    replacement_block(&mut f, sent.target, &sent.expected).await;
    slab_stack(&mut f, 0).await;
    f.ack(sent.send.interaction_sequence.unwrap()).await;
    let historical =
        serde_json::to_value(f.api.slab_placement_record().await.unwrap().unwrap()).unwrap();
    f.block(TARGET, "air").await;
    f.stack(2).await;
    f.api
        .place_cube(SUPPORT, crate::BlockFace::West)
        .await
        .unwrap();
    f.expected_packet("next cube").await;
    assert_eq!(
        serde_json::to_value(f.api.slab_placement_record().await.unwrap().unwrap()).unwrap(),
        historical
    );
    f.api.bot.disconnect().await.unwrap();
    let cube = f.api.placement_record().await.unwrap().unwrap();
    assert_eq!(cube.stage, PlacementStage::RequiresInspection);
    assert!(cube.send.dispatched);
    assert_eq!(
        serde_json::to_value(f.api.slab_placement_record().await.unwrap().unwrap()).unwrap(),
        historical
    );
    timeout(Duration::from_secs(1), f.receiver)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn slab_partial_write_retains_inspection_and_never_replays() {
    use crate::client::survival::PlacementStage;
    let mut f = Fixture::new().await;
    slab_stack(&mut f, 1).await;
    let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
    let control = crate::client::ledger::testing::Control::default();
    f.api
        .bot
        .session
        .writer
        .lock()
        .await
        .stream
        .test_replace(control.writer());
    control.release(1, false);
    let survival = client.survival();
    let waiter = tokio::spawn(async move {
        survival
            .place_slab(SUPPORT, crate::BlockFace::West, TARGET, native("air"))
            .await
    });
    control.until(|w| w.bytes().len() == 1).await;
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    let record = client
        .survival()
        .slab_placement_record()
        .await
        .unwrap()
        .unwrap();
    assert!(!record.send.dispatched);
    assert_eq!(record.stage, PlacementStage::RequiresInspection);
    assert!(
        client
            .survival()
            .place_slab(SUPPORT, crate::BlockFace::West, TARGET, native("air"))
            .await
            .is_err()
    );
    assert!(f.api.bot.session.ledger.cut(false).wait().await.is_err());
    assert_eq!(control.bytes().len(), 1);
    f.stop().await;
}
#[tokio::test]
async fn slab_new_obstruction_before_confirmation_is_latched() {
    use crate::client::{adapter::PlacementOps, survival::PlacementStage};
    let mut f = Fixture::new().await;
    let sent = slab_start(&mut f, false, 1).await;
    slab_fixed_entity(&mut f, "tnt", [1.5, 2.5, 0.5]).await;
    replacement_block(&mut f, TARGET, &sent.expected).await;
    slab_stack(&mut f, 0).await;
    f.ack(sent.send.interaction_sequence.unwrap()).await;
    let record = f.api.slab_placement_record().await.unwrap().unwrap();
    assert_eq!(record.stage, PlacementStage::RequiresInspection);
    assert!(
        record
            .requires_inspection
            .unwrap()
            .contains("building-blocking entity")
    );
    f.stop().await;
}

#[tokio::test]
async fn slab_rejects_the_connection_invalidated_by_a_nonfinite_entity_packet() {
    let mut f = Fixture::new().await;
    slab_stack(&mut f, 1).await;
    let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
    slab_fixed_entity(&mut f, "tnt", [f64::NAN, 2.5, 0.5]).await;
    {
        let state = f.api.bot.session.state.lock().await;
        assert!(state.failure.is_some());
        assert!(!state.ready);
    }
    assert!(
        client
            .survival()
            .place_slab(SUPPORT, crate::BlockFace::West, TARGET, native("air"))
            .await
            .is_err()
    );
    assert!(
        client
            .survival()
            .slab_placement_record()
            .await
            .unwrap()
            .is_none()
    );
    f.stop().await;
}

#[tokio::test]
async fn slab_living_obstruction_uses_received_scale_baby_and_pose_without_defaulting_scale() {
    use crate::client::adapter::PlacementOps;
    let version = MinecraftVersion::Java1_21_11;
    let baby_index = version
        .table()
        .entity_data("minecraft:pig")
        .iter()
        .find(|r| {
            crate::client::EntityDataField::Baby
                .official_names()
                .contains(&(r.owner, r.field))
        })
        .unwrap()
        .index;
    // An adult at unit scale ends below TARGET. Scaling it to 2 crosses the
    // top half, while baby scaling or the fixed sleeping dimensions clear it.
    for (scale, baby, pose, blocked) in [
        (None, false, 0, true),
        (Some(1.0f64), false, 0, false),
        (Some(2.0), false, 0, true),
        (Some(2.0), true, 0, false),
        (Some(2.0), false, 2, false),
    ] {
        let mut f = Fixture::new().await;
        slab_stack(&mut f, 1).await;
        slab_fixed_entity(&mut f, "pig", [1.5, 1., 0.5]).await;
        f.receive(
            ids::play_clientbound::ENTITY_METADATA,
            &[55, 6, 20, pose, baby_index, 8, u8::from(baby), 255],
        )
        .await;
        if let Some(scale) = scale {
            let mut packet = vec![55, 1];
            put_varint(&mut packet, ids::SCALE_ATTRIBUTE);
            packet.extend(scale.to_be_bytes());
            packet.push(0); // No attribute modifiers.
            f.receive(ids::play_clientbound::ENTITY_UPDATE_ATTRIBUTES, &packet)
                .await;
        }
        let result = f
            .api
            .place_slab(SUPPORT, crate::BlockFace::West, TARGET, native("air"))
            .await;
        if blocked {
            let error = result.unwrap_err().to_string();
            assert!(
                error.contains(if scale.is_none() {
                    "qualified entity geometry"
                } else {
                    "building-blocking entity"
                }),
                "{error}"
            );
            assert!(f.api.slab_placement_record().await.unwrap().is_none());
            assert!(
                timeout(Duration::from_millis(10), read_packet(&mut f.peer, None))
                    .await
                    .is_err()
            );
        } else {
            let sent = result.unwrap();
            assert_eq!(sent.expected, oak("top"));
            f.expected_packet("slab clear of received living geometry")
                .await;
        }
        f.stop().await;
    }
}

#[tokio::test]
async fn slab_all_receipt_orders_complete_on_receive_and_freeze_before_first_getter() {
    use crate::client::{adapter::PlacementOps, survival::PlacementStage};
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let mut f = Fixture::new().await;
        let sent = slab_start(&mut f, true, 1).await;
        for (index, receipt) in order.into_iter().enumerate() {
            match receipt {
                0 => replacement_block(&mut f, sent.target, &sent.expected).await,
                1 => slab_stack(&mut f, 0).await,
                2 => f.ack(sent.send.interaction_sequence.unwrap()).await,
                _ => unreachable!(),
            }
            let state = f.api.bot.session.state.lock().await;
            let native = state.placement.as_ref().unwrap();
            assert_eq!(native.observation.is_some(), index == 2, "{order:?}");
            assert!(native.requires_inspection.is_none());
        }
        // No getter after dispatch: later conflicts and new ACKs must not
        // rewrite the receipts committed by the final required packet.
        let frozen = {
            let state = f.api.bot.session.state.lock().await;
            serde_json::to_value(slab::record_in(&state).unwrap().unwrap()).unwrap()
        };
        f.block(sent.target, "stone").await;
        f.stack(12).await;
        f.ack(99).await;
        let historical = f.api.slab_placement_record().await.unwrap().unwrap();
        assert_eq!(historical.stage, PlacementStage::ObservedPlaced);
        assert_eq!(serde_json::to_value(historical).unwrap(), frozen);
        f.stop().await;
    }
}

mod stairs;

mod wet_slabs;

#[tokio::test]
async fn explicit_body_activation_ignores_outline_ground_and_default_item_and_waits_for_ack() {
    use crate::body::{Action, ActionStatus, Body, BodyConfig, HeadlessBody};
    let mut f = Fixture::new().await;
    {
        let mut state = f.api.bot.session.state.lock().await;
        state.rotation = [180., 0.]; // looks away from explicit clicked face
        state.motion.on_ground = Some(false);
    }
    let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
    let body = Body::new(
        HeadlessBody::new(
            client,
            BodyConfig {
                settle_window: Duration::from_millis(20),
                action_timeout: Duration::from_secs(2),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let action = Action::ActivateBlock {
        pos: SUPPORT,
        face: crate::BlockFace::East,
        cursor: [1., 0.17, 0.83],
        hand: crate::Hand::Off,
    };
    let id = body.submit(action.clone()).await.unwrap();
    let (packet, payload) = f.expected_packet("explicit activation").await;
    assert_eq!(packet, ids::play_serverbound::BLOCK_PLACE);
    let mut r = Reader::new(&payload);
    assert_eq!(r.varint().unwrap(), 1);
    assert_eq!(
        super::super::super::super::wire::unpack_position(r.u64().unwrap()),
        SUPPORT
    );
    assert_eq!(r.varint().unwrap(), crate::BlockFace::East as i32);
    assert_eq!(
        [r.f32().unwrap(), r.f32().unwrap(), r.f32().unwrap()],
        [1., 0.17, 0.83]
    );
    assert!(!r.bool().unwrap());
    assert!(!r.bool().unwrap());
    let sequence = r.varint().unwrap();
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert!(matches!(
        body.action_status(id).await.unwrap(),
        ActionStatus::Pending
    ));
    // Ordinary screen contents are expected for activation, not an inventory rollback.
    f.api
        .bot
        .session
        .state
        .lock()
        .await
        .body_evidence
        .push(100, crate::body::evidence::Update::Resync);
    let mut ack = Vec::new();
    put_varint(&mut ack, sequence);
    f.receive(ids::play_clientbound::ACKNOWLEDGE_PLAYER_DIGGING, &ack)
        .await;
    timeout(Duration::from_secs(1), async {
        loop {
            if body.action_status(id).await.unwrap().is_terminal() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        body.action_status(id).await.unwrap(),
        ActionStatus::Accepted
    ));
    drop(body);
    f.stop().await;
}

#[tokio::test]
async fn body_dig_unknown_estimate_uses_start_ack_once_and_waits_for_actual_air() {
    use crate::body::{Action, ActionStatus, Body, BodyConfig, HeadlessBody};
    for scenario in ["selection", "slot", "revert", "no_ack", "no_result"] {
        let mut f = Fixture::new().await;
        {
            let mut s = f.api.bot.session.state.lock().await;
            s.operations.game_mode = Some(GameMode::Survival);
            if scenario == "selection" {
                s.operations.selected_hotbar = None;
            }
            // No tool observation: this prevents the strict estimator, not START.
            s.operations.inventory.slots.clear();
        }
        let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
        assert!(client.survival().dig_estimate(SUPPORT).await.is_err());
        let body = Body::new(
            HeadlessBody::new(
                client,
                BodyConfig {
                    settle_window: Duration::from_millis(20),
                    action_timeout: Duration::from_millis(240),
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        let id = body
            .submit(Action::BreakBlock {
                pos: SUPPORT,
                face: crate::BlockFace::West,
            })
            .await
            .unwrap();
        let (packet, payload) = f.expected_packet("body START without estimate").await;
        assert_eq!(packet, ids::play_serverbound::BLOCK_DIG);
        let mut r = Reader::new(&payload);
        assert_eq!(r.varint().unwrap(), 0);
        r.u64().unwrap();
        r.u8().unwrap();
        let sequence = r.varint().unwrap();
        if scenario != "no_ack" {
            let mut ack = Vec::new();
            put_varint(&mut ack, sequence);
            f.receive(ids::play_clientbound::ACKNOWLEDGE_PLAYER_DIGGING, &ack)
                .await;
            let (packet, payload) = f.expected_packet("one evidence-driven STOP").await;
            assert_eq!(packet, ids::play_serverbound::BLOCK_DIG);
            assert_eq!(payload[0], 2);
            tokio::time::sleep(Duration::from_millis(30)).await;
            assert!(matches!(
                body.action_status(id).await.unwrap(),
                ActionStatus::Pending
            ));
            if scenario != "no_result" {
                let mut update = pack_position(SUPPORT).to_be_bytes().to_vec();
                put_varint(
                    &mut update,
                    if scenario == "revert" {
                        state_id(&native("stone")).unwrap()
                    } else {
                        0
                    },
                );
                f.receive(ids::play_clientbound::BLOCK_CHANGE, &update)
                    .await;
            }
        }
        let status = timeout(Duration::from_secs(2), async {
            loop {
                let status = body.action_status(id).await.unwrap();
                if status.is_terminal() {
                    break status;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        match scenario {
            "selection" | "slot" => assert!(matches!(status, ActionStatus::Accepted), "{status:?}"),
            "revert" => assert!(
                matches!(status, ActionStatus::Corrected { .. }),
                "{status:?}"
            ),
            _ => assert!(matches!(status, ActionStatus::Unknown { .. }), "{status:?}"),
        }
        if !matches!(scenario, "selection" | "slot") {
            let (packet, payload) = f.expected_packet("bounded ABORT").await;
            assert_eq!((packet, payload[0]), (ids::play_serverbound::BLOCK_DIG, 1));
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
        assert!(
            timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
                .await
                .is_err()
        );
        drop(body);
        f.stop().await;
    }
}

#[tokio::test]
async fn body_dig_keeps_known_hardness_range_and_hand_fences_without_estimation() {
    use crate::body::{Action, ObserveRequest};
    let mut f = Fixture::new().await;
    {
        let mut s = f.api.bot.session.state.lock().await;
        s.operations.game_mode = Some(GameMode::Survival);
    }
    let initial = f
        .api
        .body_evidence(1, &ObserveRequest::default(), None)
        .await
        .unwrap()
        .snapshot;
    f.api
        .bot
        .session
        .state
        .lock()
        .await
        .operations
        .selected_hotbar
        .as_mut()
        .unwrap()
        .slot = 1;
    let action = Action::BreakBlock {
        pos: SUPPORT,
        face: crate::BlockFace::Up,
    };
    assert!(
        f.api
            .body_block_send(initial.world, &action, 2, Some(&initial))
            .await
            .unwrap_err()
            .to_string()
            .contains("identity changed")
    );
    {
        let mut s = f.api.bot.session.state.lock().await;
        s.operations.selected_hotbar.as_mut().unwrap().slot = 0;
        s.world
            .seed_replay_cell(SUPPORT, state_id(&native("bedrock")).unwrap());
    }
    assert!(
        f.api
            .body_block_send(initial.world, &action, 0, Some(&initial))
            .await
            .unwrap_err()
            .to_string()
            .contains("cannot be broken")
    );
    // Losing a known selection remains an identity error.
    {
        let mut s = f.api.bot.session.state.lock().await;
        s.position = Some([100., 1., 0.]);
        s.operations.selected_hotbar = None;
        let player = f.api.common_player_unlocked(&s).unwrap();
        assert!(crate::body::dig::hand(&initial, &player).is_err());
    }
    // No START, STOP or cleanup packet on these pre-send refusals.
    assert!(
        timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    f.stop().await;
}

#[tokio::test]
async fn send_and_trust_requires_full_flush_and_missing_ack_still_expires() {
    use crate::body::{Action, ActionStatus, Body, BodyConfig, HeadlessBody, SendStatus};
    for complete in [true, false] {
        let f = Fixture::new().await;
        let control = crate::client::ledger::testing::Control::default();
        f.api
            .bot
            .session
            .writer
            .lock()
            .await
            .stream
            .test_replace(control.writer());
        let body = Body::new(
            HeadlessBody::new(
                crate::Client::from_java_1_21_11(f.api.bot.clone()),
                BodyConfig {
                    settle_window: Duration::from_millis(20),
                    action_timeout: Duration::from_millis(250),
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        control.release(1, false);
        let sender = body.backend().clone();
        let mut waiter = tokio::spawn(async move {
            sender
                .send_and_trust(Action::ActivateBlock {
                    pos: SUPPORT,
                    face: crate::BlockFace::West,
                    cursor: [0.5; 3],
                    hand: crate::Hand::Main,
                })
                .await
                .unwrap()
        });
        control.until(|w| w.bytes().len() == 1).await;
        assert!(
            timeout(Duration::from_millis(10), &mut waiter)
                .await
                .is_err()
        );
        if complete {
            control.release(usize::MAX, false);
            control.until(|w| w.progress().1 > 0).await;
            // Even complete bytes without the final flush are not Sent.
            assert!(
                timeout(Duration::from_millis(10), &mut waiter)
                    .await
                    .is_err()
            );
            control.release(usize::MAX, true);
            let sent = timeout(Duration::from_millis(100), waiter)
                .await
                .unwrap()
                .unwrap();
            assert!(matches!(sent.dispatch, SendStatus::Sent));
            assert!(matches!(
                body.action_status(sent.id).await.unwrap(),
                ActionStatus::Pending
            ));
            // No server ACK: Sent must not replace the bounded acceptance task.
            tokio::time::sleep(Duration::from_millis(260)).await;
            assert!(matches!(
                body.action_status(sent.id).await.unwrap(),
                ActionStatus::Unknown { .. }
            ));
        } else {
            let unsent = timeout(Duration::from_secs(1), waiter)
                .await
                .unwrap()
                .unwrap();
            assert!(matches!(
                unsent.dispatch,
                SendStatus::NotSent {
                    outcome: ActionStatus::Unknown { .. }
                }
            ));
            assert_eq!(control.bytes().len(), 1);
            control.fail(Some(1), true);
        }
        body.release_control().await.unwrap();
        drop(body);
        f.stop().await;
    }
}
