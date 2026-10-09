//! The original state decoder, with generated protocol responses discarded.
use super::*;
use crate::client::recording::{PacketPhase, PacketTrace, ReplayedBlock, ReplayedObservation};

pub(crate) fn replay_packets(
    trace: &PacketTrace,
    region: Region,
    maximum_chunks: usize,
) -> Result<ReplayedObservation> {
    let mut state = State::default();
    for record in &trace.records {
        let phase = match state.phase {
            Phase::Configuration => PacketPhase::Configuration,
            Phase::Play => PacketPhase::Play,
        };
        if record.phase != phase {
            return Err(crate::client::recording::invalid(
                "recorded phase differs from decoder state",
            ));
        }
        if matches!(state.phase, Phase::Play) && record.packet_id == ids::play_clientbound::POSITION
        {
            let basis = record
                .local_player_basis
                .as_ref()
                .context("position replay needs the recorded local baseline")?;
            state.position = basis.position;
            state.rotation = basis.rotation;
            state.motion.position_basis = if basis.velocity.is_some() {
                motion::PositionBasis::Received
            } else {
                motion::PositionBasis::Unavailable
            };
            state.operations.local_player.velocity =
                basis.velocity.map(|value| operations::VelocitySample {
                    value,
                    receive_sequence: record.sequence.saturating_sub(1),
                });
        }
        // Native JOIN/RESPAWN/reconfiguration reset the reconstruction frame.
        // Compare against the decoder's current generation, not the previous
        // record's tick, so legitimate reset histories remain replayable.
        if record.client_tick < state.reconstruction.tick {
            return Err(crate::client::recording::invalid(
                "trace local frame rewinds without native reset",
            ));
        }
        state
            .reconstruction
            .advance(&state.world, record.client_tick);
        // No writer, Session, Bot, connection actor or response dispatch exists.
        let _responses = state
            .receive(record.packet_id, &record.payload, maximum_chunks)
            .with_context(|| {
                format!(
                    "replay failed at packet {} ({:?}, {})",
                    record.sequence, record.phase, record.packet_id
                )
            })?;
    }
    if trace.through_client_tick < state.reconstruction.tick {
        return Err(crate::client::recording::invalid(
            "trace stop frame predates current native generation",
        ));
    }
    state
        .reconstruction
        .advance(&state.world, trace.through_client_tick);
    let (_, dimension) = state
        .world
        .dimension
        .as_ref()
        .context("replay has no initial play JOIN")?;
    if region.min[1] < dimension.min_y || region.max[1] >= dimension.min_y + dimension.height {
        return Err(crate::client::recording::invalid(
            "replay region outside received dimension",
        ));
    }
    let mut blocks = Vec::with_capacity(region.volume()?);
    for y in region.min[1]..=region.max[1] {
        for z in region.min[2]..=region.max[2] {
            for x in region.min[0]..=region.max[0] {
                let position = [x, y, z];
                blocks.push(ReplayedBlock {
                    position,
                    state: state
                        .world
                        .block(position)
                        .map(super::super::native_state)
                        .transpose()?,
                });
            }
        }
    }
    let player = operations::common_player_in_state(&state, trace.connection_id, false)?;
    Ok(crate::client::recording::project(&player, blocks, vec![]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_receive_replay_uses_recorded_local_relative_basis_and_received_inventory() {
        let mut state = State {
            trace: Some(TraceCapture::new(0, 1_000_000).unwrap()),
            ..State::default()
        };
        // Initial JOIN resets this nonzero configuration frame to zero.
        state.reconstruction.advance(&state.world, 5);
        let mut registry = vec![];
        put_string(&mut registry, "minecraft:dimension_type");
        registry.push(1);
        put_string(&mut registry, "minecraft:overworld");
        registry.extend([1, 10]);
        for (key, value) in [("min_y", -64i32), ("height", 384)] {
            registry.push(3);
            registry.extend((key.len() as u16).to_be_bytes());
            registry.extend(key.as_bytes());
            registry.extend(value.to_be_bytes());
        }
        registry.push(0);
        state
            .receive(ids::configuration_clientbound::REGISTRY_DATA, &registry, 64)
            .unwrap();
        state
            .receive(
                ids::configuration_clientbound::FINISH_CONFIGURATION,
                &[],
                64,
            )
            .unwrap();
        let mut join = vec![0; 4];
        join.extend([0, 0, 1, 4, 4, 0, 1, 0, 0]);
        put_string(&mut join, "minecraft:overworld");
        join.extend([0; 8]);
        join.extend([0, 255, 0, 1, 0, 0, 63, 0]);
        state
            .receive(ids::play_clientbound::LOGIN, &join, 64)
            .unwrap();
        // Local movement changes the relative decoder's inputs. Replaying only
        // the previous received position would produce a different correction.
        state.position = Some([10.5, 65.0, 7.5]);
        state.rotation = [80.0, 10.0];
        let mut position = vec![0];
        for value in [1.0f64, 0.0, -1.0, 0.0, 0.0, 0.0] {
            position.extend(value.to_be_bytes());
        }
        position.extend(5.0f32.to_be_bytes());
        position.extend(2.0f32.to_be_bytes());
        position.extend(31i32.to_be_bytes());
        state
            .receive(ids::play_clientbound::POSITION, &position, 64)
            .unwrap();
        let mut inventory = vec![0, 0, 46];
        inventory.extend([0; 47]);
        state
            .receive(ids::play_clientbound::WINDOW_ITEMS, &inventory, 64)
            .unwrap();
        let trace = state.trace.take().unwrap().finish(
            MinecraftVersion::Java1_21_11,
            77,
            state.sequence,
            state.reconstruction.tick,
        );
        let loaded: PacketTrace =
            serde_json::from_slice(&serde_json::to_vec(&trace).unwrap()).unwrap();
        let region = Region {
            min: [0, 65, 0],
            max: [0, 65, 0],
        };
        let replay = loaded.replay(region, 64).unwrap();
        let player = operations::common_player_in_state(&state, 77, false).unwrap();
        let expected = crate::client::recording::project(
            &player,
            vec![ReplayedBlock {
                position: [0, 65, 0],
                state: None,
            }],
            vec![],
        );
        assert_eq!(replay, expected);
        assert_eq!(replay.received_pose.unwrap().position, [11.5, 65.0, 6.5]);
        let mut missing = loaded.clone();
        missing.records[3].local_player_basis = None;
        assert!(missing.replay(region, 64).is_err());
        let mut wrong_phase = loaded;
        wrong_phase.records[2].phase = PacketPhase::Configuration;
        assert!(wrong_phase.replay(region, 64).is_err());
    }
}
