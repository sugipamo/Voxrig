use super::hand_swaps::fixture;
use super::*;
use crate::client::inventory::{
    InventoryClickButton, InventorySource, SelectedDropAmount as Amount, SelectedDropStage as Stage,
};

async fn read_resync(peer: &mut tokio::net::TcpStream) {
    let expected: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../data/client_api/selected_drop_resync-1.21.11.json"
    ))
    .unwrap();
    assert_eq!(
        read_packet(peer, None).await.unwrap(),
        (
            ids::play_serverbound::WINDOW_CLICK,
            hex::decode(expected["payload_hex"].as_str().unwrap()).unwrap()
        )
    );
}

#[tokio::test]
async fn selected_drop_modes_amounts_one_send_fresh_receipt_ledger_and_history() {
    let mut f = fixture().await;
    let client = f.client();
    for mode in [GameMode::Survival, GameMode::Creative] {
        let mut packet = vec![3];
        packet.extend(
            (if mode == GameMode::Survival {
                0f32
            } else {
                1f32
            })
            .to_be_bytes(),
        );
        f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &packet)
            .await;
        for (amount, count, remaining) in [
            (Amount::One, 12, 11),
            (Amount::One, 1, 0),
            (Amount::All, 12, 0),
        ] {
            f.slot(36, plain("dirt", count)).await;
            let previous = f
                .api
                .bot
                .session
                .interaction_sequence
                .load(Ordering::Acquire);
            let record = if mode == GameMode::Survival {
                client.survival().drop_selected(amount).await
            } else {
                client.creative().drop_selected(amount).await
            }
            .unwrap();
            assert!(record.send.dispatched && record.send.resync_dispatched);
            assert_eq!(record.stage, Stage::Pending);
            assert_eq!(record.send.interaction_sequence, previous + 1);
            let mut expected = vec![
                if amount == Amount::One { 4 } else { 3 },
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
            ];
            put_varint(&mut expected, previous + 1);
            assert_eq!(
                read_packet(&mut f.peer, None).await.unwrap(),
                (ids::play_serverbound::BLOCK_DIG, expected)
            );
            read_resync(&mut f.peer).await;
            let mut drain = f.api.bot.session.ledger.cut(false);
            assert!(drain.inspect().is_none());
            assert!(client.survival().drop_selected(amount).await.is_err());
            assert!(client.survival().select_hotbar(1).await.is_err());
            assert!(client.survival().swap_hands().await.is_err());
            assert!(
                client
                    .survival()
                    .click_inventory(InventorySource::Player, 9, InventoryClickButton::Left)
                    .await
                    .is_err()
            );
            assert!(f.api.swap_player_hotbar(9, 0).await.is_err());
            // Fresh unchanged source and unrelated/offhand receipts cannot confirm a drop.
            f.slot(36, plain("dirt", count)).await;
            f.slot(45, InventorySlot::Empty).await;
            f.slot(9, plain("stone", 4)).await;
            assert_eq!(
                client
                    .survival()
                    .selected_drop_record()
                    .await
                    .unwrap()
                    .unwrap()
                    .stage,
                Stage::Pending
            );
            assert!(drain.inspect().is_none());
            let after = if remaining == 0 {
                InventorySlot::Empty
            } else {
                plain("dirt", remaining)
            };
            // Canonical slot 36 is raw player-inventory hotbar slot 0.
            let mut raw = vec![0];
            put_slot(&mut raw, &after);
            f.receive(ids::play_clientbound::SET_PLAYER_INVENTORY, &raw)
                .await;
            let complete = client
                .survival()
                .selected_drop_record()
                .await
                .unwrap()
                .unwrap();
            assert_eq!(complete.stage, Stage::ObservedDecreased);
            assert_eq!(complete.id, record.id);
            assert_eq!(
                complete.main_receipt.as_ref().unwrap().value,
                record.expected_after
            );
            assert!(
                matches!(complete.main_receipt.as_ref().unwrap().source, crate::client::ValueSource::Received { sequence } if sequence > record.send.after_sequence)
            );
            drain.wait().await.unwrap();
            f.slot(36, InventorySlot::Empty).await;
            assert_eq!(
                serde_json::to_value(client.survival().selected_drop_record().await.unwrap())
                    .unwrap(),
                serde_json::to_value(Some(complete)).unwrap()
            );
            assert!(
                timeout(Duration::from_millis(10), read_packet(&mut f.peer, None))
                    .await
                    .is_err()
            );
        }
    }
    f.stop().await;
}
#[tokio::test]
async fn selected_drop_cancelled_before_writer_never_completes_or_resends() {
    let mut f = fixture().await;
    let client = f.client();
    let session = f.api.bot.session.clone();
    let writer = session.writer.lock().await;
    let ops = client.survival();
    let waiter = tokio::spawn(async move { ops.drop_selected(Amount::One).await });
    timeout(Duration::from_secs(1), async {
        while session.state.try_lock().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    drop(writer);
    f.slot(36, plain("dirt", 11)).await;
    let record = client
        .survival()
        .selected_drop_record()
        .await
        .unwrap()
        .unwrap();
    assert!(!record.send.dispatched);
    assert_eq!(record.stage, Stage::RequiresInspection);
    assert!(client.survival().drop_selected(Amount::One).await.is_err());
    assert!(session.ledger.cut(false).wait().await.is_err());
    assert!(
        timeout(Duration::from_millis(10), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    f.stop().await;
}

#[tokio::test]
async fn selected_drop_partial_write_keeps_original_intent_and_failed_ledger() {
    let f = fixture().await;
    let client = f.client();
    let control = crate::client::ledger::testing::Control::default();
    f.api
        .bot
        .session
        .writer
        .lock()
        .await
        .stream
        .test_replace(control.writer());
    control.release(1, false);
    let ops = client.survival();
    let waiter = tokio::spawn(async move { ops.drop_selected(Amount::One).await });
    control.until(|w| w.bytes().len() == 1).await;
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    let record = client
        .survival()
        .selected_drop_record()
        .await
        .unwrap()
        .unwrap();
    assert!(!record.send.dispatched);
    assert_eq!(record.stage, Stage::RequiresInspection);
    assert!(client.survival().drop_selected(Amount::One).await.is_err());
    assert!(f.api.bot.session.ledger.cut(false).wait().await.is_err());
    assert_eq!(control.bytes().len(), 1);
    f.stop().await;
}

#[tokio::test]
async fn selected_drop_conflicts_are_sticky_and_do_not_replay() {
    for conflict in [
        "selection",
        "mode",
        "cursor",
        "consumed_two",
        "data",
        "world",
        "screen",
        "native_owner",
    ] {
        let mut f = fixture().await;
        let client = f.client();
        client.survival().drop_selected(Amount::One).await.unwrap();
        read_packet(&mut f.peer, None).await.unwrap();
        read_resync(&mut f.peer).await;
        match conflict {
            "selection" => {
                f.receive(ids::play_clientbound::HELD_ITEM_SLOT, &[1]).await;
                f.receive(ids::play_clientbound::HELD_ITEM_SLOT, &[0]).await;
            }
            "mode" => {
                f.receive(
                    ids::play_clientbound::GAME_STATE_CHANGE,
                    &[3, 63, 128, 0, 0],
                )
                .await;
                f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &[3, 0, 0, 0, 0])
                    .await;
            }
            "cursor" => {
                let mut cursor = Vec::new();
                put_slot(&mut cursor, &plain("stone", 1));
                f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &cursor)
                    .await;
                f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &[0])
                    .await;
            }
            "consumed_two" => f.slot(36, plain("dirt", 10)).await,
            "data" => f.slot(36, plain("stone", 11)).await,
            "world" | "screen" | "native_owner" => {
                let mut state = f.api.bot.session.state.lock().await;
                match conflict {
                    "world" => state.loading.generation += 1,
                    "screen" => state.operations.inventory.window_id = Some(2),
                    _ => state.operations.inventory.unsupported_components = true,
                }
                super::super::drop::context_received(&mut state);
            }
            _ => unreachable!(),
        }
        f.slot(36, plain("dirt", 11)).await;
        let record = client
            .survival()
            .selected_drop_record()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.stage, Stage::RequiresInspection, "{conflict}");
        assert!(client.survival().drop_selected(Amount::All).await.is_err());
        assert!(f.api.bot.session.ledger.cut(false).wait().await.is_err());
        assert!(
            timeout(Duration::from_millis(10), read_packet(&mut f.peer, None))
                .await
                .is_err()
        );
        f.stop().await;
    }
}

#[tokio::test]
async fn selected_drop_admission_rejects_empty_unknown_mode_and_competing_owner() {
    for reason in [
        "empty",
        "selection",
        "slot",
        "cursor",
        "adventure",
        "spectator",
        "wrong_handle",
        "click",
        "hand_swap",
        "native_swap",
    ] {
        let mut f = fixture().await;
        let client = f.client();
        match reason {
            "empty" => f.slot(36, InventorySlot::Empty).await,
            "selection" => {
                f.api
                    .bot
                    .session
                    .state
                    .lock()
                    .await
                    .operations
                    .selected_hotbar = None
            }
            "slot" => {
                f.api
                    .bot
                    .session
                    .state
                    .lock()
                    .await
                    .operations
                    .inventory
                    .slots[36] = InventorySlot::Unavailable
            }
            "cursor" => {
                f.api
                    .bot
                    .session
                    .state
                    .lock()
                    .await
                    .operations
                    .inventory
                    .cursor = InventorySlot::Unavailable
            }
            "adventure" | "spectator" | "wrong_handle" => {
                let mut packet = vec![3];
                packet.extend(
                    (match reason {
                        "adventure" => 2f32,
                        "spectator" => 3f32,
                        _ => 1f32,
                    })
                    .to_be_bytes(),
                );
                f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &packet)
                    .await;
            }
            "click" => {
                client
                    .survival()
                    .click_inventory(InventorySource::Player, 9, InventoryClickButton::Left)
                    .await
                    .unwrap();
                read_packet(&mut f.peer, None).await.unwrap();
            }
            "hand_swap" => {
                client.survival().swap_hands().await.unwrap();
                read_packet(&mut f.peer, None).await.unwrap();
            }
            "native_swap" => {
                f.api.swap_player_hotbar(9, 0).await.unwrap();
                read_packet(&mut f.peer, None).await.unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            client.survival().drop_selected(Amount::One).await.is_err(),
            "{reason}"
        );
        assert!(
            client
                .survival()
                .selected_drop_record()
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            timeout(Duration::from_millis(10), read_packet(&mut f.peer, None))
                .await
                .is_err()
        );
        f.stop().await;
    }
}

#[tokio::test]
async fn selected_drop_last_hotbar_stale_receipt_and_completed_disconnect_history() {
    let mut f = fixture().await;
    let client = f.client();
    client.survival().select_hotbar(8).await.unwrap();
    read_packet(&mut f.peer, None).await.unwrap();
    f.slot(44, plain("dirt", 1)).await;
    let mut record = client.survival().drop_selected(Amount::One).await.unwrap();
    assert_eq!(record.main_slot, 44);
    read_packet(&mut f.peer, None).await.unwrap();
    read_resync(&mut f.peer).await;
    let mut stale = record.initial.clone();
    stale.inventory.slots[44] = Some(crate::client::received(
        record.expected_after.clone(),
        record.send.after_sequence,
    ));
    let owner = client.server_registry_state().await.unwrap();
    crate::client::inventory::drop::receive(&mut record, &stale, &owner);
    assert_eq!(record.stage, Stage::RequiresInspection);
    assert!(record.main_receipt.is_none());
    f.slot(44, InventorySlot::Empty).await;
    let complete = client
        .survival()
        .selected_drop_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(complete.stage, Stage::ObservedDecreased);
    f.stop().await;
    assert_eq!(
        serde_json::to_value(
            client
                .survival()
                .selected_drop_record()
                .await
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(complete).unwrap()
    );
}

#[tokio::test]
async fn selected_drop_preserves_components_and_rejects_changed_data() {
    for conflict in [false, true] {
        let mut f = fixture().await;
        let client = f.client();
        let registry =
            crate::client::registry::Registry::for_version(MinecraftVersion::Java1_21_11);
        let name = registry.item_component("minecraft:custom_name").unwrap();
        // Construct a supported custom-name patch on the selected stack.
        let mut raw = Vec::new();
        put_varint(&mut raw, 12);
        put_varint(
            &mut raw,
            registry.item("minecraft:dirt").unwrap().id.value(),
        );
        raw.extend([1, 0]);
        put_varint(&mut raw, name.id.value());
        raw.extend([8, 0, 4]); // unnamed NBT string, length 4
        raw.extend(b"drop");
        let mut packet = vec![0, 1, 0, 36];
        packet.extend(&raw);
        f.receive(ids::play_clientbound::SET_SLOT, &packet).await;
        let record = client.survival().drop_selected(Amount::One).await.unwrap();
        assert!(
            matches!(&record.main_before.value, crate::client::SlotKnowledge::Item { item } if item.data != crate::client::ItemData::Default)
        );
        read_packet(&mut f.peer, None).await.unwrap();
        read_resync(&mut f.peer).await;
        if conflict {
            f.slot(36, plain("dirt", 11)).await;
        }
        raw[0] = 11;
        let mut packet = vec![0, 2, 0, 36];
        packet.extend(&raw);
        f.receive(ids::play_clientbound::SET_SLOT, &packet).await;
        assert_eq!(
            client
                .survival()
                .selected_drop_record()
                .await
                .unwrap()
                .unwrap()
                .stage,
            if conflict {
                Stage::RequiresInspection
            } else {
                Stage::ObservedDecreased
            }
        );
        f.stop().await;
    }
}

#[tokio::test]
async fn selected_drop_item_entity_and_pickup_are_not_acknowledgements() {
    let mut f = fixture().await;
    let client = f.client();
    client.survival().drop_selected(Amount::One).await.unwrap();
    read_packet(&mut f.peer, None).await.unwrap();
    read_resync(&mut f.peer).await;
    let mut spawn = vec![43];
    spawn.extend([7; 16]);
    put_varint(
        &mut spawn,
        client
            .registry()
            .builtin_id("minecraft:entity_type", "minecraft:item")
            .unwrap()
            .value(),
    );
    for v in [9.0f64, 65.0, 8.5] {
        spawn.extend(v.to_be_bytes());
    }
    spawn.extend([0; 5]);
    f.receive(ids::play_clientbound::SPAWN_ENTITY, &spawn).await;
    let mut pickup = vec![43];
    put_varint(&mut pickup, 1);
    put_varint(&mut pickup, 1);
    f.receive(ids::play_clientbound::COLLECT, &pickup).await;
    let record = client
        .survival()
        .selected_drop_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.stage, Stage::Pending);
    assert!(record.main_receipt.is_none());
    f.stop().await;
}

#[tokio::test]
async fn selected_drop_full_contents_checks_cursor_before_completing() {
    for conflict in [false, true] {
        let mut f = fixture().await;
        let client = f.client();
        let mut cursor = Vec::new();
        put_slot(&mut cursor, &plain("stone", 7));
        f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &cursor)
            .await;
        client.survival().drop_selected(Amount::One).await.unwrap();
        read_packet(&mut f.peer, None).await.unwrap();
        read_resync(&mut f.peer).await;
        let mut inventory = f
            .api
            .bot
            .session
            .state
            .lock()
            .await
            .operations
            .inventory
            .clone();
        inventory.slots[36] = plain("dirt", 11);
        if conflict {
            inventory.cursor = InventorySlot::Empty;
        }
        f.receive(ids::play_clientbound::WINDOW_ITEMS, &full(&inventory, 131))
            .await;
        let record = client
            .survival()
            .selected_drop_record()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            record.stage,
            if conflict {
                Stage::RequiresInspection
            } else {
                Stage::ObservedDecreased
            }
        );
        if conflict {
            inventory.cursor = plain("stone", 7);
            f.receive(ids::play_clientbound::WINDOW_ITEMS, &full(&inventory, 132))
                .await;
            assert_eq!(
                client
                    .survival()
                    .selected_drop_record()
                    .await
                    .unwrap()
                    .unwrap()
                    .stage,
                Stage::RequiresInspection
            );
        }
        f.stop().await;
    }
}

#[tokio::test]
async fn selected_drop_rejects_unreceived_invalid_and_actively_consumed_predecessors() {
    let f = fixture().await;
    let client = f.client();
    let initial = client.player_state().await.unwrap();
    let owner = client.server_registry_state().await.unwrap();
    for reason in [
        "unreceived",
        "future",
        "zero",
        "over_capacity",
        "using_item",
    ] {
        let mut before = initial.clone();
        let slot = before.inventory.slots[36].as_mut().unwrap();
        match reason {
            "unreceived" => slot.source = crate::client::ValueSource::LocalCache,
            "future" => {
                slot.source = crate::client::ValueSource::Received {
                    sequence: before.receive_sequence + 1,
                }
            }
            "zero" | "over_capacity" => {
                let crate::client::SlotKnowledge::Item { item } = &mut slot.value else {
                    unreachable!()
                };
                item.count = if reason == "zero" { 0 } else { 100 };
            }
            "using_item" => {
                before.using_item = Some(crate::client::received(
                    Some(crate::client::Hand::Main),
                    before.receive_sequence,
                ))
            }
            _ => unreachable!(),
        }
        assert!(
            crate::client::inventory::drop::prepare(
                before,
                owner.clone(),
                GameMode::Survival,
                1,
                1,
                Amount::One
            )
            .is_err(),
            "{reason}"
        );
    }
    let mut record = client.survival().drop_selected(Amount::One).await.unwrap();
    let mut current = record.initial.clone();
    current.using_item = Some(crate::client::received(
        Some(crate::client::Hand::Main),
        current.receive_sequence,
    ));
    crate::client::inventory::drop::receive(&mut record, &current, &owner);
    assert_eq!(record.stage, Stage::RequiresInspection);
    current.using_item = None;
    crate::client::inventory::drop::receive(&mut record, &current, &owner);
    assert_eq!(record.stage, Stage::RequiresInspection);
    f.stop().await;
}

#[tokio::test]
async fn selected_drop_cancelled_during_resync_retains_complete_drop_without_replay() {
    let f = fixture().await;
    let client = f.client();
    let control = crate::client::ledger::testing::Control::default();
    f.api
        .bot
        .session
        .writer
        .lock()
        .await
        .stream
        .test_replace(control.writer());
    // Exactly the full drop frame: length, packet ID, 11-byte action payload.
    control.release(13, true);
    let ops = client.survival();
    let waiter = tokio::spawn(async move { ops.drop_selected(Amount::One).await });
    control
        .until(|w| w.bytes().len() == 13 && w.progress().2 == 1 && w.progress().0 >= 2)
        .await;
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    let record = client
        .survival()
        .selected_drop_record()
        .await
        .unwrap()
        .unwrap();
    assert!(record.send.dispatched);
    assert!(!record.send.resync_dispatched);
    assert_eq!(record.stage, Stage::RequiresInspection);
    assert!(client.survival().drop_selected(Amount::One).await.is_err());
    assert_eq!(control.bytes().len(), 13);
    assert!(f.api.bot.session.ledger.cut(false).wait().await.is_err());
    f.stop().await;
}
