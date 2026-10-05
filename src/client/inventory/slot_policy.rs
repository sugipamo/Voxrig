//! Slot acceptance obtained from original native constructors and item methods.
use super::{SlotKnowledge, unavailable};
use crate::{MinecraftVersion, Result};
use std::sync::OnceLock;

#[derive(serde::Deserialize)]
struct Profiles {
    items: Vec<ItemCapacity>,
    slot_policies: Vec<SlotPolicy>,
    menus: Vec<Menu>,
}
#[derive(serde::Deserialize)]
struct ItemCapacity {
    native_id: i32,
    name: String,
    maximum_stack_size: u32,
    represents_empty: bool,
    ordinary_pickup: bool,
}
#[derive(serde::Deserialize)]
struct Menu {
    name: String,
    slots: Vec<Slot>,
}
#[derive(serde::Deserialize)]
struct Slot {
    slot: usize,
    policy: usize,
}
#[derive(serde::Deserialize)]
pub(super) struct SlotPolicy {
    pub(super) may_pickup: bool,
    pub(super) base_capacity: u32,
    pub(super) rejected_default_items: Vec<String>,
}
fn profiles(version: MinecraftVersion) -> &'static Profiles {
    static LEGACY: OnceLock<Profiles> = OnceLock::new();
    static MODERN: OnceLock<Profiles> = OnceLock::new();
    match version {
        MinecraftVersion::Java1_16_1 => LEGACY.get_or_init(|| {
            serde_json::from_str(include_str!(
                "../../../data/client_api/regular_click_profiles-1.16.1.json"
            ))
            .expect("pinned native click profiles")
        }),
        MinecraftVersion::Java1_21_11 => MODERN.get_or_init(|| {
            serde_json::from_str(include_str!(
                "../../../data/client_api/regular_click_profiles-1.21.11.json"
            ))
            .expect("pinned native click profiles")
        }),
    }
}
pub(super) fn regular_slot(
    version: MinecraftVersion,
    menu: &str,
    index: usize,
) -> Result<&'static SlotPolicy> {
    let profiles = profiles(version);
    profiles
        .menus
        .iter()
        .find(|m| m.name == menu)
        .and_then(|m| m.slots.iter().find(|s| s.slot == index))
        .and_then(|s| profiles.slot_policies.get(s.policy))
        .ok_or_else(|| unavailable("native ordinary slot policy unavailable; update Voxrig"))
}
/// Original native default-stack capacity, tied to both registry ID and name.
/// AIR's empty sentinel is not an inventory stack/capacity definition.
pub(crate) fn default_item_capacity(version: MinecraftVersion, id: i32, name: &str) -> Option<u32> {
    profiles(version)
        .items
        .iter()
        .find(|i| i.native_id == id && i.name == name && !i.represents_empty)
        .map(|i| i.maximum_stack_size)
}

/// Ordinary default-item PICKUP only; item overrides require separate adapters.
pub(crate) fn pickup(
    version: MinecraftVersion,
    menu_name: &str,
    source_slot: usize,
    button: super::InventoryClickButton,
    source: &SlotKnowledge,
    cursor: &SlotKnowledge,
) -> Result<(SlotKnowledge, SlotKnowledge)> {
    pickup_inner(
        version,
        menu_name,
        source_slot,
        button,
        (source, cursor),
        None,
    )
}
pub(crate) fn pickup_with_data(
    version: MinecraftVersion,
    menu_name: &str,
    source_slot: usize,
    button: super::InventoryClickButton,
    values: (&SlotKnowledge, &SlotKnowledge),
    context: &super::data::ItemContext,
) -> Result<(SlotKnowledge, SlotKnowledge)> {
    pickup_inner(
        version,
        menu_name,
        source_slot,
        button,
        values,
        Some(context),
    )
}
fn pickup_inner(
    version: MinecraftVersion,
    menu_name: &str,
    source_slot: usize,
    button: super::InventoryClickButton,
    values: (&SlotKnowledge, &SlotKnowledge),
    context: Option<&super::data::ItemContext>,
) -> Result<(SlotKnowledge, SlotKnowledge)> {
    let (source, cursor) = values;
    use crate::client::{ItemData, registry::Registry};
    let profiles = profiles(version);
    let slot = profiles
        .menus
        .iter()
        .find(|m| m.name == menu_name)
        .and_then(|m| m.slots.iter().find(|s| s.slot == source_slot))
        .and_then(|s| profiles.slot_policies.get(s.policy))
        .ok_or_else(|| unavailable("native PICKUP slot policy unavailable; update Voxrig"))?;
    let ordinary = |value: &SlotKnowledge| match value {
        SlotKnowledge::Empty => true,
        SlotKnowledge::Item { item } => profiles
            .items
            .iter()
            .find(|p| p.native_id == item.id.value() && p.name == item.name)
            .is_some_and(|p| p.ordinary_pickup),
        _ => false,
    };
    let left_boundary = button == super::InventoryClickButton::Left
        && (matches!(source, SlotKnowledge::Empty)
            || matches!(cursor, SlotKnowledge::Empty)
            || (ordinary(source)
                && ordinary(cursor)
                && matches!((source, cursor), (SlotKnowledge::Item { item: a }, SlotKnowledge::Item { item: b }) if a.id == b.id && a.data == b.data && a.name == b.name)));
    let validate = |value: &SlotKnowledge| -> Result<Option<u32>> {
        match value {
            SlotKnowledge::Empty => Ok(None),
            SlotKnowledge::Unavailable => Err(unavailable("PICKUP predecessor unavailable")),
            SlotKnowledge::Item { item } => {
                let definition = Registry::for_version(version).item(&item.name)?;
                let profile = profiles
                    .items
                    .iter()
                    .find(|p| p.native_id == item.id.value() && p.name == item.name)
                    .filter(|p| !p.represents_empty)
                    .ok_or_else(|| {
                        crate::client::registry::invalid("native PICKUP item identity unavailable")
                    })?;
                let maximum_stack_size = if context.is_some() {
                    u32::try_from(item.properties()?.max_stack_size).map_err(|_| {
                        crate::client::registry::invalid("invalid effective PICKUP capacity")
                    })?
                } else {
                    profile.maximum_stack_size
                };
                if item.id != definition.id || item.count == 0 || item.count > maximum_stack_size {
                    return Err(crate::client::registry::invalid(
                        "PICKUP requires valid version-bound item counts",
                    ));
                }
                if (context.is_none() && item.data != ItemData::Default || !profile.ordinary_pickup)
                    && !(left_boundary && super::return_policy::default_left_item(version, item))
                {
                    return Err(crate::Error::new(
                        crate::ErrorKind::Unsupported,
                        anyhow::anyhow!(
                            "PICKUP item data/override requires additional adapter support"
                        ),
                    ));
                }
                Ok(Some(maximum_stack_size))
            }
        }
    };
    let source_max = validate(source)?;
    let cursor_max = validate(cursor)?;
    let counted = |value: &SlotKnowledge, count: u32| -> SlotKnowledge {
        if count == 0 {
            SlotKnowledge::Empty
        } else {
            let SlotKnowledge::Item { item } = value else {
                unreachable!()
            };
            let mut item = item.clone();
            item.count = count;
            SlotKnowledge::Item { item }
        }
    };
    let right = button == super::InventoryClickButton::Right;
    let (result_source, result_cursor) = match (source, cursor) {
        (SlotKnowledge::Empty, SlotKnowledge::Empty) => (source.clone(), cursor.clone()),
        (SlotKnowledge::Item { item }, SlotKnowledge::Empty) if slot.may_pickup => {
            let take = if right {
                item.count.div_ceil(2)
            } else {
                item.count
            };
            (counted(source, item.count - take), counted(source, take))
        }
        (_, SlotKnowledge::Item { item: incoming }) => {
            let may_place = !slot.rejected_default_items.contains(&incoming.name);
            let capacity = slot.base_capacity.min(cursor_max.expect("validated item"));
            match source {
                SlotKnowledge::Empty if may_place => {
                    let take = incoming.count.min(if right { 1 } else { capacity });
                    (
                        counted(cursor, take),
                        counted(cursor, incoming.count - take),
                    )
                }
                SlotKnowledge::Item { item: existing } => {
                    let same = if let Some(context) = context {
                        context.same_data(existing, incoming)?
                    } else {
                        existing.id == incoming.id
                            && existing.name == incoming.name
                            && existing.data == incoming.data
                    };
                    if same && may_place {
                        let room = capacity.saturating_sub(existing.count);
                        let take = incoming
                            .count
                            .min(room)
                            .min(if right { 1 } else { u32::MAX });
                        (
                            counted(source, existing.count + take),
                            counted(cursor, incoming.count - take),
                        )
                    } else if !same && may_place && slot.may_pickup && incoming.count <= capacity {
                        (cursor.clone(), source.clone())
                    } else if same
                        && !may_place
                        && slot.may_pickup
                        && existing.count
                            <= source_max
                                .expect("validated item")
                                .saturating_sub(incoming.count)
                    {
                        (
                            SlotKnowledge::Empty,
                            counted(cursor, incoming.count + existing.count),
                        )
                    } else {
                        (source.clone(), cursor.clone())
                    }
                }
                _ => (source.clone(), cursor.clone()),
            }
        }
        _ => (source.clone(), cursor.clone()),
    };
    Ok((result_source, result_cursor))
}

/// Whole SWAP only: a native partial insertion would require different receipts.
/// Caller separately validates received/default/version-bound stacks and layout.
pub(super) fn validate_swap(
    version: MinecraftVersion,
    menu_name: &str,
    source_slot: usize,
    source: &SlotKnowledge,
    hotbar: &SlotKnowledge,
) -> Result<()> {
    let profiles = profiles(version);
    let slot = profiles
        .menus
        .iter()
        .find(|m| m.name == menu_name)
        .and_then(|m| m.slots.iter().find(|s| s.slot == source_slot))
        .and_then(|s| profiles.slot_policies.get(s.policy))
        .ok_or_else(|| unavailable("native source slot policy unavailable; update Voxrig"))?;
    if matches!(source, SlotKnowledge::Item { .. }) && !slot.may_pickup {
        return Err(super::super::registry::invalid(
            "native slot disallows taking its contents",
        ));
    }
    if let SlotKnowledge::Item { item } = hotbar {
        if item.count > slot.base_capacity
            || slot.rejected_default_items.iter().any(|n| n == &item.name)
        {
            return Err(super::super::registry::invalid(
                "native slot disallows this whole incoming stack",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{ItemData, ItemStack, registry::Registry};
    use std::io::Read;
    #[test]
    fn frozen_native_evidence_is_bound_to_original_tools_and_outputs() {
        use sha2::{Digest, Sha256};
        let source: serde_json::Value = serde_json::from_str(include_str!(
            "../../../data/client_api/regular_click_source.json"
        ))
        .unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let check = |values: &serde_json::Value| {
            for (path, expected) in values.as_object().unwrap() {
                let bytes = std::fs::read(root.join(path)).unwrap();
                assert_eq!(
                    format!("{:x}", Sha256::digest(bytes)),
                    expected.as_str().unwrap(),
                    "{path}"
                );
            }
        };
        check(&source["generators_sha256"]);
        check(&source["upstream_inputs_sha256"]);
        for run in source["runs"].as_array().unwrap() {
            check(&run["files_sha256"]);
            assert_eq!(run["pickup_cases"], 18432);
            assert_eq!(
                run["valid_count_pickup_cases"],
                if run["version"] == "1.16.1" {
                    11450
                } else {
                    11950
                }
            );
            assert_eq!(run["valid_requested_count_pickup_cases"], 11250);
            assert_eq!(run["swap_cases"], 7500);
            assert_eq!(run["codec_roundtrips"], 120);
        }
    }

    fn value(version: MinecraftVersion, native: &serde_json::Value) -> SlotKnowledge {
        match native["item"].as_str() {
            None => {
                assert_eq!(native["count"], 0);
                SlotKnowledge::Empty
            }
            Some(name) => SlotKnowledge::Item {
                item: ItemStack {
                    id: Registry::for_version(version).item(name).unwrap().id,
                    name: name.into(),
                    count: native["count"].as_u64().unwrap().try_into().unwrap(),
                    data: ItemData::Default,
                },
            },
        }
    }
    fn cases(version: MinecraftVersion) -> serde_json::Value {
        let bytes: &[u8] = match version {
            MinecraftVersion::Java1_16_1 => {
                include_bytes!("../../../data/client_api/regular_click_cases-1.16.1.json.gz")
            }
            MinecraftVersion::Java1_21_11 => {
                include_bytes!("../../../data/client_api/regular_click_cases-1.21.11.json.gz")
            }
        };
        let mut decoded = String::new();
        flate2::read::GzDecoder::new(bytes)
            .read_to_string(&mut decoded)
            .unwrap();
        serde_json::from_str(&decoded).unwrap()
    }
    #[test]
    fn pickup_predictions_match_every_original_native_valid_default_case() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let data = cases(version);
            for case in data["cases"].as_array().unwrap() {
                let source = value(version, &case["source_before"]);
                let cursor = value(version, &case["cursor_before"]);
                let result = pickup(
                    version,
                    case["menu"].as_str().unwrap(),
                    case["slot"].as_u64().unwrap() as usize,
                    if case["button"] == 0 {
                        super::super::InventoryClickButton::Left
                    } else {
                        super::super::InventoryClickButton::Right
                    },
                    &source,
                    &cursor,
                );
                if case["valid_default_counts"] == false {
                    assert!(result.is_err(), "{version:?} {case}");
                    continue;
                }
                assert_eq!(
                    result.unwrap(),
                    (
                        value(version, &case["source_after"]),
                        value(version, &case["cursor_after"])
                    ),
                    "{version:?} {case}"
                );
            }
        }
    }
    #[test]
    fn pickup_rejects_nondefault_identity_and_count_without_normalizing_received_stack() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let mut item = value(
                version,
                &serde_json::json!({"item":"minecraft:stone","count":1}),
            );
            if let SlotKnowledge::Item { item } = &mut item {
                item.count = 65;
            }
            assert!(
                pickup(
                    version,
                    "minecraft:player",
                    9,
                    super::super::InventoryClickButton::Left,
                    &item,
                    &SlotKnowledge::Empty
                )
                .is_err()
            );
            if let SlotKnowledge::Item { item } = &mut item {
                item.count = 1;
                item.data = ItemData::LegacyNbt {
                    bytes: vec![10, 0, 0, 8, 0, 1, 120, 0, 1, 121, 0],
                };
            }
            assert_eq!(
                pickup(
                    version,
                    "minecraft:player",
                    9,
                    super::super::InventoryClickButton::Left,
                    &item,
                    &SlotKnowledge::Empty
                )
                .unwrap_err()
                .kind(),
                crate::ErrorKind::Unsupported
            );
            if let SlotKnowledge::Item { item } = &mut item {
                item.data = ItemData::Default;
                item.id = Registry::for_version(version)
                    .item("minecraft:dirt")
                    .unwrap()
                    .id;
            }
            assert!(
                pickup(
                    version,
                    "minecraft:player",
                    9,
                    super::super::InventoryClickButton::Left,
                    &item,
                    &SlotKnowledge::Empty
                )
                .is_err()
            );
        }
        for profile in profiles(MinecraftVersion::Java1_21_11)
            .items
            .iter()
            .filter(|p| !p.ordinary_pickup && !p.represents_empty)
        {
            let item = value(
                MinecraftVersion::Java1_21_11,
                &serde_json::json!({"item":profile.name,"count":1}),
            );
            for (source, cursor) in [
                (&item, &SlotKnowledge::Empty),
                (&SlotKnowledge::Empty, &item),
            ] {
                assert_eq!(
                    pickup(
                        MinecraftVersion::Java1_21_11,
                        "minecraft:player",
                        9,
                        super::super::InventoryClickButton::Right,
                        source,
                        cursor
                    )
                    .unwrap_err()
                    .kind(),
                    crate::ErrorKind::Unsupported
                );
            }
        }
    }
    #[test]
    fn admission_matches_all_original_native_whole_swaps_including_slot_refusal() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let data = cases(version);
            let cases = data["swaps"].as_array().unwrap();
            assert_eq!(cases.len(), 7500);
            let mut refused = 0;
            for case in cases {
                let source = value(version, &case["source_before"]);
                let hotbar = value(version, &case["hotbar_before"]);
                let allowed = validate_swap(
                    version,
                    case["menu"].as_str().unwrap(),
                    case["slot"].as_u64().unwrap().try_into().unwrap(),
                    &source,
                    &hotbar,
                )
                .is_ok();
                assert_eq!(allowed, case["slot_allows_hotbar"].as_bool().unwrap());
                let (expected_source, expected_hotbar) = if allowed {
                    (&hotbar, &source)
                } else {
                    refused += 1;
                    (&source, &hotbar)
                };
                assert_eq!(&value(version, &case["source_after"]), expected_source);
                assert_eq!(&value(version, &case["hotbar_after"]), expected_hotbar);
                if version == MinecraftVersion::Java1_16_1 {
                    assert_eq!(
                        value(version, &case["legacy_returned"]),
                        SlotKnowledge::Empty
                    );
                }
            }
            assert_eq!(refused, 50);
            assert!(
                validate_swap(
                    version,
                    "minecraft:unknown",
                    0,
                    &SlotKnowledge::Empty,
                    &SlotKnowledge::Empty
                )
                .is_err()
            );
            // Player crafting/result/armor/offhand policies were deliberately not audited here.
            for slot in [0, 1, 5, 8, 45, 46] {
                assert!(
                    validate_swap(
                        version,
                        "minecraft:player",
                        slot,
                        &SlotKnowledge::Empty,
                        &SlotKnowledge::Empty
                    )
                    .is_err()
                );
            }
        }
    }
    #[test]
    fn original_item_registry_capacities_and_native_player_mappings_match_common_definitions() {
        for (version, text, item_count) in [
            (
                MinecraftVersion::Java1_16_1,
                include_str!("../../../data/client_api/regular_click_profiles-1.16.1.json"),
                975,
            ),
            (
                MinecraftVersion::Java1_21_11,
                include_str!("../../../data/client_api/regular_click_profiles-1.21.11.json"),
                1505,
            ),
        ] {
            let data: serde_json::Value = serde_json::from_str(text).unwrap();
            let items = data["items"].as_array().unwrap();
            assert_eq!(items.len(), item_count);
            let registry = Registry::for_version(version);
            let mut overrides = 0;
            for item in items {
                if item["represents_empty"] == true {
                    // AIR is native empty, not an inventory stack; its sentinel
                    // capacity is not an item-capacity lookup (modern returns 1).
                    assert_eq!(item["name"], "minecraft:air");
                    assert_eq!(item["native_id"], 0);
                    continue;
                }
                let definition = registry
                    .item(item["name"].as_str().unwrap())
                    .unwrap_or_else(|e| panic!("{version:?} native item {item}: {e}"));
                assert_eq!(
                    definition.id.value(),
                    item["native_id"].as_i64().unwrap() as i32
                );
                assert_eq!(
                    definition.max_stack_size,
                    item["maximum_stack_size"].as_u64().unwrap() as u32
                );
                if version == MinecraftVersion::Java1_21_11 {
                    assert_eq!(item["enabled_by_default_flags"], true);
                    if item["ordinary_pickup"] == false {
                        overrides += 1;
                        assert!(
                            definition.name == "minecraft:bundle"
                                || definition.name.ends_with("_bundle")
                        );
                    }
                }
            }
            assert_eq!(
                overrides,
                if version == MinecraftVersion::Java1_21_11 {
                    17
                } else {
                    0
                }
            );
            let player = data["menus"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["name"] == "minecraft:player")
                .unwrap();
            assert_eq!(player["slots"].as_array().unwrap().len(), 36);
            for slot in player["slots"].as_array().unwrap() {
                let index = slot["slot"].as_u64().unwrap();
                assert_eq!(
                    slot["raw_player_slot"].as_u64().unwrap(),
                    if index < 36 { index } else { index - 36 }
                );
                let policy = &data["slot_policies"][slot["policy"].as_u64().unwrap() as usize];
                assert_eq!(
                    policy["base_capacity"],
                    if version == MinecraftVersion::Java1_16_1 {
                        64
                    } else {
                        99
                    }
                );
                assert_eq!(policy["may_pickup"], true);
                assert!(
                    policy["rejected_default_items"]
                        .as_array()
                        .unwrap()
                        .is_empty()
                );
            }
        }
    }
    #[test]
    fn native_pickup_changes_cursor_and_returns_legacy_source_predecessor() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let data = cases(version);
            let cases = data["cases"].as_array().unwrap();
            assert_eq!(cases.len(), 18432);
            let mut valid = 0;
            let mut changed = 0;
            let mut requested_valid = 0;
            let mut fixture_clamped = 0;
            for case in cases {
                requested_valid += usize::from(case["valid_requested_counts"] == true);
                fixture_clamped += usize::from(case["requested_source"] != case["source_before"]);
                assert_eq!(case["requested_cursor"], case["cursor_before"]);
                if version == MinecraftVersion::Java1_16_1 {
                    assert_eq!(case["legacy_returned"], case["source_before"]);
                }
                if case["valid_default_counts"] != true {
                    continue;
                }
                valid += 1;
                if case["source_before"] != case["source_after"]
                    || case["cursor_before"] != case["cursor_after"]
                {
                    changed += 1;
                    // A predecessor hash requests a real cursor update on modern even when
                    // returning the entire cursor to the slot. No-op clicks must not be sent.
                    assert_ne!(case["cursor_before"], case["cursor_after"]);
                }
            }
            assert_eq!(requested_valid, 11250);
            assert_eq!(
                fixture_clamped,
                if version == MinecraftVersion::Java1_16_1 {
                    256
                } else {
                    896
                }
            );
            assert_eq!(
                valid,
                if version == MinecraftVersion::Java1_16_1 {
                    11450
                } else {
                    11950
                }
            );
            assert_eq!(
                changed,
                if version == MinecraftVersion::Java1_16_1 {
                    10540
                } else {
                    10994
                }
            );
        }
    }
}
