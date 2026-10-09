//! Embedded Minecraft 1.16.1 registry and mining-time queries.

#[path = "registry_harvest_corrections.rs"]
mod harvest_corrections;

use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

pub(crate) const BLOCK_REGISTRY_IDENTITY: crate::BlockRegistryIdentity =
    crate::BlockRegistryIdentity {
        protocol_version: crate::versions::java_1_16_1::protocol::PROTOCOL_VERSION,
        minecraft_version: "1.16.1",
        descriptor_revision: 3,
    };

type DescriptorSlot = OnceLock<Option<Arc<crate::BlockPhysicalDescriptor>>>;

struct DescriptorCache {
    slots: Vec<DescriptorSlot>,
}

impl DescriptorCache {
    fn new() -> Self {
        let max_state_id = registry()
            .blocks
            .iter()
            .filter_map(|block| usize::try_from(block.max_state_id).ok())
            .max()
            .unwrap_or(0);
        Self {
            slots: (0..=max_state_id).map(|_| OnceLock::new()).collect(),
        }
    }

    fn get(&self, state_id: i32) -> crate::BlockPhysicalDescriptorLookup {
        let Some(slot) = usize::try_from(state_id)
            .ok()
            .and_then(|index| self.slots.get(index))
        else {
            return crate::BlockPhysicalDescriptorLookup::UnknownStateId;
        };
        match slot.get_or_init(|| build_physical_descriptor(state_id)) {
            Some(descriptor) => crate::BlockPhysicalDescriptorLookup::Known(Arc::clone(descriptor)),
            None => crate::BlockPhysicalDescriptorLookup::UnknownStateId,
        }
    }
}

pub(crate) fn block_physical_descriptor(state_id: i32) -> crate::BlockPhysicalDescriptorLookup {
    // Initialize only the requested state. Each slot shares both successful and
    // failed resolution across concurrent callers, without locking other states.
    static CACHE: OnceLock<DescriptorCache> = OnceLock::new();
    CACHE.get_or_init(DescriptorCache::new).get(state_id)
}

fn build_physical_descriptor(state_id: i32) -> Option<Arc<crate::BlockPhysicalDescriptor>> {
    const MAX_PROPERTIES: usize = 32;
    const MAX_SHAPES: usize = 64;
    const MAX_TOOL_FACTS: usize = 256;
    const MAX_TEXT_BYTES: usize = 128;
    let name = block_name_from_state(state_id)?;
    let properties = block_state_properties(state_id)?;
    let collision_shapes = crate::block_collision_shapes(state_id)?;
    let collision = crate::block_collision(state_id)?;
    let support_surface = crate::block_support_surface(state_id)?;
    let movement_registry = block_movement_registry_facts(state_id)?;
    let type_min_state_id = movement_registry.type_min_state_id?;
    let text_valid = |text: &str| {
        !text.is_empty() && text.len() <= MAX_TEXT_BYTES && !text.chars().any(char::is_control)
    };
    if !text_valid(name)
        || properties.len() > MAX_PROPERTIES
        || collision_shapes.len() > MAX_SHAPES
        || properties
            .iter()
            .any(|(key, value)| !text_valid(key) || !text_valid(value))
        || collision_shapes.iter().any(|shape| {
            [
                shape.min_x,
                shape.min_y,
                shape.min_z,
                shape.max_x,
                shape.max_y,
                shape.max_z,
            ]
            .iter()
            .any(|value| !value.is_finite())
        })
        || movement_registry
            .hardness
            .is_some_and(|value| !value.is_finite())
        || movement_registry
            .material
            .as_deref()
            .is_some_and(|value| !text_valid(value))
        || movement_registry
            .harvest_tools
            .as_ref()
            .is_some_and(|facts| {
                facts.len() > MAX_TOOL_FACTS || facts.iter().any(|(key, _)| !text_valid(key))
            })
        || movement_registry
            .material_tool_multipliers
            .as_ref()
            .is_some_and(|facts| {
                facts.len() > MAX_TOOL_FACTS
                    || facts
                        .iter()
                        .any(|(key, value)| !text_valid(key) || !value.is_finite())
            })
    {
        return None;
    }
    Some(Arc::new(crate::BlockPhysicalDescriptor {
        state_id,
        catalog: crate::BlockCatalogIdentity {
            canonical_name: name.to_owned(),
            type_min_state_id,
        },
        properties,
        collision_shapes,
        collision,
        support_surface,
        movement_registry,
    }))
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BlockData {
    name: String,
    hardness: Option<f64>,
    min_state_id: i32,
    max_state_id: i32,
    default_state: i32,
    emit_light: Option<u8>,
    filter_light: Option<u8>,
    transparent: Option<bool>,
    #[serde(default)]
    diggable: bool,
    material: Option<String>,
    #[serde(default)]
    bounding_box: Option<String>,
    harvest_tools: Option<HashMap<String, bool>>,
    #[serde(default)]
    states: Vec<BlockStateData>,
}

/// Only certify native opaque, non-emitting full cubes. Per-block defaults do
/// not establish the optical behavior of arbitrary state-dependent variants.
pub(crate) fn lighting_equivalent(before: i32, after: i32) -> bool {
    if before == after {
        return true;
    }
    static OPAQUE: OnceLock<Vec<bool>> = OnceLock::new();
    let opaque = OPAQUE.get_or_init(|| {
        let blocks = &registry().blocks;
        let max = blocks.iter().map(|b| b.max_state_id).max().unwrap_or(0);
        let mut certified = vec![false; usize::try_from(max).unwrap_or(0) + 1];
        for block in blocks {
            let static_state = block.states.is_empty() && block.min_state_id == block.max_state_id;
            // These properties only change appearance/orientation on a full
            // opaque cube. Lit, waterlogged, charges and other properties are
            // deliberately not inferred from minecraft-data's type defaults.
            let passive_variant = block.states.len() == 1
                && ((matches!(block.name.as_str(), "grass_block" | "podzol")
                    && block.states[0].name == "snowy"
                    && block.states[0].kind == "bool"
                    && block.states[0].num_values == 2
                    && block.max_state_id - block.min_state_id == 1)
                    || ((block.name.ends_with("_log") || block.name.ends_with("_wood"))
                        && block.states[0].name == "axis"
                        && block.states[0].kind == "enum"
                        && block.states[0].values == ["x", "y", "z"]
                        && block.max_state_id - block.min_state_id == 2));
            if !(static_state || passive_variant)
                || block.emit_light != Some(0)
                || block.filter_light != Some(15)
                || block.transparent != Some(false)
                || block.min_state_id < 0
                || !(block.min_state_id..=block.max_state_id).contains(&block.default_state)
                || !(block.min_state_id..=block.max_state_id).all(|id| {
                    crate::block_collision_shapes(id).is_some_and(|shapes| {
                        shapes.len() == 1
                            && shapes[0].min_x == 0.0
                            && shapes[0].min_y == 0.0
                            && shapes[0].min_z == 0.0
                            && shapes[0].max_x == 1.0
                            && shapes[0].max_y == 1.0
                            && shapes[0].max_z == 1.0
                    })
                })
            {
                continue;
            }
            for id in block.min_state_id..=block.max_state_id {
                certified[id as usize] = true;
            }
        }
        certified
    });
    let is_certified = |id| {
        usize::try_from(id)
            .ok()
            .and_then(|id| opaque.get(id))
            .copied()
            .unwrap_or(false)
    };
    is_certified(before) && is_certified(after)
}

pub(crate) fn placed_block_physical_descriptor(
    canonical_name: &str,
) -> crate::PlacedBlockPhysicalDescriptorLookup {
    let Some(block) = registry()
        .blocks
        .iter()
        .find(|block| block.name == canonical_name)
    else {
        return crate::PlacedBlockPhysicalDescriptorLookup::UnknownCatalog;
    };
    if !(block.min_state_id..=block.max_state_id).contains(&block.default_state) {
        return crate::PlacedBlockPhysicalDescriptorLookup::UnknownRegistry;
    }
    let crate::BlockPhysicalDescriptorLookup::Known(default) =
        block_physical_descriptor(block.default_state)
    else {
        return crate::PlacedBlockPhysicalDescriptorLookup::UnknownRegistry;
    };
    for state_id in block.min_state_id..=block.max_state_id {
        let crate::BlockPhysicalDescriptorLookup::Known(variant) =
            block_physical_descriptor(state_id)
        else {
            return crate::PlacedBlockPhysicalDescriptorLookup::UnknownRegistry;
        };
        if variant.catalog().canonical_name() != default.catalog().canonical_name()
            || variant.collision_shapes() != default.collision_shapes()
            || variant.collision() != default.collision()
            || variant.support_surface() != default.support_surface()
            || variant.movement_registry() != default.movement_registry()
        {
            return crate::PlacedBlockPhysicalDescriptorLookup::UnsupportedVariant;
        }
    }
    crate::PlacedBlockPhysicalDescriptorLookup::Known(default)
}

/// Raw `minecraft-data` bounding-box classification for a block registry
/// entry. This is registry data only; traversal and safety remain consumer
/// responsibilities.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RawBlockBoundingBox {
    /// The registry reports an empty bounding box.
    Empty,
    /// The registry reports a block bounding box, including partial shapes
    /// such as top and bottom slabs.
    Block,
}

#[derive(Clone, Debug, Deserialize)]
struct BlockStateData {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "num_values")]
    num_values: usize,
    #[serde(default)]
    values: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct ItemData {
    id: i32,
    name: String,
    #[serde(rename = "stackSize")]
    stack_size: i8,
    #[serde(default, rename = "maxDurability")]
    max_durability: u32,
}

#[derive(Clone, Debug, Deserialize)]
struct EntityData {
    id: i32,
    name: String,
    width: f64,
    height: f64,
}

#[derive(Clone, Debug, Deserialize)]
struct SoundData {
    id: i32,
    name: String,
}

/// Static registry metadata, not a limit on observed command-created equipment.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EnchantmentDefinition {
    /// Version-pinned enchantment ID.
    pub id: i32,
    /// Registry enchantment name.
    pub name: String,
    /// Vanilla maximum level.
    pub max_level: u16,
}

/// Resolve a vanilla enchantment name using the version-pinned registry.
pub fn enchantment_definition(name: &str) -> Option<&'static EnchantmentDefinition> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    registry()
        .enchantments
        .iter()
        .find(|definition| definition.name == name)
}

struct Registry {
    enchantments: Vec<EnchantmentDefinition>,
    blocks: Vec<BlockData>,
    items: HashMap<i32, ItemData>,
    materials: HashMap<String, HashMap<String, f64>>,
    recipes: HashMap<String, Vec<RawRecipe>>,
    entities: HashMap<i32, (String, f64, f64)>,
    sounds: HashMap<i32, String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// State and protocol data represented by `MiningInfo`.
pub struct MiningInfo {
    /// The `diggable` value.
    pub diggable: bool,
    /// The `harvestable` value.
    pub harvestable: bool,
    /// The `effective_tool` value.
    pub effective_tool: bool,
    /// The `predicted_ticks` value.
    pub predicted_ticks: Option<u64>,
}

/// Raw block registry facts consumed by an external movement provider.
///
/// These values describe the bundled protocol-736 registry only. They do not
/// classify a block as safe, passable, liquid, or otherwise executable.
#[derive(Clone, Debug, PartialEq)]
pub struct RawBlockMovementRegistryFact {
    /// Minimum state ID of the owning minecraft-data block type. Consumers
    /// may use this raw registry identity to reproduce version-pinned lookup
    /// rules; it is not a movement classification. `None` is fail-closed.
    pub type_min_state_id: Option<i32>,
    /// Whether the registry marks the block as diggable.
    pub diggable: bool,
    /// Registry hardness, when supplied by minecraft-data.
    pub hardness: Option<f64>,
    /// Registry material name, when supplied.
    pub material: Option<String>,
    /// Raw registry bounding-box classification, or `None` when absent or
    /// outside the pinned finite vocabulary.
    pub bounding_box: Option<RawBlockBoundingBox>,
    /// Raw harvest-tool map in deterministic key order.
    pub harvest_tools: Option<Vec<(String, bool)>>,
    /// Raw material tool-speed map in deterministic item-ID key order.
    ///
    /// `None` means that the block's material is absent or unresolved; it is
    /// not an inferred hand-speed default.
    pub material_tool_multipliers: Option<Vec<(String, f64)>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
/// State and protocol data represented by `RecipeResult`.
pub struct RecipeResult {
    /// The `id` value.
    pub id: i32,
    /// The `count` value.
    pub count: i32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
/// State and protocol data represented by `RawRecipe`.
pub struct RawRecipe {
    /// The `result` value.
    pub result: RecipeResult,
    #[serde(default)]
    /// The `ingredients` value.
    pub ingredients: Option<Vec<i32>>,
    #[serde(default)]
    /// The `in_shape` value.
    pub in_shape: Option<Vec<Vec<Option<i32>>>>,
    #[serde(default)]
    /// The `out_shape` value.
    pub out_shape: Option<Vec<Vec<Option<i32>>>>,
}

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| Registry {
        enchantments: serde_json::from_str(include_str!("../../../data/enchantments.json"))
            .expect("embedded enchantment registry must be valid"),
        blocks: harvest_corrections::blocks()
            .expect("embedded 1.16.1 harvest gate corrections must match their source"),
        items: serde_json::from_str::<Vec<ItemData>>(include_str!("../../../data/items.json"))
            .expect("embedded 1.16.1 items registry must be valid")
            .into_iter()
            .map(|mut item| {
                // Preserve the source-pinned minecraft-data input used by harvest
                // audits; effective capacities come from the original native item
                // oracle. This corrects warped_fungus_on_a_stick (64 -> 1) too.
                item.stack_size = crate::client::inventory::slot_policy::default_item_capacity(
                    crate::MinecraftVersion::Java1_16_1,
                    item.id,
                    &format!("minecraft:{}", item.name),
                )
                .and_then(|n| i8::try_from(n).ok())
                .expect("native item capacity must match pinned ID/name");
                (item.id, item)
            })
            .collect(),
        materials: serde_json::from_str(include_str!("../../../data/materials.json"))
            .expect("embedded 1.16.1 materials registry must be valid"),
        recipes: serde_json::from_str(include_str!("../../../data/recipes.json"))
            .expect("embedded 1.16.1 recipes registry must be valid"),
        entities: serde_json::from_str::<Vec<EntityData>>(include_str!(
            "../../../data/entities.json"
        ))
        .expect("embedded 1.16.1 entities registry must be valid")
        .into_iter()
        .map(|entity| (entity.id, (entity.name, entity.width, entity.height)))
        .collect(),
        sounds: serde_json::from_str::<Vec<SoundData>>(include_str!("../../../data/sounds.json"))
            .expect("embedded 1.16.1 sounds registry must be valid")
            .into_iter()
            .map(|sound| (sound.id, sound.name))
            .collect(),
    })
}

/// Performs the `recipes_for_output` operation.
pub fn recipes_for_output(item_id: i32) -> &'static [RawRecipe] {
    registry()
        .recipes
        .get(&item_id.to_string())
        .map_or(&[], Vec::as_slice)
}

/// Performs the `entity_name` operation.
pub fn entity_name(id: i32) -> Option<&'static str> {
    registry()
        .entities
        .get(&id)
        .map(|(name, _, _)| name.as_str())
}

/// Performs the `entity_dimensions` operation.
pub fn entity_dimensions(id: i32) -> Option<(f64, f64)> {
    registry()
        .entities
        .get(&id)
        .map(|(_, width, height)| (*width, *height))
}

/// Performs the `sound_name` operation.
pub fn sound_name(id: i32) -> Option<&'static str> {
    registry().sounds.get(&id).map(String::as_str)
}

/// Performs the `item_name` operation.
pub fn item_name(id: i32) -> Option<&'static str> {
    registry().items.get(&id).map(|item| item.name.as_str())
}

/// Resolves a canonical item name to its protocol item id for Caller-supplied
/// cache effects. This is a private adapter fact, not recipe selection.
pub fn item_id(name: &str) -> Option<i32> {
    registry()
        .items
        .iter()
        .find_map(|(id, item)| (item.name == name).then_some(*id))
}

/// Identity of the bundled definitions used by both item and block lookups.
pub const fn registry_identity() -> crate::BlockRegistryIdentity {
    BLOCK_REGISTRY_IDENTITY
}

/// Maximum stack size from this client's version-pinned item registry.
/// Unknown IDs and invalid capacities are not replaced by a default. Consumers
/// must keep this fact associated with the same registry version as item IDs.
pub fn item_max_stack_size(id: i32) -> Option<u8> {
    let item = registry().items.get(&id)?;
    u8::try_from(item.stack_size).ok().filter(|size| *size > 0)
}

/// Version-pinned maximum damage. Zero means a known non-damageable item;
/// None means an unknown item ID. This is a definition, not observed wear.
pub fn item_max_durability(id: i32) -> Option<u32> {
    registry().items.get(&id).map(|item| item.max_durability)
}

pub(crate) fn item_stack_size(id: i32) -> i8 {
    registry().items.get(&id).map_or(64, |item| item.stack_size)
}

/// Performs the `block_name_from_state` operation.
pub fn block_name_from_state(state_id: i32) -> Option<&'static str> {
    registry()
        .blocks
        .iter()
        .find(|block| (block.min_state_id..=block.max_state_id).contains(&state_id))
        .map(|block| block.name.as_str())
}

/// Returns ranges for exact canonical block names from the embedded registry.
/// Unknown names have no range; this does not inspect loaded world data.
pub fn block_state_ranges_for_names(names: &[&str]) -> Vec<(i32, i32)> {
    registry()
        .blocks
        .iter()
        .filter(|block| names.contains(&block.name.as_str()))
        .map(|block| (block.min_state_id, block.max_state_id))
        .collect()
}

/// Returns canonical state properties for one known block state.
///
/// The returned values are raw registry facts, not a collision or safety
/// judgment. Unknown state IDs and malformed registry ranges return `None`.
pub fn block_state_properties(state_id: i32) -> Option<Vec<(String, String)>> {
    let block = registry()
        .blocks
        .iter()
        .find(|block| (block.min_state_id..=block.max_state_id).contains(&state_id))?;
    let mut offset = usize::try_from(state_id - block.min_state_id).ok()?;
    let mut properties = Vec::with_capacity(block.states.len());
    for state in block.states.iter().rev() {
        if state.num_values == 0 {
            return None;
        }
        let value_index = offset % state.num_values;
        offset /= state.num_values;
        let value = match state.kind.as_str() {
            // The embedded minecraft-data state tables enumerate boolean
            // values as true, then false.  In particular the default furnace
            // state is 3374 (the second state in its range), and is
            // `lit=false`; reversing this order makes every idle furnace
            // appear lit to the caller's safety gate.
            "bool" => match value_index {
                0 => "true".to_owned(),
                1 => "false".to_owned(),
                _ => return None,
            },
            "int" => value_index.to_string(),
            "enum" => state.values.get(value_index)?.clone(),
            _ => return None,
        };
        properties.push((state.name.clone(), value));
    }
    if offset != 0 {
        return None;
    }
    properties.reverse();
    Some(properties)
}

/// Returns raw movement-related registry facts for one known block state.
///
/// Unknown state IDs and malformed registry records return `None`. Safety and
/// route semantics remain with the consuming application layer.
pub fn block_movement_registry_facts(state_id: i32) -> Option<RawBlockMovementRegistryFact> {
    let registry = registry();
    let block = registry
        .blocks
        .iter()
        .find(|block| (block.min_state_id..=block.max_state_id).contains(&state_id))?;
    let harvest_tools = block.harvest_tools.as_ref().map(|tools| {
        let mut tools = tools
            .iter()
            .map(|(tool, harvestable)| (tool.clone(), *harvestable))
            .collect::<Vec<_>>();
        tools.sort_by(|left, right| left.0.cmp(&right.0));
        tools
    });
    let material_tool_multipliers = block.material.as_ref().and_then(|material| {
        registry.materials.get(material).map(|tools| {
            let mut tools = tools
                .iter()
                .map(|(item_id, multiplier)| (item_id.clone(), *multiplier))
                .collect::<Vec<_>>();
            tools.sort_by(|left, right| left.0.cmp(&right.0));
            tools
        })
    });
    let bounding_box = match block.bounding_box.as_deref() {
        Some("empty") => Some(RawBlockBoundingBox::Empty),
        Some("block") => Some(RawBlockBoundingBox::Block),
        _ => None,
    };
    Some(RawBlockMovementRegistryFact {
        type_min_state_id: Some(block.min_state_id),
        diggable: block.diggable,
        hardness: block.hardness,
        material: block.material.clone(),
        bounding_box,
        harvest_tools,
        material_tool_multipliers,
    })
}

/// Performs the `mining_info` operation.
pub fn mining_info(state_id: i32, tool_id: Option<i32>) -> Option<MiningInfo> {
    let registry = registry();
    let block = registry
        .blocks
        .iter()
        .find(|block| (block.min_state_id..=block.max_state_id).contains(&state_id))?;
    if !block.diggable {
        return Some(MiningInfo {
            diggable: false,
            harvestable: false,
            effective_tool: false,
            predicted_ticks: None,
        });
    }
    let hardness = block.hardness?;
    if hardness <= 0.0 {
        return Some(MiningInfo {
            diggable: true,
            harvestable: true,
            effective_tool: true,
            predicted_ticks: Some(0),
        });
    }
    let tool_key = tool_id.map(|id| id.to_string());
    let tool_speed = block
        .material
        .as_ref()
        .and_then(|material| registry.materials.get(material))
        .and_then(|tools| tool_key.as_ref().and_then(|key| tools.get(key)))
        .copied()
        .unwrap_or(1.0);
    let harvestable = block.harvest_tools.as_ref().is_none_or(|tools| {
        tool_key
            .as_ref()
            .is_some_and(|key| tools.get(key).copied().unwrap_or(false))
    });
    let damage_per_tick = tool_speed / hardness / if harvestable { 30.0 } else { 100.0 };
    Some(MiningInfo {
        diggable: true,
        harvestable,
        effective_tool: tool_speed > 1.0,
        predicted_ticks: Some((1.0 / damage_per_tick).ceil() as u64),
    })
}

pub(crate) fn mining_ticks(state_id: i32, tool_id: Option<i32>) -> Option<u64> {
    mining_info(state_id, tool_id)?.predicted_ticks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enchantment_registry_has_unique_ids_and_names_and_resolves_unknowns() {
        let mut ids = std::collections::BTreeSet::new();
        let mut names = std::collections::BTreeSet::new();
        for row in &registry().enchantments {
            assert!(ids.insert(row.id));
            assert!(names.insert(&row.name));
            assert!(row.max_level > 0);
            assert_eq!(enchantment_definition(&row.name), Some(row));
        }
        assert_eq!(
            enchantment_definition("minecraft:silk_touch")
                .unwrap()
                .max_level,
            1
        );
        assert!(enchantment_definition("not_an_enchantment").is_none());
    }

    #[test]
    fn public_block_ranges_resolve_properties_without_world_observation() {
        assert!(block_state_ranges_for_names(&["unknown_block"]).is_empty());
        let ranges = block_state_ranges_for_names(&["oak_log"]);
        assert_eq!(ranges.len(), 1);
        let (first, last) = ranges[0];
        let axes = (first..=last)
            .map(|state| {
                assert_eq!(block_name_from_state(state), Some("oak_log"));
                block_state_properties(state)
                    .unwrap()
                    .into_iter()
                    .find(|(key, _)| key == "axis")
                    .unwrap()
                    .1
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            axes,
            ["x", "y", "z"].into_iter().map(str::to_owned).collect()
        );
        assert_eq!(
            block_state_ranges_for_names(&["oak_log", "oak_log"]),
            ranges
        );
    }

    #[test]
    fn registry_resolves_ids_and_vanilla_mining_times() {
        assert_eq!(item_name(1), Some("stone"));
        assert_eq!(block_name_from_state(1), Some("stone"));
        assert_eq!(mining_ticks(1, None), Some(150));
        assert_eq!(mining_ticks(1, Some(589)), Some(23));
        assert!(!recipes_for_output(589).is_empty());
        assert_eq!(entity_name(5), Some("blaze"));
        assert_eq!(sound_name(0), Some("ambient.cave"));
    }

    #[test]
    fn boolean_block_state_values_match_vanilla_default_states() {
        assert_eq!(
            block_state_properties(3374),
            Some(vec![
                ("facing".to_owned(), "north".to_owned()),
                ("lit".to_owned(), "false".to_owned()),
            ])
        );
        assert_eq!(
            block_state_properties(3373),
            Some(vec![
                ("facing".to_owned(), "north".to_owned()),
                ("lit".to_owned(), "true".to_owned()),
            ])
        );
        assert_eq!(
            block_state_properties(9),
            Some(vec![("snowy".to_owned(), "false".to_owned())])
        );
    }

    #[test]
    fn raw_movement_registry_facts_preserve_registry_values() {
        let stone = block_movement_registry_facts(1).expect("stone must be registered");
        assert!(stone.diggable);
        assert_eq!(stone.hardness, Some(1.5));
        assert!(stone.material.is_some());
        assert!(
            stone
                .material_tool_multipliers
                .as_ref()
                .is_some_and(|tools| tools.iter().any(|(id, speed)| id == "573" && *speed == 6.0))
        );
        assert_eq!(stone.bounding_box, Some(RawBlockBoundingBox::Block));
        assert_eq!(stone.type_min_state_id, Some(1));
        assert_eq!(
            block_movement_registry_facts(8300)
                .map(|facts| (facts.type_min_state_id, facts.bounding_box,)),
            Some((Some(8300), Some(RawBlockBoundingBox::Block)))
        );
        assert_eq!(
            block_movement_registry_facts(8303)
                .map(|facts| (facts.type_min_state_id, facts.bounding_box,)),
            Some((Some(8300), Some(RawBlockBoundingBox::Block)))
        );
        assert_eq!(
            block_movement_registry_facts(4822).map(|facts| facts.type_min_state_id),
            Some(Some(4820))
        );
        assert_eq!(
            block_movement_registry_facts(0).map(|facts| facts.bounding_box),
            Some(Some(RawBlockBoundingBox::Empty))
        );
        assert_eq!(
            block_movement_registry_facts(9651).and_then(|facts| facts.material_tool_multipliers),
            None
        );
        assert_eq!(entity_dimensions(1), Some((0.5, 1.975)));
        assert_eq!(entity_dimensions(-1), None);
        assert!(block_movement_registry_facts(-1).is_none());
    }

    #[test]
    fn descriptor_cache_is_lazy_and_shared_between_concurrent_callers() {
        let cache = DescriptorCache::new();
        assert!(cache.slots.iter().all(|slot| slot.get().is_none()));
        assert!(matches!(
            cache.get(-1),
            crate::BlockPhysicalDescriptorLookup::UnknownStateId
        ));
        assert!(matches!(
            cache.get(i32::MAX),
            crate::BlockPhysicalDescriptorLookup::UnknownStateId
        ));
        assert!(cache.slots.iter().all(|slot| slot.get().is_none()));
        let values = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..32)
                .map(|_| {
                    scope.spawn(|| {
                        let crate::BlockPhysicalDescriptorLookup::Known(value) = cache.get(0)
                        else {
                            panic!("air descriptor missing");
                        };
                        value
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(values.iter().all(|value| Arc::ptr_eq(&values[0], value)));
        assert_eq!(
            cache
                .slots
                .iter()
                .filter(|slot| slot.get().is_some())
                .count(),
            1
        );
        assert!(matches!(
            cache.get(3356),
            crate::BlockPhysicalDescriptorLookup::Known(_)
        ));
        assert_eq!(
            cache
                .slots
                .iter()
                .filter(|slot| slot.get().is_some())
                .count(),
            2
        );
        let crate::BlockPhysicalDescriptorLookup::Known(air) = cache.get(0) else {
            unreachable!()
        };
        assert!(Arc::ptr_eq(&air, &values[0]));
    }

    #[test]
    fn physical_descriptors_are_bounded_cached_and_match_every_known_state() {
        assert_eq!(BLOCK_REGISTRY_IDENTITY.protocol_version(), 736);
        assert_eq!(BLOCK_REGISTRY_IDENTITY.minecraft_version(), "1.16.1");
        assert_eq!(BLOCK_REGISTRY_IDENTITY.descriptor_revision(), 3);
        for block in &registry().blocks {
            for state_id in block.min_state_id..=block.max_state_id {
                let crate::BlockPhysicalDescriptorLookup::Known(descriptor) =
                    block_physical_descriptor(state_id)
                else {
                    panic!("known state {state_id} lost its physical descriptor");
                };
                assert_eq!(descriptor.state_id(), state_id);
                assert_eq!(descriptor.catalog().canonical_name(), block.name);
                assert_eq!(descriptor.catalog().type_min_state_id(), block.min_state_id);
                assert_eq!(
                    descriptor.properties(),
                    block_state_properties(state_id).unwrap()
                );
                assert_eq!(
                    descriptor.collision_shapes(),
                    crate::block_collision_shapes(state_id).unwrap()
                );
                assert_eq!(
                    descriptor.collision(),
                    crate::block_collision(state_id).unwrap()
                );
                assert_eq!(
                    descriptor.support_surface(),
                    crate::block_support_surface(state_id).unwrap()
                );
                assert_eq!(
                    descriptor.movement_registry(),
                    &block_movement_registry_facts(state_id).unwrap()
                );
                let crate::BlockPhysicalDescriptorLookup::Known(again) =
                    block_physical_descriptor(state_id)
                else {
                    unreachable!();
                };
                assert!(Arc::ptr_eq(&descriptor, &again));
                assert!(format!("{descriptor:?}").len() < 512);
            }
        }
        assert!(matches!(
            block_physical_descriptor(-1),
            crate::BlockPhysicalDescriptorLookup::UnknownStateId
        ));
        assert!(matches!(
            block_physical_descriptor(i32::MAX),
            crate::BlockPhysicalDescriptorLookup::UnknownStateId
        ));
    }

    #[test]
    fn placed_descriptors_use_registry_defaults_and_reject_variant_geometry() {
        for (name, expected_default) in [("crafting_table", 3356), ("furnace", 3374)] {
            let crate::PlacedBlockPhysicalDescriptorLookup::Known(descriptor) =
                placed_block_physical_descriptor(name)
            else {
                panic!("{name} has placement-invariant planning facts")
            };
            assert_eq!(descriptor.state_id(), expected_default);
            assert_eq!(descriptor.catalog().canonical_name(), name);
        }

        assert!(matches!(
            placed_block_physical_descriptor("oak_stairs"),
            crate::PlacedBlockPhysicalDescriptorLookup::UnsupportedVariant
        ));
        assert!(matches!(
            placed_block_physical_descriptor("not_a_registry_block"),
            crate::PlacedBlockPhysicalDescriptorLookup::UnknownCatalog
        ));
    }
    #[test]
    fn item_capacity_resolves_registry_values_without_unknown_defaults() {
        for (name, capacity) in [
            ("stone", 64),
            ("snowball", 16),
            ("wooden_pickaxe", 1),
            ("warped_fungus_on_a_stick", 1),
        ] {
            let id = item_id(name).expect("fixture item exists in pinned registry");
            assert_eq!(item_max_stack_size(id), Some(capacity));
            assert_eq!(item_stack_size(id), capacity as i8);
        }
        assert_eq!(item_max_stack_size(-1), None);
        assert_eq!(item_max_stack_size(i32::MAX), None);
    }

    #[test]
    fn item_durability_resolves_known_definitions_and_keeps_unknown_distinct() {
        for (name, maximum) in [
            ("wooden_pickaxe", 59),
            ("stone_pickaxe", 131),
            ("iron_pickaxe", 250),
            ("diamond_pickaxe", 1561),
            ("golden_pickaxe", 32),
            ("netherite_pickaxe", 2031),
            ("stone", 0),
        ] {
            assert_eq!(item_max_durability(item_id(name).unwrap()), Some(maximum));
        }
        assert_eq!(item_max_durability(-1), None);
        assert_eq!(item_max_durability(i32::MAX), None);
    }
}
