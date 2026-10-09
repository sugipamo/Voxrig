//! Original menu outcomes and registry-owned armor enchantment predicates.
use super::*;
use crate::MinecraftVersion;
use crate::client::{
    InventoryObservation, ItemStack,
    registry::{Registry, ServerRegistryEntry},
};
use std::io::Read;

pub(crate) fn facts(version: MinecraftVersion) -> serde_json::Value {
    let bytes: &[u8] = match version {
        MinecraftVersion::Java1_16_1 => {
            include_bytes!("../../../data/client_api/armor_transfer_cases-1.16.1.json.gz")
        }
        MinecraftVersion::Java1_21_11 => {
            include_bytes!("../../../data/client_api/armor_transfer_cases-1.21.11.json.gz")
        }
    };
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut out)
        .unwrap();
    serde_json::from_slice(&out).unwrap()
}
fn integer(rest: &mut &[u8]) -> i32 {
    let mut value = 0;
    for shift in (0..35).step_by(7) {
        let byte = rest[0];
        *rest = &rest[1..];
        value |= i32::from(byte & 127) << shift;
        if byte & 128 == 0 {
            return value;
        }
    }
    panic!("bad native oracle VarInt")
}
pub(crate) fn slot(version: MinecraftVersion, wire: &str) -> SlotKnowledge {
    let bytes = hex::decode(wire).unwrap();
    let mut rest = bytes.as_slice();
    if version == MinecraftVersion::Java1_16_1 {
        if rest == [0] {
            return SlotKnowledge::Empty;
        }
        assert_eq!(rest[0], 1);
        rest = &rest[1..];
    } else {
        let count = integer(&mut rest);
        if count == 0 {
            assert!(rest.is_empty());
            return SlotKnowledge::Empty;
        }
        let definition = Registry::for_version(version)
            .item_by_native_id(integer(&mut rest))
            .unwrap();
        let patch = crate::versions::java_1_21_11::item_components::decode_patch(rest).unwrap();
        return SlotKnowledge::Item {
            item: ItemStack {
                id: definition.id,
                name: definition.name,
                count: count.try_into().unwrap(),
                data: ItemData::ModernComponents { patch },
            },
        };
    }
    let definition = Registry::for_version(version)
        .item_by_native_id(integer(&mut rest))
        .unwrap();
    let count = u32::from(rest[0]);
    rest = &rest[1..];
    SlotKnowledge::Item {
        item: ItemStack {
            id: definition.id,
            name: definition.name,
            count,
            data: if rest == [0] {
                ItemData::Default
            } else {
                ItemData::LegacyNbt {
                    bytes: rest.to_vec(),
                }
            },
        },
    }
}
// Compare field preservation independently of compound wire ordering. Float bits
// are retained here: this oracle checks a menu operation's metadata, not the
// equality of independently decoded network receipts (where NaN is unequal).
fn preserved(version: MinecraftVersion, value: &SlotKnowledge) -> serde_json::Value {
    match value {
        SlotKnowledge::Item { item } => {
            let data = match &item.data {
                ItemData::LegacyNbt { bytes } => {
                    serde_json::to_value(crate::client::nbt::decode(bytes, version).unwrap().root())
                        .unwrap()
                }
                _ => serde_json::to_value(&item.data).unwrap(),
            };
            serde_json::json!({"id": item.id, "name": item.name, "count": item.count, "data": data})
        }
        other => serde_json::to_value(other).unwrap(),
    }
}

fn initial(version: MinecraftVersion) -> PlayerObservation {
    PlayerObservation {
        using_item: None,
        entity_id: None,
        attributes: Default::default(),
        effects: Default::default(),
        air_supply: None,
        world_time: None,
        session: SessionStamp {
            version,
            connection_id: 42,
            world_generation: 1,
        },
        receive_sequence: 10,
        pending_dispatch: false,
        dimension: None,
        position: None,
        received_pose: None,
        rotation: [0., 0.],
        rotation_source: None,
        on_ground: None,
        game_mode: None,
        may_fly: None,
        health: None,
        selected_hotbar: None,
        inventory: InventoryObservation {
            slots: vec![],
            cursor: None,
            window_id: None,
            player_screen: None,
            screen_revision: None,
            player_screen_revision: None,
            local_cache: None,
        },
    }
}
fn put(out: &mut Vec<u8>, mut value: u32) {
    loop {
        let byte = (value & 127) as u8;
        value >>= 7;
        out.push(byte | if value == 0 { 0 } else { 128 });
        if value == 0 {
            return;
        }
    }
}
fn name(out: &mut Vec<u8>, value: &str) {
    put(out, value.len() as u32);
    out.extend(value.as_bytes());
}
pub(crate) fn context(
    version: MinecraftVersion,
    entries: Option<Vec<ServerRegistryEntry>>,
) -> ItemContext {
    let mut received = crate::client::registry::received::ReceivedRegistries::default();
    received.reset(1);
    install(&mut received, version, entries);
    let initial = initial(version);
    ItemContext::new(received.capture(initial.session, 10), &initial).unwrap()
}

pub(crate) fn install(
    received: &mut crate::client::registry::received::ReceivedRegistries,
    version: MinecraftVersion,
    entries: Option<Vec<ServerRegistryEntry>>,
) {
    if version == MinecraftVersion::Java1_21_11 {
        received
            .modern_registry(
                "minecraft:enchantment".into(),
                entries.unwrap_or_else(|| {
                    let facts = facts(version);
                    let mut rows = facts["enchantments"].as_array().unwrap().clone();
                    rows.sort_by_key(|r| r["native_id"].as_i64().unwrap());
                    rows.iter()
                        .enumerate()
                        .map(|(id, row)| {
                            assert_eq!(row["native_id"], id);
                            ServerRegistryEntry {
                                name: row["name"].as_str().unwrap().into(),
                                data: hex::decode(row["data_hex"].as_str().unwrap()).unwrap(),
                            }
                        })
                        .collect()
                }),
                3,
                0,
            )
            .unwrap();
        let mut data = Vec::new();
        flate2::read::GzDecoder::new(&include_bytes!("../../../data/client_api/equipment_transfer_cases-1.21.11.json.gz")[..]).read_to_end(&mut data).unwrap();
        let tags: serde_json::Value = serde_json::from_slice(&data).unwrap();
        let groups = tags["builtin_tags"].as_object().unwrap();
        let mut packet = Vec::new();
        put(&mut packet, groups.len() as u32);
        for (registry, tags) in groups {
            name(&mut packet, registry);
            let tags = tags.as_object().unwrap();
            put(&mut packet, tags.len() as u32);
            for (tag, members) in tags {
                name(&mut packet, tag);
                let members = members.as_array().unwrap();
                put(&mut packet, members.len() as u32);
                for member in members {
                    put(
                        &mut packet,
                        Registry::for_version(version)
                            .builtin_id(registry, member.as_str().unwrap())
                            .unwrap()
                            .value() as u32,
                    );
                }
            }
        }
        received.receive_tags(&packet, 4, version).unwrap();
    }
    received.finish();
}

pub(crate) fn fixture(version: MinecraftVersion, binding: bool) -> SlotKnowledge {
    let facts = facts(version);
    let label = match (version, binding) {
        (MinecraftVersion::Java1_16_1, true) => "short-one",
        (MinecraftVersion::Java1_16_1, false) => "unbreaking",
        (MinecraftVersion::Java1_21_11, true) => "minecraft:enchantments-minecraft:binding_curse-1",
        (MinecraftVersion::Java1_21_11, false) => "minecraft:enchantments-minecraft:unbreaking-1",
    };
    let case = facts["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| {
            c["request"]["case"] == label
                && c["request"]["slot"] == 5
                && c["request"]["mode"] == "survival"
        })
        .unwrap();
    slot(version, case["before"][5].as_str().unwrap())
}

#[test]
fn exact_pickup_reuses_native_armor_take_predicate_without_changing_received_data() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let context = context(version, None);
        let facts = facts(version);
        for case in facts["cases"].as_array().unwrap() {
            let source = case["request"]["slot"].as_u64().unwrap() as usize;
            if !matches!(source, 5..=8 | 45) {
                continue;
            }
            let original = slot(version, case["before"][source].as_str().unwrap());
            let mode = if case["request"]["mode"] == "creative" {
                GameMode::Creative
            } else {
                GameMode::Survival
            };
            let (after, cursor) = slot_policy::pickup_equipment(
                version,
                source,
                InventoryClickButton::Left,
                (&original, &SlotKnowledge::Empty),
                mode,
                Some(&context),
            )
            .unwrap();
            let (expected_source, expected_cursor) = if case["may_pickup"] == true {
                (SlotKnowledge::Empty, original.clone())
            } else {
                (original.clone(), SlotKnowledge::Empty)
            };
            assert_eq!(
                preserved(version, &after),
                preserved(version, &expected_source),
                "{version:?} {case}"
            );
            assert_eq!(
                preserved(version, &cursor),
                preserved(version, &expected_cursor),
                "{version:?} {case}"
            );
        }
    }
}

#[test]
fn armor_transfer_matches_original_pickup_and_all_slots_in_both_versions_modes() {
    for (version, count) in [
        (MinecraftVersion::Java1_16_1, 276),
        (MinecraftVersion::Java1_21_11, 156),
    ] {
        let facts = facts(version);
        let cases = facts["cases"].as_array().unwrap();
        assert_eq!(cases.len(), count);
        let context = context(version, None);
        for (index, case) in cases.iter().enumerate() {
            let read = |field: &str| {
                case[field]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| slot(version, v.as_str().unwrap()))
                    .collect::<Vec<_>>()
            };
            let before = read("before");
            let after = read("after");
            let source = case["request"]["slot"].as_u64().unwrap() as usize;
            let mode = if case["request"]["mode"] == "creative" {
                GameMode::Creative
            } else {
                GameMode::Survival
            };
            assert_eq!(case["actual_creative"], mode == GameMode::Creative);
            let SlotKnowledge::Item { item } = &before[source] else {
                panic!("empty native source")
            };
            let may_pickup =
                !(5..=8).contains(&source) || context.armor_may_pickup(item, mode).unwrap();
            assert_eq!(may_pickup, case["may_pickup"], "{version:?} case{index}");
            let predicted = transfer_policy::calculate_with_data(
                version,
                "minecraft:player",
                source,
                &before,
                mode,
                &context,
            )
            .unwrap();
            // Pure menu metadata preservation, including NaN bytes, is distinct
            // from independently decoded network receipt semantic equality.
            assert_eq!(predicted.slots.len(), after.len());
            for (slot, (actual, expected)) in predicted.slots.iter().zip(&after).enumerate() {
                assert_eq!(
                    preserved(version, actual),
                    preserved(version, expected),
                    "{version:?} case{index} slot{slot}"
                );
            }
            if version == MinecraftVersion::Java1_16_1 {
                assert_eq!(
                    preserved(version, &predicted.legacy_return),
                    preserved(
                        version,
                        &slot(version, case["legacy_return_hex"].as_str().unwrap())
                    )
                );
            }
        }
    }
}

#[test]
fn armor_transfer_effect_uses_received_definition_instead_of_enchantment_name() {
    let version = MinecraftVersion::Java1_21_11;
    let facts = facts(version);
    let rows = facts["enchantments"].as_array().unwrap();
    let binding = rows
        .iter()
        .find(|r| r["name"] == "minecraft:binding_curse")
        .unwrap();
    let unbreaking = rows
        .iter()
        .find(|r| r["name"] == "minecraft:unbreaking")
        .unwrap();
    let registry = Registry::for_version(version);
    let d = registry.item("minecraft:diamond_helmet").unwrap();
    let item = ItemStack {
        id: d.id,
        name: d.name,
        count: 1,
        data: ItemData::ModernComponents {
            patch: crate::client::ItemComponentPatch {
                added: vec![crate::client::ItemComponent {
                    definition: registry.item_component("minecraft:enchantments").unwrap(),
                    bytes: vec![1, 0, 0],
                }],
                removed: vec![],
            },
        },
    };
    for (name, source, allowed) in [
        ("voxrig:custom_lock", binding, false),
        ("minecraft:binding_curse", unbreaking, true),
    ] {
        let context = context(
            version,
            Some(vec![ServerRegistryEntry {
                name: name.into(),
                data: hex::decode(source["data_hex"].as_str().unwrap()).unwrap(),
            }]),
        );
        assert_eq!(
            context.armor_may_pickup(&item, GameMode::Survival).unwrap(),
            allowed
        );
        assert!(context.armor_may_pickup(&item, GameMode::Creative).unwrap());
    }
}

#[test]
fn armor_transfer_original_evidence_is_bound_to_owned_tools_and_outputs() {
    use sha2::{Digest, Sha256};
    let facts: serde_json::Value = serde_json::from_str(include_str!(
        "../../../data/client_api/armor_transfer_source.json"
    ))
    .unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for object in std::iter::once(&facts["generators_sha256"]).chain(
        facts["runs"]
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
