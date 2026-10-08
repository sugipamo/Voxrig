use super::*;
use crate::client::{
    InventoryObservation, ItemData, ItemStack, ReceivedInventory, SlotKnowledge, received,
    registry::{Registry, received::ReceivedRegistries},
};
use flate2::read::GzDecoder;
use serde_json::Value;
use std::io::Read;
fn corpus(version: MinecraftVersion) -> Value {
    let bytes = match version {
        MinecraftVersion::Java1_16_1 => {
            include_bytes!("../../../../data/client_api/recipe_book_stock_cases-1.16.1.json.gz")
                .as_slice()
        }
        MinecraftVersion::Java1_21_11 => {
            include_bytes!("../../../../data/client_api/recipe_book_stock_cases-1.21.11.json.gz")
                .as_slice()
        }
    };
    let mut text = String::new();
    GzDecoder::new(bytes).read_to_string(&mut text).unwrap();
    serde_json::from_str(&text).unwrap()
}
fn bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}
fn decode(version: MinecraftVersion, encoded: &[u8]) -> ItemStack {
    match version {
        MinecraftVersion::Java1_16_1 => {
            let mut r = encoded;
            let native = crate::versions::java_1_16_1::inventory::read_slot(&mut r).unwrap();
            assert!(r.is_empty());
            let SlotKnowledge::Item { item } = crate::client::legacy_slot(native.as_ref()).unwrap()
            else {
                panic!("item");
            };
            item
        }
        MinecraftVersion::Java1_21_11 => {
            let mut r = encoded;
            let count = crate::protocol::get_varint(&mut r).unwrap();
            let id = crate::protocol::get_varint(&mut r).unwrap();
            let item = Registry::for_version(version)
                .item_by_native_id(id)
                .unwrap();
            let patch = crate::versions::java_1_21_11::item_components::decode_patch(r).unwrap();
            ItemStack {
                id: item.id,
                name: item.name,
                count: count.try_into().unwrap(),
                data: ItemData::ModernComponents { patch },
            }
        }
    }
}
fn inventory(item: ItemStack, connection: u64) -> ReceivedInventory {
    let session = SessionStamp {
        version: item.id.version(),
        connection_id: connection,
        world_generation: 2,
    };
    let mut observation = InventoryObservation::default();
    observation.slots[9] = Some(received(SlotKnowledge::Item { item }, 12));
    let mut registries = ReceivedRegistries::default();
    registries.reset(10);
    registries.finish();
    ReceivedInventory::capture(session, 15, &observation, registries.capture(session, 15)).unwrap()
}
#[test]
fn recipe_book_stock_matches_original_getters_and_accounting_for_both_versions() {
    let mut count = 0;
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        for case in corpus(version)["cases"].as_array().unwrap() {
            let inv = inventory(
                decode(version, &bytes(case["encoded_item_hex"].as_str().unwrap())),
                7,
            );
            let stock = inv
                .slot(9)
                .unwrap()
                .unwrap()
                .item()
                .unwrap()
                .recipe_book_stock()
                .unwrap();
            assert_eq!(stock.damaged(), case["damaged"], "{}", case["case"]);
            assert_eq!(stock.enchanted(), case["enchanted"], "{}", case["case"]);
            assert_eq!(
                stock.custom_named(),
                case["custom_named"],
                "{}",
                case["case"]
            );
            assert_eq!(stock.count(), case["accounted_count"], "{}", case["case"]);
            assert_eq!(
                stock.eligible(),
                !stock.damaged() && !stock.enchanted() && !stock.custom_named()
            );
            assert_eq!(
                stock.item(),
                inv.slot(9).unwrap().unwrap().item().unwrap().stack().id
            );
            assert_eq!(stock.session(), inv.session());
            assert_eq!(stock.registry_owner(), inv.registry_state().stamp());
            assert_eq!(stock.receive_sequence(), 12);
            count += 1;
        }
    }
    assert_eq!(count, 72);
}
#[test]
fn recipe_book_stock_preserves_signed_capacity_and_distinguishes_filter_from_membership() {
    let version = MinecraftVersion::Java1_21_11;
    let corpus = corpus(version);
    for (name, amount) in [
        ("stream-capacity--5", -5),
        ("stream-capacity-0", 0),
        ("ench-zero", 0),
        ("stored-ench", 64),
        ("glint-only", 64),
        ("item-name-only", 64),
    ] {
        let case = corpus["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["case"] == name)
            .unwrap();
        let inv = inventory(
            decode(version, &bytes(case["encoded_item_hex"].as_str().unwrap())),
            7,
        );
        let stock = inv
            .slot(9)
            .unwrap()
            .unwrap()
            .item()
            .unwrap()
            .recipe_book_stock()
            .unwrap();
        assert_eq!(stock.count(), amount);
        assert_eq!(stock.eligible(), name != "ench-zero");
    }
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let reg = Registry::for_version(version);
        let inv = inventory(
            ItemStack {
                id: reg.item("minecraft:stone").unwrap().id,
                name: "minecraft:stone".into(),
                count: 1,
                data: ItemData::Default,
            },
            7,
        );
        assert_eq!(
            inv.slot(9)
                .unwrap()
                .unwrap()
                .item()
                .unwrap()
                .recipe_book_stock()
                .unwrap()
                .count(),
            1
        );
        assert!(inv.slot(10).unwrap().is_none());
    }
}
