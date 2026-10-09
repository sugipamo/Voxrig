//! 1.21.11 chunk wire format and dimension-bound state storage.
use super::wire::{Reader, unpack_position};
use anyhow::{Context, Result, bail};
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Dimension {
    pub min_y: i32,
    pub height: i32,
}
impl Dimension {
    pub fn new(min_y: i32, height: i32) -> Result<Self> {
        if min_y % 16 != 0
            || height % 16 != 0
            || !(16..=4096).contains(&height)
            || min_y.checked_add(height).is_none()
        {
            bail!("invalid dimension height");
        }
        Ok(Self { min_y, height })
    }
}

/// Received light of one column: sections from one below the dimension to one above.
/// `None` is unknown (never received, or invalidated by a block change).
#[derive(Clone, Default)]
pub(crate) struct ColumnLight {
    pub sky: Vec<Option<Arc<[u8; 2048]>>>,
    pub block: Vec<Option<Arc<[u8; 2048]>>>,
}

#[derive(Default)]
pub(crate) struct World {
    pub dimension: Option<(String, Dimension)>,
    chunks: HashMap<(i32, i32), Vec<Arc<[i32; 4096]>>>,
    contexts: HashMap<(i32, i32), crate::client::chunk_context::ChunkData>,
    light: HashMap<(i32, i32), ColumnLight>,
    pub revision: u64,
}

impl World {
    pub(crate) fn stamp_context(&mut self, position: [i32; 2], sequence: u64) {
        if let Some(context) = self.contexts.get_mut(&(position[0], position[1])) {
            context.stamp(sequence);
        }
    }
    pub(crate) fn context(
        &self,
        position: [i32; 2],
        session: crate::client::SessionStamp,
        sequence: u64,
    ) -> Option<crate::client::ChunkContextObservation> {
        self.contexts
            .get(&(position[0], position[1]))?
            .capture(session, position, sequence)
    }
    pub(crate) fn update_entity(&mut self, payload: &[u8], sequence: u64) -> Result<()> {
        let mut r = Reader::new(payload);
        let p = unpack_position(r.u64()?);
        let kind = r.varint()?;
        if kind < 0 {
            bail!("negative block entity type");
        }
        let before = r.remaining();
        r.skip_optional_nbt()?;
        let encoded = Arc::from(&before[..before.len() - r.remaining().len()]);
        let value = crate::client::ReceivedBlockEntity::decode(
            p,
            crate::client::BlockEntityKind::ModernType(kind),
            encoded,
            crate::MinecraftVersion::Java1_21_11,
        )?;
        r.end()?;
        let dimension = self
            .dimension
            .as_ref()
            .context("entity before dimension")?
            .1;
        if p[1] < dimension.min_y || p[1] >= dimension.min_y + dimension.height {
            bail!("entity outside dimension");
        }
        let key = (p[0].div_euclid(16), p[2].div_euclid(16));
        if let Some(mut context) = self.contexts.get(&key).cloned() {
            context.insert(value)?;
            context.stamp(sequence);
            self.contexts.insert(key, context);
        }
        Ok(())
    }
    pub(crate) fn update_biomes(
        &mut self,
        payload: &[u8],
        maximum: usize,
        sequence: u64,
    ) -> Result<()> {
        let dimension = self
            .dimension
            .as_ref()
            .context("biomes before dimension")?
            .1;
        let mut r = Reader::new(payload);
        let mut updates = Vec::new();
        for _ in 0..r.count(maximum.min(65_536))? {
            let packed = r.u64()?;
            let key = (packed as i32, (packed >> 32) as i32);
            let mut data = Reader::new(r.byte_array(2_097_152)?);
            let mut ids = Vec::new();
            for _ in 0..dimension.height / 16 {
                ids.extend(palette(&mut data, 64, 1, 3, 8)?);
            }
            data.end()?;
            updates.push((
                key,
                crate::client::BiomeVolume {
                    min_y: dimension.min_y,
                    height: dimension.height,
                    ids: Arc::from(ids),
                },
            ));
        }
        r.end()?;
        for (key, value) in updates {
            if let Some(context) = self.contexts.get_mut(&key) {
                context.biomes = Some(crate::client::chunk_context::Stored::pending(value));
                context.stamp(sequence);
            }
        }
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn seed_replay_cell(&mut self, p: [i32; 3], id: i32) {
        let dimension = self.dimension.as_ref().unwrap().1;
        self.chunks
            .entry((p[0].div_euclid(16), p[2].div_euclid(16)))
            .or_insert_with(|| {
                let empty = Arc::new([0; 4096]);
                vec![empty; (dimension.height / 16) as usize]
            });
        self.set_block(p, id).unwrap();
    }
    pub fn select_dimension(&mut self, name: String, dimension: Dimension) {
        self.dimension = Some((name, dimension));
        self.chunks.clear();
        self.contexts.clear();
        self.light.clear();
        self.revision += 1;
    }
    pub fn reset(&mut self) {
        self.dimension = None;
        self.chunks.clear();
        self.contexts.clear();
        self.light.clear();
        self.revision += 1;
    }
    /// A loaded column's sections (shared) and its received light.
    pub fn column(&self, x: i32, z: i32) -> Option<(Vec<Arc<[i32; 4096]>>, ColumnLight)> {
        let sections = self.chunks.get(&(x, z))?.clone();
        let light = self.light.get(&(x, z)).cloned().unwrap_or_else(|| {
            let count = sections.len() + 2;
            ColumnLight {
                sky: vec![None; count],
                block: vec![None; count],
            }
        });
        Some((sections, light))
    }
    /// ClientboundLightUpdatePacket: received arrays replace the masked sections.
    pub fn update_light(&mut self, payload: &[u8]) -> Result<[i32; 2]> {
        let mut r = Reader::new(payload);
        let x = r.varint()?;
        let z = r.varint()?;
        let count = self.light_sections()?;
        let mut light = self.light.get(&(x, z)).cloned().unwrap_or(ColumnLight {
            sky: vec![None; count],
            block: vec![None; count],
        });
        read_light(&mut r, &mut light, count)?;
        r.end()?;
        // Light is kept only with loaded terrain (it arrives with the chunk packet).
        if self.chunks.contains_key(&(x, z)) {
            self.light.insert((x, z), light);
            self.revision += 1;
        }
        Ok([x, z])
    }
    fn light_sections(&self) -> Result<usize> {
        let dimension = self.dimension.as_ref().context("light before dimension")?.1;
        Ok((dimension.height / 16 + 2) as usize)
    }
    /// A received block change makes the light of its column and the eight around it
    /// unknown: Voxrig does not propagate light (as for 1.16.1).
    fn invalidate_light(&mut self, x: i32, z: i32) {
        for dx in -1..=1 {
            for dz in -1..=1 {
                if let Some(light) = self.light.get_mut(&(x + dx, z + dz)) {
                    light.sky.iter_mut().for_each(|s| *s = None);
                    light.block.iter_mut().for_each(|s| *s = None);
                }
            }
        }
    }
    pub fn block(&self, p: [i32; 3]) -> Option<i32> {
        let dimension = self.dimension.as_ref()?.1;
        if p[1] < dimension.min_y || p[1] >= dimension.min_y + dimension.height {
            return None;
        }
        let sections = self
            .chunks
            .get(&(p[0].div_euclid(16), p[2].div_euclid(16)))?;
        let section = &sections[((p[1] - dimension.min_y) / 16) as usize];
        Some(
            section[(p[1].rem_euclid(16) * 256 + p[2].rem_euclid(16) * 16 + p[0].rem_euclid(16))
                as usize],
        )
    }
    fn set_block(&mut self, p: [i32; 3], id: i32) -> Result<()> {
        super::validate_state_id(id)?;
        let dimension = self
            .dimension
            .as_ref()
            .context("block update before dimension")?
            .1;
        if p[1] < dimension.min_y || p[1] >= dimension.min_y + dimension.height {
            bail!("block update outside dimension");
        }
        if let Some(sections) = self
            .chunks
            .get_mut(&(p[0].div_euclid(16), p[2].div_euclid(16)))
        {
            let section = &mut sections[((p[1] - dimension.min_y) / 16) as usize];
            let cell = (p[1].rem_euclid(16) * 256 + p[2].rem_euclid(16) * 16 + p[0].rem_euclid(16))
                as usize;
            if section[cell] != id {
                Arc::make_mut(section)[cell] = id;
                if let Some(context) = self
                    .contexts
                    .get_mut(&(p[0].div_euclid(16), p[2].div_euclid(16)))
                {
                    context.invalidate_cell(p);
                }
                self.invalidate_light(p[0].div_euclid(16), p[2].div_euclid(16));
            }
            self.revision += 1;
        }
        Ok(())
    }
    pub fn block_change(&mut self, payload: &[u8]) -> Result<Vec<([i32; 3], i32)>> {
        let mut r = Reader::new(payload);
        let p = unpack_position(r.u64()?);
        let id = r.varint()?;
        r.end()?;
        self.set_block(p, id)?;
        Ok(vec![(p, id)])
    }
    pub fn section_changes(&mut self, payload: &[u8]) -> Result<Vec<([i32; 3], i32)>> {
        let mut r = Reader::new(payload);
        let p = r.u64()?;
        let x = (p as i64 >> 42) as i32;
        let z = ((p << 22) as i64 >> 42) as i32;
        let y = ((p << 44) as i64 >> 44) as i32;
        let count = r.count(4096)?;
        let mut changes = Vec::with_capacity(count);
        for _ in 0..count {
            let record = r.varint()?;
            if record < 0 {
                bail!("negative block change");
            }
            let id = record >> 12;
            super::validate_state_id(id)?;
            changes.push((
                [
                    x * 16 + ((record >> 8) & 15),
                    y * 16 + (record & 15),
                    z * 16 + ((record >> 4) & 15),
                ],
                id,
            ));
        }
        r.end()?;
        let dimension = self
            .dimension
            .as_ref()
            .context("section update before dimension")?
            .1;
        if changes
            .iter()
            .any(|(p, _)| p[1] < dimension.min_y || p[1] >= dimension.min_y + dimension.height)
        {
            bail!("section update outside dimension");
        }
        for (p, id) in &changes {
            self.set_block(*p, *id)?;
        }
        Ok(changes)
    }
    pub fn unload(&mut self, payload: &[u8]) -> Result<()> {
        let mut r = Reader::new(payload);
        let z = r.i32()?;
        let x = r.i32()?;
        r.end()?;
        self.chunks.remove(&(x, z));
        self.contexts.remove(&(x, z));
        self.light.remove(&(x, z));
        self.revision += 1;
        Ok(())
    }
    pub fn loaded_chunks(&self) -> impl Iterator<Item = [i32; 2]> + '_ {
        self.chunks.keys().map(|&(x, z)| [x, z])
    }
    pub fn load(
        &mut self,
        payload: &[u8],
        maximum: usize,
    ) -> Result<Vec<([i32; 3], super::piston_nbt::PistonData)>> {
        let mut r = Reader::new(payload);
        let x = r.i32()?;
        let z = r.i32()?;
        if !(-1_875_000..=1_875_000).contains(&x) || !(-1_875_000..=1_875_000).contains(&z) {
            bail!("chunk outside border");
        }
        if !self.chunks.contains_key(&(x, z)) && self.chunks.len() >= maximum {
            bail!("chunk cache limit exceeded");
        }
        let dimension = self.dimension.as_ref().context("chunk before dimension")?.1;
        let mut maps = std::collections::BTreeMap::new();
        for _ in 0..r.count(16)? {
            let kind = crate::client::chunk_context::HeightmapKind::from_id(r.varint()?)?;
            let count = r.count(1024)?;
            let words = (0..count)
                .map(|_| r.u64().map(|v| v as i64))
                .collect::<Result<Vec<_>>>()?;
            maps.insert(
                kind,
                crate::client::ReceivedHeightmap {
                    kind,
                    min_y: dimension.min_y,
                    height: dimension.height,
                    words: Arc::from(words),
                },
            );
        }
        let mut context = crate::client::chunk_context::ChunkData {
            complete: true,
            heightmaps: Some(crate::client::chunk_context::Stored::pending(
                maps.into_values().collect(),
            )),
            ..Default::default()
        };
        let mut data = Reader::new(r.byte_array(2_097_152)?);
        let mut sections: Vec<Arc<[i32; 4096]>> = Vec::new();
        let mut biomes = Vec::new();
        for _ in 0..dimension.height / 16 {
            let non_air = data.u16()?;
            if non_air > 4096 {
                bail!("invalid non-air count");
            }
            let states = palette(&mut data, 4096, 4, 8, 16)?;
            for id in &states {
                super::validate_state_id(*id)?;
            }
            sections.push(Arc::new(
                states
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("invalid section volume"))?,
            ));
            biomes.extend(palette(&mut data, 64, 1, 3, 8)?);
        }
        context.biomes = Some(crate::client::chunk_context::Stored::pending(
            crate::client::BiomeVolume {
                min_y: dimension.min_y,
                height: dimension.height,
                ids: Arc::from(biomes),
            },
        ));
        data.end()?;
        let mut pistons = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..r.count(65_536)? {
            let xz = r.u8()?;
            let y = r.u16()? as i16 as i32;
            let p = [x * 16 + i32::from(xz >> 4), y, z * 16 + i32::from(xz & 15)];
            if y < dimension.min_y || y >= dimension.min_y + dimension.height || !seen.insert(p) {
                bail!("invalid/duplicate block entity position");
            }
            let kind = r.varint()?;
            if kind < 0 {
                bail!("negative block entity type");
            }
            let nbt_before = r.remaining();
            if kind == super::piston_nbt::PISTON_TYPE {
                let id = sections[((y - dimension.min_y) / 16) as usize][(y.rem_euclid(16) * 256
                    + p[2].rem_euclid(16) * 16
                    + p[0].rem_euclid(16))
                    as usize];
                let carrier = super::native_state(id)?;
                if carrier.name != "minecraft:moving_piston" {
                    bail!("piston entity without moving state");
                }
                if let Some(data) = super::piston_nbt::PistonData::read(&mut r)? {
                    if carrier.properties.get("facing").map(String::as_str)
                        != Some(data.direction.name())
                    {
                        bail!("piston entity facing mismatch");
                    }
                    if pistons.len() >= 4096 {
                        bail!("chunk moving carrier limit exceeded");
                    }
                    pistons.push((p, data));
                }
            } else {
                r.skip_optional_nbt()?;
            }
            let encoded = Arc::from(&nbt_before[..nbt_before.len() - r.remaining().len()]);
            context.insert(crate::client::ReceivedBlockEntity::decode(
                p,
                crate::client::BlockEntityKind::ModernType(kind),
                encoded,
                crate::MinecraftVersion::Java1_21_11,
            )?)?;
        }
        // Same terrain keeps the previous light under the packet's own arrays; changed
        // or new terrain starts unknown.
        let count = (dimension.height / 16 + 2) as usize;
        let unchanged = self.chunks.get(&(x, z)) == Some(&sections);
        let mut light = match self.light.get(&(x, z)) {
            Some(previous) if unchanged => previous.clone(),
            _ => ColumnLight {
                sky: vec![None; count],
                block: vec![None; count],
            },
        };
        read_light(&mut r, &mut light, count)?;
        r.end()?;
        self.chunks.insert((x, z), sections);
        self.contexts.insert((x, z), context);
        self.light.insert((x, z), light);
        self.revision += 1;
        Ok(pistons)
    }
}

/// ClientboundLightUpdatePacketData: sky, block, empty-sky and empty-block masks
/// (BitSets), then the sky and block arrays in mask order.
fn read_light(r: &mut Reader<'_>, light: &mut ColumnLight, count: usize) -> Result<()> {
    let mut masks: [Vec<bool>; 4] = Default::default();
    for mask in &mut masks {
        *mask = vec![false; count];
        for word in 0..r.count(1024)? {
            let value = r.u64()?;
            for bit in 0..64 {
                if value & (1 << bit) != 0 {
                    *mask
                        .get_mut(word * 64 + bit)
                        .context("light mask outside the dimension")? = true;
                }
            }
        }
    }
    let [sky, block, empty_sky, empty_block] = masks;
    for (mask, empty, layers) in [
        (sky, empty_sky, &mut light.sky),
        (block, empty_block, &mut light.block),
    ] {
        if mask.iter().zip(&empty).any(|(a, b)| *a && *b) {
            bail!("overlapping light masks");
        }
        let arrays = r.count(count)?;
        if arrays != mask.iter().filter(|b| **b).count() {
            bail!("light array count differs from its mask");
        }
        for (index, layer) in layers.iter_mut().enumerate() {
            if empty[index] {
                *layer = Some(Arc::new([0; 2048]));
            }
            if mask[index] {
                let array: [u8; 2048] = r
                    .byte_array(2048)?
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("invalid light array size"))?;
                *layer = Some(Arc::new(array));
            }
        }
    }
    Ok(())
}

fn palette(
    r: &mut Reader<'_>,
    count: usize,
    min_bits: u8,
    indirect_max: u8,
    max_bits: u8,
) -> Result<Vec<i32>> {
    let bits = r.u8()?;
    if bits == 0 {
        let value = r.varint()?;
        if value < 0 {
            bail!("negative palette ID");
        }
        return Ok(vec![value; count]);
    }
    if bits < min_bits || bits > max_bits {
        bail!("invalid palette width {bits}");
    }
    let palette = if bits <= indirect_max {
        let size = r.count(1usize << bits)?;
        if size == 0 {
            bail!("empty palette");
        }
        let mut palette = Vec::with_capacity(size);
        for _ in 0..size {
            let id = r.varint()?;
            if id < 0 {
                bail!("negative palette ID");
            }
            palette.push(id);
        }
        Some(palette)
    } else {
        None
    };
    let per_long = 64 / bits as usize;
    let mask = (1u64 << bits) - 1;
    let mut output = Vec::with_capacity(count);
    for _ in 0..count.div_ceil(per_long) {
        let word = r.u64()?;
        for index in 0..per_long {
            if output.len() == count {
                break;
            }
            let value = ((word >> (index * bits as usize)) & mask) as i32;
            output.push(if let Some(palette) = &palette {
                *palette
                    .get(value as usize)
                    .context("palette index out of range")?
            } else {
                value
            });
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context_chunk() -> Vec<u8> {
        let mut packet = vec![0; 8];
        crate::protocol::put_varint(&mut packet, 1);
        crate::protocol::put_varint(&mut packet, 4);
        crate::protocol::put_varint(&mut packet, 37);
        for _ in 0..37 {
            packet.extend(0u64.to_be_bytes());
        }
        let sections = [0, 0, 0, 0, 0, 7].repeat(24);
        crate::protocol::put_varint(&mut packet, sections.len() as i32);
        packet.extend(sections);
        packet.push(1);
        packet.push(0x12);
        packet.extend(65u16.to_be_bytes());
        packet.extend([0, 10, 0]);
        // Four light masks, then the sky and block array counts.
        packet.extend([0; 6]);
        packet
    }
    #[test]
    fn chunk_context_aggregate_budget_refuses_update_without_publishing_other_receipts() {
        let mut world = World::default();
        world.select_dimension(
            "minecraft:overworld".into(),
            Dimension::new(-64, 384).unwrap(),
        );
        world.load(&context_chunk(), 128).unwrap();
        world.stamp_context([0, 0], 1);
        let mut nbt = vec![10, 7, 0, 1, b'v'];
        nbt.extend(700_000i32.to_be_bytes());
        nbt.extend(vec![0; 700_000]);
        nbt.push(0);
        for x in [2u64, 3] {
            let mut packet = ((x << 38) | (2 << 12) | 65).to_be_bytes().to_vec();
            packet.push(0);
            packet.extend(&nbt);
            world.update_entity(&packet, x).unwrap();
        }
        let mut packet = ((4u64 << 38) | (2 << 12) | 65).to_be_bytes().to_vec();
        packet.push(0);
        packet.extend(&nbt);
        assert!(world.update_entity(&packet, 4).is_err());
        let context = world.contexts.get(&(0, 0)).unwrap();
        assert_eq!(context.entities.len(), 3);
        assert!(!context.entities.contains_key(&[4, 65, 2]));
        let session = crate::client::SessionStamp {
            version: crate::MinecraftVersion::Java1_21_11,
            connection_id: 1,
            world_generation: 1,
        };
        let saved = context.capture(session, [0, 0], 4).unwrap();
        assert_eq!(
            saved.biomes.unwrap().source,
            crate::client::ValueSource::Received { sequence: 1 }
        );
        assert_eq!(
            saved.block_entities[1].source,
            crate::client::ValueSource::Received { sequence: 2 }
        );
    }
    #[test]
    fn chunk_context_modern_updates_are_atomic_bounded_and_do_not_resurrect_unloaded_columns() {
        let mut world = World::default();
        world.select_dimension(
            "minecraft:overworld".into(),
            Dimension::new(-64, 384).unwrap(),
        );
        let session = crate::client::SessionStamp {
            version: crate::MinecraftVersion::Java1_21_11,
            connection_id: 9,
            world_generation: 1,
        };
        let packet = context_chunk();
        world.load(&packet, 128).unwrap();
        world.stamp_context([0, 0], 10);
        let saved = world.context([0, 0], session, 10).unwrap();
        assert_eq!(
            saved.biomes.as_ref().unwrap().value.native_id(15, -64, 15),
            Some(7)
        );
        assert_eq!(
            saved.biomes.as_ref().unwrap().value.native_id(15, 319, 15),
            Some(7)
        );
        assert_eq!(
            saved.biomes.as_ref().unwrap().value.native_id(15, 320, 15),
            None
        );
        assert_eq!(
            saved.heightmaps.as_ref().unwrap().value[0].first_available_y(1, 1),
            Some(-64)
        );
        assert_eq!(saved.block_entities[0].value.position, [1, 65, 2]);
        let old = serde_json::to_value(&saved).unwrap();
        for cut in [0, 12, packet.len() - 4, packet.len() - 1] {
            assert!(world.load(&packet[..cut], 128).is_err());
            assert_eq!(
                serde_json::to_value(world.context([0, 0], session, 10)).unwrap(),
                old
            );
        }
        let mut biomes = vec![1];
        biomes.extend(0u64.to_be_bytes());
        let data = [0, 42].repeat(24);
        crate::protocol::put_varint(&mut biomes, data.len() as i32);
        biomes.extend(data);
        for cut in [0, 8, biomes.len() - 1] {
            assert!(world.update_biomes(&biomes[..cut], 128, 11).is_err());
        }
        assert!(world.update_biomes(&biomes, 0, 11).is_err());
        world.update_biomes(&biomes, 128, 11).unwrap();
        let next = world.context([0, 0], session, 11).unwrap();
        assert_eq!(next.chunk, saved.chunk);
        assert_eq!(
            next.biomes.as_ref().unwrap().value.native_id(1, 65, 2),
            Some(42)
        );
        assert_eq!(
            next.heightmaps.as_ref().unwrap().source,
            saved.heightmaps.as_ref().unwrap().source
        );
        let mut entity = ((1u64 << 38) | (2u64 << 12) | 65).to_be_bytes().to_vec();
        entity.extend([0, 0]);
        world.update_entity(&entity, 12).unwrap();
        let next = world.context([0, 0], session, 12).unwrap();
        assert!(next.block_entities[0].value.data.is_none());
        assert!(saved.block_entities[0].value.data.is_some());
        let mut trailing = entity.clone();
        trailing.push(0);
        assert!(world.update_entity(&trailing, 13).is_err());
        world.set_block([1, 65, 2], 1).unwrap();
        let next = world.context([0, 0], session, 13).unwrap();
        assert!(
            next.heightmaps.is_none()
                && next.block_entities.is_empty()
                && !next.block_entities_complete
        );
        world.unload(&[0; 8]).unwrap();
        world.update_entity(&entity, 14).unwrap();
        world.update_biomes(&biomes, 128, 14).unwrap();
        assert!(world.context([0, 0], session, 14).is_none());
        world.load(&packet, 128).unwrap();
        world.stamp_context([0, 0], 15);
        assert_ne!(
            world.context([0, 0], session, 15).unwrap().chunk,
            saved.chunk
        );
        world.reset();
        assert!(world.context([0, 0], session, 16).is_none());
        assert_eq!(serde_json::to_value(saved).unwrap(), old);
    }
    #[test]
    fn palette_uses_no_length_prefix_and_rejects_bad_indices() {
        assert_eq!(
            palette(&mut Reader::new(&[0, 1]), 4096, 4, 8, 16).unwrap(),
            vec![1; 4096]
        );
        let mut bytes = vec![4, 2, 0, 1];
        bytes.extend(0x10u64.to_be_bytes());
        assert_eq!(
            palette(&mut Reader::new(&bytes), 2, 4, 8, 16).unwrap(),
            vec![0, 1]
        );
        let mut invalid = vec![4, 1, 0];
        invalid.extend(1u64.to_be_bytes());
        assert!(palette(&mut Reader::new(&invalid), 1, 4, 8, 16).is_err());
        assert!(palette(&mut Reader::new(&[32]), 4096, 4, 8, 16).is_err());
        assert!(palette(&mut Reader::new(&[4, 1, 0]), 4096, 4, 8, 16).is_err());
    }
    fn light_update(x: u8, z: u8, sky_bit: u32, empty_block_bit: u32, value: u8) -> Vec<u8> {
        light_masks(
            x,
            z,
            [1u64 << sky_bit, 0, 0, 1u64 << empty_block_bit],
            value,
        )
    }
    /// Masks: sky, block, empty sky, empty block; one sky array, no block arrays.
    fn light_masks(x: u8, z: u8, masks: [u64; 4], value: u8) -> Vec<u8> {
        let mut p = vec![x, z];
        for mask in masks {
            if mask == 0 {
                p.push(0);
            } else {
                p.push(1);
                p.extend(mask.to_be_bytes());
            }
        }
        p.push(1); // one sky array
        p.extend([0x80, 0x10]); // varint 2048
        p.extend([value; 2048]);
        p.push(0); // no block arrays
        p
    }

    #[test]
    fn light_follows_updates_and_block_changes_invalidate_columns() {
        let mut world = World::default();
        world.select_dimension(
            "minecraft:overworld".into(),
            Dimension::new(-64, 384).unwrap(),
        );
        // Updates for unloaded columns are validated but not kept.
        world.update_light(&light_update(0, 0, 1, 2, 0x21)).unwrap();
        assert!(world.column(0, 0).is_none());
        let empty = Arc::new([0; 4096]);
        world.chunks.insert((0, 0), vec![empty.clone(); 24]);
        world.chunks.insert((1, 0), vec![empty; 24]);
        world.update_light(&light_update(0, 0, 1, 2, 0x21)).unwrap();
        world.update_light(&light_update(1, 0, 1, 2, 0x21)).unwrap();
        let (_, light) = world.column(0, 0).unwrap();
        assert_eq!(light.sky.len(), 26);
        assert_eq!(light.sky[1].as_deref().map(|a| a[0]), Some(0x21));
        assert_eq!(light.block[2].as_deref().map(|a| a[0]), Some(0));
        assert!(light.sky[0].is_none());
        // Repeating the same state keeps light; a real change clears it around.
        world.set_block([0, -64, 0], 0).unwrap();
        assert!(world.column(1, 0).unwrap().1.sky[1].is_some());
        world.set_block([0, -64, 0], 1).unwrap();
        assert!(world.column(0, 0).unwrap().1.sky[1].is_none());
        assert!(world.column(1, 0).unwrap().1.sky[1].is_none());
        // Masks outside the dimension or with overlapping bits are rejected.
        assert!(world.update_light(&light_update(0, 0, 30, 2, 0)).is_err());
        assert!(world.update_light(&light_update(0, 0, 3, 3, 0)).is_ok()); // different channels
        assert!(
            world
                .update_light(&light_masks(0, 0, [1 << 3, 0, 1 << 3, 0], 0))
                .is_err()
        );
    }

    #[test]
    fn negative_height_updates_unload_and_dimension_reset_are_distinct() {
        let mut world = World::default();
        world.select_dimension(
            "minecraft:overworld".into(),
            Dimension::new(-64, 384).unwrap(),
        );
        assert_eq!(world.block([0, -64, 0]), None);
        let empty = Arc::new([0; 4096]);
        world.chunks.insert((0, 0), vec![empty; 24]);
        world.set_block([0, -64, 0], 1).unwrap();
        assert_eq!(world.block([0, -64, 0]), Some(1));
        assert_eq!(world.block([0, 319, 0]), Some(0));
        assert_eq!(world.block([0, 320, 0]), None);
        assert!(world.set_block([0, -65, 0], 1).is_err());
        world.unload(&[0; 8]).unwrap();
        assert_eq!(world.block([0, -64, 0]), None);
        world.reset();
        assert!(world.dimension.is_none());
    }
}
