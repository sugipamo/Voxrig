use super::super::{RecipeDisplay, RecipeIngredient, RecipeSlotDisplay, test_receipts as receipts};
use super::*;
use crate::client::{GameMode, ItemData, ItemStack, received, registry::Registry};
fn value(version: MinecraftVersion, name: &str, count: u32) -> SlotKnowledge {
    let item = Registry::for_version(version).item(name).unwrap();
    SlotKnowledge::Item {
        item: ItemStack {
            id: item.id,
            name: item.name,
            count,
            data: ItemData::Default,
        },
    }
}
fn context(version: MinecraftVersion, table: bool, mode: GameMode) -> ReceivedCraftingContext {
    let mut player = receipts::player(version);
    player.game_mode = Some(mode);
    if table {
        player.inventory.window_id = Some(3);
        player.inventory.player_screen = None;
    }
    player.inventory.slots[9] = Some(received(value(version, "minecraft:oak_planks", 10), 12));
    let plank = Registry::for_version(version)
        .item("minecraft:oak_planks")
        .unwrap();
    let display = RecipeDisplay::Shaped {
        width: 1,
        height: 2,
        ingredients: vec![
            RecipeSlotDisplay::Item {
                item: plank.clone()
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
                items: vec![plank.id]
            };
            2
        ]),
        Some(true),
    );
    let screen = receipts::screen(&player, table, &[]);
    ReceivedCraftingContext::capture(
        player.clone(),
        screen,
        receipts::registries(&player),
        recipes,
    )
    .unwrap()
    .unwrap()
}
fn after(
    base: &ReceivedCraftingContext,
    inputs: u32,
    stock: u32,
    sequence: u64,
) -> ReceivedCraftingContext {
    let mut player = base.player().clone();
    player.receive_sequence = sequence;
    // Recipe catalogue is recaptured at the same actual boundary, with identity unchanged.
    let entry = &base.recipes().entries()[0];
    let recipes = receipts::recipes_with_requirements(
        &player,
        entry.display().clone(),
        entry.requirements().map(|v| v.to_vec()),
        entry.unlocked(),
    );
    let mut grid = vec![SlotKnowledge::Empty; base.grid().dimensions().iter().product()];
    let table = base.grid().dimensions() == [3, 3];
    let positions = if table { [1, 4] } else { [0, 2] };
    for i in positions {
        grid[i] = if inputs == 0 {
            SlotKnowledge::Empty
        } else {
            value(base.session().version, "minecraft:oak_planks", inputs)
        };
    }
    if !table {
        for (i, v) in grid.iter().enumerate() {
            player.inventory.slots[i + 1] = Some(received(v.clone(), sequence));
        }
    }
    player.inventory.slots[9] = Some(received(
        if stock == 0 {
            SlotKnowledge::Empty
        } else {
            value(base.session().version, "minecraft:oak_planks", stock)
        },
        sequence,
    ));
    let mut screen = receipts::screen(&player, table, &grid);
    if let Some(table) = screen.screen.as_mut() {
        // This helper represents a new actual full packet on the same opening.
        // The generic topology fixture deliberately keeps its initial ordinal.
        for slot in table.slots.iter_mut().flatten() {
            slot.source = ValueSource::Received { sequence };
        }
        table.full_contents_sequence = Some(sequence);
    }
    ReceivedCraftingContext::capture(
        player.clone(),
        screen,
        receipts::registries(&player),
        recipes,
    )
    .unwrap()
    .unwrap()
}
fn with_player(
    base: &ReceivedCraftingContext,
    player: PlayerObservation,
) -> ReceivedCraftingContext {
    let entry = &base.recipes().entries()[0];
    let recipes = receipts::recipes_with_requirements(
        &player,
        entry.display().clone(),
        entry.requirements().map(|v| v.to_vec()),
        entry.unlocked(),
    );
    ReceivedCraftingContext::capture(
        player.clone(),
        receipts::screen(&player, false, &[]),
        receipts::registries(&player),
        recipes,
    )
    .unwrap()
    .unwrap()
}
#[test]
fn placement_waits_for_inventory_conservation_not_only_fresh_matching_inputs() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        for table in [false, true] {
            for mode in [GameMode::Survival, GameMode::Creative] {
                let before = context(version, table, mode);
                let plan = before
                    .recipe_placement_plan(
                        before.recipes().entries()[0].id(),
                        RecipePlacementAmount::Next,
                    )
                    .unwrap();
                let mut record = prepare(&plan, &before, mode, 1).unwrap();
                record.send.dispatched = true;
                receive(&mut record, &after(&before, 1, 10, 21));
                assert!(!record.ready());
                assert!(record.after.is_none() && record.requires_inspection.is_none());
                receive(&mut record, &after(&before, 1, 8, 22));
                assert!(
                    record.ready(),
                    "{version:?} table={table} mode={mode:?}: {record:?}"
                );
                assert_eq!(record.after.as_ref().unwrap().receive_sequence(), 22);
                assert_eq!(
                    record
                        .inventory_after
                        .as_ref()
                        .unwrap()
                        .slot(9)
                        .unwrap()
                        .unwrap()
                        .receive_sequence(),
                    22
                );
                assert_eq!(record.plan.source_context().receive_sequence(), 20);
            }
        }
    }
}
#[test]
fn placement_before_io_rejects_equal_values_with_new_receipts_mode_and_identity_changes() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let before = context(version, false, GameMode::Survival);
        let plan = before
            .recipe_placement_plan(
                before.recipes().entries()[0].id(),
                RecipePlacementAmount::Next,
            )
            .unwrap();
        let record = prepare(&plan, &before, GameMode::Survival, 1).unwrap();
        assert!(validate_before(&record, &before).is_ok());
        assert!(validate_before(&record, &after(&before, 0, 10, 21)).is_err());
        assert!(prepare(&plan, &before, GameMode::Creative, 1).is_err());
        assert!(prepare(&plan, &before, GameMode::Survival, 0).is_err());
        let mut player = before.player().clone();
        player.pending_dispatch = true;
        let other = with_player(&before, player);
        assert!(prepare(&plan, &other, GameMode::Survival, 1).is_err());
        let mut player = before.player().clone();
        player.session.world_generation += 1;
        let other = with_player(&before, player);
        assert!(validate_before(&record, &other).is_err());
        let mut player = before.player().clone();
        player.game_mode = Some(GameMode::Creative);
        let other = with_player(&before, player);
        assert!(validate_before(&record, &other).is_err());
    }
}
#[test]
fn placement_requires_actual_changed_inputs_and_latches_first_conflict() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let before = context(version, false, GameMode::Survival);
        let plan = before
            .recipe_placement_plan(
                before.recipes().entries()[0].id(),
                RecipePlacementAmount::Maximum,
            )
            .unwrap();
        let mut record = prepare(&plan, &before, GameMode::Survival, 1).unwrap();
        record.send.dispatched = true;
        receive(&mut record, &before);
        assert!(!record.ready());
        let changed = after(&before, 5, 0, 21);
        let mut player = changed.player().clone();
        player.game_mode = Some(GameMode::Creative);
        let changed = with_player(&changed, player);
        receive(&mut record, &changed);
        let reason = record.requires_inspection.clone().unwrap();
        receive(&mut record, &after(&before, 5, 0, 22));
        assert!(!record.ready());
        assert_eq!(record.requires_inspection.as_deref(), Some(reason.as_str()));
        assert_eq!(record.stage, RecipePlacementStage::RequiresInspection);
    }
}
#[test]
fn placement_native_payload_matches_existing_original_packet_oracle() {
    for (version, bytes) in [
        (
            MinecraftVersion::Java1_16_1,
            include_bytes!("../../../../data/client_api/recipe_placement_cases-1.16.1.json.gz")
                .as_slice(),
        ),
        (
            MinecraftVersion::Java1_21_11,
            include_bytes!("../../../../data/client_api/recipe_placement_cases-1.21.11.json.gz")
                .as_slice(),
        ),
    ] {
        let cases = receipts::corpus(bytes);
        let mut checked = 0;
        for case in cases["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["domain"] == "packet")
        {
            let amount = if case["maximum"] == true {
                RecipePlacementAmount::Maximum
            } else {
                RecipePlacementAmount::Next
            };
            let packet = &case["native_packet"];
            let native = if version == MinecraftVersion::Java1_16_1 {
                recipes::NativeRecipeId::Legacy(packet["recipe"].as_str().unwrap().into())
            } else {
                recipes::NativeRecipeId::Modern(packet["display_id"].as_i64().unwrap() as i32)
            };
            let window = packet["window"].as_i64().unwrap() as i32;
            if window < 0 || (version == MinecraftVersion::Java1_16_1 && window > 127) {
                assert!(encode(version, window, &native, amount).is_err());
                continue;
            }
            assert_eq!(
                hex::encode(encode(version, window, &native, amount).unwrap()),
                packet["encoded_hex"].as_str().unwrap()
            );
            checked += 1;
        }
        assert!(checked > 0);
    }
}

#[test]
fn ghost_accepts_actual_player_zero_after_exact_submitted_close_without_rebinding_a_table() {
    use crate::client::crafting::ghost::{GhostContext, GhostReceipts};
    use crate::client::registry::received::ReceivedRegistries;
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let table = context(version, true, GameMode::Survival);
        let screen = receipts::screen(table.player(), true, &[]).screen.unwrap();
        let mut close = crate::client::container::prepare_close(
            table.player().clone(),
            screen.clone(),
            screen.id,
            GameMode::Survival,
            None,
        )
        .unwrap();
        close.sent();
        let mut p = receipts::player(version);
        p.inventory.window_id = Some(3);
        p.inventory.player_screen = Some(PlayerScreenAccess::SubmittedClose { close: close.id });
        let before = with_player(&context(version, false, GameMode::Survival), p);
        let plan = before
            .recipe_placement_plan(
                before.recipes().entries()[0].id(),
                RecipePlacementAmount::Next,
            )
            .unwrap();
        assert!(plan.can_request() && !plan.can_place());
        let mut record = prepare(&plan, &before, GameMode::Survival, 1).unwrap();
        record.send.dispatched = true;
        let mut p = before.player().clone();
        p.receive_sequence = 21;
        p.inventory.window_id = Some(0);
        p.inventory.player_screen = Some(PlayerScreenAccess::Received);
        let current = with_player(&before, p);
        let mut registries = ReceivedRegistries::default();
        registries.reset(10);
        registries.finish();
        let context = GhostContext {
            generation: before.session().world_generation,
            sequence: 21,
            active_window: Some(0),
            screen: None,
            close: Some(close),
            registries,
        };
        let ghost = match plan.recipe().native() {
            recipes::NativeRecipeId::Legacy(name) => GhostReceipts::named(
                context.clone(),
                0,
                name.clone(),
                &recipes::RecipeReceipts::default(),
            ),
            recipes::NativeRecipeId::Modern(_) => GhostReceipts::displayed(
                context.clone(),
                0,
                before.recipes().entries()[0].display().clone(),
            ),
        }
        .capture(before.session())
        .unwrap()
        .unwrap();
        let mut other = context;
        other.active_window = Some(3);
        let mut receipt = crate::client::container::ScreenReceipts::open(
            version,
            3,
            None,
            crate::client::container::ScreenTitle::Unavailable,
            21,
        );
        receipt.menu_name = Some("minecraft:crafting".into());
        other.screen = Some(receipt);
        let table_ghost =
            GhostReceipts::displayed(other, 3, before.recipes().entries()[0].display().clone())
                .capture(before.session())
                .unwrap()
                .unwrap();
        receive_with_ghost(&mut record, &current, Some(&table_ghost));
        assert!(record.ghost.is_none() && !record.ready());
        receive_with_ghost(&mut record, &current, Some(&ghost));
        assert!(record.ready() && record.requires_inspection.is_none());
        assert_eq!(
            record.ghost.as_ref().unwrap().source(),
            current.grid().source()
        );
        assert_ne!(record.plan.layout().source(), ghost.source());
        assert!(matches!(
            record.plan.layout().source(),
            CraftingSource::Player {
                access: PlayerScreenAccess::SubmittedClose { .. }
            }
        ));
    }
}
