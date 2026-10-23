use super::*;
use std::io::Read;

#[test]
fn crafting_input_swap_original_every_input_mode_hotbar_capacity_and_data() {
    let version = crate::MinecraftVersion::Java1_21_11;
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(
        &include_bytes!("../../../data/client_api/crafting_input_swap_cases-1.21.11.json.gz")[..],
    )
    .read_to_end(&mut raw)
    .unwrap();
    let facts: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    let cases = facts["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 2600);
    for (index, case) in cases.iter().enumerate() {
        let request = &case["request"];
        let menu = request["menu"].as_str().unwrap();
        let input = request["slot"].as_u64().unwrap() as usize;
        let hotbar = request["hotbar"].as_u64().unwrap() as u8;
        let destination =
            crate::client::crafting::layout::input_swap(version, menu, input, hotbar).unwrap();
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
        slot_policy::validate_crafting_input_swap(
            version,
            menu,
            input,
            &before[input],
            &before[destination],
        )
        .unwrap();
        let mut expected = before.clone();
        expected.swap(input, destination);
        let semantic = |values: &[SlotKnowledge]| {
            values
                .iter()
                .map(|v| armor_tests::preserved(version, v))
                .collect::<Vec<_>>()
        };
        assert_eq!(semantic(&expected), semantic(&after), "case {index}");
        assert_eq!(case["cursor_before_hex"], case["cursor_after_hex"]);
        assert_eq!(case["actual_creative"], request["mode"] == "creative");
    }
    for (menu, end) in [("minecraft:player", 5), ("minecraft:crafting", 10)] {
        for input in [0, end, 45] {
            assert!(crate::client::crafting::layout::input_swap(version, menu, input, 0).is_none());
            assert!(
                slot_policy::validate_crafting_input_swap(
                    version,
                    menu,
                    input,
                    &SlotKnowledge::Empty,
                    &SlotKnowledge::Empty
                )
                .is_err()
            );
        }
        assert!(crate::client::crafting::layout::input_swap(version, menu, 1, 9).is_none());
    }
}

#[test]
fn crafting_input_swap_original_inputs_and_artifacts_have_owned_hashes() {
    use sha2::{Digest, Sha256};
    let evidence: serde_json::Value = serde_json::from_str(include_str!(
        "../../../data/client_api/crafting_input_swap_source.json"
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
