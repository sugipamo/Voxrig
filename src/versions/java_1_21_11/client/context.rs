//! Actual context packets, decoded before committing any field.
use super::{Reader, State, ids};
use crate::client::{DefaultSpawnPosition, Experience, received};

pub(super) fn receive(state: &mut State, id: i32, payload: &[u8]) -> anyhow::Result<bool> {
    use ids::play_clientbound as p;
    let mut r = Reader::new(payload);
    match id {
        p::SET_COOLDOWN => {
            let group = r.string()?;
            anyhow::ensure!(
                group.len() <= 32_767,
                "cooldown group exceeds native resource-key limit"
            );
            crate::client::identifier::parts(&group)?;
            let ticks = r.varint()?;
            r.end()?;
            state.context.cooldown(
                crate::client::CooldownKey::Group(group),
                ticks,
                state.sequence,
            )?;
        }
        p::DIFFICULTY => {
            let value = crate::client::WorldDifficulty {
                id: r.u8()?,
                locked: r.u8()? != 0,
            };
            r.end()?;
            state.context.difficulty = Some(received(value, state.sequence));
        }
        p::GAME_STATE_CHANGE => {
            let reason = r.u8()?;
            let value = r.f32()?;
            r.end()?;
            state.context.weather(reason, value, state.sequence);
            // The existing operation handler still owns mode/loading events.
            return Ok(false);
        }
        p::EXPERIENCE => {
            let value = Experience {
                progress: r.f32()?,
                level: r.varint()?,
                total: r.varint()?,
            };
            r.end()?;
            state.context.experience = Some(received(value, state.sequence));
        }
        p::SPAWN_POSITION => {
            let dimension = r.string()?;
            anyhow::ensure!(
                dimension.len() <= 32_767,
                "spawn dimension exceeds native resource-key limit"
            );
            crate::client::identifier::parts(&dimension)?;
            let position =
                super::super::wire::unpack_position(u64::from_be_bytes(r.take(8)?.try_into()?));
            let yaw = r.f32()?;
            let pitch = r.f32()?;
            r.end()?;
            state.context.default_spawn = Some(received(
                DefaultSpawnPosition {
                    position,
                    dimension: Some(dimension),
                    yaw: Some(yaw),
                    pitch: Some(pitch),
                },
                state.sequence,
            ));
        }
        p::UPDATE_VIEW_POSITION => {
            let value = [r.varint()?, r.varint()?];
            r.end()?;
            state.context.world_view.center = Some(received(value, state.sequence));
        }
        p::UPDATE_VIEW_DISTANCE | p::SIMULATION_DISTANCE => {
            let value = r.varint()?;
            anyhow::ensure!(value >= 0, "negative world-view distance");
            r.end()?;
            let target = if id == p::SIMULATION_DISTANCE {
                &mut state.context.world_view.simulation_distance
            } else {
                &mut state.context.world_view.distance
            };
            *target = Some(received(value, state.sequence));
        }
        _ => return Ok(false),
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{SessionStamp, ValueSource};
    #[test]
    fn world_entry_is_atomic_including_login_and_respawn_suffixes() {
        let mut state = State {
            phase: super::super::Phase::Play,
            ..Default::default()
        };
        state
            .dimensions
            .push(super::super::Dimension::new(-64, 384).unwrap());
        let mut spawn = vec![0];
        crate::protocol::put_string(&mut spawn, "minecraft:overworld");
        spawn.extend(123i64.to_be_bytes());
        spawn.extend([1, 255, 0, 1, 1]);
        crate::protocol::put_string(&mut spawn, "minecraft:the_nether");
        spawn.extend(0u64.to_be_bytes());
        spawn.extend([7, 63]);
        let mut login = 77i32.to_be_bytes().to_vec();
        login.extend([1, 0, 5, 3, 2, 1, 0, 1]);
        login.extend(&spawn);
        login.push(1);
        super::super::apply_play(&mut state, ids::play_clientbound::LOGIN, &login, 64).unwrap();
        let saved = state.context.world_entry.clone();
        assert_eq!(saved.as_ref().unwrap().value.previous_game_mode, -1);
        assert_eq!(
            saved
                .as_ref()
                .unwrap()
                .value
                .last_death
                .as_ref()
                .unwrap()
                .dimension,
            "minecraft:the_nether"
        );
        assert_eq!(
            state
                .context
                .login_conditions
                .as_ref()
                .unwrap()
                .value
                .max_players,
            5
        );
        let generation = state.loading.generation;
        let conditions = state.context.login_conditions.clone();
        for id in [ids::play_clientbound::LOGIN, ids::play_clientbound::RESPAWN] {
            let mut whole = if id == ids::play_clientbound::LOGIN {
                login.clone()
            } else {
                let mut p = spawn.clone();
                p.push(3);
                p
            };
            for end in 0..whole.len() {
                assert!(super::super::apply_play(&mut state, id, &whole[..end], 64).is_err());
                assert_eq!(state.context.world_entry, saved);
                assert_eq!(state.context.login_conditions, conditions);
                assert_eq!(state.loading.generation, generation);
                assert_eq!(
                    super::super::operations::common_player_in_state(&state, 0, false)
                        .unwrap()
                        .game_mode,
                    Some(crate::client::GameMode::Creative)
                );
            }
            whole.push(0);
            assert!(super::super::apply_play(&mut state, id, &whole, 64).is_err());
            assert_eq!(state.context.world_entry, saved);
        }
        state.sequence = 10;
        spawn.push(3);
        super::super::apply_play(&mut state, ids::play_clientbound::RESPAWN, &spawn, 64).unwrap();
        assert_eq!(
            state.context.world_entry.as_ref().unwrap().source,
            ValueSource::Received { sequence: 10 }
        );
        assert_eq!(
            state
                .context
                .world_entry
                .as_ref()
                .unwrap()
                .value
                .respawn_keep_data,
            Some(3)
        );
        assert!(state.context.login_conditions.is_none());
        assert_eq!(saved.unwrap().value.respawn_keep_data, None);
    }
    #[test]
    fn cooldown_group_notifications_are_bounded_atomic_and_keep_zero() {
        let mut state = State {
            phase: super::super::Phase::Play,
            ..Default::default()
        };
        let mut packet = Vec::new();
        crate::protocol::put_string(&mut packet, "minecraft:ender_pearl");
        packet.push(20);
        state.sequence = 3;
        receive(&mut state, ids::play_clientbound::SET_COOLDOWN, &packet).unwrap();
        let saved = state.context.item_cooldowns.clone();
        for n in 0..packet.len() {
            assert!(
                receive(
                    &mut state,
                    ids::play_clientbound::SET_COOLDOWN,
                    &packet[..n]
                )
                .is_err()
            );
            assert_eq!(state.context.item_cooldowns, saved);
        }
        let mut trailing = packet.clone();
        trailing.push(0);
        assert!(receive(&mut state, ids::play_clientbound::SET_COOLDOWN, &trailing).is_err());
        state.sequence = 4;
        *packet.last_mut().unwrap() = 0;
        receive(&mut state, ids::play_clientbound::SET_COOLDOWN, &packet).unwrap();
        assert_eq!(
            state.context.item_cooldowns.values().next().unwrap().value,
            0
        );
        assert_eq!(saved.values().next().unwrap().value, 20);
        state
            .receive(ids::play_clientbound::START_CONFIGURATION, &[], 64)
            .unwrap();
        assert!(state.context.item_cooldowns.is_empty());
        let mut ledger = crate::client::context::ContextLedger::default();
        for id in 0..1024 {
            ledger
                .cooldown(crate::client::CooldownKey::LegacyItem(id), 0, 1)
                .unwrap();
        }
        assert!(
            ledger
                .cooldown(crate::client::CooldownKey::LegacyItem(1024), 1, 2)
                .is_err()
        );
        ledger
            .cooldown(crate::client::CooldownKey::LegacyItem(0), 3, 2)
            .unwrap();
        assert!(
            ledger
                .cooldown(crate::client::CooldownKey::LegacyItem(0), -1, 3)
                .is_err()
        );
        assert_eq!(ledger.item_cooldowns.len(), 1024);
        assert_eq!(
            ledger
                .item_cooldowns
                .get(&crate::client::CooldownKey::LegacyItem(0))
                .unwrap()
                .value,
            3
        );
    }
    #[test]
    fn abilities_and_difficulty_modern_are_atomic_received_world_context() {
        let mut state = State {
            phase: super::super::Phase::Play,
            ..Default::default()
        };
        let mut packet = vec![15];
        packet.extend(0.075f32.to_be_bytes());
        packet.extend(0.125f32.to_be_bytes());
        state
            .receive(ids::play_clientbound::ABILITIES, &packet, 64)
            .unwrap();
        state
            .receive(ids::play_clientbound::DIFFICULTY, &[0, 0], 64)
            .unwrap();
        let abilities = state.context.abilities.clone();
        let difficulty = state.context.difficulty.clone();
        assert_eq!(abilities.as_ref().unwrap().value.flags, 15);
        assert_eq!(abilities.as_ref().unwrap().value.flying_speed, 0.075);
        assert_eq!(abilities.as_ref().unwrap().value.walking_speed, 0.125);
        assert_eq!(
            difficulty.as_ref().unwrap().value,
            crate::client::WorldDifficulty {
                id: 0,
                locked: false
            }
        );
        let native = state.operations.abilities_receipt();
        let mut invalid: Vec<Vec<u8>> = (0..packet.len()).map(|n| packet[..n].to_vec()).collect();
        let mut trailing = packet.clone();
        trailing.push(0);
        invalid.push(trailing);
        let mut unknown = packet.clone();
        unknown[0] = 16;
        invalid.push(unknown);
        for offset in [1, 5] {
            for value in [f32::NAN, f32::INFINITY] {
                let mut bad = packet.clone();
                bad[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
                invalid.push(bad);
            }
        }
        for bad in invalid {
            assert!(
                super::super::operations::receive(
                    &mut state,
                    ids::play_clientbound::ABILITIES,
                    &bad
                )
                .is_err()
            );
            assert_eq!(state.operations.abilities_receipt(), native);
            assert_eq!(state.context.abilities, abilities);
        }
        for bad in [vec![], vec![1], vec![1, 0, 0]] {
            assert!(receive(&mut state, ids::play_clientbound::DIFFICULTY, &bad).is_err());
            assert_eq!(state.context.difficulty, difficulty);
        }
        state
            .receive(ids::play_clientbound::DIFFICULTY, &[255, 2], 64)
            .unwrap();
        assert_eq!(
            state.context.difficulty.as_ref().unwrap().value,
            crate::client::WorldDifficulty {
                id: 255,
                locked: true
            }
        );
        assert_eq!(state.context.abilities, abilities);
        state
            .receive(ids::play_clientbound::ABILITIES, &[0; 9], 64)
            .unwrap();
        assert_eq!(state.context.abilities.as_ref().unwrap().value.flags, 0);
        assert_eq!(
            state.context.abilities.as_ref().unwrap().value.flying_speed,
            0.
        );
        state
            .receive(ids::play_clientbound::START_CONFIGURATION, &[], 64)
            .unwrap();
        assert!(state.context.abilities.is_none() && state.context.difficulty.is_none());
        assert!(state.operations.abilities_receipt().is_none());
        assert_eq!(abilities.unwrap().value.flags, 15);
        // The transport receiver is terminal on malformed input; parser
        // atomicity above does not authorize replaying a failed stream.
        let mut failed = State {
            phase: super::super::Phase::Play,
            ..Default::default()
        };
        assert!(
            failed
                .receive(ids::play_clientbound::ABILITIES, &[15], 64)
                .is_err()
        );
        assert!(
            failed
                .receive(ids::play_clientbound::ABILITIES, &packet, 64)
                .is_err()
        );
        assert!(failed.context.abilities.is_none());
    }
    #[test]
    fn player_context_modern_packets_are_atomic_and_reset_at_configuration_boundary() {
        let mut state = State {
            phase: super::super::Phase::Play,
            ..Default::default()
        };
        let stamp = SessionStamp {
            version: crate::MinecraftVersion::Java1_21_11,
            connection_id: 8,
            world_generation: 0,
        };
        state.sequence = 1;
        let mut rain = vec![7];
        rain.extend(0.25f32.to_be_bytes());
        receive(&mut state, ids::play_clientbound::GAME_STATE_CHANGE, &rain).unwrap();
        assert!(state.context.weather.raining.is_none());
        state.sequence = 2;
        let mut xp = 0.25f32.to_be_bytes().to_vec();
        xp.extend([7, 20]);
        receive(&mut state, ids::play_clientbound::EXPERIENCE, &xp).unwrap();
        let before = state.context.capture(stamp, 2);
        for end in 0..xp.len() {
            state.sequence += 1;
            assert!(receive(&mut state, ids::play_clientbound::EXPERIENCE, &xp[..end]).is_err());
            assert_eq!(state.context.experience, before.experience);
        }
        let mut spawn = Vec::new();
        crate::protocol::put_string(&mut spawn, "minecraft:overworld");
        spawn.extend(0u64.to_be_bytes());
        spawn.extend(37f32.to_be_bytes());
        spawn.extend((-12f32).to_be_bytes());
        receive(&mut state, ids::play_clientbound::SPAWN_POSITION, &spawn).unwrap();
        let saved = state.context.default_spawn.clone();
        for end in 0..spawn.len() {
            assert!(
                receive(
                    &mut state,
                    ids::play_clientbound::SPAWN_POSITION,
                    &spawn[..end]
                )
                .is_err()
            );
            assert_eq!(state.context.default_spawn, saved);
        }
        let mut invalid = xp.clone();
        invalid[0..4].copy_from_slice(&f32::NAN.to_be_bytes());
        assert!(receive(&mut state, ids::play_clientbound::EXPERIENCE, &invalid).is_err());
        assert_eq!(state.context.experience, before.experience);
        assert_eq!(
            before.experience.as_ref().unwrap().source,
            ValueSource::Received { sequence: 2 }
        );
        assert_eq!(
            before.weather.rain_level.as_ref().unwrap().source,
            ValueSource::Received { sequence: 1 }
        );
        state
            .receive(ids::play_clientbound::START_CONFIGURATION, &[], 64)
            .unwrap();
        let next = state.context.capture(
            SessionStamp {
                world_generation: state.loading.generation,
                ..stamp
            },
            state.sequence,
        );
        assert_ne!(
            next.session.world_generation,
            before.session.world_generation
        );
        assert!(
            next.experience.is_none()
                && next.weather.rain_level.is_none()
                && next.default_spawn.is_none()
        );
        assert_eq!(before.experience.as_ref().unwrap().value.level, 7);
    }
}
