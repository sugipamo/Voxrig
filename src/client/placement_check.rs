//! Received-state pre-check of one block placement: is the target loaded, can the
//! official client reach the clicked face, and is the target cell free of the own
//! player and of entities that block building (`EntityGetter.isUnobstructed`).
//! See docs/common-placement-check.md.
use super::{Aabb, BlockFace, EntityObservation, PlayerObservation};
use crate::{MinecraftVersion, NativeBlockState, Result};

/// Standing eye height of a player (`Player.getStandingEyeHeight`).
const EYE_HEIGHT: f64 = 1.62;

/// Received facts for placing against `support`'s `face`.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct PlacementCheck {
    /// Clicked support cell.
    pub support: [i32; 3],
    /// Cell the block would occupy: `support` moved one step out of `face`.
    pub target: [i32; 3],
    /// Received support state (None if unloaded).
    pub support_state: Option<NativeBlockState>,
    /// Received target state (None if unloaded).
    pub target_state: Option<NativeBlockState>,
    /// The target column is loaded.
    pub target_loaded: bool,
    /// Distance from the standing eye to the nearest point of the clicked face.
    pub face_distance: f64,
    /// The official client's pick range: `block_interaction_range` (1.21.11) or
    /// 4.5, 5.0 in creative (1.16.1).
    pub interaction_range: f64,
    /// `target_loaded` and the clicked face lies within `interaction_range`.
    pub reachable: bool,
    /// The own player's box intersects the target cell.
    pub player_intersects_target_cell: bool,
    /// Native ids of received entities that block building whose default box
    /// intersects the target cell.
    pub blocking_entities: Vec<i32>,
}

impl PlacementCheck {
    /// Loaded, reachable, and no own player or blocking entity in the target cell.
    /// Target replaceability and the block's own shape remain the server's decision.
    pub fn clear(&self) -> bool {
        self.reachable && !self.player_intersects_target_cell && self.blocking_entities.is_empty()
    }
}

pub(crate) fn offset(cell: [i32; 3], face: BlockFace) -> [i32; 3] {
    let [x, y, z] = cell;
    match face {
        BlockFace::Down => [x, y - 1, z],
        BlockFace::Up => [x, y + 1, z],
        BlockFace::North => [x, y, z - 1],
        BlockFace::South => [x, y, z + 1],
        BlockFace::West => [x - 1, y, z],
        BlockFace::East => [x + 1, y, z],
    }
}

fn cell_box([x, y, z]: [i32; 3]) -> Aabb {
    let (x, y, z) = (f64::from(x), f64::from(y), f64::from(z));
    Aabb {
        min_x: x,
        min_y: y,
        min_z: z,
        max_x: x + 1.0,
        max_y: y + 1.0,
        max_z: z + 1.0,
    }
}

fn intersects(a: &Aabb, b: &Aabb) -> bool {
    a.min_x < b.max_x
        && a.max_x > b.min_x
        && a.min_y < b.max_y
        && a.max_y > b.min_y
        && a.min_z < b.max_z
        && a.max_z > b.min_z
}

/// Distance from `eye` to the nearest point of `face` of `cell`.
fn face_distance(eye: [f64; 3], cell: [i32; 3], face: BlockFace) -> f64 {
    let b = cell_box(cell);
    let mut min = [b.min_x, b.min_y, b.min_z];
    let mut max = [b.max_x, b.max_y, b.max_z];
    let (axis, high) = match face {
        BlockFace::Down => (1, false),
        BlockFace::Up => (1, true),
        BlockFace::North => (2, false),
        BlockFace::South => (2, true),
        BlockFace::West => (0, false),
        BlockFace::East => (0, true),
    };
    let plane = if high { max[axis] } else { min[axis] };
    min[axis] = plane;
    max[axis] = plane;
    (0..3)
        .map(|i| (eye[i] - eye[i].clamp(min[i], max[i])).powi(2))
        .sum::<f64>()
        .sqrt()
}

/// `Entity.blocksBuilding` by type: living entities (armor stands unless marker),
/// falling blocks, primed TNT, end crystals, minecarts and boats.
pub(crate) fn blocks_building(version: MinecraftVersion, entity: &EntityObservation) -> bool {
    let Some(name) = entity.motion.entity.type_name.as_deref() else {
        return false;
    };
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    if name == "armor_stand" {
        // ArmorStand client flags: index 14 (1.16.1) / 15 (1.21.11), marker bit 0x10.
        let index = match version {
            MinecraftVersion::Java1_16_1 => 14,
            _ => 15,
        };
        let marker = matches!(
            entity.metadata.get(&index).map(|v| &v.value),
            Some(super::EntityDataValue::Byte(flags)) if flags & 0x10 != 0
        );
        return !marker;
    }
    entity.living == Some(true)
        || matches!(name, "falling_block" | "tnt" | "end_crystal")
        || name.ends_with("minecart")
        || name.ends_with("boat")
        || name.ends_with("raft")
}

pub(crate) fn interaction_range(version: MinecraftVersion, player: &PlayerObservation) -> f64 {
    match version {
        MinecraftVersion::Java1_16_1 => {
            if player.game_mode == Some(super::GameMode::Creative) {
                5.0
            } else {
                4.5
            }
        }
        _ => player
            .attributes
            .get("minecraft:block_interaction_range")
            .map_or(4.5, |a| a.value.value),
    }
}

/// The pre-check from received player, block and entity state.
pub(crate) fn check(
    version: MinecraftVersion,
    player: &PlayerObservation,
    support: [i32; 3],
    face: BlockFace,
    support_state: Option<NativeBlockState>,
    target_state: Option<NativeBlockState>,
    entities: &[EntityObservation],
) -> Result<PlacementCheck> {
    let position = player
        .position
        .as_ref()
        .ok_or_else(|| {
            crate::Error::new(
                crate::ErrorKind::State,
                anyhow::anyhow!("own position not received"),
            )
        })?
        .value;
    let target = offset(support, face);
    let target_box = cell_box(target);
    let eye = [position[0], position[1] + EYE_HEIGHT, position[2]];
    let face_distance = face_distance(eye, support, face);
    let interaction_range = interaction_range(version, player);
    let target_loaded = target_state.is_some();
    let player_box = Aabb {
        min_x: position[0] - 0.3,
        min_y: position[1],
        min_z: position[2] - 0.3,
        max_x: position[0] + 0.3,
        max_y: position[1] + 1.8,
        max_z: position[2] + 0.3,
    };
    let blocking_entities = entities
        .iter()
        .filter(|e| Some(e.motion.entity.id.native_id()) != player.entity_id)
        .filter(|e| blocks_building(version, e))
        .filter(|e| {
            e.bounding_box
                .as_ref()
                .is_some_and(|b| intersects(b, &target_box))
        })
        .map(|e| e.motion.entity.id.native_id())
        .collect();
    Ok(PlacementCheck {
        support,
        target,
        support_state,
        target_state,
        target_loaded,
        face_distance,
        interaction_range,
        reachable: target_loaded && face_distance <= interaction_range,
        player_intersects_target_cell: intersects(&player_box, &target_box),
        blocking_entities,
    })
}

impl super::Survival {
    /// Received-state pre-check of placing against `support`'s `face`. Reads only
    /// received state and sends nothing; the server still decides.
    pub async fn placement_check(
        &self,
        support: [i32; 3],
        face: BlockFace,
    ) -> Result<PlacementCheck> {
        let version = self.client.version();
        let player = self.client.player_state().await?;
        let target = offset(support, face);
        let support_state = self.client.block_state(support).await?;
        let target_state = self.client.block_state(target).await?;
        let entities = self.client.entities().await?;
        check(
            version,
            &player,
            support,
            face,
            support_state,
            target_state,
            &entities.entities,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn face_distance_is_to_the_clicked_face() {
        // Eye above the support's top face: straight-down distance.
        let d = face_distance([0.5, 3.0, 0.5], [0, 0, 0], BlockFace::Up);
        assert!((d - 2.0).abs() < 1e-12);
        // The west face of [4,0,0] is the plane x = 4.
        let d = face_distance([0.5, 0.5, 0.5], [4, 0, 0], BlockFace::West);
        assert!((d - 3.5).abs() < 1e-12);
        // The east face is one block farther.
        let d = face_distance([0.5, 0.5, 0.5], [4, 0, 0], BlockFace::East);
        assert!((d - 4.5).abs() < 1e-12);
    }

    #[test]
    fn offset_steps_out_of_the_face() {
        assert_eq!(offset([1, 2, 3], BlockFace::Up), [1, 3, 3]);
        assert_eq!(offset([1, 2, 3], BlockFace::North), [1, 2, 2]);
        assert_eq!(offset([1, 2, 3], BlockFace::West), [0, 2, 3]);
    }

    #[test]
    fn touching_boxes_do_not_intersect() {
        let cell = cell_box([0, 0, 0]);
        let beside = Aabb {
            min_x: 1.0,
            min_y: 0.0,
            min_z: 0.0,
            max_x: 1.6,
            max_y: 1.8,
            max_z: 0.6,
        };
        assert!(!intersects(&cell, &beside));
        let inside = Aabb {
            min_x: 0.9,
            ..beside
        };
        assert!(intersects(&cell, &inside));
    }
}
