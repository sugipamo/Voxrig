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
struct SlotPolicy {
    may_pickup: bool,
    base_capacity: u32,
    rejected_default_items: Vec<String>,
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
/// Original native default-stack capacity, tied to both registry ID and name.
/// AIR's empty sentinel is not an inventory stack/capacity definition.
pub(crate) fn default_item_capacity(version: MinecraftVersion, id: i32, name: &str) -> Option<u32> {
    profiles(version)
        .items
        .iter()
        .find(|i| i.native_id == id && i.name == name && !i.represents_empty)
        .map(|i| i.maximum_stack_size)
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
