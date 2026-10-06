use super::*;
use crate::client::{SessionStamp, nbt::NbtValue};
use flate2::read::GzDecoder;
use serde_json::{Value, json};
use std::io::Read;
fn oracle() -> Value {
    let mut text = String::new();
    GzDecoder::new(
        include_bytes!("../../../../../data/client_api/recipe_display_cases-1.21.11.json.gz")
            .as_slice(),
    )
    .read_to_string(&mut text)
    .unwrap();
    serde_json::from_str(&text).unwrap()
}
fn bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}
fn state() -> State {
    use crate::client::registry::ServerRegistryEntry;
    let mut state = State {
        sequence: 12,
        ..Default::default()
    };
    // Explicit test-only configuration from the original registry oracle. Runtime
    // code never inserts bundled dynamic entries into a received catalogue.
    let registry: Value = serde_json::from_str(include_str!(
        "../../../../../data/client_api/registry_catalog_cases-1.21.11.json"
    ))
    .unwrap();
    let trim = registry["registries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "minecraft:trim_pattern")
        .unwrap();
    let entries = trim["entries"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, e)| {
            assert_eq!(e["id"], i);
            ServerRegistryEntry {
                name: e["name"].as_str().unwrap().into(),
                data: vec![10, 0],
            }
        })
        .collect();
    state
        .registries
        .modern_registry("minecraft:trim_pattern".into(), entries, 10, 2)
        .unwrap();
    state.registries.finish();
    state
}
fn snapshot(state: &State) -> crate::client::ReceivedRecipes {
    let session = SessionStamp {
        version: MinecraftVersion::Java1_21_11,
        connection_id: 7,
        world_generation: 3,
    };
    state
        .recipes
        .capture(
            session,
            state.sequence,
            state.registries.capture(session, state.sequence),
        )
        .unwrap()
}
fn slot_json(display: &RecipeSlotDisplay) -> Value {
    match display {
        RecipeSlotDisplay::Empty => json!({"type":"minecraft:empty"}),
        RecipeSlotDisplay::AnyFuel => json!({"type":"minecraft:any_fuel"}),
        RecipeSlotDisplay::Item { item } => json!({"type":"minecraft:item","item":item.name}),
        RecipeSlotDisplay::Stack { item } => {
            json!({"type":"minecraft:item_stack","item":{"id":item.name,"count":item.count}})
        }
        RecipeSlotDisplay::Tag { name } => json!({"type":"minecraft:tag","tag":name}),
        RecipeSlotDisplay::Composite { contents } => {
            json!({"type":"minecraft:composite","contents":contents.iter().map(slot_json).collect::<Vec<_>>()})
        }
        RecipeSlotDisplay::WithRemainder { input, remainder } => {
            json!({"type":"minecraft:with_remainder","input":slot_json(input),"remainder":slot_json(remainder)})
        }
        RecipeSlotDisplay::TrimPreview {
            base,
            material,
            pattern,
        } => {
            let pattern = if let Some(def) = pattern.inline_definition() {
                let NbtValue::String(text) = def.description.as_ref() else {
                    panic!("fixture description");
                };
                json!({"asset_id":def.asset,"description":text.text().unwrap(),"decal":def.decal})
            } else {
                // This original fixture's sentry pattern has ID 8. Runtime binding
                // is independently checked against received registry data below.
                assert_eq!(pattern.raw_reference, Some(8));
                json!("minecraft:sentry")
            };
            json!({"type":"minecraft:smithing_trim","base":slot_json(base),"material":slot_json(material),"pattern":pattern})
        }
    }
}
fn display_json(display: &RecipeDisplay) -> Value {
    match display {
        RecipeDisplay::Shapeless {
            ingredients,
            result,
            crafting_station,
        } => {
            json!({"type":"minecraft:crafting_shapeless","ingredients":ingredients.iter().map(slot_json).collect::<Vec<_>>(),"result":slot_json(result),"crafting_station":slot_json(crafting_station.as_ref().unwrap())})
        }
        RecipeDisplay::Shaped {
            width,
            height,
            ingredients,
            result,
            crafting_station,
        } => {
            json!({"type":"minecraft:crafting_shaped","width":width,"height":height,"ingredients":ingredients.iter().map(slot_json).collect::<Vec<_>>(),"result":slot_json(result),"crafting_station":slot_json(crafting_station.as_ref().unwrap())})
        }
        RecipeDisplay::Furnace {
            ingredient,
            fuel,
            result,
            crafting_station,
            duration,
            experience_bits,
            ..
        } => {
            json!({"type":"minecraft:furnace","ingredient":slot_json(ingredient),"fuel":slot_json(fuel.as_ref().unwrap()),"result":slot_json(result),"crafting_station":slot_json(crafting_station.as_ref().unwrap()),"duration":duration,"experience":f32::from_bits(*experience_bits).to_string().parse::<f64>().unwrap()})
        }
        RecipeDisplay::Stonecutter {
            input,
            result,
            crafting_station,
        } => {
            json!({"type":"minecraft:stonecutter","input":slot_json(input),"result":slot_json(result),"crafting_station":slot_json(crafting_station.as_ref().unwrap())})
        }
        RecipeDisplay::Smithing {
            template,
            base,
            addition,
            result,
            crafting_station,
        } => {
            json!({"type":"minecraft:smithing","template":slot_json(template.as_ref().unwrap()),"base":slot_json(base),"addition":slot_json(addition),"result":slot_json(result),"crafting_station":slot_json(crafting_station.as_ref().unwrap())})
        }
        RecipeDisplay::Special { .. } => panic!("not a modern display"),
    }
}
#[test]
fn recipe_display_original_codecs_preserve_every_accepted_field_and_packet() {
    let oracle = oracle();
    let mut accepted = 0;
    for case in oracle["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["accepted"] == true)
    {
        let bytes = bytes(case["encoded_hex"].as_str().unwrap());
        let mut r = Reader::new(&bytes);
        let mut budget = MAX_DISPLAY_NODES;
        let value = if case["domain"] == "slot" {
            slot_json(&slot(&mut r, 0, &mut budget).unwrap())
        } else {
            display_json(&display(&mut r, &mut budget).unwrap())
        };
        r.end().unwrap();
        if case["id"] == "furnace" {
            let expected = (case["native_json"]["experience"].as_f64().unwrap() as f32).to_bits();
            assert_eq!(
                u32::from_be_bytes(bytes[bytes.len() - 4..].try_into().unwrap()),
                expected
            );
        }
        assert_eq!(value, case["native_json"], "{}", case["id"]);
        accepted += 1;
        for end in 0..bytes.len() {
            let mut r = Reader::new(&bytes[..end]);
            let mut budget = MAX_DISPLAY_NODES;
            let valid = if case["domain"] == "slot" {
                slot(&mut r, 0, &mut budget).is_ok()
            } else {
                display(&mut r, &mut budget).is_ok()
            };
            assert!(!valid, "{} truncated at {end}", case["id"]);
        }
        if let Some(hex) = case["add_packet_hex"].as_str() {
            let packet = super::tests::bytes(hex);
            let mut state = state();
            receive(&mut state, ids::play_clientbound::RECIPE_BOOK_ADD, &packet).unwrap();
            let captured = snapshot(&state);
            assert!(captured.book_initialized());
            assert_eq!(captured.entries().len(), 1);
            let entry = &captured.entries()[0];
            assert_eq!(display_json(entry.display()), case["native_json"]);
            assert_eq!(entry.group(), Some("7"));
            assert_eq!(entry.category(), Some("minecraft:crafting_building_blocks"));
            assert_eq!(entry.unlocked(), Some(true));
            assert_eq!(entry.highlighted(), Some(false));
            assert_eq!(entry.notification(), Some(true));
            assert_eq!(entry.id().receive_sequence(), 12);
            let requirements = case["requirements"].as_array();
            assert_eq!(
                entry.requirements().map(<[_]>::len),
                requirements.map(Vec::len)
            );
            if let Some(requirements) = requirements {
                for (actual, expected) in entry.requirements().unwrap().iter().zip(requirements) {
                    match actual {
                        RecipeIngredient::Tag { name } => {
                            assert_eq!(*expected, json!(format!("#{name}")))
                        }
                        RecipeIngredient::Items { items } => {
                            assert_eq!(items.len(), 1);
                            assert_eq!(
                                registry().item_definition(items[0]).unwrap().name,
                                expected.as_str().unwrap()
                            );
                        }
                    }
                }
            }
        }
    }
    assert_eq!(accepted, 24);
    for (key, name) in [
        ("slot_types", "minecraft:slot_display"),
        ("recipe_types", "minecraft:recipe_display"),
        ("categories", "minecraft:recipe_book_category"),
    ] {
        for entry in oracle[key].as_array().unwrap() {
            let id = registry()
                .builtin_id_by_native_id(name, entry["id"].as_i64().unwrap() as i32)
                .unwrap();
            assert_eq!(registry().builtin_name(&id).unwrap(), entry["name"]);
        }
    }
}
#[test]
fn recipe_book_native_packet_boundaries_are_atomic_and_reused_ids_are_new_entries() {
    let oracle = oracle();
    let case = oracle["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "shapeless")
        .unwrap();
    let packet = bytes(case["add_packet_hex"].as_str().unwrap());
    let mut state = state();
    receive(&mut state, ids::play_clientbound::RECIPE_BOOK_ADD, &packet).unwrap();
    let before = snapshot(&state);
    let id = before.entries()[0].id().clone();
    for end in 0..packet.len() {
        assert!(
            receive(
                &mut state,
                ids::play_clientbound::RECIPE_BOOK_ADD,
                &packet[..end]
            )
            .is_err()
        );
        assert_eq!(
            serde_json::to_value(snapshot(&state)).unwrap(),
            serde_json::to_value(&before).unwrap()
        );
    }
    let mut trailing = packet.clone();
    trailing.push(0);
    assert!(
        receive(
            &mut state,
            ids::play_clientbound::RECIPE_BOOK_ADD,
            &trailing
        )
        .is_err()
    );
    let mut duplicate = vec![2];
    duplicate.extend_from_slice(&packet[1..packet.len() - 1]);
    duplicate.extend_from_slice(&packet[1..packet.len() - 1]);
    duplicate.push(1);
    assert!(
        receive(
            &mut state,
            ids::play_clientbound::RECIPE_BOOK_ADD,
            &duplicate
        )
        .is_err()
    );
    assert_eq!(snapshot(&state).entries()[0].id(), &id);
    state.sequence = 13;
    receive(&mut state, ids::play_clientbound::RECIPE_BOOK_ADD, &packet).unwrap();
    assert!(snapshot(&state).entry(&id).is_err());
    assert!(before.entry(&id).is_ok());
    state.sequence = 14;
    receive(
        &mut state,
        ids::play_clientbound::RECIPE_BOOK_REMOVE,
        &[1, 20],
    )
    .unwrap();
    assert!(snapshot(&state).entries().is_empty());
    assert!(snapshot(&state).book_initialized());
    assert_eq!(snapshot(&state).book_sequence(), Some(14));
    let mut empty = State {
        sequence: 12,
        ..Default::default()
    };
    receive(
        &mut empty,
        ids::play_clientbound::RECIPE_BOOK_SETTINGS,
        &[0; 8],
    )
    .unwrap();
    assert!(!snapshot(&empty).book_initialized());
    assert!(snapshot(&empty).book_sequence().is_none());
    receive(&mut empty, ids::play_clientbound::RECIPE_BOOK_ADD, &[0, 1]).unwrap();
    assert!(snapshot(&empty).book_initialized());
    assert!(snapshot(&empty).entries().is_empty());
}
#[test]
fn recipe_display_malformed_unknown_and_bounded_inputs_fail_without_allocating_dimensions() {
    for payload in [
        vec![8],
        vec![2, 255, 255, 255, 255, 15],
        vec![3, 0],
        vec![4, 1, b':'],
        vec![7, 255, 255, 255, 255, 15],
    ] {
        assert!(
            slot(
                &mut Reader::new(&payload),
                0,
                &mut MAX_DISPLAY_NODES.clone()
            )
            .is_err()
        );
    }
    let mut nested = [7, 1].repeat(MAX_DEPTH);
    nested.push(0);
    assert!(slot(&mut Reader::new(&nested), 0, &mut MAX_DISPLAY_NODES.clone()).is_err());
    assert!(slot(&mut Reader::new(&[0]), 0, &mut 0).is_err());
    assert!(
        display(
            &mut Reader::new(&[1, 2, 2, 1]),
            &mut MAX_DISPLAY_NODES.clone()
        )
        .is_err()
    );
    assert!(display(&mut Reader::new(&[5]), &mut MAX_DISPLAY_NODES.clone()).is_err());
    // Native ctor-accepted zero/negative/large dimensions are display facts,
    // not proof of a usable 2x2/3x3 execution plan.
    let oracle = oracle();
    for name in ["shaped-0-0", "shaped--1--1", "shaped-4-4"] {
        assert!(
            oracle["cases"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["id"] == name && c["accepted"] == true)
        );
    }
}
