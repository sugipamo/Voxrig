//! Capture shares the exact legacy applied receive boundary.
use super::*;
use crate::client::recording::{PacketTrace, TraceCapture, validate_limit};

impl Bot {
    pub(crate) async fn start_packet_trace(&self, maximum_bytes: usize) -> Result<()> {
        validate_limit(maximum_bytes)?;
        let _gate = self.coherent_state_gate.lock().await;
        if self.is_stopped() {
            return Err(crate::client::survival::mining::unavailable(
                "connection closed",
            ));
        }
        let mut active = self.packet_trace.lock().await;
        if active.is_some() {
            return Err(crate::client::survival::mining::unavailable(
                "trace already active",
            ));
        }
        *active = Some(TraceCapture::new(
            self.protocol_packet_sequence.load(Ordering::Acquire),
            maximum_bytes,
        )?);
        Ok(())
    }
    pub(crate) async fn stop_packet_trace(&self) -> Result<PacketTrace> {
        let _gate = self.coherent_state_gate.lock().await;
        let trace = self
            .packet_trace
            .lock()
            .await
            .take()
            .context("no active packet trace")?;
        Ok(trace.finish(
            crate::MinecraftVersion::Java1_16_1,
            self.connection_id(),
            self.protocol_packet_sequence.load(Ordering::Acquire),
            self.connected_at.elapsed().as_millis() as u64 / 50,
        ))
    }
}

/// The same pure native correction parser serves live receive and replay.
pub(super) fn decode_position(
    payload: &[u8],
    before: [f64; 3],
    rotation: [f32; 2],
) -> Result<([f64; 3], [f32; 2], i32)> {
    let mut c = Cursor::new(payload);
    let values = [
        c.read_f64::<BigEndian>()?,
        c.read_f64::<BigEndian>()?,
        c.read_f64::<BigEndian>()?,
    ];
    let angles = [c.read_f32::<BigEndian>()?, c.read_f32::<BigEndian>()?];
    let flags = c.read_u8()?;
    let mut rest = &payload[c.position() as usize..];
    let teleport = get_varint(&mut rest)?;
    let position = std::array::from_fn(|i| {
        if flags & (1 << i) != 0 {
            before[i] + values[i]
        } else {
            values[i]
        }
    });
    let rotation = std::array::from_fn(|i| {
        if flags & (1 << (i + 3)) != 0 {
            rotation[i] + angles[i]
        } else {
            angles[i]
        }
    });
    validate_position(position[0], position[1], position[2])?;
    if rotation.iter().any(|v| !v.is_finite()) {
        return Err(crate::client::recording::invalid(
            "server position contains a non-finite rotation",
        ));
    }
    Ok((position, rotation, teleport))
}

pub(crate) fn replay_packets(
    trace: &PacketTrace,
    region: crate::Region,
    maximum_chunks: usize,
) -> Result<crate::client::recording::ReplayedObservation> {
    use crate::client::{
        self as api,
        recording::{PacketPhase, ReplayedBlock},
    };
    let mut world = World::default();
    let mut receipts = api::LegacyReceipts::default();
    let mut dimension = None;
    let mut game_mode = None;
    let mut joined = false;
    let mut unhandled = std::collections::BTreeSet::new();
    for record in &trace.records {
        if record.phase != PacketPhase::Play {
            return Err(api::recording::invalid(
                "legacy replay requires play packets",
            ));
        }
        let p = &record.payload;
        let sequence = record.sequence;
        match record.packet_id {
            0x25 => {
                let join = parse_join(p)?;
                joined = true;
                dimension = Some(api::Dimension {
                    name: join.dimension,
                    min_y: 0,
                    height: 256,
                });
                game_mode = Some(api::GameMode::decode(join.game_mode & 7)?);
                receipts.generation = sequence;
                receipts.pose = None;
                receipts.inventory.window_id = None;
                receipts.inventory.cursor = None;
                receipts.container = None;
                receipts.player_starts.clear();
                receipts
                    .registries
                    .legacy_join(join.registry_codec, sequence)?;
            }
            0x3a => {
                let respawn = parse_respawn(p)?;
                dimension = Some(api::Dimension {
                    name: respawn.dimension,
                    min_y: 0,
                    height: 256,
                });
                game_mode = Some(api::GameMode::decode(respawn.game_mode & 7)?);
                receipts.generation = sequence;
                receipts.pose = None;
                receipts.health = None;
                receipts.may_fly = None;
                receipts.inventory.window_id = None;
                receipts.inventory.cursor = None;
                receipts.container = None;
                receipts.player_starts.clear();
                if !respawn.copy_metadata {
                    receipts.inventory = Default::default();
                    receipts.selected_hotbar = None;
                }
                world.clear();
            }
            0x35 => {
                let basis = record
                    .local_player_basis
                    .as_ref()
                    .context("position replay needs the recorded local baseline")?;
                let before = basis
                    .position
                    .context("legacy position replay needs local feet")?;
                let (position, rotation, _teleport) = decode_position(p, before, basis.rotation)?;
                receipts.pose = Some(api::ReceivedPose {
                    position,
                    rotation,
                    receive_sequence: sequence,
                });
            }
            0x0b => {
                world.apply_block_change(p)?;
            }
            0x0f => {
                world.apply_multi_block_change_with_changes(p)?;
            }
            0x07 => {
                let ack = parse_digging_ack(p)?;
                world.apply_acknowledged_block_state(ack.position, ack.block_state_id);
            }
            0x21 => {
                world.apply_chunk(p, maximum_chunks)?;
            }
            0x1d => {
                world.unload_chunk(p)?;
            }
            0x1c => {
                let explosion = parse_explosion(p)?;
                world.apply_explosion_blocks(&explosion.affected_blocks);
            }
            0x14 => {
                let (window, slots) = parse_window_items(p)?;
                receipts.window_items(window, &slots, sequence)?;
            }
            0x16 => {
                let mut update = parse_set_slot(p)?;
                update.packet_sequence = sequence;
                receipts.slot(&update)?;
            }
            0x13 => {
                let window = *p.first().context("missing close window")? as i8;
                if receipts
                    .container
                    .as_ref()
                    .is_some_and(|s| s.window == i32::from(window))
                {
                    receipts.container = None;
                    receipts.inventory.window_id = Some(0);
                }
            }
            0x2e => {
                let mut rest = p.as_slice();
                let window = get_varint(&mut rest)?;
                if !(1..=127).contains(&window) {
                    return Err(api::recording::invalid(format!(
                        "invalid open window ID {window}"
                    )));
                }
                let kind = get_varint(&mut rest)?;
                let title = get_string(&mut rest)?;
                receipts.inventory.window_id = Some(window);
                receipts.inventory.cursor = None;
                receipts.player_starts.remove(&(window as i8));
                receipts.container = Some(api::container::ScreenReceipts::open(
                    crate::MinecraftVersion::Java1_16_1,
                    window,
                    Some(kind),
                    api::container::ScreenTitle::LegacyJson { json: title },
                    sequence,
                ));
            }
            0x49 => {
                let h = parse_vitals(p)?;
                receipts.health = Some(api::received(
                    api::Health {
                        health: h.health,
                        food: h.food,
                        saturation: h.saturation,
                    },
                    sequence,
                ));
            }
            0x3f => {
                let slot = *p.first().context("missing held item slot")?;
                if slot > 8 {
                    return Err(api::recording::invalid(format!(
                        "invalid held item slot {slot}"
                    )));
                }
                receipts.selected_hotbar = Some(api::received(slot, sequence));
            }
            0x31 => {
                let mut c = Cursor::new(p);
                let flags = c.read_u8()?;
                c.read_f32::<BigEndian>()?;
                c.read_f32::<BigEndian>()?;
                receipts.may_fly = Some(flags & 4 != 0);
            }
            0x1e => {
                let mut c = Cursor::new(p);
                let reason = c.read_u8()?;
                let value = c.read_f32::<BigEndian>()?;
                if reason == 3 {
                    game_mode = Some(api::GameMode::decode(value as u8 & 7)?);
                }
            }
            _ => {
                unhandled.insert(record.packet_id);
            }
        }
    }
    if !joined {
        return Err(api::recording::invalid("replay has no initial play JOIN"));
    }
    if region.min[1] < 0 || region.max[1] > 255 {
        return Err(api::recording::invalid(
            "replay region outside legacy dimension",
        ));
    }
    let mut blocks = Vec::with_capacity(region.volume()?);
    for y in region.min[1]..=region.max[1] {
        for z in region.min[2]..=region.max[2] {
            for x in region.min[0]..=region.max[0] {
                blocks.push(ReplayedBlock {
                    position: [x, y, z],
                    state: world
                        .block(x, y, z)
                        .map(crate::versions::java_1_16_1::native_state)
                        .transpose()?,
                });
            }
        }
    }
    let player = api::PlayerObservation {
        using_item: None,
        session: api::SessionStamp {
            version: crate::MinecraftVersion::Java1_16_1,
            connection_id: trace.connection_id,
            world_generation: receipts.generation,
        },
        receive_sequence: trace.through_sequence,
        pending_dispatch: false,
        dimension,
        position: None,
        rotation: receipts.pose.as_ref().map_or([0.0; 2], |p| p.rotation),
        received_pose: receipts.pose.clone(),
        game_mode,
        may_fly: receipts.may_fly,
        health: receipts.health.clone(),
        selected_hotbar: receipts.selected_hotbar.clone(),
        inventory: receipts.inventory,
    };
    Ok(api::recording::project(
        &player,
        blocks,
        unhandled.into_iter().collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_relative_position_decoder_preserves_native_inputs_and_rejects_bad_angles() {
        let mut payload = vec![];
        for n in [1.0f64, 65.0, -1.0] {
            payload.extend(n.to_be_bytes());
        }
        payload.extend(5.0f32.to_be_bytes());
        payload.extend(2.0f32.to_be_bytes());
        payload.extend([1 | 4 | 8 | 16, 7]);
        let (p, r, id) = decode_position(&payload, [10.5, 100.0, 7.5], [80.0, 10.0]).unwrap();
        assert_eq!((p, r, id), ([11.5, 65.0, 6.5], [85.0, 12.0], 7));
        payload[24..28].copy_from_slice(&f32::NAN.to_be_bytes());
        assert!(decode_position(&payload, [0.0; 3], [0.0; 2]).is_err());
        for end in 0..34 {
            assert!(decode_position(&payload[..end], [0.0; 3], [0.0; 2]).is_err());
        }
    }
}

#[cfg(test)]
mod replay_tests {
    use super::*;
    use crate::client::recording::{PacketPhase, RecordedSlotKnowledge};
    #[test]
    fn legacy_replay_preserves_received_vs_missing_slots_and_correction_provenance() {
        let mut capture = TraceCapture::new(0, 1_000_000).unwrap();
        let mut join = vec![0, 0, 0, 1, 0, 255, 0, 10, 0, 0, 0];
        put_string(&mut join, "minecraft:overworld");
        put_string(&mut join, "minecraft:overworld");
        let mut position = vec![];
        for n in [1.0f64, 65.0, -1.0] {
            position.extend(n.to_be_bytes());
        }
        position.extend(5.0f32.to_be_bytes());
        position.extend(2.0f32.to_be_bytes());
        position.extend([1 | 4 | 8 | 16, 7]);
        let mut full = vec![0];
        full.extend(46i16.to_be_bytes());
        full.extend([0; 46]);
        let cursor = vec![255, 255, 255, 0];
        let packets = [
            (0x25, join, None),
            (
                0x35,
                position,
                Some(crate::client::recording::LocalPlayerBasis {
                    position: Some([10.5, 100.0, 7.5]),
                    rotation: [80.0, 10.0],
                    velocity: None,
                }),
            ),
            (0x14, full, None),
            (0x16, cursor, None),
            (0x20, vec![0; 8], None),
        ];
        for (index, (id, p, basis)) in packets.into_iter().enumerate() {
            capture.record(index as u64 + 1, 0, PacketPhase::Play, id, &p, basis);
        }
        let trace = capture.finish(crate::MinecraftVersion::Java1_16_1, 77, 5, 0);
        let loaded: PacketTrace =
            serde_json::from_slice(&serde_json::to_vec(&trace).unwrap()).unwrap();
        let replay = loaded
            .replay(
                crate::Region {
                    min: [0, 65, 0],
                    max: [0, 65, 0],
                },
                1,
            )
            .unwrap();
        assert_eq!(
            replay.received_pose.unwrap(),
            crate::client::ReceivedPose {
                position: [11.5, 65.0, 6.5],
                rotation: [85.0, 12.0],
                receive_sequence: 2
            }
        );
        assert_eq!(replay.world_generation, 1);
        assert_eq!(replay.source_connection_id, 77);
        assert!(replay.inventory.slots.iter().all(|s|matches!(s,Some(v) if v.value==RecordedSlotKnowledge::Empty && v.source==crate::client::ValueSource::Received{sequence:3})));
        assert_eq!(
            replay.inventory.cursor.unwrap().source,
            crate::client::ValueSource::Received { sequence: 4 }
        );
        assert_eq!(replay.unhandled_packets, vec![0x20]);
        assert_eq!(replay.blocks[0].state, None);
        assert_eq!(replay.health, None);
        assert_eq!(replay.selected_hotbar, None);
    }
}
