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
