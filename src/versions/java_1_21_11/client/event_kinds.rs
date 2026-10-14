//! Common change notifications for applied Java 1.21.11 packets.
use super::super::{ids::play_clientbound as input, wire::Reader};
use crate::client::EventKind as K;

/// Kinds for one packet that has already been applied successfully. Fields are
/// re-read only for coordinates and identifiers; a short read yields nothing.
pub(super) fn kinds(id: i32, payload: &[u8]) -> Vec<K> {
    read(id, payload).unwrap_or_default()
}

fn read(id: i32, payload: &[u8]) -> anyhow::Result<Vec<K>> {
    let mut r = Reader::new(payload);
    let block = |p: [i32; 3]| K::BlocksChanged { min: p, max: p };
    let mut kinds = match id {
        input::BLOCK_CHANGE | input::TILE_ENTITY_DATA => {
            vec![block(super::super::wire::unpack_position(r.u64()?))]
        }
        input::MULTI_BLOCK_CHANGE => {
            let section = r.u64()? as i64;
            let origin = [
                (section >> 42) as i32 * 16,
                (section << 44 >> 44) as i32 * 16,
                (section << 22 >> 42) as i32 * 16,
            ];
            vec![K::BlocksChanged {
                min: origin,
                max: origin.map(|v| v + 15),
            }]
        }
        input::MAP_CHUNK => {
            let x = r.i32()?;
            let z = r.i32()?;
            vec![K::ChunkLoaded { x, z }]
        }
        input::UNLOAD_CHUNK => {
            let z = r.i32()?;
            let x = r.i32()?;
            vec![K::ChunkUnloaded { x, z }]
        }
        input::WINDOW_ITEMS | input::SET_SLOT => {
            if r.varint()? == 0 {
                vec![K::InventoryChanged]
            } else {
                vec![K::ScreenChanged]
            }
        }
        input::SET_PLAYER_INVENTORY | input::SET_CURSOR_ITEM | input::HELD_ITEM_SLOT => {
            vec![K::InventoryChanged]
        }
        input::OPEN_WINDOW | input::CLOSE_WINDOW | input::CRAFT_PROGRESS_BAR => {
            vec![K::ScreenChanged]
        }
        input::POSITION
        | input::UPDATE_HEALTH
        | input::EXPERIENCE
        | input::ABILITIES
        | input::GAME_STATE_CHANGE => vec![K::PlayerChanged],
        input::LOGIN | input::RESPAWN | input::START_CONFIGURATION => vec![K::WorldChanged],
        input::SYNC_ENTITY_POSITION
        | input::REL_ENTITY_MOVE
        | input::ENTITY_MOVE_LOOK
        | input::ENTITY_LOOK
        | input::ENTITY_HEAD_ROTATION
        | input::ENTITY_METADATA
        | input::ENTITY_VELOCITY
        | input::ENTITY_EQUIPMENT
        | input::ENTITY_TELEPORT => vec![K::EntityUpdated {
            native_id: r.varint()?,
        }],
        input::ENTITY_STATUS => vec![K::EntityStatus {
            native_id: r.i32()?,
            status: r.u8()? as i8,
        }],
        input::DAMAGE_EVENT => vec![K::EntityDamaged {
            native_id: r.varint()?,
        }],
        input::DEATH_COMBAT_EVENT => vec![K::PlayerKilled {
            native_id: r.varint()?,
        }],
        input::SPAWN_ENTITY => vec![K::EntitySpawned {
            native_id: r.varint()?,
        }],
        input::ENTITY_DESTROY => {
            let mut removed = Vec::new();
            for _ in 0..r.count(65_536)? {
                removed.push(K::EntityRemoved {
                    native_id: r.varint()?,
                });
            }
            removed
        }
        input::SYSTEM_CHAT | input::PLAYER_CHAT | input::PROFILELESS_CHAT => {
            vec![K::ChatReceived]
        }
        input::TEAMS
        | input::BOSS_BAR
        | input::SCOREBOARD_OBJECTIVE
        | input::SCOREBOARD_DISPLAY_OBJECTIVE
        | input::SCOREBOARD_SCORE
        | input::RESET_SCORE
        | input::PLAYER_INFO
        | input::PLAYER_REMOVE
        | input::SET_TITLE_TEXT
        | input::SET_TITLE_SUBTITLE
        | input::ACTION_BAR
        | input::SET_TITLE_TIME
        | input::CLEAR_TITLES
        | input::PLAYERLIST_HEADER
        | input::INITIALIZE_WORLD_BORDER
        | input::WORLD_BORDER_CENTER
        | input::WORLD_BORDER_SIZE
        | input::WORLD_BORDER_LERP_SIZE
        | input::WORLD_BORDER_WARNING_DELAY
        | input::WORLD_BORDER_WARNING_REACH => vec![K::UiChanged],
        _ => Vec::new(),
    };
    if matches!(
        id,
        input::LOGIN
            | input::START_CONFIGURATION
            | input::RESPAWN
            | input::ABILITIES
            | input::DIFFICULTY
            | input::EXPERIENCE
            | input::GAME_STATE_CHANGE
            | input::SPAWN_POSITION
            | input::UPDATE_VIEW_POSITION
            | input::UPDATE_VIEW_DISTANCE
            | input::SIMULATION_DISTANCE
            | input::SET_COOLDOWN
    ) {
        kinds.push(K::ContextChanged);
    }
    Ok(kinds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack_section(x: i64, y: i64, z: i64) -> i64 {
        ((x & 0x3f_ffff) << 42) | ((z & 0x3f_ffff) << 20) | (y & 0xf_ffff)
    }

    #[test]
    fn section_positions_cover_negative_sections() {
        let section = pack_section(-1, -4, 2);
        let kinds = kinds(input::MULTI_BLOCK_CHANGE, &section.to_be_bytes());
        assert_eq!(
            kinds,
            vec![K::BlocksChanged {
                min: [-16, -64, 32],
                max: [-1, -49, 47]
            }]
        );
    }

    #[test]
    fn removals_and_window_targets_are_distinguished() {
        assert_eq!(
            kinds(input::ENTITY_DESTROY, &[2, 5, 7]),
            vec![
                K::EntityRemoved { native_id: 5 },
                K::EntityRemoved { native_id: 7 }
            ]
        );
        assert_eq!(kinds(input::SET_SLOT, &[0]), vec![K::InventoryChanged]);
        assert_eq!(kinds(input::SET_SLOT, &[3]), vec![K::ScreenChanged]);
        assert!(kinds(input::BLOCK_CHANGE, &[1]).is_empty());
    }
}

#[cfg(test)]
mod entity_event_tests {
    use super::*;

    #[test]
    fn entity_updates_status_damage_and_death_map_to_common_kinds() {
        assert_eq!(
            kinds(input::REL_ENTITY_MOVE, &[5, 0, 1, 0, 0, 0, 0, 1]),
            vec![K::EntityUpdated { native_id: 5 }]
        );
        assert_eq!(
            kinds(input::ENTITY_STATUS, &[0, 0, 0, 5, 3]),
            vec![K::EntityStatus {
                native_id: 5,
                status: 3
            }]
        );
        assert_eq!(
            kinds(input::DAMAGE_EVENT, &[5, 1, 0, 0, 0]),
            vec![K::EntityDamaged { native_id: 5 }]
        );
        assert_eq!(
            kinds(input::DEATH_COMBAT_EVENT, &[7, 8, 0]),
            vec![K::PlayerKilled { native_id: 7 }]
        );
    }
}
