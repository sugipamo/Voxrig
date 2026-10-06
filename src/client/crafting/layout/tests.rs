use super::*;
use crate::{MinecraftVersion, client::crafting::test_receipts as receipts};
fn corpus(version: MinecraftVersion) -> serde_json::Value {
    receipts::corpus(match version {
        MinecraftVersion::Java1_16_1 => {
            include_bytes!("../../../../data/client_api/recipe_placement_cases-1.16.1.json.gz")
        }
        MinecraftVersion::Java1_21_11 => {
            include_bytes!("../../../../data/client_api/recipe_placement_cases-1.21.11.json.gz")
        }
    })
}
#[test]
fn crafting_recipe_layout_coordinates_match_original_methods_and_native_ui_slots() {
    let mut cases = 0;
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        for case in corpus(version)["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["domain"] == "geometry")
        {
            let size = case["grid_width"].as_u64().unwrap() as usize;
            let recipe = if case["shaped"].as_bool().unwrap() {
                [
                    case["recipe_width"].as_u64().unwrap() as usize,
                    case["recipe_height"].as_u64().unwrap() as usize,
                ]
            } else {
                [size, size]
            };
            let entries = case["entries"].as_u64().unwrap() as usize;
            let expected = case["positions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    [
                        p["x"].as_u64().unwrap() as usize,
                        p["y"].as_u64().unwrap() as usize,
                    ]
                })
                .collect::<Vec<_>>();
            assert_eq!(
                positions([size, size], recipe, entries),
                expected,
                "{} {version:?}",
                case["case"]
            );
            if case["shaped"] == true && entries != recipe[0] * recipe[1] {
                continue;
            }
            let context = receipts::context(
                receipts::player(version),
                size == 3,
                &[],
                receipts::display(
                    version,
                    recipe[0],
                    recipe[1],
                    case["shaped"] == true,
                    entries,
                ),
            );
            let id = context.recipes().entries()[0].id();
            let layout = context.recipe_layout(id).unwrap();
            assert_eq!(layout.session(), context.session());
            assert_eq!(layout.receive_sequence(), 20);
            assert_eq!(layout.recipe(), id);
            assert_eq!(layout.grid_dimensions(), [size, size]);
            assert_eq!(layout.recipe_dimensions(), recipe);
            assert_eq!(layout.cells().len(), entries);
            for ((cell, position), native) in layout
                .cells()
                .iter()
                .zip(expected)
                .zip(case["positions"].as_array().unwrap())
            {
                assert_eq!(cell.coordinate(), position);
                assert_eq!(cell.display_index(), native["entry"]);
                let slot = cell.input_source().1;
                if version == MinecraftVersion::Java1_16_1 {
                    assert_eq!(slot, native["native_slot"]);
                } else {
                    assert_eq!(
                        usize::from(slot),
                        native["grid_index"].as_u64().unwrap() as usize + 1
                    );
                }
                assert_eq!(
                    cell.input_source(),
                    context
                        .grid()
                        .input_source(position[0], position[1])
                        .unwrap()
                );
            }
            cases += 1;
        }
    }
    assert_eq!(cases, 52);
}
#[test]
fn crafting_recipe_layout_refuses_invalid_dimensions_counts_and_foreign_entries() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        for display in [
            receipts::display(version, 0, 1, true, 0),
            receipts::display(version, 3, 3, true, 9),
            receipts::display(version, 2, 2, true, 1),
            receipts::display(version, 2, 2, false, 5),
        ] {
            let context = receipts::context(receipts::player(version), false, &[], display);
            assert!(
                context
                    .recipe_layout(context.recipes().entries()[0].id())
                    .is_err()
            );
        }
        let context = receipts::context(
            receipts::player(version),
            false,
            &[],
            receipts::display(version, 1, 2, true, 2),
        );
        let mut foreign = receipts::player(version);
        foreign.session.connection_id += 1;
        let other = receipts::context(
            foreign,
            false,
            &[],
            receipts::display(version, 1, 2, true, 2),
        );
        assert!(
            context
                .recipe_layout(other.recipes().entries()[0].id())
                .is_err()
        );
    }
}
