use super::*;
use crate::client::{
    crafting::{RecipeDisplay, RecipeIngredient, RecipeSlotDisplay, test_receipts as receipts},
    received,
};

fn item(version: MinecraftVersion, name: &str, count: u32) -> ItemStack {
    let item = Registry::for_version(version).item(name).unwrap();
    ItemStack {
        id: item.id,
        name: item.name,
        count,
        data: crate::client::ItemData::Default,
    }
}
fn context(
    version: MinecraftVersion,
    table: bool,
    counts: &[u32],
    stock: u32,
) -> ReceivedCraftingContext {
    let mut player = receipts::player(version);
    if table {
        player.inventory.window_id = Some(3);
        player.inventory.player_screen = None;
    }
    if stock > 0 {
        player.inventory.slots[9] = Some(received(
            SlotKnowledge::Item {
                item: item(version, "minecraft:oak_planks", stock),
            },
            12,
        ));
    }
    let mut inputs = vec![SlotKnowledge::Empty; if table { 9 } else { 4 }];
    for (i, count) in counts.iter().enumerate() {
        if *count > 0 {
            inputs[if table { i * 3 + 1 } else { i * 2 }] = SlotKnowledge::Item {
                item: item(version, "minecraft:oak_planks", *count),
            };
        }
    }
    if !table {
        for (i, value) in inputs.iter().enumerate() {
            player.inventory.slots[i + 1] = Some(received(value.clone(), 12));
        }
    }
    let ingredient = Registry::for_version(version)
        .item("minecraft:oak_planks")
        .unwrap();
    let display = RecipeDisplay::Shaped {
        width: 1,
        height: 2,
        ingredients: vec![
            RecipeSlotDisplay::Item {
                item: ingredient.clone()
            };
            2
        ],
        result: RecipeSlotDisplay::Item {
            item: Registry::for_version(version)
                .item("minecraft:stick")
                .unwrap(),
        },
        crafting_station: None,
    };
    let recipes = receipts::recipes_with_requirements(
        &player,
        display,
        Some(vec![
            RecipeIngredient::Items {
                items: vec![ingredient.id]
            };
            2
        ]),
        Some(true),
    );
    let screen = receipts::screen(&player, table, &inputs);
    ReceivedCraftingContext::capture(
        player.clone(),
        screen,
        receipts::registries(&player),
        recipes,
    )
    .unwrap()
    .unwrap()
}
#[test]
fn placement_plan_combines_current_grid_materials_next_maximum_and_native_capacity() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        for table in [false, true] {
            let context = context(version, table, &[2, 3], 3);
            let id = context.recipes().entries()[0].id();
            let next = context
                .recipe_placement_plan(id, RecipePlacementAmount::Next)
                .unwrap();
            let maximum = context
                .recipe_placement_plan(id, RecipePlacementAmount::Maximum)
                .unwrap();
            assert!(next.grid_matches());
            assert_eq!(next.material_maximum(), 4);
            assert_eq!(next.requested_crafts(), 3);
            assert_eq!(maximum.requested_crafts(), 4);
            assert!(next.can_place() && maximum.can_place());
            assert_eq!(
                next.source_context().receive_sequence(),
                context.receive_sequence()
            );
            assert!(
                next.grid_return()
                    .predictions()
                    .iter()
                    .all(|(_, v)| v.source == ValueSource::Predicted)
            );
            let blocked = self::context(version, table, &[2, 2], 0);
            let plan = blocked
                .recipe_placement_plan(
                    blocked.recipes().entries()[0].id(),
                    RecipePlacementAmount::Maximum,
                )
                .unwrap();
            assert_eq!(plan.material_maximum(), 2);
            assert!(!plan.matched_capacity());
            assert_eq!(plan.requested_crafts(), 0);
            assert!(!plan.can_place());
            let unmatched = self::context(version, table, &[0, 0], 7);
            let plan = unmatched
                .recipe_placement_plan(
                    unmatched.recipes().entries()[0].id(),
                    RecipePlacementAmount::Next,
                )
                .unwrap();
            assert!(!plan.grid_matches());
            assert_eq!(plan.requested_crafts(), 1);
            assert_eq!(plan.material_maximum(), 3);
            assert!(plan.can_place());
        }
    }
}
#[test]
fn placement_plan_unknown_receipts_unresolved_mutation_and_foreign_recipe_are_errors() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let initial = context(version, false, &[], 3);
        let id = initial.recipes().entries()[0].id();
        for pending in [false, true] {
            let mut player = initial.player().clone();
            if pending {
                player.pending_dispatch = true;
            } else {
                player.inventory.slots[9] = None;
            }
            let other = ReceivedCraftingContext::capture(
                player.clone(),
                receipts::screen(&player, false, &[]),
                receipts::registries(&player),
                initial.recipes().clone(),
            )
            .unwrap()
            .unwrap();
            assert!(
                other
                    .recipe_placement_plan(id, RecipePlacementAmount::Next)
                    .is_err()
            );
        }
        let mut player = initial.player().clone();
        player.session.connection_id += 1;
        let foreign = receipts::recipes(&player, receipts::display(version, 1, 1, true, 1));
        assert!(
            initial
                .recipe_placement_plan(foreign.entries()[0].id(), RecipePlacementAmount::Next)
                .is_err()
        );
    }
}

fn recapture(
    base: &ReceivedCraftingContext,
    player: crate::client::PlayerObservation,
) -> ReceivedCraftingContext {
    ReceivedCraftingContext::capture(
        player.clone(),
        receipts::screen(&player, false, &[]),
        receipts::registries(&player),
        base.recipes().clone(),
    )
    .unwrap()
    .unwrap()
}
#[test]
fn placement_returning_to_offhand_is_not_main_inventory_ingredient_stock() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let base = context(version, false, &[1], 1);
        let mut player = base.player().clone();
        player.inventory.slots[45] = Some(received(
            SlotKnowledge::Item {
                item: item(version, "minecraft:oak_planks", 1),
            },
            12,
        ));
        let context = recapture(&base, player);
        let plan = context
            .recipe_placement_plan(
                context.recipes().entries()[0].id(),
                RecipePlacementAmount::Next,
            )
            .unwrap();
        assert_eq!(plan.material_maximum(), 1);
        assert_eq!(plan.requested_crafts(), 1);
        assert!(plan.grid_return().fits());
        assert_eq!(plan.grid_return().steps()[0].player_slot(), 45);
        assert!(!plan.source_data_safe());
        assert!(!plan.can_place());
    }
}
#[test]
fn creative_placement_still_refuses_an_unsafe_full_inventory_return() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let base = context(version, false, &[1], 0);
        let mut player = base.player().clone();
        player.game_mode = Some(GameMode::Creative);
        for slot in 9..45 {
            player.inventory.slots[slot] = Some(received(
                SlotKnowledge::Item {
                    item: item(version, "minecraft:stone", 64),
                },
                12,
            ));
        }
        let context = recapture(&base, player);
        let plan = context
            .recipe_placement_plan(
                context.recipes().entries()[0].id(),
                RecipePlacementAmount::Next,
            )
            .unwrap();
        assert_eq!(plan.mode(), GameMode::Creative);
        assert!(!plan.grid_return().fits());
        assert!(!plan.can_place());
    }
}
#[test]
fn modern_placement_type_counts_do_not_mix_incompatible_source_data_within_a_cell() {
    let version = MinecraftVersion::Java1_21_11;
    let corpus = receipts::corpus(include_bytes!(
        "../../../../data/client_api/crafting_return_cases-1.21.11.json.gz"
    ));
    let raw = corpus["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["case"] == "modified-small-capacity-source-100")
        .unwrap();
    let SlotKnowledge::Item { item: modified } =
        receipts::value(version, raw["inputs_encoded"][0].as_str().unwrap())
    else {
        panic!("native stone stack")
    };
    assert_eq!(modified.properties().unwrap().max_stack_size, 16);
    for (first, second, maximum_safe) in [(3, 5, false), (4, 4, true)] {
        let mut player = receipts::player(version);
        let mut variant = modified.clone();
        variant.count = first;
        player.inventory.slots[36] = Some(received(SlotKnowledge::Item { item: variant }, 12));
        player.inventory.slots[9] = Some(received(
            SlotKnowledge::Item {
                item: item(version, "minecraft:stone", second),
            },
            12,
        ));
        let stone = Registry::for_version(version)
            .item("minecraft:stone")
            .unwrap();
        let recipes = receipts::recipes_with_requirements(
            &player,
            RecipeDisplay::Shaped {
                width: 1,
                height: 2,
                ingredients: vec![
                    RecipeSlotDisplay::Item {
                        item: stone.clone()
                    };
                    2
                ],
                result: RecipeSlotDisplay::Item {
                    item: stone.clone(),
                },
                crafting_station: None,
            },
            Some(vec![
                RecipeIngredient::Items {
                    items: vec![stone.id]
                };
                2
            ]),
            Some(true),
        );
        let context = ReceivedCraftingContext::capture(
            player.clone(),
            receipts::screen(&player, false, &[]),
            receipts::registries(&player),
            recipes,
        )
        .unwrap()
        .unwrap();
        let id = context.recipes().entries()[0].id();
        let next = context
            .recipe_placement_plan(id, RecipePlacementAmount::Next)
            .unwrap();
        let maximum = context
            .recipe_placement_plan(id, RecipePlacementAmount::Maximum)
            .unwrap();
        assert_eq!(maximum.material_maximum(), 4);
        assert!(next.can_place());
        assert_eq!(maximum.source_data_safe(), maximum_safe);
        assert_eq!(maximum.can_place(), maximum_safe);
    }
}
