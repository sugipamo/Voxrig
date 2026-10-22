use super::super::Phase;
use super::*;
use crate::client::{EventKind, ValueSource, effects::*};
use crate::protocol::{put_string, put_varint};

fn state() -> State {
    let mut state = State {
        phase: Phase::Configuration,
        connection_id: 42,
        ..State::default()
    };
    registry(&mut state, &["example:custom", "minecraft:generic"]);
    state
}
fn registry(state: &mut State, names: &[&str]) {
    let mut bytes = Vec::new();
    put_string(&mut bytes, "minecraft:damage_type");
    put_varint(&mut bytes, names.len() as i32);
    for name in names {
        put_string(&mut bytes, name);
        bytes.extend([1, 10, 0]); // present, unnamed empty compound
    }
    state
        .receive(
            crate::versions::java_1_21_11::ids::configuration_clientbound::REGISTRY_DATA,
            &bytes,
            16,
        )
        .unwrap();
    // Minimal constructed configuration: complete the registry receipt directly;
    // FINISH_CONFIGURATION additionally requires unrelated dimension fixtures.
    state.registries.finish();
    state.phase = Phase::Play;
}
fn damage(
    type_id: i32,
    cause: Option<i32>,
    direct: Option<i32>,
    position: Option<[f64; 3]>,
) -> Vec<u8> {
    let mut bytes = vec![];
    for value in [
        37,
        type_id,
        cause.unwrap_or(-1).wrapping_add(1),
        direct.unwrap_or(-1).wrapping_add(1),
    ] {
        put_varint(&mut bytes, value);
    }
    bytes.push(u8::from(position.is_some()));
    if let Some(position) = position {
        for value in position {
            bytes.extend(value.to_be_bytes());
        }
    }
    bytes
}
fn kill(player: i32) -> Vec<u8> {
    let mut bytes = vec![];
    put_varint(&mut bytes, player);
    bytes.extend([8, 0, 4]);
    bytes.extend(b"dead");
    bytes
}
fn hurt(yaw: f32) -> Vec<u8> {
    let mut bytes = vec![37];
    bytes.extend(yaw.to_be_bytes());
    bytes
}

#[test]
fn received_registry_order_and_all_optional_damage_fields_are_retained() {
    let mut state = state();
    for (cause, direct, position) in [
        (None, None, None),
        (Some(0), Some(99), None),
        (None, Some(37), Some([-12.5, 64.25, 8.125])),
        (Some(i32::MAX), Some(-2), Some([0., -64., 30000000.])),
    ] {
        state
            .receive(input::DAMAGE_EVENT, &damage(0, cause, direct, position), 16)
            .unwrap();
        let log = state
            .effects
            .after(state.sequence - 1, state.sequence)
            .unwrap();
        assert_eq!(log.effects.len(), 1);
        let observed = &log.effects[0];
        assert_eq!(observed.receive_sequence, state.sequence);
        assert_eq!(observed.session, state.snapshot_session());
        let PlayEffect::Damage(d) = &observed.effect else {
            panic!()
        };
        assert_eq!(d.entity_id, 37);
        assert_eq!(d.source_type.value(), 0);
        assert_eq!(d.source_type.registry(), "minecraft:damage_type");
        assert_eq!(d.source_type_name, "example:custom");
        assert_eq!(
            (d.cause_entity_id, d.direct_entity_id, d.source_position),
            (cause, direct, position)
        );
        let registry = state
            .registries
            .capture(state.snapshot_session(), state.sequence);
        assert_eq!(
            registry.resolve(&d.source_type).unwrap().name,
            d.source_type_name
        );
        assert_eq!(
            registry.registries()["minecraft:damage_type"].source,
            ValueSource::Received { sequence: 1 }
        );
        assert_eq!(
            state
                .events
                .after(0, state.sequence)
                .unwrap()
                .events
                .last()
                .unwrap()
                .kind,
            EventKind::EntityDamaged { native_id: 37 }
        );
    }
}

#[test]
fn hurt_combat_and_own_death_publish_complete_payloads_with_matching_notifications() {
    let mut state = state();
    state.operations.local_player.entity_id = Some(37);
    let mut duration = vec![];
    put_varint(&mut duration, -123);
    let packets = [
        (
            input::HURT_ANIMATION,
            hurt(-450.25),
            PlayEffect::HurtAnimation {
                entity_id: 37,
                yaw: -450.25,
            },
            EventKind::HurtAnimationReceived { native_id: 37 },
        ),
        (
            input::ENTER_COMBAT_EVENT,
            vec![],
            PlayEffect::CombatEntered,
            EventKind::CombatChanged,
        ),
        (
            input::END_COMBAT_EVENT,
            duration,
            PlayEffect::CombatEnded {
                duration_ticks: -123,
            },
            EventKind::CombatChanged,
        ),
        (
            input::DEATH_COMBAT_EVENT,
            kill(37),
            PlayEffect::CombatKill {
                player_id: 37,
                message: crate::client::ui::UiText::NativeNbt {
                    bytes: kill(37)[1..].to_vec(),
                },
            },
            EventKind::PlayerKilled { native_id: 37 },
        ),
    ];
    for (id, bytes, effect, kind) in packets {
        state.receive(id, &bytes, 16).unwrap();
        let log = state
            .effects
            .after(state.sequence - 1, state.sequence)
            .unwrap();
        assert_eq!(log.effects[0].effect, effect);
        let event = *state
            .events
            .after(0, state.sequence)
            .unwrap()
            .events
            .last()
            .unwrap();
        assert_eq!(
            (event.receive_sequence, event.kind),
            (log.effects[0].receive_sequence, kind)
        );
    }
    let death = state.death_message.clone().unwrap();
    assert_eq!(
        death.source,
        ValueSource::Received {
            sequence: state.sequence
        }
    );
    state
        .receive(input::DEATH_COMBAT_EVENT, &kill(99), 16)
        .unwrap();
    assert_eq!(state.death_message.unwrap().value, death.value);
}

#[test]
fn malformed_packets_never_publish_payloads_notifications_or_death_text() {
    let good = [
        (
            input::DAMAGE_EVENT,
            damage(0, Some(3), None, Some([1., 2., 3.])),
        ),
        (input::HURT_ANIMATION, hurt(12.)),
        (input::END_COMBAT_EVENT, vec![123]),
        (input::DEATH_COMBAT_EVENT, kill(37)),
        (input::ENTER_COMBAT_EVENT, vec![]),
    ];
    for (id, bytes) in good {
        let mut invalid = (0..bytes.len())
            .map(|n| bytes[..n].to_vec())
            .collect::<Vec<_>>();
        let mut trailing = bytes;
        trailing.push(0);
        invalid.push(trailing);
        for bytes in invalid {
            let mut state = state();
            state.operations.local_player.entity_id = Some(37);
            assert!(state.receive(id, &bytes, 16).is_err(), "{id:#x} {bytes:?}");
            assert!(
                state
                    .effects
                    .after(0, state.sequence)
                    .unwrap()
                    .effects
                    .is_empty()
            );
            assert!(
                state
                    .events
                    .after(0, state.sequence)
                    .unwrap()
                    .events
                    .is_empty()
            );
            assert!(state.death_message.is_none());
            assert_eq!(state.failure.unwrap().kind(), crate::ErrorKind::Protocol);
        }
    }
    let mut invalid_bool = damage(0, None, None, None);
    invalid_bool[4] = 2;
    for (id, bytes) in [
        (input::DAMAGE_EVENT, invalid_bool),
        (input::DAMAGE_EVENT, damage(-1, None, None, None)),
        (input::DAMAGE_EVENT, damage(2, None, None, None)),
        (
            input::DAMAGE_EVENT,
            damage(0, None, None, Some([f64::NAN, 0., 0.])),
        ),
        (input::DAMAGE_EVENT, vec![0; MAX_EFFECT_PACKET_BYTES + 1]),
        (input::HURT_ANIMATION, hurt(f32::INFINITY)),
    ] {
        assert!(state().receive(id, &bytes, 16).is_err());
    }
    let mut missing = State {
        phase: Phase::Play,
        ..State::default()
    };
    assert!(
        missing
            .receive(input::DAMAGE_EVENT, &damage(0, None, None, None), 16)
            .is_err()
    );
}

#[test]
fn registry_reconfiguration_does_not_reinterpret_retained_damage() {
    let mut state = state();
    state
        .receive(input::DAMAGE_EVENT, &damage(0, None, None, None), 16)
        .unwrap();
    let first = state.effects.after(0, state.sequence).unwrap().effects[0].clone();
    state.registries.reset(state.sequence + 1);
    state.phase = Phase::Configuration;
    registry(&mut state, &["minecraft:generic", "example:custom"]);
    state.loading.generation += 1;
    state
        .receive(input::DAMAGE_EVENT, &damage(0, None, None, None), 16)
        .unwrap();
    let records = state.effects.after(0, state.sequence).unwrap().effects;
    assert_eq!(records[0], first);
    let PlayEffect::Damage(old) = &records[0].effect else {
        panic!()
    };
    let PlayEffect::Damage(new) = &records[1].effect else {
        panic!()
    };
    assert_eq!(old.source_type_name, "example:custom");
    assert_eq!(new.source_type_name, "minecraft:generic");
    assert_ne!(old.source_type.stamp(), new.source_type.stamp());
    assert_ne!(records[0].session, records[1].session);
    let registry = state
        .registries
        .capture(state.snapshot_session(), state.sequence);
    assert!(registry.resolve(&old.source_type).is_err());
    assert!(registry.resolve(&new.source_type).is_ok());
}

#[test]
fn captured_official_server_damage_registry_and_packets_match_live_observations() {
    let evidence: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../docs/evidence/damage-details-20261022.json"
    ))
    .unwrap();
    assert_eq!(evidence["status"], "PASS");
    assert_eq!(evidence["trace_complete"], true);
    let mut state = State {
        connection_id: 1,
        phase: Phase::Configuration,
        ..State::default()
    };
    let registry = &evidence["received_registry"]["damage_type"];
    let mut bytes = vec![];
    put_string(&mut bytes, "minecraft:damage_type");
    let entries = registry["value"].as_array().unwrap();
    put_varint(&mut bytes, entries.len() as i32);
    for entry in entries {
        put_string(&mut bytes, entry["name"].as_str().unwrap());
        bytes.push(1);
        bytes.extend(
            entry["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|b| b.as_u64().unwrap() as u8),
        );
    }
    state.sequence = registry["source"]["sequence"].as_u64().unwrap() - 1;
    state
        .receive(
            crate::versions::java_1_21_11::ids::configuration_clientbound::REGISTRY_DATA,
            &bytes,
            16,
        )
        .unwrap();
    state.registries.finish();
    state.phase = Phase::Play;
    state.loading.generation = evidence["observations"][0]["session"]["world_generation"]
        .as_u64()
        .unwrap();
    state.operations.local_player.entity_id = Some(1);
    let mut matched = 0;
    for packet in evidence["packets"].as_array().unwrap() {
        let sequence = packet["receive_sequence"].as_u64().unwrap();
        state.sequence = sequence - 1;
        let id = packet["packet_id"].as_i64().unwrap() as i32;
        let bytes = hex::decode(packet["payload_hex"].as_str().unwrap()).unwrap();
        state.receive(id, &bytes, 16).unwrap();
        let log = state.effects.after(sequence - 1, sequence).unwrap();
        assert_eq!(log.effects.len(), 1);
        for live in evidence["observations"].as_array().unwrap() {
            if live["receive_sequence"] == sequence {
                let actual: serde_json::Value =
                    serde_json::from_str(&serde_json::to_string(&log.effects[0]).unwrap()).unwrap();
                assert_eq!(&actual, live);
                matched += 1;
            }
        }
    }
    assert_eq!(matched, 6);
    assert_eq!(
        state
            .effects
            .after(0, state.sequence)
            .unwrap()
            .effects
            .len(),
        7
    );
    let death = state.death_message.unwrap();
    assert_eq!(death.source, ValueSource::Received { sequence: 213 });
}
