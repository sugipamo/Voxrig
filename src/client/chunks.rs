//! Whole loaded chunk columns: received block states and received light.
use super::SessionStamp;
use super::registry::RegistryId;
use crate::{MinecraftVersion, NativeBlockState};
use std::sync::Arc;

/// Section volume: 16×16×16 native block-state IDs, index `y * 256 + z * 16 + x`.
pub(crate) type Section = Arc<[i32; 4096]>;
/// Light nibbles of one section (`DataLayer`), index as in [`Section`].
pub(crate) type LightSection = Arc<[u8; 2048]>;

/// Loaded chunk columns of the current world.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct LoadedChunks {
    /// Owning connection and world.
    pub session: SessionStamp,
    /// Last applied receive ordinal at capture.
    pub receive_sequence: u64,
    /// Chunk coordinates (`[x, z]`), in no particular order.
    pub chunks: Vec<[i32; 2]>,
}

/// One loaded chunk column at a capture boundary. Sections share storage with the
/// adapter's cache until it changes, so taking many columns is cheap.
#[derive(Clone)]
pub struct ChunkObservation {
    /// Owning connection and world.
    pub session: SessionStamp,
    /// Last applied receive ordinal at capture.
    pub receive_sequence: u64,
    /// Chunk coordinates `[x, z]`.
    pub position: [i32; 2],
    /// Lowest block Y of the dimension.
    pub min_y: i32,
    /// Dimension height in blocks.
    pub height: i32,
    pub(crate) sections: Vec<Section>,
    /// Light sections from one below `min_y` to one above the top (`height / 16 + 2`).
    /// `None` is unknown: never received, or invalidated by a received block change.
    pub(crate) sky_light: Vec<Option<LightSection>>,
    pub(crate) block_light: Vec<Option<LightSection>>,
}

impl std::fmt::Debug for ChunkObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChunkObservation")
            .field("session", &self.session)
            .field("receive_sequence", &self.receive_sequence)
            .field("position", &self.position)
            .field("min_y", &self.min_y)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl ChunkObservation {
    /// Owning version (block-state IDs are bound to it).
    pub fn version(&self) -> MinecraftVersion {
        self.session.version
    }

    fn index(&self, p: [i32; 3]) -> Option<(usize, usize)> {
        if p[0].div_euclid(16) != self.position[0]
            || p[2].div_euclid(16) != self.position[1]
            || p[1] < self.min_y
            || p[1] >= self.min_y + self.height
        {
            return None;
        }
        let section = ((p[1] - self.min_y) / 16) as usize;
        let cell =
            (p[1].rem_euclid(16) * 256 + p[2].rem_euclid(16) * 16 + p[0].rem_euclid(16)) as usize;
        Some((section, cell))
    }

    /// Native block-state ID at a world position inside this column.
    pub fn state_id(&self, p: [i32; 3]) -> Option<i32> {
        let (section, cell) = self.index(p)?;
        Some(self.sections[section][cell])
    }

    /// Version-bound block-state identity at a world position inside this column.
    pub fn state(&self, p: [i32; 3]) -> Option<RegistryId> {
        let id = self.state_id(p)?;
        super::registry::Registry::for_version(self.version())
            .block_state_id(id)
            .ok()
    }

    /// Decoded block state at a world position inside this column.
    pub fn block(&self, p: [i32; 3]) -> Option<NativeBlockState> {
        let registry = super::registry::Registry::for_version(self.version());
        registry.block_state(self.state(p)?).ok()
    }

    /// Received sky light (0..15), `None` when unknown or outside the column.
    pub fn sky_light(&self, p: [i32; 3]) -> Option<u8> {
        self.light(&self.sky_light, p)
    }

    /// Received block light (0..15), `None` when unknown or outside the column.
    pub fn block_light(&self, p: [i32; 3]) -> Option<u8> {
        self.light(&self.block_light, p)
    }

    fn light(&self, layers: &[Option<LightSection>], p: [i32; 3]) -> Option<u8> {
        let (section, cell) = self.index(p)?;
        nibble(layers.get(section + 1)?.as_ref()?, cell)
    }
}

/// `DataLayer.get`: two cells per byte, low nibble first.
pub(crate) fn nibble(layer: &[u8; 2048], cell: usize) -> Option<u8> {
    let packed = *layer.get(cell / 2)?;
    Some(if cell % 2 == 0 {
        packed & 0x0f
    } else {
        packed >> 4
    })
}

/// A shared all-air section for sections absent from a legacy column.
pub(crate) fn air_section() -> Section {
    static AIR: std::sync::OnceLock<Section> = std::sync::OnceLock::new();
    AIR.get_or_init(|| Arc::new([0; 4096])).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_index_sections_and_light_with_one_section_below() {
        let mut states = [0; 4096];
        states[3 * 256 + 2 * 16 + 1] = 1;
        let mut light = [0u8; 2048];
        let cell: usize = 3 * 256 + 2 * 16 + 1; // odd cell: high nibble
        light[cell / 2] = 0x70;
        let chunk = ChunkObservation {
            session: SessionStamp {
                version: MinecraftVersion::Java1_21_11,
                connection_id: 1,
                world_generation: 1,
            },
            receive_sequence: 1,
            position: [-1, 2],
            min_y: -64,
            height: 32,
            sections: vec![Arc::new(states), air_section()],
            sky_light: vec![None, Some(Arc::new(light)), None, None],
            block_light: vec![None; 4],
        };
        let p = [-16 + 1, -64 + 3, 32 + 2];
        assert_eq!(chunk.state_id(p), Some(1));
        assert_eq!(chunk.block(p).unwrap().name, "minecraft:stone");
        assert_eq!(chunk.sky_light(p), Some(7));
        assert_eq!(chunk.block_light(p), None);
        assert_eq!(chunk.state_id([0, -64, 32]), None); // other column
        assert_eq!(chunk.state_id([-15, -33, 32]), Some(0)); // top cell
        assert_eq!(chunk.state_id([-15, -32, 32]), None); // above the dimension
    }
}
