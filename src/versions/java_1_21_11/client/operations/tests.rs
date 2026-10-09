use super::*;

#[test]
fn block_hit_reach_preserves_hit_point_and_center_based_digging_admission() {
    let mut state = State {
        position: Some([0.19061256589492, 65., 0.499862279722396]),
        ..Default::default()
    };
    state.world.select_dimension(
        "minecraft:overworld".into(),
        super::super::super::world::Dimension::new(-64, 384).unwrap(),
    );
    let support = [2, 66, 4];
    state.world.seed_replay_cell(support, 1);
    assert!(check_reach(&state, support).is_err());
    assert!(check_hit_reach(&state, support, [0., 0.5, 0.5]).is_ok());
    assert!(check_hit_reach(&state, support, [1., 0.5, 0.5]).is_err());
    assert!(
        check_hit_reach(&state, [32, 66, 4], [0., 0.5, 0.5])
            .unwrap_err()
            .to_string()
            .contains("not loaded")
    );
    state.position = None;
    assert!(check_hit_reach(&state, support, [0., 0.5, 0.5]).is_err());
}

#[test]
fn received_item_registry_owner_survives_native_reconfiguration_packet_reset() {
    use crate::client::{ReceivedInventory, registry::Registry};
    let registry = Registry::for_version(MinecraftVersion::Java1_21_11);
    let stone = registry.item("minecraft:stone").unwrap();
    let enchantments = registry.item_component("minecraft:enchantments").unwrap();
    let mut slot = vec![0, 2]; // hotbar index zero, count two
    put_varint(&mut slot, stone.id.value());
    slot.extend([1, 0]);
    put_varint(&mut slot, enchantments.id.value());
    slot.extend([1, 0, 2]); // one enchantment, received registry ID zero, level two
    let declared = |names: [&str; 2]| {
        let mut payload = Vec::new();
        put_string(&mut payload, "minecraft:enchantment");
        payload.push(2);
        for name in names {
            put_string(&mut payload, name);
            payload.extend([1, 10, 0]);
        }
        payload
    };
    let capture = |state: &State| {
        let player = common_player_in_state(state, 7, false).unwrap();
        ReceivedInventory::capture(
            player.session,
            state.sequence,
            &player.inventory,
            state.registries.capture(player.session, state.sequence),
        )
        .unwrap()
    };
    let mut state = State::default();
    state
        .receive(
            ids::configuration_clientbound::REGISTRY_DATA,
            &declared(["example:first", "example:second"]),
            64,
        )
        .unwrap();
    state.registries.finish();
    state.phase = Phase::Play;
    state
        .receive(ids::play_clientbound::SET_PLAYER_INVENTORY, &slot, 64)
        .unwrap();
    let old = capture(&state);
    let old_item = old.slot(36).unwrap().unwrap().item().unwrap();
    let old_id = old_item
        .registry_state()
        .bind_entry("minecraft:enchantment", 0)
        .unwrap();
    assert_eq!(
        old_item.registry_state().entry_name(&old_id).unwrap(),
        "example:first"
    );
    state
        .receive(ids::play_clientbound::START_CONFIGURATION, &[], 64)
        .unwrap();
    let resetting = capture(&state);
    assert!(!resetting.registry_state().complete());
    assert!(resetting.slots().iter().all(Option::is_none));
    assert!(resetting.cursor().is_none());
    assert_eq!(old_item.stack().count, 2);
    assert!(old_item.registry_state().entry_name(&old_id).is_ok());
    state
        .receive(
            ids::configuration_clientbound::REGISTRY_DATA,
            &declared(["example:second", "example:first"]),
            64,
        )
        .unwrap();
    state.registries.finish();
    state.phase = Phase::Play;
    state
        .receive(ids::play_clientbound::SET_PLAYER_INVENTORY, &slot, 64)
        .unwrap();
    let new = capture(&state);
    let new_item = new.slot(36).unwrap().unwrap().item().unwrap();
    assert_eq!(old_item.stack(), new_item.stack()); // same bytes, different owner
    assert!(new_item.registry_state().entry_name(&old_id).is_err());
    let new_id = new_item
        .registry_state()
        .bind_entry("minecraft:enchantment", 0)
        .unwrap();
    assert_eq!(
        new_item.registry_state().entry_name(&new_id).unwrap(),
        "example:second"
    );
    assert_ne!(old_id, new_id);
    assert_eq!(
        old_item.registry_state().entry_name(&old_id).unwrap(),
        "example:first"
    );
}

#[test]
fn inventory_is_received_only_and_recursive_components_never_become_empty_slots() {
    let mut state = State {
        sequence: 9,
        ..State::default()
    };
    let mut full = vec![0, 0, 46];
    full.extend([0; 47]);
    receive(&mut state, ids::play_clientbound::WINDOW_ITEMS, &full).unwrap();
    assert!(
        state
            .operations
            .inventory
            .slots
            .iter()
            .all(|v| *v == InventorySlot::Empty)
    );
    let stone = items().iter().find(|i| i.name == "stone").unwrap().id;
    let mut changed = vec![0, 1];
    state.operations.inventory.pending_creative.push(0);
    state.operations.inventory.slots[36] = InventorySlot::Unavailable;
    put_varint(&mut changed, stone);
    changed.extend([0, 0]);
    receive(
        &mut state,
        ids::play_clientbound::SET_PLAYER_INVENTORY,
        &changed,
    )
    .unwrap();
    assert!(state.operations.inventory.pending_creative.is_empty());
    assert!(
        matches!(&state.operations.inventory.slots[36],InventorySlot::Item {item} if item.name=="minecraft:stone"&&item.count==1)
    );
    let prior = state.operations.inventory.clone();
    for length in 0..changed.len() {
        assert!(
            receive(
                &mut state,
                ids::play_clientbound::SET_PLAYER_INVENTORY,
                &changed[..length]
            )
            .is_err()
        );
        assert_eq!(state.operations.inventory.slots, prior.slots);
    }
    let mut unsupported = vec![0, 1];
    put_varint(&mut unsupported, stone);
    // Original native recursive component remains received data, never Empty.
    unsupported.extend([1, 0, 48, 0]);
    receive(
        &mut state,
        ids::play_clientbound::SET_PLAYER_INVENTORY,
        &unsupported,
    )
    .unwrap();
    assert!(
        matches!(&state.operations.inventory.slots[36], InventorySlot::ItemWithComponents { components, .. } if components.added[0].definition.name == "minecraft:bundle_contents" && components.added[0].bytes == vec![0])
    );
    assert!(!state.operations.inventory.unsupported_components);
    receive(&mut state, ids::play_clientbound::WINDOW_ITEMS, &full).unwrap();
    assert!(!state.operations.inventory.unsupported_components);
    // Other windows cannot silently preserve stale player inventory knowledge.
    receive(
        &mut state,
        ids::play_clientbound::WINDOW_ITEMS,
        &[1, 0, 0, 0],
    )
    .unwrap();
    assert!(
        state
            .operations
            .inventory
            .slots
            .iter()
            .all(|v| *v == InventorySlot::Unavailable)
    );
}

#[test]
fn game_mode_abilities_and_acknowledgements_are_validated_atomically() {
    let mut s = State::default();
    s.operations.reset_world(1).unwrap();
    assert_eq!(s.operations.game_mode, Some(GameMode::Creative));
    let mut abilities = vec![15];
    abilities.extend(0.05f32.to_be_bytes());
    abilities.extend(0.1f32.to_be_bytes());
    receive(&mut s, ids::play_clientbound::ABILITIES, &abilities).unwrap();
    assert_eq!(s.operations.abilities, Some(15));
    abilities[0] = 16;
    assert!(receive(&mut s, ids::play_clientbound::ABILITIES, &abilities).is_err());
    assert_eq!(s.operations.abilities, Some(15));
    receive(
        &mut s,
        ids::play_clientbound::ACKNOWLEDGE_PLAYER_DIGGING,
        &[3],
    )
    .unwrap();
    receive(
        &mut s,
        ids::play_clientbound::ACKNOWLEDGE_PLAYER_DIGGING,
        &[2],
    )
    .unwrap();
    assert_eq!(s.operations.ack, Some(3));
    let mut mode = vec![3];
    mode.extend(0.0f32.to_be_bytes());
    receive(&mut s, ids::play_clientbound::GAME_STATE_CHANGE, &mode).unwrap();
    assert_eq!(s.operations.game_mode, Some(GameMode::Survival));
    mode[1..].copy_from_slice(&f32::NAN.to_be_bytes());
    assert!(receive(&mut s, ids::play_clientbound::GAME_STATE_CHANGE, &mode).is_err());
    assert_eq!(s.operations.game_mode, Some(GameMode::Survival));
}

#[test]
fn item_registry_ids_are_unique_and_player_pose_rejects_nonfinite_values() {
    let mut ids = std::collections::BTreeSet::new();
    let mut names = std::collections::BTreeSet::new();
    for item in items() {
        assert!(ids.insert(item.id));
        assert!(names.insert(&item.name));
        assert!(item.stack_size > 0);
    }
    assert!(validate_pose([0.0; 3], [0.0, 90.0]).is_ok());
    assert!(validate_pose([f64::NAN; 3], [0.0; 2]).is_err());
    assert!(validate_pose([0.0; 3], [0.0, 91.0]).is_err());
    assert_eq!(
        super::super::super::wire::unpack_position(pack_position([-1, -64, 17]) as u64),
        [-1, -64, 17]
    );
}

#[test]
fn system_messages_preserve_native_components_and_mark_dropped_history() {
    let mut state = State::default();
    let message = b"\x0a\x08\x00\x04text\x00\x05hello\x00\x00";
    for sequence in 1..=130 {
        state.sequence = sequence;
        receive(&mut state, ids::play_clientbound::SYSTEM_CHAT, message).unwrap();
    }
    assert_eq!(state.operations.messages.len(), 128);
    assert_eq!(state.operations.messages_dropped_through, 2);
    assert_eq!(state.operations.messages[0].receive_sequence, 3);
    assert_eq!(
        state.operations.messages[0].component,
        Some(serde_json::json!({"text":"hello"}))
    );
    assert!(
        receive(
            &mut state,
            ids::play_clientbound::SYSTEM_CHAT,
            &message[..message.len() - 1]
        )
        .is_err()
    );
    assert_eq!(state.operations.messages.len(), 128);
    state.operations.reset_configuration(130);
    assert!(state.operations.messages.is_empty());
    assert_eq!(state.operations.messages_dropped_through, 130);
}
