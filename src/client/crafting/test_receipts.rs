//! Test-only received boundaries built from original encoded primitive fixtures.
use super::*;
use crate::client::{
    GameMode, InventoryObservation, ItemData, ItemStack, SlotKnowledge,
    container::{ScreenReceipts, ScreenTitle},
    crafting::recipes::{NativeRecipeId, RecipeEntry, RecipeReceipts},
    received,
    registry::{Registry, received::ReceivedRegistries},
};
use flate2::read::GzDecoder;
use serde_json::Value;
use std::io::Read;
pub(super) fn corpus(bytes: &[u8]) -> Value {
    let mut text = String::new();
    GzDecoder::new(bytes).read_to_string(&mut text).unwrap();
    serde_json::from_str(&text).unwrap()
}
pub(super) fn value(version: MinecraftVersion, hex: &str) -> SlotKnowledge {
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect::<Vec<_>>();
    if version == MinecraftVersion::Java1_16_1 {
        let mut input = bytes.as_slice();
        let native = crate::versions::java_1_16_1::inventory::read_slot(&mut input).unwrap();
        assert!(input.is_empty());
        crate::client::legacy_slot(native.as_ref()).unwrap()
    } else {
        let mut input = bytes.as_slice();
        let count = crate::protocol::get_varint(&mut input).unwrap();
        if count == 0 {
            assert!(input.is_empty());
            return SlotKnowledge::Empty;
        }
        let id = crate::protocol::get_varint(&mut input).unwrap();
        let item = Registry::for_version(version)
            .item_by_native_id(id)
            .unwrap();
        let patch = crate::versions::java_1_21_11::item_components::decode_patch(input).unwrap();
        SlotKnowledge::Item {
            item: ItemStack {
                id: item.id,
                name: item.name,
                count: count.try_into().unwrap(),
                data: ItemData::ModernComponents { patch },
            },
        }
    }
}
pub(super) fn player(version: MinecraftVersion) -> PlayerObservation {
    PlayerObservation {
        using_item: None,
        entity_id: None,
        attributes: Default::default(),
        effects: Default::default(),
        air_supply: None,
        world_time: None,
        session: SessionStamp {
            version,
            connection_id: 7,
            world_generation: 2,
        },
        receive_sequence: 20,
        pending_dispatch: false,
        dimension: None,
        position: None,
        received_pose: None,
        rotation: [0.; 2],
        rotation_source: None,
        on_ground: None,
        game_mode: Some(GameMode::Survival),
        may_fly: None,
        health: None,
        selected_hotbar: Some(received(0, 12)),
        inventory: InventoryObservation {
            window_id: Some(0),
            player_screen: Some(PlayerScreenAccess::Received),
            slots: vec![Some(received(SlotKnowledge::Empty, 12)); 46],
            cursor: Some(received(SlotKnowledge::Empty, 12)),
            ..Default::default()
        },
    }
}
pub(super) fn screen(
    player: &PlayerObservation,
    table: bool,
    inputs: &[SlotKnowledge],
) -> ScreenObservation {
    let mut screen = ScreenObservation {
        session: player.session,
        receive_sequence: player.receive_sequence,
        active_window: player.inventory.window_id,
        player_screen: player.inventory.player_screen,
        screen: None,
        cursor: player.inventory.cursor.clone(),
    };
    if table {
        let native = native_menu(player.session.version, "minecraft:crafting").unwrap();
        let mut receipt = ScreenReceipts::open(
            player.session.version,
            3,
            native.native_id,
            ScreenTitle::Unavailable,
            11,
        );
        let mut slots = vec![Some(received(SlotKnowledge::Empty, 12)); 46];
        for (i, value) in inputs.iter().enumerate() {
            slots[i + 1] = Some(received(value.clone(), 12));
        }
        for mapping in &native.player_slots {
            let canonical = if mapping.raw_player_slot < 9 {
                36 + mapping.raw_player_slot
            } else {
                mapping.raw_player_slot
            };
            slots[mapping.screen_slot] = player.inventory.slots[canonical].clone();
        }
        receipt.full_items(slots, Some(2), 12).unwrap();
        screen.screen = Some(receipt.capture(player.session));
    }
    screen
}
pub(super) fn registries(player: &PlayerObservation) -> ServerRegistryObservation {
    let mut r = ReceivedRegistries::default();
    r.reset(10);
    r.finish();
    if player.session.version == MinecraftVersion::Java1_21_11 {
        // Actual loaded vanilla tag declarations from the original return
        // oracle. Tool component holders need these even for plain stacks.
        let facts = corpus(include_bytes!(
            "../../../data/client_api/crafting_return_cases-1.21.11.json.gz"
        ));
        let tags = facts["builtin_tags"].as_object().unwrap();
        let registry = Registry::for_version(player.session.version);
        let mut packet = Vec::new();
        fn integer(out: &mut Vec<u8>, value: usize) {
            crate::protocol::put_varint(out, value.try_into().unwrap());
        }
        fn name(out: &mut Vec<u8>, value: &str) {
            integer(out, value.len());
            out.extend_from_slice(value.as_bytes());
        }
        integer(&mut packet, tags.len());
        for (group, declarations) in tags {
            name(&mut packet, group);
            let declarations = declarations.as_object().unwrap();
            integer(&mut packet, declarations.len());
            for (tag, members) in declarations {
                name(&mut packet, tag);
                let members = members.as_array().unwrap();
                integer(&mut packet, members.len());
                for member in members {
                    integer(
                        &mut packet,
                        registry
                            .builtin_id(group, member.as_str().unwrap())
                            .unwrap()
                            .value() as usize,
                    );
                }
            }
        }
        r.receive_tags(&packet, 11, player.session.version).unwrap();
    }
    r.capture(player.session, player.receive_sequence)
}
pub(super) fn recipes(player: &PlayerObservation, display: RecipeDisplay) -> ReceivedRecipes {
    recipes_with_requirements(player, display, None, None)
}
pub(super) fn recipes_with_requirements(
    player: &PlayerObservation,
    display: RecipeDisplay,
    requirements: Option<Vec<RecipeIngredient>>,
    unlocked: Option<bool>,
) -> ReceivedRecipes {
    let mut receipts = RecipeReceipts::default();
    let entry = RecipeEntry {
        display,
        requirements,
        group: None,
        category: None,
        highlighted: None,
        notification: None,
    };
    if player.session.version == MinecraftVersion::Java1_16_1 {
        receipts.declare_legacy(
            [(NativeRecipeId::Legacy("voxrig:layout".into()), entry)].into(),
            11,
        );
        if let Some(unlocked) = unlocked {
            receipts.legacy_book(
                if unlocked {
                    ["voxrig:layout".into()].into()
                } else {
                    Default::default()
                },
                Default::default(),
                true,
                12,
            );
        }
    } else {
        receipts.add_modern(vec![(0, entry)], true, 11);
    }
    receipts
        .capture(player.session, player.receive_sequence, registries(player))
        .unwrap()
}
pub(super) fn context(
    mut player: PlayerObservation,
    table: bool,
    inputs: &[SlotKnowledge],
    display: RecipeDisplay,
) -> ReceivedCraftingContext {
    if table {
        player.inventory.window_id = Some(3);
        player.inventory.player_screen = None;
    } else {
        for (i, value) in inputs.iter().enumerate() {
            player.inventory.slots[i + 1] = Some(received(value.clone(), 12));
        }
    }
    let r = registries(&player);
    let recipes = recipes(&player, display);
    let s = screen(&player, table, inputs);
    ReceivedCraftingContext::capture(player, s, r, recipes)
        .unwrap()
        .unwrap()
}
pub(super) fn display(
    version: MinecraftVersion,
    width: usize,
    height: usize,
    shaped: bool,
    entries: usize,
) -> RecipeDisplay {
    let result = RecipeSlotDisplay::Item {
        item: Registry::for_version(version)
            .item("minecraft:stone")
            .unwrap(),
    };
    let ingredients = vec![result.clone(); entries];
    if shaped {
        RecipeDisplay::Shaped {
            width: width as i32,
            height: height as i32,
            ingredients,
            result,
            crafting_station: None,
        }
    } else {
        RecipeDisplay::Shapeless {
            ingredients,
            result,
            crafting_station: None,
        }
    }
}
