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
    /// Equipment slots indexed by native slot number.
    pub equipment_slots: &'static [EquipmentSlot],
    /// Default dimensions sorted by name.
    pub dimensions: &'static [EntityDimensions],
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
    pub storage_outlines: &'static str,
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
            storage_outlines: include_str!(concat!(
                "../../data/client_api/storage_outlines-",
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
        equipment_slots: &[MainHand, OffHand, Feet, Legs, Chest, Head],
        dimensions: super::java_1_16_1::generated::ENTITY_DIMENSIONS,
    },
    generic_slot_class: "bhw",
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
        equipment_slots: &[MainHand, OffHand, Feet, Legs, Chest, Head, Body, Saddle],
        dimensions: super::java_1_21_11::generated::ENTITY_DIMENSIONS,
    },
    generic_slot_class: "dji",
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
        }
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
    }
}
