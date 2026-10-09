//! Actual context packets, decoded before committing any field.
use super::{Reader, State, ids};
use crate::client::{DefaultSpawnPosition, Experience, received};

pub(super) fn receive(state: &mut State, id: i32, payload: &[u8]) -> anyhow::Result<bool> {
    use ids::play_clientbound as p;
    let mut r = Reader::new(payload);
    match id {
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
