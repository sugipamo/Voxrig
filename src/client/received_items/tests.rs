use super::*;
use crate::MinecraftVersion;
use crate::client::{
    InventoryObservation, ItemData, ObservedValue, PlayerObservation,
    registry::{Registry, ServerRegistryEntry, received::ReceivedRegistries},
};

fn player(version: MinecraftVersion, sequence: u64) -> PlayerObservation {
    PlayerObservation {
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
