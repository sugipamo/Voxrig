use crate::versions::java_1_16_1::physics::Aabb;
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashMap, sync::OnceLock};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RelativeAabb([f64; 6]);

impl RelativeAabb {
    pub fn at(self, x: i32, y: i32, z: i32) -> Aabb {
        Aabb {
            min_x: f64::from(x) + self.0[0],
            min_y: f64::from(y) + self.0[1],
            min_z: f64::from(z) + self.0[2],
            max_x: f64::from(x) + self.0[3],
            max_y: f64::from(y) + self.0[4],
            max_z: f64::from(z) + self.0[5],
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BlockRange {
    name: String,
    min_state_id: usize,
    max_state_id: usize,
}

#[derive(Deserialize)]
struct ShapeData {
    blocks: HashMap<String, Value>,
    shapes: HashMap<String, Vec<[f64; 6]>>,
}

struct CollisionRegistry {
    by_state: Vec<Vec<RelativeAabb>>,
    names_by_state: Vec<String>,
}
static REGISTRY: OnceLock<CollisionRegistry> = OnceLock::new();

pub(crate) fn shapes_for(state_id: i32) -> &'static [RelativeAabb] {
    if state_id < 0 {
        return &[];
    }
    REGISTRY
        .get_or_init(load)
        .by_state
        .get(state_id as usize)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

pub(crate) fn block_name(state_id: i32) -> Option<&'static str> {
    if state_id < 0 {
        return None;
    }
    REGISTRY
        .get_or_init(load)
        .names_by_state
        .get(state_id as usize)
        .map(String::as_str)
}

/// Coarse collision fact for one known block state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockCollision {
    /// The state has no collision boxes.
    Empty,
    /// The state has at least one collision box.
    NonEmpty,
}

/// Registry geometry fact describing whether a block state exposes a complete
/// unit-square support surface at the top of its block cell.
///
/// This cross-crate fact is intentionally not a traversal or safety decision.
/// Consumers combine it with fresh world position and movement facts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockSupportSurface {
    /// Collision boxes cover the complete `[0, 1] x [0, 1]` top face at y=1.
    FullTop,
    /// The state is known, but its collision boxes do not prove a full top face.
    NotFullTop,
}

/// One raw collision box relative to the block's integer position.
///
/// This is registry geometry only. It does not decide whether a player can
/// traverse a cell, and it must not be used as a route or safety result by the
/// client.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockCollisionShape {
    /// Minimum X offset in the block.
    pub min_x: f64,
    /// Minimum Y offset in the block.
    pub min_y: f64,
    /// Minimum Z offset in the block.
    pub min_z: f64,
    /// Maximum X offset in the block.
    pub max_x: f64,
    /// Maximum Y offset in the block.
    pub max_y: f64,
    /// Maximum Z offset in the block.
    pub max_z: f64,
}

/// Returns the collision fact for a known state ID.
pub fn block_collision(state_id: i32) -> Option<BlockCollision> {
    block_name(state_id).map(|_| {
        if shapes_for(state_id).is_empty() {
            BlockCollision::Empty
        } else {
            BlockCollision::NonEmpty
        }
    })
}

/// Returns the exact full-top support fact derived by the version-pinned
/// collision registry. Unknown state IDs remain `None` rather than being
/// weakened to `NotFullTop`.
pub fn block_support_surface(state_id: i32) -> Option<BlockSupportSurface> {
    block_name(state_id)?;
    support_surface_from_shapes(shapes_for(state_id))
}

fn support_surface_from_shapes(shapes: &[RelativeAabb]) -> Option<BlockSupportSurface> {
    let valid = shapes.iter().all(|shape| {
        shape.0.iter().all(|value| value.is_finite())
            && shape.0[0] <= shape.0[3]
            && shape.0[1] <= shape.0[4]
            && shape.0[2] <= shape.0[5]
    });
    if !valid {
        return None;
    }
    let unit_bounded = shapes.iter().all(|shape| {
        shape.0[0] >= 0.0
            && shape.0[1] >= 0.0
            && shape.0[2] >= 0.0
            && shape.0[3] <= 1.0
            && shape.0[4] <= 1.0
            && shape.0[5] <= 1.0
    });
    if !unit_bounded {
        return Some(BlockSupportSurface::NotFullTop);
    }
    let rectangles = shapes
        .iter()
        .filter_map(|shape| {
            (shape.0[1] < 1.0 && shape.0[4] == 1.0).then(|| {
                (
                    shape.0[0].clamp(0.0, 1.0),
                    shape.0[3].clamp(0.0, 1.0),
                    shape.0[2].clamp(0.0, 1.0),
                    shape.0[5].clamp(0.0, 1.0),
                )
            })
        })
        .filter(|(min_x, max_x, min_z, max_z)| min_x < max_x && min_z < max_z)
        .collect::<Vec<_>>();
    let mut xs = vec![0.0, 1.0];
    let mut zs = vec![0.0, 1.0];
    for &(min_x, max_x, min_z, max_z) in &rectangles {
        xs.extend([min_x, max_x]);
        zs.extend([min_z, max_z]);
    }
    xs.sort_by(f64::total_cmp);
    xs.dedup_by(|left, right| left.to_bits() == right.to_bits());
    zs.sort_by(f64::total_cmp);
    zs.dedup_by(|left, right| left.to_bits() == right.to_bits());
    let covered = xs.windows(2).all(|x| {
        zs.windows(2).all(|z| {
            let sample_x = (x[0] + x[1]) * 0.5;
            let sample_z = (z[0] + z[1]) * 0.5;
            rectangles.iter().any(|&(min_x, max_x, min_z, max_z)| {
                min_x <= sample_x && sample_x <= max_x && min_z <= sample_z && sample_z <= max_z
            })
        })
    });
    Some(if covered {
        BlockSupportSurface::FullTop
    } else {
        BlockSupportSurface::NotFullTop
    })
}

/// Returns the exact raw collision boxes for a known block state.
///
/// `None` means that the state ID is not present in the bundled 1.16.1
/// registry. A known non-colliding state returns `Some(Vec::new())`. The
/// returned geometry is a bounded registry fact; path selection, traversal
/// validation, and semantic safety remain responsibilities of the consumer.
pub fn block_collision_shapes(state_id: i32) -> Option<Vec<BlockCollisionShape>> {
    block_name(state_id)?;
    Some(
        shapes_for(state_id)
            .iter()
            .map(|shape| BlockCollisionShape {
                min_x: shape.0[0],
                min_y: shape.0[1],
                min_z: shape.0[2],
                max_x: shape.0[3],
                max_y: shape.0[4],
                max_z: shape.0[5],
            })
            .collect(),
    )
}

fn load() -> CollisionRegistry {
    let ranges: Vec<BlockRange> =
        serde_json::from_str(include_str!("../../../data/block_state_ranges.json"))
            .expect("bundled block ranges must be valid");
    let data: ShapeData =
        serde_json::from_str(include_str!("../../../data/block_collision_shapes.json"))
            .expect("bundled collision shapes must be valid");
    let max_state = ranges.iter().map(|r| r.max_state_id).max().unwrap_or(0);
    let mut by_state = vec![Vec::new(); max_state + 1];
    let mut names_by_state = vec![String::new(); max_state + 1];
    for range in ranges {
        let Some(mapping) = data.blocks.get(&range.name) else {
            continue;
        };
        for (offset, slot) in by_state[range.min_state_id..=range.max_state_id]
            .iter_mut()
            .enumerate()
        {
            names_by_state[range.min_state_id + offset] = range.name.clone();
            let shape_id = match mapping {
                Value::Number(number) => number.as_u64(),
                Value::Array(ids) => ids.get(offset).and_then(Value::as_u64),
                _ => None,
            };
            let Some(shape_id) = shape_id else {
                continue;
            };
            if let Some(boxes) = data.shapes.get(&shape_id.to_string()) {
                *slot = boxes.iter().copied().map(RelativeAabb).collect();
            }
        }
    }
    CollisionRegistry {
        by_state,
        names_by_state,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_distinguishes_air_full_blocks_and_slabs() {
        assert!(shapes_for(0).is_empty());
        assert_eq!(
            shapes_for(1),
            &[RelativeAabb([0.0, 0.0, 0.0, 1.0, 1.0, 1.0])]
        );
        let ranges: Vec<BlockRange> =
            serde_json::from_str(include_str!("../../../data/block_state_ranges.json")).unwrap();
        let slab = ranges.iter().find(|r| r.name == "oak_slab").unwrap();
        assert!(
            shapes_for(slab.min_state_id as i32)
                .iter()
                .any(|shape| shape.0[4] == 0.5 || shape.0[1] == 0.5)
        );
        assert!(shapes_for(3968).iter().any(|shape| shape.0[4] == 1.5));
    }

    #[test]
    fn exact_shapes_are_raw_and_distinguish_unknown_from_empty() {
        assert_eq!(block_collision_shapes(0), Some(Vec::new()));
        assert!(block_collision_shapes(1).is_some_and(|shapes| {
            shapes
                == vec![BlockCollisionShape {
                    min_x: 0.0,
                    min_y: 0.0,
                    min_z: 0.0,
                    max_x: 1.0,
                    max_y: 1.0,
                    max_z: 1.0,
                }]
        }));
        assert!(block_collision_shapes(-1).is_none());
        assert!(block_collision_shapes(i32::MAX).is_none());
    }

    #[test]
    fn full_top_support_is_a_registry_fact_and_unknown_stays_unknown() {
        assert_eq!(
            block_support_surface(0),
            Some(BlockSupportSurface::NotFullTop)
        );
        assert_eq!(block_support_surface(1), Some(BlockSupportSurface::FullTop));
        assert_eq!(block_support_surface(-1), None);
        assert_eq!(block_support_surface(i32::MAX), None);
    }

    #[test]
    fn full_top_support_accepts_bounded_rectangle_unions_but_not_partial_shapes() {
        let halves = [
            RelativeAabb([0.0, 0.0, 0.0, 0.5, 1.0, 1.0]),
            RelativeAabb([0.5, 0.0, 0.0, 1.0, 1.0, 1.0]),
        ];
        assert_eq!(
            support_surface_from_shapes(&halves),
            Some(BlockSupportSurface::FullTop)
        );
        assert_eq!(
            support_surface_from_shapes(&halves[..1]),
            Some(BlockSupportSurface::NotFullTop)
        );
        assert_eq!(
            support_surface_from_shapes(&[RelativeAabb([0.0, 0.0, 0.0, 1.0, 0.5, 1.0])]),
            Some(BlockSupportSurface::NotFullTop)
        );
        assert_eq!(
            support_surface_from_shapes(&[RelativeAabb([0.0, 0.0, 0.0, 1.0, 1.5, 1.0])]),
            Some(BlockSupportSurface::NotFullTop)
        );
        assert_eq!(
            support_surface_from_shapes(&[RelativeAabb([0.0, 0.0, 0.0, f64::NAN, 1.0, 1.0,])]),
            None
        );
    }

    #[test]
    fn every_block_range_has_a_complete_shape_mapping() {
        let ranges: Vec<BlockRange> =
            serde_json::from_str(include_str!("../../../data/block_state_ranges.json")).unwrap();
        let data: ShapeData =
            serde_json::from_str(include_str!("../../../data/block_collision_shapes.json"))
                .unwrap();
        for range in ranges {
            let mapping = data
                .blocks
                .get(&range.name)
                .unwrap_or_else(|| panic!("missing collision mapping for {}", range.name));
            if let Value::Array(ids) = mapping {
                assert!(
                    ids.len() > range.max_state_id - range.min_state_id,
                    "not enough state shapes for {}",
                    range.name
                );
            }
        }
    }
}
