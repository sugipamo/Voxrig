use super::*;
use crate::client::{EventKind, effects::*, events::MAX_RETAINED_EVENTS};
use crate::protocol::put_varint;
use crate::versions::java_1_21_11::client::{Phase, State};
use serde_json::Value;

fn fixtures() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!(
        "../../../../../data/client_api/effect_packets-1.21.11.json"
    ))
    .unwrap()["packets"]
        .as_array()
        .unwrap()
        .clone()
}
fn packet(row: &Value) -> (i32, Vec<u8>) {
    (
        row["packet_id"].as_i64().unwrap() as i32,
        hex::decode(row["payload_hex"].as_str().unwrap()).unwrap(),
    )
}
fn state() -> State {
    State {
        phase: Phase::Play,
        connection_id: 42,
        ..State::default()
    }
}
fn expected_particle(row: &Value) -> ParticleData {
    use ParticleData as D;
    match row["particle_name"].as_str().unwrap() {
        "minecraft:block"
        | "minecraft:block_marker"
        | "minecraft:falling_dust"
        | "minecraft:dust_pillar"
        | "minecraft:block_crumble" => D::BlockState(123),
        "minecraft:entity_effect" | "minecraft:tinted_leaves" | "minecraft:flash" => {
            D::Color(0x81abcdef)
        }
        "minecraft:dust" => D::Dust {
            color: 0x123456,
            scale: 1.25,
        },
        "minecraft:dust_color_transition" => D::DustTransition {
            from_color: 0x123456,
            to_color: 0xabcdef,
            scale: 1.25,
        },
        "minecraft:dragon_breath" => D::Power(0.75),
        "minecraft:sculk_charge" => D::SculkCharge(0.75),
        "minecraft:effect" | "minecraft:instant_effect" => D::Spell {
            color: 0x81abcdef,
            power: 0.75,
        },
        "minecraft:shriek" => D::Shriek(12),
        "minecraft:trail" => D::Trail {
            target: [-4.5, 70.25, 8.125],
            color: 0x81abcdef,
            duration_ticks: 12,
        },
        "minecraft:vibration" => D::Vibration {
            destination: if row["name"] == "particle/vibration/0" {
                VibrationDestination::Block([-17, -60, 23])
            } else {
                VibrationDestination::Entity {
                    native_id: 37,
                    eye_height: 1.25,
                }
            },
            arrival_ticks: 12,
        },
        "minecraft:item" => {
            let bytes = hex::decode(if row["name"] == "particle/item/0" {
                "0000"
            } else {
                "01000307"
            })
            .unwrap();
            D::Item(ParticleItem {
                count: 2,
                native_id: 1,
                components: crate::versions::java_1_21_11::item_components::decode_patch(&bytes)
                    .unwrap(),
            })
        }
        _ => {
            assert_eq!(row["option_class"], "md");
            D::Simple
        }
    }
}

#[test]
fn official_codec_packets_cover_every_particle_layout_and_preserve_native_fields() {
    let fixtures = fixtures();
    assert_eq!(fixtures.len(), 135);
    let mut particle_ids = std::collections::BTreeSet::new();
    for row in fixtures {
        let (id, bytes) = packet(&row);
        let effect = decode(id, &bytes).unwrap().unwrap();
        let name = row["name"].as_str().unwrap();
        match effect {
            PlayEffect::Particles(p) => {
                particle_ids.insert(p.native_id);
                assert!(p.override_limiter);
                assert!(!p.always_show);
                assert_eq!(p.position, [-12.5, 64.25, 8.]);
                assert_eq!(p.offset, [0.25, 0.5, -0.125]);
                assert_eq!(p.max_speed, 0.75);
                assert_eq!(p.count, if name.ends_with("/0") { 0 } else { i32::MAX });
                assert_eq!(p.native_id, row["particle_id"].as_i64().unwrap() as i32);
                assert_eq!(p.data, expected_particle(&row), "{name}");
            }
            PlayEffect::Sound(p) => {
                assert_eq!(p.source, 7);
                assert_eq!((p.volume, p.pitch, p.seed), (0.75, 1.25, -29));
                assert_eq!(
                    p.origin,
                    if name.contains("/true/") {
                        SoundOrigin::Entity(37)
                    } else {
                        SoundOrigin::PositionEighths([-801, -512, 1604])
                    }
                );
                assert_eq!(
                    p.sound,
                    if name.ends_with("/0") {
                        SoundReference::RegistryId(0)
                    } else {
                        SoundReference::Inline {
                            name: "voxrig:probe".into(),
                            fixed_range: if name.ends_with("/2") {
                                Some(24.5)
                            } else {
                                None
                            },
                        }
                    }
                );
            }
            PlayEffect::SoundStopped {
                source,
                name: sound,
            } => {
                let flags = name[5..].parse::<u8>().unwrap();
                assert_eq!(source, (flags & 1 != 0).then_some(7));
                assert_eq!(sound, (flags & 2 != 0).then(|| "voxrig:probe".into()));
            }
            PlayEffect::LevelEvent {
                event_id,
                position,
                data,
                global,
            } => assert_eq!(
                (event_id, position, data, global),
                (2001, [-17, -60, 23], -123, true)
            ),
            PlayEffect::BlockDestruction {
                breaker_id,
                position,
                progress,
            } => {
                assert_eq!((breaker_id, position), (37, [-17, -60, 23]));
                assert_eq!(progress, name[6..].parse::<i32>().unwrap() as u8);
            }
            PlayEffect::ItemPickup {
                collected_id,
                collector_id,
                amount,
            } => assert_eq!((collected_id, collector_id, amount), (37, 99, 5)),
            _ => panic!("unexpected effect fixture"),
        }
    }
    assert_eq!(
        particle_ids.into_iter().collect::<Vec<_>>(),
        (0..115).collect::<Vec<_>>()
    );
}

#[test]
fn receive_publishes_notifications_and_payloads_at_the_same_sequence_without_state_inference() {
    let mut s = state();
    for (index, row) in fixtures().iter().enumerate() {
        let (id, bytes) = packet(row);
        assert!(s.receive(id, &bytes, 16).unwrap().is_empty());
        let sequence = index as u64 + 1;
        let log = s.effects.after(sequence - 1, s.sequence).unwrap();
        assert_eq!(log.receive_sequence, sequence);
        assert_eq!(log.effects.len(), 1);
        let observed = &log.effects[0];
        assert_eq!(observed.receive_sequence, sequence);
        assert_eq!(observed.session.connection_id, 42);
        assert_eq!(observed.session, s.snapshot_session());
        assert_eq!(observed.effect, decode(id, &bytes).unwrap().unwrap());
        let events = s.events.after(sequence - 1, s.sequence).unwrap();
        assert_eq!(events.events.len(), 1);
        assert_eq!(events.events[0].receive_sequence, sequence);
        assert_eq!(
            events.events[0].kind,
            super::super::event_kinds::EFFECT_KINDS
                .iter()
                .find(|(packet, _)| *packet == id)
                .unwrap()
                .1
        );
        // These packets must never fabricate inventory, block, or entity changes.
        assert!(!matches!(
            events.events[0].kind,
            EventKind::InventoryChanged
                | EventKind::EntityRemoved { .. }
                | EventKind::BlocksChanged { .. }
        ));
    }
    let tail = s.effects.after(s.sequence, s.sequence).unwrap();
    assert!(tail.effects.is_empty());
    assert_eq!(
        s.effects
            .after(s.sequence + 1, s.sequence)
            .unwrap_err()
            .kind(),
        crate::ErrorKind::InvalidInput
    );
}

#[test]
fn truncation_trailing_bytes_and_malformed_fields_never_publish_partial_observations() {
    for row in fixtures() {
        let (id, bytes) = packet(&row);
        for end in 0..bytes.len() {
            assert!(
                decode(id, &bytes[..end]).is_err(),
                "{} prefix {end}",
                row["name"]
            );
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(
            decode(id, &trailing).is_err(),
            "{} trailing data",
            row["name"]
        );
        for invalid in [bytes[..bytes.len() - 1].to_vec(), trailing] {
            let mut s = state();
            assert!(s.receive(id, &invalid, 16).is_err());
            assert!(s.effects.after(0, s.sequence).unwrap().effects.is_empty());
            assert!(s.events.after(0, s.sequence).unwrap().events.is_empty());
            assert_eq!(
                s.failure.as_ref().unwrap().kind(),
                crate::ErrorKind::Protocol
            );
        }
    }
    assert!(decode(input::STOP_SOUND, &[4]).is_err());
    assert!(decode(input::STOP_SOUND, &[1, 10]).is_err());
    assert!(decode(input::STOP_SOUND, &[1, 255, 255, 255, 255, 15]).is_err());
    let row = fixtures()
        .into_iter()
        .find(|row| row["name"] == "sound/false/0")
        .unwrap();
    let (_, mut bytes) = packet(&row);
    bytes[14..18].copy_from_slice(&f32::INFINITY.to_be_bytes());
    assert!(decode(input::SOUND_EFFECT, &bytes).is_err());
    let row = fixtures()
        .into_iter()
        .find(|row| row["name"] == "particle/dust/0")
        .unwrap();
    let (_, mut bytes) = packet(&row);
    bytes[2..10].copy_from_slice(&f64::NAN.to_be_bytes());
    assert!(decode(input::WORLD_PARTICLES, &bytes).is_err());
    let (_, mut bytes) = packet(&row);
    *bytes.last_mut().unwrap() = 0; // replace scale below, preserving header
    let end = bytes.len();
    bytes[end - 4..].copy_from_slice(&f32::NEG_INFINITY.to_be_bytes());
    assert!(decode(input::WORLD_PARTICLES, &bytes).is_err());
}

fn particle_packet(id: i32, options: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; 46];
    put_varint(&mut bytes, id);
    bytes.extend(options);
    bytes
}
#[test]
fn unknown_ids_are_lossless_bounded_and_never_guessed_as_simple() {
    for (id, options) in [(9999, vec![1, 2, 3]), (48, vec![7, 9, 8, 7])] {
        let bytes = particle_packet(id, &options);
        let Some(PlayEffect::Particles(p)) = decode(input::WORLD_PARTICLES, &bytes).unwrap() else {
            panic!()
        };
        assert_eq!(p.native_id, id);
        assert_eq!(p.data, ParticleData::Unknown { bytes: options });
    }
    let header = particle_packet(9999, &[]).len();
    let bytes = particle_packet(9999, &vec![7; MAX_EFFECT_PACKET_BYTES - header]);
    assert!(decode(input::WORLD_PARTICLES, &bytes).is_ok());
    let mut oversized = bytes;
    oversized.push(0);
    let mut s = state();
    assert!(s.receive(input::WORLD_PARTICLES, &oversized, 16).is_err());
    assert!(s.effects.after(0, s.sequence).unwrap().effects.is_empty());
    assert!(s.events.after(0, s.sequence).unwrap().events.is_empty());
    for options in [
        &[0][..],
        &[255, 255, 255, 255, 15][..],
        &[1, 1, 1, 0, 3][..],
    ] {
        assert!(decode(input::WORLD_PARTICLES, &particle_packet(47, options)).is_err());
    }
    assert!(decode(input::WORLD_PARTICLES, &particle_packet(-1, &[])).is_err());
    let mut unknown = vec![];
    put_varint(&mut unknown, i32::MAX);
    unknown.extend([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // position and progress
    assert!(decode(input::BLOCK_BREAK_ANIMATION, &unknown).is_err());
    assert!(decode(input::KEEP_ALIVE, &[]).unwrap().is_none());
}

#[test]
fn payload_history_has_independent_eviction_and_preserves_old_world_provenance() {
    let mut s = state();
    let row = fixtures()
        .into_iter()
        .find(|row| row["name"] == "pickup")
        .unwrap();
    let (id, bytes) = packet(&row);
    s.receive(id, &bytes, 16).unwrap();
    let first = s.effects.after(0, s.sequence).unwrap().effects[0].clone();
    s.loading.generation += 1;
    // Many unrelated notifications cannot evict payloads.
    for n in 0..MAX_RETAINED_EVENTS + 1 {
        s.events.record(2 + n as u64, EventKind::ChatReceived);
    }
    assert_eq!(s.effects.after(0, s.sequence).unwrap().effects[0], first);
    for _ in 0..MAX_RETAINED_EFFECTS {
        s.receive(id, &bytes, 16).unwrap();
    }
    assert_eq!(
        s.effects.after(0, s.sequence).unwrap_err().kind(),
        crate::ErrorKind::State
    );
    let log = s.effects.after(1, s.sequence).unwrap();
    assert_eq!(log.effects.len(), MAX_RETAINED_EFFECTS);
    assert_eq!(
        log.effects[0].session.world_generation,
        first.session.world_generation + 1
    );
    assert_eq!(first.session.connection_id, 42);
    assert_eq!(first.receive_sequence, 1);
    assert_eq!(
        log.effects.last().unwrap().receive_sequence,
        log.receive_sequence
    );
}

#[test]
fn captured_official_server_payloads_match_the_live_observations() {
    let evidence: Value = serde_json::from_str(include_str!(
        "../../../../../docs/evidence/effect-events-20261022.json"
    ))
    .unwrap();
    let mut s = state();
    let mut matched = 0;
    for row in evidence["packets"].as_array().unwrap() {
        let sequence = row["receive_sequence"].as_u64().unwrap();
        let (id, bytes) = packet(row);
        s.sequence = sequence - 1;
        s.receive(id, &bytes, 16).unwrap();
        let log = s.effects.after(sequence - 1, s.sequence).unwrap();
        assert_eq!(log.effects.len(), 1);
        let effect = &log.effects[0].effect;
        for observed in evidence["observations"].as_array().unwrap() {
            if observed["receive_sequence"] == sequence {
                // Match the live JSON wire projection. Value's f32 serializer
                // first widens to f64, unlike the JSON text serializer used by
                // the probe, so compare after the same text round trip.
                let actual: Value =
                    serde_json::from_str(&serde_json::to_string(effect).unwrap()).unwrap();
                assert_eq!(actual, observed["effect"]);
                matched += 1;
            }
        }
    }
    assert_eq!(matched, 6);
    let effects = s.effects.after(0, s.sequence).unwrap().effects;
    assert!(
        effects
            .iter()
            .any(|e| matches!(e.effect, PlayEffect::BlockDestruction { progress: 255, .. }))
    );
}
