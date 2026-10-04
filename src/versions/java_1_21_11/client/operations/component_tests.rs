use super::*;
use crate::client::{ItemData, SlotKnowledge, ValueSource};
use serde_json::Value;

pub(super) fn corpus() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../data/client_api/item_component_cases-1.21.11.json"
    ))
    .unwrap()
}
pub(super) fn baseline() -> State {
    let mut state = State {
        sequence: 9,
        ..State::default()
    };
    let mut player_full = vec![0, 0, 46];
    player_full.extend([0; 47]);
    receive(
        &mut state,
        ids::play_clientbound::WINDOW_ITEMS,
        &player_full,
    )
    .unwrap();
    state
}
fn opened(mut state: State) -> State {
    state.sequence = 10;
    receive(
        &mut state,
        ids::play_clientbound::OPEN_WINDOW,
        &[3, 2, 8, 0, 1, b'X'],
    )
    .unwrap();
    state
}
#[test]
fn original_item_packets_preserve_components_following_slots_cursor_and_received_ordinals() {
    let corpus = corpus();
    assert_eq!(corpus["stacks"].as_array().unwrap().len(), 95);
    let mut applied = 0;
    for sample in corpus["stacks"].as_array().unwrap() {
        let bytes = hex::decode(sample["stack_hex"].as_str().unwrap()).unwrap();
        let mut reader = Reader::new(&bytes);
        let expected = slot(&mut reader).unwrap();
        if sample["name"] == "minecraft:bundle_contents" {
            assert!(expected.is_none());
            continue;
        }
        reader.end().unwrap();
        let expected = expected.unwrap();
        let expected_common = common_slot(&expected).unwrap();
        let SlotKnowledge::Item { item } = &expected_common else {
            panic!("original item fixture")
        };
        assert_eq!(item.name, "minecraft:stone");
        assert_eq!(item.count, 3);
        let patch_bytes = hex::decode(sample["actual_patch_hex"].as_str().unwrap()).unwrap();
        let mut patch_reader = Reader::new(&patch_bytes);
        let patch = super::super::super::item_components::read_patch(&mut patch_reader)
            .unwrap()
            .unwrap();
        patch_reader.end().unwrap();
        assert_eq!(
            item.data,
            if patch.added.is_empty() && patch.removed.is_empty() {
                ItemData::Default
            } else {
                ItemData::ModernComponents { patch }
            }
        );
        for packet in sample["packets"].as_array().unwrap() {
            let label = packet["label"].as_str().unwrap();
            let (id, storage) = match label {
                "storage_slot" => (ids::play_clientbound::SET_SLOT, true),
                "cursor" => (ids::play_clientbound::SET_CURSOR_ITEM, false),
                "raw_player_slot" => (ids::play_clientbound::SET_PLAYER_INVENTORY, false),
                "storage_full" => (ids::play_clientbound::WINDOW_ITEMS, true),
                "player_full" => (ids::play_clientbound::WINDOW_ITEMS, false),
                _ => panic!("unknown original packet fixture"),
            };
            let mut state = if storage {
                opened(baseline())
            } else {
                baseline()
            };
            state.sequence = 13;
            let bytes = hex::decode(packet["payload_hex"].as_str().unwrap()).unwrap();
            receive(&mut state, id, &bytes).unwrap();
            assert!(!state.operations.inventory.unsupported_components);
            let received = common_player_in_state(&state, 71, false).unwrap();
            let actual = if label == "cursor" {
                received.inventory.cursor.as_ref().unwrap()
            } else {
                received.inventory.slots[9].as_ref().unwrap()
            };
            assert_eq!(actual.value, expected_common);
            assert_eq!(actual.source, ValueSource::Received { sequence: 13 });
            if label.ends_with("full") {
                let cursor = received.inventory.cursor.as_ref().unwrap();
                assert_eq!(cursor.value, expected_common);
                assert_eq!(cursor.source, ValueSource::Received { sequence: 13 });
                assert_eq!(
                    received.inventory.slots[10].as_ref().unwrap().value,
                    SlotKnowledge::Empty
                );
                let SlotKnowledge::Item { item: dirt } = &received.inventory.slots
                    [if storage { 44 } else { 45 }]
                .as_ref()
                .unwrap()
                .value
                else {
                    panic!("following default item")
                };
                assert_eq!(
                    (dirt.name.as_str(), dirt.count, dirt.data.clone()),
                    ("minecraft:dirt", 2, ItemData::Default)
                );
            }
            if storage {
                let screen = state.operations.inventory.container.as_ref().unwrap();
                let screen = screen.capture(received.session);
                let screen_item = screen.slots[27].as_ref().unwrap();
                assert_eq!(screen_item.value, expected_common);
                assert_eq!(screen_item.source, ValueSource::Received { sequence: 13 });
            }
            // Truncation cannot partially publish a known component or following
            // fields. Original fixture, not a mirror of our encoder.
            for end in 0..bytes.len() {
                let mut partial = if storage {
                    opened(baseline())
                } else {
                    baseline()
                };
                let original = serde_json::to_value(&partial.operations.inventory).unwrap();
                let sequences = partial.operations.inventory.slot_sequences.clone();
                assert!(
                    receive(&mut partial, id, &bytes[..end]).is_err(),
                    "{} {label} prefix {end}",
                    sample["name"]
                );
                assert_eq!(
                    serde_json::to_value(&partial.operations.inventory).unwrap(),
                    original
                );
                assert_eq!(partial.operations.inventory.slot_sequences, sequences);
            }
            let mut extra = bytes.clone();
            extra.push(0);
            let mut trailing = if storage {
                opened(baseline())
            } else {
                baseline()
            };
            assert!(receive(&mut trailing, id, &extra).is_err());
            applied += 1;
        }
    }
    assert_eq!(applied, 470);
}
