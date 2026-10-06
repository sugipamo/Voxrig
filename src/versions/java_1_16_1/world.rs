//! Loaded chunk storage, block queries, lighting, collision, and raycasts.

use crate::versions::java_1_16_1::{
    collision::{block_name, shapes_for},
    interaction::{BlockFace, BlockPos, BlockRaycastHit},
    physics::{Aabb, Vec3},
    protocol::get_varint,
};
use anyhow::{Context, Result, bail};
use byteorder::{BigEndian, ReadBytesExt};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
    io::{Cursor, Read},
    sync::{Arc, Mutex as StdMutex, OnceLock, Weak},
};

type ChunkResourceIndex = HashMap<i32, BTreeSet<(i32, i32, i32)>>;

pub(crate) struct World {
    chunks: HashMap<(i32, i32), Chunk>,
    resource_index: HashMap<(i32, i32), ChunkResourceIndex>,
    resource_index_entries: usize,
    resource_index_incomplete_chunks: BTreeSet<(i32, i32)>,
    storage: Arc<SharedChunkStorage>,
}

impl Default for World {
    fn default() -> Self {
        Self::with_storage(Arc::new(SharedChunkStorage::default()))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
/// State and protocol data represented by `ChunkStorageStats`.
pub struct ChunkStorageStats {
    /// The `hash_buckets` value.
    pub hash_buckets: usize,
    /// The `live_sections` value.
    pub live_sections: usize,
}

/// Deduplicates immutable chunk-section buffers across clients.
/// Block updates use `Arc::make_mut`, so one Bot never mutates another Bot's view.
#[derive(Default)]
pub struct SharedChunkStorage {
    inner: StdMutex<SharedChunkStorageInner>,
}

#[derive(Default)]
struct SharedChunkStorageInner {
    sections: HashMap<u64, Vec<Weak<[i32; 4096]>>>,
    intern_calls: usize,
}

impl SharedChunkStorage {
    fn intern_section(&self, values: [i32; 4096]) -> Arc<[i32; 4096]> {
        let mut hasher = DefaultHasher::new();
        values.hash(&mut hasher);
        let hash = hasher.finish();
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        inner.intern_calls = inner.intern_calls.wrapping_add(1);
        if inner.intern_calls % 1024 == 0 {
            prune_dead_sections(&mut inner.sections);
        }
        let bucket = inner.sections.entry(hash).or_default();
        bucket.retain(|candidate| candidate.strong_count() > 0);
        if let Some(existing) = bucket
            .iter()
            .filter_map(Weak::upgrade)
            .find(|candidate| candidate.as_ref() == &values)
        {
            return existing;
        }
        let values = Arc::new(values);
        bucket.push(Arc::downgrade(&values));
        values
    }

    /// Performs the `stats` operation.
    pub fn stats(&self) -> ChunkStorageStats {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        prune_dead_sections(&mut inner.sections);
        ChunkStorageStats {
            hash_buckets: inner.sections.len(),
            live_sections: inner.sections.values().map(Vec::len).sum(),
        }
    }
}

fn prune_dead_sections(sections: &mut HashMap<u64, Vec<Weak<[i32; 4096]>>>) {
    sections.retain(|_, bucket| {
        bucket.retain(|candidate| candidate.strong_count() > 0);
        !bucket.is_empty()
    });
}

#[derive(Default)]
struct Chunk {
    terrain_received: bool,
    sections: HashMap<i32, Arc<[i32; 4096]>>,
    biomes: Option<Arc<[i32; 1024]>>,
    heightmaps_nbt: Arc<[u8]>,
    block_entities_nbt: Vec<Arc<[u8]>>,
    sky_light: HashMap<i32, Arc<[u8; 2048]>>,
    block_light: HashMap<i32, Arc<[u8; 2048]>>,
    block_entities: HashMap<BlockPos, BlockEntityData>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
/// State and protocol data represented by `ChunkPos`.
pub struct ChunkPos {
    /// The `x` value.
    pub x: i32,
    /// The `z` value.
    pub z: i32,
}

/// A low-copy view of a decoded chunk. Cloning shares section and NBT buffers.
#[derive(Clone, Debug, PartialEq)]
pub struct ChunkSnapshot {
    /// The `position` value.
    pub position: ChunkPos,
    /// The `sections` value.
    pub sections: HashMap<i32, Arc<[i32; 4096]>>,
    /// The `biomes` value.
    pub biomes: Option<Arc<[i32; 1024]>>,
    /// The `heightmaps_nbt` value.
    pub heightmaps_nbt: Arc<[u8]>,
    /// The `block_entities_nbt` value.
    pub block_entities_nbt: Vec<Arc<[u8]>>,
    /// The `sky_light` value.
    pub sky_light: HashMap<i32, Arc<[u8; 2048]>>,
    /// The `block_light` value.
    pub block_light: HashMap<i32, Arc<[u8; 2048]>>,
    /// The `block_entities` value.
    pub block_entities: HashMap<BlockPos, BlockEntityData>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// State and protocol data represented by `BlockEntityData`.
pub struct BlockEntityData {
    /// The `position` value.
    pub position: BlockPos,
    /// The `action` value.
    pub action: u8,
    /// Complete optional-NBT payload including its root tag byte.
    pub nbt: Arc<[u8]>,
}

impl ChunkSnapshot {
    /// Performs the `block` operation.
    pub fn block(&self, local_x: u8, y: i32, local_z: u8) -> Option<i32> {
        if local_x > 15 || local_z > 15 || !(0..=255).contains(&y) {
            return None;
        }
        let Some(section) = self.sections.get(&y.div_euclid(16)) else {
            return Some(0);
        };
        let index =
            (y.rem_euclid(16) * 256 + i32::from(local_z) * 16 + i32::from(local_x)) as usize;
        Some(section[index])
    }

    /// Performs the `biome` operation.
    pub fn biome(&self, quart_x: u8, quart_y: u8, quart_z: u8) -> Option<i32> {
        if quart_x > 3 || quart_y > 63 || quart_z > 3 {
            return None;
        }
        let index = usize::from(quart_y) * 16 + usize::from(quart_z) * 4 + usize::from(quart_x);
        self.biomes.as_ref().map(|biomes| biomes[index])
    }

    /// Performs the `sky_light` operation.
    pub fn sky_light(&self, local_x: u8, y: i32, local_z: u8) -> Option<u8> {
        light_value(&self.sky_light, local_x, y, local_z)
    }

    /// Performs the `block_light` operation.
    pub fn block_light(&self, local_x: u8, y: i32, local_z: u8) -> Option<u8> {
        light_value(&self.block_light, local_x, y, local_z)
    }
}

fn light_value(
    sections: &HashMap<i32, Arc<[u8; 2048]>>,
    local_x: u8,
    y: i32,
    local_z: u8,
) -> Option<u8> {
    if local_x > 15 || local_z > 15 || !(0..=255).contains(&y) {
        return None;
    }
    let values = sections.get(&y.div_euclid(16))?;
    let index = (y.rem_euclid(16) * 256 + i32::from(local_z) * 16 + i32::from(local_x)) as usize;
    let packed = values[index / 2];
    Some(if index % 2 == 0 {
        packed & 0x0f
    } else {
        packed >> 4
    })
}

#[derive(Clone, Debug, PartialEq)]
/// State and protocol data represented by `BlockObservation`.
pub struct BlockObservation {
    /// The `x` value.
    pub x: i32,
    /// The `y` value.
    pub y: i32,
    /// The `z` value.
    pub z: i32,
    /// The `state_id` value.
    pub state_id: Option<i32>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// State and protocol data represented by `BlockRegion`.
pub struct BlockRegion {
    /// The `min` value.
    pub min: BlockPos,
    /// The `max` value.
    pub max: BlockPos,
}

impl BlockRegion {
    /// Performs the `new` operation.
    pub fn new(a: BlockPos, b: BlockPos) -> Self {
        Self {
            min: BlockPos {
                x: a.x.min(b.x),
                y: a.y.min(b.y),
                z: a.z.min(b.z),
            },
            max: BlockPos {
                x: a.x.max(b.x),
                y: a.y.max(b.y),
                z: a.z.max(b.z),
            },
        }
    }
    fn volume(self) -> Option<usize> {
        let x = i64::from(self.max.x) - i64::from(self.min.x) + 1;
        let y = i64::from(self.max.y) - i64::from(self.min.y) + 1;
        let z = i64::from(self.max.z) - i64::from(self.min.z) + 1;
        usize::try_from(x.checked_mul(y)?.checked_mul(z)?).ok()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Possible values represented by `FluidKind`.
pub enum FluidKind {
    /// The `Water` variant.
    Water,
    /// The `Lava` variant.
    Lava,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
/// State and protocol data represented by `EnvironmentState`.
pub struct EnvironmentState {
    /// The `fluid` value.
    pub fluid: Option<FluidKind>,
    /// The `eyes_submerged` value.
    pub eyes_submerged: bool,
    /// The `climbable` value.
    pub climbable: bool,
    /// The `cobweb` value.
    pub cobweb: bool,
    /// The `berry_bush` value.
    pub berry_bush: bool,
    /// The `bubble_push` value.
    pub bubble_push: i8,
    /// The `supporting_block` value.
    pub supporting_block: Option<BlockPos>,
    /// The `supporting_state_id` value.
    pub supporting_state_id: Option<i32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Fluid {
    Water,
    Lava,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ResourceIndexQuery {
    Complete {
        candidates: Vec<BlockPos>,
        omitted: u32,
    },
    Unsupported,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ContactEffects {
    pub cobweb: bool,
    pub berry_bush: bool,
    pub bubble_push: i8,
}

impl World {
    pub(crate) fn with_storage(storage: Arc<SharedChunkStorage>) -> Self {
        Self {
            chunks: HashMap::new(),
            resource_index: HashMap::new(),
            resource_index_entries: 0,
            resource_index_incomplete_chunks: BTreeSet::new(),
            storage,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.chunks.clear();
        self.resource_index.clear();
        self.resource_index_entries = 0;
        self.resource_index_incomplete_chunks.clear();
    }

    pub(crate) fn apply_explosion_blocks(&mut self, positions: &[BlockPos]) {
        for position in positions {
            self.set_block(position.x, position.y, position.z, 0);
        }
    }
    pub(crate) fn environment_state(&self, player: Aabb) -> EnvironmentState {
        let fluid = self.fluid_intersecting(player).map(|fluid| match fluid {
            Fluid::Water => FluidKind::Water,
            Fluid::Lava => FluidKind::Lava,
        });
        let eye = Aabb {
            min_x: (player.min_x + player.max_x) * 0.5 - 0.01,
            min_y: player.min_y + 1.61,
            min_z: (player.min_z + player.max_z) * 0.5 - 0.01,
            max_x: (player.min_x + player.max_x) * 0.5 + 0.01,
            max_y: player.min_y + 1.63,
            max_z: (player.min_z + player.max_z) * 0.5 + 0.01,
        };
        let effects = self.contact_effects(player);
        let supporting_block = BlockPos {
            x: ((player.min_x + player.max_x) * 0.5).floor() as i32,
            y: (player.min_y - 0.01).floor() as i32,
            z: ((player.min_z + player.max_z) * 0.5).floor() as i32,
        };
        let supporting_state_id = self
            .block(supporting_block.x, supporting_block.y, supporting_block.z)
            .filter(|state| *state != 0);
        EnvironmentState {
            fluid,
            eyes_submerged: self.fluid_intersecting(eye).is_some(),
            climbable: self.is_climbable(player),
            cobweb: effects.cobweb,
            berry_bush: effects.berry_bush,
            bubble_push: effects.bubble_push,
            supporting_block: supporting_state_id.map(|_| supporting_block),
            supporting_state_id,
        }
    }
    pub(crate) fn loaded_chunks(&self) -> Vec<ChunkPos> {
        self.chunks
            .keys()
            .map(|&(x, z)| ChunkPos { x, z })
            .collect()
    }

    pub(crate) fn loaded_geometry_parts(
        &self,
    ) -> (Vec<ChunkPos>, Vec<crate::LoadedGeometrySection>) {
        let mut loaded_chunks = self
            .chunks
            .keys()
            .map(|&(x, z)| ChunkPos { x, z })
            .collect::<Vec<_>>();
        loaded_chunks.sort();
        let mut sections = self
            .chunks
            .iter()
            .flat_map(|(&(x, z), chunk)| {
                chunk.sections.iter().map(move |(&section_y, state_ids)| {
                    crate::LoadedGeometrySection {
                        chunk: ChunkPos { x, z },
                        section_y,
                        state_ids: Arc::clone(state_ids),
                    }
                })
            })
            .collect::<Vec<_>>();
        sections.sort_by_key(|section| (section.chunk, section.section_y));
        (loaded_chunks, sections)
    }
    pub(crate) fn query_blocks(
        &self,
        region: BlockRegion,
        state_ids: &[i32],
        limit: usize,
    ) -> Result<Vec<BlockPos>> {
        let volume = region.volume().context("block region volume overflow")?;
        if volume > 1_048_576 {
            bail!("block query region exceeds 1,048,576 cells");
        }
        if limit == 0 {
            return Ok(Vec::new());
        }
        let states: std::collections::HashSet<_> = state_ids.iter().copied().collect();
        let mut out = Vec::new();
        for y in region.min.y..=region.max.y {
            for z in region.min.z..=region.max.z {
                for x in region.min.x..=region.max.x {
                    if self
                        .block(x, y, z)
                        .is_some_and(|state| states.contains(&state))
                    {
                        out.push(BlockPos { x, y, z });
                        if out.len() >= limit {
                            return Ok(out);
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    pub(crate) fn query_resource_index(
        &self,
        region: BlockRegion,
        block_name: &str,
        limit: usize,
    ) -> ResourceIndexQuery {
        if !is_indexed_resource(block_name) {
            return ResourceIndexQuery::Unsupported;
        }
        let min_chunk_x = region.min.x.div_euclid(16);
        let max_chunk_x = region.max.x.div_euclid(16);
        let min_chunk_z = region.min.z.div_euclid(16);
        let max_chunk_z = region.max.z.div_euclid(16);
        if self.resource_index_incomplete_chunks.iter().any(|(x, z)| {
            (min_chunk_x..=max_chunk_x).contains(x) && (min_chunk_z..=max_chunk_z).contains(z)
        }) {
            return ResourceIndexQuery::Unknown;
        }
        let mut matches = BTreeSet::new();
        for ((chunk_x, chunk_z), states) in &self.resource_index {
            if *chunk_x < region.min.x.div_euclid(16)
                || *chunk_x > region.max.x.div_euclid(16)
                || *chunk_z < region.min.z.div_euclid(16)
                || *chunk_z > region.max.z.div_euclid(16)
            {
                continue;
            }
            for (state_id, positions) in states {
                if crate::block_name_from_state(*state_id) != Some(block_name) {
                    continue;
                }
                matches.extend(positions.iter().copied().filter(|(x, y, z)| {
                    (region.min.x..=region.max.x).contains(x)
                        && (region.min.y..=region.max.y).contains(y)
                        && (region.min.z..=region.max.z).contains(z)
                }));
            }
        }
        let total = matches.len();
        let candidates = matches
            .into_iter()
            .take(limit)
            .map(|(x, y, z)| BlockPos { x, y, z })
            .collect::<Vec<_>>();
        let omitted = total.saturating_sub(candidates.len());
        ResourceIndexQuery::Complete {
            candidates,
            omitted: u32::try_from(omitted).unwrap_or(u32::MAX),
        }
    }

    fn remove_resource_index_chunk(&mut self, chunk: (i32, i32)) {
        if let Some(states) = self.resource_index.remove(&chunk) {
            let removed = states.values().map(BTreeSet::len).sum::<usize>();
            self.resource_index_entries = self.resource_index_entries.saturating_sub(removed);
        }
        self.resource_index_incomplete_chunks.remove(&chunk);
    }

    fn rebuild_resource_index_chunk(&mut self, chunk_x: i32, chunk_z: i32) {
        const MAX_RESOURCE_INDEX_ENTRIES_PER_CHUNK: usize = 8_192;
        const MAX_RESOURCE_INDEX_ENTRIES: usize = 262_144;
        let chunk_key = (chunk_x, chunk_z);
        self.remove_resource_index_chunk(chunk_key);
        let mut indexed = HashMap::<i32, BTreeSet<(i32, i32, i32)>>::new();
        let mut indexed_count = 0_usize;
        if let Some(chunk) = self.chunks.get(&(chunk_x, chunk_z)) {
            for (section_y, states) in &chunk.sections {
                for (index, state_id) in states.iter().copied().enumerate() {
                    if !is_indexed_resource_state(state_id) {
                        continue;
                    }
                    let local_y = i32::try_from(index / 256).unwrap_or(0);
                    let local_z = i32::try_from((index % 256) / 16).unwrap_or(0);
                    let local_x = i32::try_from(index % 16).unwrap_or(0);
                    if indexed.entry(state_id).or_default().insert((
                        chunk_x.saturating_mul(16).saturating_add(local_x),
                        section_y.saturating_mul(16).saturating_add(local_y),
                        chunk_z.saturating_mul(16).saturating_add(local_z),
                    )) {
                        indexed_count = indexed_count.saturating_add(1);
                    }
                    if indexed_count > MAX_RESOURCE_INDEX_ENTRIES_PER_CHUNK {
                        self.resource_index_incomplete_chunks.insert(chunk_key);
                        return;
                    }
                }
            }
        }
        if self.resource_index_entries.saturating_add(indexed_count) > MAX_RESOURCE_INDEX_ENTRIES {
            self.resource_index_incomplete_chunks.insert(chunk_key);
            return;
        }
        if indexed.is_empty() {
            self.resource_index.remove(&chunk_key);
        } else {
            self.resource_index_entries = self.resource_index_entries.saturating_add(indexed_count);
            self.resource_index.insert(chunk_key, indexed);
        }
    }

    fn update_resource_index_block(
        &mut self,
        position: BlockPos,
        old_state_id: i32,
        new_state_id: i32,
    ) {
        let chunk = (position.x.div_euclid(16), position.z.div_euclid(16));
        if self.resource_index_incomplete_chunks.contains(&chunk) {
            return;
        }
        if is_indexed_resource_state(old_state_id) {
            if let Some(states) = self.resource_index.get_mut(&chunk) {
                let mut removed = false;
                if let Some(positions) = states.get_mut(&old_state_id) {
                    removed = positions.remove(&(position.x, position.y, position.z));
                    if positions.is_empty() {
                        states.remove(&old_state_id);
                    }
                }
                let empty = states.is_empty();
                if removed {
                    self.resource_index_entries = self.resource_index_entries.saturating_sub(1);
                }
                if empty {
                    self.resource_index.remove(&chunk);
                }
            }
        }
        if (0..=255).contains(&position.y) && is_indexed_resource_state(new_state_id) {
            let chunk_entries = self
                .resource_index
                .get(&chunk)
                .map(|states| states.values().map(BTreeSet::len).sum::<usize>())
                .unwrap_or(0);
            if self.resource_index_entries >= 262_144 || chunk_entries >= 8_192 {
                self.remove_resource_index_chunk(chunk);
                self.resource_index_incomplete_chunks.insert(chunk);
                return;
            }
            let inserted = self
                .resource_index
                .entry(chunk)
                .or_default()
                .entry(new_state_id)
                .or_default()
                .insert((position.x, position.y, position.z));
            if inserted {
                self.resource_index_entries = self.resource_index_entries.saturating_add(1);
            }
        }
    }

    pub(crate) fn chunk_snapshot(&self, position: ChunkPos) -> Option<ChunkSnapshot> {
        let chunk = self.chunks.get(&(position.x, position.z))?;
        Some(ChunkSnapshot {
            position,
            sections: chunk.sections.clone(),
            biomes: chunk.biomes.clone(),
            heightmaps_nbt: chunk.heightmaps_nbt.clone(),
            block_entities_nbt: chunk.block_entities_nbt.clone(),
            sky_light: chunk.sky_light.clone(),
            block_light: chunk.block_light.clone(),
            block_entities: chunk.block_entities.clone(),
        })
    }

    pub(crate) fn light_at(&self, x: i32, y: i32, z: i32) -> Option<(u8, u8)> {
        let chunk = self.chunks.get(&(x.div_euclid(16), z.div_euclid(16)))?;
        let local_x = u8::try_from(x.rem_euclid(16)).ok()?;
        let local_z = u8::try_from(z.rem_euclid(16)).ok()?;
        Some((
            light_value(&chunk.block_light, local_x, y, local_z)?,
            light_value(&chunk.sky_light, local_x, y, local_z)?,
        ))
    }
    pub(crate) fn raycast_blocks(
        &self,
        origin: Vec3,
        direction: Vec3,
        max_distance: f64,
    ) -> Option<BlockRaycastHit> {
        self.raycast_blocks_with_target(origin, direction, max_distance, None)
    }

    /// A digging target can have no movement collision (plants/crops).
    /// Use its cell as a target only, while keeping collision-shape occlusion
    /// for intervening blocks and rejecting unloaded parts of the sight line.
    pub(crate) fn block_visible(&self, origin: Vec3, target: BlockPos) -> bool {
        let Some(state) = self.block(target.x, target.y, target.z) else {
            return false;
        };
        if matches!(
            block_name(state),
            None | Some("air" | "cave_air" | "void_air")
        ) {
            return false;
        }
        let direction = Vec3 {
            x: f64::from(target.x) + 0.5 - origin.x,
            y: f64::from(target.y) + 0.5 - origin.y,
            z: f64::from(target.z) + 0.5 - origin.z,
        };
        let distance =
            (direction.x * direction.x + direction.y * direction.y + direction.z * direction.z)
                .sqrt();
        self.raycast_blocks_with_target(origin, direction, distance + 1.0e-7, Some(target))
            .is_some_and(|hit| hit.position == target)
    }

    fn raycast_blocks_with_target(
        &self,
        origin: Vec3,
        direction: Vec3,
        max_distance: f64,
        target: Option<BlockPos>,
    ) -> Option<BlockRaycastHit> {
        let length =
            (direction.x * direction.x + direction.y * direction.y + direction.z * direction.z)
                .sqrt();
        if !length.is_finite()
            || length <= 1.0e-12
            || !origin.x.is_finite()
            || !origin.y.is_finite()
            || !origin.z.is_finite()
            || origin.x.abs() > 30_000_000.0
            || origin.z.abs() > 30_000_000.0
            || origin.y.abs() > 2048.0
            || !max_distance.is_finite()
            || !(0.0..=1024.0).contains(&max_distance)
        {
            return None;
        }
        let direction = Vec3 {
            x: direction.x / length,
            y: direction.y / length,
            z: direction.z / length,
        };
        let mut x = origin.x.floor() as i32;
        let mut y = origin.y.floor() as i32;
        let mut z = origin.z.floor() as i32;
        let (step_x, mut t_max_x, t_delta_x) = dda_axis(origin.x, direction.x);
        let (step_y, mut t_max_y, t_delta_y) = dda_axis(origin.y, direction.y);
        let (step_z, mut t_max_z, t_delta_z) = dda_axis(origin.z, direction.z);
        let max_steps = (max_distance.ceil() as usize).saturating_mul(3) + 4;
        for _ in 0..max_steps {
            let exit_distance = t_max_x.min(t_max_y).min(t_max_z).min(max_distance);
            if let Some(state_id) = self.block(x, y, z) {
                let mut nearest = None;
                let shapes = shapes_for(state_id);
                let target_cell = (shapes.is_empty() && target == Some(BlockPos { x, y, z }))
                    .then(|| Aabb::block(x, y, z));
                for shape in shapes
                    .iter()
                    .map(|shape| shape.at(x, y, z))
                    .chain(target_cell)
                {
                    let Some((distance, face)) = ray_aabb(origin, direction, shape) else {
                        continue;
                    };
                    let is_nearer = match nearest {
                        Some((_, _, nearest_distance)) => distance < nearest_distance,
                        None => true,
                    };
                    if distance <= exit_distance + 1.0e-9 && is_nearer {
                        nearest = Some((face, state_id, distance));
                    }
                }
                if let Some((face, state_id, distance)) = nearest {
                    return Some(BlockRaycastHit {
                        position: BlockPos { x, y, z },
                        state_id,
                        face,
                        point: Vec3 {
                            x: origin.x + direction.x * distance,
                            y: origin.y + direction.y * distance,
                            z: origin.z + direction.z * distance,
                        },
                        distance,
                    });
                }
            }
            if target.is_some() && self.block(x, y, z).is_none() {
                return None;
            }
            let next = t_max_x.min(t_max_y).min(t_max_z);
            if next > max_distance {
                break;
            }
            if (t_max_x - next).abs() <= 1.0e-12 {
                x = x.checked_add(step_x)?;
                t_max_x += t_delta_x;
            }
            if (t_max_y - next).abs() <= 1.0e-12 {
                y = y.checked_add(step_y)?;
                t_max_y += t_delta_y;
            }
            if (t_max_z - next).abs() <= 1.0e-12 {
                z = z.checked_add(step_z)?;
                t_max_z += t_delta_z;
            }
        }
        None
    }

    pub(crate) fn block_below_name(&self, aabb: Aabb) -> Option<&'static str> {
        let x = ((aabb.min_x + aabb.max_x) * 0.5).floor() as i32;
        let y = (aabb.min_y - 0.01).floor() as i32;
        let z = ((aabb.min_z + aabb.max_z) * 0.5).floor() as i32;
        block_name(self.block(x, y, z)?)
    }

    pub(crate) fn contact_effects(&self, aabb: Aabb) -> ContactEffects {
        let mut effects = ContactEffects::default();
        for y in aabb.min_y.floor() as i32..aabb.max_y.ceil() as i32 {
            for z in aabb.min_z.floor() as i32..aabb.max_z.ceil() as i32 {
                for x in aabb.min_x.floor() as i32..aabb.max_x.ceil() as i32 {
                    let Some(state) = self.block(x, y, z) else {
                        continue;
                    };
                    match block_name(state) {
                        Some("cobweb") => effects.cobweb = true,
                        Some("sweet_berry_bush") => effects.berry_bush = true,
                        Some("bubble_column") => {
                            effects.bubble_push = if state == 9668 { 1 } else { -1 };
                        }
                        _ => {}
                    }
                }
            }
        }
        effects
    }

    pub(crate) fn is_climbable(&self, aabb: Aabb) -> bool {
        let min_x = aabb.min_x.floor() as i32;
        let max_x = aabb.max_x.ceil() as i32;
        let body_min_y = aabb.min_y.floor() as i32;
        let min_y = body_min_y - 1;
        let max_y = aabb.max_y.ceil() as i32;
        let min_z = aabb.min_z.floor() as i32;
        let max_z = aabb.max_z.ceil() as i32;
        for y in min_y..max_y {
            for z in min_z..max_z {
                for x in min_x..max_x {
                    let Some(state) = self.block(x, y, z) else {
                        continue;
                    };
                    let name = block_name(state);
                    if (y >= body_min_y && matches!(name, Some("ladder" | "vine")))
                        || name == Some("scaffolding")
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    pub(crate) fn fluid_intersecting(&self, aabb: Aabb) -> Option<Fluid> {
        let min_x = aabb.min_x.floor() as i32;
        let max_x = aabb.max_x.ceil() as i32;
        let min_y = aabb.min_y.floor() as i32;
        let max_y = aabb.max_y.ceil() as i32;
        let min_z = aabb.min_z.floor() as i32;
        let max_z = aabb.max_z.ceil() as i32;
        let mut found = None;
        for y in min_y..max_y {
            for z in min_z..max_z {
                for x in min_x..max_x {
                    let state = self.block(x, y, z)?;
                    let fluid = match block_name(state) {
                        Some("water") | Some("bubble_column") => Fluid::Water,
                        Some("lava") => Fluid::Lava,
                        _ => continue,
                    };
                    let height = self.fluid_surface_height(x, y, z, fluid)?;
                    if aabb.min_y < f64::from(y) + height {
                        if fluid == Fluid::Lava {
                            return Some(Fluid::Lava);
                        }
                        found = Some(fluid);
                    }
                }
            }
        }
        found
    }

    /// Maximum observed fluid depth above the player's feet. Vanilla uses
    /// this to distinguish a grounded jump in shallow water from swimming.
    pub(crate) fn fluid_depth(&self, aabb: Aabb, fluid: Fluid) -> Option<f64> {
        let mut depth: f64 = 0.0;
        for y in aabb.min_y.floor() as i32..aabb.max_y.ceil() as i32 {
            for z in aabb.min_z.floor() as i32..aabb.max_z.ceil() as i32 {
                for x in aabb.min_x.floor() as i32..aabb.max_x.ceil() as i32 {
                    self.block(x, y, z)?;
                    if let Some(height) = self.fluid_surface_height(x, y, z, fluid) {
                        let surface = y as f64 + height;
                        depth = depth.max(surface - aabb.min_y);
                    }
                }
            }
        }
        Some(depth)
    }

    pub(crate) fn swimming_jump(&self, aabb: Aabb, fluid: Fluid, grounded: bool) -> bool {
        !grounded
            || self
                .fluid_depth(aabb, fluid)
                .is_none_or(|depth| depth > 0.4)
    }

    fn fluid_surface_height(&self, x: i32, y: i32, z: i32, fluid: Fluid) -> Option<f64> {
        let level = self.fluid_level(x, y, z, fluid)?;
        Some(if self.fluid_level(x, y + 1, z, fluid).is_some() {
            1.0
        } else {
            (8.0 - level) / 9.0
        })
    }

    /// Vanilla's fluid wall exit impulse, after movement and fluid drag.
    /// Unknown, solid, or still submerged escape space cannot authorize it.
    pub(crate) fn fluid_exit_boost(
        &self,
        moved: Aabb,
        velocity: Vec3,
        vertical_displacement: f64,
        fluid: Option<Fluid>,
        collided_horizontal: bool,
    ) -> Option<f64> {
        if fluid.is_none() || !collided_horizontal {
            return None;
        }
        let escape = moved.offset(Vec3 {
            x: velocity.x,
            y: velocity.y + 0.6 - vertical_displacement,
            z: velocity.z,
        });
        for y in (escape.min_y.floor() as i32 - 1)..escape.max_y.ceil() as i32 {
            for z in escape.min_z.floor() as i32..escape.max_z.ceil() as i32 {
                for x in escape.min_x.floor() as i32..escape.max_x.ceil() as i32 {
                    let state = self.block(x, y, z)?;
                    // Entity.isFree uses containsAnyLiquid: even the empty
                    // upper portion of a flowing-fluid cell blocks the probe.
                    if y >= escape.min_y.floor() as i32
                        && matches!(block_name(state), Some("water" | "lava" | "bubble_column"))
                    {
                        return None;
                    }
                    for shape in shapes_for(state) {
                        let b = shape.at(x, y, z);
                        if escape.min_x < b.max_x
                            && escape.max_x > b.min_x
                            && escape.min_y < b.max_y
                            && escape.max_y > b.min_y
                            && escape.min_z < b.max_z
                            && escape.max_z > b.min_z
                        {
                            return None;
                        }
                    }
                }
            }
        }
        Some(0.3)
    }

    pub(crate) fn fluid_flow(&self, aabb: Aabb, fluid: Fluid) -> Vec3 {
        let x = ((aabb.min_x + aabb.max_x) * 0.5).floor() as i32;
        let y = aabb.min_y.floor() as i32;
        let z = ((aabb.min_z + aabb.max_z) * 0.5).floor() as i32;
        let Some(level) = self.fluid_level(x, y, z, fluid) else {
            return Vec3::default();
        };
        let mut flow = Vec3::default();
        for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let neighbor = self.fluid_level(x + dx, y, z + dz, fluid);
            let difference = match neighbor {
                Some(other) => other - level,
                None if self.block(x + dx, y, z + dz) == Some(0) => 8.0 - level,
                None => continue,
            };
            flow.x += f64::from(dx) * difference;
            flow.z += f64::from(dz) * difference;
        }
        let length = flow.x.hypot(flow.z);
        if length > 1.0e-9 {
            flow.x /= length;
            flow.z /= length;
        }
        flow
    }

    fn fluid_level(&self, x: i32, y: i32, z: i32, fluid: Fluid) -> Option<f64> {
        let state = self.block(x, y, z)?;
        let level = match (fluid, block_name(state)) {
            (Fluid::Water, Some("water")) => state - 34,
            (Fluid::Water, Some("bubble_column")) => 0,
            (Fluid::Lava, Some("lava")) => state - 50,
            _ => return None,
        };
        Some(f64::from(if level >= 8 { 0 } else { level }))
    }

    pub(crate) fn has_chunk_at(&self, x: i32, z: i32) -> bool {
        self.chunks
            .contains_key(&(x.div_euclid(16), z.div_euclid(16)))
    }

    pub(crate) fn block(&self, x: i32, y: i32, z: i32) -> Option<i32> {
        let chunk = self.chunks.get(&(x.div_euclid(16), z.div_euclid(16)))?;
        if !(0..=255).contains(&y) {
            return Some(0);
        }
        let Some(section) = chunk.sections.get(&y.div_euclid(16)) else {
            return Some(0);
        };
        let i = ((y.rem_euclid(16) * 256) + (z.rem_euclid(16) * 16) + x.rem_euclid(16)) as usize;
        Some(section[i])
    }

    pub(crate) fn collide(&self, aabb: Aabb, requested: Vec3) -> Option<(Aabb, Vec3)> {
        let query = aabb.expand(requested);
        let min_x = query.min_x.floor() as i32;
        let max_x = query.max_x.ceil() as i32;
        // Fence and wall collision boxes extend up to 0.5 block above their
        // owning block, so the scan must include the block below the query.
        let min_y = query.min_y.floor() as i32 - 1;
        let max_y = query.max_y.ceil() as i32;
        let min_z = query.min_z.floor() as i32;
        let max_z = query.max_z.ceil() as i32;
        let mut obstacles = Vec::new();
        for y in min_y..max_y {
            for z in min_z..max_z {
                for x in min_x..max_x {
                    let state = self.block(x, y, z)?;
                    let scaffolding = block_name(state) == Some("scaffolding");
                    obstacles.extend(
                        shapes_for(state)
                            .iter()
                            .map(|shape| (shape.at(x, y, z), scaffolding)),
                    );
                }
            }
        }
        let mut moved = aabb;
        let mut dy = requested.y;
        for (obstacle, scaffolding) in &obstacles {
            if !scaffolding || dy <= 0.0 {
                dy = moved.clip_y(*obstacle, dy);
            }
        }
        moved = moved.offset(Vec3 {
            x: 0.0,
            y: dy,
            z: 0.0,
        });
        let mut dx = requested.x;
        for (obstacle, scaffolding) in &obstacles {
            if !scaffolding {
                dx = moved.clip_x(*obstacle, dx);
            }
        }
        moved = moved.offset(Vec3 {
            x: dx,
            y: 0.0,
            z: 0.0,
        });
        let mut dz = requested.z;
        for (obstacle, scaffolding) in &obstacles {
            if !scaffolding {
                dz = moved.clip_z(*obstacle, dz);
            }
        }
        moved = moved.offset(Vec3 {
            x: 0.0,
            y: 0.0,
            z: dz,
        });
        Some((
            moved,
            Vec3 {
                x: dx,
                y: dy,
                z: dz,
            },
        ))
    }

    pub(crate) fn collide_with_step(
        &self,
        aabb: Aabb,
        requested: Vec3,
        on_ground: bool,
        step_height: f64,
    ) -> Option<(Aabb, Vec3)> {
        let normal = self.collide(aabb, requested)?;
        let blocked =
            (normal.1.x - requested.x).abs() > 1.0e-9 || (normal.1.z - requested.z).abs() > 1.0e-9;
        if !on_ground || !blocked {
            return Some(normal);
        }
        let (raised, up) = self.collide(
            aabb,
            Vec3 {
                x: requested.x,
                y: step_height,
                z: requested.z,
            },
        )?;
        let (stepped, down) = self.collide(
            raised,
            Vec3 {
                x: 0.0,
                y: requested.y.min(0.0) - up.y,
                z: 0.0,
            },
        )?;
        let step = Vec3 {
            x: up.x,
            y: up.y + down.y,
            z: up.z,
        };
        if step.x * step.x + step.z * step.z > normal.1.x * normal.1.x + normal.1.z * normal.1.z {
            Some((stepped, step))
        } else {
            Some(normal)
        }
    }

    pub(crate) fn limit_sneak_movement(
        &self,
        aabb: Aabb,
        mut dx: f64,
        mut dz: f64,
    ) -> Option<(f64, f64)> {
        const STEP: f64 = 0.05;
        while dx.abs() > 1.0e-9 && !self.has_support(aabb, dx, 0.0)? {
            dx = reduce_toward_zero(dx, STEP);
        }
        while dz.abs() > 1.0e-9 && !self.has_support(aabb, 0.0, dz)? {
            dz = reduce_toward_zero(dz, STEP);
        }
        while dx.abs() > 1.0e-9 && dz.abs() > 1.0e-9 && !self.has_support(aabb, dx, dz)? {
            dx = reduce_toward_zero(dx, STEP);
            dz = reduce_toward_zero(dz, STEP);
        }
        Some((dx, dz))
    }

    fn has_support(&self, aabb: Aabb, dx: f64, dz: f64) -> Option<bool> {
        let shifted = aabb.offset(Vec3 {
            x: dx,
            y: 0.0,
            z: dz,
        });
        let (_, down) = self.collide(
            shifted,
            Vec3 {
                x: 0.0,
                y: -0.6,
                z: 0.0,
            },
        )?;
        Some(down.y > -0.6 + 1.0e-9)
    }

    pub(crate) fn observe_cube(
        &self,
        x: i32,
        y: i32,
        z: i32,
        radius: i32,
    ) -> Vec<BlockObservation> {
        let mut out = Vec::new();
        for by in y - radius..=y + radius {
            for bz in z - radius..=z + radius {
                for bx in x - radius..=x + radius {
                    out.push(BlockObservation {
                        x: bx,
                        y: by,
                        z: bz,
                        state_id: self.block(bx, by, bz),
                    });
                }
            }
        }
        out
    }

    pub(crate) fn apply_chunk(&mut self, payload: &[u8], max_chunks: usize) -> Result<(i32, i32)> {
        let mut c = Cursor::new(payload);
        let x = c.read_i32::<BigEndian>()?;
        let z = c.read_i32::<BigEndian>()?;
        if !self.chunks.contains_key(&(x, z)) && self.chunks.len() >= max_chunks {
            bail!("chunk cache limit of {max_chunks} exceeded");
        }
        let ground_up = c.read_u8()? != 0;
        let _ignore_old = c.read_u8()? != 0;
        let bitmap = read_vi(&mut c)?;
        let heightmap_start = c.position() as usize;
        skip_nbt(&mut c)?;
        let heightmap_end = c.position() as usize;
        let heightmaps_nbt = Arc::from(payload[heightmap_start..heightmap_end].to_vec());
        let biomes = if ground_up {
            let mut values = [0i32; 1024];
            for value in &mut values {
                *value = c.read_i32::<BigEndian>()?;
            }
            Some(Arc::new(values))
        } else {
            None
        };
        let data_len = read_vi(&mut c)?;
        if !(0..=2_097_152).contains(&data_len) {
            bail!("invalid chunk data length {data_len}");
        }
        let mut data = vec![0; data_len as usize];
        c.read_exact(&mut data)?;
        let mut d = Cursor::new(data);
        let mut sections = HashMap::new();
        for sy in 0..16 {
            if bitmap & (1 << sy) == 0 {
                continue;
            }
            let _block_count = d.read_i16::<BigEndian>()?;
            let bits = d.read_u8()?.max(4);
            if bits > 14 {
                bail!("invalid block-state palette width {bits}");
            }
            let palette = if bits <= 8 {
                let n = read_vi(&mut d)?;
                if !(0..=256).contains(&n) {
                    bail!("invalid palette length {n}");
                }
                let mut p = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    p.push(read_vi(&mut d)?);
                }
                Some(p)
            } else {
                None
            };
            let longs_len = read_vi(&mut d)?;
            if !(0..=4096).contains(&longs_len) {
                bail!("invalid packed array length {longs_len}");
            }
            let longs_len = longs_len as usize;
            let values_per_long = 64 / bits as usize;
            let required_longs = 4096_usize.div_ceil(values_per_long);
            if longs_len != required_longs {
                bail!("packed block-state array has {longs_len} longs, expected {required_longs}");
            }
            let mut longs = Vec::with_capacity(longs_len);
            for _ in 0..longs_len {
                longs.push(d.read_u64::<BigEndian>()?);
            }
            let mut states = [0i32; 4096];
            let mask = (1u64 << bits) - 1;
            for (i, state) in states.iter_mut().enumerate() {
                // Since 1.16, entries are padded and never span two longs.
                let li = i / values_per_long;
                let offset = (i % values_per_long) * bits as usize;
                let value = longs.get(li).copied().unwrap_or(0) >> offset;
                let index = (value & mask) as usize;
                *state = decode_palette_state(palette.as_deref(), index)?;
            }
            sections.insert(sy, self.storage.intern_section(states));
        }
        let block_entity_count = read_vi(&mut c)?;
        if !(0..=65_536).contains(&block_entity_count) {
            bail!("invalid block entity count {block_entity_count}");
        }
        let mut block_entities_nbt = Vec::with_capacity(block_entity_count as usize);
        for _ in 0..block_entity_count {
            let start = c.position() as usize;
            skip_nbt(&mut c)?;
            let end = c.position() as usize;
            block_entities_nbt.push(Arc::from(payload[start..end].to_vec()));
        }
        let previous = self.chunks.remove(&(x, z)).unwrap_or_default();
        if !ground_up {
            for (section, states) in &previous.sections {
                sections.entry(*section).or_insert_with(|| states.clone());
            }
        }
        let changed = previous.terrain_received && previous.sections != sections;
        self.chunks.insert(
            (x, z),
            Chunk {
                terrain_received: true,
                sections,
                biomes: biomes.or(previous.biomes),
                heightmaps_nbt,
                block_entities_nbt,
                sky_light: previous.sky_light,
                block_light: previous.block_light,
                block_entities: previous.block_entities,
            },
        );
        if changed {
            self.invalidate_light_neighborhood(x, z);
        }
        self.rebuild_resource_index_chunk(x, z);
        Ok((x, z))
    }

    pub(crate) fn apply_light(&mut self, payload: &[u8], max_chunks: usize) -> Result<(i32, i32)> {
        let mut rest = payload;
        let x = get_varint(&mut rest)?;
        let z = get_varint(&mut rest)?;
        if !self.chunks.contains_key(&(x, z)) && self.chunks.len() >= max_chunks {
            bail!("chunk cache limit of {max_chunks} exceeded by light update");
        }
        let _trust_edges = *rest.first().context("missing trust-edges flag")? != 0;
        rest = &rest[1..];
        let sky_mask = get_varint(&mut rest)? as u32;
        let block_mask = get_varint(&mut rest)? as u32;
        let empty_sky_mask = get_varint(&mut rest)? as u32;
        let empty_block_mask = get_varint(&mut rest)? as u32;
        // Validate both channels before publishing any cached light observation.
        let sky = read_light_mask(sky_mask, empty_sky_mask, &mut rest)?;
        let block = read_light_mask(block_mask, empty_block_mask, &mut rest)?;
        if !rest.is_empty() {
            bail!("trailing light update data");
        }
        let chunk = self.chunks.entry((x, z)).or_default();
        chunk.sky_light.extend(sky);
        chunk.block_light.extend(block);
        Ok((x, z))
    }

    pub(crate) fn apply_block_entity(&mut self, payload: &[u8]) -> Result<BlockEntityData> {
        let mut cursor = Cursor::new(payload);
        let position = BlockPos::unpack(cursor.read_u64::<BigEndian>()?);
        let action = cursor.read_u8()?;
        let nbt = Arc::from(payload[cursor.position() as usize..].to_vec());
        let update = BlockEntityData {
            position,
            action,
            nbt,
        };
        if let Some(chunk) = self
            .chunks
            .get_mut(&(position.x.div_euclid(16), position.z.div_euclid(16)))
        {
            chunk.block_entities.insert(position, update.clone());
        }
        Ok(update)
    }

    pub(crate) fn unload_chunk(&mut self, payload: &[u8]) -> Result<(i32, i32)> {
        let mut c = Cursor::new(payload);
        let x = c.read_i32::<BigEndian>()?;
        let z = c.read_i32::<BigEndian>()?;
        self.chunks.remove(&(x, z));
        self.remove_resource_index_chunk((x, z));
        Ok((x, z))
    }

    pub(crate) fn apply_block_change(&mut self, payload: &[u8]) -> Result<(i32, i32, i32, i32)> {
        let mut c = Cursor::new(payload);
        let (x, y, z) = unpack_position(c.read_u64::<BigEndian>()?);
        let state_id = read_vi(&mut c)?;
        self.set_block(x, y, z, state_id);
        Ok((x, y, z, state_id))
    }

    pub(crate) fn apply_acknowledged_block_state(&mut self, position: BlockPos, state_id: i32) {
        self.set_block(position.x, position.y, position.z, state_id);
    }

    pub(crate) fn apply_multi_block_change(&mut self, payload: &[u8]) -> Result<usize> {
        let mut c = Cursor::new(payload);
        let chunk_x = c.read_i32::<BigEndian>()?;
        let chunk_z = c.read_i32::<BigEndian>()?;
        if !(-1_875_000..=1_875_000).contains(&chunk_x)
            || !(-1_875_000..=1_875_000).contains(&chunk_z)
        {
            bail!("multi block change chunk coordinate is outside the world border");
        }
        let count = read_vi(&mut c)?;
        if !(0..=65_536).contains(&count) {
            bail!("invalid multi block change count {count}");
        }
        for _ in 0..count {
            let horizontal = c.read_u8()?;
            let y = i32::from(c.read_u8()?);
            let state_id = read_vi(&mut c)?;
            let x = chunk_x
                .checked_mul(16)
                .and_then(|base| base.checked_add(i32::from(horizontal >> 4)))
                .context("multi block change x overflow")?;
            let z = chunk_z
                .checked_mul(16)
                .and_then(|base| base.checked_add(i32::from(horizontal & 0x0f)))
                .context("multi block change z overflow")?;
            self.set_block(x, y, z, state_id);
        }
        Ok(count as usize)
    }

    fn set_block(&mut self, x: i32, y: i32, z: i32, state_id: i32) {
        let position = BlockPos { x, y, z };
        let Some(chunk) = self.chunks.get_mut(&(x.div_euclid(16), z.div_euclid(16))) else {
            return;
        };
        let section = chunk
            .sections
            .entry(y.div_euclid(16))
            .or_insert_with(|| Arc::new([0; 4096]));
        let i = ((y.rem_euclid(16) * 256) + (z.rem_euclid(16) * 16) + x.rem_euclid(16)) as usize;
        let old_state_id = Arc::make_mut(section)[i];
        Arc::make_mut(section)[i] = state_id;
        if old_state_id != state_id {
            self.invalidate_light_neighborhood(x.div_euclid(16), z.div_euclid(16));
        }
        self.update_resource_index_block(position, old_state_id, state_id);
    }

    fn invalidate_light_neighborhood(&mut self, x: i32, z: i32) {
        // Block changes can affect propagation across a chunk boundary. Until
        // the server supplies new arrays, cached lighting is unknown, not current.
        for dx in -1..=1 {
            for dz in -1..=1 {
                if let Some(chunk) = self.chunks.get_mut(&(x + dx, z + dz)) {
                    chunk.sky_light.clear();
                    chunk.block_light.clear();
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn set_block_for_test(&mut self, position: BlockPos, state_id: i32) {
        self.set_block(position.x, position.y, position.z, state_id);
    }
}

fn is_indexed_resource(name: &str) -> bool {
    INDEXED_RESOURCES.contains(&name)
}

fn is_indexed_resource_state(state_id: i32) -> bool {
    static STATES: OnceLock<HashSet<i32>> = OnceLock::new();
    STATES
        .get_or_init(|| {
            crate::registry::block_state_ranges_for_names(INDEXED_RESOURCES)
                .into_iter()
                .flat_map(|(min, max)| min..=max)
                .collect()
        })
        .contains(&state_id)
}

const INDEXED_RESOURCES: &[&str] = &[
    "iron_ore",
    "coal_ore",
    "gold_ore",
    "diamond_ore",
    "redstone_ore",
    "lapis_ore",
    "emerald_ore",
    "nether_quartz_ore",
    "ancient_debris",
    "oak_log",
    "birch_log",
    "spruce_log",
    "jungle_log",
    "acacia_log",
    "dark_oak_log",
];

fn read_light_mask(
    present: u32,
    empty: u32,
    rest: &mut &[u8],
) -> Result<HashMap<i32, Arc<[u8; 2048]>>> {
    const SECTION_MASK: u32 = (1 << 18) - 1;
    if (present | empty) & !SECTION_MASK != 0 || present & empty != 0 {
        bail!("invalid or overlapping light section masks");
    }
    let mut target = HashMap::new();
    for bit in 0..18 {
        let section_y = bit - 1;
        if empty & (1 << bit) != 0 {
            target.insert(section_y, Arc::new([0; 2048]));
        }
        if present & (1 << bit) != 0 {
            let length = get_varint(rest)?;
            if length != 2048 {
                bail!("invalid light array length {length}; expected 2048");
            }
            if rest.len() < 2048 {
                bail!("truncated light array");
            }
            let mut values = [0; 2048];
            values.copy_from_slice(&rest[..2048]);
            *rest = &rest[2048..];
            target.insert(section_y, Arc::new(values));
        }
    }
    Ok(target)
}

fn dda_axis(origin: f64, direction: f64) -> (i32, f64, f64) {
    if direction > 1.0e-12 {
        let boundary = origin.floor() + 1.0;
        (1, (boundary - origin) / direction, 1.0 / direction)
    } else if direction < -1.0e-12 {
        let boundary = origin.floor();
        (-1, (origin - boundary) / -direction, 1.0 / -direction)
    } else {
        (0, f64::INFINITY, f64::INFINITY)
    }
}

fn ray_aabb(origin: Vec3, direction: Vec3, aabb: Aabb) -> Option<(f64, BlockFace)> {
    let axes = [
        (
            origin.x,
            direction.x,
            aabb.min_x,
            aabb.max_x,
            BlockFace::West,
            BlockFace::East,
        ),
        (
            origin.y,
            direction.y,
            aabb.min_y,
            aabb.max_y,
            BlockFace::Down,
            BlockFace::Up,
        ),
        (
            origin.z,
            direction.z,
            aabb.min_z,
            aabb.max_z,
            BlockFace::North,
            BlockFace::South,
        ),
    ];
    let mut near = f64::NEG_INFINITY;
    let mut far = f64::INFINITY;
    let mut face = BlockFace::Down;
    for (start, delta, min, max, min_face, max_face) in axes {
        if delta.abs() <= 1.0e-12 {
            if start < min || start > max {
                return None;
            }
            continue;
        }
        let mut first = (min - start) / delta;
        let mut last = (max - start) / delta;
        let entering_face = if first <= last {
            min_face
        } else {
            std::mem::swap(&mut first, &mut last);
            max_face
        };
        if first > near {
            near = first;
            face = entering_face;
        }
        far = far.min(last);
        if near > far {
            return None;
        }
    }
    if far < 0.0 {
        None
    } else {
        Some((near.max(0.0), face))
    }
}

fn reduce_toward_zero(value: f64, amount: f64) -> f64 {
    if value > 0.0 {
        (value - amount).max(0.0)
    } else {
        (value + amount).min(0.0)
    }
}

fn unpack_position(value: u64) -> (i32, i32, i32) {
    let x = ((value as i64) >> 38) as i32;
    let y = ((value & 0xfff) as i32) << 20 >> 20;
    let z = (((value >> 12) & 0x3ff_ffff) as i32) << 6 >> 6;
    (x, y, z)
}

fn read_vi(c: &mut Cursor<impl AsRef<[u8]>>) -> Result<i32> {
    let pos = c.position() as usize;
    let all = c.get_ref().as_ref();
    let mut slice = all.get(pos..).context("cursor past end")?;
    let before = slice.len();
    let v = get_varint(&mut slice)?;
    c.set_position((pos + before - slice.len()) as u64);
    Ok(v)
}

fn decode_palette_state(palette: Option<&[i32]>, index: usize) -> Result<i32> {
    if let Some(palette) = palette {
        palette.get(index).copied().with_context(|| {
            format!(
                "block-state palette index {index} exceeds {} entries",
                palette.len()
            )
        })
    } else {
        i32::try_from(index).context("global block-state ID exceeds i32")
    }
}

pub(crate) fn skip_nbt<R: Read>(r: &mut R) -> Result<()> {
    let mut budget = NbtBudget::default();
    let tag = r.read_u8()?;
    if tag == 0 {
        return Ok(());
    }
    skip_string(r)?;
    skip_tag_payload(r, tag, 0, &mut budget)
}
fn skip_string<R: Read>(r: &mut R) -> Result<()> {
    let n = r.read_u16::<BigEndian>()?;
    discard_bytes(r, n as usize)
}
const MAX_NBT_DEPTH: usize = 64;
const MAX_NBT_NODES: usize = 65_536;
const MAX_NBT_ARRAY_BYTES: usize = 2_097_152;

struct NbtBudget {
    nodes_left: usize,
}

impl Default for NbtBudget {
    fn default() -> Self {
        Self {
            nodes_left: MAX_NBT_NODES,
        }
    }
}

fn skip_tag_payload<R: Read>(
    r: &mut R,
    tag: u8,
    depth: usize,
    budget: &mut NbtBudget,
) -> Result<()> {
    if depth > MAX_NBT_DEPTH {
        bail!("NBT nesting exceeds {MAX_NBT_DEPTH}");
    }
    budget.nodes_left = budget
        .nodes_left
        .checked_sub(1)
        .context("NBT node budget exceeded")?;
    match tag {
        0 => {}
        1 => {
            r.read_i8()?;
        }
        2 => {
            r.read_i16::<BigEndian>()?;
        }
        3 => {
            r.read_i32::<BigEndian>()?;
        }
        4 => {
            r.read_i64::<BigEndian>()?;
        }
        5 => {
            r.read_f32::<BigEndian>()?;
        }
        6 => {
            r.read_f64::<BigEndian>()?;
        }
        7 => {
            let n = r.read_i32::<BigEndian>()?;
            discard(r, n, 1)?;
        }
        8 => skip_string(r)?,
        9 => {
            let t = r.read_u8()?;
            let n = r.read_i32::<BigEndian>()?;
            if n < 0 || n as usize > budget.nodes_left || (t == 0 && n != 0) {
                bail!("invalid NBT list length {n} for tag {t}");
            }
            for _ in 0..n {
                skip_tag_payload(r, t, depth + 1, budget)?;
            }
        }
        10 => loop {
            let t = r.read_u8()?;
            if t == 0 {
                break;
            }
            skip_string(r)?;
            skip_tag_payload(r, t, depth + 1, budget)?;
        },
        11 => {
            let n = r.read_i32::<BigEndian>()?;
            discard(r, n, 4)?;
        }
        12 => {
            let n = r.read_i32::<BigEndian>()?;
            discard(r, n, 8)?;
        }
        _ => bail!("unknown NBT tag {tag}"),
    }
    Ok(())
}
fn discard<R: Read>(r: &mut R, n: i32, size: usize) -> Result<()> {
    if n < 0 {
        bail!("negative NBT array")
    };
    let bytes = (n as usize)
        .checked_mul(size)
        .context("NBT array length overflow")?;
    if bytes > MAX_NBT_ARRAY_BYTES {
        bail!("NBT array exceeds {MAX_NBT_ARRAY_BYTES} bytes");
    }
    discard_bytes(r, bytes)
}

fn discard_bytes<R: Read>(r: &mut R, mut bytes: usize) -> Result<()> {
    let mut buffer = [0_u8; 8192];
    while bytes > 0 {
        let amount = bytes.min(buffer.len());
        r.read_exact(&mut buffer[..amount])?;
        bytes -= amount;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct DifferentialFile {
        fixtures: Vec<DifferentialFixture>,
    }

    #[derive(Deserialize)]
    struct DifferentialFixture {
        name: String,
        trajectory: Vec<DifferentialTick>,
    }

    #[derive(Deserialize)]
    struct DifferentialTick {
        position: [f64; 3],
        velocity: [f64; 3],
        on_ground: bool,
        collided_horizontal: bool,
        collided_vertical: bool,
        fall_distance: f64,
        sprinting: bool,
    }

    fn pack_position(x: i32, y: i32, z: i32) -> u64 {
        ((x as u64 & 0x3ff_ffff) << 38) | ((z as u64 & 0x3ff_ffff) << 12) | (y as u64 & 0xfff)
    }

    fn indexed_state(name: &str) -> i32 {
        crate::registry::block_state_ranges_for_names(&[name])
            .into_iter()
            .next()
            .map(|(min, _)| min)
            .unwrap()
    }

    /// Builds the smallest valid protocol chunk containing one palette state
    /// repeated throughout section 0. The resource index tests use this to
    /// exercise the real chunk-replace path rather than inserting a fixture
    /// directly into the decoded world.
    fn uniform_chunk_packet(x: i32, z: i32, state_id: i32) -> Vec<u8> {
        let mut packet = Vec::new();
        packet.extend(x.to_be_bytes());
        packet.extend(z.to_be_bytes());
        packet.extend([1, 0]);
        crate::protocol::put_varint(&mut packet, 1);
        packet.push(0); // End tag heightmap NBT.
        packet.extend([0; 1024 * 4]);

        let mut section = Vec::new();
        section.extend(0_i16.to_be_bytes());
        section.push(4); // Four bits per palette entry.
        crate::protocol::put_varint(&mut section, 1);
        crate::protocol::put_varint(&mut section, state_id);
        crate::protocol::put_varint(&mut section, 256);
        section.extend([0; 256 * 8]);
        crate::protocol::put_varint(&mut packet, i32::try_from(section.len()).unwrap());
        packet.extend(section);
        crate::protocol::put_varint(&mut packet, 0); // Block entities.
        packet
    }

    fn block_change_packet(position: BlockPos, state_id: i32) -> Vec<u8> {
        let mut packet = pack_position(position.x, position.y, position.z)
            .to_be_bytes()
            .to_vec();
        crate::protocol::put_varint(&mut packet, state_id);
        packet
    }

    fn multi_block_change_packet(chunk_x: i32, chunk_z: i32, updates: &[(u8, u8, i32)]) -> Vec<u8> {
        let mut packet = Vec::new();
        packet.extend(chunk_x.to_be_bytes());
        packet.extend(chunk_z.to_be_bytes());
        crate::protocol::put_varint(&mut packet, i32::try_from(updates.len()).unwrap());
        for &(horizontal, y, state_id) in updates {
            packet.extend([horizontal, y]);
            crate::protocol::put_varint(&mut packet, state_id);
        }
        packet
    }

    fn unload_chunk_packet(chunk_x: i32, chunk_z: i32) -> Vec<u8> {
        [chunk_x.to_be_bytes(), chunk_z.to_be_bytes()].concat()
    }

    fn query_region(min: BlockPos, max: BlockPos) -> BlockRegion {
        BlockRegion::new(min, max)
    }

    #[test]
    fn packed_positions_preserve_signed_coordinates() {
        for position in [(0, 0, 0), (99, 4, -198), (-30_000_000, -64, 30_000_000)] {
            assert_eq!(
                unpack_position(pack_position(position.0, position.1, position.2)),
                position
            );
        }
    }

    #[test]
    fn local_palette_rejects_an_out_of_range_index() {
        assert_eq!(decode_palette_state(Some(&[17, 42]), 1).unwrap(), 42);
        assert!(decode_palette_state(Some(&[17, 42]), 2).is_err());
        assert_eq!(decode_palette_state(None, 42).unwrap(), 42);
    }

    #[test]
    fn multi_block_change_rejects_extreme_chunk_coordinates() {
        let mut packet = Vec::new();
        packet.extend(i32::MIN.to_be_bytes());
        packet.extend(0_i32.to_be_bytes());
        crate::versions::java_1_16_1::protocol::put_varint(&mut packet, 0);
        assert!(World::default().apply_multi_block_change(&packet).is_err());
    }

    #[test]
    fn chunk_limit_rejects_new_chunk_before_mutation() {
        let mut world = World::default();
        let mut packet = Vec::new();
        packet.extend(7_i32.to_be_bytes());
        packet.extend((-3_i32).to_be_bytes());

        assert!(world.apply_chunk(&packet, 0).is_err());
        assert!(world.chunks.is_empty());
    }

    #[test]
    fn block_change_updates_loaded_chunk() {
        let mut world = World::default();
        world.chunks.insert((6, -13), Chunk::default());
        let mut packet = pack_position(99, 4, -198).to_be_bytes().to_vec();
        crate::versions::java_1_16_1::protocol::put_varint(&mut packet, 1);
        assert_eq!(world.apply_block_change(&packet).unwrap(), (99, 4, -198, 1));
        assert_eq!(world.block(99, 4, -198), Some(1));
    }

    #[test]
    fn replacing_a_chunk_builds_a_matching_canonical_resource_index() {
        let state = indexed_state("iron_ore");
        let mut world = World::default();
        world
            .apply_chunk(&uniform_chunk_packet(-1, -1, state), 256)
            .unwrap();
        let region = query_region(
            BlockPos {
                x: -16,
                y: 0,
                z: -16,
            },
            BlockPos { x: -1, y: 0, z: -1 },
        );
        let indexed = world.query_resource_index(region, "iron_ore", 3);
        assert_eq!(
            indexed,
            ResourceIndexQuery::Complete {
                candidates: vec![
                    BlockPos {
                        x: -16,
                        y: 0,
                        z: -16
                    },
                    BlockPos {
                        x: -16,
                        y: 0,
                        z: -15
                    },
                    BlockPos {
                        x: -16,
                        y: 0,
                        z: -14
                    },
                ],
                omitted: 253,
            }
        );
        let mut control = Vec::new();
        for x in region.min.x..=region.max.x {
            for y in region.min.y..=region.max.y {
                for z in region.min.z..=region.max.z {
                    if world.block(x, y, z) == Some(state) {
                        control.push(BlockPos { x, y, z });
                        if control.len() == 3 {
                            break;
                        }
                    }
                }
                if control.len() == 3 {
                    break;
                }
            }
            if control.len() == 3 {
                break;
            }
        }
        assert_eq!(
            indexed,
            ResourceIndexQuery::Complete {
                candidates: control,
                omitted: 253,
            }
        );
    }

    #[test]
    fn single_multi_explosion_and_ack_updates_remove_and_add_index_entries() {
        let iron = indexed_state("iron_ore");
        let stone = indexed_state("stone");
        let mut world = World::default();
        world.chunks.insert((0, 0), Chunk::default());
        let first = BlockPos { x: 1, y: 4, z: 2 };
        let second = BlockPos { x: 3, y: 4, z: 2 };
        let third = BlockPos { x: 5, y: 4, z: 2 };
        world.set_block(first.x, first.y, first.z, iron);
        assert_eq!(
            world.query_resource_index(query_region(first, first), "iron_ore", 4),
            ResourceIndexQuery::Complete {
                candidates: vec![first],
                omitted: 0
            }
        );

        world
            .apply_block_change(&block_change_packet(first, stone))
            .unwrap();
        assert_eq!(
            world.query_resource_index(query_region(first, first), "iron_ore", 4),
            ResourceIndexQuery::Complete {
                candidates: vec![],
                omitted: 0
            }
        );
        world
            .apply_block_change(&block_change_packet(first, iron))
            .unwrap();
        world.set_block(second.x, second.y, second.z, stone);
        world
            .apply_multi_block_change(&multi_block_change_packet(
                0,
                0,
                &[(2 << 4 | 2, 4, stone), (3 << 4 | 2, 4, iron)],
            ))
            .unwrap();
        assert_eq!(
            world.query_resource_index(query_region(first, second), "iron_ore", 4),
            ResourceIndexQuery::Complete {
                candidates: vec![first, second],
                omitted: 0
            }
        );

        world.apply_explosion_blocks(&[first]);
        assert_eq!(
            world.query_resource_index(query_region(first, second), "iron_ore", 4),
            ResourceIndexQuery::Complete {
                candidates: vec![second],
                omitted: 0
            }
        );
        world.apply_acknowledged_block_state(third, iron);
        assert_eq!(
            world.query_resource_index(query_region(first, third), "iron_ore", 4),
            ResourceIndexQuery::Complete {
                candidates: vec![second, third],
                omitted: 0
            }
        );
    }

    #[test]
    fn unload_and_clear_remove_resource_index_state() {
        let iron = indexed_state("iron_ore");
        let mut world = World::default();
        world.chunks.insert((2, -3), Chunk::default());
        world.set_block(32, 2, -48, iron);
        assert!(matches!(world.query_resource_index(
            query_region(BlockPos { x: 32, y: 2, z: -48 }, BlockPos { x: 32, y: 2, z: -48 }),
            "iron_ore", 4), ResourceIndexQuery::Complete { candidates, omitted: 0 }
            if candidates == vec![BlockPos { x: 32, y: 2, z: -48 }]
        ));
        world.unload_chunk(&unload_chunk_packet(2, -3)).unwrap();
        assert!(world.loaded_chunks().is_empty());
        assert_eq!(world.resource_index_entries, 0);
        assert_eq!(
            world.query_resource_index(
                query_region(
                    BlockPos {
                        x: 32,
                        y: 2,
                        z: -48
                    },
                    BlockPos {
                        x: 32,
                        y: 2,
                        z: -48
                    }
                ),
                "iron_ore",
                4
            ),
            ResourceIndexQuery::Complete {
                candidates: vec![],
                omitted: 0
            }
        );
        world.chunks.insert((0, 0), Chunk::default());
        world.set_block(0, 0, 0, iron);
        world.clear();
        assert!(world.chunks.is_empty());
        assert!(world.resource_index.is_empty());
        assert_eq!(world.resource_index_entries, 0);
        assert!(world.resource_index_incomplete_chunks.is_empty());
    }

    #[test]
    fn negative_chunks_and_out_of_range_y_are_handled_without_indexing_invalid_cells() {
        let iron = indexed_state("iron_ore");
        let mut world = World::default();
        world.chunks.insert((-1, -1), Chunk::default());
        let negative = BlockPos { x: -1, y: 0, z: -1 };
        world.set_block(negative.x, negative.y, negative.z, iron);
        world.set_block(-1, -1, -1, iron);
        world.set_block(-1, 256, -1, iron);
        assert_eq!(
            world.query_resource_index(
                query_region(
                    BlockPos {
                        x: -1,
                        y: -1,
                        z: -1
                    },
                    BlockPos {
                        x: -1,
                        y: 256,
                        z: -1
                    }
                ),
                "iron_ore",
                4
            ),
            ResourceIndexQuery::Complete {
                candidates: vec![negative],
                omitted: 0
            }
        );
    }

    #[test]
    fn unsupported_resources_and_capacity_overflow_fail_closed() {
        let iron = indexed_state("iron_ore");
        let mut world = World::default();
        world.chunks.insert((0, 0), Chunk::default());
        assert_eq!(
            world.query_resource_index(
                query_region(BlockPos { x: 0, y: 0, z: 0 }, BlockPos { x: 0, y: 0, z: 0 }),
                "air",
                4
            ),
            ResourceIndexQuery::Unsupported
        );
        assert_eq!(
            world.query_resource_index(
                query_region(BlockPos { x: 0, y: 0, z: 0 }, BlockPos { x: 0, y: 0, z: 0 }),
                "not_a_block",
                4
            ),
            ResourceIndexQuery::Unsupported
        );
        for section_y in 0..3 {
            world
                .chunks
                .get_mut(&(0, 0))
                .unwrap()
                .sections
                .insert(section_y, Arc::new([iron; 4096]));
        }
        world.rebuild_resource_index_chunk(0, 0);
        assert_eq!(
            world.query_resource_index(
                query_region(
                    BlockPos { x: 0, y: 0, z: 0 },
                    BlockPos {
                        x: 15,
                        y: 47,
                        z: 15
                    }
                ),
                "iron_ore",
                4
            ),
            ResourceIndexQuery::Unknown
        );
    }

    #[test]
    fn block_query_filters_state_ids_and_honors_limit() {
        let mut world = World::default();
        world.chunks.insert((0, 0), Chunk::default());
        world.set_block(1, 2, 3, 1);
        world.set_block(2, 2, 3, 1);
        world.set_block(3, 2, 3, 2);
        let found = world
            .query_blocks(
                BlockRegion::new(
                    BlockPos { x: 0, y: 0, z: 0 },
                    BlockPos { x: 15, y: 4, z: 15 },
                ),
                &[1],
                1,
            )
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(world.block(found[0].x, found[0].y, found[0].z), Some(1));
    }

    #[test]
    fn block_visibility_does_not_accept_unloaded_sight_lines_or_air() {
        let mut world = World::default();
        world.chunks.insert((0, 0), Chunk::default());
        world.chunks.insert((2, 0), Chunk::default());
        let target = BlockPos { x: 32, y: 1, z: 0 };
        let wheat = (0..18000)
            .find(|id| crate::block_name_from_state(*id) == Some("wheat"))
            .unwrap();
        world.set_block(target.x, target.y, target.z, wheat);
        let origin = Vec3 {
            x: 15.5,
            y: 1.5,
            z: 0.5,
        };
        assert!(!world.block_visible(origin, target));
        world.chunks.insert((1, 0), Chunk::default());
        assert!(world.block_visible(origin, target));
        assert!(!world.block_visible(origin, BlockPos { x: 16, y: 1, z: 0 }));
        assert!(!world.block_visible(
            Vec3 {
                x: f64::NAN,
                ..origin
            },
            target
        ));
    }

    #[test]
    fn raycast_uses_collision_shapes_and_reports_entering_face() {
        let mut world = World::default();
        world.chunks.insert((0, 0), Chunk::default());
        world.set_block(0, 1, 3, 1);
        let hit = world
            .raycast_blocks(
                Vec3 {
                    x: 0.5,
                    y: 1.5,
                    z: 0.5,
                },
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                },
                5.0,
            )
            .unwrap();
        assert_eq!(hit.position, BlockPos { x: 0, y: 1, z: 3 });
        assert_eq!(hit.face, BlockFace::North);
        assert!((hit.distance - 2.5).abs() < 1.0e-12);
    }

    #[test]
    fn raycast_ignores_air_and_respects_max_distance() {
        let mut world = World::default();
        world.chunks.insert((0, 0), Chunk::default());
        world.set_block(0, 1, 3, 1);
        let origin = Vec3 {
            x: 0.5,
            y: 1.5,
            z: 0.5,
        };
        let direction = Vec3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        };
        assert!(world.raycast_blocks(origin, direction, 2.49).is_none());
        assert!(world.raycast_blocks(origin, Vec3::default(), 5.0).is_none());
    }

    #[test]
    fn full_block_collision_stops_floor_and_wall_motion() {
        let mut world = World::default();
        world.chunks.insert((0, 0), Chunk::default());
        world.set_block(0, 0, 0, 1);
        world.set_block(1, 1, 0, 1);
        let player = Aabb::player(0.5, 1.0, 0.5);
        let (_, actual) = world
            .collide(
                player,
                Vec3 {
                    x: 1.0,
                    y: -0.2,
                    z: 0.0,
                },
            )
            .unwrap();
        assert_eq!(actual.y, 0.0);
        assert!((actual.x - 0.2).abs() < 1.0e-12);
    }

    #[test]
    fn absent_section_in_loaded_chunk_is_air() {
        let mut world = World::default();
        world.chunks.insert((0, 0), Chunk::default());
        assert_eq!(world.block(0, 100, 0), Some(0));
        assert_eq!(world.block(16, 100, 0), None);
    }

    #[test]
    fn chunk_snapshots_share_section_storage_and_expose_biomes() {
        let section = Arc::new([1; 4096]);
        let mut sections = HashMap::new();
        sections.insert(0, section.clone());
        let mut biomes = [0; 1024];
        biomes[3 * 16 + 2 * 4 + 1] = 42;
        let mut world = World::default();
        world.chunks.insert(
            (2, -3),
            Chunk {
                sections,
                biomes: Some(Arc::new(biomes)),
                ..Chunk::default()
            },
        );
        let snapshot = world.chunk_snapshot(ChunkPos { x: 2, z: -3 }).unwrap();
        assert!(Arc::ptr_eq(&section, snapshot.sections.get(&0).unwrap()));
        assert_eq!(snapshot.block(0, 0, 0), Some(1));
        assert_eq!(snapshot.biome(1, 3, 2), Some(42));
    }

    #[test]
    fn shared_storage_deduplicates_equal_sections_and_updates_are_copy_on_write() {
        let storage = SharedChunkStorage::default();
        let first = storage.intern_section([1; 4096]);
        let mut second = storage.intern_section([1; 4096]);
        assert!(Arc::ptr_eq(&first, &second));
        Arc::make_mut(&mut second)[0] = 2;
        assert_eq!(first[0], 1);
        assert_eq!(second[0], 2);
        assert_eq!(storage.stats().live_sections, 1);
    }

    #[test]
    fn nbt_skip_rejects_huge_arrays_and_nonempty_end_lists() {
        let mut huge_array = vec![7, 0, 0];
        huge_array.extend(((MAX_NBT_ARRAY_BYTES + 1) as i32).to_be_bytes());
        assert!(skip_nbt(&mut huge_array.as_slice()).is_err());

        let mut invalid_list = vec![9, 0, 0, 0];
        invalid_list.extend(1_i32.to_be_bytes());
        assert!(skip_nbt(&mut invalid_list.as_slice()).is_err());
    }

    #[test]
    fn light_updates_are_kept_as_nibbles_in_chunk_snapshots() {
        let mut packet = Vec::new();
        for value in [2, 3, 1, 1 << 1, 1 << 1, 0, 0] {
            crate::versions::java_1_16_1::protocol::put_varint(&mut packet, value);
        }
        crate::protocol::put_varint(&mut packet, 2048);
        packet.extend([0x21; 2048]);
        crate::protocol::put_varint(&mut packet, 2048);
        packet.extend([0xa5; 2048]);
        let mut world = World::default();
        assert_eq!(world.apply_light(&packet, 256).unwrap(), (2, 3));
        let snapshot = world.chunk_snapshot(ChunkPos { x: 2, z: 3 }).unwrap();
        assert_eq!(snapshot.sky_light(0, 0, 0), Some(1));
        assert_eq!(snapshot.sky_light(1, 0, 0), Some(2));
        assert_eq!(snapshot.block_light(0, 0, 0), Some(5));
        assert_eq!(snapshot.block_light(1, 0, 0), Some(10));
    }

    #[test]
    fn light_lookup_handles_negative_chunk_coordinates() {
        let mut packet = Vec::new();
        for value in [-1, -1, 1, 1 << 1, 1 << 1, 0, 0] {
            crate::protocol::put_varint(&mut packet, value);
        }
        crate::protocol::put_varint(&mut packet, 2048);
        packet.extend([0x21; 2048]);
        crate::protocol::put_varint(&mut packet, 2048);
        packet.extend([0xa5; 2048]);
        let mut world = World::default();
        assert_eq!(world.apply_light(&packet, 256).unwrap(), (-1, -1));
        assert_eq!(world.light_at(-16, 0, -16), Some((5, 1)));
        assert_eq!(world.light_at(-1, 0, -1), Some((10, 2)));
        assert_eq!(world.light_at(-17, 0, -16), None);
    }

    fn framed_light_packet(masks: [i32; 4], arrays: &[[u8; 2048]]) -> Vec<u8> {
        let mut packet = Vec::new();
        for value in [-1, -1, 1].into_iter().chain(masks) {
            crate::protocol::put_varint(&mut packet, value);
        }
        for array in arrays {
            crate::protocol::put_varint(&mut packet, 2048);
            packet.extend(array);
        }
        packet
    }

    #[test]
    fn framed_light_arrays_keep_multiple_sections_and_channels_aligned() {
        let mask = (1 << 1) | (1 << 5) | (1 << 16);
        let packet = framed_light_packet(
            [mask, mask, 0, 0],
            &[
                [0x21; 2048],
                [0x43; 2048],
                [0x65; 2048],
                [0xa5; 2048],
                [0xcb; 2048],
                [0xed; 2048],
            ],
        );
        let mut world = World::default();
        world.apply_light(&packet, 256).unwrap();
        assert_eq!(world.light_at(-16, 0, -16), Some((5, 1)));
        assert_eq!(world.light_at(-15, 64, -16), Some((12, 4)));
        assert_eq!(world.light_at(-16, 255, -16), Some((13, 5)));
        assert_eq!(world.light_at(-16, 32, -16), None);
    }

    #[test]
    fn partial_light_updates_preserve_omitted_channels_and_old_snapshots() {
        let mut world = World::default();
        world
            .apply_light(
                &framed_light_packet([2, 2, 0, 0], &[[0x21; 2048], [0xa5; 2048]]),
                256,
            )
            .unwrap();
        let old = world.chunk_snapshot(ChunkPos { x: -1, z: -1 }).unwrap();
        world
            .apply_light(&framed_light_packet([0, 2, 0, 0], &[[0xee; 2048]]), 256)
            .unwrap();
        assert_eq!(world.light_at(-16, 0, -16), Some((14, 1)));
        assert_eq!(old.block_light(0, 0, 0), Some(5));
        world
            .apply_light(&framed_light_packet([0, 0, 0, 2], &[]), 256)
            .unwrap();
        assert_eq!(world.light_at(-16, 0, -16), Some((0, 1)));
    }

    #[test]
    fn changed_geometry_invalidates_light_and_neighbors_without_rewriting_snapshots() {
        let mut world = World::default();
        for x in -2..=0 {
            world.chunks.insert(
                (x, -1),
                Chunk {
                    sky_light: HashMap::from([(0, Arc::new([0xff; 2048]))]),
                    block_light: HashMap::from([(0, Arc::new([0xee; 2048]))]),
                    ..Default::default()
                },
            );
        }
        let old = world.chunk_snapshot(ChunkPos { x: -1, z: -1 }).unwrap();
        world.set_block(-1, 0, -1, 1);
        for x in -2..=0 {
            assert_eq!(world.light_at(x * 16, 0, -16), None);
        }
        assert_eq!(old.block_light(0, 0, 0), Some(14));
        world
            .apply_light(&framed_light_packet([0, 2, 0, 0], &[[0xee; 2048]]), 256)
            .unwrap();
        let restored = world.chunk_snapshot(ChunkPos { x: -1, z: -1 }).unwrap();
        assert_eq!(restored.block_light(0, 0, 0), Some(14));
        assert_eq!(restored.sky_light(0, 0, 0), None);
        world.set_block(-1, 0, -1, 1);
        assert_eq!(
            world
                .chunk_snapshot(ChunkPos { x: -1, z: -1 })
                .unwrap()
                .block_light(0, 0, 0),
            Some(14)
        );
    }

    #[test]
    fn initial_light_before_terrain_and_identical_chunk_delivery_remain_known() {
        let mut world = World::default();
        let packet = uniform_chunk_packet(-1, -1, 1);
        world
            .apply_light(
                &framed_light_packet([2, 2, 0, 0], &[[0xff; 2048], [0xee; 2048]]),
                256,
            )
            .unwrap();
        world.apply_chunk(&packet, 256).unwrap();
        assert_eq!(world.light_at(-16, 0, -16), Some((14, 15)));
        world.apply_chunk(&packet, 256).unwrap();
        assert_eq!(world.light_at(-16, 0, -16), Some((14, 15)));
        world
            .apply_chunk(&uniform_chunk_packet(-1, -1, 2), 256)
            .unwrap();
        assert_eq!(world.light_at(-16, 0, -16), None);
        world.unload_chunk(&unload_chunk_packet(-1, -1)).unwrap();
        world
            .apply_light(
                &framed_light_packet([2, 2, 0, 0], &[[0xff; 2048], [0xee; 2048]]),
                256,
            )
            .unwrap();
        world.apply_chunk(&packet, 256).unwrap();
        assert_eq!(world.light_at(-16, 0, -16), Some((14, 15)));
    }

    #[test]
    fn every_truncated_framed_light_packet_leaves_cache_unchanged() {
        let initial = framed_light_packet([2, 2, 0, 0], &[[0x21; 2048], [0xa5; 2048]]);
        let replacement = framed_light_packet([2, 2, 0, 0], &[[0xff; 2048], [0xee; 2048]]);
        let mut world = World::default();
        world.apply_light(&initial, 256).unwrap();
        for end in 0..replacement.len() {
            assert!(
                world.apply_light(&replacement[..end], 256).is_err(),
                "accepted prefix {end}"
            );
            assert_eq!(world.light_at(-16, 0, -16), Some((5, 1)));
        }
    }

    #[test]
    fn invalid_light_lengths_masks_and_trailing_bytes_never_publish() {
        let mut invalid = Vec::new();
        for length in [-1, 0, 2047, 2049, i32::MAX] {
            let mut packet = framed_light_packet([0, 2, 0, 0], &[]);
            crate::protocol::put_varint(&mut packet, length);
            packet.extend([0xee; 2048]);
            invalid.push(packet);
        }
        for masks in [
            [-1, 0, 0, 0],
            [1 << 18, 0, 0, 0],
            [2, 0, 2, 0],
            [0, 2, 0, 2],
        ] {
            invalid.push(framed_light_packet(masks, &[[0xee; 2048]]));
        }
        let mut trailing = framed_light_packet([0, 2, 0, 0], &[[0xee; 2048]]);
        trailing.push(0);
        invalid.push(trailing);
        for packet in invalid {
            let mut world = World::default();
            assert!(world.apply_light(&packet, 256).is_err());
            assert!(world.chunk_snapshot(ChunkPos { x: -1, z: -1 }).is_none());
        }
    }

    #[test]
    fn block_state_update_changes_door_collision_immediately() {
        let mut world = World::default();
        world.chunks.insert((0, 0), Chunk::default());
        let player = Aabb::player(0.5, 4.0, 4.0);
        world.set_block(0, 4, 5, 3580);
        let (_, closed) = world
            .collide(
                player,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 2.0,
                },
            )
            .unwrap();
        assert!(closed.z < 2.0);
        world.set_block(0, 4, 5, 3582);
        let (_, open) = world
            .collide(
                player,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 2.0,
                },
            )
            .unwrap();
        assert_eq!(open.z, 2.0);
    }

    #[test]
    fn step_candidate_climbs_bottom_slab() {
        let mut world = World::default();
        world.chunks.insert((0, 0), Chunk::default());
        for x in 0..3 {
            world.set_block(x, 0, 0, 1);
        }
        world.set_block(1, 1, 0, 8302);
        let (moved, actual) = world
            .collide_with_step(
                Aabb::player(0.5, 1.0, 0.5),
                Vec3 {
                    x: 1.0,
                    y: -0.0784,
                    z: 0.0,
                },
                true,
                0.6,
            )
            .unwrap();
        assert!(actual.x > 0.9);
        assert!((moved.min_y - 1.5).abs() < 1.0e-9);
    }

    #[test]
    fn sneak_limits_motion_past_platform_edge() {
        let mut world = World::default();
        world.chunks.insert((0, 0), Chunk::default());
        world.set_block(0, 0, 0, 1);
        let player = Aabb::player(0.5, 1.0, 0.5);
        let (dx, _) = world.limit_sneak_movement(player, 1.0, 0.0).unwrap();
        assert!(dx < 1.0);
        assert!(world.has_support(player, dx, 0.0).unwrap());
    }

    #[test]
    fn fluid_detection_observes_surface_height_and_kind() {
        let mut world = World::default();
        world.chunks.insert((0, 0), Chunk::default());
        world.set_block(0, 1, 0, 34);
        assert_eq!(
            world.fluid_intersecting(Aabb::player(0.5, 1.2, 0.5)),
            Some(Fluid::Water)
        );
        world.set_block(0, 1, 0, 50);
        assert_eq!(
            world.fluid_intersecting(Aabb::player(0.5, 1.2, 0.5)),
            Some(Fluid::Lava)
        );
        world.set_block(0, 1, 0, 41);
        assert_eq!(world.fluid_intersecting(Aabb::player(0.5, 1.3, 0.5)), None);
    }

    #[test]
    fn grounded_shallow_water_jumps_instead_of_swimming() {
        let mut world = World::default();
        world.chunks.insert((0, 0), Chunk::default());
        let player = Aabb::player(0.5, 1.0, 0.5);
        world.set_block(0, 1, 0, 40); // level 6, historical trapped stance
        assert!(!world.swimming_jump(player, Fluid::Water, true));
        assert!(world.swimming_jump(player, Fluid::Water, false));
        world.set_block(0, 1, 0, 34);
        assert!(world.swimming_jump(player, Fluid::Water, true));
        world.set_block(0, 1, 0, 39); // level 5 is still shallower than 0.4
        assert!(!world.swimming_jump(player, Fluid::Water, true));
        world.set_block(0, 2, 0, 34);
        assert!(world.swimming_jump(player, Fluid::Water, true));
        let unknown = Aabb::player(16.5, 1.0, 0.5);
        assert!(world.swimming_jump(unknown, Fluid::Water, true));
    }

    #[test]
    fn fluid_wall_exit_requires_known_dry_collision_free_headroom() {
        let mut world = World::default();
        world.chunks.insert((0, 0), Chunk::default());
        world.set_block(0, 1, 0, 39); // shallow flowing water
        world.set_block(1, 1, 0, 1); // lip
        let pose = Aabb::player(0.7, 1.5, 0.5);
        let velocity = Vec3 {
            x: 0.0,
            y: 0.01,
            z: 0.0,
        };
        assert_eq!(
            world.fluid_exit_boost(pose, velocity, 0.04, Some(Fluid::Water), true),
            Some(0.3)
        );
        assert_eq!(
            world.fluid_exit_boost(pose, velocity, 0.04, None, true),
            None
        );
        assert_eq!(
            world.fluid_exit_boost(pose, velocity, 0.04, Some(Fluid::Water), false),
            None
        );
        // A low ceiling and deep water must not create an upward impulse.
        world.set_block(0, 3, 0, 1);
        assert_eq!(
            world.fluid_exit_boost(pose, velocity, 0.04, Some(Fluid::Water), true),
            None
        );
        world.set_block(0, 3, 0, 0);
        world.set_block(0, 2, 0, 34);
        assert_eq!(
            world.fluid_exit_boost(pose, velocity, 0.04, Some(Fluid::Water), true),
            None
        );
        world.set_block(0, 2, 0, 0);
        assert_eq!(
            world.fluid_exit_boost(
                Aabb::player(0.7, 1.4, 0.5),
                velocity,
                0.04,
                Some(Fluid::Water),
                true
            ),
            None
        );
        let border = Aabb::player(15.9, 1.4, 0.5);
        assert_eq!(
            world.fluid_exit_boost(border, velocity, 0.04, Some(Fluid::Water), true),
            None
        );
    }

    #[test]
    fn fluid_flow_points_from_source_toward_lower_level() {
        let mut world = World::default();
        world.chunks.insert((0, 0), Chunk::default());
        world.set_block(0, 1, 0, 34);
        world.set_block(1, 1, 0, 39);
        world.set_block(-1, 1, 0, 1);
        world.set_block(0, 1, -1, 1);
        world.set_block(0, 1, 1, 1);
        let flow = world.fluid_flow(Aabb::player(0.5, 1.0, 0.5), Fluid::Water);
        assert!(flow.x > 0.99);
        assert!(flow.z.abs() < 1.0e-9);
    }

    #[test]
    fn ladder_vine_and_scaffolding_are_climbable() {
        let mut world = World::default();
        world.chunks.insert((0, 0), Chunk::default());
        let player = Aabb::player(0.5, 1.0, 0.5);
        for state in [3637, 4788, 14755] {
            world.set_block(0, 1, 0, state);
            assert!(
                world.is_climbable(player),
                "state {state} was not climbable"
            );
            world.set_block(0, 1, 0, 0);
        }
        assert!(!world.is_climbable(player));
    }

    #[test]
    fn scaffolding_allows_horizontal_entry_and_upward_climb() {
        let mut world = World::default();
        world.chunks.insert((0, 0), Chunk::default());
        world.set_block(1, 1, 0, 14755);
        world.set_block(1, 2, 0, 14755);
        let (entered, horizontal) = world
            .collide(
                Aabb::player(0.5, 1.0, 0.5),
                Vec3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            )
            .unwrap();
        assert_eq!(horizontal.x, 1.0);
        assert!(world.is_climbable(entered));
        let (_, upward) = world
            .collide(
                Aabb::player(1.5, 2.0, 0.5),
                Vec3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
            )
            .unwrap();
        assert_eq!(upward.y, 1.0);
    }

    #[test]
    fn special_block_effects_are_classified() {
        let mut world = World::default();
        world.chunks.insert((0, 0), Chunk::default());
        let player = Aabb::player(0.5, 1.0, 0.5);
        world.set_block(0, 0, 0, 9648);
        assert_eq!(world.block_below_name(player), Some("blue_ice"));
        world.set_block(0, 1, 0, 1341);
        assert!(world.contact_effects(player).cobweb);
        world.set_block(0, 1, 0, 14954);
        assert!(world.contact_effects(player).berry_bush);
        world.set_block(0, 1, 0, 9668);
        assert_eq!(world.contact_effects(player).bubble_push, 1);
        world.set_block(0, 1, 0, 9667);
        assert_eq!(world.contact_effects(player).bubble_push, -1);

        let environment = world.environment_state(player);
        assert_eq!(environment.supporting_state_id, Some(9648));
        assert_eq!(
            environment.supporting_block,
            Some(BlockPos { x: 0, y: 0, z: 0 })
        );
        assert_eq!(environment.bubble_push, -1);
    }

    #[test]
    fn obstacle_trajectories_match_prismarine_physics_fixtures() {
        let fixtures: DifferentialFile = serde_json::from_str(include_str!(
            "../../../data/prismarine_physics_fixtures.json"
        ))
        .unwrap();
        for fixture in fixtures
            .fixtures
            .into_iter()
            .filter(|fixture| matches!(fixture.name.as_str(), "wall" | "slab" | "stairs"))
        {
            let mut world = World::default();
            world.chunks.insert((0, 0), Chunk::default());
            for z in 0..12 {
                world.set_block(0, 0, z, 1);
            }
            match fixture.name.as_str() {
                "wall" => world.set_block(0, 1, 4, 1),
                "slab" => world.set_block(0, 1, 2, 8303),
                "stairs" => world.set_block(0, 1, 2, 1965),
                _ => unreachable!(),
            }
            let mut position = Vec3 {
                x: 0.5,
                y: 1.0,
                z: 0.5,
            };
            let mut velocity = Vec3::default();
            let mut on_ground = true;
            let mut fall_distance = 0.0;
            for (index, expected) in fixture.trajectory.iter().enumerate() {
                let was_on_ground = on_ground;
                let friction: f64 = 0.91 * 0.6;
                let acceleration = if was_on_ground {
                    0.1 * (0.162_771_36 / friction.powi(3))
                } else {
                    0.02
                };
                velocity.z += acceleration * 0.98;
                let requested = velocity;
                let (moved, actual) = world
                    .collide_with_step(
                        Aabb::player(position.x, position.y, position.z),
                        requested,
                        was_on_ground,
                        0.6,
                    )
                    .unwrap();
                let collided_x = (requested.x - actual.x).abs() > 1.0e-9;
                let collided_y = (requested.y - actual.y).abs() > 1.0e-9;
                let collided_z = (requested.z - actual.z).abs() > 1.0e-9;
                position = Vec3 {
                    x: (moved.min_x + moved.max_x) * 0.5,
                    y: moved.min_y,
                    z: (moved.min_z + moved.max_z) * 0.5,
                };
                on_ground = collided_y && requested.y < 0.0;
                if on_ground {
                    fall_distance = 0.0;
                } else if actual.y < 0.0 {
                    fall_distance += -actual.y;
                }
                if collided_x {
                    velocity.x = 0.0;
                }
                if collided_y {
                    velocity.y = 0.0;
                }
                if collided_z {
                    velocity.z = 0.0;
                }
                velocity.y = (velocity.y - 0.08) * 0.98;
                let drag = if was_on_ground { friction } else { 0.91 };
                velocity.x *= drag;
                velocity.z *= drag;
                let epsilon = 2.0e-5;
                for (actual, reference) in [position.x, position.y, position.z]
                    .into_iter()
                    .zip(expected.position)
                {
                    assert!(
                        (actual - reference).abs() < epsilon,
                        "{} tick {} position: {actual} != {reference}",
                        fixture.name,
                        index + 1
                    );
                }
                for (actual, reference) in [velocity.x, velocity.y, velocity.z]
                    .into_iter()
                    .zip(expected.velocity)
                {
                    assert!(
                        (actual - reference).abs() < epsilon,
                        "{} tick {} velocity: {actual} != {reference}",
                        fixture.name,
                        index + 1
                    );
                }
                assert_eq!(
                    on_ground,
                    expected.on_ground,
                    "{} tick {}",
                    fixture.name,
                    index + 1
                );
                assert_eq!(
                    collided_x || collided_z,
                    expected.collided_horizontal,
                    "{} tick {}",
                    fixture.name,
                    index + 1
                );
                assert_eq!(
                    collided_y,
                    expected.collided_vertical,
                    "{} tick {}",
                    fixture.name,
                    index + 1
                );
                assert!((fall_distance - expected.fall_distance).abs() < epsilon);
                assert!(!expected.sprinting);
            }
        }
    }
}
