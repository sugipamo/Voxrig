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
            if let Some((entity, health)) = living_health(state, payload) {
                state
                    .entities
                    .receive_health(entity, health, state.sequence);
            }
        }
        ids::play_clientbound::ENTITY_EQUIPMENT => {
            let (entity, slots) = equipment(payload);
            for (slot, item) in slots {
                state
                    .entities
                    .receive_equipment(entity, slot, item, state.sequence);
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
            complete = true;
            break;
        }
        let Ok(kind) = r.varint() else { break };
        let value = match kind {
            0 => r.u8().ok().map(|v| V::Byte(v as i8)),
            1 => r.varint().ok().map(V::Int),
            2 => r.varlong().ok().map(V::Long),
            3 => r.f32().ok().map(V::Float),
            4 => r.string().ok().map(V::String),
            8 => r.bool().ok().map(V::Bool),
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
fn equipment(
    payload: &[u8],
) -> (
    i32,
    Vec<(crate::client::EquipmentSlot, crate::client::SlotKnowledge)>,
) {
    let mut r = Reader::new(payload);
    let mut slots = Vec::new();
    let Ok(entity) = r.varint() else {
        return (-1, slots);
    };
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
            break;
        }
    }
    (entity, slots)
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
    use crate::client::EntityDataValue as V;

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
                (6, V::Other(20)),
                (9, V::Float(2.5)),
                (8, V::Bool(true))
            ]
        );
        let (_, values, complete) = common_metadata(&[9, 0, 0, 1, 255]).unwrap();
        assert_eq!((values, complete), (vec![(0, V::Byte(1))], true));
    }
}
