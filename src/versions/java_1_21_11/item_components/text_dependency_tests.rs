use super::{comparison, comparison_tests::fixture};
use crate::{
    MinecraftVersion,
    client::{ItemComponent, registry::Registry},
};
use serde_json::Value as Json;
use std::io::Read;
fn facts() -> Json {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(
        include_bytes!("../../../../data/client_api/text_dependency_cases-1.21.11.json.gz")
            .as_slice(),
    )
    .read_to_end(&mut bytes)
    .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
#[test]
fn original_text_dependency_fields_constructors_and_native_pairs_match_bound_keys() {
    let data = facts();
    let owner = fixture();
    let registry = Registry::for_version(MinecraftVersion::Java1_21_11);
    let definition = registry.item_component("minecraft:custom_name").unwrap();
    let mut keys = Vec::new();
    let mut unsupported = Vec::new();
    for (index, row) in data["cases"].as_array().unwrap().iter().enumerate() {
        let field = ItemComponent {
            definition: definition.clone(),
            bytes: hex::decode(row["input_hex"].as_str().unwrap()).unwrap(),
        };
        let value = comparison::field(&field, &owner);
        if row["accepted"] == false {
            assert!(value.is_err(), "native reject {index} {}", row["case"]);
            keys.push(None);
            continue;
        }
        let key = match value {
            Ok(v) => v,
            Err(error) => {
                unsupported.push((index, row["case"].clone(), error.to_string()));
                keys.push(None);
                continue;
            }
        };
        if let Some(item_hex) = row["fields"]["style"]["hover_item_hex"].as_str() {
            let crate::client::item_semantics::Component::Text(text) = &key else {
                panic!("native text key required")
            };
            assert_eq!(
                text.hover_item().unwrap(),
                &super::comparison_tests::item_key(&hex::decode(item_hex).unwrap(), &owner),
                "native hover getter {index}"
            );
        }
        if let Some(hex) = row["canonical_hex"].as_str() {
            assert_eq!(
                key,
                comparison::field(
                    &ItemComponent {
                        definition: definition.clone(),
                        bytes: hex::decode(hex).unwrap()
                    },
                    &owner
                )
                .unwrap(),
                "canonical {index} {}",
                row["case"]
            );
        }
        keys.push(Some(key));
    }
    let mut matched = 0;
    for pair in data["pairs"].as_array().unwrap() {
        let a = pair["a"].as_u64().unwrap() as usize;
        let b = pair["b"].as_u64().unwrap() as usize;
        if let (Some(a_key), Some(b_key)) = (&keys[a], &keys[b]) {
            assert_eq!(
                a_key == b_key,
                pair["equal"].as_bool().unwrap(),
                "native pair {a} {b}"
            );
            matched += 1;
        }
    }
    eprintln!(
        "text dependencies: supported{} unsupported{:?} pairs{}",
        keys.iter().flatten().count(),
        unsupported,
        matched
    );
    assert_eq!(
        (keys.iter().flatten().count(), unsupported.len(), matched),
        (413, 4, 85491)
    );
    assert!(unsupported.iter().all(|(_, case, error)| {
        let name = case.as_str().unwrap();
        (name.starts_with("dialog-{") && error.contains("inline dialog"))
            || (name.starts_with("unresolved-") && error.contains("persistent hover item"))
    }));
}

#[test]
fn original_text_dependency_source_is_bound_to_tools_runtime_and_inputs() {
    use sha2::{Digest, Sha256};
    let source: Json = serde_json::from_str(include_str!(
        "../../../../data/client_api/text_dependency_source.json"
    ))
    .unwrap();
    for (name, bytes) in [
        (
            "scripts/ExportTextDependencies.java",
            include_bytes!("../../../../scripts/ExportTextDependencies.java").as_slice(),
        ),
        (
            "scripts/export_text_dependencies.py",
            include_bytes!("../../../../scripts/export_text_dependencies.py").as_slice(),
        ),
        (
            "scripts/ExportTextCore.java",
            include_bytes!("../../../../scripts/ExportTextCore.java").as_slice(),
        ),
        (
            "scripts/ExportItemProperties.java",
            include_bytes!("../../../../scripts/ExportItemProperties.java").as_slice(),
        ),
    ] {
        assert_eq!(
            source["generators_sha256"][name],
            hex::encode(Sha256::digest(bytes)),
            "{name}"
        );
    }
    assert_eq!(
        source["files_sha256"]["data/client_api/text_dependency_cases-1.21.11.json.gz"],
        hex::encode(Sha256::digest(include_bytes!(
            "../../../../data/client_api/text_dependency_cases-1.21.11.json.gz"
        )))
    );
    assert_eq!(
        (
            source["cases"].as_u64().unwrap(),
            source["accepted"].as_u64().unwrap(),
            source["pairs"].as_u64().unwrap()
        ),
        (507, 417, 87153)
    );
    assert_eq!(
        source["original_server_jar_sha1"],
        "64bb6d763bed0a9f1d632ec347938594144943ed"
    );
    for name in [
        "java_executable_sha256",
        "jdk_modules_sha256",
        "generated_compiler_sha256",
        "requests_sha256",
        "raw_output_sha256",
    ] {
        assert_eq!(source[name].as_str().unwrap().len(), 64, "{name}");
    }
}

#[test]
fn missing_hover_prototype_tag_context_remains_unresolved() {
    use crate::client::{SessionStamp, registry::received::ReceivedRegistries};
    let mut received = ReceivedRegistries::default();
    received.finish();
    let owner = received.capture(
        SessionStamp {
            version: MinecraftVersion::Java1_21_11,
            connection_id: 7,
            world_generation: 0,
        },
        0,
    );
    let data = facts();
    let row = data["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["case"] == "item-id-minecraft:diamond_sword")
        .unwrap();
    assert_eq!(row["accepted"], true);
    let field = ItemComponent {
        definition: Registry::for_version(MinecraftVersion::Java1_21_11)
            .item_component("minecraft:custom_name")
            .unwrap(),
        bytes: hex::decode(row["input_hex"].as_str().unwrap()).unwrap(),
    };
    let error = comparison::field(&field, &owner).unwrap_err();
    assert!(
        error
            .downcast_ref::<crate::client::constructor::Unresolved>()
            .is_some(),
        "{error:#}"
    );
}
