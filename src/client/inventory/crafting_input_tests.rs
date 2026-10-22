//! Original QUICK_MOVE with occupied cursors and retained item data.
use super::*;
use crate::MinecraftVersion;
use std::io::Read;

#[test]
fn crafting_input_transfer_original_all_menus_preserve_cursor_and_slot_outcomes() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let bytes: &[u8] = match version {
            MinecraftVersion::Java1_16_1 => {
                include_bytes!(
                    "../../../data/client_api/crafting_input_transfer_cases-1.16.1.json.gz"
                )
            }
            MinecraftVersion::Java1_21_11 => include_bytes!(
                "../../../data/client_api/crafting_input_transfer_cases-1.21.11.json.gz"
            ),
        };
        let mut raw = Vec::new();
        flate2::read::GzDecoder::new(bytes)
            .read_to_end(&mut raw)
            .unwrap();
        let facts: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        let cases = facts["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 2496);
        let context = armor_tests::context(version, None);
        for (index, case) in cases.iter().enumerate() {
            let request = &case["request"];
            let read = |field: &str| {
                case[field]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| armor_tests::slot(version, v.as_str().unwrap()))
                    .collect::<Vec<_>>()
            };
            let before = read("before");
            let after = read("after");
            let mode = if request["mode"] == "creative" {
                GameMode::Creative
            } else {
                GameMode::Survival
            };
            assert_eq!(case["actual_creative"], mode == GameMode::Creative);
            let held = armor_tests::slot(version, case["cursor_before_hex"].as_str().unwrap());
            transfer_policy::validate(version, &held, Some(&context)).unwrap();
            assert_eq!(
                case["cursor_before_hex"], case["cursor_after_hex"],
                "{version:?} case{index}"
            );
            let predicted = transfer_policy::calculate_with_data(
                version,
                request["menu"].as_str().unwrap(),
                request["slot"].as_u64().unwrap() as usize,
                &before,
                mode,
                &context,
            )
            .unwrap();
            // Native copies can serialize NBT compounds in another wire order.
            // Compare the independent decoded trees, retaining numeric types/bits.
            let preserved = |values: &[SlotKnowledge]| {
                values
                    .iter()
                    .map(|v| armor_tests::preserved(version, v))
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                preserved(&predicted.slots),
                preserved(&after),
                "{version:?} case{index}"
            );
            if version == MinecraftVersion::Java1_16_1 {
                assert_eq!(
                    armor_tests::preserved(version, &predicted.legacy_return),
                    armor_tests::preserved(
                        version,
                        &armor_tests::slot(version, case["legacy_return_hex"].as_str().unwrap())
                    )
                );
            }
        }
    }
}
#[test]
fn crafting_input_transfer_original_evidence_binds_owned_sources_and_outputs() {
    use sha2::{Digest, Sha256};
    let evidence: serde_json::Value = serde_json::from_str(include_str!(
        "../../../data/client_api/crafting_input_transfer_source.json"
    ))
    .unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for object in std::iter::once(&evidence["generators_sha256"]).chain(
        evidence["runs"]
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
