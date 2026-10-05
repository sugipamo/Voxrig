//! Original QUICK_MOVE routes; the model never modifies received slots.
use super::{SlotKnowledge, unavailable};
use crate::client::{ItemData, ItemStack, registry::Registry};
use crate::{MinecraftVersion, Result};
use std::sync::OnceLock;

#[derive(serde::Deserialize)]
struct Profiles {
    routes: Vec<Route>,
    equipment_slots: Vec<EquipmentSlot>,
}
#[derive(serde::Deserialize)]
struct Route {
    name: String,
    native_id: i32,
    preferred_equipment_slot: Option<usize>,
    default_legacy_nbt: Option<Vec<u8>>,
}
#[derive(serde::Deserialize)]
struct EquipmentSlot {
    slot: usize,
    base_capacity: u32,
    may_pickup: bool,
    accepted_default_items: Vec<String>,
}
fn profiles(version: MinecraftVersion) -> &'static Profiles {
    static LEGACY: OnceLock<Profiles> = OnceLock::new();
    static MODERN: OnceLock<Profiles> = OnceLock::new();
    match version {
        MinecraftVersion::Java1_16_1 => LEGACY.get_or_init(|| {
            serde_json::from_str(include_str!(
                "../../../data/client_api/inventory_transfer_profiles-1.16.1.json"
            ))
            .expect("pinned native transfer profiles")
        }),
        MinecraftVersion::Java1_21_11 => MODERN.get_or_init(|| {
            serde_json::from_str(include_str!(
                "../../../data/client_api/inventory_transfer_profiles-1.21.11.json"
            ))
            .expect("pinned native transfer profiles")
        }),
    }
}
fn validate(
    version: MinecraftVersion,
    value: &SlotKnowledge,
    context: Option<&super::data::ItemContext>,
) -> Result<()> {
    match value {
        SlotKnowledge::Empty => Ok(()),
        SlotKnowledge::Unavailable => {
            Err(unavailable("complete received transfer slot unavailable"))
        }
        SlotKnowledge::Item { item } => {
            let definition = Registry::for_version(version).item(&item.name)?;
            if definition.id != item.id
                || item.count == 0
                || item.count > maximum(version, item, context)?
            {
                return Err(crate::client::registry::invalid(
                    "transfer requires valid version-bound default counts",
                ));
            }
            if context.is_none() && !default_data(version, item) {
                return Err(crate::Error::new(
                    crate::ErrorKind::Unsupported,
                    anyhow::anyhow!("transfer model requires default item data"),
                ));
            }
            if let Some(context) = context {
                if !context.default_transfer_equipment(item)? {
                    return Err(crate::Error::new(
                        crate::ErrorKind::Unsupported,
                        anyhow::anyhow!(
                            "modified equippable QUICK_MOVE routing remains unimplemented"
                        ),
                    ));
                }
            }
            Ok(())
        }
    }
}
fn maximum(
    version: MinecraftVersion,
    item: &ItemStack,
    context: Option<&super::data::ItemContext>,
) -> Result<u32> {
    if context.is_some() {
        u32::try_from(item.properties()?.max_stack_size)
            .map_err(|e| crate::Error::new(crate::ErrorKind::InvalidInput, e))
    } else {
        super::slot_policy::default_item_capacity(version, item.id.value(), &item.name)
            .ok_or_else(|| unavailable("native transfer item capacity unavailable"))
    }
}
fn default_data(version: MinecraftVersion, item: &ItemStack) -> bool {
    if item.data == ItemData::Default {
        return true;
    }
    match (version, &item.data) {
        (MinecraftVersion::Java1_16_1, ItemData::LegacyNbt { bytes }) => profiles(version)
            .routes
            .iter()
            .find(|r| r.native_id == item.id.value() && r.name == item.name)
            .and_then(|r| r.default_legacy_nbt.as_ref())
            .is_some_and(|actual| actual == bytes),
        _ => false,
    }
}
/// Exact comparison data accepted by the private legacy transfer actor.
/// Tags are preserved, never erased or guessed from a general item category.
pub(crate) fn legacy_comparison_supported(stack: &crate::versions::java_1_16_1::ItemStack) -> bool {
    let Some(route) = profiles(MinecraftVersion::Java1_16_1)
        .routes
        .iter()
        .find(|r| r.native_id == stack.item_id)
    else {
        return false;
    };
    stack.count > 0
        && super::slot_policy::default_item_capacity(
            MinecraftVersion::Java1_16_1,
            stack.item_id,
            &route.name,
        )
        .is_some_and(|max| stack.count as u32 <= max)
        && stack
            .nbt
            .as_ref()
            .is_none_or(|nbt| crate::client::nbt::decode(nbt, MinecraftVersion::Java1_16_1).is_ok())
}
fn capacity(
    version: MinecraftVersion,
    menu: &str,
    index: usize,
    item: &ItemStack,
    context: Option<&super::data::ItemContext>,
) -> Result<u32> {
    let max = maximum(version, item, context)?;
    let base = if menu == "minecraft:player" && matches!(index, 5..=8 | 45) {
        profiles(version)
            .equipment_slots
            .iter()
            .find(|s| s.slot == index)
            .ok_or_else(|| unavailable("native equipment capacity unavailable"))?
            .base_capacity
    } else {
        super::slot_policy::regular_slot(version, menu, index)?.base_capacity
    };
    Ok(base.min(max))
}
fn may_place(
    version: MinecraftVersion,
    menu: &str,
    index: usize,
    item: &ItemStack,
) -> Result<bool> {
    if menu == "minecraft:player" && matches!(index, 5..=8 | 45) {
        Ok(profiles(version)
            .equipment_slots
            .iter()
            .find(|s| s.slot == index)
            .ok_or_else(|| unavailable("native equipment acceptance unavailable"))?
            .accepted_default_items
            .contains(&item.name))
    } else {
        Ok(!super::slot_policy::regular_slot(version, menu, index)?
            .rejected_default_items
            .contains(&item.name))
    }
}
fn same(a: &ItemStack, b: &ItemStack) -> bool {
    a.id == b.id && a.name == b.name && a.data == b.data
}
fn counted(item: &ItemStack, count: u32) -> SlotKnowledge {
    if count == 0 {
        SlotKnowledge::Empty
    } else {
        let mut item = item.clone();
        item.count = count;
        SlotKnowledge::Item { item }
    }
}
/// One complete native clicked QUICK_MOVE, including its internal repeated moves.
/// The legacy return describes native comparisons, never a received result.
pub(super) struct Prediction {
    pub(super) slots: Vec<SlotKnowledge>,
    pub(super) legacy_return: SlotKnowledge,
}
fn storage_size(menu: &str) -> Option<usize> {
    match menu {
        "minecraft:generic_9x1" => Some(9),
        "minecraft:generic_9x2" => Some(18),
        "minecraft:generic_9x3" => Some(27),
        "minecraft:generic_9x4" => Some(36),
        "minecraft:generic_9x5" => Some(45),
        "minecraft:generic_9x6" => Some(54),
        "minecraft:generic_3x3" => Some(9),
        "minecraft:hopper" => Some(5),
        "minecraft:shulker_box" => Some(27),
        _ => None,
    }
}
pub(super) fn calculate(
    version: MinecraftVersion,
    menu: &str,
    source: usize,
    before: &[SlotKnowledge],
) -> Result<Prediction> {
    calculate_inner(version, menu, source, before, None)
}
pub(super) fn calculate_with_data(
    version: MinecraftVersion,
    menu: &str,
    source: usize,
    before: &[SlotKnowledge],
    context: &super::data::ItemContext,
) -> Result<Prediction> {
    calculate_inner(version, menu, source, before, Some(context))
}
fn calculate_inner(
    version: MinecraftVersion,
    menu: &str,
    source: usize,
    before: &[SlotKnowledge],
    context: Option<&super::data::ItemContext>,
) -> Result<Prediction> {
    let storage = if menu == "minecraft:player" {
        if before.len() != 46 || !(5..=45).contains(&source) {
            return Err(crate::client::registry::invalid(
                "player transfer requires slots 5..45 and native 46-slot layout",
            ));
        }
        None
    } else {
        let n = storage_size(menu)
            .ok_or_else(|| unavailable("native transfer route unavailable; update Voxrig"))?;
        if before.len() != n + 36 || source >= before.len() {
            return Err(crate::client::registry::invalid(
                "transfer outside native storage layout",
            ));
        }
        Some(n)
    };
    validate(version, &before[source], context)?;
    let mut result = Prediction {
        slots: before.to_vec(),
        legacy_return: SlotKnowledge::Empty,
    };
    let SlotKnowledge::Item { item: original } = &before[source] else {
        return Ok(result);
    };
    let route = profiles(version)
        .routes
        .iter()
        .find(|r| r.native_id == original.id.value() && r.name == original.name)
        .ok_or_else(|| unavailable("native default transfer route unavailable; update Voxrig"))?;
    if context.is_some()
        && storage.is_none()
        && (5..=8).contains(&source)
        && !default_data(version, original)
    {
        return Err(crate::Error::new(
            crate::ErrorKind::Unsupported,
            anyhow::anyhow!("data-bearing equipped-item pickup rules remain unimplemented"),
        ));
    }
    let may_pickup = if storage.is_none() && matches!(source, 5..=8 | 45) {
        profiles(version)
            .equipment_slots
            .iter()
            .find(|s| s.slot == source)
            .ok_or_else(|| unavailable("equipment take policy unavailable"))?
            .may_pickup
    } else {
        super::slot_policy::regular_slot(version, menu, source)?.may_pickup
    };
    if !may_pickup {
        return Ok(result);
    }
    let mut remaining = original.count;
    // Every successful round reduces this valid original count; never an I/O retry.
    while remaining > 0 {
        let destinations: Vec<usize> = match storage {
            Some(n) if source < n => (n..n + 36).rev().collect(),
            Some(n) => (0..n).collect(),
            None if (5..=8).contains(&source) => (9..45).collect(),
            None => {
                let equipment = route.preferred_equipment_slot;
                if let Some(slot) = equipment {
                    validate(version, &result.slots[slot], context)?;
                    if result.slots[slot] == SlotKnowledge::Empty {
                        vec![slot]
                    } else if (9..=35).contains(&source) {
                        (36..45).collect()
                    } else if (36..=44).contains(&source) {
                        (9..36).collect()
                    } else {
                        (9..45).collect()
                    }
                } else if (9..=35).contains(&source) {
                    (36..45).collect()
                } else if (36..=44).contains(&source) {
                    (9..36).collect()
                } else {
                    (9..45).collect()
                }
            }
        };
        for &index in &destinations {
            validate(version, &result.slots[index], context)?;
        }
        let start = remaining;
        let stackable = if context.is_some() {
            original.properties()?.stackable
        } else {
            maximum(version, original, context)? > 1
        };
        if stackable {
            for &index in &destinations {
                if remaining == 0 {
                    break;
                }
                if let SlotKnowledge::Item { item } = &result.slots[index] {
                    let same_data = if let Some(context) = context {
                        context.same_data(original, item)?
                    } else {
                        same(original, item)
                    };
                    if same_data {
                        let room = capacity(version, menu, index, original, context)?
                            .saturating_sub(item.count);
                        let amount = remaining.min(room);
                        result.slots[index] = counted(item, item.count + amount);
                        remaining -= amount;
                    }
                }
            }
        }
        if remaining > 0 {
            for &index in &destinations {
                if result.slots[index] == SlotKnowledge::Empty
                    && may_place(version, menu, index, original)?
                {
                    let amount = remaining.min(capacity(version, menu, index, original, context)?);
                    result.slots[index] = counted(original, amount);
                    remaining -= amount;
                    break; // Original moveItemStackTo stops at its first eligible empty slot.
                }
            }
        }
        result.slots[source] = counted(original, remaining);
        if remaining == start {
            break;
        }
        // Original legacy clicked stores a returned predecessor only when
        // the source still holds that item after a successful native round.
        if remaining > 0 {
            result.legacy_return = counted(original, start);
        }
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    fn values(version: MinecraftVersion, array: &serde_json::Value) -> Vec<SlotKnowledge> {
        array
            .as_array()
            .unwrap()
            .iter()
            .map(|v| match v["item"].as_str() {
                None => {
                    assert_eq!(v["count"], 0);
                    SlotKnowledge::Empty
                }
                Some(name) => {
                    let definition = Registry::for_version(version).item(name).unwrap();
                    SlotKnowledge::Item {
                        item: ItemStack {
                            id: definition.id,
                            name: definition.name,
                            count: v["count"].as_u64().unwrap() as u32,
                            data: v
                                .get("nbt")
                                .filter(|v| **v != serde_json::json!([0]))
                                .map_or(ItemData::Default, |v| ItemData::LegacyNbt {
                                    bytes: v
                                        .as_array()
                                        .unwrap()
                                        .iter()
                                        .map(|v| v.as_u64().unwrap() as u8)
                                        .collect(),
                                }),
                        },
                    }
                }
            })
            .collect()
    }
    #[test]
    fn complete_shift_prediction_and_legacy_return_match_every_original_native_case() {
        for (version, bytes, expected) in [
            (
                MinecraftVersion::Java1_16_1,
                include_bytes!("../../../data/client_api/inventory_transfer_cases-1.16.1.json.gz")
                    .as_slice(),
                4988,
            ),
            (
                MinecraftVersion::Java1_21_11,
                include_bytes!("../../../data/client_api/inventory_transfer_cases-1.21.11.json.gz")
                    .as_slice(),
                6238,
            ),
        ] {
            let mut text = String::new();
            flate2::read::GzDecoder::new(bytes)
                .read_to_string(&mut text)
                .unwrap();
            let data: serde_json::Value = serde_json::from_str(&text).unwrap();
            let cases = data["cases"].as_array().unwrap();
            assert_eq!(cases.len(), expected);
            for case in cases {
                let before = values(version, &case["before"]);
                let after = values(version, &case["after"]);
                let prediction = calculate(
                    version,
                    case["menu"].as_str().unwrap(),
                    case["slot"].as_u64().unwrap() as usize,
                    &before,
                )
                .unwrap();
                assert_eq!(prediction.slots, after, "{version:?} {case}");
                assert_eq!(case["cursor_before"], case["cursor_after"]);
                if version == MinecraftVersion::Java1_16_1 {
                    assert_eq!(
                        prediction.legacy_return,
                        values(version, &serde_json::json!([case["legacy_returned"]]))[0],
                        "{case}"
                    );
                }
            }
        }
    }
    #[test]
    fn frozen_transfer_evidence_is_bound_to_original_exporters_and_outputs() {
        use sha2::{Digest, Sha256};
        let data: serde_json::Value = serde_json::from_str(include_str!(
            "../../../data/client_api/inventory_transfer_source.json"
        ))
        .unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for object in std::iter::once(&data["generators_sha256"]).chain(
            data["runs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| &r["files_sha256"]),
        ) {
            for (path, hash) in object.as_object().unwrap() {
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
    }
}
