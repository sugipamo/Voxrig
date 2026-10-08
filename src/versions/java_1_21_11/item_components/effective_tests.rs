use super::{Reader, definition, framing, read_patch};
use crate::{
    MinecraftVersion,
    client::{ItemComponent, item_components::ComponentFields, modern_prototype_components},
};
use serde_json::Value as Json;
use std::{collections::BTreeMap, io::Read};

fn facts() -> Json {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(
        &include_bytes!(
            "../../../../data/client_api/effective_item_component_cases-1.21.11.json.gz"
        )[..],
    )
    .read_to_end(&mut bytes)
    .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
fn fields(value: &ItemComponent, cache: &mut BTreeMap<(i32, Vec<u8>), Json>) -> Json {
    let key = (value.definition.id.value(), value.bytes.clone());
    cache
        .entry(key)
        .or_insert_with(|| {
            serde_json::to_value(
                framing::decode_value(
                    definition(value.definition.id.value()).unwrap(),
                    &value.bytes,
                )
                .unwrap(),
            )
            .unwrap()
        })
        .clone()
}

#[test]
fn original_effective_item_fields_match_prototypes_additions_and_removals() {
    let facts = facts();
    let registry = crate::client::registry::Registry::for_version(MinecraftVersion::Java1_21_11);
    let mut cache = BTreeMap::new();
    let native: Vec<_> = facts["values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let field = ItemComponent {
                definition: registry
                    .item_component_by_native_id(row["native_id"].as_i64().unwrap() as i32)
                    .unwrap(),
                bytes: hex::decode(row["value_hex"].as_str().unwrap()).unwrap(),
            };
            assert_eq!(field.definition.name, row["name"]);
            (field.definition.id.value(), fields(&field, &mut cache))
        })
        .collect();
    for row in facts["cases"].as_array().unwrap() {
        let bytes = hex::decode(row["input_hex"].as_str().unwrap()).unwrap();
        let mut reader = Reader::new(&bytes);
        let count = reader.varint().unwrap();
        if count <= 0 {
            reader.end().unwrap();
            assert!(row["empty"].as_bool().unwrap());
            assert_eq!(row["count"], 0);
            assert!(row["components"].as_array().unwrap().is_empty());
            continue;
        }
        let id = reader.varint().unwrap();
        let patch = read_patch(&mut reader).unwrap().unwrap();
        reader.end().unwrap();
        if id == 0 {
            assert!(row["empty"].as_bool().unwrap());
            assert_eq!(row["count"], 0);
            assert!(row["components"].as_array().unwrap().is_empty());
            continue;
        }
        assert!(!row["empty"].as_bool().unwrap());
        assert_eq!(count, row["count"].as_i64().unwrap() as i32);
        let effective = ComponentFields::apply(
            MinecraftVersion::Java1_21_11,
            modern_prototype_components(id).unwrap(),
            Some(&patch),
        )
        .unwrap();
        let expected: BTreeMap<_, _> = row["components"]
            .as_array()
            .unwrap()
            .iter()
            .map(|index| native[index.as_u64().unwrap() as usize].clone())
            .collect();
        let actual: BTreeMap<_, _> = effective
            .iter()
            .map(|field| (field.definition.id.value(), fields(field, &mut cache)))
            .collect();
        assert_eq!(actual, expected, "input {}", row["input_hex"]);
    }
    // Structural field observations are tested here. JSON equality is not a
    // production component identity, native equals or live registry binding.
    assert_eq!(
        (facts["cases"].as_array().unwrap().len(), native.len()),
        (38_218, 3_694)
    );
    assert_eq!(
        native
            .iter()
            .map(|(id, _)| *id)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        104
    );
}

#[test]
fn original_effective_item_field_sources_and_request_coverage_are_bound() {
    use sha2::{Digest, Sha256};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source: Json = serde_json::from_str(include_str!(
        "../../../../data/client_api/effective_item_component_source.json"
    ))
    .unwrap();
    for group in ["generators_sha256", "files_sha256"] {
        for (path, hash) in source[group].as_object().unwrap() {
            assert_eq!(
                hex::encode(Sha256::digest(std::fs::read(root.join(path)).unwrap())),
                hash.as_str().unwrap(),
                "{path}"
            );
        }
    }
    let facts = facts();
    assert_eq!(source["cases"], facts["cases"].as_array().unwrap().len());
    assert_eq!(source["values"], facts["values"].as_array().unwrap().len());
    let observed: std::collections::BTreeSet<_> = facts["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["input_hex"].as_str().unwrap())
        .collect();
    let mut native = Vec::new();
    flate2::read::GzDecoder::new(
        &include_bytes!("../../../../data/client_api/item_semantics-1.21.11.json.gz")[..],
    )
    .read_to_end(&mut native)
    .unwrap();
    let native: Json = serde_json::from_slice(&native).unwrap();
    for row in native["items"].as_array().unwrap() {
        assert!(observed.contains(row["input_hex"].as_str().unwrap()));
    }
    for row in native["prototype_cases"].as_array().unwrap() {
        for key in ["base_hex", "changed_hex"] {
            assert!(observed.contains(row[key].as_str().unwrap()));
        }
    }
    let mut prefix = vec![1];
    let registry = crate::client::registry::Registry::for_version(MinecraftVersion::Java1_21_11);
    crate::protocol::put_varint(
        &mut prefix,
        registry.item("minecraft:stone").unwrap().id.value(),
    );
    prefix.extend_from_slice(&[1, 0]);
    for row in native["components"].as_array().unwrap() {
        let mut wire = prefix.clone();
        crate::protocol::put_varint(&mut wire, row["native_id"].as_i64().unwrap() as i32);
        wire.extend_from_slice(&hex::decode(row["input_hex"].as_str().unwrap()).unwrap());
        assert!(observed.contains(hex::encode(wire).as_str()));
    }
    let components: Json = serde_json::from_str(include_str!(
        "../../../../data/client_api/item_component_cases-1.21.11.json"
    ))
    .unwrap();
    for row in components["stacks"].as_array().unwrap() {
        assert!(observed.contains(row["stack_hex"].as_str().unwrap()));
    }
}
