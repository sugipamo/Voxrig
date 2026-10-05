//! Effective item properties from original prototypes and exact received data.
use super::{ItemData, ItemStack, nbt::NbtValue, registry::Registry};
use crate::{Error, ErrorKind, MinecraftVersion, Result};
use std::{collections::BTreeMap, sync::OnceLock};

/// Read-only native properties after applying the item's data to its default values.
/// Native signed values are retained even for untrusted stream inputs; this value
/// does not prove stack equality, slot acceptance, inventory capacity or action permission.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct ItemProperties {
    /// Effective native maximum stack size, rather than the static default capacity.
    pub max_stack_size: i32,
    /// Native maximum durability; absence is distinct from present zero in modern data.
    pub max_damage: i32,
    /// Native damage getter after the owning adapter's receive/clamp rules.
    pub damage: i32,
    /// Native damageability, including unbreakable and component-presence rules.
    pub damageable: bool,
    /// Native damaged-item predicate, independent of raw nonzero damage metadata.
    pub damaged: bool,
    /// Native stackability, considering capacity, damageability and current damage.
    pub stackable: bool,
}

#[derive(serde::Deserialize)]
struct Defaults {
    defaults: Vec<DefaultItem>,
    #[serde(default)]
    prototype_values: Vec<PrototypeField>,
}
#[derive(serde::Deserialize)]
struct PrototypeField {
    name: String,
    native_id: i32,
    value_hex: String,
}
#[derive(serde::Deserialize)]
struct DefaultItem {
    native_id: i32,
    name: String,
    represents_empty: bool,
    properties: ItemProperties,
    #[serde(default)]
    normalizes_damage_on_read: bool,
    #[serde(default)]
    prototype_values: Vec<usize>,
}
fn defaults(version: MinecraftVersion) -> &'static Defaults {
    static LEGACY: OnceLock<Defaults> = OnceLock::new();
    static MODERN: OnceLock<Defaults> = OnceLock::new();
    match version {
        MinecraftVersion::Java1_16_1 => LEGACY.get_or_init(|| {
            serde_json::from_str(include_str!(
                "../../data/client_api/item_properties-1.16.1.json"
            ))
            .expect("pinned native legacy default item properties")
        }),
        MinecraftVersion::Java1_21_11 => MODERN.get_or_init(|| {
            serde_json::from_str(include_str!(
                "../../data/client_api/item_properties-1.21.11.json"
            ))
            .expect("pinned native modern default item properties")
        }),
    }
}
pub(crate) fn legacy_constructor_tag(item: &ItemStack) -> Result<Option<std::sync::Arc<NbtValue>>> {
    if item.id.version() != MinecraftVersion::Java1_16_1 {
        return Err(invalid("legacy item belongs to another version"));
    }
    let properties = item.properties()?;
    let default = defaults(MinecraftVersion::Java1_16_1)
        .defaults
        .iter()
        .find(|d| d.native_id == item.id.value() && d.name == item.name && !d.represents_empty)
        .ok_or_else(|| invalid("legacy constructor facts unavailable; update Voxrig"))?;
    let root = item.custom_data()?.map(|v| v.item_root());
    Ok(if default.normalizes_damage_on_read {
        Some(super::nbt::item_damage(root, properties.damage))
    } else {
        root
    })
}

fn invalid(message: &str) -> Error {
    Error::new(ErrorKind::InvalidInput, anyhow::anyhow!("{message}"))
}
struct ModernPrototypes {
    values: Vec<super::ItemComponent>,
    items: BTreeMap<i32, Vec<usize>>,
}
pub(crate) fn modern_prototype_components(
    native_id: i32,
) -> anyhow::Result<impl Iterator<Item = &'static super::ItemComponent>> {
    use anyhow::Context;
    static PROTOTYPES: OnceLock<std::result::Result<ModernPrototypes, String>> = OnceLock::new();
    let prototypes = PROTOTYPES.get_or_init(|| {
        let load = || -> anyhow::Result<ModernPrototypes> {
            let source = defaults(MinecraftVersion::Java1_21_11);
            let registry = Registry::for_version(MinecraftVersion::Java1_21_11);
            let values = source
                .prototype_values
                .iter()
                .map(|field| {
                    let definition = registry.item_component_by_native_id(field.native_id)?;
                    anyhow::ensure!(
                        definition.name == field.name,
                        "prototype component name mismatch"
                    );
                    Ok(super::ItemComponent {
                        definition,
                        bytes: prototype_bytes(&field.value_hex)?,
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            let mut items = BTreeMap::new();
            for item in &source.defaults {
                if item.represents_empty {
                    continue;
                }
                let mut ids = std::collections::BTreeSet::new();
                for &index in &item.prototype_values {
                    let value = values.get(index).context("invalid prototype field index")?;
                    anyhow::ensure!(
                        ids.insert(value.definition.id.value()),
                        "duplicate prototype field"
                    );
                }
                anyhow::ensure!(
                    items
                        .insert(item.native_id, item.prototype_values.clone())
                        .is_none(),
                    "duplicate prototype item"
                );
            }
            Ok(ModernPrototypes { values, items })
        };
        load().map_err(|error| format!("{error:#}"))
    });
    let prototypes = prototypes
        .as_ref()
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let indices = prototypes
        .items
        .get(&native_id)
        .context("missing native item prototype; update Voxrig")?;
    Ok(indices.iter().map(|&index| &prototypes.values[index]))
}
fn prototype_bytes(value: &str) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(value.len() % 2 == 0, "invalid prototype byte length");
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair)?;
            Ok(u8::from_str_radix(pair, 16)?)
        })
        .collect()
}
pub(crate) fn modern_weight_defaults(
    native_id: i32,
) -> anyhow::Result<super::item_constructor::WeightFields> {
    use anyhow::{Context, bail};
    let source = defaults(MinecraftVersion::Java1_21_11);
    let item = source
        .defaults
        .iter()
        .find(|item| item.native_id == native_id && !item.represents_empty)
        .context("missing native item weight prototype; update Voxrig")?;
    let mut fields = super::item_constructor::WeightFields {
        max_stack_size: item.properties.max_stack_size,
        bundle: None,
        has_bees: false,
    };
    for &index in &item.prototype_values {
        let field = source
            .prototype_values
            .get(index)
            .context("invalid native prototype index")?;
        if matches!(
            field.name.as_str(),
            "minecraft:bundle_contents" | "minecraft:bees"
        ) {
            // Every original prototype of these two types is an empty list.
            // A future nonempty prototype requires decoding its full fields.
            if field.value_hex != "00" {
                bail!("native nonempty weight prototype requires Voxrig update");
            }
            if field.name == "minecraft:bundle_contents" {
                fields.bundle = Some(super::fraction::Fraction::ZERO);
            }
        }
    }
    Ok(fields)
}
impl ItemStack {
    /// Read effective native stack/durability properties through the same API on both versions.
    /// Applies native default prototypes, added values, explicit removal and legacy receive
    /// normalization without changing ItemData bytes or claiming a new received observation.
    /// Untrusted native signed values remain visible, including invalid capacities;
    /// they are not an inventory policy, semantic item identity or action permission.
    ///
    /// ```no_run
    /// use voxrig::client::prelude::*;
    /// fn inspect(item: &ItemStack) -> Result<()> {
    ///     let properties = item.properties()?;
    ///     println!("capacity = {}, damage = {}", properties.max_stack_size, properties.damage);
    ///     Ok(())
    /// }
    /// ```
    pub fn properties(&self) -> Result<ItemProperties> {
        let version = self.id.version();
        let registry = Registry::for_version(version);
        if self.count == 0 || registry.item_definition(self.id)?.name != self.name {
            return Err(invalid("invalid item count or identity"));
        }
        let default = defaults(version)
            .defaults
            .iter()
            .find(|d| d.native_id == self.id.value() && d.name == self.name && !d.represents_empty)
            .ok_or_else(|| invalid("native item properties unavailable; update Voxrig"))?;
        match (version, &self.data) {
            (MinecraftVersion::Java1_16_1, ItemData::Default | ItemData::LegacyNbt { .. }) => {
                let tag = self.custom_data()?;
                let mut damage = tag
                    .as_ref()
                    .and_then(|tag| tag.root().get("Damage"))
                    .map_or(0, legacy_int);
                if default.normalizes_damage_on_read {
                    damage = damage.max(0);
                }
                let unbreakable = tag
                    .as_ref()
                    .and_then(|tag| tag.root().get("Unbreakable"))
                    .is_some_and(|value| legacy_int(value) as i8 != 0);
                let max_stack_size = default.properties.max_stack_size;
                let max_damage = default.properties.max_damage;
                let damageable = max_damage > 0 && !unbreakable;
                let damaged = damageable && damage > 0;
                Ok(ItemProperties {
                    max_stack_size,
                    max_damage,
                    damage,
                    damageable,
                    damaged,
                    stackable: max_stack_size > 1 && (!damageable || !damaged),
                })
            }
            (
                MinecraftVersion::Java1_21_11,
                ItemData::Default | ItemData::ModernComponents { .. },
            ) => {
                let patch = if let ItemData::ModernComponents { patch } = &self.data {
                    crate::versions::java_1_21_11::item_components::validate_patch(patch)
                        .map_err(|e| Error::new(ErrorKind::InvalidInput, e))?;
                    Some(patch)
                } else {
                    None
                };
                let prototype = super::modern_prototype_components(self.id.value())
                    .map_err(|e| Error::new(ErrorKind::InvalidInput, e))?;
                let fields =
                    super::item_components::ComponentFields::apply(version, prototype, patch)
                        .map_err(|e| Error::new(ErrorKind::InvalidInput, e))?;
                let number = |name: &str, fallback: i32| -> Result<i32> {
                    fields.get(name).map_or(Ok(fallback), |value| {
                        crate::versions::java_1_21_11::item_components::scalar_value(value)
                            .map_err(|e| Error::new(ErrorKind::InvalidInput, e))
                    })
                };
                let max_stack_size = number("minecraft:max_stack_size", 1)?;
                let max_damage = number("minecraft:max_damage", 0)?;
                let damage = number("minecraft:damage", 0)?;
                // Native clamp uses ordered branches even when max_damage is negative.
                let damage = if damage < 0 {
                    0
                } else if damage > max_damage {
                    max_damage
                } else {
                    damage
                };
                let damageable = fields.get("minecraft:max_damage").is_some()
                    && fields.get("minecraft:damage").is_some()
                    && fields.get("minecraft:unbreakable").is_none();
                let damaged = damageable && damage > 0;
                Ok(ItemProperties {
                    max_stack_size,
                    max_damage,
                    damage,
                    damageable,
                    damaged,
                    stackable: max_stack_size > 1 && (!damageable || !damaged),
                })
            }
            _ => Err(invalid(
                "item data representation belongs to another adapter",
            )),
        }
    }
}

// Original legacy numeric NBT getters use signed-width narrowing, and their float
// conversion uses Java's saturating cast followed by Mth.floor's wrapping decrement.
// This coercion is deliberately separate from NbtValue's strict typed accessors.
fn legacy_int(value: &NbtValue) -> i32 {
    match value {
        NbtValue::Byte(v) => i32::from(*v),
        NbtValue::Short(v) => i32::from(*v),
        NbtValue::Int(v) => *v,
        NbtValue::Long(v) => *v as i32,
        NbtValue::Float { bits } => {
            let value = f32::from_bits(*bits);
            let integer = value as i32;
            if value < integer as f32 {
                integer.wrapping_sub(1)
            } else {
                integer
            }
        }
        NbtValue::Double { bits } => {
            let value = f64::from_bits(*bits);
            let integer = value as i32;
            if value < f64::from(integer) {
                integer.wrapping_sub(1)
            } else {
                integer
            }
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::SlotKnowledge;
    use serde_json::Value;
    fn corpus(version: MinecraftVersion) -> Value {
        serde_json::from_str(match version {
            MinecraftVersion::Java1_16_1 => {
                include_str!("../../data/client_api/item_properties-1.16.1.json")
            }
            MinecraftVersion::Java1_21_11 => {
                include_str!("../../data/client_api/item_properties-1.21.11.json")
            }
        })
        .unwrap()
    }
    fn decoded(bytes: &[u8], version: MinecraftVersion) -> ItemStack {
        match version {
            MinecraftVersion::Java1_16_1 => {
                let mut input = bytes;
                let native =
                    crate::versions::java_1_16_1::inventory::read_slot(&mut input).unwrap();
                assert!(input.is_empty());
                let SlotKnowledge::Item { item } =
                    super::super::observation::legacy_slot(native.as_ref()).unwrap()
                else {
                    panic!("expected item");
                };
                item
            }
            MinecraftVersion::Java1_21_11 => {
                let mut bytes = bytes;
                let count = crate::protocol::get_varint(&mut bytes).unwrap() as u32;
                let definition = Registry::for_version(version)
                    .item_by_native_id(crate::protocol::get_varint(&mut bytes).unwrap())
                    .unwrap();
                let patch =
                    crate::versions::java_1_21_11::item_components::decode_patch(bytes).unwrap();
                ItemStack {
                    id: definition.id,
                    name: definition.name,
                    count,
                    data: ItemData::ModernComponents { patch },
                }
            }
        }
    }
    #[test]
    fn effective_properties_match_all_original_default_and_received_item_cases() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let data = corpus(version);
            assert!(data["failures"].as_array().unwrap().is_empty());
            assert_eq!(
                data["defaults"].as_array().unwrap().len(),
                if version == MinecraftVersion::Java1_16_1 {
                    975
                } else {
                    1505
                }
            );
            assert_eq!(
                data["cases"].as_array().unwrap().len(),
                if version == MinecraftVersion::Java1_16_1 {
                    1428
                } else {
                    1288
                }
            );
            for row in data["defaults"].as_array().unwrap() {
                if row["represents_empty"].as_bool().unwrap() {
                    assert_eq!(row["name"], "minecraft:air");
                    assert_eq!(row["native_id"], 0);
                    assert_eq!(row["encoded_item_hex"], "00");
                    continue;
                }
                let registry = Registry::for_version(version);
                let definition = registry.item(row["name"].as_str().unwrap()).unwrap();
                assert_eq!(
                    definition.id.value(),
                    row["native_id"].as_i64().unwrap() as i32
                );
                let default = ItemStack {
                    id: definition.id,
                    name: definition.name,
                    count: 1,
                    data: ItemData::Default,
                };
                let expected: ItemProperties =
                    serde_json::from_value(row["properties"].clone()).unwrap();
                assert_eq!(
                    default.properties().unwrap(),
                    expected,
                    "{version:?} default {}",
                    default.name
                );
                let received = decoded(
                    &hex::decode(row["encoded_item_hex"].as_str().unwrap()).unwrap(),
                    version,
                );
                assert_eq!(
                    received.properties().unwrap(),
                    expected,
                    "{version:?} received default {}",
                    default.name
                );
            }
            for row in data["cases"].as_array().unwrap() {
                let original = hex::decode(row["input_item_hex"].as_str().unwrap()).unwrap();
                let received = decoded(&original, version);
                let before = received.data.clone();
                let expected: ItemProperties =
                    serde_json::from_value(row["properties"].clone()).unwrap();
                assert_eq!(
                    received.properties().unwrap(),
                    expected,
                    "{version:?} {}",
                    row["case"]
                );
                assert_eq!(
                    received.data, before,
                    "getter must preserve original native data"
                );
                let canonical = decoded(
                    &hex::decode(row["canonical_item_hex"].as_str().unwrap()).unwrap(),
                    version,
                );
                assert_eq!(
                    canonical.properties().unwrap(),
                    expected,
                    "{version:?} canonical {}",
                    row["case"]
                );
            }
        }
    }
    #[test]
    fn prototypes_and_property_oracles_are_source_bound_without_live_registry_injection() {
        use sha2::{Digest, Sha256};
        let source: Value = serde_json::from_str(include_str!(
            "../../data/client_api/item_properties_source.json"
        ))
        .unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for (path, hash) in source["generators_sha256"].as_object().unwrap() {
            assert_eq!(
                format!(
                    "{:x}",
                    Sha256::digest(std::fs::read(root.join(path)).unwrap())
                ),
                hash.as_str().unwrap(),
                "{path}"
            );
        }
        for run in source["runs"].as_array().unwrap() {
            for (path, hash) in run["files_sha256"].as_object().unwrap() {
                assert_eq!(
                    format!(
                        "{:x}",
                        Sha256::digest(std::fs::read(root.join(path)).unwrap())
                    ),
                    hash.as_str().unwrap(),
                    "{path}"
                );
            }
        }
        let data = corpus(MinecraftVersion::Java1_21_11);
        let values = data["prototype_values"].as_array().unwrap();
        assert_eq!(values.len(), 3490);
        for value in values {
            let definition = Registry::for_version(MinecraftVersion::Java1_21_11)
                .item_component_by_native_id(value["native_id"].as_i64().unwrap() as i32)
                .unwrap();
            assert_eq!(definition.name, value["name"].as_str().unwrap());
        }
        for row in data["defaults"].as_array().unwrap() {
            let mut seen = std::collections::BTreeSet::new();
            for i in row["prototype_values"].as_array().unwrap() {
                let value = &values[i.as_u64().unwrap() as usize];
                assert!(seen.insert(value["native_id"].as_i64().unwrap()));
            }
        }
    }
    #[test]
    fn effective_properties_refuse_forged_duplicate_malformed_and_wrong_adapter_data() {
        use crate::client::{ItemComponent, ItemComponentPatch};
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let registry = Registry::for_version(version);
            let definition = registry.item("minecraft:stone").unwrap();
            let mut item = ItemStack {
                id: definition.id,
                name: definition.name,
                count: 1,
                data: ItemData::Default,
            };
            item.count = 0;
            assert!(item.properties().is_err());
            item.count = 1;
            item.name = "minecraft:dirt".into();
            assert!(item.properties().is_err());
            item.name = "minecraft:stone".into();
            if version == MinecraftVersion::Java1_16_1 {
                item.data = ItemData::ModernComponents {
                    patch: ItemComponentPatch {
                        added: vec![],
                        removed: vec![],
                    },
                };
                assert!(item.properties().is_err());
                item.data = ItemData::LegacyNbt {
                    bytes: vec![10, 0, 0, 0, 1],
                };
                assert!(item.properties().is_err());
                continue;
            }
            item.data = ItemData::LegacyNbt {
                bytes: vec![10, 0, 0, 0],
            };
            assert!(item.properties().is_err());
            let field = ItemComponent {
                definition: registry.item_component("minecraft:max_stack_size").unwrap(),
                bytes: vec![16],
            };
            let mut patch = ItemComponentPatch {
                added: vec![field.clone()],
                removed: vec![],
            };
            item.data = ItemData::ModernComponents {
                patch: patch.clone(),
            };
            assert_eq!(item.properties().unwrap().max_stack_size, 16);
            patch.added.push(field.clone());
            item.data = ItemData::ModernComponents {
                patch: patch.clone(),
            };
            assert!(item.properties().is_err());
            patch.added.pop();
            patch.removed.push(field.definition.clone());
            item.data = ItemData::ModernComponents {
                patch: patch.clone(),
            };
            assert!(item.properties().is_err());
            patch.removed.clear();
            patch.added[0].definition.name = "minecraft:damage".into();
            item.data = ItemData::ModernComponents {
                patch: patch.clone(),
            };
            assert!(item.properties().is_err());
            patch.added[0] = field;
            for bytes in [vec![], vec![128], vec![16, 1], vec![255; 6]] {
                patch.added[0].bytes = bytes;
                item.data = ItemData::ModernComponents {
                    patch: patch.clone(),
                };
                assert!(item.properties().is_err());
            }
            // A known but irrelevant component still requires valid complete field boundaries.
            patch.added[0] = ItemComponent {
                definition: registry.item_component("minecraft:custom_name").unwrap(),
                bytes: vec![],
            };
            item.data = ItemData::ModernComponents { patch };
            assert!(item.properties().is_err());
        }
    }
}
