use super::*;
use crate::client::{
    SessionStamp,
    registry::{ServerRegistryEntry, received::ReceivedRegistries},
};
use serde_json::Value as Json;
fn facts(version: MinecraftVersion) -> Json {
    serde_json::from_str(match version {
        MinecraftVersion::Java1_16_1 => {
            include_str!("../../../../data/client_api/registry_catalog_cases-1.16.1.json")
        }
        MinecraftVersion::Java1_21_11 => {
            include_str!("../../../../data/client_api/registry_catalog_cases-1.21.11.json")
        }
    })
    .unwrap()
}
fn session(version: MinecraftVersion, connection_id: u64, world_generation: u64) -> SessionStamp {
    SessionStamp {
        version,
        connection_id,
        world_generation,
    }
}
#[test]
fn original_builtin_registry_entries_and_dynamic_exclusions_match_both_versions() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let native = facts(version);
        let registry = Registry::for_version(version);
        let mut counts = (0, 0, 0);
        for row in native["registries"].as_array().unwrap() {
            let key = row["name"].as_str().unwrap();
            let builtin = row["builtin"].as_bool().unwrap();
            assert_eq!(is_builtin(version, key).unwrap(), builtin);
            if builtin {
                counts.0 += 1;
            } else {
                counts.1 += 1;
                assert!(catalog(version).registries[key].entries.is_empty());
            }
            for entry in row["entries"].as_array().unwrap() {
                let id = entry["id"].as_i64().unwrap() as i32;
                let name = entry["name"].as_str().unwrap();
                let bound = registry.builtin_id_by_native_id(key, id);
                assert_eq!(bound.is_ok(), builtin, "{version:?} {key} {name}");
                if builtin {
                    let bound = bound.unwrap();
                    assert_eq!(bound, registry.builtin_id(key, name).unwrap());
                    assert_eq!(
                        (bound.version(), bound.registry(), bound.value()),
                        (version, key, id)
                    );
                    assert_eq!(registry.builtin_name(&bound).unwrap(), name);
                    let other = Registry::for_version(if version == MinecraftVersion::Java1_16_1 {
                        MinecraftVersion::Java1_21_11
                    } else {
                        MinecraftVersion::Java1_16_1
                    });
                    assert!(other.builtin_name(&bound).is_err());
                    counts.2 += 1;
                }
            }
            assert!(registry.builtin_id_by_native_id(key, -1).is_err());
            assert!(registry.builtin_id(key, "example:missing").is_err());
        }
        assert_eq!(
            (counts.0, counts.1),
            if version == MinecraftVersion::Java1_16_1 {
                (47, 1)
            } else {
                (95, 38)
            }
        );
        assert!(counts.2 > 1000);
        assert!(is_builtin(version, "example:unknown_registry").is_err());
        // Block-state identities cannot enter the general block-entry namespace.
        let block = registry
            .builtin_id("minecraft:block", "minecraft:stone")
            .unwrap();
        let item = registry
            .builtin_id("minecraft:item", "minecraft:stone")
            .unwrap();
        assert_ne!(block, item);
    }
}

#[test]
fn original_legacy_codec_entry_order_names_and_raw_fields_match_common_resolution() {
    let native = facts(MinecraftVersion::Java1_16_1);
    let mut cases = native["legacy_codec_cases"].as_array().unwrap().clone();
    let dimensions = native["registries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "minecraft:dimension_type")
        .unwrap();
    cases.push(serde_json::json!({"case":"builtin", "codec_hex":native["legacy_builtin_codec_hex"], "entries":dimensions["entries"]}));
    for (i, case) in cases.iter().enumerate() {
        let bytes = hex::decode(case["codec_hex"].as_str().unwrap()).unwrap();
        let mut state = ReceivedRegistries::default();
        state.legacy_join(bytes.clone(), 10 + i as u64).unwrap();
        let observed = state.capture(session(MinecraftVersion::Java1_16_1, 31, 1), 20);
        assert_eq!(*observed.legacy_codec().unwrap().value, bytes);
        let rows = case["entries"].as_array().unwrap();
        assert_eq!(
            observed.registries()["minecraft:dimension_type"]
                .value
                .len(),
            rows.len()
        );
        for entry in rows {
            let name = entry["name"].as_str().unwrap();
            let id = observed
                .find_entry("minecraft:dimension_type", name)
                .unwrap();
            assert_eq!(id.value(), entry["id"].as_i64().unwrap() as i32);
            assert_eq!(
                observed
                    .bind_entry("minecraft:dimension_type", id.value())
                    .unwrap(),
                id
            );
            assert_eq!(observed.entry_name(&id).unwrap(), name);
            let RegistryEntryId::Server(server_id) = &id else {
                panic!("dimension entry must be server owned")
            };
            let data = &observed.resolve(server_id).unwrap().data;
            // Prefixing an unnamed entry with an empty name reconstructs a
            // legacy named compound for the independent shared NBT decoder.
            let mut named = vec![10, 0, 0];
            named.extend_from_slice(&data[1..]);
            let parsed = crate::client::nbt::decode(&named, MinecraftVersion::Java1_16_1).unwrap();
            assert_eq!(
                parsed
                    .root()
                    .get("name")
                    .unwrap()
                    .as_string()
                    .unwrap()
                    .text()
                    .unwrap(),
                name
            );
            assert!(bytes.windows(data.len() - 1).any(|part| part == &data[1..]));
        }
        // Every cut and a trailing byte fail without replacing preceding state.
        for end in 0..bytes.len() {
            assert!(
                state.legacy_join(bytes[..end].to_vec(), 90).is_err(),
                "{} prefix {end}",
                case["case"]
            );
            assert_eq!(
                state
                    .capture(session(MinecraftVersion::Java1_16_1, 31, 1), 100)
                    .stamp(),
                observed.stamp()
            );
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(state.legacy_join(trailing, 90).is_err());
    }
}

#[test]
fn common_registry_resolution_preserves_received_owner_and_never_injects_dynamic_defaults() {
    let version = MinecraftVersion::Java1_21_11;
    let mut state = ReceivedRegistries::default();
    assert!(
        state
            .capture(session(version, 12, 1), 1)
            .bind_entry("minecraft:item", 1)
            .is_err()
    );
    state
        .modern_registry(
            "minecraft:enchantment".into(),
            vec![
                ServerRegistryEntry {
                    name: "minecraft:unbreaking".into(),
                    data: vec![10, 0],
                },
                ServerRegistryEntry {
                    name: "minecraft:mending".into(),
                    data: vec![10, 0],
                },
            ],
            5,
            40,
        )
        .unwrap();
    state.finish();
    let first = state.capture(session(version, 12, 1), 6);
    let builtin = first
        .find_entry("minecraft:item", "minecraft:stone")
        .unwrap();
    let enchantment = first
        .find_entry("minecraft:enchantment", "minecraft:unbreaking")
        .unwrap();
    assert_eq!(first.entry_name(&builtin).unwrap(), "minecraft:stone");
    assert_eq!(enchantment.value(), 0); // deliberately different from vanilla ordering
    assert_eq!(enchantment.server_stamp(), Some(first.stamp()));
    assert!(
        first
            .find_entry("minecraft:worldgen/configured_feature", "minecraft:oak")
            .is_err()
    );
    assert!(
        first
            .bind_entry("minecraft:worldgen/configured_feature", 0)
            .is_err()
    );
    assert!(
        first
            .find_entry("minecraft:dialog", "minecraft:custom_options")
            .is_err()
    );
    let other = state.capture(session(version, 13, 1), 7);
    assert!(other.entry_name(&enchantment).is_err());
    assert_eq!(other.entry_name(&builtin).unwrap(), "minecraft:stone");
    assert_ne!(
        enchantment,
        other
            .find_entry("minecraft:enchantment", "minecraft:unbreaking")
            .unwrap()
    );
    assert_eq!(
        first
            .find_entry("minecraft:item", "minecraft:stone")
            .unwrap(),
        other
            .find_entry("minecraft:item", "minecraft:stone")
            .unwrap()
    );
    assert!(
        state
            .capture(session(version, 12, 2), 8)
            .entry_name(&enchantment)
            .is_ok()
    );
    state.reset(20);
    state
        .modern_registry(
            "minecraft:enchantment".into(),
            vec![ServerRegistryEntry {
                name: "minecraft:unbreaking".into(),
                data: vec![10, 0],
            }],
            21,
            20,
        )
        .unwrap();
    state.finish();
    let changed = state.capture(session(version, 12, 2), 22);
    assert!(changed.entry_name(&enchantment).is_err());
    assert_ne!(
        enchantment,
        changed
            .find_entry("minecraft:enchantment", "minecraft:unbreaking")
            .unwrap()
    );
    let mut conflict = ReceivedRegistries::default();
    conflict
        .modern_registry(
            "minecraft:item".into(),
            vec![ServerRegistryEntry {
                name: "minecraft:stone".into(),
                data: vec![10, 0],
            }],
            1,
            20,
        )
        .unwrap();
    conflict.finish();
    assert!(
        conflict
            .capture(session(version, 15, 1), 2)
            .find_entry("minecraft:item", "minecraft:stone")
            .is_err()
    );
    let raw = conflict.capture(session(version, 15, 1), 2);
    let mislabeled = RegistryEntryId::Server(raw.bind("minecraft:item", 0).unwrap());
    assert!(raw.entry_name(&mislabeled).is_err());
}

#[test]
fn original_registry_catalog_sources_are_bound_without_dynamic_runtime_ids() {
    use sha2::{Digest, Sha256};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source: Json = serde_json::from_str(include_str!(
        "../../../../data/client_api/registry_catalog_source.json"
    ))
    .unwrap();
    for (path, hash) in source["generators_sha256"].as_object().unwrap() {
        assert_eq!(
            hex::encode(Sha256::digest(std::fs::read(root.join(path)).unwrap())),
            hash.as_str().unwrap(),
            "{path}"
        );
    }
    for run in source["runs"].as_array().unwrap() {
        for (path, hash) in run["files_sha256"].as_object().unwrap() {
            assert_eq!(
                hex::encode(Sha256::digest(std::fs::read(root.join(path)).unwrap())),
                hash.as_str().unwrap(),
                "{path}"
            );
        }
    }
}

#[test]
fn legacy_codec_projection_uses_last_nbt_key_and_rejects_duplicate_entry_names_atomically() {
    let native = facts(MinecraftVersion::Java1_16_1);
    let original = hex::decode(native["legacy_builtin_codec_hex"].as_str().unwrap()).unwrap();
    let mut superseded = vec![10, 0, 0, 3, 0, 9];
    superseded.extend_from_slice(b"dimension");
    superseded.extend_from_slice(&42i32.to_be_bytes());
    superseded.extend_from_slice(&original[3..]);
    let version = MinecraftVersion::Java1_16_1;
    let mut state = ReceivedRegistries::default();
    state.legacy_join(superseded, 4).unwrap();
    let before = state.capture(session(version, 42, 1), 5);
    assert_eq!(
        before
            .find_entry("minecraft:dimension_type", "minecraft:the_nether")
            .unwrap()
            .value(),
        2
    );
    let mut wrong_last = original[..original.len() - 1].to_vec();
    wrong_last.extend_from_slice(&[3, 0, 9]);
    wrong_last.extend_from_slice(b"dimension");
    wrong_last.extend_from_slice(&42i32.to_be_bytes());
    wrong_last.push(0);
    assert!(state.legacy_join(wrong_last, 6).is_err());
    let custom = native["legacy_codec_cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["case"] == "custom_names")
        .unwrap();
    let mut duplicate = hex::decode(custom["codec_hex"].as_str().unwrap()).unwrap();
    let from = b"example:dimension_1";
    let position = duplicate
        .windows(from.len())
        .position(|value| value == from)
        .unwrap();
    duplicate[position + from.len() - 1] = b'0';
    assert!(state.legacy_join(duplicate, 6).is_err());
    assert_eq!(
        state.capture(session(version, 42, 1), 7).stamp(),
        before.stamp()
    );
}
