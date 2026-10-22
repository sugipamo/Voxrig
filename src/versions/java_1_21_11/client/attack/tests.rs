use super::*;
use std::time::Duration;

fn state() -> State {
    let mut state = State::default();
    state.operations.local_player = operations::LocalPlayerState::spawned(42);
    state
}
fn attribute(entity: i32, base: f64, modifiers: &[(&str, f64, u8)]) -> Vec<u8> {
    let mut packet = Vec::new();
    put_varint(&mut packet, entity);
    put_varint(&mut packet, 1);
    put_varint(&mut packet, ATTACK_SPEED_ATTRIBUTE);
    packet.extend(base.to_be_bytes());
    put_varint(&mut packet, modifiers.len() as i32);
    for (name, amount, operation) in modifiers {
        put_string(&mut packet, name);
        packet.extend(amount.to_be_bytes());
        packet.push(*operation);
    }
    packet
}
fn receive(state: &mut State, packet: &[u8]) -> anyhow::Result<bool> {
    operations::receive(
        state,
        ids::play_clientbound::ENTITY_UPDATE_ATTRIBUTES,
        packet,
    )
}
fn charge(state: &State) -> f32 {
    state.attack_strength.value(&state.operations.local_player)
}

#[tokio::test(start_paused = true)]
async fn attack_strength_whole_ticks_default_reset_and_saturation() {
    let mut s = state();
    assert_eq!(charge(&s), 0.0);
    tokio::time::advance(Duration::from_millis(49)).await;
    assert_eq!(charge(&s), 0.0);
    tokio::time::advance(Duration::from_millis(1)).await;
    assert_eq!(charge(&s), 0.2);
    tokio::time::advance(Duration::from_millis(200)).await;
    assert_eq!(charge(&s), 1.0);
    tokio::time::advance(Duration::from_secs(60)).await;
    assert_eq!(charge(&s), 1.0);
    s.attack_strength.reset();
    assert_eq!(charge(&s), 0.0);
    tokio::time::advance(Duration::from_millis(100)).await;
    assert_eq!(charge(&s), 0.4);
}

#[tokio::test(start_paused = true)]
async fn attack_strength_received_weapon_and_all_modifier_operations() {
    let mut s = state();
    // Same modifier and float-derived amount as an own-player iron sword update.
    receive(
        &mut s,
        &attribute(
            42,
            4.0,
            &[("minecraft:base_attack_speed", -2.4_f32 as f64, 0)],
        ),
    )
    .unwrap();
    tokio::time::advance(Duration::from_millis(600)).await;
    let speed = 4.0 - 2.4_f32 as f64;
    assert_eq!(charge(&s), 12.0 / (20.0 / speed) as f32);
    assert!(charge(&s) < 1.0);
    tokio::time::advance(Duration::from_millis(50)).await;
    assert_eq!(charge(&s), 1.0);

    s.attack_strength.reset();
    receive(
        &mut s,
        &attribute(
            42,
            4.0,
            &[
                ("voxrig:add", -2.0, 0),
                ("voxrig:base", 0.5, 1),
                ("voxrig:total", 1.0, 2),
            ],
        ),
    )
    .unwrap();
    tokio::time::advance(Duration::from_millis(100)).await;
    assert_eq!(charge(&s), 2.0 / (20.0 / 6.0) as f32);
    // Attribute updates change the period, never reset elapsed charge ticks.
    receive(&mut s, &attribute(42, 10.0, &[])).unwrap();
    assert_eq!(charge(&s), 1.0);
    receive(&mut s, &attribute(42, 4.0, &[])).unwrap();
    assert_eq!(charge(&s), 0.4);
}

#[tokio::test(start_paused = true)]
async fn attack_strength_zero_negative_and_capped_speed() {
    let mut s = state();
    for base in [0.0, -1.0] {
        receive(&mut s, &attribute(42, base, &[])).unwrap();
        tokio::time::advance(Duration::from_secs(60)).await;
        assert_eq!(charge(&s), 0.0);
    }
    for base in [1024.0, 2048.0] {
        receive(&mut s, &attribute(42, base, &[])).unwrap();
        s.attack_strength.reset();
        assert_eq!(charge(&s), 0.0);
        tokio::time::advance(Duration::from_millis(50)).await;
        assert_eq!(charge(&s), 1.0);
    }
}

#[tokio::test(start_paused = true)]
async fn attack_strength_other_entities_and_invalid_packets_cannot_replace_own_speed() {
    let mut s = state();
    receive(&mut s, &attribute(42, 2.0, &[])).unwrap();
    tokio::time::advance(Duration::from_millis(100)).await;
    assert_eq!(charge(&s), 0.2);
    assert!(!receive(&mut s, &attribute(43, 100.0, &[])).unwrap());
    assert_eq!(charge(&s), 0.2);
    let packet = attribute(42, 100.0, &[("voxrig:test", 1.0, 0)]);
    for len in 1..packet.len() {
        assert!(receive(&mut s, &packet[..len]).is_err());
        assert_eq!(charge(&s), 0.2);
    }
    for packet in [
        attribute(42, f64::NAN, &[]),
        attribute(42, f64::INFINITY, &[]),
        attribute(42, 4.0, &[("voxrig:test", 1.0, 3)]),
    ] {
        assert!(receive(&mut s, &packet).is_err());
        assert_eq!(charge(&s), 0.2);
    }
}

fn slot(state: &mut State, index: u8, name: &str, count: u8) {
    let mut packet = vec![index, count];
    if count != 0 {
        let registry =
            crate::client::registry::Registry::for_version(MinecraftVersion::Java1_21_11);
        put_varint(&mut packet, registry.item(name).unwrap().id.value());
        packet.extend([0, 0]);
    }
    operations::receive(state, ids::play_clientbound::SET_PLAYER_INVENTORY, &packet).unwrap();
}

#[tokio::test(start_paused = true)]
async fn attack_strength_received_selection_and_main_hand_item_identity_changes() {
    let mut s = state();
    slot(&mut s, 0, "minecraft:stone", 1);
    tokio::time::advance(Duration::from_millis(250)).await;
    assert_eq!(charge(&s), 1.0);
    slot(&mut s, 0, "minecraft:stone", 2); // Count-only update.
    slot(&mut s, 1, "minecraft:iron_sword", 1); // Unselected item.
    assert_eq!(charge(&s), 1.0);
    operations::receive(&mut s, ids::play_clientbound::HELD_ITEM_SLOT, &[0]).unwrap();
    assert_eq!(charge(&s), 1.0); // Same slot.
    operations::receive(&mut s, ids::play_clientbound::HELD_ITEM_SLOT, &[1]).unwrap();
    assert_eq!(charge(&s), 0.0);
    tokio::time::advance(Duration::from_millis(250)).await;
    assert_eq!(charge(&s), 1.0);
    assert!(operations::receive(&mut s, ids::play_clientbound::HELD_ITEM_SLOT, &[9]).is_err());
    assert_eq!(charge(&s), 1.0);
    // Updating damage on the same held sword preserves elapsed charge.
    let registry = crate::client::registry::Registry::for_version(MinecraftVersion::Java1_21_11);
    let mut damage = vec![1, 1]; // Hotbar index, count.
    put_varint(
        &mut damage,
        registry.item("minecraft:iron_sword").unwrap().id.value(),
    );
    damage.extend([1, 0]); // One added component, none removed.
    put_varint(
        &mut damage,
        registry
            .item_component("minecraft:damage")
            .unwrap()
            .id
            .value(),
    );
    put_varint(&mut damage, 7);
    operations::receive(&mut s, ids::play_clientbound::SET_PLAYER_INVENTORY, &damage).unwrap();
    assert_eq!(charge(&s), 1.0);
    slot(&mut s, 1, "minecraft:stone", 1);
    assert_eq!(charge(&s), 0.0);
    tokio::time::advance(Duration::from_millis(250)).await;
    slot(&mut s, 1, "minecraft:air", 0);
    assert_eq!(charge(&s), 0.0);
}

#[tokio::test(start_paused = true)]
async fn attack_strength_reconfiguration_discards_old_ticks_and_attributes() {
    let mut s = state();
    s.phase = Phase::Play;
    receive(&mut s, &attribute(42, 2.0, &[])).unwrap();
    tokio::time::advance(Duration::from_millis(500)).await;
    assert_eq!(charge(&s), 1.0);
    s.receive(ids::play_clientbound::START_CONFIGURATION, &[], 64)
        .unwrap();
    assert!(s.operations.local_player.received_attributes.is_empty());
    assert_eq!(charge(&s), 0.0);
    tokio::time::advance(Duration::from_millis(50)).await;
    assert_eq!(charge(&s), 0.2);
}

#[tokio::test(start_paused = true)]
async fn attack_strength_login_and_respawn_reset_world_charge() {
    let mut s = state();
    s.phase = Phase::Play;
    s.dimensions.push(Dimension::new(-64, 384).unwrap());
    let mut spawn = vec![0]; // Dimension type registry ID.
    put_string(&mut spawn, "minecraft:overworld");
    spawn.extend(123i64.to_be_bytes());
    spawn.extend([0, 255, 0, 1, 0, 0, 63]); // Mode, previous, debug, flat, no death, portal, sea.
    let mut login = 42i32.to_be_bytes().to_vec();
    login.extend([0, 0, 5, 3, 2, 0, 1, 0]);
    login.extend(&spawn);
    login.push(0); // Secure chat flag.
    let mut respawn = spawn;
    respawn.push(3); // Keep data bits; received-attribute cache still starts fresh.
    for (id, packet) in [
        (ids::play_clientbound::LOGIN, login),
        (ids::play_clientbound::RESPAWN, respawn),
    ] {
        receive(&mut s, &attribute(42, 2.0, &[])).unwrap();
        tokio::time::advance(Duration::from_millis(500)).await;
        assert_eq!(charge(&s), 1.0);
        // Invalid frames must not reset a charged player.
        assert!(apply_play(&mut s, id, &packet[..packet.len() - 1], 64).is_err());
        assert_eq!(charge(&s), 1.0);
        apply_play(&mut s, id, &packet, 64).unwrap();
        assert!(s.operations.local_player.received_attributes.is_empty());
        assert_eq!(charge(&s), 0.0);
        tokio::time::advance(Duration::from_millis(50)).await;
        assert_eq!(charge(&s), 0.2);
    }
}
