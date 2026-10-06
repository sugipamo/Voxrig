//! General received spawn lifetimes, separate from remote-player motion modeling.
use super::{Reader, State, ids};
use crate::client::entity::NativeSpawn;

pub(super) fn receive(state: &mut State, id: i32, payload: &[u8]) -> anyhow::Result<()> {
    let mut r = Reader::new(payload);
    match id {
        ids::play_clientbound::SPAWN_ENTITY => {
            let entity_id = r.varint()?;
            let uuid = r.take(16)?.try_into()?;
            let type_id = r.varint()?;
            let position = [r.f64()?, r.f64()?, r.f64()?];
            super::super::wire::velocity(&mut r)?;
            r.take(3)?; // pitch, yaw, head yaw; not claimed as current pose.
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
        }
        ids::play_clientbound::ENTITY_DESTROY => {
            let mut removed = Vec::new();
            for _ in 0..r.count(65536)? {
                removed.push(r.varint()?);
            }
            r.end()?;
            for id in removed {
                state.entities.remove(id);
            }
        }
        _ => {}
    }
    Ok(())
}
