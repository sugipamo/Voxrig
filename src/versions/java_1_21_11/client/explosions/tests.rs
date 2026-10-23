use super::*;
use crate::client::{
    EventKind,
    effects::{ParticleData, SoundReference},
};
use crate::protocol::put_varint;
use crate::versions::java_1_21_11::{
    client::{Phase, State},
    ids::play_clientbound as input,
};
use serde_json::Value;

fn fixtures() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!(
        "../../../../../data/client_api/explosion_packets-1.21.11.json"
    ))
    .unwrap()["packets"]
        .as_array()
        .unwrap()
        .clone()
}
fn bytes(row: &Value) -> Vec<u8> {
    hex::decode(row["payload_hex"].as_str().unwrap()).unwrap()
}
fn state() -> State {
    State {
        phase: Phase::Play,
        connection_id: 42,
        ..State::default()
    }
}

#[test]
fn official_codec_preserves_optional_impulse_and_presentation_separately() {
    let rows = fixtures();
    assert_eq!(rows.len(), 9);
    for (index, row) in rows.iter().enumerate() {
        let impulse = index / 3;
        let presentation = index % 3;
        let p = decode(&bytes(row)).unwrap();
        assert_eq!(p.center, [-12.5, 64.25, 8.125]);
        assert_eq!((p.radius, p.block_count), (4.5, -123));
        assert_eq!(
            p.player_knockback,
            match impulse {
                0 => None,
                1 => Some([0.; 3]),
                _ => Some([-0.25, 0.5, 1.25]),
            }
        );
        let ExplosionPresentation::Decoded {
            particle,
            sound,
            block_particles,
        } = p.presentation
        else {
            panic!()
        };
        assert_eq!(
            particle,
            if presentation == 2 {
                ExplosionParticle {
                    native_id: 14,
                    data: ParticleData::Dust {
                        color: 0x123456,
                        scale: 1.25,
                    },
                }
            } else {
                ExplosionParticle {
                    native_id: 22,
                    data: ParticleData::Simple,
                }
            }
        );
        assert_eq!(
            sound,
            if presentation == 0 {
                SoundReference::RegistryId(0)
            } else {
                SoundReference::Inline {
                    name: "voxrig:explosion".into(),
                    fixed_range: (presentation == 2).then_some(24.5),
                }
            }
        );
        assert_eq!(
            block_particles,
            if presentation == 0 {
                vec![]
            } else {
                vec![
                    ExplosionBlockParticle {
                        particle: ExplosionParticle {
                            native_id: 1,
                            data: ParticleData::BlockState(123),
                        },
                        scaling: 0.75,
                        speed: 1.25,
                        weight: 0,
                    },
                    ExplosionBlockParticle {
                        particle: ExplosionParticle {
                            native_id: 23,
                            data: ParticleData::Simple,
                        },
                        scaling: -0.5,
                        speed: 0.,
                        weight: 3,
                    },
                ]
            }
        );
    }
}
fn rejects(payload: &[u8]) {
    let mut s = state();
    assert!(s.receive(input::EXPLOSION, payload, 16).is_err());
    assert_eq!(
        s.failure.as_ref().unwrap().kind(),
        crate::ErrorKind::Protocol
    );
    assert!(
        s.explosions
            .after(0, s.sequence)
            .unwrap()
            .explosions
            .is_empty()
    );
    assert!(s.events.after(0, s.sequence).unwrap().events.is_empty());
    assert!(s.operations.local_player.motion_interruption.is_none());
}
#[test]
fn malformed_packets_never_publish_partial_observations() {
    for row in fixtures() {
        let packet = bytes(&row);
        for end in 0..packet.len() {
            rejects(&packet[..end]);
        }
        let mut trailing = packet;
        trailing.push(0);
        rejects(&trailing);
    }
    let original = bytes(&fixtures()[8]);
    // Center, radius, impulse, dust scale, inline sound range, list scaling/speed.
    for (start, double) in [
        (0, true),
        (8, true),
        (16, true),
        (24, false),
        (33, true),
        (41, true),
        (49, true),
        (62, false),
        (85, false),
        (92, false),
        (96, false),
        (102, false),
        (106, false),
    ] {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut packet = original.clone();
            if double {
                packet[start..start + 8].copy_from_slice(&value.to_be_bytes());
            } else {
                packet[start..start + 4].copy_from_slice(&(value as f32).to_be_bytes());
            }
            rejects(&packet);
        }
    }
    let mut packet = original.clone();
    packet[32] = 2;
    rejects(&packet);
    let mut packet = original;
    packet[84] = 2;
    rejects(&packet);
    rejects(&vec![0; MAX_EXPLOSION_PACKET_BYTES + 1]);
    // Known simple particle and sound followed by an excessive or negative list count.
    for count in [-1, i32::MAX] {
        let mut packet = bytes(&fixtures()[0]);
        packet.pop();
        put_varint(&mut packet, count);
        rejects(&packet);
    }
    // Negative weight and overflowing sum are not valid WeightedList values.
    for weights in [[-1, 1], [i32::MAX, 1]] {
        let mut packet = bytes(&fixtures()[0]);
        packet.pop();
        packet.push(2);
        for weight in weights {
            packet.push(23);
            packet.extend([0; 8]);
            put_varint(&mut packet, weight);
        }
        rejects(&packet);
    }
}
#[test]
fn unknown_embedded_layout_preserves_entire_context_without_inferred_absence() {
    let header = bytes(&fixtures()[6])[..57].to_vec();
    for tail in [
        vec![0x8f, 0x4e, 1, 2, 3],
        vec![48, 7, 8, 9],
        vec![22, 1, 1, 0x8f, 0x4e, 9],
    ] {
        let mut packet = header.clone();
        packet.extend(&tail);
        let p = decode(&packet).unwrap();
        assert_eq!(p.player_knockback, Some([-0.25, 0.5, 1.25]));
        assert_eq!(
            p.presentation,
            ExplosionPresentation::Undecoded { bytes: tail }
        );
    }
    let mut packet = header;
    packet.extend([0x8f, 0x4e]);
    packet.resize(MAX_EXPLOSION_PACKET_BYTES, 7);
    assert!(decode(&packet).is_ok());
    packet.push(0);
    rejects(&packet);
}
#[test]
fn reception_retains_order_old_world_and_gaps_without_inventing_motion() {
    let mut s = state();
    let packets = fixtures();
    for row in &packets {
        s.receive(input::EXPLOSION, &bytes(row), 16).unwrap();
        let log = s.explosions.after(s.sequence - 1, s.sequence).unwrap();
        assert_eq!(log.explosions.len(), 1);
        let record = &log.explosions[0];
        assert_eq!(record.session, s.snapshot_session());
        assert_eq!(record.receive_sequence, s.sequence);
        assert_eq!(record.explosion, decode(&bytes(row)).unwrap());
        assert_eq!(
            s.operations
                .local_player
                .motion_interruption
                .as_ref()
                .unwrap()
                .receive_sequence,
            s.sequence
        );
        assert_eq!(
            s.events
                .after(0, s.sequence)
                .unwrap()
                .events
                .last()
                .unwrap()
                .kind,
            EventKind::ExplosionReceived
        );
        assert!(s.position.is_none());
        assert!(s.effects.after(0, s.sequence).unwrap().effects.is_empty());
    }
    let first = s.explosions.after(0, s.sequence).unwrap().explosions[0].clone();
    s.receive(input::START_CONFIGURATION, &[], 16).unwrap();
    assert_ne!(first.session, s.snapshot_session());
    assert_eq!(
        s.explosions.after(0, s.sequence).unwrap().explosions[0],
        first
    );
    assert!(s.operations.local_player.motion_interruption.is_none());
    // Simulate return to play after reset, keeping the new world generation.
    s.phase = Phase::Play;
    s.receive(input::EXPLOSION, &bytes(&packets[0]), 16)
        .unwrap();
    assert_eq!(
        s.explosions.after(9, s.sequence).unwrap().explosions[0].session,
        s.snapshot_session()
    );
    for _ in 0..MAX_RETAINED_EXPLOSIONS {
        s.receive(input::EXPLOSION, &bytes(&packets[0]), 16)
            .unwrap();
    }
    assert_eq!(
        s.explosions.after(0, s.sequence).unwrap_err().kind(),
        crate::ErrorKind::State
    );
    let log = s
        .explosions
        .after(s.sequence - MAX_RETAINED_EXPLOSIONS as u64, s.sequence)
        .unwrap();
    assert_eq!(log.explosions.len(), MAX_RETAINED_EXPLOSIONS);
    assert!(
        log.explosions
            .windows(2)
            .all(|p| p[0].receive_sequence < p[1].receive_sequence)
    );
    assert!(
        s.explosions
            .after(s.sequence, s.sequence)
            .unwrap()
            .explosions
            .is_empty()
    );
    assert_eq!(
        s.explosions
            .after(s.sequence + 1, s.sequence)
            .unwrap_err()
            .kind(),
        crate::ErrorKind::InvalidInput
    );
}

#[test]
fn captured_tnt_creeper_and_wind_packets_match_live_receipts() {
    let evidence: Value = serde_json::from_str(include_str!(
        "../../../../../docs/evidence/explosion-observation-20261023.json"
    ))
    .unwrap();
    let mut s = state();
    let packets = evidence["packets"].as_array().unwrap();
    let observations = evidence["observations"].as_array().unwrap();
    assert_eq!(packets.len(), 6);
    for (index, (packet, observed)) in packets.iter().zip(observations).enumerate() {
        let sequence = packet["receive_sequence"].as_u64().unwrap();
        s.sequence = sequence - 1;
        s.receive(input::EXPLOSION, &bytes(packet), 16).unwrap();
        let record = &s
            .explosions
            .after(sequence - 1, s.sequence)
            .unwrap()
            .explosions[0];
        let actual: Value =
            serde_json::from_str(&serde_json::to_string(&record.explosion).unwrap()).unwrap();
        assert_eq!(actual, observed["explosion"]);
        assert_eq!(record.receive_sequence, observed["receive_sequence"]);
        assert_eq!(record.explosion.player_knockback.is_some(), index % 2 == 1);
        assert_eq!(
            s.events
                .after(0, s.sequence)
                .unwrap()
                .events
                .last()
                .unwrap()
                .receive_sequence,
            sequence
        );
    }
}

#[test]
fn respawn_keeps_original_receipts_and_new_explosions_use_the_new_world() {
    let mut s = state();
    s.dimensions
        .push(crate::versions::java_1_21_11::world::Dimension::new(-64, 384).unwrap());
    let packet = bytes(&fixtures()[0]);
    s.receive(input::EXPLOSION, &packet, 16).unwrap();
    let before = s.explosions.after(0, s.sequence).unwrap();
    let first = before.explosions[0].clone();
    // A real respawn packet, including CommonPlayerSpawnInfo and keep-data flags.
    let mut respawn = vec![0];
    crate::protocol::put_string(&mut respawn, "minecraft:overworld");
    respawn.extend(0i64.to_be_bytes());
    respawn.extend([0, 255, 0, 0, 0, 0, 63, 3]);
    s.receive(input::RESPAWN, &respawn, 16).unwrap();
    let new_session = s.snapshot_session();
    assert_ne!(new_session.world_generation, first.session.world_generation);
    assert_eq!(new_session.connection_id, first.session.connection_id);
    assert!(s.operations.local_player.motion_interruption.is_none());
    s.receive(input::EXPLOSION, &packet, 16).unwrap();
    let all = s.explosions.after(0, s.sequence).unwrap();
    assert_eq!(all.explosions.len(), 2);
    assert_eq!(all.explosions[0], first);
    assert_eq!(before.explosions, vec![first]);
    let next = s
        .explosions
        .after(before.receive_sequence, s.sequence)
        .unwrap();
    assert_eq!(next.explosions.len(), 1);
    assert_eq!(next.explosions[0].session, new_session);
    assert_eq!(next.explosions[0].receive_sequence, s.sequence);
    let events = s.events.after(0, s.sequence).unwrap().events;
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![
            EventKind::ExplosionReceived,
            EventKind::WorldChanged,
            EventKind::ContextChanged,
            EventKind::ExplosionReceived
        ]
    );
    assert_eq!(
        events[3].receive_sequence,
        next.explosions[0].receive_sequence
    );
}

#[test]
fn undecoded_presentation_is_observed_and_still_interrupts_motion() {
    let tail = vec![22, 1, 1, 0x8f, 0x4e, 9]; // Unknown weighted particle options.
    let mut packet = bytes(&fixtures()[0])[..33].to_vec();
    packet.extend(&tail);
    let mut s = state();
    s.receive(input::EXPLOSION, &packet, 16).unwrap();
    let log = s.explosions.after(0, s.sequence).unwrap();
    let [record] = log.explosions.as_slice() else {
        panic!()
    };
    assert_eq!(record.explosion.player_knockback, None);
    assert_eq!(
        record.explosion.presentation,
        ExplosionPresentation::Undecoded { bytes: tail }
    );
    assert_eq!(record.session, s.snapshot_session());
    assert_eq!(record.receive_sequence, s.sequence);
    assert_eq!(
        s.operations
            .local_player
            .motion_interruption
            .as_ref()
            .unwrap()
            .receive_sequence,
        s.sequence
    );
    let events = s.events.after(0, s.sequence).unwrap().events;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, EventKind::ExplosionReceived);
    assert_eq!(events[0].receive_sequence, record.receive_sequence);
    assert!(s.effects.after(0, s.sequence).unwrap().effects.is_empty());
}
