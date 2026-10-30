use super::*;
use crate::client::{ValueSource, container::properties::MAX_SCREEN_PROPERTIES};

fn packet(window: i32, property: i16, value: i16) -> Vec<u8> {
    let mut payload = Vec::new();
    put_varint(&mut payload, window);
    payload.extend(property.to_be_bytes());
    payload.extend(value.to_be_bytes());
    payload
}
fn ordinal(value: &crate::client::ObservedValue<i16>) -> u64 {
    match value.source {
        ValueSource::Received { sequence } => sequence,
        _ => panic!("property must have received provenance"),
    }
}

#[test]
fn property_values_match_original_codec_fixtures() {
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../../data/client_api/screen_properties-1.21.11.json"
    ))
    .unwrap();
    let packets = fixtures["packets"].as_array().unwrap();
    assert_eq!(packets.len(), 75);
    for row in packets {
        let window = row["window"].as_i64().unwrap() as i32;
        let property = row["property"].as_i64().unwrap() as i16;
        let value = row["value"].as_i64().unwrap() as i16;
        let mut inventory = Inventory::default();
        let mut open = Vec::new();
        put_varint(&mut open, window);
        open.extend([14, 10, 0]);
        receive(
            &mut inventory,
            ids::play_clientbound::OPEN_WINDOW,
            &open,
            5,
            false,
        )
        .unwrap();
        let bytes = hex::decode(row["payload_hex"].as_str().unwrap()).unwrap();
        assert_eq!(bytes, packet(window, property, value));
        receive(
            &mut inventory,
            ids::play_clientbound::CRAFT_PROGRESS_BAR,
            &bytes,
            6,
            false,
        )
        .unwrap();
        let properties = &inventory
            .container
            .as_ref()
            .unwrap()
            .properties
            .as_ref()
            .unwrap()
            .values;
        assert_eq!(properties.len(), 1);
        assert_eq!(properties[&property].value, value);
        assert_eq!(ordinal(&properties[&property]), 6);
    }
}

#[test]
fn property_packets_validate_every_cut_and_tail_before_mutation() {
    let mut inventory = Inventory::default();
    let open = [0xac, 2, 14, 10, 0]; // window 300, furnace, empty compound title
    receive(
        &mut inventory,
        ids::play_clientbound::OPEN_WINDOW,
        &open,
        10,
        false,
    )
    .unwrap();
    let data = packet(300, i16::MIN, i16::MAX);
    for length in 0..data.len() {
        assert!(
            receive(
                &mut inventory,
                ids::play_clientbound::CRAFT_PROGRESS_BAR,
                &data[..length],
                11,
                false
            )
            .is_err(),
            "cut {length}"
        );
        assert!(
            inventory
                .container
                .as_ref()
                .unwrap()
                .properties
                .as_ref()
                .unwrap()
                .values
                .is_empty()
        );
    }
    let mut trailing = data.clone();
    trailing.push(0);
    assert!(
        receive(
            &mut inventory,
            ids::play_clientbound::CRAFT_PROGRESS_BAR,
            &trailing,
            11,
            false
        )
        .is_err()
    );
    assert!(
        inventory
            .container
            .as_ref()
            .unwrap()
            .properties
            .as_ref()
            .unwrap()
            .values
            .is_empty()
    );
    receive(
        &mut inventory,
        ids::play_clientbound::CRAFT_PROGRESS_BAR,
        &data,
        12,
        false,
    )
    .unwrap();
    let value = inventory
        .container
        .as_ref()
        .unwrap()
        .properties
        .as_ref()
        .unwrap()
        .values[&i16::MIN]
        .clone();
    assert_eq!(value.value, i16::MAX);
    assert_eq!(ordinal(&value), 12);
    // A malformed replacement cannot overwrite a retained receipt either.
    assert!(
        receive(
            &mut inventory,
            ids::play_clientbound::CRAFT_PROGRESS_BAR,
            &packet(300, i16::MIN, 0)[..5],
            13,
            false
        )
        .is_err()
    );
    assert_eq!(
        inventory
            .container
            .as_ref()
            .unwrap()
            .properties
            .as_ref()
            .unwrap()
            .values[&i16::MIN],
        value
    );
}

#[tokio::test]
async fn property_receipts_are_read_only_per_property_and_opening_bound() {
    let mut f = CommonFixture::new().await;
    let client = f.client();
    f.receive(
        ids::play_clientbound::CRAFT_PROGRESS_BAR,
        &packet(300, 0, 9),
    )
    .await;
    assert!(client.screen_properties().await.unwrap().screen.is_none());
    f.receive(ids::play_clientbound::OPEN_WINDOW, &[0xac, 2, 14, 10, 0])
        .await;
    let initial = client.screen_properties().await.unwrap().screen.unwrap();
    assert!(initial.values.is_empty());
    assert_eq!(
        initial.id,
        client.screen_state().await.unwrap().screen.unwrap().id
    );
    for (property, value) in [(0, 0), (1, 1600), (2, 17), (3, 200), (-7, -123)] {
        f.receive(
            ids::play_clientbound::CRAFT_PROGRESS_BAR,
            &packet(300, property, value),
        )
        .await;
    }
    let first = client.screen_properties().await.unwrap().screen.unwrap();
    assert_eq!(first.values[&0].value, 0);
    assert!(!first.values.contains_key(&4));
    assert_eq!(first.values[&-7].value, -123);
    f.receive(
        ids::play_clientbound::CRAFT_PROGRESS_BAR,
        &packet(300, 2, 17),
    )
    .await;
    let repeated = client.screen_properties().await.unwrap().screen.unwrap();
    assert!(ordinal(&repeated.values[&2]) > ordinal(&first.values[&2]));
    assert_eq!(repeated.values[&0], first.values[&0]);
    // Wrong window, including player zero, cannot replace opening data.
    for window in [0, 301, -1] {
        f.receive(
            ids::play_clientbound::CRAFT_PROGRESS_BAR,
            &packet(window, 2, 99),
        )
        .await;
    }
    f.receive(ids::play_clientbound::CLOSE_WINDOW, &[1]).await;
    assert_eq!(
        client
            .screen_properties()
            .await
            .unwrap()
            .screen
            .unwrap()
            .values,
        repeated.values
    );
    // Captures do not advance data or send a packet.
    let a = client.screen_properties().await.unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    let b = client.screen_properties().await.unwrap();
    assert_eq!(a.receive_sequence, b.receive_sequence);
    assert_eq!(a.screen.unwrap().values, b.screen.unwrap().values);
    assert!(
        timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    // Reopening the same numeric ID starts empty, even without an intervening close.
    f.receive(ids::play_clientbound::OPEN_WINDOW, &[0xac, 2, 14, 10, 0])
        .await;
    let reopened = client.screen_properties().await.unwrap().screen.unwrap();
    assert_ne!(reopened.id, initial.id);
    assert!(reopened.values.is_empty());
    f.receive(
        ids::play_clientbound::CRAFT_PROGRESS_BAR,
        &packet(300, 1, 0),
    )
    .await;
    f.receive(ids::play_clientbound::CLOSE_WINDOW, &[0xac, 2])
        .await;
    assert!(client.screen_properties().await.unwrap().screen.is_none());
    f.receive(
        ids::play_clientbound::CRAFT_PROGRESS_BAR,
        &packet(300, 1, 9),
    )
    .await;
    assert!(client.screen_properties().await.unwrap().screen.is_none());
    // A different window also begins with no inherited values.
    f.receive(ids::play_clientbound::OPEN_WINDOW, &[4, 0, 10, 0])
        .await;
    assert!(
        client
            .screen_properties()
            .await
            .unwrap()
            .screen
            .unwrap()
            .values
            .is_empty()
    );
    f.receive(ids::play_clientbound::CRAFT_PROGRESS_BAR, &packet(4, 1, 9))
        .await;
    let before_respawn = client.screen_properties().await.unwrap().session;
    f.api
        .bot
        .session
        .state
        .lock()
        .await
        .dimensions
        .push(crate::versions::java_1_21_11::world::Dimension::new(-64, 384).unwrap());
    let mut respawn = vec![0];
    put_string(&mut respawn, "minecraft:overworld");
    respawn.extend(0i64.to_be_bytes());
    respawn.extend([0, 255, 0, 0, 0, 0, 63, 3]);
    f.receive(ids::play_clientbound::RESPAWN, &respawn).await;
    let after_respawn = client.screen_properties().await.unwrap();
    assert_ne!(after_respawn.session, before_respawn);
    assert!(after_respawn.screen.is_none());
    f.receive(ids::play_clientbound::OPEN_WINDOW, &[4, 0, 10, 0])
        .await;
    f.receive(ids::play_clientbound::CRAFT_PROGRESS_BAR, &packet(4, 1, 9))
        .await;
    let old_session = client.screen_properties().await.unwrap().session;
    f.receive(ids::play_clientbound::START_CONFIGURATION, &[])
        .await;
    let reset = client.screen_properties().await.unwrap();
    assert_ne!(reset.session, old_session);
    assert!(reset.screen.is_none());
    f.stop().await;
    assert!(client.screen_properties().await.is_err());
}

#[tokio::test]
async fn unknown_properties_are_bounded_by_oldest_receipt_and_reset_on_open() {
    let mut f = CommonFixture::new().await;
    let client = f.client();
    f.open_swap_container().await;
    for property in 0..MAX_SCREEN_PROPERTIES as i16 {
        f.receive(
            ids::play_clientbound::CRAFT_PROGRESS_BAR,
            &packet(3, property, property),
        )
        .await;
    }
    f.receive(ids::play_clientbound::CRAFT_PROGRESS_BAR, &packet(3, 0, 0))
        .await;
    assert!(
        !client
            .screen_properties()
            .await
            .unwrap()
            .screen
            .unwrap()
            .evicted
    );
    f.receive(
        ids::play_clientbound::CRAFT_PROGRESS_BAR,
        &packet(3, i16::MAX, i16::MIN),
    )
    .await;
    let bounded = client.screen_properties().await.unwrap().screen.unwrap();
    assert_eq!(bounded.values.len(), MAX_SCREEN_PROPERTIES);
    assert!(bounded.evicted);
    assert!(bounded.values.contains_key(&0));
    assert!(!bounded.values.contains_key(&1));
    assert_eq!(bounded.values[&i16::MAX].value, i16::MIN);
    f.open_swap_container().await;
    let empty = client.screen_properties().await.unwrap().screen.unwrap();
    assert!(empty.values.is_empty());
    assert!(!empty.evicted);
    f.stop().await;
}

#[tokio::test]
async fn received_close_retires_properties_after_submitted_close_and_player_resync() {
    let mut f = CommonFixture::new().await;
    let client = f.client();
    let id = f.open_swap_container().await;
    f.receive(ids::play_clientbound::CRAFT_PROGRESS_BAR, &packet(3, 2, 33))
        .await;
    client.survival().close_container(id).await.unwrap();
    read_packet(&mut f.peer, None).await.unwrap();
    let mut player = vec![0, 1, 46];
    player.extend([0; 47]); // 46 empty slots and empty cursor.
    f.receive(ids::play_clientbound::WINDOW_ITEMS, &player)
        .await;
    let screen = client.screen_state().await.unwrap();
    assert_eq!(screen.active_window, Some(0));
    // This full player receipt retires the exactly closed historical screen
    // and its properties, without manufacturing a received server close.
    assert!(screen.screen.is_none());
    assert!(matches!(
        screen.active_screen(),
        crate::client::container::ActiveScreen::Closed
    ));
    assert!(client.screen_properties().await.unwrap().screen.is_none());
    assert!(
        client
            .survival()
            .container_close_record()
            .await
            .unwrap()
            .unwrap()
            .server_close_sequence
            .is_none()
    );
    f.receive(ids::play_clientbound::CLOSE_WINDOW, &[3]).await;
    assert!(client.screen_properties().await.unwrap().screen.is_none());
    f.receive(ids::play_clientbound::CRAFT_PROGRESS_BAR, &packet(3, 2, 99))
        .await;
    assert!(client.screen_properties().await.unwrap().screen.is_none());
    f.open_swap_container().await;
    let reopened = client.screen_properties().await.unwrap().screen.unwrap();
    assert_ne!(reopened.id, id);
    assert!(reopened.values.is_empty());
    f.stop().await;
}

#[tokio::test]
async fn window0_full_contents_does_not_retire_close_from_another_connection() {
    let mut f = CommonFixture::new().await;
    let client = f.client();
    f.open_swap_container().await;
    let id = client.screen_state().await.unwrap().screen.unwrap().id;
    client.survival().close_container(id).await.unwrap();
    let packet = read_packet(&mut f.peer, None).await.unwrap();
    assert_eq!(packet, (ids::play_serverbound::CLOSE_WINDOW, vec![3]));
    // Even if an obsolete close were retained across a connection boundary,
    // its session must not authorize retiring the current received opening.
    f.api.bot.session.state.lock().await.connection_id += 1;
    let mut player = vec![0, 1, 46];
    player.extend([0; 47]);
    f.receive(ids::play_clientbound::WINDOW_ITEMS, &player)
        .await;
    {
        let state = f.api.bot.session.state.lock().await;
        assert_eq!(state.operations.inventory.window_id, Some(3));
        assert!(state.operations.inventory.container.is_some());
    }
    f.stop().await;
}
