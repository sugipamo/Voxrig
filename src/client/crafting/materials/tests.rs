use super::*;
use crate::{
    MinecraftVersion,
    client::{
        InventoryObservation, ItemData, ItemStack,
        crafting::recipes::{
            NativeRecipeId, RecipeEntry, RecipeIngredient, RecipeReceipts, RecipeSlotDisplay,
        },
        received,
        registry::{Registry, received::ReceivedRegistries},
    },
};
use flate2::read::GzDecoder;
use serde_json::Value;
use std::io::Read;
fn corpus(version: MinecraftVersion) -> Value {
    let bytes = match version {
        MinecraftVersion::Java1_16_1 => {
            include_bytes!("../../../../data/client_api/recipe_book_matching_cases-1.16.1.json.gz")
                .as_slice()
        }
        MinecraftVersion::Java1_21_11 => {
            include_bytes!("../../../../data/client_api/recipe_book_matching_cases-1.21.11.json.gz")
                .as_slice()
        }
    };
    let mut text = String::new();
    GzDecoder::new(bytes).read_to_string(&mut text).unwrap();
    serde_json::from_str(&text).unwrap()
}
fn fixture(version: MinecraftVersion, case: &Value) -> (ReceivedRecipes, ReceivedInventory) {
    fixture_options(version, case, 15, 7, None, true)
}
fn fixture_options(
    version: MinecraftVersion,
    case: &Value,
    sequence: u64,
    connection_id: u64,
    missing: Option<usize>,
    has_requirements: bool,
) -> (ReceivedRecipes, ReceivedInventory) {
    let session = SessionStamp {
        version,
        connection_id,
        world_generation: 2,
    };
    let reg = Registry::for_version(version);
    let requirements = case["ingredients"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| RecipeIngredient::Items {
            items: r
                .as_array()
                .unwrap()
                .iter()
                .map(|s| reg.item(s.as_str().unwrap()).unwrap().id)
                .collect(),
        })
        .collect();
    let entry = RecipeEntry {
        display: RecipeDisplay::Shapeless {
            ingredients: vec![],
            result: RecipeSlotDisplay::Item {
                item: reg.item("minecraft:stone").unwrap(),
            },
            crafting_station: None,
        },
        requirements: has_requirements.then_some(requirements),
        group: None,
        category: None,
        highlighted: None,
        notification: None,
    };
    let mut receipts = RecipeReceipts::default();
    if version == MinecraftVersion::Java1_16_1 {
        receipts.declare_legacy(
            [(NativeRecipeId::Legacy("voxrig:matching".into()), entry)].into(),
            11,
        );
        receipts.legacy_book(
            ["voxrig:matching".into()].into(),
            Default::default(),
            true,
            12,
        );
    } else {
        receipts.add_modern(vec![(0, entry)], true, 12);
    }
    let mut registries = ReceivedRegistries::default();
    registries.reset(10);
    registries.finish();
    let owner = registries.capture(session, sequence);
    let catalogue = receipts.capture(session, sequence, owner.clone()).unwrap();
    let mut observation = InventoryObservation::default();
    for index in 9..45 {
        observation.slots[index] = Some(received(SlotKnowledge::Empty, 12));
    }
    for (index, stock) in case["stocks"].as_array().unwrap().iter().enumerate() {
        let count = stock["count"].as_u64().unwrap() as u32;
        if count == 0 {
            continue;
        }
        let item = reg.item(stock["item"].as_str().unwrap()).unwrap();
        observation.slots[9 + index] = Some(received(
            SlotKnowledge::Item {
                item: ItemStack {
                    id: item.id,
                    name: item.name,
                    count,
                    data: ItemData::Default,
                },
            },
            12,
        ));
    }
    if let Some(index) = missing {
        observation.slots[index] = None;
    }
    (
        catalogue,
        ReceivedInventory::capture(session, sequence, &observation, owner).unwrap(),
    )
}
#[test]
fn recipe_material_assignment_and_bounded_maximum_match_original_pickers() {
    let mut cases = 0;
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        for case in corpus(version)["cases"].as_array().unwrap() {
            let (catalogue, inventory) = fixture(version, case);
            let recipe = catalogue.entries()[0].id().clone();
            let result = RecipeBookMaterials::capture(
                catalogue.clone(),
                inventory,
                &recipe,
                case["crafts"].as_u64().unwrap() as u32,
                case["bound"].as_u64().unwrap() as u32,
            )
            .unwrap();
            assert_eq!(
                result.assignment().is_some(),
                case["possible"],
                "{} {version:?}",
                case["case"]
            );
            assert_eq!(
                result.maximum(),
                case["maximum"],
                "{} {version:?}",
                case["case"]
            );
            if let Some(assignment) = result.assignment() {
                let requirements = catalogue.entry(&recipe).unwrap().requirements().unwrap();
                assert_eq!(assignment.len(), requirements.len());
                let mut consumed = BTreeMap::<i32, u32>::new();
                for (item, required) in assignment.iter().zip(requirements) {
                    assert!(required.items(&catalogue).unwrap().contains(item));
                    *consumed.entry(item.value()).or_default() += result.crafts();
                }
                for (item, amount) in consumed {
                    let supplied: i32 = result
                        .stocks()
                        .iter()
                        .filter(|(_, s)| s.item().value() == item)
                        .map(|(_, s)| s.count())
                        .sum();
                    assert!(amount <= supplied as u32);
                }
            }
            assert_eq!(result.session(), catalogue.session());
            assert_eq!(result.receive_sequence(), 15);
            assert_eq!(result.unlocked(), Some(true));
            cases += 1;
        }
    }
    assert_eq!(cases, 80);
}
#[test]
fn recipe_material_capture_rejects_unknown_stock_foreign_ids_and_mixed_boundaries() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let source = corpus(version);
        let case = &source["cases"][0];
        let (catalogue, inventory) = fixture(version, case);
        let recipe = catalogue.entries()[0].id().clone();
        let check = |catalogue: ReceivedRecipes,
                     inventory: ReceivedInventory,
                     id: &RecipeId,
                     crafts,
                     bound| {
            RecipeBookMaterials::capture(catalogue, inventory, id, crafts, bound)
        };
        assert!(check(catalogue.clone(), inventory.clone(), &recipe, 0, 64).is_err());
        assert!(check(catalogue.clone(), inventory.clone(), &recipe, 1, 0).is_err());
        assert!(check(catalogue.clone(), inventory.clone(), &recipe, u32::MAX, 64).is_err());
        let (other, _) = fixture_options(version, case, 16, 7, None, true);
        assert!(check(other, inventory.clone(), &recipe, 1, 64).is_err());
        let (other, _) = fixture_options(version, case, 15, 8, None, true);
        assert!(check(other, inventory.clone(), &recipe, 1, 64).is_err());
        let (other, _) = fixture_options(version, case, 15, 8, None, true);
        let wrong = other.entries()[0].id().clone();
        assert!(check(catalogue.clone(), inventory.clone(), &wrong, 1, 64).is_err());
        let (_, missing) = fixture_options(version, case, 15, 7, Some(44), true);
        assert!(check(catalogue.clone(), missing, &recipe, 1, 64).is_err());
        let (other, _) = fixture_options(version, case, 15, 7, None, false);
        assert!(check(other, inventory.clone(), &recipe, 1, 64).is_err());
        let mut empty = case.clone();
        empty["ingredients"] = serde_json::json!([]);
        let (other, _) = fixture(version, &empty);
        assert!(check(other, inventory.clone(), &recipe, 1, 64).is_err());
    }
}
