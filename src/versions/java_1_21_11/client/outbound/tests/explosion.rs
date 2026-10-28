use super::*;
use crate::client::{
    GameMode,
    control::{ControlKind, ControlSession, ControlStatus, Controls, Received},
    physics::{Body, Environment},
};

fn explosion(impulse: Option<[f64; 3]>) -> Vec<u8> {
    let mut packet = Vec::new();
    for v in [0f64, 65., 0.] {
        packet.extend(v.to_be_bytes());
    }
    packet.extend(4f32.to_be_bytes());
    packet.extend(0i32.to_be_bytes());
    packet.push(u8::from(impulse.is_some()));
    if let Some(v) = impulse {
        for x in v {
            packet.extend(x.to_be_bytes());
        }
    }
    packet.extend([22, 1, 0]); // Simple particle, registry sound, empty weighted list.
    packet
}

fn correction(position: [f64; 3], velocity: [f64; 3], flags: u32) -> Vec<u8> {
    let mut packet = vec![1];
    for v in position.into_iter().chain(velocity) {
        packet.extend(v.to_be_bytes());
    }
    packet.extend([0; 8]);
    packet.extend(flags.to_be_bytes());
    packet
}

fn received(state: &State) -> Received {
    Received {
        abilities_sequence: None,
        environment: Environment::defaults(crate::MinecraftVersion::Java1_21_11),
        pose: state
            .motion
            .received_pose
            .as_ref()
            .map(|p| (p.receive_sequence, p.position, p.velocity)),
        velocity: state
            .operations
            .local_player
            .velocity
            .map(|v| (v.receive_sequence, v.value)),
        using_item: None,
    }
}

async fn tick(session: &Session, handle: &crate::client::PlayerControl, count: u64) {
    timeout(Duration::from_secs(2), async {
        loop {
            let changed = session.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let record = handle.control_record().await.unwrap().unwrap();
            assert!(record.dispatched_ticks <= count, "{record:?}");
            if record.dispatched_ticks == count {
                return;
            }
            changed.await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn explosion_packets_match_independent_original_handler_and_travel_frames() {
    let data: serde_json::Value = serde_json::from_reader(flate2::read::GzDecoder::new(
        &include_bytes!("../../../../../../data/client_api/explosion_ground_oracle.json.gz")[..],
    ))
    .unwrap();
    assert_eq!(data["scenarios"].as_array().unwrap().len(), 10);
    for (scenario, result) in data["scenarios"]
        .as_array()
        .unwrap()
        .iter()
        .zip(data["results"]["1.21.11"].as_array().unwrap())
    {
        let (transport, _, _peer) = common_ground_fixture(GameMode::Survival).await;
        let mut state = transport.state.lock().await;
        let position = std::array::from_fn(|i| {
            scenario["start"][i].as_f64().unwrap() + [1024., 100., 1024.][i]
        });
        let velocity = std::array::from_fn(|i| scenario["velocity"][i].as_f64().unwrap());
        state.position = Some(position);
        let generation = state.loading.generation;
        let sequence = state.sequence;
        state.motion.receive(operations::ReceivedPose {
            generation,
            receive_sequence: sequence,
            position,
            rotation: [0.; 2],
            velocity: Some(velocity),
        });
        state.operations.local_player.velocity = Some(operations::VelocitySample {
            value: velocity,
            receive_sequence: sequence,
        });
        let mut body = Body::new(position);
        body.velocity = velocity;
        body.on_ground = scenario["on_ground"].as_bool().unwrap();
        body.first_tick = true;
        let owner = ControlSession::from_model(
            crate::MinecraftVersion::Java1_21_11,
            1,
            body,
            [0.; 2],
            &received(&state),
        );
        state.control.seed_session(owner, generation);
        for (input, expected) in scenario["ticks"]
            .as_array()
            .unwrap()
            .iter()
            .zip(result["frames"].as_array().unwrap())
        {
            if let Some(motions) = input["motion_inputs"].as_array() {
                for motion in motions {
                    let (id, bytes) = if motion["kind"] == "explosion" {
                        let impulse = if motion["value"].is_null() {
                            None
                        } else {
                            Some(std::array::from_fn(|i| {
                                motion["value"][i].as_f64().unwrap()
                            }))
                        };
                        (ids::play_clientbound::EXPLOSION, explosion(impulse))
                    } else {
                        assert_eq!(motion["value"], serde_json::json!([0., 0., 0.]));
                        (ids::play_clientbound::ENTITY_VELOCITY, vec![42, 0])
                    };
                    state.receive(id, &bytes, 256).unwrap();
                }
            }
            assert!(state.operations.local_player.motion_interruption.is_none());
            let receipts = received(&state);
            let owner = state.control.session.as_mut().unwrap();
            owner.controls = Controls {
                forward: 1,
                ..Default::default()
            };
            owner
                .step(&receipts, &mut |p| {
                    Ok(crate::NativeBlockState {
                        name: if p[1] == 99 {
                            "minecraft:stone"
                        } else {
                            "minecraft:air"
                        }
                        .into(),
                        properties: Default::default(),
                    })
                })
                .unwrap()
                .unwrap();
            for axis in 0..3 {
                let expected_position: f64 = expected["position"][axis]
                    .as_str()
                    .unwrap()
                    .parse()
                    .unwrap();
                let expected_velocity: f64 = expected["velocity"][axis]
                    .as_str()
                    .unwrap()
                    .parse()
                    .unwrap();
                assert_eq!(
                    (owner.body.position[axis] - [1024., 100., 1024.][axis]).to_bits(),
                    expected_position.to_bits(),
                    "{} position axis {axis}",
                    scenario["name"]
                );
                assert_eq!(
                    owner.body.velocity[axis].to_bits(),
                    expected_velocity.to_bits(),
                    "{} velocity axis {axis}",
                    scenario["name"]
                );
            }
            assert_eq!(
                owner.body.on_ground,
                expected["on_ground"].as_bool().unwrap()
            );
            state.position = Some(owner.body.position);
        }
        drop(state);
        transport.stop();
    }
}

#[tokio::test(start_paused = true)]
async fn explosion_order_corrections_getters_and_stop_restart_are_once_only() {
    for mode in [GameMode::Survival, GameMode::Adventure, GameMode::Creative] {
        let (session, api, _peer) = common_ground_fixture(mode).await;
        let client = crate::Client::from_java_1_21_11(api.bot.clone());
        let handle = client.player_control(mode);
        let start = handle.start_control().await.unwrap();
        tick(&session, &handle, 1).await;
        {
            let mut state = session.state.lock().await;
            state
                .receive(
                    ids::play_clientbound::EXPLOSION,
                    &explosion(Some([0.125, 0.5, 0.])),
                    256,
                )
                .unwrap();
            state
                .receive(ids::play_clientbound::ENTITY_VELOCITY, &[42, 0], 256)
                .unwrap();
            state
                .receive(
                    ids::play_clientbound::EXPLOSION,
                    &explosion(Some([0.25, 0.25, 0.])),
                    256,
                )
                .unwrap();
            // Relative velocity must use the current numerical velocity after the explosion.
            state
                .receive(
                    ids::play_clientbound::POSITION,
                    &correction([8.5, 65., 8.5], [0.125, 0.125, 0.], 224),
                    256,
                )
                .unwrap();
            state
                .receive(
                    ids::play_clientbound::EXPLOSION,
                    &explosion(Some([0.125, 0.125, 0.])),
                    256,
                )
                .unwrap();
            assert_eq!(
                state.control.session.as_ref().unwrap().body.velocity,
                [0.5, 0.5, 0.]
            );
            state
                .receive(
                    ids::play_clientbound::POSITION,
                    &correction([8.5, 65., 8.5], [0.; 3], 0),
                    256,
                )
                .unwrap();
            state
                .receive(
                    ids::play_clientbound::EXPLOSION,
                    &explosion(Some([0.125, 0.25, 0.])),
                    256,
                )
                .unwrap();
            assert_eq!(
                state.control.session.as_ref().unwrap().body.velocity,
                [0.125, 0.25, 0.]
            );
        }
        let before = session
            .state
            .lock()
            .await
            .control
            .session
            .as_ref()
            .unwrap()
            .body
            .velocity;
        let history = client.explosions_after(0).await.unwrap();
        for _ in 0..3 {
            assert_eq!(
                client.explosions_after(0).await.unwrap().explosions,
                history.explosions
            );
            let _ = handle.control_record().await.unwrap();
        }
        assert_eq!(
            session
                .state
                .lock()
                .await
                .control
                .session
                .as_ref()
                .unwrap()
                .body
                .velocity,
            before
        );
        let stop = handle.stop_control_for(start.session_id).await.unwrap();
        assert!(matches!(stop.status, ControlStatus::Stopped { .. }));
        // Each received impulse is counted once, however often it is read.
        assert_eq!(stop.explosion_impulses, 4);
        // A newly received stopped-model impulse is applied once, then inherited.
        {
            let mut state = session.state.lock().await;
            state
                .receive(
                    ids::play_clientbound::EXPLOSION,
                    &explosion(Some([0.125, 0., 0.])),
                    256,
                )
                .unwrap();
            assert_eq!(
                state.control.session.as_ref().unwrap().body.velocity,
                [0.25, 0.25, 0.]
            );
        }
        let resume = handle.start_control().await.unwrap();
        assert_ne!(resume.session_id, start.session_id);
        // The stopped-model impulse is part of the resumed seed, not a new count.
        assert_eq!(resume.explosion_impulses, 0);
        tick(&session, &handle, 1).await;
        let frame = handle
            .control_record()
            .await
            .unwrap()
            .unwrap()
            .frame
            .unwrap();
        assert_eq!(frame.position, [8.75, 65.25, 8.5]);
        handle.stop_control_for(resume.session_id).await.unwrap();
        session.stop();
    }
}

#[tokio::test(start_paused = true)]
async fn explosion_keeps_unsupported_owners_and_uncertain_lifetimes_interrupted() {
    for refusal in [
        "no_owner",
        "flight",
        "glide",
        "vehicle",
        "world",
        "world_packet",
        "write",
        "death",
        "undecoded",
    ] {
        let (session, api, _peer) = common_ground_fixture(GameMode::Survival).await;
        let client = crate::Client::from_java_1_21_11(api.bot.clone());
        let handle = client.player_control(GameMode::Survival);
        if refusal != "no_owner" {
            handle.start_control().await.unwrap();
            tick(&session, &handle, 1).await;
        }
        let mut state = session.state.lock().await;
        let before = state.control.session.as_ref().map(|s| s.body.velocity);
        match refusal {
            "flight" => state.control.session.as_mut().unwrap().kind = ControlKind::Flight,
            "glide" => state.control.session.as_mut().unwrap().kind = ControlKind::Glide,
            "vehicle" => {
                state
                    .receive(ids::play_clientbound::VEHICLE_MOVE, &[0; 32], 256)
                    .unwrap();
            }
            "world" => state.loading.generation += 1,
            "world_packet" => {
                state.dimensions = vec![Dimension::new(-64, 384).unwrap()];
                let mut packet = vec![0];
                put_string(&mut packet, "minecraft:overworld");
                packet.extend([0; 8]);
                packet.extend([0, 255, 0, 1, 0, 0, 63, 0]);
                state
                    .receive(ids::play_clientbound::RESPAWN, &packet, 256)
                    .unwrap();
            }
            "write" => {
                let generation = state.loading.generation;
                let sequence = state.sequence;
                let position = state.position.unwrap();
                state
                    .motion
                    .begin(generation, sequence, position, [0.; 2])
                    .unwrap();
            }
            "death" => {
                let mut packet = 0f32.to_be_bytes().to_vec();
                packet.push(20);
                packet.extend(5f32.to_be_bytes());
                state
                    .receive(ids::play_clientbound::UPDATE_HEALTH, &packet, 256)
                    .unwrap();
            }
            _ => {}
        }
        let mut packet = explosion(Some([0.125, 0.25, 0.]));
        if refusal == "undecoded" {
            packet.truncate(57);
            put_varint(&mut packet, 9999);
        }
        state
            .receive(ids::play_clientbound::EXPLOSION, &packet, 256)
            .unwrap();
        assert_eq!(
            state.control.session.as_ref().map(|s| s.body.velocity),
            before,
            "{refusal}"
        );
        let interruption = state
            .operations
            .local_player
            .motion_interruption
            .as_ref()
            .unwrap();
        assert_eq!(interruption.packet_id, ids::play_clientbound::EXPLOSION);
        assert_eq!(interruption.receive_sequence, state.sequence);
        assert_eq!(
            state
                .explosions
                .after(0, state.sequence)
                .unwrap()
                .explosions
                .len(),
            1
        );
        drop(state);
        assert!(handle.start_control().await.is_err());
        session.stop();
    }
}

#[tokio::test]
async fn explosion_survives_cancelled_stop_wait_without_replaying_on_new_owner() {
    use crate::client::{adapter::ControlOps, control::Output};
    let (session, api, mut peer) = common_ground_fixture(GameMode::Survival).await;
    {
        let mut state = session.state.lock().await;
        let mut owner = ControlSession::new(
            crate::MinecraftVersion::Java1_21_11,
            1,
            state.position.unwrap(),
            &received(&state),
        );
        owner.dispatched(&Output {
            sneak: None,
            sprint: Some(true),
            input: Some(1),
            position: state.position.unwrap(),
            rotation: [0.; 2],
            on_ground: true,
            horizontal_collision: false,
        });
        let generation = state.loading.generation;
        state.control.seed_session(owner, generation);
        state
            .receive(
                ids::play_clientbound::EXPLOSION,
                &explosion(Some([0.125, 0.5, 0.])),
                256,
            )
            .unwrap();
        assert_eq!(
            state.control.session.as_ref().unwrap().body.velocity,
            [0.125, 0.5, 0.]
        );
    }
    let writer = session.writer.lock().await;
    let mut waiting = Box::pin(api.stop_control_for(1));
    assert!(
        timeout(Duration::from_millis(30), waiting.as_mut())
            .await
            .is_err()
    );
    drop(waiting);
    drop(writer);
    assert_eq!(
        timeout(Duration::from_secs(1), read_packet(&mut peer, None))
            .await
            .unwrap()
            .unwrap(),
        (ids::play_serverbound::PLAYER_INPUT, vec![0])
    );
    read_packet(&mut peer, None).await.unwrap(); // Stop sprint.
    let state = session.state.lock().await;
    let previous = state.control.session.as_ref().unwrap();
    assert!(matches!(previous.status, ControlStatus::Stopped { .. }));
    let receipts = received(&state);
    let mut next = previous.resume(2, [0.; 2], &receipts);
    assert_eq!(next.body.velocity, [0.125, 0.5, 0.]);
    // Repeated private delivery, too, is fenced independently of public history.
    assert!(next.ground_explosion(state.sequence, Some([0.125, 0.5, 0.]), &receipts));
    assert_eq!(next.body.velocity, [0.125, 0.5, 0.]);
    assert_eq!(
        state
            .explosions
            .after(0, state.sequence)
            .unwrap()
            .explosions
            .len(),
        1
    );
    drop(state);
    session.stop();
}

#[tokio::test(start_paused = true)]
async fn malformed_explosion_never_applies_or_publishes_and_death_cannot_be_restored() {
    for malformed in [true, false] {
        let (session, api, _peer) = common_ground_fixture(GameMode::Survival).await;
        let client = crate::Client::from_java_1_21_11(api.bot.clone());
        let handle = client.player_control(GameMode::Survival);
        handle.start_control().await.unwrap();
        tick(&session, &handle, 1).await;
        let mut state = session.state.lock().await;
        let before = state.control.session.as_ref().unwrap().body.velocity;
        let mut bytes = explosion(Some([0.125, 0.5, 0.]));
        if malformed {
            bytes.pop();
            assert!(
                state
                    .receive(ids::play_clientbound::EXPLOSION, &bytes, 256)
                    .is_err()
            );
            assert!(state.failure.is_some());
            assert!(
                state
                    .explosions
                    .after(0, state.sequence)
                    .unwrap()
                    .explosions
                    .is_empty()
            );
        } else {
            for health in [0f32, 20.] {
                let mut bytes = health.to_be_bytes().to_vec();
                bytes.push(20);
                bytes.extend(5f32.to_be_bytes());
                state
                    .receive(ids::play_clientbound::UPDATE_HEALTH, &bytes, 256)
                    .unwrap();
            }
            state
                .receive(ids::play_clientbound::EXPLOSION, &bytes, 256)
                .unwrap();
            assert!(state.operations.local_player.motion_interruption.is_some());
        }
        assert_eq!(
            state.control.session.as_ref().unwrap().body.velocity,
            before
        );
        drop(state);
        session.stop();
    }
}

#[tokio::test]
async fn explosion_after_partial_or_unflushed_release_keeps_the_failed_owner_stopped() {
    use crate::client::{adapter::ControlOps, control::Output};
    for (after, flush) in [(Some(1), false), (Some(4), false), (None, true)] {
        let (session, api, _peer) = common_ground_fixture(GameMode::Survival).await;
        {
            let mut state = session.state.lock().await;
            let mut owner = ControlSession::new(
                crate::MinecraftVersion::Java1_21_11,
                1,
                state.position.unwrap(),
                &received(&state),
            );
            owner.dispatched(&Output {
                sneak: None,
                sprint: Some(true),
                input: Some(1),
                position: state.position.unwrap(),
                rotation: [0.; 2],
                on_ground: true,
                horizontal_collision: false,
            });
            let generation = state.loading.generation;
            state.control.seed_session(owner, generation);
            state
                .receive(
                    ids::play_clientbound::EXPLOSION,
                    &explosion(Some([0.125, 0.5, 0.])),
                    256,
                )
                .unwrap();
        }
        // Exercise the real release writer: either only part of its header is
        // written, or its complete bytes fail to flush. Neither is a clean stop.
        let writer = crate::client::ledger::testing::Control::default();
        writer.release(usize::MAX, true);
        writer.fail(after, flush);
        session
            .writer
            .lock()
            .await
            .stream
            .test_replace(writer.writer());
        assert_eq!(
            api.stop_control_for(1).await.unwrap_err().kind(),
            ErrorKind::UncertainDispatch
        );
        assert_eq!(writer.bytes().len(), after.unwrap_or(3));
        let stopped = api.control_record().await.unwrap().unwrap();
        assert!(matches!(
            &stopped.status,
            ControlStatus::Stopped { reason } if reason.contains("release write failed")
        ));
        {
            let mut state = session.state.lock().await;
            state
                .receive(
                    ids::play_clientbound::EXPLOSION,
                    &explosion(Some([0.25, 0.25, 0.])),
                    256,
                )
                .unwrap();
            assert_eq!(
                state.control.session.as_ref().unwrap().body.velocity,
                [0.125, 0.5, 0.]
            );
            let interruption = state
                .operations
                .local_player
                .motion_interruption
                .as_ref()
                .unwrap();
            assert_eq!(interruption.packet_id, ids::play_clientbound::EXPLOSION);
            assert_eq!(interruption.receive_sequence, state.sequence);
            assert_eq!(
                state
                    .explosions
                    .after(0, state.sequence)
                    .unwrap()
                    .explosions
                    .len(),
                2
            );
        }
        let bytes = writer.bytes();
        assert!(api.start_control(GameMode::Survival).await.is_err());
        assert_eq!(api.control_record().await.unwrap().unwrap(), stopped);
        assert_eq!(writer.bytes(), bytes);
        session.stop();
    }
}

#[tokio::test(start_paused = true)]
async fn explosion_before_first_tick_keeps_model_and_stationary_receipts_separate() {
    let (session, api, _peer) = common_ground_fixture(GameMode::Survival).await;
    assert!(api.standing_context().await.is_ok());
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    let handle = client.player_control(GameMode::Survival);
    handle.start_control().await.unwrap();
    {
        let mut state = session.state.lock().await;
        assert_eq!(state.control.session.as_ref().unwrap().dispatched_ticks, 0);
        state
            .receive(
                ids::play_clientbound::EXPLOSION,
                &explosion(Some([0.125, 0.25, 0.])),
                256,
            )
            .unwrap();
        assert_eq!(
            state.control.session.as_ref().unwrap().body.velocity,
            [0.125, 0.25, 0.]
        );
        assert_eq!(
            state.operations.local_player.velocity.unwrap().value,
            [0.; 3]
        );
    }
    assert!(api.standing_context().await.is_err());
    tick(&session, &handle, 1).await;
    let frame = handle
        .control_record()
        .await
        .unwrap()
        .unwrap()
        .frame
        .unwrap();
    assert_eq!(frame.position, [8.625, 65.25, 8.5]);
    session.stop();
}

#[tokio::test(start_paused = true)]
async fn explosion_and_death_do_not_seed_a_fresh_respawn_owner() {
    let (session, api, _peer) = common_ground_fixture(GameMode::Survival).await;
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    let handle = client.player_control(GameMode::Survival);
    let old = handle.start_control().await.unwrap();
    tick(&session, &handle, 1).await;
    {
        let mut state = session.state.lock().await;
        state
            .receive(
                ids::play_clientbound::EXPLOSION,
                &explosion(Some([0.125, 0.5, 0.])),
                256,
            )
            .unwrap();
        let mut health = 0f32.to_be_bytes().to_vec();
        health.push(20);
        health.extend(5f32.to_be_bytes());
        state
            .receive(ids::play_clientbound::UPDATE_HEALTH, &health, 256)
            .unwrap();
    }
    tokio::time::advance(Duration::from_millis(50)).await;
    loop {
        if matches!(
            handle.control_record().await.unwrap().unwrap().status,
            ControlStatus::Stopped { .. }
        ) {
            break;
        }
        tokio::task::yield_now().await;
    }
    {
        let mut state = session.state.lock().await;
        let floor = std::mem::take(&mut state.world);
        state.dimensions = vec![Dimension::new(-64, 384).unwrap()];
        let mut packet = vec![0];
        put_string(&mut packet, "minecraft:overworld");
        packet.extend([0; 8]);
        packet.extend([0, 255, 0, 1, 0, 0, 63, 0]);
        state
            .receive(ids::play_clientbound::RESPAWN, &packet, 256)
            .unwrap();
        let generation = state.loading.generation;
        state.world = floor; // Independent known floor in this unit fixture's new world.
        state
            .receive(
                ids::play_clientbound::POSITION,
                &correction([8.5, 65., 8.5], [0.; 3], 0),
                256,
            )
            .unwrap();
        let mut health = 20f32.to_be_bytes().to_vec();
        health.push(20);
        health.extend(5f32.to_be_bytes());
        state
            .receive(ids::play_clientbound::UPDATE_HEALTH, &health, 256)
            .unwrap();
        state.loading = loading::InteractionLoading::completed_fixture();
        state.loading.generation = generation;
        state.ready = true;
    }
    let new = handle.start_control().await.unwrap();
    assert_ne!(old.session_id, new.session_id);
    tick(&session, &handle, 1).await;
    let frame = handle
        .control_record()
        .await
        .unwrap()
        .unwrap()
        .frame
        .unwrap();
    assert_eq!(frame.position, [8.5, 65., 8.5]);
    assert_eq!(frame.velocity[0], 0.);
    assert!(frame.velocity[1] < 0.);
    session.stop();
}

#[tokio::test(start_paused = true)]
async fn captured_ground_explosions_apply_once_without_draining_observation() {
    let proof: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../docs/evidence/common-explosion-ground-20261023.json"
    ))
    .unwrap();
    assert_eq!(proof["status"], "PASS");
    let mut count = 0;
    for check in proof["checks"].as_array().unwrap() {
        let Some(packets) = check["original_packets"].as_array() else {
            continue;
        };
        for packet in packets
            .iter()
            .filter(|p| p["packet_id"] == ids::play_clientbound::EXPLOSION)
        {
            let (session, api, _peer) = common_ground_fixture(GameMode::Survival).await;
            let client = crate::Client::from_java_1_21_11(api.bot.clone());
            let handle = client.player_control(GameMode::Survival);
            handle.start_control().await.unwrap();
            tick(&session, &handle, 1).await;
            let before;
            // Retain exact wire bits: diagnostic JSON decimal parsing can differ by one ULP.
            let impulse = std::array::from_fn(|i| {
                f64::from_bits(
                    u64::from_str_radix(check["player_knockback_bits"][i].as_str().unwrap(), 16)
                        .unwrap(),
                )
            });
            {
                let mut state = session.state.lock().await;
                before = state.control.session.as_ref().unwrap().body.velocity;
                let bytes = hex::decode(packet["body_hex"].as_str().unwrap()).unwrap();
                state
                    .receive(ids::play_clientbound::EXPLOSION, &bytes, 256)
                    .unwrap();
                assert!(state.operations.local_player.motion_interruption.is_none());
                assert_eq!(
                    state.control.session.as_ref().unwrap().body.velocity,
                    std::array::from_fn(|i| before[i] + impulse[i])
                );
            }
            for _ in 0..2 {
                let log = client.explosions_after(0).await.unwrap();
                assert_eq!(log.explosions.len(), 1);
                assert_eq!(log.explosions[0].explosion.player_knockback, Some(impulse));
                assert_eq!(
                    session
                        .state
                        .lock()
                        .await
                        .control
                        .session
                        .as_ref()
                        .unwrap()
                        .body
                        .velocity,
                    std::array::from_fn(|i| before[i] + impulse[i])
                );
            }
            session.stop();
            count += 1;
        }
    }
    assert_eq!(count, 3);
}

#[tokio::test(start_paused = true)]
async fn explosion_after_finite_owner_admission_cannot_use_the_stopped_ground_model() {
    use crate::client::{adapter::ControlOps, survival::SurvivalControl};
    let (session, api, mut peer) = common_ground_fixture(GameMode::Survival).await;
    {
        let mut state = session.state.lock().await;
        let owner = ControlSession::new(
            crate::MinecraftVersion::Java1_21_11,
            1,
            state.position.unwrap(),
            &received(&state),
        );
        let generation = state.loading.generation;
        state.control.seed_session(owner, generation);
    }
    api.stop_control_for(1).await.unwrap();
    // The actual finite admission retains its owner before spawning the first tick.
    let controls = [SurvivalControl {
        yaw: 0.,
        input: Default::default(),
    }; 4];
    let finite = api.start_predicted_survival_path(&controls).await.unwrap();
    assert_eq!(finite.dispatched_ticks, 0);
    {
        let mut state = session.state.lock().await;
        assert_eq!(state.survival_motion.as_ref().unwrap().dispatched_ticks, 0);
        assert!(state.motion.last_submission.is_none());
        let before = state.control.finite.as_ref().unwrap().model.frame.velocity;
        state
            .receive(
                ids::play_clientbound::EXPLOSION,
                &explosion(Some([0.125, 0.5, 0.])),
                256,
            )
            .unwrap();
        assert_eq!(
            state.control.finite.as_ref().unwrap().model.frame.velocity,
            before
        );
        let interruption = state
            .operations
            .local_player
            .motion_interruption
            .as_ref()
            .unwrap();
        assert_eq!(interruption.packet_id, ids::play_clientbound::EXPLOSION);
        assert_eq!(interruption.receive_sequence, state.sequence);
    }
    tokio::time::advance(Duration::from_millis(50)).await;
    timeout(Duration::from_secs(2), async {
        loop {
            let state = session.state.lock().await;
            let record = state.survival_motion.as_ref().unwrap();
            assert_eq!(record.dispatched_ticks, 0);
            if record.status != operations::SurvivalMotionStatus::Running {
                break;
            }
            drop(state);
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        timeout(Duration::from_millis(20), read_packet(&mut peer, None))
            .await
            .is_err()
    );
    session.stop();
}
