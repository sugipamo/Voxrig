//! Per-version constant tables.
//!
//! Every value, bundled data file and static per-version function that differs
//! between versions is a field here, so adding a field forces both versions to
//! supply it at compile time. Protocol behavior (packet layouts, flows, session
//! state) stays in the version adapters behind the `client::adapter` traits.
use super::MinecraftVersion;
use crate::client::EquipmentSlot;
use crate::{NativeBlockState, Result};
use std::sync::OnceLock;

/// Default dimensions of one entity type, generated from bundled data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EntityDimensions {
    /// Namespaced type name; tables are sorted by it.
    pub name: &'static str,
    pub width: f64,
    pub height: f64,
    /// None where the bundled source does not record it.
    pub eye_height: Option<f64>,
    /// None where living-ness comes from the spawn packet instead.
    pub living: Option<bool>,
}

/// Per-block movement factors (native `f32`), generated from bundled data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BlockPhysics {
    pub name: &'static str,
    /// Slipperiness; the default is `PhysicsConstants::default_slipperiness`.
    pub friction: f32,
    pub speed_factor: f32,
    pub jump_factor: f32,
}

/// Default of a synched entity-data field, as the official server defines it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum EntityDataDefault {
    Byte(i8),
    Int(i32),
    Long(i64),
    Float(f32),
    Bool(bool),
    /// A serializer whose default the common layer does not decode.
    Other,
    /// Not determinable without a world (registry-backed variants and what follows them).
    Unknown,
}

/// One synched entity-data accessor of an entity type, with Mojang-mapped names.
#[derive(Debug)]
pub(crate) struct EntityDataRow {
    /// Namespaced type name; tables are sorted by it, then by index.
    pub entity: &'static str,
    /// Simple name of the class declaring the accessor.
    pub owner: &'static str,
    pub field: &'static str,
    pub index: u8,
    pub default: EntityDataDefault,
}

/// Static per-version functions over bundled registries.
pub(crate) struct RegistryFns {
    pub native_state: fn(i32) -> Result<NativeBlockState>,
    pub state_id: fn(&NativeBlockState) -> Result<i32>,
    /// Cell-relative native collision boxes of a complete state.
    pub collision_boxes: fn(&NativeBlockState) -> Result<Vec<[f64; 6]>>,
    /// Native item id of an item name without its namespace.
    pub item_id: fn(&str) -> Result<i32>,
    /// Namespaced name and maximum stack size of a native item id.
    pub item: fn(i32) -> Result<(String, u32)>,
}

/// Entity values that differ between versions.
pub(crate) struct EntityTable {
    /// LivingEntity health metadata index.
    pub health_metadata_index: u8,
    /// LivingEntity flags metadata index (bit 1 using an item, bit 2 off hand).
    pub living_flags_metadata_index: u8,
    /// Equipment slots indexed by native slot number.
    pub equipment_slots: &'static [EquipmentSlot],
    /// Default dimensions sorted by name.
    pub dimensions: &'static [EntityDimensions],
    /// Synched data accessors per type, sorted by type name then index.
    pub data: &'static [EntityDataRow],
}

/// Bundled data files read by the common layer.
pub(crate) struct DataFiles {
    pub dry_terrain: &'static [u8],
    pub rail_terrain: &'static [u8],
    pub menus: &'static str,
    pub furnace_menus: &'static [u8],
    pub cursor_return_profiles: &'static str,
    pub regular_click_profiles: &'static str,
    pub inventory_transfer_profiles: &'static str,
    pub registry_catalog: &'static str,
    pub crafting_menus: &'static [u8],
    pub crafting_outlines: &'static [u8],
    pub item_properties: &'static str,
    pub mining_tools: &'static [u8],
    /// Official dig profiles (hardness, correct tool, default item speeds) of every state.
    pub dig_profiles: &'static [u8],
    pub storage_outlines: &'static str,
    /// Audited per-state collision and per-block movement behaviour.
    pub movement_blocks: &'static str,
}

/// Player physics constants. Types follow the native evaluation: `f32`
/// values are combined in `f32` before widening, exactly as the game does.
pub(crate) struct PhysicsConstants {
    pub gravity: f64,
    pub vertical_drag: f32,
    pub air_friction: f32,
    pub default_slipperiness: f32,
    pub base_movement_speed: f32,
    /// Ground acceleration numerator (`0.6³ × friction-independent speed`).
    pub ground_acceleration: f32,
    pub air_acceleration: f32,
    pub input_drag: f32,
    pub jump_velocity: f32,
    pub jump_cooldown_ticks: u8,
    pub step_height: f32,
    /// Per-axis velocity below which the axis is zeroed.
    pub small_velocity: f64,
    /// Squared horizontal input below which no acceleration is applied.
    pub min_input_sq: f64,
}

/// Small horizontal velocity cutoff.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SmallVelocity {
    /// Each horizontal axis below `small_velocity` is zeroed.
    PerAxis,
    /// Both horizontal axes are zeroed when the squared length is below 9e-6.
    HorizontalLength,
}

/// Step-up search inside collision resolution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StepSearch {
    /// Older two-candidate search.
    TwoCandidate,
    /// Newer scan over candidate box heights.
    HeightScan,
}

/// Families of the client-side movement procedure that change together between
/// the supported versions: input handling, sprint conditions, push-out from
/// blocks, edge back-off while sneaking, the supporting block and when block
/// effects apply (inside `move` for legacy, after `travel` for modern).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PhysicsGeneration {
    Legacy,
    Modern,
}

/// Version differences in the order or precision of the physics procedure.
pub(crate) struct PhysicsRules {
    pub generation: PhysicsGeneration,
    /// Identity of the client's sprint movement-speed modifier.
    pub sprint_modifier: &'static str,
    /// Sine table indexing: legacy f32 scaling or modern f64 scaling.
    pub modern_trig: bool,
    /// Normalize diagonal input on the unit square before scaling (modern).
    pub normalize_input: bool,
    /// Reciprocal length of over-long input computed through an f32 square root.
    pub f32_input_length: bool,
    pub small_velocity: SmallVelocity,
    /// Jumping keeps a larger existing upward velocity.
    pub jump_keeps_rising: bool,
    pub step: StepSearch,
    /// Apply position even when collision almost cancelled the motion.
    pub move_when_nearly_stopped: bool,
    /// Horizontal velocity is zeroed only when collision changed it by at
    /// least this much; None compares exactly.
    pub horizontal_collision_tolerance: Option<f64>,
    /// Moving shifts the persisted bounding box and the position is read back
    /// from its center (legacy); otherwise the position moves and the box follows.
    pub position_from_bounds: bool,
}

/// Everything that differs between versions as values.
pub(crate) struct VersionTable {
    /// Exact release name.
    pub name: &'static str,
    /// Wire protocol number.
    pub protocol: i32,
    pub registry: RegistryFns,
    pub entities: EntityTable,
    /// Obfuscated class of the original generic container `Slot`.
    pub generic_slot_class: &'static str,
    pub physics: PhysicsConstants,
    pub physics_rules: PhysicsRules,
    /// Blocks with non-default movement factors, sorted by name.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "read by the shared physics engine from P3")
    )]
    pub block_physics: &'static [BlockPhysics],
    pub data: DataFiles,
}

macro_rules! data_files {
    ($v:literal) => {
        DataFiles {
            dry_terrain: include_bytes!(concat!(
                "../../data/client_api/dry_terrain-",
                $v,
                ".json.gz"
            )),
            rail_terrain: include_bytes!(concat!(
                "../../data/client_api/rail_terrain-",
                $v,
                ".json.gz"
            )),
            menus: include_str!(concat!("../../data/client_api/menus-", $v, ".json")),
            furnace_menus: include_bytes!(concat!(
                "../../data/client_api/furnace_menus-",
                $v,
                ".json.gz"
            )),
            cursor_return_profiles: include_str!(concat!(
                "../../data/client_api/cursor_return_profiles-",
                $v,
                ".json"
            )),
            regular_click_profiles: include_str!(concat!(
                "../../data/client_api/regular_click_profiles-",
                $v,
                ".json"
            )),
            inventory_transfer_profiles: include_str!(concat!(
                "../../data/client_api/inventory_transfer_profiles-",
                $v,
                ".json"
            )),
            registry_catalog: include_str!(concat!(
                "../../data/client_api/registry_catalog-",
                $v,
                ".json"
            )),
            crafting_menus: include_bytes!(concat!(
                "../../data/client_api/crafting_menus-",
                $v,
                ".json.gz"
            )),
            crafting_outlines: include_bytes!(concat!(
                "../../data/client_api/crafting_outlines-",
                $v,
                ".json.gz"
            )),
            item_properties: include_str!(concat!(
                "../../data/client_api/item_properties-",
                $v,
                ".json"
            )),
            mining_tools: include_bytes!(concat!(
                "../../data/client_api/mining_tools-",
                $v,
                ".json.gz"
            )),
            dig_profiles: include_bytes!(concat!(
                "../../data/client_api/dig_profiles-",
                $v,
                ".json.gz"
            )),
            storage_outlines: include_str!(concat!(
                "../../data/client_api/storage_outlines-",
                $v,
                ".json"
            )),
            movement_blocks: include_str!(concat!(
                "../../data/client_api/movement_blocks-",
                $v,
                ".json"
            )),
        }
    };
}

fn legacy_collision_boxes(state: &NativeBlockState) -> Result<Vec<[f64; 6]>> {
    let id = super::java_1_16_1::state_id(state)?;
    Ok(super::java_1_16_1::collision::shapes_for(id)
        .iter()
        .map(|aabb| aabb.bounds())
        .collect())
}
fn invalid(message: &str) -> crate::Error {
    crate::Error::new(crate::ErrorKind::InvalidInput, anyhow::anyhow!("{message}"))
}
fn legacy_item_id(name: &str) -> Result<i32> {
    super::java_1_16_1::item_id(name)
        .ok_or_else(|| invalid("unknown item; update Voxrig for new definitions"))
}
fn legacy_item(value: i32) -> Result<(String, u32)> {
    let name =
        super::java_1_16_1::item_name(value).ok_or_else(|| invalid("unknown native item ID"))?;
    let size = super::java_1_16_1::registry::item_max_stack_size(value)
        .ok_or_else(|| invalid("invalid native item capacity"))?;
    Ok((format!("minecraft:{name}"), u32::from(size)))
}
fn modern_item_id(name: &str) -> Result<i32> {
    Ok(super::java_1_21_11::operations::default_item(name, 1)?.item_id)
}
fn modern_item(value: i32) -> Result<(String, u32)> {
    super::java_1_21_11::operations::item_definition(value)
}
fn modern_collision_boxes(state: &NativeBlockState) -> Result<Vec<[f64; 6]>> {
    Ok(super::java_1_21_11::raycast::collision_boxes(state)?.to_vec())
}

use EquipmentSlot::*;

/// Constants shared by both current versions; a later version may differ.
const PHYSICS: PhysicsConstants = PhysicsConstants {
    gravity: 0.08,
    vertical_drag: 0.98,
    air_friction: 0.91,
    default_slipperiness: 0.6,
    base_movement_speed: 0.1,
    ground_acceleration: 0.21600002,
    air_acceleration: 0.02,
    input_drag: 0.98,
    jump_velocity: 0.42,
    jump_cooldown_ticks: 10,
    step_height: 0.6,
    small_velocity: 0.003,
    min_input_sq: 1e-7,
};

pub(crate) const JAVA_1_16_1: VersionTable = VersionTable {
    name: "1.16.1",
    protocol: 736,
    registry: RegistryFns {
        native_state: super::java_1_16_1::native_state,
        state_id: super::java_1_16_1::state_id,
        collision_boxes: legacy_collision_boxes,
        item_id: legacy_item_id,
        item: legacy_item,
    },
    entities: EntityTable {
        health_metadata_index: 8,
        living_flags_metadata_index: 7,
        equipment_slots: &[MainHand, OffHand, Feet, Legs, Chest, Head],
        dimensions: super::java_1_16_1::generated::ENTITY_DIMENSIONS,
        data: super::java_1_16_1::generated::ENTITY_DATA,
    },
    block_physics: super::java_1_16_1::generated::BLOCK_PHYSICS,
    generic_slot_class: "bhw",
    physics: PHYSICS,
    physics_rules: PhysicsRules {
        generation: PhysicsGeneration::Legacy,
        sprint_modifier: "662a6b8d-da3e-4c1c-8813-96ea6097278d",
        modern_trig: false,
        normalize_input: false,
        f32_input_length: true,
        small_velocity: SmallVelocity::PerAxis,
        jump_keeps_rising: false,
        step: StepSearch::TwoCandidate,
        move_when_nearly_stopped: false,
        horizontal_collision_tolerance: None,
        position_from_bounds: true,
    },
    data: data_files!("1.16.1"),
};

pub(crate) const JAVA_1_21_11: VersionTable = VersionTable {
    name: "1.21.11",
    protocol: 774,
    registry: RegistryFns {
        native_state: super::java_1_21_11::native_state,
        state_id: super::java_1_21_11::state_id,
        collision_boxes: modern_collision_boxes,
        item_id: modern_item_id,
        item: modern_item,
    },
    entities: EntityTable {
        health_metadata_index: 9,
        living_flags_metadata_index: 8,
        equipment_slots: &[MainHand, OffHand, Feet, Legs, Chest, Head, Body, Saddle],
        dimensions: super::java_1_21_11::generated::ENTITY_DIMENSIONS,
        data: super::java_1_21_11::generated::ENTITY_DATA,
    },
    block_physics: super::java_1_21_11::generated::BLOCK_PHYSICS,
    generic_slot_class: "dji",
    physics: PHYSICS,
    physics_rules: PhysicsRules {
        generation: PhysicsGeneration::Modern,
        sprint_modifier: "minecraft:sprinting",
        modern_trig: true,
        normalize_input: true,
        f32_input_length: false,
        small_velocity: SmallVelocity::HorizontalLength,
        jump_keeps_rising: true,
        step: StepSearch::HeightScan,
        move_when_nearly_stopped: true,
        horizontal_collision_tolerance: Some(1e-5),
        position_from_bounds: false,
    },
    data: data_files!("1.21.11"),
};

impl MinecraftVersion {
    /// The constant table of this version.
    pub(crate) const fn table(self) -> &'static VersionTable {
        match self {
            Self::Java1_16_1 => &JAVA_1_16_1,
            Self::Java1_21_11 => &JAVA_1_21_11,
        }
    }
    const fn index(self) -> usize {
        match self {
            Self::Java1_16_1 => 0,
            Self::Java1_21_11 => 1,
        }
    }
}

impl VersionTable {
    /// Default dimensions of a namespaced entity type.
    pub(crate) fn entity_dimensions(&self, name: &str) -> Option<&'static EntityDimensions> {
        let table = self.entities.dimensions;
        table
            .binary_search_by(|row| row.name.cmp(name))
            .ok()
            .map(|index| &table[index])
    }
    /// Synched data accessors of a namespaced entity type, ordered by index.
    pub(crate) fn entity_data(&self, name: &str) -> &'static [EntityDataRow] {
        let table = self.entities.data;
        let start = table.partition_point(|row| row.entity < name);
        let end = table.partition_point(|row| row.entity <= name);
        &table[start..end]
    }
    /// Movement factors of a namespaced block; unlisted blocks use the defaults.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "read by the shared physics engine from P3")
    )]
    pub(crate) fn block_physics(&self, name: &str) -> (f32, f32, f32) {
        self.block_physics
            .binary_search_by(|row| row.name.cmp(name))
            .map_or((self.physics.default_slipperiness, 1.0, 1.0), |index| {
                let row = &self.block_physics[index];
                (row.friction, row.speed_factor, row.jump_factor)
            })
    }
    /// Common equipment slot for a native slot number.
    pub(crate) fn equipment_slot(&self, native: u8) -> Option<EquipmentSlot> {
        self.entities
            .equipment_slots
            .get(usize::from(native))
            .copied()
    }
}

/// A value derived once per version from that version's table.
pub(crate) struct PerVersion<T>([OnceLock<T>; 2]);
impl<T> PerVersion<T> {
    pub(crate) const fn new() -> Self {
        Self([OnceLock::new(), OnceLock::new()])
    }
    pub(crate) fn get(
        &self,
        version: MinecraftVersion,
        init: impl FnOnce(&'static VersionTable) -> T,
    ) -> &T {
        self.0[version.index()].get_or_init(|| init(version.table()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_identify_their_version() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let table = version.table();
            assert_eq!(table.protocol, version.protocol());
            assert_eq!(table.name, version.name());
            assert!(table.entities.dimensions.is_sorted_by_key(|row| row.name));
            assert!(
                table
                    .entities
                    .data
                    .is_sorted_by_key(|row| (row.entity, row.index))
            );
        }
    }

    /// The exported accessor tables agree with the hand-written living indices.
    #[test]
    fn entity_data_tables_agree_with_living_indices() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let table = version.table();
            let zombie = table.entity_data("minecraft:zombie");
            let index = |field: &str| zombie.iter().find(|row| row.field == field).unwrap().index;
            assert_eq!(
                index("DATA_HEALTH_ID"),
                table.entities.health_metadata_index
            );
            assert_eq!(
                index("DATA_LIVING_ENTITY_FLAGS"),
                table.entities.living_flags_metadata_index
            );
            for name in table.entities.dimensions.iter().map(|row| row.name) {
                assert!(!table.entity_data(name).is_empty(), "{name}");
            }
        }
        let creeper = MinecraftVersion::Java1_21_11
            .table()
            .entity_data("minecraft:creeper");
        let ignited = creeper
            .iter()
            .find(|row| row.field == "DATA_IS_IGNITED")
            .unwrap();
        assert_eq!(
            (ignited.index, ignited.default),
            (18, EntityDataDefault::Bool(false))
        );
    }

    /// Generated tables record their source digest; regenerate with
    /// `python3 scripts/generate_version_tables.py` when this fails.
    #[test]
    fn generated_tables_match_their_sources() {
        use sha2::{Digest, Sha256};
        for (generated, source) in [
            (
                include_str!("java_1_16_1/generated.rs"),
                &include_bytes!("../../data/entities.json")[..],
            ),
            (
                include_str!("java_1_21_11/generated.rs"),
                &include_bytes!("../../data/java_1_21_11/entity_dimensions.json")[..],
            ),
            (
                include_str!("java_1_16_1/generated.rs"),
                &include_bytes!("../../data/client_api/block_physics-1.16.1.json")[..],
            ),
            (
                include_str!("java_1_16_1/generated.rs"),
                &include_bytes!("../../data/client_api/entity_data-1.16.1.json")[..],
            ),
            (
                include_str!("java_1_21_11/generated.rs"),
                &include_bytes!("../../data/client_api/entity_data-1.21.11.json")[..],
            ),
            (
                include_str!("java_1_21_11/generated.rs"),
                &include_bytes!("../../data/client_api/block_physics-1.21.11.json")[..],
            ),
        ] {
            let digest: String = Sha256::digest(source)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            assert!(
                generated.contains(&format!("(sha256 {digest})")),
                "stale generated table"
            );
        }
        let pig = MinecraftVersion::Java1_21_11
            .table()
            .entity_dimensions("minecraft:pig")
            .unwrap();
        assert_eq!((pig.width, pig.height, pig.living), (0.9, 0.9, Some(true)));
        assert_eq!(
            MinecraftVersion::Java1_16_1.table().equipment_slot(5),
            Some(EquipmentSlot::Head)
        );
        assert_eq!(MinecraftVersion::Java1_16_1.table().equipment_slot(6), None);
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let table = version.table();
            assert!(table.block_physics.is_sorted_by_key(|row| row.name));
            assert_eq!(table.block_physics("minecraft:stone"), (0.6, 1.0, 1.0));
            assert_eq!(table.block_physics("minecraft:blue_ice"), (0.989, 1.0, 1.0));
            assert_eq!(
                table.block_physics("minecraft:honey_block"),
                (0.6, 0.4, 0.5)
            );
        }
    }
}
