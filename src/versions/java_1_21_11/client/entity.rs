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
