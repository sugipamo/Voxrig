//! Native default left PICKUP facts for known constructors and empty-boundary overrides.
use crate::{
    MinecraftVersion,
    client::{ItemData, ItemStack},
};
use std::sync::OnceLock;
#[derive(serde::Deserialize)]
struct Profiles {
    items: Vec<Item>,
}
#[derive(serde::Deserialize)]
struct Item {
    name: String,
    native_id: i32,
    default_legacy_nbt: Option<Vec<u8>>,
}
fn profiles(version: MinecraftVersion) -> &'static Profiles {
    static LEGACY: OnceLock<Profiles> = OnceLock::new();
    static MODERN: OnceLock<Profiles> = OnceLock::new();
    match version {
        MinecraftVersion::Java1_16_1 => LEGACY.get_or_init(|| {
            serde_json::from_str(include_str!(
                "../../../data/client_api/cursor_return_profiles-1.16.1.json"
            ))
            .expect("pinned cursor return profiles")
        }),
        MinecraftVersion::Java1_21_11 => MODERN.get_or_init(|| {
            serde_json::from_str(include_str!(
                "../../../data/client_api/cursor_return_profiles-1.21.11.json"
            ))
            .expect("pinned cursor return profiles")
        }),
    }
}
pub(super) fn default_left_item(version: MinecraftVersion, item: &ItemStack) -> bool {
    let Some(profile) = profiles(version)
        .items
        .iter()
        .find(|p| p.native_id == item.id.value() && p.name == item.name)
    else {
        return false;
    };
    match &item.data {
        ItemData::Default => true,
        ItemData::LegacyNbt { bytes } => {
            version == MinecraftVersion::Java1_16_1
                && profile.default_legacy_nbt.as_ref() == Some(bytes)
        }
        ItemData::ModernComponents { .. } => false,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{
        SlotKnowledge,
        inventory::{InventoryClickButton, slot_policy},
        registry::Registry,
    };
    use std::io::Read;
    fn value(version: MinecraftVersion, v: &serde_json::Value) -> SlotKnowledge {
        let Some(name) = v["item"].as_str() else {
            return SlotKnowledge::Empty;
        };
        let definition = Registry::for_version(version).item(name).unwrap();
        let bytes: Vec<u8> = v["nbt"].as_array().map_or(Vec::new(), |a| {
            a.iter().map(|b| b.as_u64().unwrap() as u8).collect()
        });
        SlotKnowledge::Item {
            item: ItemStack {
                id: definition.id,
                name: definition.name,
                count: v["count"].as_u64().unwrap() as u32,
                data: if bytes.is_empty() || bytes == [0] {
                    ItemData::Default
                } else {
                    ItemData::LegacyNbt { bytes }
                },
            },
        }
    }
    #[test]
    fn cursor_return_facts_are_bound_to_original_tools_and_every_packaged_output() {
        use sha2::{Digest, Sha256};
        let source: serde_json::Value = serde_json::from_str(include_str!(
            "../../../data/client_api/cursor_return_source.json"
        ))
        .unwrap();
        for (name, raw) in [
            (
                "scripts/ExportCursorReturns.java",
                include_bytes!("../../../scripts/ExportCursorReturns.java").as_slice(),
            ),
            (
                "scripts/ExportInventoryTransfers.java",
                include_bytes!("../../../scripts/ExportInventoryTransfers.java").as_slice(),
            ),
            (
                "scripts/export_cursor_returns.py",
                include_bytes!("../../../scripts/export_cursor_returns.py").as_slice(),
            ),
            (
                "scripts/export_regular_clicks.py",
                include_bytes!("../../../scripts/export_regular_clicks.py").as_slice(),
            ),
        ] {
            assert_eq!(
                source["generators_sha256"][name],
                format!("{:x}", Sha256::digest(raw))
            );
        }
        for (index, cases, profiles, packets) in [
            (
                0,
                include_bytes!("../../../data/client_api/cursor_return_cases-1.16.1.json.gz")
                    .as_slice(),
                include_bytes!("../../../data/client_api/cursor_return_profiles-1.16.1.json")
                    .as_slice(),
                include_bytes!("../../../data/client_api/cursor_return_packets-1.16.1.json")
                    .as_slice(),
            ),
            (
                1,
                include_bytes!("../../../data/client_api/cursor_return_cases-1.21.11.json.gz")
                    .as_slice(),
                include_bytes!("../../../data/client_api/cursor_return_profiles-1.21.11.json")
                    .as_slice(),
                include_bytes!("../../../data/client_api/cursor_return_packets-1.21.11.json")
                    .as_slice(),
            ),
        ] {
            let run = &source["runs"][index];
            let version = run["version"].as_str().unwrap();
            for (kind, suffix, raw) in [
                ("cases", ".json.gz", cases),
                ("profiles", ".json", profiles),
                ("packets", ".json", packets),
            ] {
                let path = format!("data/client_api/cursor_return_{kind}-{version}{suffix}");
                assert_eq!(
                    run["files_sha256"][&path],
                    format!("{:x}", Sha256::digest(raw))
                );
            }
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(packets)
                    .unwrap()
                    .as_array()
                    .unwrap()
                    .len() as u64,
                run["codec_roundtrips"].as_u64().unwrap()
            );
        }
    }
    #[test]
    fn default_left_predictions_preserve_all_original_native_return_take_merge_cases() {
        for (version, raw) in [
            (
                MinecraftVersion::Java1_16_1,
                include_bytes!("../../../data/client_api/cursor_return_cases-1.16.1.json.gz")
                    .as_slice(),
            ),
            (
                MinecraftVersion::Java1_21_11,
                include_bytes!("../../../data/client_api/cursor_return_cases-1.21.11.json.gz")
                    .as_slice(),
            ),
        ] {
            let mut data = String::new();
            flate2::read::GzDecoder::new(raw)
                .read_to_string(&mut data)
                .unwrap();
            let data: serde_json::Value = serde_json::from_str(&data).unwrap();
            for c in data["cases"].as_array().unwrap() {
                let source = value(version, &c["source_before"]);
                let cursor = value(version, &c["cursor_before"]);
                let (after, held) = slot_policy::pickup(
                    version,
                    "minecraft:generic_9x3",
                    c["slot"].as_u64().unwrap() as usize,
                    InventoryClickButton::Left,
                    &source,
                    &cursor,
                )
                .unwrap_or_else(|e| panic!("{version:?} {c}: {e}"));
                assert_eq!(after, value(version, &c["source_after"]), "{version:?} {c}");
                assert_eq!(held, value(version, &c["cursor_after"]), "{version:?} {c}");
                if version == MinecraftVersion::Java1_16_1 {
                    assert_eq!(source, value(version, &c["legacy_returned"]));
                }
            }
        }
    }
}
