use super::{
    comparison::{self, Context},
    definition, framing,
};
use crate::{
    MinecraftVersion,
    client::{
        ItemComponent, SessionStamp,
        registry::{
            Registry, ServerRegistryEntry, ServerRegistryObservation, received::ReceivedRegistries,
        },
    },
};
use serde_json::Value as Json;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
};

fn compressed(bytes: &[u8]) -> Json {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut out)
        .unwrap();
    serde_json::from_slice(&out).unwrap()
}
fn components() -> Json {
    compressed(include_bytes!(
        "../../../../data/client_api/item_semantics-1.21.11.json.gz"
    ))
}
fn collect_tags(value: &Json, tags: &mut BTreeMap<String, BTreeSet<String>>) {
    if value["type"] == "holder_tag" {
        tags.entry(value["value"]["registry"].as_str().unwrap().into())
            .or_default()
            .insert(value["value"]["tag"].as_str().unwrap().into());
    }
    match value {
        Json::Array(values) => {
            for v in values {
                collect_tags(v, tags);
            }
        }
        Json::Object(values) => {
            for v in values.values() {
                collect_tags(v, tags);
            }
        }
        _ => {}
    }
}
fn fixture() -> ServerRegistryObservation {
    // The following vanilla IDs and accepted tag names are native test inputs,
    // never runtime defaults. Empty tag membership is deliberate: named-set
    // equals uses the cached lookup object rather than its current members.
    let catalog: Json = serde_json::from_str(include_str!(
        "../../../../data/client_api/registry_catalog_cases-1.21.11.json"
    ))
    .unwrap();
    let mut received = ReceivedRegistries::default();
    for row in catalog["registries"].as_array().unwrap() {
        if row["builtin"] == true {
            continue;
        }
        let entries = row["entries"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                assert_eq!(entry["id"].as_u64().unwrap() as usize, index);
                ServerRegistryEntry {
                    name: entry["name"].as_str().unwrap().into(),
                    data: vec![10, 0],
                }
            })
            .collect();
        received
            .modern_registry(row["name"].as_str().unwrap().into(), entries, 1, 0)
            .unwrap();
    }
    let mut tags = BTreeMap::new();
    let data = components();
    for row in data["components"].as_array().unwrap() {
        let native = definition(row["native_id"].as_i64().unwrap() as i32).unwrap();
        let value = framing::decode_value(
            native,
            &hex::decode(row["input_hex"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        collect_tags(&serde_json::to_value(value).unwrap(), &mut tags);
    }
    let prototypes: Json = serde_json::from_str(include_str!(
        "../../../../data/client_api/item_properties-1.21.11.json"
    ))
    .unwrap();
    for row in prototypes["prototype_values"].as_array().unwrap() {
        let value = framing::decode_value(
            definition(row["native_id"].as_i64().unwrap() as i32).unwrap(),
            &hex::decode(row["value_hex"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        collect_tags(&serde_json::to_value(value).unwrap(), &mut tags);
    }
    let mut bytes = Vec::new();
    crate::protocol::put_varint(&mut bytes, tags.len() as i32);
    for (registry, names) in tags {
        crate::protocol::put_string(&mut bytes, &registry);
        crate::protocol::put_varint(&mut bytes, names.len() as i32);
        for name in names {
            crate::protocol::put_string(&mut bytes, &name);
            bytes.push(0);
        }
    }
    received
        .receive_tags(&bytes, 2, MinecraftVersion::Java1_21_11)
        .unwrap();
    received.finish();
    received.capture(
        SessionStamp {
            version: MinecraftVersion::Java1_21_11,
            connection_id: 7,
            world_generation: 0,
        },
        2,
    )
}

#[test]
fn original_component_equivalence_groups_match_registry_bound_typed_keys() {
    let data = components();
    let registry = Registry::for_version(MinecraftVersion::Java1_21_11);
    let owner = fixture();
    let mut keys = Vec::new();
    let mut unresolved = Vec::new();
    for (index, row) in data["components"].as_array().unwrap().iter().enumerate() {
        let id = row["native_id"].as_i64().unwrap() as i32;
        let field = ItemComponent {
            definition: registry.item_component_by_native_id(id).unwrap(),
            bytes: hex::decode(row["input_hex"].as_str().unwrap()).unwrap(),
        };
        match comparison::field(&field, &owner) {
            Ok(key) => {
                let canonical = ItemComponent {
                    bytes: hex::decode(row["canonical_hex"].as_str().unwrap()).unwrap(),
                    ..field
                };
                assert_eq!(
                    key,
                    comparison::field(&canonical, &owner).unwrap(),
                    "canonical constructor index {index}"
                );
                keys.push(Some(key));
            }
            Err(error) => {
                unresolved.push((
                    index,
                    row["component"].clone(),
                    row["case"].clone(),
                    error.to_string(),
                ));
                keys.push(None);
            }
        }
    }
    let mut pairs = 0usize;
    let mut mismatches = Vec::new();
    let rows = data["components"].as_array().unwrap();
    for (a, row) in rows.iter().enumerate() {
        let Some(left) = &keys[a] else {
            continue;
        };
        for b in a..rows.len() {
            if row["native_id"] != rows[b]["native_id"] {
                continue;
            }
            let Some(right) = &keys[b] else {
                continue;
            };
            pairs += 1;
            if (left == right) != (row["group"] == rows[b]["group"]) {
                mismatches.push((
                    a,
                    b,
                    row["component"].clone(),
                    row["case"].clone(),
                    rows[b]["case"].clone(),
                ));
            }
        }
    }
    eprintln!(
        "component typed keys: {} supported, {} unresolved, {pairs} pairs",
        keys.iter().filter(|k| k.is_some()).count(),
        unresolved.len()
    );
    eprintln!("unresolved: {unresolved:?}");
    assert!(
        mismatches.is_empty(),
        "native component mismatches: {mismatches:?}"
    );
    assert_eq!(
        (
            keys.iter().filter(|k| k.is_some()).count(),
            unresolved.len(),
            pairs
        ),
        (4134, 0, 2313719)
    );
}

#[test]
fn original_nested_item_component_pairs_match_effective_typed_keys() {
    let data = compressed(include_bytes!(
        "../../../../data/client_api/nested_item_constructor_cases-1.21.11.json.gz"
    ));
    let owner = fixture();
    let keys: Vec<_> = data["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            if row["accepted"] != true {
                return None;
            }
            let native = super::definitions()
                .iter()
                .find(|v| v.name == row["component"].as_str().unwrap())
                .unwrap();
            let value = framing::decode_value(
                native,
                &hex::decode(row["input_hex"].as_str().unwrap()).unwrap(),
            )
            .unwrap();
            Some(Context::new(&owner).unwrap().component(&value, 0).unwrap())
        })
        .collect();
    let mut mismatches = Vec::new();
    for row in data["pairs"].as_array().unwrap() {
        let a = row["a"].as_u64().unwrap() as usize;
        let b = row["b"].as_u64().unwrap() as usize;
        if (keys[a] == keys[b]) != row["equal"].as_bool().unwrap() {
            mismatches.push((a, b));
        }
    }
    assert!(
        mismatches.is_empty(),
        "native nested item mismatches: {mismatches:?}"
    );
    assert_eq!(data["pairs"].as_array().unwrap().len(), 66430);
}

#[test]
fn original_holder_lookup_and_stream_pairs_match_bound_reference_and_tag_keys() {
    use crate::client::item_semantics::Component as Key;
    let data = compressed(include_bytes!(
        "../../../../data/client_api/holder_comparison_cases-1.21.11.json.gz"
    ));
    let owners: Vec<_> = (0..2)
        .map(|owner| {
            let mut registries = ReceivedRegistries::default();
            let names = if owner == 0 {
                ["example:first", "example:second"]
            } else {
                ["example:second", "example:first"]
            };
            registries
                .modern_registry(
                    "minecraft:enchantment".into(),
                    names
                        .into_iter()
                        .map(|name| ServerRegistryEntry {
                            name: name.into(),
                            data: vec![10, 0],
                        })
                        .collect(),
                    1,
                    0,
                )
                .unwrap();
            let mut tags = vec![1];
            crate::protocol::put_string(&mut tags, "minecraft:enchantment");
            tags.push(2);
            for name in ["example:tag", "example:alias"] {
                crate::protocol::put_string(&mut tags, name);
                tags.extend([1, 0]);
            }
            registries
                .receive_tags(&tags, 2, MinecraftVersion::Java1_21_11)
                .unwrap();
            registries.finish();
            registries.capture(
                SessionStamp {
                    version: MinecraftVersion::Java1_21_11,
                    connection_id: 7 + owner,
                    world_generation: 0,
                },
                2,
            )
        })
        .collect();
    let reference = |name: &str, owner: usize| {
        Key::Reference(
            owners[owner]
                .find_entry("minecraft:enchantment", name)
                .unwrap()
                .into(),
        )
    };
    let keys: Vec<_> = data["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let name = row["case"].as_str().unwrap();
            let owner = row["owner"].as_u64().unwrap() as usize;
            if name.starts_with("independent-empty-named") {
                return None;
            }
            Some(match row["kind"].as_str().unwrap() {
                "reference" => reference(row["entry"].as_str().unwrap(), owner),
                "direct_set" => Key::HolderList(
                    "minecraft:enchantment".into(),
                    row["members"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| reference(v.as_str().unwrap(), owner))
                        .collect(),
                ),
                "direct" => Key::Inline(
                    "minecraft:enchantment".into(),
                    Box::new(Key::Integer(
                        name.rsplit('-').next().unwrap().parse().unwrap(),
                    )),
                ),
                "named" => {
                    let tag = row["tag"]
                        .as_str()
                        .unwrap()
                        .split(" / ")
                        .nth(1)
                        .unwrap()
                        .trim_end_matches(']');
                    Context::new(&owners[owner])
                        .unwrap()
                        .component(
                            &super::values::Value::HolderTag {
                                registry: "minecraft:enchantment".into(),
                                tag: tag.into(),
                            },
                            0,
                        )
                        .unwrap()
                }
                _ => panic!("unknown original holder kind"),
            })
        })
        .collect();
    let mut compared = 0;
    for row in data["pairs"].as_array().unwrap() {
        let a = row["a"].as_u64().unwrap() as usize;
        let b = row["b"].as_u64().unwrap() as usize;
        if let (Some(a), Some(b)) = (&keys[a], &keys[b]) {
            assert_eq!(a == b, row["equal"].as_bool().unwrap(), "holder pair {row}");
            compared += 1;
        } else {
            assert_eq!(
                row["equal"].as_bool().unwrap(),
                a == b,
                "independent emptyNamed factory identity"
            );
        }
    }
    for failure in data["stream_failures"].as_array().unwrap() {
        assert_eq!(failure["failure"], "java.util.NoSuchElementException");
        let owner = failure["owner"].as_u64().unwrap() as usize;
        assert!(
            Context::new(&owners[owner])
                .unwrap()
                .component(
                    &super::values::Value::HolderTag {
                        registry: "minecraft:enchantment".into(),
                        tag: "example:missing".into()
                    },
                    0
                )
                .is_err()
        );
    }
    assert_eq!(
        (
            data["cases"].as_array().unwrap().len(),
            data["pairs"].as_array().unwrap().len(),
            compared
        ),
        (42, 903, 741)
    );
}

#[test]
fn original_holder_comparison_sources_and_missing_tag_failures_are_bound() {
    use sha2::{Digest, Sha256};
    let source: Json = serde_json::from_str(include_str!(
        "../../../../data/client_api/holder_comparison_source.json"
    ))
    .unwrap();
    let digest = |bytes: &[u8]| hex::encode(Sha256::digest(bytes));
    for (name, bytes) in [
        (
            "scripts/ExportHolderComparison.java",
            include_bytes!("../../../../scripts/ExportHolderComparison.java").as_slice(),
        ),
        (
            "scripts/export_holder_comparison.py",
            include_bytes!("../../../../scripts/export_holder_comparison.py").as_slice(),
        ),
        (
            "scripts/ExportInventoryTransfers.java",
            include_bytes!("../../../../scripts/ExportInventoryTransfers.java").as_slice(),
        ),
        (
            "scripts/ExportItemComponents.java",
            include_bytes!("../../../../scripts/ExportItemComponents.java").as_slice(),
        ),
        (
            "scripts/ExportItemProperties.java",
            include_bytes!("../../../../scripts/ExportItemProperties.java").as_slice(),
        ),
    ] {
        assert_eq!(
            source["generators_sha256"][name],
            digest(bytes),
            "native source {name}"
        );
    }
    assert_eq!(
        source["files_sha256"]["data/client_api/holder_comparison_cases-1.21.11.json.gz"],
        digest(include_bytes!(
            "../../../../data/client_api/holder_comparison_cases-1.21.11.json.gz"
        ))
    );
    assert_eq!(
        (
            source["cases"].as_u64().unwrap(),
            source["pairs"].as_u64().unwrap()
        ),
        (42, 903)
    );
    assert_eq!(
        source["original_server_jar_sha1"],
        "64bb6d763bed0a9f1d632ec347938594144943ed"
    );
}

#[test]
fn original_compound_component_decoders_reject_end_and_scalar_roots() {
    let data = compressed(include_bytes!(
        "../../../../data/client_api/holder_comparison_cases-1.21.11.json.gz"
    ));
    let registry = Registry::for_version(MinecraftVersion::Java1_21_11);
    let owner = fixture();
    let rows = data["nbt_boundaries"].as_array().unwrap();
    assert_eq!(rows.len(), 12);
    for row in rows {
        let field = ItemComponent {
            definition: registry
                .item_component_by_native_id(
                    super::definitions()
                        .iter()
                        .find(|d| d.name == row["component"].as_str().unwrap())
                        .unwrap()
                        .native_id,
                )
                .unwrap(),
            bytes: hex::decode(row["input_hex"].as_str().unwrap()).unwrap(),
        };
        let accepted = row["accepted"].as_bool().unwrap();
        assert_eq!(comparison::field(&field, &owner).is_ok(), accepted, "{row}");
        let native = definition(field.definition.id.value()).unwrap();
        let mut reader = super::Reader::new(&field.bytes);
        let mut budget = framing::Budget::new(&reader);
        assert_eq!(
            framing::value(&mut reader, native, &mut budget, 0).is_ok(),
            accepted,
            "receive {row}"
        );
    }
}

fn item_key(
    bytes: &[u8],
    owner: &ServerRegistryObservation,
) -> crate::client::item_semantics::Item {
    use crate::client::{ItemData, ItemStack, item_semantics::Item};
    let mut reader = super::Reader::new(bytes);
    let count = reader.varint().unwrap();
    if count <= 0 {
        reader.end().unwrap();
        return Item::empty();
    }
    let id = reader.varint().unwrap();
    let patch = super::read_patch(&mut reader).unwrap().unwrap();
    reader.end().unwrap();
    if id == 0 {
        return Item::empty();
    }
    let definition = Registry::for_version(MinecraftVersion::Java1_21_11)
        .item_by_native_id(id)
        .unwrap();
    comparison::stack(
        &ItemStack {
            id: definition.id,
            name: definition.name,
            count: count as u32,
            data: if patch.added.is_empty() && patch.removed.is_empty() {
                ItemData::Default
            } else {
                ItemData::ModernComponents { patch }
            },
        },
        owner,
    )
    .unwrap()
}

#[test]
fn original_items_and_prototype_operations_match_effective_typed_comparison() {
    let data = components();
    let owner = fixture();
    let mut cache = BTreeMap::new();
    let mut get = |hex: &str| {
        cache
            .entry(hex.to_owned())
            .or_insert_with(|| item_key(&hex::decode(hex).unwrap(), &owner))
            .clone()
    };
    let mut keys = Vec::new();
    let mut groups = BTreeMap::new();
    for (index, row) in data["items"].as_array().unwrap().iter().enumerate() {
        let key = get(row["input_hex"].as_str().unwrap());
        assert_eq!(
            key,
            get(row["canonical_hex"].as_str().unwrap()),
            "native item canonical {index}"
        );
        let group = row["group"].as_u64().unwrap();
        if let Some(previous) = groups.insert(group, key.clone()) {
            assert_eq!(previous, key, "native same item group {group}");
        }
        keys.push((group, key));
    }
    // Distinct native groups must remain distinct; inspect every group pair,
    // avoiding an inference based only on identical-input/canonical checks.
    let groups: Vec<_> = groups.into_iter().collect();
    for a in 0..groups.len() {
        for b in a + 1..groups.len() {
            assert_ne!(
                groups[a].1, groups[b].1,
                "distinct native item groups {} {}",
                groups[a].0, groups[b].0
            );
        }
    }
    for (index, row) in data["prototype_cases"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let base = get(row["base_hex"].as_str().unwrap());
        let changed = get(row["changed_hex"].as_str().unwrap());
        assert_eq!(
            base == changed,
            row["matches"].as_bool().unwrap(),
            "native prototype matches {index} {}",
            row["case"]
        );
        assert_eq!(
            base.native_id == changed.native_id && base.components == changed.components,
            row["same_item_data"].as_bool().unwrap(),
            "native prototype item data {index} {}",
            row["case"]
        );
        assert_eq!(
            changed,
            get(row["canonical_changed_hex"].as_str().unwrap()),
            "native prototype canonical {index}"
        );
    }
    assert_eq!(
        (
            keys.len(),
            data["prototype_cases"].as_array().unwrap().len()
        ),
        (13853, 24789)
    );
    eprintln!(
        "modern item keys: {} inputs, {} groups,24789 prototype operations",
        keys.len(),
        groups.len()
    );
}
