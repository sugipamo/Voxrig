use super::*;
use crate::{
    MinecraftVersion,
    client::{crafting::test_receipts as receipts, received},
};
fn corpus(version: MinecraftVersion) -> serde_json::Value {
    receipts::corpus(match version {
        MinecraftVersion::Java1_16_1 => {
            include_bytes!("../../../../data/client_api/crafting_return_cases-1.16.1.json.gz")
        }
        MinecraftVersion::Java1_21_11 => {
            include_bytes!("../../../../data/client_api/crafting_return_cases-1.21.11.json.gz")
        }
    })
}
fn canonical(raw: usize) -> usize {
    if raw < 9 {
        36 + raw
    } else if raw == 40 {
        45
    } else {
        raw
    }
}
#[test]
fn crafting_grid_return_predictions_match_original_inventory_resource_methods() {
    let mut cases = 0;
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        for case in corpus(version)["cases"].as_array().unwrap() {
            let mut player = receipts::player(version);
            player.selected_hotbar = Some(received(case["selected"].as_u64().unwrap() as u8, 12));
            let initial = case["initial_encoded"].as_array().unwrap();
            for (index, item) in initial.iter().enumerate() {
                player.inventory.slots[canonical(if index == 36 { 40 } else { index })] = Some(
                    received(receipts::value(version, item.as_str().unwrap()), 12),
                );
            }
            let inputs = case["inputs_encoded"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| receipts::value(version, s.as_str().unwrap()))
                .collect::<Vec<_>>();
            for table in [false, true] {
                if !table && inputs.len() > 4 {
                    continue;
                }
                let context = receipts::context(
                    player.clone(),
                    table,
                    &inputs,
                    receipts::display(version, 1, 1, true, 1),
                );
                let result = context.grid_return_plan();
                let plan = result.unwrap();
                assert_eq!(plan.session(), context.session());
                assert_eq!(plan.receive_sequence(), 20);
                assert_eq!(
                    plan.registry_owner(),
                    context.inventory().registry_state().stamp()
                );
                assert_eq!(plan.source(), context.grid().source());
                let width = if table { 3 } else { 2 };
                let expected = case["moves"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|m| {
                        (
                            m["input"].as_u64().unwrap() as usize,
                            canonical(m["destination"].as_u64().unwrap() as usize),
                            m["amount"].as_u64().unwrap() as u32,
                        )
                    })
                    .collect::<Vec<_>>();
                let actual = plan
                    .steps()
                    .iter()
                    .map(|s| {
                        (
                            s.input()[1] * width + s.input()[0],
                            s.player_slot(),
                            s.amount(),
                        )
                    })
                    .collect::<Vec<_>>();
                assert_eq!(actual, expected, "{} {version:?}", case["case"]);
                let expected_stranded = case["stranded"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| {
                        (
                            v["input"].as_u64().unwrap() as usize,
                            canonical(v["destination"].as_u64().unwrap() as usize),
                            v["amount"].as_u64().unwrap() as u32,
                        )
                    })
                    .collect::<Vec<_>>();
                let stranded = plan
                    .unreturned_splits()
                    .iter()
                    .map(|v| {
                        (
                            v.input()[1] * width + v.input()[0],
                            v.player_slot(),
                            v.amount(),
                        )
                    })
                    .collect::<Vec<_>>();
                assert_eq!(stranded, expected_stranded, "{} {version:?}", case["case"]);
                if version == MinecraftVersion::Java1_21_11 && plan.fits() {
                    assert_eq!(
                        case["original_successful_return_encoded"],
                        case["after_encoded"]
                    );
                }

                for (index, encoded) in case["after_encoded"].as_array().unwrap().iter().enumerate()
                {
                    let index = canonical(if index == 36 { 40 } else { index });
                    let predicted = &plan
                        .predictions()
                        .iter()
                        .find(|(i, _)| *i == index)
                        .unwrap()
                        .1;
                    assert_eq!(predicted.source, ValueSource::Predicted);
                    let expected = receipts::value(version, encoded.as_str().unwrap());
                    let semantic = ItemContext::new(
                        context.inventory().registry_state().clone(),
                        context.player(),
                    )
                    .unwrap();
                    assert!(
                        semantic
                            .equivalent_values(&predicted.value, &expected, false)
                            .unwrap(),
                        "{} {version:?} {index}",
                        case["case"]
                    );
                }
                let remaining = case["remaining_encoded"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                    .filter_map(|(index, s)| {
                        let value = receipts::value(version, s.as_str().unwrap());
                        if let SlotKnowledge::Item { item } = value {
                            Some(([index % width, index / width], item))
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>();
                assert_eq!(plan.remaining(), remaining, "{} {version:?}", case["case"]);
                assert_eq!(plan.fits(), remaining.is_empty());
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 126);
}
#[test]
fn crafting_grid_return_unknown_destinations_or_inputs_never_become_capacity() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        for index in [9, 36, 45] {
            let mut player = receipts::player(version);
            player.inventory.slots[index] = None;
            let context = receipts::context(
                player,
                false,
                &[],
                receipts::display(version, 1, 1, true, 1),
            );
            assert!(context.grid_return_plan().is_err());
        }
        for index in [1, 4] {
            let mut player = receipts::player(version);
            player.inventory.slots[index] = None;
            assert!(
                receipts::context(
                    player,
                    false,
                    &[],
                    receipts::display(version, 1, 1, true, 1)
                )
                .grid_return_plan()
                .is_err()
            );
        }
        let mut player = receipts::player(version);
        player.selected_hotbar = Some(ObservedValue {
            value: 2,
            source: ValueSource::Submitted,
        });
        let plan = receipts::context(
            player.clone(),
            false,
            &[],
            receipts::display(version, 1, 1, true, 1),
        )
        .grid_return_plan()
        .unwrap();
        assert_eq!(
            plan.selected_hotbar(),
            player.selected_hotbar.as_ref().unwrap()
        );
        assert!(plan.fits());
        player.pending_dispatch = true;
        assert!(
            receipts::context(
                player,
                false,
                &[],
                receipts::display(version, 1, 1, true, 1)
            )
            .grid_return_plan()
            .is_err()
        );
        for source in [
            ValueSource::Predicted,
            ValueSource::LocalCache,
            ValueSource::Received { sequence: 9 },
            ValueSource::Received { sequence: 21 },
        ] {
            let mut player = receipts::player(version);
            player.selected_hotbar = Some(ObservedValue { value: 0, source });
            assert!(
                receipts::context(
                    player,
                    false,
                    &[],
                    receipts::display(version, 1, 1, true, 1)
                )
                .grid_return_plan()
                .is_err()
            );
        }
        let mut player = receipts::player(version);
        player.selected_hotbar = None;
        assert!(
            receipts::context(
                player,
                false,
                &[],
                receipts::display(version, 1, 1, true, 1)
            )
            .grid_return_plan()
            .is_err()
        );
    }
}
