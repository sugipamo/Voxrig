use super::*;
use crate::client::{ItemData, ItemStack, registry::Registry};
use serde_json::Value as Json;
use std::{collections::BTreeMap, io::Read};

fn decode(bytes: &[u8]) -> LegacyItem {
    let (&present, rest) = bytes.split_first().unwrap();
    if present == 0 {
        assert!(rest.is_empty());
        return LegacyItem::empty();
    }
    let mut remaining = rest;
    let id = crate::protocol::get_varint(&mut remaining).unwrap();
    let count = remaining[0] as i8;
    let tag = &remaining[1..];
    if id == 0 || count <= 0 {
        return LegacyItem::empty();
    }
    let definition = Registry::for_version(MinecraftVersion::Java1_16_1)
        .item_by_native_id(id)
        .unwrap();
    LegacyItem::stack(&ItemStack {
        id: definition.id,
        name: definition.name,
        count: count as u32,
        data: if tag == [0] {
            ItemData::Default
        } else {
            ItemData::LegacyNbt {
                bytes: tag.to_vec(),
            }
        },
    })
    .unwrap()
}

#[test]
fn original_legacy_item_groups_and_independent_decodes_match_constructor_keys() {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(
        &include_bytes!("../../../data/client_api/item_semantics-1.16.1.json.gz")[..],
    )
    .read_to_end(&mut bytes)
    .unwrap();
    let data: Json = serde_json::from_slice(&bytes).unwrap();
    let rows = data["items"].as_array().unwrap();
    let mut keys = Vec::new();
    let mut groups = BTreeMap::new();
    for (index, row) in rows.iter().enumerate() {
        let key = decode(&hex::decode(row["input_hex"].as_str().unwrap()).unwrap());
        let canonical = decode(&hex::decode(row["canonical_hex"].as_str().unwrap()).unwrap());
        assert_eq!(
            key.matches(&canonical, false),
            row["independent_decode_matches"].as_bool().unwrap(),
            "legacy independently decoded canonical {index} {}",
            row["case"]
        );
        if let Some(previous) = groups.insert(row["group"].as_u64().unwrap(), key.clone()) {
            assert!(
                key.matches(&previous, false),
                "legacy same native group {index} {}",
                row["case"]
            );
        }
        keys.push(key);
    }
    for a in 0..keys.len() {
        for b in a..keys.len() {
            assert_eq!(
                keys[a].matches(&keys[b], false),
                rows[a]["group"] == rows[b]["group"],
                "legacy native group pair {a} {b}"
            );
        }
    }
    assert_eq!(rows.len(), 4805);
    eprintln!("legacy item keys:4805 inputs,{} groups", groups.len());
}
