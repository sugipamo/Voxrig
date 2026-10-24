//! General received spawn lifetimes, separate from remote-player motion modeling.
use super::{Reader, State, ids};
use crate::client::entity::{
    EntityPositionCorrection, NativeMotion, NativeSpawn, NativeSpawnMotion,
};

pub(super) fn receive(state: &mut State, id: i32, payload: &[u8]) -> anyhow::Result<()> {
    let mut r = Reader::new(payload);
    match id {
        ids::play_clientbound::SPAWN_ENTITY => {
            let entity_id = r.varint()?;
            let uuid = r.take(16)?.try_into()?;
            let type_id = r.varint()?;
            let position = coordinates(&mut r)?;
            let velocity = super::super::wire::velocity(&mut r)?;
            let pitch = angle(&mut r)?;
            let yaw = angle(&mut r)?;
            let head_yaw = angle(&mut r)?;
            r.varint()?; // type-dependent object data remains native.
            r.end()?;
            state.entities.insert(
                crate::MinecraftVersion::Java1_21_11,
                NativeSpawn {
                    living: None,
                    id: entity_id,
                    uuid: Some(uuid),
                    type_id: Some(type_id),
                    dedicated_type_name: None,
                    position,
                },
                state.sequence,
                4096,
            )?;
            state.entities.initialize_motion(
                entity_id,
                NativeSpawnMotion {
                    position: Some(position),
                    rotation: Some([yaw, pitch]),
                    head_yaw: Some(head_yaw),
                    velocity: Some(velocity),
                },
                state.sequence,
            );
            state.vehicles.retire(entity_id);
        }
        ids::play_clientbound::ENTITY_STATUS => {
            let native_id = r.i32()?;
            let status = r.u8()? as i8;
            r.end()?;
            state.entities.history_signal(native_id, Some(status), None);
        }
        ids::play_clientbound::ANIMATION => {
            let native_id = r.varint()?;
            let animation = r.u8()?;
            r.end()?;
            state
                .entities
                .history_signal(native_id, None, Some(animation));
        }
        ids::play_clientbound::ENTITY_DESTROY => {
            let mut removed = Vec::new();
            for _ in 0..r.count(65536)? {
                removed.push(r.varint()?);
            }
            r.end()?;
            for id in removed {
                state.entities.remove(id);
                state.vehicles.retire(id);
            }
        }
        // Health and equipment are best-effort: a payload this reader cannot
        // follow is left to the other receivers and records nothing here.
        ids::play_clientbound::ENTITY_METADATA => {
            if let Some((entity, values, complete)) = common_metadata(payload) {
                state
                    .entities
                    .receive_metadata(entity, values, complete, state.sequence);
            }
            if let Some((entity, items)) = item_metadata(payload) {
                state
                    .entities
                    .receive_item_metadata(entity, items, state.sequence);
            }
            if let Some((entity, health)) = living_health(state, payload) {
                state
                    .entities
                    .receive_health(entity, health, state.sequence);
            }
        }
        ids::play_clientbound::ENTITY_EQUIPMENT => {
            if let Some((entity, slots, complete)) = equipment(payload) {
                state
                    .entities
                    .receive_equipment(entity, slots, complete, state.sequence);
            }
        }
        ids::play_clientbound::ENTITY_UPDATE_ATTRIBUTES => {
            let entity = r.varint()?;
            let attributes = super::players::read_attribute_details(&mut r)?;
            r.end()?; // Complete packet before mutating the lifetime ledger.
            if let Some((_, scale)) = attributes.get(&super::ids::SCALE_ATTRIBUTE) {
                state
                    .entities
                    .receive_scale(entity, scale.clone(), state.sequence);
            }
        }
        _ => {
            if let Some((target, update)) = decode(id, payload)? {
                let target = target.or_else(|| {
                    state
                        .vehicles
                        .mounted_entity(state.operations.local_player.entity_id, &state.entities)
                });
                if let Some(target) = target {
                    state.entities.receive_motion(
                        crate::MinecraftVersion::Java1_21_11,
                        target,
                        update,
                        state.sequence,
                    );
                }
            }
        }
    }
    Ok(())
}

/// Health (the table's LivingEntity metadata index, float) of a living entity type.
/// Entity data in the common form. Entries after a serializer this reader cannot follow
/// are not recorded (the ones before it are).
/// Entity id, entries and whether the terminator was reached.
type DecodedMetadata = (i32, Vec<(u8, crate::client::EntityDataValue)>, bool);

/// Values decoded up to the first entry this reader cannot follow; the flag is
/// true when the terminator was reached.
fn common_metadata(payload: &[u8]) -> Option<DecodedMetadata> {
    use crate::client::EntityDataValue as V;
    let mut r = Reader::new(payload);
    let entity = r.varint().ok()?;
    let mut values = Vec::new();
    let mut complete = false;
    loop {
        let Ok(key) = r.u8() else { break };
        if key == 255 {
            complete = r.end().is_ok();
            break;
        }
        let Ok(kind) = r.varint() else { break };
        let value = match kind {
            0 => r.u8().ok().map(|v| V::Byte(v as i8)),
            1 => r.varint().ok().map(V::Int),
            2 => r.varlong().ok().map(V::Long),
            3 => r.f32().ok().map(V::Float),
            4 => r.string().ok().map(V::String),
            // Read item-stack framing only in this additive entity observation.
            // The own-player parser keeps its existing unsupported-input policy.
            7 => super::operations::slot(&mut r)
                .ok()
                .flatten()
                .map(|_| V::Other(kind)),
            8 => r.bool().ok().map(V::Bool),
            10 => r.take(8).ok().map(|bytes| {
                V::BlockPosition(crate::client::BlockPos::unpack(u64::from_be_bytes(
                    bytes.try_into().expect("eight bytes"),
                )))
            }),
            // The original optional-position codec uses Netty readBoolean:
            // any nonzero byte means present. Keep this local to this codec.
            11 => (|| -> anyhow::Result<V> {
                let value = if r.u8()? != 0 {
                    Some(crate::client::BlockPos::unpack(u64::from_be_bytes(
                        r.take(8)?.try_into()?,
                    )))
                } else {
                    None
                };
                Ok(V::OptionalBlockPosition(value))
            })()
            .ok(),
            20 => r.varint().ok().map(V::Pose),
            _ => super::players::skip_metadata(&mut r, kind)
                .ok()
                .filter(|known| *known)
                .map(|_| V::Other(kind)),
        };
        let Some(value) = value else { break };
        values.push((key, value));
    }
    Some((entity, values, complete))
}

fn living_health(state: &State, payload: &[u8]) -> Option<(i32, f32)> {
    let mut r = Reader::new(payload);
    let entity = r.varint().ok()?;
    if !crate::client::entity::modern_living(state.entities.type_name(entity)?) {
        return None;
    }
    loop {
        let key = r.u8().ok()?;
        if key == 255 {
            return None;
        }
        let kind = r.varint().ok()?;
        if key
            == crate::MinecraftVersion::Java1_21_11
                .table()
                .entities
                .health_metadata_index
        {
            return (kind == 3)
                .then(|| r.f32().ok())
                .flatten()
                .map(|h| (entity, h));
        }
        if key
            > crate::MinecraftVersion::Java1_21_11
                .table()
                .entities
                .health_metadata_index
            || !super::players::skip_metadata(&mut r, kind).ok()?
        {
            return None;
        }
    }
}

/// Equipment entries up to the first one that cannot be decoded.
type DecodedEquipment = (
    i32,
    Vec<(crate::client::EquipmentSlot, crate::client::SlotKnowledge)>,
    bool,
);
fn equipment(payload: &[u8]) -> Option<DecodedEquipment> {
    let mut r = Reader::new(payload);
    let mut slots = Vec::new();
    let entity = r.varint().ok().filter(|id| *id >= 0)?;
    let mut complete = false;
    for _ in 0..16 {
        let Ok(raw) = r.u8() else { break };
        let Some(slot) = crate::client::EquipmentSlot::from_native(
            crate::MinecraftVersion::Java1_21_11,
            raw & 0x7f,
        ) else {
            break;
        };
        let Ok(Some(item)) = super::operations::slot(&mut r) else {
            break;
        };
        let item = super::operations::common_slot(&item)
            .unwrap_or(crate::client::SlotKnowledge::Unavailable);
        slots.push((slot, item));
        if raw & 0x80 == 0 {
            complete = r.end().is_ok();
            break;
        }
    }
    Some((entity, slots, complete))
}

fn angle(r: &mut Reader<'_>) -> anyhow::Result<f32> {
    Ok(f32::from(r.u8()? as i8) * 360.0 / 256.0)
}
fn short(r: &mut Reader<'_>) -> anyhow::Result<i16> {
    Ok(i16::from_be_bytes(r.take(2)?.try_into()?))
}
fn decode(id: i32, payload: &[u8]) -> anyhow::Result<Option<(Option<i32>, NativeMotion)>> {
    use ids::play_clientbound as p;
    if !matches!(
        id,
        p::REL_ENTITY_MOVE
            | p::ENTITY_MOVE_LOOK
            | p::ENTITY_LOOK
            | p::VEHICLE_MOVE
            | p::ENTITY_HEAD_ROTATION
            | p::ENTITY_VELOCITY
            | p::ENTITY_TELEPORT
            | p::SYNC_ENTITY_POSITION
    ) {
        return Ok(None);
    }
    let mut r = Reader::new(payload);
    let target = if id == p::VEHICLE_MOVE {
        None
    } else {
        Some(r.varint()?)
    };
    anyhow::ensure!(target.is_none_or(|id| id >= 0), "invalid entity motion ID");
    let update = match id {
        p::REL_ENTITY_MOVE | p::ENTITY_MOVE_LOOK => {
            let delta = [short(&mut r)?, short(&mut r)?, short(&mut r)?];
            let rotation = if id == p::ENTITY_MOVE_LOOK {
                Some([angle(&mut r)?, angle(&mut r)?])
            } else {
                None
            };
            NativeMotion::Relative {
                delta,
                rotation,
                ground: r.bool()?,
            }
        }
        p::ENTITY_LOOK => NativeMotion::Rotation {
            rotation: [angle(&mut r)?, angle(&mut r)?],
            ground: r.bool()?,
        },
        p::VEHICLE_MOVE | p::SYNC_ENTITY_POSITION => {
            let position = coordinates(&mut r)?;
            let velocity = if id == p::SYNC_ENTITY_POSITION {
                Some([r.f64()?, r.f64()?, r.f64()?])
            } else {
                None
            };
            let rotation = [r.f32()?, r.f32()?];
            let ground = if id == p::SYNC_ENTITY_POSITION {
                Some(r.bool()?)
            } else {
                None
            };
            NativeMotion::Absolute {
                position,
                rotation,
                velocity,
                ground,
                reset_base: id == p::SYNC_ENTITY_POSITION,
            }
        }
        p::ENTITY_TELEPORT => {
            let change = super::correction::Correction::read(&mut r)?;
            if change.flags & 7 == 0 {
                anyhow::ensure!(
                    change.position.iter().all(|v| v.abs() <= 33_554_432.0),
                    "entity correction coordinate out of bounds"
                );
            }
            NativeMotion::Correction {
                change: EntityPositionCorrection {
                    position: change.position,
                    delta: change.delta,
                    rotation: change.rotation,
                    flags: change.flags,
                },
                ground: r.bool()?,
            }
        }
        p::ENTITY_HEAD_ROTATION => NativeMotion::HeadYaw(angle(&mut r)?),
        p::ENTITY_VELOCITY => NativeMotion::Velocity(super::super::wire::velocity(&mut r)?),
        _ => unreachable!(),
    };
    r.end()?;
    Ok(Some((target, update)))
}

fn coordinates(r: &mut Reader<'_>) -> anyhow::Result<[f64; 3]> {
    let position = [r.f64()?, r.f64()?, r.f64()?];
    anyhow::ensure!(
        position.iter().all(|v| v.abs() <= 33_554_432.0),
        "entity coordinate out of bounds"
    );
    Ok(position)
}

#[cfg(test)]
mod common_metadata_tests {
    use super::*;
    #[test]
    fn original_position_codecs_preserve_values_and_boundaries() {
        use crate::client::EntityDataValue as V;
        for (packet, value) in
            crate::client::entity::original_position_packets(crate::MinecraftVersion::Java1_21_11)
        {
            let (_, entries, complete) = common_metadata(&packet).unwrap();
            assert!(complete);
            assert_eq!(entries, [(30, value.clone()), (31, V::Byte(42))]);
            if matches!(value, V::OptionalBlockPosition(Some(_))) {
                let mut nonzero = packet.clone();
                nonzero[3] = 255;
                assert_eq!(common_metadata(&nonzero).unwrap().1, entries);
            }
            for length in 3..packet.len() - 4 {
                let (_, entries, complete) = common_metadata(&packet[..length]).unwrap();
                assert!(entries.is_empty() && !complete);
            }
            let mut trailing = packet.clone();
            trailing.push(0);
            assert!(!common_metadata(&trailing).unwrap().2);
        }
    }

    use crate::client::EntityDataValue as V;

    #[test]
    fn partial_equipment_receipts_keep_a_single_ordered_packet_boundary() {
        use crate::client::{
            EntityHistoryKind as K, EntityHistoryValue as H, EquipmentSlot as S, SessionStamp,
            SlotKnowledge,
        };
        let version = crate::MinecraftVersion::Java1_21_11;
        let mut state = State::default();
        let cases: &[(&[u8], bool, &[S])] = &[
            (&[7, 0x85, 0, 5, 0], true, &[S::Head, S::Head]),
            (&[7, 5, 0, 1], false, &[S::Head]),
            (&[7, 0x80, 0, 99, 0], false, &[S::MainHand]),
            (&[7, 0x80, 0, 5, 1], false, &[S::MainHand]),
            (&[7], false, &[]),
        ];
        for (n, (payload, _, _)) in cases.iter().enumerate() {
            state.sequence = n as u64 + 1;
            state.entities.history_context(version, 0, state.sequence);
            receive(&mut state, ids::play_clientbound::ENTITY_EQUIPMENT, payload).unwrap();
        }
        // An unparseable ID cannot create a fabricated native_id=-1 receipt.
        receive(&mut state, ids::play_clientbound::ENTITY_EQUIPMENT, &[]).unwrap();
        let page = state
            .entities
            .history_after(
                SessionStamp {
                    version,
                    connection_id: 9,
                    world_generation: 0,
                },
                state.sequence,
                None,
                10,
            )
            .unwrap();
        assert_eq!(page.records.len(), cases.len());
        for (n, (record, (_, expected_complete, expected_slots))) in
            page.records.iter().zip(cases).enumerate()
        {
            assert_eq!(record.receive_sequence, n as u64 + 1);
            let K::Equipment {
                native_id,
                entity,
                entries: H::Retained { value },
                complete,
            } = &record.kind
            else {
                panic!()
            };
            assert_eq!(*native_id, 7);
            assert!(entity.is_none());
            assert_eq!(complete, expected_complete);
            assert_eq!(
                value.iter().map(|v| v.slot).collect::<Vec<_>>(),
                *expected_slots
            );
            assert!(value.iter().all(|v| v.value == SlotKnowledge::Empty));
        }
    }

    #[test]
    fn plain_values_decode_and_unknown_serializers_stop_the_entry_list() {
        // entity 9: index 0 byte 1, 17 varlong 300, 6 pose (20) 1, 9 float, 8 bool true,
        // then an unsupported serializer (99) and an entry after it.
        let mut payload = vec![9, 0, 0, 1, 17, 2, 0xac, 0x02, 6, 20, 1, 9, 3];
        payload.extend(2.5f32.to_be_bytes());
        payload.extend([8, 8, 1, 10, 99, 0, 11, 0, 0, 255]);
        let (entity, values, complete) = common_metadata(&payload).unwrap();
        assert_eq!(entity, 9);
        assert!(!complete);
        assert_eq!(
            values,
            vec![
                (0, V::Byte(1)),
                (17, V::Long(300)),
                (6, V::Pose(1)),
                (9, V::Float(2.5)),
                (8, V::Bool(true))
            ]
        );
        let (_, values, complete) = common_metadata(&[9, 0, 0, 1, 255]).unwrap();
        assert_eq!((values, complete), (vec![(0, V::Byte(1))], true));
    }

    #[test]
    fn pose_entries_require_the_original_serializer_and_clean_terminator() {
        for id in 0..18 {
            let (_, values, complete) = common_metadata(&[9, 6, 20, id, 255]).unwrap();
            assert_eq!(
                (values, complete),
                (vec![(6, V::Pose(i32::from(id)))], true)
            );
        }
        let (_, values, complete) = common_metadata(&[9, 6, 20, 5, 255, 0]).unwrap();
        assert_eq!((values, complete), (vec![(6, V::Pose(5))], false));
        let (_, values, complete) = common_metadata(&[9, 6, 1, 5, 255]).unwrap();
        assert_eq!((values, complete), (vec![(6, V::Int(5))], true));
    }
}

#[cfg(test)]
mod scale_receipt_tests {
    use super::*;
    use crate::{
        MinecraftVersion,
        client::{SessionStamp, ValueSource},
    };
    #[test]
    fn scale_updates_are_atomic_and_scoped_to_complete_live_spawns() {
        let mut state = State::default();
        let version = MinecraftVersion::Java1_21_11;
        let session = SessionStamp {
            version,
            connection_id: 1,
            world_generation: 0,
        };
        let kind = crate::client::registry::Registry::for_version(version)
            .builtin_id("minecraft:entity_type", "minecraft:cow")
            .unwrap()
            .value();
        state
            .entities
            .insert(
                version,
                NativeSpawn {
                    id: 7,
                    uuid: None,
                    type_id: Some(kind),
                    dedicated_type_name: None,
                    position: [0.; 3],
                    living: Some(true),
                },
                1,
                16,
            )
            .unwrap();
        let capture = |state: &State| {
            state
                .entities
                .capture_all(version, session, 99)
                .entities
                .pop()
                .unwrap()
                .scale_attribute
        };
        let mut packet = vec![7, 1, 25];
        packet.extend(2f64.to_be_bytes());
        packet.push(0);
        state.sequence = 3;
        receive(
            &mut state,
            ids::play_clientbound::ENTITY_UPDATE_ATTRIBUTES,
            &packet,
        )
        .unwrap();
        let original = capture(&state).unwrap();
        assert_eq!(original.value.base, 2.);
        assert_eq!(original.source, ValueSource::Received { sequence: 3 });
        let mut trailing = packet.clone();
        trailing.push(0);
        let mut duplicate = vec![7, 2];
        duplicate.extend_from_slice(&packet[2..]);
        duplicate.extend_from_slice(&packet[2..]);
        let mut nan = vec![7, 1, 25];
        nan.extend(f64::NAN.to_be_bytes());
        nan.push(0);
        for malformed in [
            &packet[..packet.len() - 1],
            trailing.as_slice(),
            duplicate.as_slice(),
            nan.as_slice(),
        ] {
            assert!(
                receive(
                    &mut state,
                    ids::play_clientbound::ENTITY_UPDATE_ATTRIBUTES,
                    malformed
                )
                .is_err()
            );
            assert_eq!(capture(&state), Some(original.clone()));
        }
        // Another entity and a packet omitting scale cannot rewrite this receipt.
        let mut other = packet.clone();
        other[0] = 8;
        receive(
            &mut state,
            ids::play_clientbound::ENTITY_UPDATE_ATTRIBUTES,
            &other,
        )
        .unwrap();
        receive(
            &mut state,
            ids::play_clientbound::ENTITY_UPDATE_ATTRIBUTES,
            &[7, 0],
        )
        .unwrap();
        assert_eq!(capture(&state), Some(original));
        state.entities.remove(7);
        receive(
            &mut state,
            ids::play_clientbound::ENTITY_UPDATE_ATTRIBUTES,
            &packet,
        )
        .unwrap();
        state
            .entities
            .insert(
                version,
                NativeSpawn {
                    id: 7,
                    uuid: None,
                    type_id: Some(kind),
                    dedicated_type_name: None,
                    position: [0.; 3],
                    living: Some(true),
                },
                9,
                16,
            )
            .unwrap();
        assert!(capture(&state).is_none());
    }
}

/// Read the item sidecar without changing the public exhaustive scalar enum.
fn item_metadata(payload: &[u8]) -> Option<(i32, Vec<(u8, crate::client::SlotKnowledge)>)> {
    use crate::client::SlotKnowledge;
    let mut r = Reader::new(payload);
    let entity = r.varint().ok()?;
    let mut items = Vec::new();
    while let Ok(index) = r.u8() {
        if index == 255 {
            break;
        }
        let Ok(kind) = r.varint() else { break };
        if kind == 7 {
            match super::operations::slot(&mut r) {
                Ok(Some(stack)) => items.push((
                    index,
                    super::operations::common_slot(&stack).unwrap_or(SlotKnowledge::Unavailable),
                )),
                // An unsupported/truncated new stack must not leave an old known one usable.
                _ => {
                    items.push((index, SlotKnowledge::Unavailable));
                    break;
                }
            }
        } else if !super::players::skip_metadata(&mut r, kind).unwrap_or(false) {
            break;
        }
    }
    Some((entity, items))
}

#[cfg(test)]
mod item_observation_gap_tests {
    use super::*;
    use crate::protocol::put_varint;
    use crate::{
        MinecraftVersion,
        client::{ItemData, SlotKnowledge, ValueSource, registry::Registry},
    };
    #[test]
    fn modern_item_metadata_retains_component_patch_and_invalidates_unsupported_stack() {
        let registry = Registry::for_version(MinecraftVersion::Java1_21_11);
        let item = registry.item("minecraft:diamond").unwrap();
        let component = registry.item_component("minecraft:custom_data").unwrap();
        let removed = registry.item_component("minecraft:custom_name").unwrap();
        let mut packet = vec![7, 8, 7, 3];
        put_varint(&mut packet, item.id.value());
        packet.extend([1, 1]);
        put_varint(&mut packet, component.id.value());
        let nbt = vec![10, 3, 0, 1, b'x', 0, 0, 0, 42, 0];
        packet.extend(&nbt);
        put_varint(&mut packet, removed.id.value());
        packet.extend([9, 0, 42, 255]);
        let (_, items) = item_metadata(&packet).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0, 8);
        let SlotKnowledge::Item { item: stack } = &items[0].1 else {
            panic!("known stack")
        };
        assert_eq!(stack.name, "minecraft:diamond");
        assert_eq!(stack.id, item.id);
        assert_eq!(stack.count, 3);
        assert_eq!(
            stack
                .custom_data()
                .unwrap()
                .unwrap()
                .root()
                .get("x")
                .unwrap()
                .as_int(),
            Some(42)
        );
        let ItemData::ModernComponents { patch } = &stack.data else {
            panic!("component patch")
        };
        assert_eq!(patch.added[0].bytes, nbt);
        assert_eq!(patch.removed, [removed]);
        let (_, scalar, complete) = common_metadata(&packet).unwrap();
        assert!(complete);
        assert_eq!(
            scalar.last(),
            Some(&(9, crate::client::EntityDataValue::Byte(42)))
        );
        let mut state = State {
            sequence: 5,
            ..Default::default()
        };
        state
            .entities
            .insert(
                MinecraftVersion::Java1_21_11,
                NativeSpawn {
                    id: 7,
                    uuid: None,
                    type_id: Some(
                        registry
                            .builtin_id("minecraft:entity_type", "minecraft:item")
                            .unwrap()
                            .value(),
                    ),
                    dedicated_type_name: None,
                    position: [0.; 3],
                    living: None,
                },
                1,
                16,
            )
            .unwrap();
        receive(&mut state, ids::play_clientbound::ENTITY_METADATA, &packet).unwrap();
        let session = crate::client::SessionStamp {
            version: MinecraftVersion::Java1_21_11,
            connection_id: 1,
            world_generation: 0,
        };
        let observed = state.entities.capture_items(session, 6);
        assert_eq!(
            observed.stacks[0].stack.source,
            ValueSource::Received { sequence: 5 }
        );
        assert_eq!(observed.stacks[0].stack.value, items[0].1);
        let mut unsupported = vec![7, 8, 7, 1];
        put_varint(&mut unsupported, item.id.value());
        unsupported.extend([1, 0]);
        put_varint(&mut unsupported, i32::MAX);
        state.sequence = 8;
        receive(
            &mut state,
            ids::play_clientbound::ENTITY_METADATA,
            &unsupported,
        )
        .unwrap();
        assert_eq!(
            state.entities.capture_items(session, 9).stacks[0].stack,
            crate::client::received(SlotKnowledge::Unavailable, 8)
        );
        receive(
            &mut state,
            ids::play_clientbound::ENTITY_METADATA,
            &[7, 8, 7, 0, 255],
        )
        .unwrap();
        assert_eq!(
            state.entities.capture_items(session, 9).stacks[0]
                .stack
                .value,
            SlotKnowledge::Empty
        );
        state.entities.remove(7);
        assert!(state.entities.capture_items(session, 10).stacks.is_empty());
    }
}
