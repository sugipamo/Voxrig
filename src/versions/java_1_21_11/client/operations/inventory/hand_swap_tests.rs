use super::*;
use crate::client::inventory::{HandSwapStage as Stage, InventoryClickButton, InventorySource};

async fn fixture() -> CommonFixture {
    let mut f = CommonFixture::new().await;
    crate::client::inventory::armor_tests::install(
        &mut f.api.bot.session.state.lock().await.registries,
        MinecraftVersion::Java1_21_11,
        None,
    );
    // Original builtin item/entity tags plus the original mining block tags are
    // needed by shield repairable and pickaxe tool component constructors.
    let owner = f.client().server_registry_state().await.unwrap();
    let mut tags = owner.tags().unwrap().value.as_ref().clone();
    let mut raw = Vec::new();
    std::io::Read::read_to_end(
        &mut flate2::read::GzDecoder::new(
            &include_bytes!("../../../../../../data/client_api/mining_tools-1.21.11.json.gz")[..],
        ),
        &mut raw,
    )
    .unwrap();
    let raw: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    tags.insert(
        "minecraft:block".into(),
        serde_json::from_value(raw["block_tags"].clone()).unwrap(),
    );
    let mut payload = Vec::new();
    let name = |payload: &mut Vec<u8>, value: &str| {
        put_varint(payload, value.len() as i32);
        payload.extend(value.as_bytes());
    };
    put_varint(&mut payload, tags.len() as i32);
    for (registry, tags) in tags {
        name(&mut payload, &registry);
        put_varint(&mut payload, tags.len() as i32);
        for (tag, members) in tags {
            name(&mut payload, &tag);
            put_varint(&mut payload, members.len() as i32);
            for id in members {
                put_varint(&mut payload, id);
            }
        }
    }
    f.api
        .bot
        .session
        .state
        .lock()
        .await
        .registries
        .receive_tags(&payload, 4, MinecraftVersion::Java1_21_11)
        .unwrap();
    f.receive(ids::play_clientbound::HELD_ITEM_SLOT, &[0]).await;
    f.slot(45, plain("shield", 1)).await;
    f
}

#[tokio::test]
async fn hand_swap_both_modes_two_fresh_destinations_shared_sequence_and_history() {
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
        // Hand exchange never clicks the cursor: preserve an actual held stack.
        let mut cursor = Vec::new();
        put_slot(&mut cursor, &plain("stone", 7));
        f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &cursor)
            .await;
        for main in [
            plain("iron_pickaxe", 1),
            InventorySlot::Empty,
            plain("dirt", 2),
        ] {
            f.slot(36, main.clone()).await;
            f.slot(45, plain("shield", 1)).await;
            let previous_sequence = f.api.next_sequence().unwrap();
            let record = if mode == GameMode::Survival {
                client.survival().swap_hands().await
            } else {
                client.creative().swap_hands().await
            }
            .unwrap();
            assert_eq!(record.stage, Stage::Pending);
            assert!(record.send.dispatched);
            assert_eq!(record.send.interaction_sequence, previous_sequence + 1);
            assert_eq!(record.main_slot, 36);
            let mut expected = vec![6, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            put_varint(&mut expected, previous_sequence + 1);
            assert_eq!(
                read_packet(&mut f.peer, None).await.unwrap(),
                (ids::play_serverbound::BLOCK_DIG, expected)
            );
            let mut drain = f.api.bot.session.ledger.cut(false);
            assert!(drain.inspect().is_none());
            assert!(client.survival().swap_hands().await.is_err());
            assert!(client.survival().select_hotbar(1).await.is_err());
            assert!(
                client
                    .survival()
                    .click_inventory(InventorySource::Player, 9, InventoryClickButton::Left)
                    .await
                    .is_err()
            );
            assert!(f.api.swap_player_hotbar(9, 0).await.is_err());
            // Offhand first, raw Inventory hotbar second; unrelated slot cannot complete.
            f.slot(45, main).await;
            f.slot(9, plain("stone", 4)).await;
            let pending = client.survival().hand_swap_record().await.unwrap().unwrap();
            assert_eq!(pending.stage, Stage::Pending);
            assert!(pending.offhand_receipt.is_some() && pending.main_receipt.is_none());
            let mut raw = vec![0];
            put_slot(&mut raw, &plain("shield", 1));
            f.receive(ids::play_clientbound::SET_PLAYER_INVENTORY, &raw)
                .await;
            let complete = client.creative().hand_swap_record().await.unwrap().unwrap();
            assert_eq!(complete.id, record.id);
            assert_eq!(complete.stage, Stage::ObservedSwapped);
            assert_eq!(
                complete.main_receipt.as_ref().unwrap().value,
                record.offhand_before.value
            );
            assert_eq!(
                complete.offhand_receipt.as_ref().unwrap().value,
                record.main_before.value
            );
            assert_eq!(complete.cursor_before.value, record.cursor_before.value);
            drain.wait().await.unwrap();
            f.slot(36, InventorySlot::Empty).await;
            let history = client.survival().hand_swap_record().await.unwrap().unwrap();
            assert_eq!(
                serde_json::to_value(history).unwrap(),
                serde_json::to_value(complete).unwrap()
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
async fn hand_swap_keeps_component_patch_and_requires_exact_data() {
    let mut f = fixture().await;
    let client = f.client();
    // Construct a supported damage patch on the shield; no default-stack conversion.
    let mut bytes = Vec::new();
    put_varint(&mut bytes, 1); // count
    let shield = default_item("shield", 1).unwrap();
    put_varint(&mut bytes, shield.item_id);
    bytes.extend([1, 0]); // one added component, none removed
    let damage = crate::client::registry::Registry::for_version(MinecraftVersion::Java1_21_11)
        .item_component("minecraft:damage")
        .unwrap();
    put_varint(&mut bytes, damage.id.value());
    put_varint(&mut bytes, 17);
    let mut packet = vec![0, 1, 0, 45];
    packet.extend(&bytes);
    f.receive(ids::play_clientbound::SET_SLOT, &packet).await;
    let native = f
        .api
        .bot
        .session
        .state
        .lock()
        .await
        .operations
        .inventory
        .slots[45]
        .clone();
    let record = client.survival().swap_hands().await.unwrap();
    assert!(
        matches!(&record.offhand_before.value, crate::client::SlotKnowledge::Item { item } if item.data != crate::client::ItemData::Default)
    );
    read_packet(&mut f.peer, None).await.unwrap();
    f.slot(36, native.clone()).await;
    assert_eq!(
        client
            .survival()
            .hand_swap_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        Stage::Pending
    );
    f.slot(45, plain("dirt", 12)).await;
    assert_eq!(
        client
            .survival()
            .hand_swap_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        Stage::ObservedSwapped
    );
    // A second attempt cannot accept a default shield as the damaged destination.
    let second = client.survival().swap_hands().await.unwrap();
    assert_ne!(second.id, record.id);
    read_packet(&mut f.peer, None).await.unwrap();
    f.slot(45, plain("shield", 1)).await;
    f.slot(45, native).await;
    f.slot(36, plain("dirt", 12)).await;
    assert_eq!(
        client
            .survival()
            .hand_swap_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        Stage::RequiresInspection
    );
    f.stop().await;
}

#[tokio::test]
async fn hand_swap_context_conflicts_are_sticky_after_restoration() {
    for conflict in [
        "selection",
        "mode",
        "cursor",
        "slot",
        "world",
        "screen",
        "returned_slot",
    ] {
        let mut f = fixture().await;
        let client = f.client();
        client.survival().swap_hands().await.unwrap();
        read_packet(&mut f.peer, None).await.unwrap();
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
            "slot" => {
                f.slot(36, plain("stone", 1)).await;
            }
            "returned_slot" => {
                f.slot(36, plain("shield", 1)).await;
                f.slot(36, plain("dirt", 12)).await;
            }
            "world" | "screen" => {
                // These are the same state boundaries the packet handlers update.
                let mut state = f.api.bot.session.state.lock().await;
                if conflict == "world" {
                    state.loading.generation += 1;
                } else {
                    state.operations.inventory.window_id = Some(2);
                }
                super::super::hands::context_received(&mut state);
            }
            _ => unreachable!(),
        }
        f.slot(36, plain("shield", 1)).await;
        f.slot(45, plain("dirt", 12)).await;
        let record = client.survival().hand_swap_record().await.unwrap().unwrap();
        assert_eq!(record.stage, Stage::RequiresInspection, "{conflict}");
        assert!(client.survival().swap_hands().await.is_err());
        assert!(
            timeout(Duration::from_millis(10), read_packet(&mut f.peer, None))
                .await
                .is_err()
        );
        f.stop().await;
    }
}

#[tokio::test]
async fn hand_swap_cancelled_before_writer_never_completes_or_resends() {
    let mut f = fixture().await;
    let client = f.client();
    let session = f.api.bot.session.clone();
    let writer = session.writer.lock().await;
    let ops = client.survival();
    let waiter = tokio::spawn(async move { ops.swap_hands().await });
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
    f.slot(36, plain("shield", 1)).await;
    f.slot(45, plain("dirt", 12)).await;
    let record = client.survival().hand_swap_record().await.unwrap().unwrap();
    assert!(!record.send.dispatched);
    assert_eq!(record.stage, Stage::RequiresInspection);
    assert!(client.survival().swap_hands().await.is_err());
    assert!(session.ledger.cut(false).wait().await.is_err());
    assert!(
        timeout(Duration::from_millis(10), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    f.stop().await;
}

#[tokio::test]
async fn hand_swap_partial_write_keeps_original_intent_and_failed_ledger() {
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
    let waiter = tokio::spawn(async move { ops.swap_hands().await });
    control.until(|w| w.bytes().len() == 1).await;
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    let record = client.survival().hand_swap_record().await.unwrap().unwrap();
    assert!(!record.send.dispatched);
    assert_eq!(record.stage, Stage::RequiresInspection);
    assert!(client.survival().swap_hands().await.is_err());
    assert!(f.api.bot.session.ledger.cut(false).wait().await.is_err());
    assert_eq!(control.bytes().len(), 1);
    f.stop().await;
}

#[tokio::test]
async fn hand_swap_rejects_missing_identical_and_competing_predecessors_before_io() {
    for reason in ["selection", "slot", "identical", "mode", "click"] {
        let mut f = fixture().await;
        let client = f.client();
        match reason {
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
                    .slots[45] = InventorySlot::Unavailable
            }
            "identical" => f.slot(45, plain("dirt", 12)).await,
            "mode" => {
                f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &[3, 64, 0, 0, 0])
                    .await
            }
            "click" => {
                client
                    .survival()
                    .click_inventory(InventorySource::Player, 9, InventoryClickButton::Left)
                    .await
                    .unwrap();
                read_packet(&mut f.peer, None).await.unwrap();
            }
            _ => unreachable!(),
        }
        assert!(client.survival().swap_hands().await.is_err(), "{reason}");
        assert!(
            client
                .survival()
                .hand_swap_record()
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
async fn hand_swap_selected_last_hotbar_and_stale_destination_cannot_confirm() {
    let mut f = fixture().await;
    let client = f.client();
    // Selection is a completely submitted local command, not a server echo.
    client.survival().select_hotbar(8).await.unwrap();
    read_packet(&mut f.peer, None).await.unwrap();
    f.slot(44, plain("dirt", 1)).await;
    let mut record = client.survival().swap_hands().await.unwrap();
    assert_eq!(record.main_slot, 44);
    read_packet(&mut f.peer, None).await.unwrap();
    let mut stale = record.initial.clone();
    stale.inventory.slots[44] = Some(crate::client::received(
        record.offhand_before.value.clone(),
        record.send.after_sequence,
    ));
    stale.inventory.slots[45] = Some(crate::client::received(
        record.main_before.value.clone(),
        record.send.after_sequence,
    ));
    let owner = client.server_registry_state().await.unwrap();
    crate::client::inventory::hands::receive(&mut record, &stale, &owner);
    assert_eq!(record.stage, Stage::RequiresInspection);
    assert!(record.main_receipt.is_none() && record.offhand_receipt.is_none());
    // The real owned record still requires both fresh actual packets.
    f.slot(44, plain("shield", 1)).await;
    assert_eq!(
        client
            .survival()
            .hand_swap_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        Stage::Pending
    );
    f.slot(45, plain("dirt", 1)).await;
    let complete = client.survival().hand_swap_record().await.unwrap().unwrap();
    assert_eq!(complete.stage, Stage::ObservedSwapped);
    f.stop().await;
    assert_eq!(
        serde_json::to_value(client.survival().hand_swap_record().await.unwrap().unwrap()).unwrap(),
        serde_json::to_value(complete).unwrap()
    );
}

#[tokio::test]
async fn hand_swap_after_local_close_and_actual_reconfiguration_keep_owned_history() {
    let mut f = fixture().await;
    let client = f.client();
    let opening = f.open_swap_container().await;
    assert!(client.survival().swap_hands().await.is_err());
    let closed = client.survival().close_container(opening).await.unwrap();
    read_packet(&mut f.peer, None).await.unwrap();
    let record = client.survival().swap_hands().await.unwrap();
    assert!(matches!(record.initial.inventory.player_screen,
        Some(crate::client::container::PlayerScreenAccess::SubmittedClose { close }) if close == closed.id));
    read_packet(&mut f.peer, None).await.unwrap();
    f.slot(36, plain("shield", 1)).await;
    f.slot(45, plain("dirt", 2)).await;
    assert_eq!(
        client
            .survival()
            .hand_swap_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        Stage::ObservedSwapped
    );
    let next = client.survival().swap_hands().await.unwrap();
    read_packet(&mut f.peer, None).await.unwrap();
    f.receive(ids::play_clientbound::START_CONFIGURATION, &[])
        .await;
    let latest = client.survival().hand_swap_record().await.unwrap().unwrap();
    assert_eq!(latest.id, next.id);
    assert_eq!(latest.stage, Stage::RequiresInspection);
    assert!(client.survival().swap_hands().await.is_err());
    f.stop().await;
}
