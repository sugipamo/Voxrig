//! Received chunk metadata; no local biome, heightmap or NBT reconstruction.
use super::nbt::{NbtData, NbtValue};
use super::{ObservedValue, SessionStamp, received};
use anyhow::{Context, Result, bail};
use std::{collections::BTreeMap, sync::Arc};

/// Identity of a received column incarnation. Coordinates alone are insufficient.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ChunkIdentity {
    /// Original connection and world generation.
    pub session: SessionStamp,
    /// Original column coordinates.
    pub position: [i32; 2],
    /// First partial load or most recent full replacement receipt.
    pub load_sequence: u64,
}
/// Received quart-resolution biome IDs, bound to this version/world's registry.
#[derive(Clone, Debug, serde::Serialize)]
pub struct BiomeVolume {
    /// Lowest block Y of the owning dimension.
    pub min_y: i32,
    /// Dimension height in blocks.
    pub height: i32,
    /// Native IDs, ordered quart Y, Z, X. No built-in registry fallback.
    pub ids: Arc<[i32]>,
}
impl BiomeVolume {
    /// Original biome ID for a block inside the owning column's height range.
    /// X/Z are local block coordinates; biome selection noise is not applied.
    pub fn native_id(&self, local_x: u8, y: i32, local_z: u8) -> Option<i32> {
        let relative = y.checked_sub(self.min_y)?;
        if local_x > 15 || local_z > 15 || !(0..self.height).contains(&relative) {
            return None;
        }
        self.ids
            .get(
                (relative / 4) as usize * 16
                    + usize::from(local_z / 4) * 4
                    + usize::from(local_x / 4),
            )
            .copied()
    }
}
/// Original heightmap kind. Names also identify legacy compound fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub enum HeightmapKind {
    /// World-generation surface.
    WorldSurfaceWg,
    /// World surface.
    WorldSurface,
    /// World-generation ocean floor.
    OceanFloorWg,
    /// Ocean floor.
    OceanFloor,
    /// Motion-blocking surface.
    MotionBlocking,
    /// Motion-blocking surface excluding leaves.
    MotionBlockingNoLeaves,
}
impl HeightmapKind {
    pub(crate) fn from_id(id: i32) -> Result<Self> {
        Ok(match id {
            0 => Self::WorldSurfaceWg,
            1 => Self::WorldSurface,
            2 => Self::OceanFloorWg,
            3 => Self::OceanFloor,
            4 => Self::MotionBlocking,
            5 => Self::MotionBlockingNoLeaves,
            _ => bail!("invalid native heightmap kind"),
        })
    }
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::WorldSurfaceWg => "WORLD_SURFACE_WG",
            Self::WorldSurface => "WORLD_SURFACE",
            Self::OceanFloorWg => "OCEAN_FLOOR_WG",
            Self::OceanFloor => "OCEAN_FLOOR",
            Self::MotionBlocking => "MOTION_BLOCKING",
            Self::MotionBlockingNoLeaves => "MOTION_BLOCKING_NO_LEAVES",
        }
    }
}
/// Original packed heightmap. Never recomputed from received block states.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ReceivedHeightmap {
    /// Original native kind.
    pub kind: HeightmapKind,
    /// Dimension minimum Y used by the original compact representation.
    pub min_y: i32,
    /// Dimension height.
    pub height: i32,
    /// Original signed long values, without normalization.
    pub words: Arc<[i64]>,
}
impl ReceivedHeightmap {
    /// Decode the native first-available Y at local X/Z. This is a received
    /// heightmap value, not a present-time surface or a safe standing location.
    /// Wrong native array lengths are retained but cannot be sampled.
    pub fn first_available_y(&self, local_x: u8, local_z: u8) -> Option<i32> {
        if local_x > 15 || local_z > 15 || !(16..=4096).contains(&self.height) {
            return None;
        }
        let bits = 32 - (self.height as u32).leading_zeros();
        let per_word = 64 / bits as usize;
        if self.words.len() != 256usize.div_ceil(per_word) {
            return None;
        }
        let cell = usize::from(local_z) * 16 + usize::from(local_x);
        let value = (self.words[cell / per_word] as u64 >> ((cell % per_word) * bits as usize))
            & ((1u64 << bits) - 1);
        self.min_y.checked_add(value as i32)
    }
}
/// Version-specific original block-entity kind, without conflating update actions.
#[derive(Clone, Debug, serde::Serialize)]
pub enum BlockEntityKind {
    /// Legacy full-chunk NBT's optional explicit `id` String.
    LegacyIdentifier(Option<String>),
    /// Legacy update action byte, not a registry type ID.
    LegacyUpdateAction(u8),
    /// Original 1.21.11 block-entity type registry ID.
    ModernType(i32),
}
/// One original block-entity payload, including explicit absent optional NBT.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ReceivedBlockEntity {
    /// Original world block coordinates.
    pub position: [i32; 3],
    /// Original version-specific kind.
    pub kind: BlockEntityKind,
    /// Decoded original compound; None is an explicit EndTag receipt.
    pub data: Option<NbtData>,
    encoded_nbt: Arc<[u8]>,
}
impl ReceivedBlockEntity {
    /// Original named (legacy) or unnamed (modern) optional NBT field bytes.
    pub fn encoded_nbt(&self) -> &[u8] {
        &self.encoded_nbt
    }
    pub(crate) fn decode(
        position: [i32; 3],
        kind: BlockEntityKind,
        bytes: Arc<[u8]>,
        version: crate::MinecraftVersion,
    ) -> Result<Self> {
        let data = optional_nbt(&bytes, version)?;
        Ok(Self {
            position,
            kind,
            data,
            encoded_nbt: bytes,
        })
    }
}
/// Coherent bounded received metadata for one column incarnation.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ChunkContextObservation {
    /// Original loaded-column identity.
    pub chunk: ChunkIdentity,
    /// Capture boundary, independent of all original field sources.
    pub receive_sequence: u64,
    /// Last explicit biome volume, not a noise-sampled player biome.
    pub biomes: Option<ObservedValue<BiomeVolume>>,
    /// Last explicit heightmaps; invalidated by subsequent terrain updates.
    pub heightmaps: Option<ObservedValue<Vec<ReceivedHeightmap>>>,
    /// Retained original entity receipts, sorted by position. A subsequent block
    /// change invalidates that cell until a new entity packet supplies it.
    pub block_entities: Vec<ObservedValue<ReceivedBlockEntity>>,
    /// A full chunk's complete entity list remains untouched by later terrain
    /// changes. False never proves an absent cell has no block entity.
    pub block_entities_complete: bool,
}
impl crate::Client {
    /// Read one column's bounded last-received metadata, including after closure.
    /// Unloaded columns return None; saved captures keep their original identity.
    pub async fn chunk_context(
        &self,
        position: [i32; 2],
    ) -> crate::Result<Option<ChunkContextObservation>> {
        super::dispatch!(&self.adapter,a=>super::adapter::ChunkOps::chunk_context(a,position).await)
    }
}

pub(crate) fn optional_nbt(
    bytes: &[u8],
    version: crate::MinecraftVersion,
) -> Result<Option<NbtData>> {
    if bytes == [0] {
        return Ok(None);
    }
    Ok(Some(super::nbt::decode(bytes, version)?))
}
#[derive(Clone)]
pub(crate) struct Stored<T> {
    pub value: T,
    sequence: u64,
}
impl<T: Clone> Stored<T> {
    pub(crate) fn pending(value: T) -> Self {
        Self { value, sequence: 0 }
    }
    fn stamp(&mut self, sequence: u64) {
        if self.sequence == 0 {
            self.sequence = sequence;
        }
    }
    fn observation(&self) -> Option<ObservedValue<T>> {
        (self.sequence > 0).then(|| received(self.value.clone(), self.sequence))
    }
}
#[derive(Clone, Default)]
pub(crate) struct ChunkData {
    pub loaded_sequence: u64,
    pub biomes: Option<Stored<BiomeVolume>>,
    pub heightmaps: Option<Stored<Vec<ReceivedHeightmap>>>,
    pub entities: BTreeMap<[i32; 3], Stored<ReceivedBlockEntity>>,
    pub complete: bool,
}
impl ChunkData {
    pub(crate) fn stamp(&mut self, sequence: u64) {
        if self.loaded_sequence == 0 {
            self.loaded_sequence = sequence;
        }
        if let Some(value) = &mut self.biomes {
            value.stamp(sequence);
        }
        if let Some(value) = &mut self.heightmaps {
            value.stamp(sequence);
        }
        for value in self.entities.values_mut() {
            value.stamp(sequence);
        }
    }
    pub(crate) fn capture(
        &self,
        session: SessionStamp,
        position: [i32; 2],
        receive_sequence: u64,
    ) -> Option<ChunkContextObservation> {
        if self.loaded_sequence == 0 {
            return None;
        }
        Some(ChunkContextObservation {
            chunk: ChunkIdentity {
                session,
                position,
                load_sequence: self.loaded_sequence,
            },
            receive_sequence,
            biomes: self.biomes.as_ref().and_then(Stored::observation),
            heightmaps: self.heightmaps.as_ref().and_then(Stored::observation),
            block_entities: self
                .entities
                .values()
                .filter_map(Stored::observation)
                .collect(),
            block_entities_complete: self.complete,
        })
    }
    pub(crate) fn invalidate_cell(&mut self, position: [i32; 3]) {
        self.heightmaps = None;
        self.entities.remove(&position);
        self.complete = false;
    }
    pub(crate) fn insert(&mut self, value: ReceivedBlockEntity) -> Result<()> {
        self.entities.insert(value.position, Stored::pending(value));
        self.check_budget()
    }
    pub(crate) fn check_budget(&self) -> Result<()> {
        if self.entities.len() > 4096
            || self
                .entities
                .values()
                .map(|v| v.value.encoded_nbt.len())
                .sum::<usize>()
                > 2_097_152
        {
            bail!("chunk metadata exceeds entity/NBT budget");
        }
        Ok(())
    }
    pub(crate) fn legacy(
        previous: Option<Self>,
        position: [i32; 2],
        full: bool,
        sections: i32,
        biomes: Option<Arc<[i32; 1024]>>,
        heightmaps: &[u8],
        entities: &[Arc<[u8]>],
    ) -> Result<Self> {
        let mut replacement = if full {
            Self::default()
        } else {
            previous.unwrap_or_default()
        };
        replacement.complete = full;
        if !full {
            replacement
                .entities
                .retain(|p, _| sections & (1 << (p[1] / 16)) == 0);
        }
        if let Some(ids) = biomes {
            replacement.biomes = Some(Stored::pending(BiomeVolume {
                min_y: 0,
                height: 256,
                ids,
            }));
        }
        let nbt = optional_nbt(heightmaps, crate::MinecraftVersion::Java1_16_1)?;
        let mut maps = Vec::new();
        if let Some(nbt) = nbt {
            for id in 0..6 {
                let kind = HeightmapKind::from_id(id)?;
                if let Some(NbtValue::LongArray(words)) = nbt.root().get(kind.name()) {
                    maps.push(ReceivedHeightmap {
                        kind,
                        min_y: 0,
                        height: 256,
                        words: Arc::from(words.clone()),
                    });
                }
            }
        }
        replacement.heightmaps = Some(Stored::pending(maps));
        for bytes in entities {
            let data = optional_nbt(bytes, crate::MinecraftVersion::Java1_16_1)?
                .context("full chunk entity needs a compound")?;
            let root = data.root();
            let cell = ["x", "y", "z"].map(|key| root.get(key).and_then(NbtValue::as_int));
            let cell = [
                cell[0].context("entity x missing")?,
                cell[1].context("entity y missing")?,
                cell[2].context("entity z missing")?,
            ];
            if [cell[0].div_euclid(16), cell[2].div_euclid(16)] != position
                || !(0..256).contains(&cell[1])
            {
                bail!("entity outside its original column");
            }
            let kind = root
                .get("id")
                .and_then(NbtValue::as_string)
                .map(|s| s.text())
                .transpose()?;
            replacement.insert(ReceivedBlockEntity {
                position: cell,
                kind: BlockEntityKind::LegacyIdentifier(kind),
                data: Some(data),
                encoded_nbt: bytes.clone(),
            })?;
        }
        replacement.check_budget()?;
        Ok(replacement)
    }
}
