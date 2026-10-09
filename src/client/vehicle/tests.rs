use super::*;
#[test]
fn continuous_mount_keeps_identity_while_other_passengers_change() {
    let session = SessionStamp {
        version: MinecraftVersion::Java1_21_11,
        connection_id: 4,
        world_generation: 7,
    };
    let spawns = SpawnLedger::default();
    let mut ledger = PassengerLedger::default();
    ledger.receive(
        &NativePassengers::decode(&[10, 1, 42]).unwrap(),
        Some(42),
        &spawns,
        12,
    );
    let VehicleRelation::Mounted { mount } = ledger
        .capture(session, 12, Some(42), &spawns)
        .relation
        .unwrap()
        .value
    else {
        panic!()
    };
    ledger.receive(
        &NativePassengers::decode(&[10, 2, 43, 42]).unwrap(),
        Some(42),
        &spawns,
        13,
    );
    let refreshed = ledger.capture(session, 13, Some(42), &spawns);
    assert_eq!(
        refreshed.relation.as_ref().unwrap().value,
        VehicleRelation::Mounted { mount }
    );
    assert_eq!(
        refreshed.relation.unwrap().source,
        ValueSource::Received { sequence: 13 }
    );
    ledger.receive(
        &NativePassengers::decode(&[10, 1, 43]).unwrap(),
        Some(42),
        &spawns,
        14,
    );
    assert_eq!(
        ledger
            .capture(session, 14, Some(42), &spawns)
            .relation
            .unwrap()
            .value,
        VehicleRelation::Unmounted {
            previous_mount: mount
        }
    );
}

#[test]
fn dismount_inputs_and_passenger_fields_match_original_packet_codecs() {
    let oracle: serde_json::Value = serde_json::from_str(include_str!(
        "../../../data/client_api/vehicle_input_packets.json"
    ))
    .unwrap();
    for v in oracle["versions"].as_array().unwrap() {
        let version = if v["version"] == "1.16.1" {
            MinecraftVersion::Java1_16_1
        } else {
            MinecraftVersion::Java1_21_11
        };
        for row in v["inputs"].as_array().unwrap() {
            let (_, payload) = super::dismount::payload(version, !row["shift"].as_bool().unwrap());
            assert_eq!(hex::encode(payload), row["payload_hex"].as_str().unwrap());
        }
        for row in v["passengers"].as_array().unwrap() {
            let decoded = NativePassengers::decode(
                &hex::decode(row["payload_hex"].as_str().unwrap()).unwrap(),
            )
            .unwrap();
            assert_eq!(
                decoded.vehicle,
                i32::try_from(row["vehicle"].as_i64().unwrap()).unwrap()
            );
            assert_eq!(serde_json::json!(decoded.passengers), row["passengers"]);
        }
    }
}
use crate::{
    MinecraftVersion,
    client::{ValueSource, entity::NativeSpawn},
};

#[test]
fn vehicle_receipts_keep_unknown_explicit_absence_and_original_lifetimes_distinct() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let session = SessionStamp {
            version,
            connection_id: 4,
            world_generation: 7,
        };
        let mut spawns = SpawnLedger::default();
        let mut ledger = PassengerLedger::default();
        let capture = |ledger: &PassengerLedger, spawns: &SpawnLedger| {
            ledger.capture(session, 100, Some(42), spawns)
        };
        assert!(capture(&ledger, &spawns).relation.is_none());
        // Another vehicle's absence supplies no own-player baseline.
        ledger.receive(
            &NativePassengers::decode(&[10, 0]).unwrap(),
            Some(42),
            &spawns,
            10,
        );
        assert!(capture(&ledger, &spawns).relation.is_none());
        let spawn = || NativeSpawn {
            living: None,
            id: 10,
            uuid: Some([7; 16]),
            type_id: None,
            dedicated_type_name: Some("minecart"),
            position: [0.5, 65.0, 2.5],
        };
        spawns.insert(version, spawn(), 11, 4).unwrap();
        ledger.receive(
            &NativePassengers::decode(&[10, 2, 43, 42]).unwrap(),
            Some(42),
            &spawns,
            12,
        );
        let mounted = capture(&ledger, &spawns);
        let VehicleRelation::Mounted { mount } = mounted.relation.as_ref().unwrap().value else {
            panic!()
        };
        assert_eq!(mount.vehicle(), spawns.identity(session, 10));
        assert_eq!(mount.receive_sequence(), 12);
        assert_eq!(
            mounted.relation.as_ref().unwrap().source,
            ValueSource::Received { sequence: 12 }
        );
        assert_eq!(mounted.passengers.unwrap().value, [43, 42]);
        ledger.receive(
            &NativePassengers::decode(&[20, 0]).unwrap(),
            Some(42),
            &spawns,
            13,
        );
        assert_eq!(
            capture(&ledger, &spawns).relation.unwrap().source,
            ValueSource::Received { sequence: 12 }
        );
        ledger.receive(
            &NativePassengers::decode(&[10, 1, 43]).unwrap(),
            Some(42),
            &spawns,
            14,
        );
        let unmounted = capture(&ledger, &spawns);
        assert_eq!(
            unmounted.relation.as_ref().unwrap().value,
            VehicleRelation::Unmounted {
                previous_mount: mount
            }
        );
        assert_eq!(
            unmounted.relation.unwrap().source,
            ValueSource::Received { sequence: 14 }
        );
        assert!(ledger.motion_interrupted());
        // Despawn/reuse is unknown, never a synthetic dismount or a new mount.
        spawns.remove(10);
        ledger.retire(10);
        assert!(capture(&ledger, &spawns).relation.is_none());
        spawns.insert(version, spawn(), 15, 4).unwrap();
        assert!(capture(&ledger, &spawns).relation.is_none());
        ledger.receive(
            &NativePassengers::decode(&[10, 1, 42]).unwrap(),
            Some(42),
            &spawns,
            16,
        );
        let VehicleRelation::Mounted { mount: next } =
            capture(&ledger, &spawns).relation.unwrap().value
        else {
            panic!()
        };
        assert_ne!(next, mount);
        assert_eq!(next.vehicle().unwrap().spawn_sequence(), 15);
        // A different vehicle has an actual own-player list but no known spawn.
        ledger.receive(
            &NativePassengers::decode(&[20, 1, 42]).unwrap(),
            Some(42),
            &spawns,
            17,
        );
        let VehicleRelation::Mounted { mount: unresolved } =
            capture(&ledger, &spawns).relation.unwrap().value
        else {
            panic!()
        };
        assert!(unresolved.vehicle().is_none());
        ledger.receive(
            &NativePassengers::decode(&[10, 0]).unwrap(),
            Some(42),
            &spawns,
            18,
        );
        assert!(
            matches!(capture(&ledger, &spawns).relation.unwrap().value, VehicleRelation::Mounted { mount } if mount == unresolved)
        );
        let mut later = spawn();
        later.id = 20;
        spawns.insert(version, later, 19, 4).unwrap();
        assert!(
            matches!(capture(&ledger, &spawns).relation.unwrap().value, VehicleRelation::Mounted { mount } if mount.vehicle().is_none())
        );
        ledger.clear();
        assert!(capture(&ledger, &spawns).relation.is_none());
        assert!(!ledger.motion_interrupted());
    }
}

#[test]
fn passenger_frames_validate_all_ids_count_and_end_before_any_application() {
    for bytes in [
        &[10, 1][..],
        &[10, 1, 42, 0],
        &[10, 2, 42, 42],
        &[10, 1, 10],
        &[255, 255, 255, 255, 15, 0],
        &[10, 1, 255, 255, 255, 255, 15],
        &[10, 129, 8],
    ] {
        assert!(NativePassengers::decode(bytes).is_err(), "{bytes:?}");
    }
    let valid = NativePassengers::decode(&[128, 1, 2, 172, 2, 42]).unwrap();
    assert_eq!(valid.vehicle, 128);
    assert_eq!(valid.passengers, [300, 42]);
}

#[test]
fn vehicle_control_digital_fields_match_all_original_native_packet_codecs() {
    let oracle: serde_json::Value = serde_json::from_str(include_str!(
        "../../../data/client_api/vehicle_control_packets.json"
    ))
    .unwrap();
    for row in oracle["versions"].as_array().unwrap() {
        let version = if row["version"] == "1.16.1" {
            MinecraftVersion::Java1_16_1
        } else {
            MinecraftVersion::Java1_21_11
        };
        assert_eq!(row["inputs"].as_array().unwrap().len(), 18);
        for sample in row["inputs"].as_array().unwrap() {
            let input = control::VehicleInput {
                forward: sample["forward"].as_i64().unwrap() as i8,
                strafe: sample["strafe"].as_i64().unwrap() as i8,
                jump: sample["jump"].as_bool().unwrap(),
            };
            assert_eq!(
                hex::encode(control::payload(version, input).1),
                sample["payload_hex"].as_str().unwrap()
            );
        }
    }
}
#[test]
fn vehicle_control_requires_final_neutral_and_latches_original_mount_conflicts() {
    use crate::client::{GameMode, Health, InventoryObservation, PlayerObservation, received};
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let session = SessionStamp {
            version,
            connection_id: 4,
            world_generation: 7,
        };
        let spawns = SpawnLedger::default();
        let mut ledger = PassengerLedger::default();
        ledger.receive(
            &NativePassengers {
                vehicle: 10,
                passengers: vec![42],
            },
            Some(42),
            &spawns,
            12,
        );
        let vehicle = ledger.capture(session, 12, Some(42), &spawns);
        let VehicleRelation::Mounted { mount } = vehicle.relation.as_ref().unwrap().value else {
            panic!()
        };
        let player = PlayerObservation {
            using_item: None,
            entity_id: None,
            attributes: Default::default(),
            effects: Default::default(),
            air_supply: None,
            world_time: None,
            session,
            receive_sequence: 12,
            pending_dispatch: false,
            dimension: None,
            position: None,
            received_pose: None,
            rotation: [0.; 2],
            rotation_source: None,
            on_ground: None,
            game_mode: Some(GameMode::Survival),
            may_fly: None,
            health: Some(received(
                Health {
                    health: 20.,
                    food: 20,
                    saturation: 5.,
                },
                11,
            )),
            selected_hotbar: None,
            inventory: InventoryObservation::default(),
        };
        let forward = control::VehicleInput {
            forward: 1,
            ..Default::default()
        };
        let inputs = [forward, Default::default()];
        for invalid in [
            vec![],
            vec![forward],
            vec![forward; 121],
            vec![
                control::VehicleInput {
                    forward: 2,
                    ..Default::default()
                },
                Default::default(),
            ],
            vec![
                control::VehicleInput {
                    strafe: -2,
                    ..Default::default()
                },
                Default::default(),
            ],
        ] {
            assert!(
                control::prepare(
                    player.clone(),
                    vehicle.clone(),
                    GameMode::Survival,
                    mount,
                    &invalid,
                    None
                )
                .is_err()
            );
        }
        assert!(
            control::prepare(
                player.clone(),
                vehicle.clone(),
                GameMode::Creative,
                mount,
                &inputs,
                None
            )
            .is_err()
        );
        let mut record = control::prepare(
            player.clone(),
            vehicle.clone(),
            GameMode::Survival,
            mount,
            &inputs,
            None,
        )
        .unwrap();
        // Even an unchanged mount is no longer a valid prediction context
        // after the original server corrects vehicle motion.
        let mut corrected_record = record.clone();
        let mut corrected_vehicle = vehicle.clone();
        corrected_vehicle.motion_correction_sequence = Some(13);
        let mut corrected_player = player.clone();
        corrected_player.receive_sequence = 13;
        corrected_vehicle.receive_sequence = 13;
        control::receive(&mut corrected_record, &corrected_player, &corrected_vehicle);
        assert_eq!(
            corrected_record.stage,
            control::VehicleControlStage::RequiresInspection
        );
        assert_eq!(corrected_record.dispatched_ticks, 0);
        assert!(control::validate(&corrected_record, &player, &vehicle).is_err());
        let correction_reason = corrected_record.requires_inspection.clone();
        control::receive(&mut corrected_record, &player, &vehicle);
        assert_eq!(corrected_record.requires_inspection, correction_reason);
        assert!(
            control::prepare(
                player.clone(),
                vehicle.clone(),
                GameMode::Survival,
                mount,
                &inputs,
                Some(&record)
            )
            .is_err()
        );
        ledger.receive(
            &NativePassengers {
                vehicle: 10,
                passengers: vec![43, 42],
            },
            Some(42),
            &spawns,
            13,
        );
        let mut newer = player.clone();
        newer.receive_sequence = 13;
        let same = ledger.capture(session, 13, Some(42), &spawns);
        control::receive(&mut record, &newer, &same);
        assert_eq!(record.stage, control::VehicleControlStage::Running);
        ledger.receive(
            &NativePassengers {
                vehicle: 10,
                passengers: vec![43],
            },
            Some(42),
            &spawns,
            14,
        );
        newer.receive_sequence = 14;
        control::receive(
            &mut record,
            &newer,
            &ledger.capture(session, 14, Some(42), &spawns),
        );
        assert_eq!(
            record.stage,
            control::VehicleControlStage::RequiresInspection
        );
        let first = record.requires_inspection.clone();
        ledger.receive(
            &NativePassengers {
                vehicle: 10,
                passengers: vec![42],
            },
            Some(42),
            &spawns,
            15,
        );
        newer.receive_sequence = 15;
        control::receive(
            &mut record,
            &newer,
            &ledger.capture(session, 15, Some(42), &spawns),
        );
        assert_eq!(record.requires_inspection, first);
        assert!(control::validate(&record, &player, &vehicle).is_err());
        let mut completed = control::prepare(
            player.clone(),
            vehicle.clone(),
            GameMode::Survival,
            mount,
            &inputs,
            None,
        )
        .unwrap();
        completed.stage = control::VehicleControlStage::Submitted;
        let next = control::prepare(
            player.clone(),
            vehicle.clone(),
            GameMode::Survival,
            mount,
            &inputs,
            Some(&completed),
        )
        .unwrap();
        assert_eq!(next.id.attempt(), 2);
        let mut changed = player.clone();
        changed.pending_dispatch = true;
        assert!(
            control::prepare(
                changed,
                vehicle.clone(),
                GameMode::Survival,
                mount,
                &inputs,
                None
            )
            .is_err()
        );
        let mut changed = player.clone();
        changed.session.world_generation += 1;
        assert!(
            control::prepare(
                changed,
                vehicle.clone(),
                GameMode::Survival,
                mount,
                &inputs,
                None
            )
            .is_err()
        );
    }
}

fn boat_velocity_fixture(
    version: MinecraftVersion,
) -> (
    control::VehicleControlRecord,
    crate::client::EntityMotionObservation,
) {
    use crate::client::{
        EntityMotionObservation, EntityPosition, GameMode, Health, InventoryObservation,
        PlayerObservation, received,
    };
    let session = SessionStamp {
        version,
        connection_id: 4,
        world_generation: 7,
    };
    let mut spawns = SpawnLedger::default();
    spawns
        .insert(
            version,
            NativeSpawn {
                id: 10,
                uuid: Some([7; 16]),
                type_id: None,
                dedicated_type_name: Some(if version == MinecraftVersion::Java1_16_1 {
                    "boat"
                } else {
                    "oak_boat"
                }),
                position: [0.5, 65., 2.5],
                living: None,
            },
            11,
            4,
        )
        .unwrap();
    let mut ledger = PassengerLedger::default();
    ledger.receive(
        &NativePassengers {
            vehicle: 10,
            passengers: vec![42],
        },
        Some(42),
        &spawns,
        12,
    );
    let vehicle = ledger.capture(session, 12, Some(42), &spawns);
    let VehicleRelation::Mounted { mount } = vehicle.relation.as_ref().unwrap().value else {
        panic!()
    };
    let player = PlayerObservation {
        using_item: None,
        entity_id: None,
        attributes: Default::default(),
        effects: Default::default(),
        air_supply: None,
        world_time: None,
        session,
        receive_sequence: 12,
        pending_dispatch: false,
        dimension: None,
        position: None,
        received_pose: None,
        rotation: [0.; 2],
        rotation_source: None,
        on_ground: None,
        game_mode: Some(GameMode::Survival),
        may_fly: None,
        health: Some(received(
            Health {
                health: 20.,
                food: 20,
                saturation: 5.,
            },
            11,
        )),
        selected_hotbar: None,
        inventory: InventoryObservation::default(),
    };
    let record = control::prepare(
        player,
        vehicle,
        GameMode::Survival,
        mount,
        &[Default::default(); 3],
        None,
    )
    .unwrap();
    let motion = EntityMotionObservation {
        entity: spawns.capture(session, 12).entities.remove(0),
        receive_sequence: 12,
        position: Some(received(
            EntityPosition {
                position: [0.5, 65., 2.5],
                quantization_error: [0.; 3],
            },
            11,
        )),
        rotation: Some(received([0.; 2], 11)),
        head_yaw: None,
        velocity: Some(received([0.; 3], 11)),
        on_ground: None,
        correction: None,
    };
    (record, motion)
}

#[test]
fn boat_velocity_receipts_fold_once_without_rewriting_submitted_frames() {
    use crate::client::{GameMode, received};
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let (mut prior, motion) = boat_velocity_fixture(version);
        control::configure_boat(&mut prior, Some(motion.clone()), None).unwrap();
        let frame = control::BoatFrame::new([1.5, 65., 2.5], [0.; 2], [0.4, 0.2, 0.1]);
        prior
            .boat_motion
            .as_mut()
            .unwrap()
            .frames
            .push(frame.clone());
        prior.dispatched_ticks = 3;
        prior.attempted_tick = 3;
        prior.stage = control::VehicleControlStage::Submitted;
        let make = |previous: &control::VehicleControlRecord| {
            control::prepare(
                previous.initial.clone(),
                previous.vehicle.clone(),
                GameMode::Survival,
                previous.id.mount(),
                &[Default::default(); 3],
                Some(previous),
            )
            .unwrap()
        };
        let mut unchanged = make(&prior);
        control::configure_boat(&mut unchanged, Some(motion.clone()), Some(&prior)).unwrap();
        assert_eq!(unchanged.boat_motion.as_ref().unwrap().initial_frame, frame);
        let mut live = motion.clone();
        live.receive_sequence = 13;
        live.velocity = Some(received([0., 2.7, 0.], 13));
        assert!(control::receive_boat_velocity(&mut unchanged, &live).unwrap());
        assert!(!control::receive_boat_velocity(&mut unchanged, &live).unwrap());
        let boat = unchanged.boat_motion.as_ref().unwrap();
        assert_eq!(boat.velocity_updates.len(), 1);
        assert_eq!(boat.velocity_updates[0].sampled_before_tick, 1);
        assert_eq!(boat.initial_frame, frame);
        assert_eq!(prior.boat_motion.as_ref().unwrap().frames, [frame.clone()]);
        let air = crate::NativeBlockState {
            name: "minecraft:air".into(),
            properties: Default::default(),
        };
        let mut blocks = |_: [i32; 3]| Ok(air.clone());
        let first = control::boat_step(&unchanged, Default::default(), &mut blocks)
            .unwrap()
            .unwrap();
        assert_eq!(first.position[0], frame.position[0]);
        assert!(first.velocity[1] > 2.6);
        unchanged
            .boat_motion
            .as_mut()
            .unwrap()
            .frames
            .push(first.clone());
        unchanged.boat_motion.as_mut().unwrap().pending_velocity = None;
        unchanged.dispatched_ticks = 1;
        assert!(!control::receive_boat_velocity(&mut unchanged, &live).unwrap());
        let second = control::boat_step(&unchanged, Default::default(), &mut blocks)
            .unwrap()
            .unwrap();
        assert!(second.velocity[1] < first.velocity[1]);
        assert_eq!(unchanged.boat_motion.as_ref().unwrap().frames, [first]);
        // A new finite plan also consumes the newest original receipt only once.
        let mut player = prior.initial.clone();
        player.receive_sequence = 13;
        let mut vehicle = prior.vehicle.clone();
        vehicle.receive_sequence = 13;
        let mut restarted = control::prepare(
            player,
            vehicle,
            GameMode::Survival,
            prior.id.mount(),
            &[Default::default(); 3],
            Some(&prior),
        )
        .unwrap();
        control::configure_boat(&mut restarted, Some(live.clone()), Some(&prior)).unwrap();
        assert_eq!(
            restarted
                .boat_motion
                .as_ref()
                .unwrap()
                .initial_frame
                .position,
            frame.position
        );
        assert_eq!(
            restarted
                .boat_motion
                .as_ref()
                .unwrap()
                .initial_frame
                .velocity,
            [0., 2.7, 0.]
        );
        assert!(
            restarted
                .boat_motion
                .as_ref()
                .unwrap()
                .velocity_updates
                .is_empty()
        );
    }
}

#[test]
fn boat_velocity_receipts_refuse_stale_spawn_future_sources_and_failed_owners() {
    use crate::client::received;
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let (mut record, motion) = boat_velocity_fixture(version);
        control::configure_boat(&mut record, Some(motion.clone()), None).unwrap();
        let mut live = motion.clone();
        live.receive_sequence = 13;
        live.velocity = Some(received([0., -0.7, 0.], 13));
        let mut other = boat_velocity_fixture(version).1;
        let mut stale = record.initial.session;
        stale.world_generation += 1;
        let mut spawns = SpawnLedger::default();
        spawns
            .insert(
                version,
                NativeSpawn {
                    id: 10,
                    uuid: Some([7; 16]),
                    type_id: None,
                    dedicated_type_name: Some("boat"),
                    position: [0.5, 65., 2.5],
                    living: None,
                },
                11,
                4,
            )
            .unwrap();
        other.entity.id = spawns.identity(stale, 10).unwrap();
        assert!(control::receive_boat_velocity(&mut record, &other).is_err());
        let mut future = live.clone();
        future.receive_sequence = 12;
        assert!(control::receive_boat_velocity(&mut record, &future).is_err());
        assert!(control::configure_boat(&mut record, Some(future), None).is_err());
        assert!(control::receive_boat_velocity(&mut record, &live).unwrap());
        assert!(control::receive_boat_velocity(&mut record, &motion).is_err());
        record.inspection("uncertain write");
        live.receive_sequence = 14;
        live.velocity = Some(received([0., 2.7, 0.], 14));
        assert!(control::receive_boat_velocity(&mut record, &live).is_err());
        assert_eq!(
            record.requires_inspection.as_deref(),
            Some("uncertain write")
        );
        assert_eq!(
            record.boat_motion.as_ref().unwrap().velocity_updates.len(),
            1
        );
        assert_eq!(
            record.boat_motion.as_ref().unwrap().received_velocity.value,
            [0., -0.7, 0.]
        );
    }
}
