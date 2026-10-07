//! Block search and collision raycasts over one coherent capture, common to both versions.
//!
//! Both operate on received client state only. Unloaded cells are reported as
//! such and are never treated as air.
use super::adapter::CoreOps;
use super::{BlockFace, SessionStamp};
use crate::{Error, ErrorKind, MinecraftVersion, NativeBlockState, ObservedBlock, Region, Result};

/// Longest supported raycast, in blocks; keeps the captured box bounded.
pub const MAX_RAYCAST_DISTANCE: f64 = 32.0;

/// Blocks matching a name filter inside one capture.
#[derive(Clone, Debug, serde::Serialize)]
pub struct BlockSearch {
    /// Connection and world of the capture.
    pub session: SessionStamp,
    /// Receive boundary of the capture.
    pub receive_sequence: u64,
    /// Matching cells, in capture order.
    pub matches: Vec<ObservedBlock>,
    /// Cells that were not loaded; a non-zero count means the search is incomplete.
    pub unloaded: usize,
}

/// Where a collision raycast stopped.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub enum BlockRaycast {
    /// The ray entered a block collision box.
    Hit {
        /// Cell whose collision shape was hit.
        position: [i32; 3],
        /// Complete received state of that cell.
        state: NativeBlockState,
        /// Face of the box the ray entered through.
        face: BlockFace,
        /// Entry point.
        point: [f64; 3],
        /// Distance from the origin to `point`.
        distance: f64,
    },
    /// Every traversed cell was loaded and nothing was hit within the distance.
    Miss,
    /// The ray reached an unloaded cell before hitting anything.
    Unloaded {
        /// First unloaded cell on the path.
        position: [i32; 3],
    },
}

/// A raycast result with the capture it was computed from.
#[derive(Clone, Debug, serde::Serialize)]
pub struct BlockRaycastObservation {
    /// Connection and world of the capture.
    pub session: SessionStamp,
    /// Receive boundary of the capture.
    pub receive_sequence: u64,
    /// Outcome.
    pub result: BlockRaycast,
}

fn invalid(message: &str) -> Error {
    Error::new(ErrorKind::InvalidInput, anyhow::anyhow!("{message}"))
}

/// Native collision boxes of a complete state, cell-relative.
pub(crate) fn collision_boxes(
    version: MinecraftVersion,
    state: &NativeBlockState,
) -> Result<Vec<[f64; 6]>> {
    (version.table().registry.collision_boxes)(state)
}

/// Entry distance and face of a ray into an axis-aligned box, if within `limit`.
fn enter(
    origin: [f64; 3],
    direction: [f64; 3],
    b: [f64; 6],
    limit: f64,
) -> Option<(f64, usize, bool)> {
    let (mut near, mut far) = (f64::NEG_INFINITY, limit);
    let mut entry = (0usize, false);
    for axis in 0..3 {
        if direction[axis] == 0.0 {
            if origin[axis] < b[axis] || origin[axis] > b[axis + 3] {
                return None;
            }
            continue;
        }
        let t1 = (b[axis] - origin[axis]) / direction[axis];
        let t2 = (b[axis + 3] - origin[axis]) / direction[axis];
        let (lo, hi) = if t1 < t2 { (t1, t2) } else { (t2, t1) };
        if lo > near {
            near = lo;
            entry = (axis, direction[axis] > 0.0);
        }
        far = far.min(hi);
        if near > far {
            return None;
        }
    }
    // An origin inside the box is not an entry.
    (near >= 0.0).then_some((near, entry.0, entry.1))
}

fn face(axis: usize, positive: bool) -> BlockFace {
    // A ray travelling +axis enters through the box's negative face.
    match (axis, positive) {
        (0, true) => BlockFace::West,
        (0, false) => BlockFace::East,
        (1, true) => BlockFace::Down,
        (1, false) => BlockFace::Up,
        (2, true) => BlockFace::North,
        _ => BlockFace::South,
    }
}

/// Traverse cells along the ray (Amanatides–Woo) and test each cell's boxes,
/// stopping at the first cell with a hit, like the native clip.
pub(crate) fn raycast(
    version: MinecraftVersion,
    origin: [f64; 3],
    direction: [f64; 3],
    distance: f64,
    cell: impl Fn([i32; 3]) -> Option<Option<NativeBlockState>>,
) -> Result<BlockRaycast> {
    let mut position = origin.map(|v| v.floor() as i32);
    let step = direction.map(|d| {
        if d > 0.0 {
            1
        } else if d < 0.0 {
            -1
        } else {
            0
        }
    });
    let mut next = [0.0f64; 3];
    let mut delta = [f64::INFINITY; 3];
    for axis in 0..3 {
        if direction[axis] != 0.0 {
            delta[axis] = 1.0 / direction[axis].abs();
            let boundary = f64::from(position[axis]) + if step[axis] > 0 { 1.0 } else { 0.0 };
            next[axis] = (boundary - origin[axis]) / direction[axis];
        } else {
            next[axis] = f64::INFINITY;
        }
    }
    loop {
        match cell(position).ok_or_else(|| invalid("raycast left the captured region"))? {
            None => return Ok(BlockRaycast::Unloaded { position }),
            Some(state) => {
                let mut best: Option<(f64, usize, bool)> = None;
                for b in collision_boxes(version, &state)? {
                    let world = [
                        b[0] + f64::from(position[0]),
                        b[1] + f64::from(position[1]),
                        b[2] + f64::from(position[2]),
                        b[3] + f64::from(position[0]),
                        b[4] + f64::from(position[1]),
                        b[5] + f64::from(position[2]),
                    ];
                    if let Some(hit) = enter(origin, direction, world, distance) {
                        if best.is_none_or(|(t, ..)| hit.0 < t) {
                            best = Some(hit);
                        }
                    }
                }
                if let Some((t, axis, positive)) = best {
                    return Ok(BlockRaycast::Hit {
                        position,
                        state,
                        face: face(axis, positive),
                        point: [0, 1, 2].map(|a| origin[a] + direction[a] * t),
                        distance: t,
                    });
                }
            }
        }
        let axis = (0..3)
            .min_by(|&a, &b| next[a].total_cmp(&next[b]))
            .expect("three axes");
        if next[axis] > distance {
            return Ok(BlockRaycast::Miss);
        }
        position[axis] += step[axis];
        next[axis] += delta[axis];
    }
}

impl super::Client {
    /// Find loaded blocks whose namespaced name is in `names` (for example
    /// `"minecraft:oak_log"`), inside one bounded capture.
    pub async fn find_blocks(&self, region: Region, names: &[&str]) -> Result<BlockSearch> {
        if names.is_empty() {
            return Err(invalid("block search needs at least one name"));
        }
        let capture = super::dispatch!(&self.adapter, a => CoreOps::capture(a, region).await)?;
        let mut unloaded = 0;
        let matches = capture
            .world
            .blocks
            .into_iter()
            .filter(|cell| match &cell.state {
                None => {
                    unloaded += 1;
                    false
                }
                Some(state) => names.contains(&state.name.as_str()),
            })
            .collect();
        Ok(BlockSearch {
            session: capture.player.session,
            receive_sequence: capture.player.receive_sequence,
            matches,
            unloaded,
        })
    }

    /// Cast a ray against received block collision shapes, from `origin` along
    /// `direction` for at most `max_distance` (up to 32) blocks. Computed from
    /// one capture; this is client geometry, not server line of sight.
    pub async fn raycast_blocks(
        &self,
        origin: [f64; 3],
        direction: [f64; 3],
        max_distance: f64,
    ) -> Result<BlockRaycastObservation> {
        let length = direction.iter().map(|v| v * v).sum::<f64>().sqrt();
        if origin.iter().chain(&direction).any(|v| !v.is_finite()) || length == 0.0 {
            return Err(invalid(
                "raycast origin and direction must be finite and non-zero",
            ));
        }
        if !(max_distance > 0.0 && max_distance <= MAX_RAYCAST_DISTANCE) {
            return Err(invalid("raycast distance must be in (0, 32]"));
        }
        let direction = direction.map(|v| v / length);
        let end = [0, 1, 2].map(|a| origin[a] + direction[a] * max_distance);
        let mut region = Region {
            min: [0, 1, 2].map(|a| origin[a].min(end[a]).floor() as i32),
            max: [0, 1, 2].map(|a| origin[a].max(end[a]).floor() as i32),
        };
        // Cells above or below the world contain nothing; only capture inside it.
        let player = super::dispatch!(&self.adapter, a => CoreOps::player_state(a).await)?;
        let dimension = player.dimension.ok_or_else(|| {
            Error::new(
                ErrorKind::State,
                anyhow::anyhow!("dimension height is unknown"),
            )
        })?;
        let (bottom, top) = (dimension.min_y, dimension.min_y + dimension.height - 1);
        region.min[1] = region.min[1].clamp(bottom, top);
        region.max[1] = region.max[1].clamp(bottom, top);
        let capture = super::dispatch!(&self.adapter, a => CoreOps::capture(a, region).await)?;
        if capture.player.session != player.session {
            return Err(Error::new(
                ErrorKind::State,
                anyhow::anyhow!("world changed during raycast"),
            ));
        }
        let cells: std::collections::HashMap<[i32; 3], &Option<NativeBlockState>> = capture
            .world
            .blocks
            .iter()
            .map(|block| (block.position, &block.state))
            .collect();
        let air = NativeBlockState {
            name: "minecraft:air".into(),
            properties: Default::default(),
        };
        let lookup = |p: [i32; 3]| {
            if p[1] < bottom || p[1] > top {
                return Some(Some(air.clone()));
            }
            cells.get(&p).map(|state| (*state).clone())
        };
        let result = raycast(self.version(), origin, direction, max_distance, lookup)?;
        Ok(BlockRaycastObservation {
            session: capture.player.session,
            receive_sequence: capture.player.receive_sequence,
            result,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn state(name: &str) -> NativeBlockState {
        NativeBlockState {
            name: name.into(),
            properties: BTreeMap::new(),
        }
    }

    fn world(
        stone_at: [i32; 3],
        unloaded: Option<[i32; 3]>,
    ) -> impl Fn([i32; 3]) -> Option<Option<NativeBlockState>> {
        move |p| {
            if Some(p) == unloaded {
                Some(None)
            } else if p == stone_at {
                Some(Some(state("minecraft:stone")))
            } else {
                Some(Some(state("minecraft:air")))
            }
        }
    }

    #[test]
    fn rays_hit_full_cubes_on_the_entry_face_on_both_versions() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let hit = raycast(
                version,
                [0.5, 0.5, 0.5],
                [1.0, 0.0, 0.0],
                10.0,
                world([3, 0, 0], None),
            )
            .unwrap();
            match hit {
                BlockRaycast::Hit {
                    position,
                    face,
                    distance,
                    ..
                } => {
                    assert_eq!(position, [3, 0, 0]);
                    assert_eq!(face, BlockFace::West);
                    assert!((distance - 2.5).abs() < 1e-9);
                }
                other => panic!("{version:?}: {other:?}"),
            }
            let down = raycast(
                version,
                [0.5, 5.5, 0.5],
                [0.0, -1.0, 0.0],
                10.0,
                world([0, 1, 0], None),
            )
            .unwrap();
            assert!(matches!(
                down,
                BlockRaycast::Hit {
                    face: BlockFace::Up,
                    ..
                }
            ));
        }
    }

    #[test]
    fn unloaded_cells_stop_the_ray_and_misses_require_a_loaded_path() {
        let v = MinecraftVersion::Java1_21_11;
        let blocked = raycast(
            v,
            [0.5, 0.5, 0.5],
            [1.0, 0.0, 0.0],
            10.0,
            world([5, 0, 0], Some([2, 0, 0])),
        )
        .unwrap();
        assert_eq!(
            blocked,
            BlockRaycast::Unloaded {
                position: [2, 0, 0]
            }
        );
        let miss = raycast(
            v,
            [0.5, 0.5, 0.5],
            [1.0, 0.0, 0.0],
            2.0,
            world([5, 0, 0], None),
        )
        .unwrap();
        assert_eq!(miss, BlockRaycast::Miss);
    }

    #[test]
    fn partial_shapes_use_native_collision_boxes() {
        let mut slab = state("minecraft:oak_slab");
        slab.properties.insert("type".into(), "bottom".into());
        slab.properties.insert("waterlogged".into(), "false".into());
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let slab = slab.clone();
            let cells = move |p: [i32; 3]| {
                Some(Some(if p == [0, 0, 0] {
                    slab.clone()
                } else {
                    state("minecraft:air")
                }))
            };
            // Passing over the bottom slab's top half misses it; going down hits y=0.5.
            let over = raycast(version, [-2.0, 0.75, 0.5], [1.0, 0.0, 0.0], 4.0, &cells).unwrap();
            assert_eq!(over, BlockRaycast::Miss, "{version:?}");
            let down = raycast(version, [0.5, 3.0, 0.5], [0.0, -1.0, 0.0], 4.0, &cells).unwrap();
            match down {
                BlockRaycast::Hit { point, face, .. } => {
                    assert!((point[1] - 0.5).abs() < 1e-9, "{version:?}");
                    assert_eq!(face, BlockFace::Up);
                }
                other => panic!("{version:?}: {other:?}"),
            }
        }
    }
}
