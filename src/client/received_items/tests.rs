use super::*;
use crate::MinecraftVersion;
use crate::client::{
    InventoryObservation, ItemData, ObservedValue, PlayerObservation,
    registry::{Registry, ServerRegistryEntry, received::ReceivedRegistries},
};

fn player(version: MinecraftVersion, sequence: u64) -> PlayerObservation {
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
        receive_sequence: sequence,
        pending_dispatch: false,
        dimension: None,
        position: None,
        received_pose: None,
        rotation: [0.0; 2],
        game_mode: None,
        may_fly: None,
        health: None,
        selected_hotbar: None,
        inventory: InventoryObservation::default(),
    }
}
fn item(version: MinecraftVersion) -> SlotKnowledge {
    let stone = Registry::for_version(version)
        .item("minecraft:stone")
        .unwrap();
    SlotKnowledge::Item {
        item: ItemStack {
            id: stone.id,
            name: stone.name,
            count: 2,
            data: ItemData::Default,
        },
    }
}
fn observation(value: SlotKnowledge, sequence: u64) -> ObservedValue<SlotKnowledge> {
    ObservedValue {
        value,
        source: ValueSource::Received { sequence },
    }
}
fn registries() -> ReceivedRegistries {
    let mut registries = ReceivedRegistries::default();
    registries.reset(10);
    registries
        .modern_registry(
            "minecraft:enchantment".into(),
            vec![ServerRegistryEntry {
                name: "example:first".into(),
                data: vec![10, 0],
            }],
            11,
            0,
        )
        .unwrap();
    registries.finish();
    registries
}

#[test]
fn received_inventory_keeps_old_owner_after_configuration_and_world_changes() {
    let mut player = player(MinecraftVersion::Java1_21_11, 20);
    player.inventory.slots[9] = Some(observation(item(player.session.version), 12));
    player.inventory.slots[10] = Some(observation(SlotKnowledge::Empty, 13));
    player.inventory.slots[11] = Some(observation(SlotKnowledge::Unavailable, 14));
    player.inventory.cursor = Some(observation(SlotKnowledge::Empty, 15));
    player.inventory.local_cache = Some(vec![SlotKnowledge::Empty; 46]);
    let mut registries = registries();
    let captured = capture(player.clone(), registries.capture(player.session, 20)).unwrap();
    let received = captured.slot(9).unwrap().unwrap().item().unwrap();
    let old_id = received
        .registry_state()
        .bind("minecraft:enchantment", 0)
        .unwrap();
    assert_eq!(
        received.registry_state().resolve(&old_id).unwrap().name,
        "example:first"
    );
    assert_eq!(received.receive_sequence(), 12);
    assert_eq!(received.stack().count, 2);
    assert!(captured.slot(0).unwrap().is_none());
    assert_eq!(
        captured.slot(10).unwrap().unwrap().value(),
        &SlotKnowledge::Empty
    );
    assert_eq!(
        captured.slot(11).unwrap().unwrap().value(),
        &SlotKnowledge::Unavailable
    );
    assert!(captured.cursor().unwrap().item().is_none());
    assert!(captured.slot(46).is_err());
    player.session.world_generation += 1;
    let respawn = registries.capture(player.session, 21);
    assert_eq!(respawn.stamp(), captured.registry_state().stamp());
    assert!(respawn.resolve(&old_id).is_ok());
    registries.reset(30);
    registries
        .modern_registry(
            "minecraft:enchantment".into(),
            vec![ServerRegistryEntry {
                name: "example:replacement".into(),
                data: vec![10, 0],
            }],
            31,
            0,
        )
        .unwrap();
    registries.finish();
    assert!(
        registries
            .capture(player.session, 32)
            .resolve(&old_id)
            .is_err()
    );
    assert_eq!(
        received.registry_state().resolve(&old_id).unwrap().name,
        "example:first"
    );
    let json = serde_json::to_value(&captured).unwrap();
    assert!(json["slots"][9].get("registries").is_none());
    assert_eq!(
        json["slots"][9]["registry_stamp"]["configuration_generation"],
        10
    );
    assert!(json.get("local_cache").is_none());
}

#[test]
fn received_inventory_rejects_mixed_boundaries_and_non_receipt_sources() {
    let mut player = player(MinecraftVersion::Java1_21_11, 20);
    let registries = registries();
    for source in [
        ValueSource::Submitted,
        ValueSource::Predicted,
        ValueSource::LocalCache,
        ValueSource::Received { sequence: 9 },
        ValueSource::Received { sequence: 21 },
    ] {
        player.inventory.slots[9] = Some(ObservedValue {
            value: item(player.session.version),
            source,
        });
        assert!(capture(player.clone(), registries.capture(player.session, 20)).is_err());
        player.inventory.slots[9] = None;
        player.inventory.cursor = Some(ObservedValue {
            value: SlotKnowledge::Empty,
            source,
        });
        assert!(capture(player.clone(), registries.capture(player.session, 20)).is_err());
        player.inventory.cursor = None;
    }
    assert!(capture(player.clone(), registries.capture(player.session, 19)).is_err());
    let mut foreign = player.session;
    foreign.connection_id += 1;
    assert!(capture(player.clone(), registries.capture(foreign, 20)).is_err());
    foreign = player.session;
    foreign.world_generation += 1;
    assert!(capture(player.clone(), registries.capture(foreign, 20)).is_err());
    player.inventory.slots[9] = Some(observation(item(MinecraftVersion::Java1_16_1), 12));
    assert!(capture(player.clone(), registries.capture(player.session, 20)).is_err());
}

fn capture(
    player: PlayerObservation,
    registries: ServerRegistryObservation,
) -> Result<ReceivedInventory> {
    ReceivedInventory::capture(
        player.session,
        player.receive_sequence,
        &player.inventory,
        registries,
    )
}

fn inventory_for(item: ItemStack, sequence: u64, packet: u64) -> ReceivedInventory {
    let version = item.id.version();
    let mut player = player(version, sequence);
    player.inventory.slots[9] = Some(observation(
        SlotKnowledge::Item { item: item.clone() },
        packet,
    ));
    player.inventory.slots[10] = Some(observation(
        SlotKnowledge::Item { item: item.clone() },
        packet,
    ));
    player.inventory.cursor = Some(observation(SlotKnowledge::Item { item }, packet));
    let mut registries = registries();
    if version == MinecraftVersion::Java1_16_1 {
        registries = ReceivedRegistries::default();
        let data: serde_json::Value = serde_json::from_str(include_str!(
            "../../../data/client_api/registry_catalog_cases-1.16.1.json"
        ))
        .unwrap();
        registries
            .legacy_join(
                hex::decode(data["legacy_builtin_codec_hex"].as_str().unwrap()).unwrap(),
                10,
            )
            .unwrap();
    }
    let received = registries.capture(player.session, sequence);
    capture(player, received).unwrap()
}

#[test]
fn received_item_native_comparison_uses_decoded_fields_and_exact_receipt_location() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let registry = Registry::for_version(version);
        let stone = registry.item("minecraft:stone").unwrap();
        let make = |value: u32| {
            let mut bytes = vec![10];
            if version == MinecraftVersion::Java1_16_1 {
                bytes.extend([0, 0]);
            }
            bytes.extend([5, 0, 1, b'x']);
            bytes.extend(value.to_be_bytes());
            bytes.push(0);
            ItemStack {
                id: stone.id,
                name: stone.name.clone(),
                count: 2,
                data: match version {
                    MinecraftVersion::Java1_16_1 => ItemData::LegacyNbt { bytes },
                    MinecraftVersion::Java1_21_11 => ItemData::ModernComponents {
                        patch: crate::client::ItemComponentPatch {
                            added: vec![crate::client::ItemComponent {
                                definition: registry
                                    .item_component("minecraft:custom_data")
                                    .unwrap(),
                                bytes,
                            }],
                            removed: vec![],
                        },
                    },
                },
            }
        };
        let first = inventory_for(make(f32::NAN.to_bits()), 20, 12);
        let later = inventory_for(make(f32::NAN.to_bits()), 21, 12);
        let distinct = inventory_for(make(f32::NAN.to_bits()), 21, 13);
        let a = first.slot(9).unwrap().unwrap().item().unwrap();
        assert!(
            a.native_equivalent(&later.slot(9).unwrap().unwrap().item().unwrap())
                .unwrap()
        );
        let modern = version == MinecraftVersion::Java1_21_11;
        assert_eq!(
            a.native_equivalent(&first.slot(10).unwrap().unwrap().item().unwrap())
                .unwrap(),
            modern
        );
        assert_eq!(
            a.native_equivalent(&first.cursor().unwrap().item().unwrap())
                .unwrap(),
            modern
        );
        assert_eq!(
            a.native_equivalent(&distinct.slot(9).unwrap().unwrap().item().unwrap())
                .unwrap(),
            modern
        );
        let finite = inventory_for(make(1.0f32.to_bits()), 21, 13);
        let b = finite.slot(9).unwrap().unwrap().item().unwrap();
        assert!(!a.native_equivalent(&b).unwrap());
        assert!(
            b.native_equivalent(&finite.slot(10).unwrap().unwrap().item().unwrap())
                .unwrap()
        );
        let clone = first.slot(9).unwrap().unwrap().clone();
        assert!(a.native_equivalent(&clone.item().unwrap()).unwrap());
    }
}

#[test]
fn received_modern_item_comparison_applies_prototypes_removals_and_context_guards() {
    let registry = Registry::for_version(MinecraftVersion::Java1_21_11);
    let stone = registry.item("minecraft:stone").unwrap();
    let base = ItemStack {
        id: stone.id,
        name: stone.name,
        count: 2,
        data: ItemData::Default,
    };
    let default = inventory_for(base.clone(), 20, 12);
    let mut explicit = base.clone();
    explicit.data = ItemData::ModernComponents {
        patch: crate::client::ItemComponentPatch {
            added: vec![crate::client::ItemComponent {
                definition: registry.item_component("minecraft:max_stack_size").unwrap(),
                bytes: vec![0xc0, 0],
            }],
            removed: vec![],
        },
    };
    let equivalent = inventory_for(explicit, 21, 13);
    let a = default.slot(9).unwrap().unwrap().item().unwrap();
    let b = equivalent.slot(9).unwrap().unwrap().item().unwrap();
    assert!(a.native_data_equivalent(&b).unwrap());
    assert_ne!(a.stack(), b.stack());
    assert!(a.native_equivalent(&b).unwrap());
    let mut removed = base.clone();
    removed.data = ItemData::ModernComponents {
        patch: crate::client::ItemComponentPatch {
            added: vec![],
            removed: vec![registry.item_component("minecraft:max_stack_size").unwrap()],
        },
    };
    let removed = inventory_for(removed, 21, 13);
    assert!(
        !a.native_equivalent(&removed.slot(9).unwrap().unwrap().item().unwrap())
            .unwrap()
    );
    let mut changed = base.clone();
    changed.count = 3;
    let changed = inventory_for(changed, 21, 13);
    assert!(
        a.native_data_equivalent(&changed.slot(9).unwrap().unwrap().item().unwrap())
            .unwrap()
    );
    assert!(
        !a.native_equivalent(&changed.slot(9).unwrap().unwrap().item().unwrap())
            .unwrap()
    );
    let mut player = player(registry.version(), 20);
    player.session.connection_id += 1;
    player.inventory.slots[9] = Some(observation(SlotKnowledge::Item { item: base }, 12));
    let foreign = capture(player.clone(), registries().capture(player.session, 20)).unwrap();
    assert!(
        a.native_equivalent(&foreign.slot(9).unwrap().unwrap().item().unwrap())
            .is_err()
    );
}

#[test]
fn received_item_native_comparison_blocks_unverified_named_holder_reload_lifetimes() {
    let registry = Registry::for_version(MinecraftVersion::Java1_21_11);
    let stone = registry.item("minecraft:stone").unwrap();
    let mut stack = ItemStack {
        id: stone.id,
        name: stone.name,
        count: 1,
        data: ItemData::Default,
    };
    let mut state = player(registry.version(), 20);
    state.inventory.slots[10] = Some(observation(
        SlotKnowledge::Item {
            item: stack.clone(),
        },
        13,
    ));
    stack.data = ItemData::ModernComponents {
        patch: crate::client::ItemComponentPatch {
            added: vec![crate::client::ItemComponent {
                definition: registry.item_component("minecraft:can_break").unwrap(),
                bytes: hex::decode(
                    "0101001a6d696e6563726166743a6d696e6561626c652f7069636b61786500000000",
                )
                .unwrap(),
            }],
            removed: vec![],
        },
    };
    state.inventory.slots[9] = Some(observation(SlotKnowledge::Item { item: stack }, 13));
    let mut owners = registries();
    let mut tags = vec![1];
    crate::protocol::put_string(&mut tags, "minecraft:block");
    tags.push(1);
    crate::protocol::put_string(&mut tags, "minecraft:mineable/pickaxe");
    tags.push(0);
    owners.receive_tags(&tags, 12, registry.version()).unwrap();
    let before = capture(state.clone(), owners.capture(state.session, 20)).unwrap();
    owners.receive_tags(&tags, 21, registry.version()).unwrap();
    state.receive_sequence = 22;
    let after = capture(state.clone(), owners.capture(state.session, 22)).unwrap();
    let a = before.slot(9).unwrap().unwrap().item().unwrap();
    let b = after.slot(9).unwrap().unwrap().item().unwrap();
    assert!(a.native_equivalent(&a).unwrap());
    assert_eq!(
        a.native_equivalent(&b).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
    assert!(
        before
            .slot(10)
            .unwrap()
            .unwrap()
            .item()
            .unwrap()
            .native_equivalent(&after.slot(10).unwrap().unwrap().item().unwrap())
            .unwrap()
    );
}

#[test]
fn received_item_comparison_resolves_hover_counts_and_custom_dialog_registry_names() {
    use std::io::Read;
    let registry = Registry::for_version(MinecraftVersion::Java1_21_11);
    let stone = registry.item("minecraft:stone").unwrap();
    let name = registry.item_component("minecraft:custom_name").unwrap();
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(
        include_bytes!("../../../data/client_api/text_dependency_cases-1.21.11.json.gz").as_slice(),
    )
    .read_to_end(&mut bytes)
    .unwrap();
    let facts: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let field = |case: &str| {
        let row = facts["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["case"] == case)
            .unwrap();
        hex::decode(row["input_hex"].as_str().unwrap()).unwrap()
    };
    let stack = |bytes| SlotKnowledge::Item {
        item: ItemStack {
            id: stone.id,
            name: stone.name.clone(),
            count: 1,
            data: ItemData::ModernComponents {
                patch: crate::client::ItemComponentPatch {
                    added: vec![crate::client::ItemComponent {
                        definition: name.clone(),
                        bytes,
                    }],
                    removed: vec![],
                },
            },
        },
    };
    let mut state = player(registry.version(), 20);
    state.inventory.slots[9] = Some(observation(stack(field("hover-item")), 12));
    state.inventory.slots[10] = Some(observation(stack(field("item-count-5-3ff33333")), 13));
    state.inventory.slots[11] = Some(observation(stack(field("hover-item-count")), 14));
    let received = capture(state.clone(), registries().capture(state.session, 20)).unwrap();
    let a = received.slot(9).unwrap().unwrap().item().unwrap();
    assert!(
        !a.native_data_equivalent(&received.slot(11).unwrap().unwrap().item().unwrap())
            .unwrap()
    );
    assert!(
        a.native_equivalent(&received.slot(10).unwrap().unwrap().item().unwrap())
            .unwrap()
    );
    assert!(
        !a.native_equivalent(&received.slot(11).unwrap().unwrap().item().unwrap())
            .unwrap()
    );
    fn string(out: &mut Vec<u8>, value: &str) {
        out.extend_from_slice(&(value.len() as u16).to_be_bytes());
        out.extend_from_slice(value.as_bytes());
    }
    let dialog = |value: &str| {
        let mut out = vec![10, 8];
        string(&mut out, "text");
        string(&mut out, "Voxrig");
        out.push(10);
        string(&mut out, "click_event");
        out.push(8);
        string(&mut out, "action");
        string(&mut out, "show_dialog");
        out.push(8);
        string(&mut out, "dialog");
        string(&mut out, value);
        out.extend_from_slice(&[0, 0]);
        out
    };
    let mut owners = ReceivedRegistries::default();
    owners.reset(10);
    owners
        .modern_registry(
            "minecraft:dialog".into(),
            vec![
                ServerRegistryEntry {
                    name: "example:first".into(),
                    data: vec![10, 0],
                },
                ServerRegistryEntry {
                    name: "example:second".into(),
                    data: vec![10, 0],
                },
            ],
            11,
            0,
        )
        .unwrap();
    owners.finish();
    state.inventory.slots[9] = Some(observation(stack(dialog("example:first")), 12));
    state.inventory.slots[10] = Some(observation(stack(dialog("example:second")), 13));
    state.inventory.slots[11] = Some(observation(stack(dialog("example:missing")), 14));
    let received = capture(state.clone(), owners.capture(state.session, 20)).unwrap();
    let a = received.slot(9).unwrap().unwrap().item().unwrap();
    assert!(a.native_equivalent(&a).unwrap());
    assert!(
        !a.native_equivalent(&received.slot(10).unwrap().unwrap().item().unwrap())
            .unwrap()
    );
    assert!(
        a.native_equivalent(&received.slot(11).unwrap().unwrap().item().unwrap())
            .is_err()
    );
}
