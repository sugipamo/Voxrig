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
            state: Mutex::new(fixture_state()),
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
    for row in rows.as_array().unwrap() {
        let name = row["before"].as_str().unwrap();
        let material = format!("minecraft:{}", row["material"].as_str().unwrap());
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
