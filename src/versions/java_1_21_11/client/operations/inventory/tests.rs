use super::*;
#[test]
fn pickup_cursor_comparison_encoding_matches_original_native_codec() {
    let cases: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../data/client_api/regular_click_packets-1.21.11.json"
    ))
    .unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 120);
    for case in cases.as_array().unwrap() {
        let cursor = if let Some(name) = case["cursor_comparison"]["item"].as_str() {
            let definition = crate::client::registry::Registry::for_version(
                crate::MinecraftVersion::Java1_21_11,
            )
            .item(name)
            .unwrap();
            InventorySlot::Item {
                item: PlainItem {
                    name: definition.name,
                    item_id: definition.id.value(),
                    count: case["cursor_comparison"]["count"]
                        .as_i64()
                        .unwrap()
                        .try_into()
                        .unwrap(),
                },
            }
        } else {
            InventorySlot::Empty
        };
        let mut payload = Vec::new();
        put_varint(
            &mut payload,
            case["window"].as_i64().unwrap().try_into().unwrap(),
        );
        put_varint(
            &mut payload,
            case["revision"].as_i64().unwrap().try_into().unwrap(),
        );
        payload.extend((case["slot"].as_i64().unwrap() as i16).to_be_bytes());
        payload.extend([case["button"].as_u64().unwrap() as u8, 0, 0]); // PICKUP, no predicted modified-slot hashes
        put_default_cursor_hash(&mut payload, &cursor).unwrap();
        assert_eq!(hex::encode(payload), case["payload_hex"].as_str().unwrap());
    }
    for cursor in [
        InventorySlot::Unavailable,
        InventorySlot::Item {
            item: PlainItem {
                name: "minecraft:stone".into(),
                item_id: 0,
                count: 1,
            },
        },
        InventorySlot::Item {
            item: PlainItem {
                name: "minecraft:stone".into(),
                item_id: 1,
                count: 0,
            },
        },
    ] {
        let mut bytes = vec![42];
        assert!(put_default_cursor_hash(&mut bytes, &cursor).is_err());
        assert_eq!(bytes, vec![42]);
    }
}
#[tokio::test]
async fn player_screen_after_close_reconfiguration_clears_player_revision_and_local_basis() {
    let mut f = CommonFixture::new().await;
    let id = f.open_swap_container().await;
    let client = f.client();
    let close = client.survival().close_container(id).await.unwrap();
    read_packet(&mut f.peer, None).await.unwrap();
    f.receive(ids::play_clientbound::START_CONFIGURATION, &[])
        .await;
    {
        let state = f.api.bot.session.state.lock().await;
        assert_ne!(
            state.loading.generation,
            close.initial.session.world_generation
        );
        assert!(state.operations.inventory.player_revision.is_none());
        assert!(state.operations.inventory.container.is_none());
    }
    let current = client.player_state().await.unwrap();
    assert_ne!(current.session, close.initial.session);
    assert!(current.inventory.player_screen.is_none());
    assert!(current.inventory.player_screen_revision.is_none());
    assert!(client.survival().swap_hotbar(9, 0).await.is_err());
    assert_eq!(
        client
            .survival()
            .container_close_record()
            .await
            .unwrap()
            .unwrap()
            .id,
        close.id
    );
    f.stop().await;
}
#[tokio::test]
async fn player_screen_after_close_uses_same_consumer_and_actual_player_revision_not_container_revision()
 {
    use crate::client::{GameMode as Mode, container::PlayerScreenAccess};
    let mut f = CommonFixture::new().await;
    let id = f.open_swap_container().await;
    let client = f.client();
    let close = client.survival().close_container(id).await.unwrap();
    assert_eq!(
        read_packet(&mut f.peer, None).await.unwrap(),
        (ids::play_serverbound::CLOSE_WINDOW, vec![3])
    );
    crate::client::tests::common_closed_player_screen_scenario(&client, close.id).await;
    let first =
        crate::client::tests::common_swap_start_scenario(&client, Mode::Survival, 9, 0).await;
    assert_eq!(first.send.screen_revision, Some(128));
    let revision = first
        .initial
        .inventory
        .player_screen_revision
        .as_ref()
        .unwrap();
    assert_eq!(revision.value, 128);
    assert!(
        matches!(revision.source,crate::client::ValueSource::Received {sequence} if sequence < id.opened_sequence())
    );
    assert_eq!(
        read_packet(&mut f.peer, None).await.unwrap(),
        (
            ids::play_serverbound::WINDOW_CLICK,
            vec![0, 128, 1, 0, 9, 0, 2, 0, 0]
        )
    );
    f.slot(9, plain("dirt", 2)).await;
    crate::client::tests::common_swap_pending_scenario(&client).await;
    f.slot(36, InventorySlot::Empty).await;
    crate::client::tests::common_swap_completed_scenario(&client, first.id).await;
    crate::client::tests::common_closed_player_screen_scenario(&client, close.id).await;
    let updated = client.player_state().await.unwrap();
    assert_eq!(
        updated
            .inventory
            .player_screen_revision
            .as_ref()
            .unwrap()
            .value,
        130
    );
    // SET_SLOT receipts did not invent active received player window zero.
    assert_eq!(updated.inventory.window_id, Some(3));
    let mut mode = vec![3];
    mode.extend(1f32.to_be_bytes());
    f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &mode)
        .await;
    let second =
        crate::client::tests::common_swap_start_scenario(&client, Mode::Creative, 9, 0).await;
    assert_eq!(second.send.screen_revision, Some(130));
    assert_eq!(read_packet(&mut f.peer, None).await.unwrap().1[0], 0);
    f.slot(9, InventorySlot::Empty).await;
    f.slot(36, plain("dirt", 2)).await;
    crate::client::tests::common_swap_completed_scenario(&client, second.id).await;
    assert!(
        matches!(client.screen_state().await.unwrap().player_screen,Some(PlayerScreenAccess::SubmittedClose { close: owning }) if owning==close.id)
    );
    client.disconnect().await.unwrap();
    assert_eq!(
        client
            .creative()
            .inventory_swap_record()
            .await
            .unwrap()
            .unwrap()
            .id,
        second.id
    );
    f.stop().await;
}
#[tokio::test]
async fn player_screen_after_close_latches_a_new_opening_before_matching_restoration() {
    use crate::client::inventory::InventorySwapStage;
    let mut f = CommonFixture::new().await;
    let id = f.open_swap_container().await;
    let client = f.client();
    client.survival().close_container(id).await.unwrap();
    read_packet(&mut f.peer, None).await.unwrap();
    let first = client.survival().swap_hotbar(9, 0).await.unwrap();
    read_packet(&mut f.peer, None).await.unwrap();
    f.open_swap_container().await;
    assert!(
        client
            .player_state()
            .await
            .unwrap()
            .inventory
            .player_screen
            .is_none()
    );
    f.slot(9, plain("dirt", 2)).await;
    f.slot(36, InventorySlot::Empty).await;
    assert_eq!(
        client
            .survival()
            .inventory_swap_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        InventorySwapStage::RequiresInspection
    );
    assert!(client.survival().swap_hotbar(9, 0).await.is_err());
    assert_eq!(
        client
            .survival()
            .inventory_swap_record()
            .await
            .unwrap()
            .unwrap()
            .id,
        first.id
    );
    f.stop().await;
}
#[tokio::test]
async fn player_screen_after_close_never_uses_foreign_revision_when_player_revision_missing() {
    let mut f = CommonFixture::new().await;
    let id = f.open_swap_container().await;
    let client = f.client();
    f.api
        .bot
        .session
        .state
        .lock()
        .await
        .operations
        .inventory
        .player_revision = None;
    client.survival().close_container(id).await.unwrap();
    read_packet(&mut f.peer, None).await.unwrap();
    assert!(
        client
            .player_state()
            .await
            .unwrap()
            .inventory
            .player_screen_revision
            .is_none()
    );
    assert_eq!(
        client
            .screen_state()
            .await
            .unwrap()
            .screen
            .unwrap()
            .revision
            .unwrap()
            .value,
        5
    );
    assert!(client.survival().swap_hotbar(9, 0).await.is_err());
    assert!(
        client
            .survival()
            .inventory_swap_record()
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    f.stop().await;
}
#[tokio::test]
async fn common_container_close_refuses_missing_cursor_unreceived_health_and_pending_swap() {
    let mut f = CommonFixture::new().await;
    let client = f.client();
    f.receive(ids::play_clientbound::OPEN_WINDOW, &[3, 2, 10, 0])
        .await;
    let unknown = client.screen_state().await.unwrap();
    assert!(unknown.cursor.is_none());
    assert!(
        client
            .survival()
            .close_container(unknown.screen.unwrap().id)
            .await
            .is_err()
    );
    assert!(
        client
            .survival()
            .container_close_record()
            .await
            .unwrap()
            .is_none()
    );
    let id = f.open_swap_container().await;
    let mut cursor = Vec::new();
    put_slot(&mut cursor, &plain("stone", 1));
    f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &cursor)
        .await;
    assert!(client.survival().close_container(id).await.is_err());
    assert!(
        client
            .survival()
            .container_close_record()
            .await
            .unwrap()
            .is_none()
    );
    f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &[0])
        .await;
    client
        .survival()
        .swap_container_hotbar(id, 0, 0)
        .await
        .unwrap();
    assert_eq!(
        read_packet(&mut f.peer, None).await.unwrap().0,
        ids::play_serverbound::WINDOW_CLICK
    );
    assert!(client.survival().close_container(id).await.is_err());
    assert!(
        client
            .survival()
            .container_close_record()
            .await
            .unwrap()
            .is_none()
    );
    f.stop().await;
}

#[tokio::test]
async fn common_container_close_retains_dispatch_and_only_matches_original_opening_reply() {
    use crate::client::{GameMode as Mode, container::ContainerCloseStage};
    let mut f = CommonFixture::new().await;
    let id = f.open_swap_container().await;
    let client = f.client();
    let first =
        crate::client::tests::common_container_close_scenario(&client, Mode::Survival, id).await;
    assert_eq!(
        read_packet(&mut f.peer, None).await.unwrap(),
        (ids::play_serverbound::CLOSE_WINDOW, vec![3])
    );
    assert_eq!(client.screen_state().await.unwrap().screen.unwrap().id, id);
    f.receive(ids::play_clientbound::CLOSE_WINDOW, &[4]).await;
    assert_eq!(
        client
            .survival()
            .container_close_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        ContainerCloseStage::Dispatched
    );
    f.receive(ids::play_clientbound::CLOSE_WINDOW, &[3]).await;
    let observed = client
        .survival()
        .container_close_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(observed.stage, ContainerCloseStage::ObservedClosed);
    assert!(observed.server_close_sequence.unwrap() > first.initial.receive_sequence);
    let new = f.open_swap_container().await;
    assert_ne!(new, id);
    let mut mode = vec![3];
    mode.extend(1f32.to_be_bytes());
    f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &mode)
        .await;
    let second =
        crate::client::tests::common_container_close_scenario(&client, Mode::Creative, new).await;
    assert_eq!(second.id.attempt(), first.id.attempt() + 1);
    assert_eq!(
        read_packet(&mut f.peer, None).await.unwrap(),
        (ids::play_serverbound::CLOSE_WINDOW, vec![3])
    );
    let newest = f.open_swap_container().await;
    assert_ne!(newest, new);
    f.receive(ids::play_clientbound::CLOSE_WINDOW, &[3]).await;
    assert_eq!(
        client
            .creative()
            .container_close_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        ContainerCloseStage::Dispatched
    );
    client.disconnect().await.unwrap();
    assert_eq!(
        client
            .creative()
            .container_close_record()
            .await
            .unwrap()
            .unwrap()
            .id,
        second.id
    );
    f.stop().await;
}
#[tokio::test]
async fn common_container_close_cancelled_modern_waiter_keeps_one_owned_write_and_prompt_history() {
    use crate::client::container::ContainerCloseStage;
    let mut f = CommonFixture::new().await;
    let id = f.open_swap_container().await;
    let client = f.client();
    let session = f.api.bot.session.clone();
    let writer = session.writer.lock().await;
    let ops = client.survival();
    let waiter = tokio::spawn(async move { ops.close_container(id).await });
    timeout(Duration::from_secs(1), async {
        while session.state.try_lock().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let retained = timeout(
        Duration::from_millis(100),
        client.survival().container_close_record(),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    assert_eq!(retained.id.screen(), id);
    assert_eq!(retained.stage, ContainerCloseStage::Pending);
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    drop(writer);
    assert_eq!(
        timeout(Duration::from_secs(1), read_packet(&mut f.peer, None))
            .await
            .unwrap()
            .unwrap(),
        (ids::play_serverbound::CLOSE_WINDOW, vec![3])
    );
    timeout(Duration::from_secs(1), async {
        while !client
            .survival()
            .container_close_record()
            .await
            .unwrap()
            .unwrap()
            .dispatched
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(client.survival().close_container(id).await.is_err());
    f.receive(ids::play_clientbound::CLOSE_WINDOW, &[3]).await;
    let observed = client
        .survival()
        .container_close_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(observed.stage, ContainerCloseStage::ObservedClosed);
    assert!(
        timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    client.disconnect().await.unwrap();
    assert_eq!(
        client
            .survival()
            .container_close_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        ContainerCloseStage::ObservedClosed
    );
    f.stop().await;
}

impl CommonFixture {
    async fn open_swap_container(&mut self) -> crate::client::container::ScreenId {
        self.receive(ids::play_clientbound::OPEN_WINDOW, &[3, 2, 10, 0])
            .await;
        let mut full = vec![3, 5, 63];
        for index in 0..63 {
            put_slot(
                &mut full,
                &match index {
                    0 => plain("stone", 3),
                    54 => plain("dirt", 2),
                    _ => InventorySlot::Empty,
                },
            );
        }
        full.push(0);
        self.receive(ids::play_clientbound::WINDOW_ITEMS, &full)
            .await;
        self.client()
            .screen_state()
            .await
            .unwrap()
            .screen
            .unwrap()
            .id
    }
    async fn container_slot(&mut self, index: u16, value: InventorySlot) {
        let mut p = vec![3, 6];
        p.extend(index.to_be_bytes());
        put_slot(&mut p, &value);
        self.receive(ids::play_clientbound::SET_SLOT, &p).await;
    }
}
#[tokio::test]
async fn shulker_slot_refusal_is_shared_by_both_modes_before_owner_or_click() {
    use crate::client::GameMode as Mode;
    for mode in [Mode::Survival, Mode::Creative] {
        let mut f = CommonFixture::new().await;
        f.receive(ids::play_clientbound::OPEN_WINDOW, &[3, 20, 10, 0])
            .await;
        let mut full = vec![3, 5, 63];
        for index in 0..63 {
            put_slot(
                &mut full,
                &match index {
                    0 => plain("stone", 3),
                    54 => plain("white_shulker_box", 1),
                    _ => InventorySlot::Empty,
                },
            );
        }
        full.push(0);
        f.receive(ids::play_clientbound::WINDOW_ITEMS, &full).await;
        if mode == Mode::Creative {
            let mut p = vec![3];
            p.extend(1f32.to_be_bytes());
            f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &p)
                .await;
        }
        let client = f.client();
        let screen = client.screen_state().await.unwrap().screen.unwrap().id;
        crate::client::tests::common_refused_shulker_swap_scenario(&client, mode, screen).await;
        assert!(
            timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
                .await
                .is_err()
        );
        f.container_slot(54, plain("dirt", 2)).await;
        let record = match mode {
            Mode::Survival => client.survival().swap_container_hotbar(screen, 0, 0).await,
            Mode::Creative => client.creative().swap_container_hotbar(screen, 0, 0).await,
            _ => unreachable!(),
        }
        .unwrap();
        assert!(record.send.dispatched);
        assert_eq!(
            read_packet(&mut f.peer, None).await.unwrap().0,
            ids::play_serverbound::WINDOW_CLICK
        );
        client.disconnect().await.unwrap();
        f.stop().await;
    }
}
#[tokio::test]
async fn container_swap_same_consumer_checks_fresh_destinations_and_next_creative_exchange() {
    use crate::client::{GameMode as Mode, inventory::InventorySwapStage};
    let mut f = CommonFixture::new().await;
    let id = f.open_swap_container().await;
    let client = f.client();
    for slot in [54, 63] {
        assert!(
            client
                .survival()
                .swap_container_hotbar(id, slot, 0)
                .await
                .is_err()
        );
    }
    let record =
        crate::client::tests::common_container_swap_start_scenario(&client, Mode::Survival, id)
            .await;
    assert_eq!(
        read_packet(&mut f.peer, None).await.unwrap(),
        (
            ids::play_serverbound::WINDOW_CLICK,
            vec![3, 5, 0, 0, 0, 2, 0, 0]
        )
    );
    assert!(
        f.api
            .bot
            .session
            .state
            .lock()
            .await
            .operations
            .inventory
            .pending_swap
            .is_none()
    );
    f.container_slot(0, plain("dirt", 2)).await;
    crate::client::tests::common_swap_pending_scenario(&client).await;
    f.container_slot(54, plain("stone", 3)).await;
    crate::client::tests::common_swap_completed_scenario(&client, record.id).await;
    let mut mode = vec![3];
    mode.extend(1f32.to_be_bytes());
    f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &mode)
        .await;
    let next =
        crate::client::tests::common_container_swap_start_scenario(&client, Mode::Creative, id)
            .await;
    read_packet(&mut f.peer, None).await.unwrap();
    assert_ne!(next.id, record.id);
    f.container_slot(0, plain("stone", 3)).await;
    f.container_slot(54, plain("dirt", 2)).await;
    crate::client::tests::common_swap_completed_scenario(&client, next.id).await;
    client.disconnect().await.unwrap();
    assert_eq!(
        client
            .creative()
            .inventory_swap_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        InventorySwapStage::ObservedSwapped
    );
    f.stop().await;
}
#[tokio::test]
async fn container_swap_reopened_numeric_id_and_restored_values_cannot_confirm() {
    use crate::client::inventory::InventorySwapStage;
    let mut f = CommonFixture::new().await;
    let id = f.open_swap_container().await;
    let client = f.client();
    let record = client
        .survival()
        .swap_container_hotbar(id, 0, 0)
        .await
        .unwrap();
    read_packet(&mut f.peer, None).await.unwrap();
    let new = f.open_swap_container().await;
    assert_ne!(new, id);
    f.container_slot(0, plain("dirt", 2)).await;
    f.container_slot(54, plain("stone", 3)).await;
    let result = client
        .survival()
        .inventory_swap_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.id, record.id);
    assert_eq!(result.stage, InventorySwapStage::RequiresInspection);
    assert!(
        client
            .survival()
            .swap_container_hotbar(new, 0, 0)
            .await
            .is_err()
    );
    f.stop().await;
}
#[tokio::test]
async fn container_swap_cancelled_before_writer_never_completes_from_later_matching_slots() {
    use crate::client::inventory::InventorySwapStage;
    let mut f = CommonFixture::new().await;
    let id = f.open_swap_container().await;
    let client = f.client();
    let session = f.api.bot.session.clone();
    let writer = session.writer.lock().await;
    let ops = client.survival();
    let waiter = tokio::spawn(async move { ops.swap_container_hotbar(id, 0, 0).await });
    timeout(Duration::from_secs(1), async {
        while session.state.try_lock().is_ok() {
            tokio::task::yield_now().await
        }
    })
    .await
    .unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    drop(writer);
    f.container_slot(0, plain("dirt", 2)).await;
    f.container_slot(54, plain("stone", 3)).await;
    let result = client
        .survival()
        .inventory_swap_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.stage, InventorySwapStage::Pending);
    assert!(!result.send.dispatched);
    assert!(
        timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    client.disconnect().await.unwrap();
    assert_eq!(
        client
            .survival()
            .inventory_swap_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        InventorySwapStage::RequiresInspection
    );
    f.stop().await;
}

#[tokio::test]
async fn common_container_receives_native_content_and_reused_id_is_a_new_opening() {
    let mut f = CommonFixture::new().await;
    let client = f.client();
    f.receive(ids::play_clientbound::OPEN_WINDOW, &[3, 2, 10, 0])
        .await;
    let before = crate::client::tests::common_container_capture_scenario(&client, false).await;
    let mut full = vec![3, 5, 63];
    for index in 0..63 {
        put_slot(
            &mut full,
            &match index {
                0 => plain("stone", 3),
                27 => plain("dirt", 2),
                _ => InventorySlot::Empty,
            },
        );
    }
    full.push(0);
    f.receive(ids::play_clientbound::WINDOW_ITEMS, &full).await;
    assert_eq!(
        crate::client::tests::common_container_capture_scenario(&client, true).await,
        before
    );
    let screen = client.screen_state().await.unwrap().screen.unwrap();
    assert_eq!(screen.revision.as_ref().unwrap().value, 5);
    let mut raw = vec![0];
    put_slot(&mut raw, &plain("dirt", 7));
    f.receive(ids::play_clientbound::SET_PLAYER_INVENTORY, &raw)
        .await;
    let updated = client.screen_state().await.unwrap().screen.unwrap();
    assert!(
        matches!(&updated.slots[54],Some(v) if matches!(&v.value,crate::client::SlotKnowledge::Item{item} if item.count==7))
    );
    assert_eq!(updated.revision, screen.revision);
    f.receive(ids::play_clientbound::OPEN_WINDOW, &[3, 2, 10, 0])
        .await;
    let reopened = crate::client::tests::common_container_capture_scenario(&client, false).await;
    assert_ne!(reopened, before);
    let mut stale = vec![4, 8, 0, 0];
    put_slot(&mut stale, &plain("stone", 3));
    f.receive(ids::play_clientbound::SET_SLOT, &stale).await;
    crate::client::tests::common_container_capture_scenario(&client, false).await;
    f.receive(ids::play_clientbound::CLOSE_WINDOW, &[4]).await;
    assert_eq!(
        client.screen_state().await.unwrap().screen.unwrap().id,
        reopened
    );
    f.receive(ids::play_clientbound::CLOSE_WINDOW, &[3]).await;
    assert!(client.screen_state().await.unwrap().screen.is_none());
    f.stop().await;
}

#[test]
fn container_content_malformed_packets_are_atomic_and_do_not_guess_layout() {
    let mut state = baseline();
    super::super::receive(
        &mut state,
        ids::play_clientbound::OPEN_WINDOW,
        &[3, 2, 10, 0],
    )
    .unwrap();
    let mut full = vec![3, 5, 63];
    full.extend([0; 64]);
    for length in 0..full.len() {
        assert!(
            super::super::receive(
                &mut state,
                ids::play_clientbound::WINDOW_ITEMS,
                &full[..length]
            )
            .is_err()
        );
        let screen = state.operations.inventory.container.as_ref().unwrap();
        assert!(screen.full_contents_sequence.is_none());
        assert!(screen.slots.iter().all(Option::is_none));
    }
    let mut trailing = full.clone();
    trailing.push(0);
    assert!(
        super::super::receive(&mut state, ids::play_clientbound::WINDOW_ITEMS, &trailing).is_err()
    );
    let mut wrong = vec![3, 5, 62];
    wrong.extend([0; 63]);
    assert!(
        super::super::receive(&mut state, ids::play_clientbound::WINDOW_ITEMS, &wrong).is_err()
    );
    super::super::receive(&mut state, ids::play_clientbound::WINDOW_ITEMS, &full).unwrap();
    let mut partial = vec![3, 6, 0, 63];
    put_slot(&mut partial, &InventorySlot::Empty);
    assert!(super::super::receive(&mut state, ids::play_clientbound::SET_SLOT, &partial).is_err());
    assert_eq!(
        state
            .operations
            .inventory
            .container
            .as_ref()
            .unwrap()
            .revision
            .as_ref()
            .unwrap()
            .value,
        5
    );
    // An unaudited menu keeps actual contents without an appended-player mapping.
    // Equipment is independent same-world history, with its original receive ordinals.
    let equipment = [5, 6, 7, 8, 45].map(|i| {
        (
            i,
            state.operations.inventory.slots[i].clone(),
            state.operations.inventory.slot_sequences[i],
        )
    });
    super::super::receive(
        &mut state,
        ids::play_clientbound::OPEN_WINDOW,
        &[4, 127, 10, 0],
    )
    .unwrap();
    let mut unknown = vec![4, 1, 63];
    unknown.extend([0; 64]);
    super::super::receive(&mut state, ids::play_clientbound::WINDOW_ITEMS, &unknown).unwrap();
    let screen = state.operations.inventory.container.as_ref().unwrap();
    assert!(screen.layout.is_none());
    assert!(screen.menu_name.is_none());
    assert_eq!(screen.slots.len(), 63);
    for (index, value, sequence) in equipment {
        assert_eq!(state.operations.inventory.slots[index], value);
        assert_eq!(state.operations.inventory.slot_sequences[index], sequence);
    }
    for index in 0..46 {
        if ![5, 6, 7, 8, 45].contains(&index) {
            assert_eq!(
                state.operations.inventory.slots[index],
                InventorySlot::Unavailable
            );
            assert!(state.operations.inventory.slot_sequences[index].is_none());
        }
    }
    state.operations.reset_world(0).unwrap();
    assert!(state.operations.inventory.container.is_none());
}

fn plain(name: &str, count: u8) -> InventorySlot {
    InventorySlot::Item {
        item: default_item(name, count).unwrap(),
    }
}
fn put_slot(payload: &mut Vec<u8>, value: &InventorySlot) {
    match value {
        InventorySlot::Empty => payload.push(0),
        InventorySlot::Item { item } => {
            put_varint(payload, item.count);
            put_varint(payload, item.item_id);
            payload.extend([0, 0]);
        }
        InventorySlot::Unavailable => panic!("cannot encode unknown inventory"),
        InventorySlot::ItemWithComponents { item, components } => {
            put_varint(payload, item.count);
            put_varint(payload, item.item_id);
            put_varint(payload, components.added.len() as i32);
            put_varint(payload, components.removed.len() as i32);
            for field in &components.added {
                put_varint(payload, field.definition.id.value());
                payload.extend(&field.bytes);
            }
            for field in &components.removed {
                put_varint(payload, field.id.value());
            }
        }
    }
}
fn full(inventory: &Inventory, revision: i32) -> Vec<u8> {
    let mut payload = vec![0];
    put_varint(&mut payload, revision);
    payload.push(46);
    for value in &inventory.slots {
        put_slot(&mut payload, value);
    }
    put_slot(&mut payload, &inventory.cursor);
    payload
}
fn slot_update(index: u16, value: &InventorySlot, revision: i32) -> Vec<u8> {
    let mut payload = vec![0];
    put_varint(&mut payload, revision);
    payload.extend(index.to_be_bytes());
    put_slot(&mut payload, value);
    payload
}
fn baseline() -> State {
    let mut state = State {
        sequence: 10,
        ready: true,
        loading: loading::InteractionLoading::completed_fixture(),
        ..State::default()
    };
    state.operations.game_mode = Some(GameMode::Survival);
    let mut inventory = Inventory {
        cursor: InventorySlot::Empty,
        ..Default::default()
    };
    inventory.slots.fill(InventorySlot::Empty);
    inventory.slots[9] = plain("stone", 32);
    inventory.slots[36] = plain("dirt", 12);
    super::super::receive(
        &mut state,
        ids::play_clientbound::WINDOW_ITEMS,
        &full(&inventory, 128),
    )
    .unwrap();
    state
}

struct CommonFixture {
    api: Operations,
    peer: tokio::net::TcpStream,
    receiver: tokio::task::JoinHandle<()>,
}
#[tokio::test]
async fn common_item_data_same_consumer_preserves_native_patch_and_refuses_default_click_before_io()
{
    use crate::client::{
        ItemData,
        inventory::{InventoryClickButton, InventorySource},
    };
    let cases = super::super::component_tests::corpus();
    let mut fixture = CommonFixture::new().await;
    let client = fixture.client();
    for mode in [GameMode::Survival, GameMode::Creative] {
        let mut packet = vec![3];
        packet.extend(
            (if mode == GameMode::Creative {
                1_f32
            } else {
                0_f32
            })
            .to_be_bytes(),
        );
        fixture
            .receive(ids::play_clientbound::GAME_STATE_CHANGE, &packet)
            .await;
        for name in [
            "minecraft:custom_data",
            "minecraft:custom_name",
            "minecraft:damage",
        ] {
            let sample = cases["stacks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|s| s["name"] == name)
                .unwrap();
            let full = sample["packets"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["label"] == "player_full")
                .unwrap();
            fixture
                .receive(
                    ids::play_clientbound::WINDOW_ITEMS,
                    &hex::decode(full["payload_hex"].as_str().unwrap()).unwrap(),
                )
                .await;
            let bytes = hex::decode(sample["actual_patch_hex"].as_str().unwrap()).unwrap();
            let patch = crate::versions::java_1_21_11::item_components::read_patch(
                &mut Reader::new(&bytes),
            )
            .unwrap()
            .unwrap();
            crate::client::tests::common_item_data_scenario(
                &client,
                9,
                "minecraft:stone",
                3,
                ItemData::ModernComponents { patch },
            )
            .await;
            fixture
                .receive(ids::play_clientbound::SET_CURSOR_ITEM, &[0])
                .await;
            let result = if mode == GameMode::Creative {
                client
                    .creative()
                    .click_inventory(InventorySource::Player, 9, InventoryClickButton::Left)
                    .await
            } else {
                client
                    .survival()
                    .click_inventory(InventorySource::Player, 9, InventoryClickButton::Left)
                    .await
            };
            assert_eq!(result.unwrap_err().kind(), crate::ErrorKind::Unsupported);
            assert!(
                client
                    .survival()
                    .inventory_click_record()
                    .await
                    .unwrap()
                    .is_none()
            );
            assert!(
                timeout(
                    Duration::from_millis(15),
                    read_packet(&mut fixture.peer, None)
                )
                .await
                .is_err()
            );
        }
    }
    drop(client);
    fixture.stop().await;
}
impl CommonFixture {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let stream = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (peer, _) = listener.accept().await.unwrap();
        let (reader, writer) = stream.into_split();
        let mut state = baseline();
        state.phase = Phase::Play;
        let session = Arc::new(Session {
            id: 42,
            started: Instant::now(),
            writer: Mutex::new(Writer {
                stream: writer,
                compression: None,
            }),
            state: Mutex::new(state),
            changed: Notify::new(),
            cancel: Notify::new(),
            stopped: AtomicBool::new(false),
            interrupted_packet: AtomicI32::new(-1),
            limits: crate::client::ClientLimits::default(),
            interaction_sequence: AtomicI32::new(0),
        });
        let api = Operations {
            bot: Bot {
                crafting_take_history: {
                    let state = session.state.try_lock().expect("new session");
                    state.crafting_take_history.clone()
                },
                close_history: session
                    .state
                    .try_lock()
                    .expect("new session")
                    .close_history
                    .clone(),
                session: session.clone(),
                _lease: Arc::new(Lease(Arc::downgrade(&session))),
            },
        };
        let receiver = tokio::spawn(async move {
            session.run_receiver(reader).await;
        });
        Self {
            api,
            peer,
            receiver,
        }
    }
    fn client(&self) -> crate::Client {
        crate::Client::from_java_1_21_11(self.api.bot.clone())
    }
    async fn receive(&mut self, id: i32, payload: &[u8]) {
        let before = self.api.bot.session.state.lock().await.sequence;
        write_packet(&mut self.peer, None, id, payload)
            .await
            .unwrap();
        timeout(Duration::from_secs(1), async {
            while self.api.bot.session.state.lock().await.sequence == before {
                tokio::task::yield_now().await
            }
        })
        .await
        .unwrap();
    }
    async fn slot(&mut self, index: u16, value: InventorySlot) {
        self.receive(
            ids::play_clientbound::SET_SLOT,
            &slot_update(index, &value, 130),
        )
        .await;
    }
    async fn stop(self) {
        self.api.bot.disconnect().await.unwrap();
        self.receiver.await.unwrap();
    }
}
#[tokio::test]
async fn common_swap_same_consumer_receives_both_destinations_then_next_creative_empty_swap() {
    use crate::client::{GameMode as Mode, inventory::InventorySwapStage};
    let mut f = CommonFixture::new().await;
    let client = f.client();
    let record =
        crate::client::tests::common_swap_start_scenario(&client, Mode::Survival, 9, 0).await;
    assert_eq!(
        read_packet(&mut f.peer, None).await.unwrap(),
        (
            ids::play_serverbound::WINDOW_CLICK,
            vec![0, 128, 1, 0, 9, 0, 2, 0, 0]
        )
    );
    assert!(record.send.legacy_action.is_none() && record.legacy_reply.is_none());
    assert_eq!(record.send.screen_revision, Some(128));
    let submission = f
        .api
        .bot
        .session
        .state
        .lock()
        .await
        .operations
        .inventory
        .pending_swap
        .clone()
        .unwrap();
    assert!(
        f.api
            .wait_inventory_swap(&submission, Duration::from_millis(1))
            .await
            .is_err()
    );
    f.slot(9, submission.hotbar_before.clone()).await;
    crate::client::tests::common_swap_pending_scenario(&client).await;
    // Direct raw-player hotbar update maps to canonical player-screen slot 36.
    let mut raw = vec![0];
    put_slot(&mut raw, &submission.main_before);
    f.receive(ids::play_clientbound::SET_PLAYER_INVENTORY, &raw)
        .await;
    let complete = crate::client::tests::common_swap_completed_scenario(&client, record.id).await;
    assert!(complete.legacy_reply.is_none());
    f.slot(9, InventorySlot::Empty).await;
    assert_eq!(
        client
            .survival()
            .inventory_swap_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        InventorySwapStage::ObservedSwapped
    );
    let mut mode = vec![3];
    mode.extend(1f32.to_be_bytes());
    f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &mode)
        .await;
    let next =
        crate::client::tests::common_swap_start_scenario(&client, Mode::Creative, 9, 0).await;
    assert_ne!(next.id, record.id);
    read_packet(&mut f.peer, None).await.unwrap();
    let next_submission = f
        .api
        .bot
        .session
        .state
        .lock()
        .await
        .operations
        .inventory
        .pending_swap
        .clone()
        .unwrap();
    f.slot(9, next_submission.hotbar_before).await;
    f.slot(36, next_submission.main_before).await;
    crate::client::tests::common_swap_completed_scenario(&client, next.id).await;
    f.api.bot.disconnect().await.unwrap();
    assert_eq!(
        client
            .creative()
            .inventory_swap_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        InventorySwapStage::ObservedSwapped
    );
    f.receiver.await.unwrap();
}
#[tokio::test]
async fn common_swap_transient_destination_and_screen_conflicts_stay_latched() {
    use crate::client::inventory::InventorySwapStage;
    let mut f = CommonFixture::new().await;
    let client = f.client();
    client.survival().swap_hotbar(9, 0).await.unwrap();
    read_packet(&mut f.peer, None).await.unwrap();
    let submission = f
        .api
        .bot
        .session
        .state
        .lock()
        .await
        .operations
        .inventory
        .pending_swap
        .clone()
        .unwrap();
    f.slot(9, submission.hotbar_before.clone()).await;
    f.slot(9, submission.main_before.clone()).await;
    f.slot(9, submission.hotbar_before.clone()).await;
    f.slot(36, submission.main_before.clone()).await;
    let result = client
        .survival()
        .inventory_swap_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.stage, InventorySwapStage::RequiresInspection);
    assert!(
        result
            .requires_inspection
            .unwrap()
            .contains("changed again")
    );
    assert!(
        f.api
            .wait_inventory_swap(&submission, Duration::from_millis(1))
            .await
            .is_err()
    );
    assert!(f.api.select_hotbar(0).await.is_err());
    f.stop().await;
}
#[tokio::test]
async fn common_swap_cancelled_before_writer_cannot_confirm_from_other_actor_updates() {
    use crate::client::inventory::InventorySwapStage;
    let mut f = CommonFixture::new().await;
    let client = f.client();
    let session = f.api.bot.session.clone();
    let writer = session.writer.lock().await;
    let ops = client.survival();
    let waiter = tokio::spawn(async move { ops.swap_hotbar(9, 0).await });
    timeout(Duration::from_secs(1), async {
        while session.state.try_lock().is_ok() {
            tokio::task::yield_now().await
        }
    })
    .await
    .unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    drop(writer);
    let submission = session
        .state
        .lock()
        .await
        .operations
        .inventory
        .pending_swap
        .clone()
        .unwrap();
    assert!(
        !session
            .state
            .lock()
            .await
            .common_inventory_swap
            .as_ref()
            .unwrap()
            .record
            .send
            .dispatched
    );
    f.slot(9, submission.hotbar_before).await;
    f.slot(36, submission.main_before).await;
    let result = client
        .survival()
        .inventory_swap_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.stage, InventorySwapStage::Pending);
    assert!(!result.send.dispatched);
    assert!(
        timeout(Duration::from_millis(10), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    assert!(client.survival().swap_hotbar(9, 0).await.is_err());
    f.api.bot.disconnect().await.unwrap();
    assert_eq!(
        client
            .survival()
            .inventory_swap_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        InventorySwapStage::RequiresInspection
    );
    f.receiver.await.unwrap();
}
#[tokio::test]
async fn common_swap_requires_actual_empty_cursor_and_latches_transient_mode_change() {
    use crate::client::inventory::InventorySwapStage;
    let mut f = CommonFixture::new().await;
    let client = f.client();
    f.api
        .bot
        .session
        .state
        .lock()
        .await
        .operations
        .inventory
        .cursor_sequence = None;
    assert!(client.survival().swap_hotbar(9, 0).await.is_err());
    assert!(
        f.api
            .bot
            .session
            .state
            .lock()
            .await
            .common_inventory_swap
            .is_none()
    );
    assert!(
        timeout(Duration::from_millis(10), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &[0])
        .await;
    client.survival().swap_hotbar(9, 0).await.unwrap();
    read_packet(&mut f.peer, None).await.unwrap();
    let mut mode = vec![3];
    mode.extend(1f32.to_be_bytes());
    f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &mode)
        .await;
    mode[1..].copy_from_slice(&0f32.to_be_bytes());
    f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &mode)
        .await;
    let submission = f
        .api
        .bot
        .session
        .state
        .lock()
        .await
        .operations
        .inventory
        .pending_swap
        .clone()
        .unwrap();
    f.slot(9, submission.hotbar_before).await;
    f.slot(36, submission.main_before).await;
    assert_eq!(
        client
            .survival()
            .inventory_swap_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        InventorySwapStage::RequiresInspection
    );
    f.stop().await;
}

#[test]
fn player_cursor_revision_and_slot_updates_preserve_atomic_receive() {
    let mut state = baseline();
    assert_eq!(state.operations.inventory.window_id, Some(0));
    assert_eq!(state.operations.inventory.screen_revision, Some(128));
    assert_eq!(state.operations.inventory.cursor, InventorySlot::Empty);
    let update = slot_update(9, &plain("glass", 7), 129);
    for length in 0..update.len() {
        let before = serde_json::to_value(&state.operations.inventory).unwrap();
        assert!(
            super::super::receive(
                &mut state,
                ids::play_clientbound::SET_SLOT,
                &update[..length]
            )
            .is_err()
        );
        assert_eq!(
            serde_json::to_value(&state.operations.inventory).unwrap(),
            before
        );
    }
    state.sequence = 11;
    super::super::receive(&mut state, ids::play_clientbound::SET_SLOT, &update).unwrap();
    assert_eq!(state.operations.inventory.screen_revision, Some(129));
    assert_eq!(state.operations.inventory.slot_sequences[9], Some(11));
    let mut cursor = Vec::new();
    put_slot(&mut cursor, &plain("stone", 1));
    super::super::receive(&mut state, ids::play_clientbound::SET_CURSOR_ITEM, &cursor).unwrap();
    assert!(prepare(&state.operations.inventory, 1, 11, 9, 0).is_err());
    for malformed in [
        vec![0, 255, 255, 255, 255, 15, 46], // negative screen revision
        vec![0, 0, 129, 8],                  // excessive inventory count
        vec![0, 0, 45],                      // incorrect player slot count
        vec![0, 0, 255, 255, 0],             // negative slot
        vec![0, 0, 0, 46, 0],                // slot outside player inventory
    ] {
        let id = if malformed.len() == 5 {
            ids::play_clientbound::SET_SLOT
        } else {
            ids::play_clientbound::WINDOW_ITEMS
        };
        let before = serde_json::to_value(&state.operations.inventory).unwrap();
        assert!(super::super::receive(&mut state, id, &malformed).is_err());
        assert_eq!(
            serde_json::to_value(&state.operations.inventory).unwrap(),
            before
        );
    }
}

#[test]
fn swap_admission_never_turns_unknown_or_pending_state_into_permission() {
    let state = baseline();
    let baseline = state.operations.inventory;
    let (submission, payload) = prepare(&baseline, 42, 10, 9, 0).unwrap();
    assert_eq!(payload, [0, 128, 1, 0, 9, 0, 2, 0, 0]);
    let cases: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../data/java_1_21_11/inventory_swap_packets.json"
    ))
    .unwrap();
    for case in cases.as_array().unwrap() {
        let mut received = baseline.clone();
        received.screen_revision = Some(case["revision"].as_i64().unwrap() as i32);
        let main = case["main_slot"].as_u64().unwrap() as u8;
        let hotbar = case["hotbar"].as_u64().unwrap() as u8;
        received.slots[usize::from(main)] = plain("stone", 32);
        received.slots[36 + usize::from(hotbar)] = InventorySlot::Empty;
        assert_eq!(
            prepare(&received, 42, 10, main, hotbar).unwrap().1,
            hex::decode(case["payload_hex"].as_str().unwrap()).unwrap()
        );
    }
    assert_eq!(baseline.slots[9], plain("stone", 32));
    for (main, hotbar) in [(8, 0), (36, 0), (9, 9)] {
        assert_eq!(
            prepare(&baseline, 42, 10, main, hotbar).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
    }
    assert!(prepare(&baseline, 42, 10, 10, 1).is_err()); // two empty slots
    let mut changed = baseline.clone();
    changed.pending_swap = Some(submission);
    assert!(prepare(&changed, 42, 10, 9, 1).is_err());
    changed = baseline.clone();
    changed.screen_revision = None;
    assert!(prepare(&changed, 42, 10, 9, 0).is_err());
    changed = baseline.clone();
    changed.slots[9] = InventorySlot::Unavailable;
    assert!(prepare(&changed, 42, 10, 9, 0).is_err());
    changed = baseline.clone();
    changed.cursor = InventorySlot::Unavailable;
    assert!(prepare(&changed, 42, 10, 9, 0).is_err());
    changed = baseline.clone();
    if let InventorySlot::Item { item } = &mut changed.slots[9] {
        item.count = 96;
    }
    assert_eq!(
        prepare(&changed, 42, 10, 9, 0).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    changed = baseline;
    changed.unsupported_components = true;
    assert!(prepare(&changed, 42, 10, 9, 0).is_err());
}

#[test]
fn confirmation_requires_both_post_submission_destinations() {
    let mut state = baseline();
    let (submission, _) = prepare(&state.operations.inventory, 42, 10, 9, 0).unwrap();
    state.operations.inventory.pending_swap = Some(submission.clone());
    // Matching values without new per-slot packets cannot confirm an action.
    state.operations.inventory.slots.swap(9, 36);
    assert!(observed(&state.operations.inventory, &submission, 100).is_none());
    state.sequence = 11;
    super::super::receive(
        &mut state,
        ids::play_clientbound::SET_SLOT,
        &slot_update(9, &submission.hotbar_before, 129),
    )
    .unwrap();
    assert!(observed(&state.operations.inventory, &submission, 11).is_none());
    state.sequence = 12;
    super::super::receive(
        &mut state,
        ids::play_clientbound::SET_SLOT,
        &slot_update(36, &submission.main_before, 130),
    )
    .unwrap();
    let proof = observed(&state.operations.inventory, &submission, 12).unwrap();
    assert_eq!((proof.main_sequence, proof.hotbar_sequence), (11, 12));
    assert_eq!(proof.submission.connection_id, 42);
    // A foreign container invalidates the cursor/revision and keeps uncertainty.
    super::super::receive(
        &mut state,
        ids::play_clientbound::WINDOW_ITEMS,
        &[1, 0, 0, 0],
    )
    .unwrap();
    assert_eq!(
        state.operations.inventory.pending_swap.as_ref(),
        Some(&submission)
    );
    assert!(observed(&state.operations.inventory, &submission, 13).is_none());
    assert!(prepare(&state.operations.inventory, 42, 13, 9, 0).is_err());
}

#[test]
fn window_changes_and_unsupported_cursor_cannot_restore_stale_swap_authority() {
    let mut state = baseline();
    let full_player = full(&state.operations.inventory, 129);
    let open = [1, 0, 10, 0]; // container 1, type 0, empty title NBT
    super::super::receive(&mut state, ids::play_clientbound::OPEN_WINDOW, &open).unwrap();
    assert_eq!(state.operations.inventory.window_id, Some(1));
    super::super::receive(
        &mut state,
        ids::play_clientbound::WINDOW_ITEMS,
        &full_player,
    )
    .unwrap();
    assert_eq!(state.operations.inventory.window_id, Some(1));
    assert!(prepare(&state.operations.inventory, 42, 10, 9, 0).is_err());
    super::super::receive(&mut state, ids::play_clientbound::CLOSE_WINDOW, &[1]).unwrap();
    assert_eq!(state.operations.inventory.window_id, None);
    assert!(prepare(&state.operations.inventory, 42, 10, 9, 0).is_err());
    super::super::receive(
        &mut state,
        ids::play_clientbound::WINDOW_ITEMS,
        &full_player,
    )
    .unwrap();
    assert!(prepare(&state.operations.inventory, 42, 10, 9, 0).is_ok());
    let cases = super::super::component_tests::corpus();
    let original = cases["stacks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "minecraft:bundle_contents")
        .unwrap();
    let packet = original["packets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["label"] == "cursor")
        .unwrap();
    let cursor = hex::decode(packet["payload_hex"].as_str().unwrap()).unwrap();
    super::super::receive(&mut state, ids::play_clientbound::SET_CURSOR_ITEM, &cursor).unwrap();
    assert!(matches!(
        state.operations.inventory.cursor,
        InventorySlot::ItemWithComponents { .. }
    ));
    assert!(!state.operations.inventory.unsupported_components);
    assert!(prepare(&state.operations.inventory, 42, 10, 9, 0).is_err());
}

#[tokio::test]
async fn ordinary_click_uses_real_transport_and_timeout_never_resubmits() {
    use super::super::super::{Lease, Phase, Session, Writer};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let connection = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (mut peer, _) = listener.accept().await.unwrap();
    let (reader, writer) = connection.into_split();
    let mut state = baseline();
    state.phase = Phase::Play;
    let session = Arc::new(Session {
        id: 42,
        started: Instant::now(),
        writer: Mutex::new(Writer {
            stream: writer,
            compression: None,
        }),
        state: Mutex::new(state),
        changed: Notify::new(),
        cancel: Notify::new(),
        stopped: AtomicBool::new(false),
        interrupted_packet: AtomicI32::new(-1),
        limits: crate::client::ClientLimits::default(),
        interaction_sequence: AtomicI32::new(0),
    });
    let operations = Operations {
        bot: Bot {
            crafting_take_history: {
                let state = session.state.try_lock().expect("new session");
                state.crafting_take_history.clone()
            },
            close_history: session
                .state
                .try_lock()
                .expect("new session")
                .close_history
                .clone(),
            session: session.clone(),
            _lease: Arc::new(Lease(Arc::downgrade(&session))),
        },
    };
    let running = session.clone();
    let receiver = tokio::spawn(async move { running.receive_loop(reader).await });
    let submission = operations.swap_player_hotbar(9, 0).await.unwrap();
    let (id, payload) = read_packet(&mut peer, None).await.unwrap();
    assert_eq!(id, ids::play_serverbound::WINDOW_CLICK);
    assert_eq!(payload, [0, 128, 1, 0, 9, 0, 2, 0, 0]);
    assert_eq!(
        operations.player_state().await.unwrap().inventory.slots[9],
        submission.main_before
    );
    assert_eq!(
        operations
            .swap_player_hotbar(9, 1)
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::State
    );
    assert_eq!(
        operations.select_hotbar(1).await.unwrap_err().kind(),
        ErrorKind::State
    );
    assert_eq!(
        operations
            .use_on_block([0, 0, 0], crate::BlockFace::Up, [0.5; 3])
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::State
    );
    assert_eq!(
        operations
            .wait_inventory_swap(&submission, Duration::from_millis(5))
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::Timeout
    );
    assert!(
        operations
            .player_state()
            .await
            .unwrap()
            .inventory
            .pending_swap
            .is_some()
    );
    write_packet(
        &mut peer,
        None,
        ids::play_clientbound::SET_SLOT,
        &slot_update(9, &submission.hotbar_before, 129),
    )
    .await
    .unwrap();
    assert_eq!(
        operations
            .wait_inventory_swap(&submission, Duration::from_millis(10))
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::Timeout
    );
    write_packet(
        &mut peer,
        None,
        ids::play_clientbound::SET_SLOT,
        &slot_update(36, &submission.main_before, 130),
    )
    .await
    .unwrap();
    let proof = operations
        .wait_inventory_swap(&submission, Duration::from_secs(1))
        .await
        .unwrap();
    assert!(proof.hotbar_sequence > proof.main_sequence);
    assert!(
        operations
            .player_state()
            .await
            .unwrap()
            .inventory
            .pending_swap
            .is_none()
    );
    assert!(
        timeout(Duration::from_millis(10), read_packet(&mut peer, None))
            .await
            .is_err()
    );
    let reverse = operations.swap_player_hotbar(9, 0).await.unwrap();
    assert_eq!(
        read_packet(&mut peer, None).await.unwrap().0,
        ids::play_serverbound::WINDOW_CLICK
    );
    write_packet(
        &mut peer,
        None,
        ids::play_clientbound::SET_SLOT,
        &slot_update(36, &reverse.main_before, 131),
    )
    .await
    .unwrap();
    write_packet(
        &mut peer,
        None,
        ids::play_clientbound::SET_SLOT,
        &slot_update(9, &reverse.hotbar_before, 132),
    )
    .await
    .unwrap();
    operations
        .wait_inventory_swap(&reverse, Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(
        operations.player_state().await.unwrap().inventory.slots[9],
        submission.main_before
    );
    let mut foreign = reverse;
    foreign.connection_id = 99;
    assert_eq!(
        operations
            .wait_inventory_swap(&foreign, Duration::from_millis(1))
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::State
    );
    operations.bot.disconnect().await.unwrap();
    receiver.await.unwrap().unwrap();
}

impl CommonFixture {
    async fn seed_common_open(&mut self) {
        use super::super::survival::{LocalPlayerState, PlayerHealth, VelocitySample};
        use crate::versions::java_1_21_11::{state_id, world::Dimension};
        let mut state = self.api.bot.session.state.lock().await;
        state.position = Some([8.5, 65.0, 8.5]);
        state.operations.local_player = LocalPlayerState::spawned(42);
        state.operations.local_player.velocity = Some(VelocitySample {
            value: [0.0; 3],
            receive_sequence: 10,
        });
        state.operations.local_player.health = Some(PlayerHealth {
            health: 20.0,
            food: 20,
            saturation: 5.0,
            receive_sequence: 10,
        });
        let pose = super::super::super::motion::ReceivedPose {
            generation: state.loading.generation,
            receive_sequence: 10,
            position: state.position.unwrap(),
            rotation: state.rotation,
            velocity: Some([0.0; 3]),
        };
        state.motion.receive(pose);
        state.world.select_dimension(
            "minecraft:overworld".into(),
            Dimension::new(-64, 384).unwrap(),
        );
        for x in 0..16 {
            for z in 0..16 {
                for y in 63..73 {
                    state
                        .world
                        .seed_replay_cell([x, y, z], if y == 64 { 1 } else { 0 });
                }
            }
        }
        let chest = crate::NativeBlockState {
            name: "minecraft:chest".into(),
            properties: std::collections::BTreeMap::from([
                ("facing".into(), "north".into()),
                ("type".into(), "single".into()),
                ("waterlogged".into(), "false".into()),
            ]),
        };
        state
            .world
            .seed_replay_cell([8, 66, 11], state_id(&chest).unwrap());
        drop(state);
        self.slot(36, InventorySlot::Empty).await;
        self.receive(ids::play_clientbound::HELD_ITEM_SLOT, &[0])
            .await;
    }
    async fn common_open_contents(&mut self) {
        let mut full = vec![3, 5, 63];
        full.extend([0; 64]);
        self.receive(ids::play_clientbound::WINDOW_ITEMS, &full)
            .await;
    }
}
#[tokio::test]
async fn common_container_open_same_consumer_in_both_modes_needs_actual_contents_and_modern_ack() {
    use crate::client::{GameMode as Mode, container::ContainerOpenStage};
    let mut f = CommonFixture::new().await;
    f.seed_common_open().await;
    let client = f.client();
    let mut old = None;
    for mode in [Mode::Survival, Mode::Creative] {
        if mode == Mode::Creative {
            let mut p = vec![3];
            p.extend(1f32.to_be_bytes());
            f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &p)
                .await;
        }
        let record =
            crate::client::tests::common_open_start_scenario(&client, mode, [8, 66, 11]).await;
        assert_eq!(record.before_screen, old);
        let (id, packet) = read_packet(&mut f.peer, None).await.unwrap();
        assert_eq!(id, ids::play_serverbound::BLOCK_PLACE);
        let mut expected = vec![0];
        expected.extend(
            crate::BlockPos { x: 8, y: 66, z: 11 }
                .packed()
                .to_be_bytes(),
        );
        expected.push(2);
        for cursor in record.cursor {
            expected.extend(cursor.to_be_bytes());
        }
        expected.extend([0, 0]);
        put_varint(&mut expected, record.send.interaction_sequence.unwrap());
        assert_eq!(packet, expected);
        // A real identical own-position refresh has a new ordinal, not a new pose.
        let pose = record.initial.received_pose.as_ref().unwrap();
        let mut refresh = vec![77];
        for v in pose.position {
            refresh.extend(v.to_be_bytes());
        }
        for _ in 0..3 {
            refresh.extend(0f64.to_be_bytes());
        }
        for v in pose.rotation {
            refresh.extend(v.to_be_bytes());
        }
        refresh.extend(0u32.to_be_bytes());
        f.receive(ids::play_clientbound::POSITION, &refresh).await;
        assert_eq!(
            read_packet(&mut f.peer, None).await.unwrap(),
            (ids::play_serverbound::TELEPORT_CONFIRM, vec![77])
        );
        assert_eq!(
            read_packet(&mut f.peer, None).await.unwrap().0,
            ids::play_serverbound::POSITION_LOOK
        );
        let refreshed = client
            .survival()
            .container_open_record()
            .await
            .unwrap()
            .unwrap();
        assert!(refreshed.requires_inspection.is_none());
        assert_eq!(
            refreshed.initial.received_pose,
            record.initial.received_pose
        );
        assert!(
            client
                .player_state()
                .await
                .unwrap()
                .received_pose
                .unwrap()
                .receive_sequence
                > pose.receive_sequence
        );

        f.receive(ids::play_clientbound::OPEN_WINDOW, &[3, 2, 10, 0])
            .await;
        let pending = client
            .survival()
            .container_open_record()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(pending.stage, ContainerOpenStage::ObservedScreen);
        assert!(
            pending
                .observed_screen
                .unwrap()
                .full_contents_sequence
                .is_none()
        );
        assert!(pending.received_cursor.is_none());
        f.common_open_contents().await;
        assert_eq!(
            client
                .survival()
                .container_open_record()
                .await
                .unwrap()
                .unwrap()
                .stage,
            ContainerOpenStage::ObservedScreen
        );
        assert!(client.player_state().await.unwrap().pending_dispatch);
        let mut ack = Vec::new();
        put_varint(&mut ack, record.send.interaction_sequence.unwrap());
        f.receive(ids::play_clientbound::ACKNOWLEDGE_PLAYER_DIGGING, &ack)
            .await;
        let complete =
            crate::client::tests::common_open_completed_scenario(&client, record.id).await;
        let screen = complete.observed_screen.unwrap().id;
        assert_ne!(Some(screen), old);
        let close =
            crate::client::tests::common_container_close_scenario(&client, mode, screen).await;
        assert_eq!(
            read_packet(&mut f.peer, None).await.unwrap(),
            (ids::play_serverbound::CLOSE_WINDOW, vec![3])
        );
        crate::client::tests::common_closed_player_screen_scenario(&client, close.id).await;
        old = Some(screen);
    }
    client.disconnect().await.unwrap();
    let history = client
        .creative()
        .container_open_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(history.stage, ContainerOpenStage::ObservedContents);
    assert_eq!(history.observed_screen.unwrap().id, old.unwrap());
    f.stop().await;
}
#[tokio::test]
async fn common_container_open_reused_numeric_window_and_cursor_conflicts_latch_before_restoration()
{
    use crate::client::container::ContainerOpenStage;
    for replace in [true, false] {
        let mut f = CommonFixture::new().await;
        f.seed_common_open().await;
        let client = f.client();
        let record = client.survival().open_container([8, 66, 11]).await.unwrap();
        read_packet(&mut f.peer, None).await.unwrap();
        f.receive(ids::play_clientbound::OPEN_WINDOW, &[3, 2, 10, 0])
            .await;
        let first = client.screen_state().await.unwrap().screen.unwrap().id;
        if replace {
            f.receive(ids::play_clientbound::OPEN_WINDOW, &[3, 2, 10, 0])
                .await;
            assert_ne!(
                client.screen_state().await.unwrap().screen.unwrap().id,
                first
            );
        } else {
            let mut cursor = Vec::new();
            put_slot(&mut cursor, &plain("stone", 1));
            f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &cursor)
                .await;
        }
        f.common_open_contents().await;
        let mut ack = Vec::new();
        put_varint(&mut ack, record.send.interaction_sequence.unwrap());
        f.receive(ids::play_clientbound::ACKNOWLEDGE_PLAYER_DIGGING, &ack)
            .await;
        let history = client
            .survival()
            .container_open_record()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(history.stage, ContainerOpenStage::RequiresInspection);
        assert_eq!(history.observed_screen.unwrap().id, first);
        assert!(client.survival().open_container([8, 66, 11]).await.is_err());
        assert!(
            timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
                .await
                .is_err()
        );
        f.stop().await;
    }
}
#[tokio::test]
async fn common_container_open_cancelled_modern_write_retains_unsent_intent_without_replay() {
    use crate::client::container::ContainerOpenStage;
    let mut f = CommonFixture::new().await;
    f.seed_common_open().await;
    let client = f.client();
    let session = f.api.bot.session.clone();
    let writer = session.writer.lock().await;
    let ops = client.survival();
    let waiter = tokio::spawn(async move { ops.open_container([8, 66, 11]).await });
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
    let record = client
        .survival()
        .container_open_record()
        .await
        .unwrap()
        .unwrap();
    assert!(!record.send.dispatched);
    assert_eq!(record.stage, ContainerOpenStage::Pending);
    f.receive(ids::play_clientbound::OPEN_WINDOW, &[3, 2, 10, 0])
        .await;
    f.common_open_contents().await;
    let mut ack = Vec::new();
    put_varint(&mut ack, record.send.interaction_sequence.unwrap());
    f.receive(ids::play_clientbound::ACKNOWLEDGE_PLAYER_DIGGING, &ack)
        .await;
    let history = client
        .creative()
        .container_open_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(history.stage, ContainerOpenStage::RequiresInspection);
    assert!(!history.send.dispatched);
    assert!(client.survival().open_container([8, 66, 11]).await.is_err());
    assert!(
        timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    f.stop().await;
}
#[tokio::test]
async fn common_container_open_received_mode_conflict_survives_restoration_and_matching_outcome() {
    use crate::client::container::ContainerOpenStage;
    let mut f = CommonFixture::new().await;
    f.seed_common_open().await;
    let client = f.client();
    let record = client.survival().open_container([8, 66, 11]).await.unwrap();
    read_packet(&mut f.peer, None).await.unwrap();
    for mode in [1f32, 0f32] {
        let mut p = vec![3];
        p.extend(mode.to_be_bytes());
        f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &p)
            .await;
    }
    f.receive(ids::play_clientbound::OPEN_WINDOW, &[3, 2, 10, 0])
        .await;
    f.common_open_contents().await;
    let mut ack = Vec::new();
    put_varint(&mut ack, record.send.interaction_sequence.unwrap());
    f.receive(ids::play_clientbound::ACKNOWLEDGE_PLAYER_DIGGING, &ack)
        .await;
    assert_eq!(
        client
            .survival()
            .container_open_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        ContainerOpenStage::RequiresInspection
    );
    assert!(client.player_state().await.unwrap().pending_dispatch);
    f.stop().await;
}
#[test]
fn screen_open_close_preserves_only_same_world_received_equipment_with_original_ordinals() {
    let mut state = baseline();
    let original = state.operations.inventory.clone();
    for id in [
        ids::play_clientbound::OPEN_WINDOW,
        ids::play_clientbound::CLOSE_WINDOW,
    ] {
        state.sequence += 1;
        super::super::receive(
            &mut state,
            id,
            if id == ids::play_clientbound::OPEN_WINDOW {
                &[3, 2, 10, 0]
            } else {
                &[3]
            },
        )
        .unwrap();
        for index in [5, 6, 7, 8, 45] {
            assert_eq!(
                state.operations.inventory.slots[index],
                original.slots[index]
            );
            assert_eq!(
                state.operations.inventory.slot_sequences[index],
                original.slot_sequences[index]
            );
        }
        assert!(matches!(
            state.operations.inventory.slots[36],
            InventorySlot::Unavailable
        ));
        assert!(state.operations.inventory.slot_sequences[36].is_none());
        assert!(state.operations.inventory.cursor_sequence.is_none());
    }
    state.operations.reset_world(0).unwrap();
    for index in [5, 6, 7, 8, 45] {
        assert!(matches!(
            state.operations.inventory.slots[index],
            InventorySlot::Unavailable
        ));
        assert!(state.operations.inventory.slot_sequences[index].is_none());
    }
}

#[tokio::test]
async fn common_container_open_barrel_native_flag_change_is_distinct_from_facing_conflict() {
    use crate::client::container::ContainerOpenStage;
    for conflict in [false, true] {
        let mut f = CommonFixture::new().await;
        f.seed_common_open().await;
        let client = f.client();
        let mut barrel = crate::NativeBlockState {
            name: "minecraft:barrel".into(),
            properties: std::collections::BTreeMap::from([
                ("facing".into(), "north".into()),
                ("open".into(), "false".into()),
            ]),
        };
        let packet = |state: &crate::NativeBlockState| {
            let mut p = crate::BlockPos { x: 8, y: 66, z: 11 }
                .packed()
                .to_be_bytes()
                .to_vec();
            put_varint(
                &mut p,
                crate::versions::java_1_21_11::state_id(state).unwrap(),
            );
            p
        };
        f.receive(ids::play_clientbound::BLOCK_CHANGE, &packet(&barrel))
            .await;
        let record = client.survival().open_container([8, 66, 11]).await.unwrap();
        read_packet(&mut f.peer, None).await.unwrap();
        barrel.properties.insert(
            if conflict { "facing" } else { "open" }.into(),
            if conflict { "south" } else { "true" }.into(),
        );
        f.receive(ids::play_clientbound::BLOCK_CHANGE, &packet(&barrel))
            .await;
        if conflict {
            barrel.properties.insert("facing".into(), "north".into());
            f.receive(ids::play_clientbound::BLOCK_CHANGE, &packet(&barrel))
                .await;
        }
        f.receive(ids::play_clientbound::OPEN_WINDOW, &[3, 2, 10, 0])
            .await;
        f.common_open_contents().await;
        let mut ack = Vec::new();
        put_varint(&mut ack, record.send.interaction_sequence.unwrap());
        f.receive(ids::play_clientbound::ACKNOWLEDGE_PLAYER_DIGGING, &ack)
            .await;
        let history = client
            .survival()
            .container_open_record()
            .await
            .unwrap()
            .unwrap();
        if conflict {
            assert_eq!(history.stage, ContainerOpenStage::RequiresInspection);
            assert_eq!(
                history.target_state.unwrap().state.properties["facing"],
                "south"
            );
        } else {
            crate::client::tests::common_open_completed_scenario(&client, record.id).await;
            assert_eq!(
                history.target_state.unwrap().state.properties["open"],
                "true"
            );
            assert_eq!(record.target.state.properties["open"], "false");
        }
        f.stop().await;
    }
}

#[tokio::test]
async fn ordinary_pickup_both_modes_player_storage_and_appended_player_require_actual_cursor() {
    use crate::client::{
        GameMode as Mode,
        inventory::{
            InventoryClickButton as Button, InventoryClickSource as Source,
            InventoryClickStage as Stage,
        },
    };
    for mode in [Mode::Survival, Mode::Creative] {
        for storage_slot in [None, Some(0), Some(54)] {
            let mut f = CommonFixture::new().await;
            let source = if storage_slot.is_some() {
                Source::Container {
                    screen: f.open_swap_container().await,
                }
            } else {
                Source::Player
            };
            // A small real predecessor makes split, one-place and all-return distinct.
            if storage_slot.is_none() {
                f.slot(9, plain("stone", 3)).await;
            }
            if mode == Mode::Creative {
                let mut p = vec![3];
                p.extend(1f32.to_be_bytes());
                f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &p)
                    .await;
            }
            let client = f.client();
            let index = storage_slot.unwrap_or(9);
            for button in [
                if storage_slot == Some(54) {
                    Button::Left
                } else {
                    Button::Right
                },
                Button::Right,
                Button::Left,
            ] {
                let record = crate::client::tests::common_pickup_start_scenario(
                    &client, mode, source, index, button,
                )
                .await;
                let (id, p) = read_packet(&mut f.peer, None).await.unwrap();
                assert_eq!(id, ids::play_serverbound::WINDOW_CLICK);
                assert_eq!(p, super::click::payload(&record).unwrap());
                let native = |value: &crate::client::SlotKnowledge| match value {
                    crate::client::SlotKnowledge::Empty => InventorySlot::Empty,
                    crate::client::SlotKnowledge::Item { item } => {
                        plain(&item.name, item.count as u8)
                    }
                    _ => unreachable!(),
                };
                if storage_slot.is_some() {
                    f.container_slot(index, native(&record.prediction.source.value))
                        .await;
                } else {
                    f.slot(index, native(&record.prediction.source.value)).await;
                }
                assert_eq!(
                    client
                        .survival()
                        .inventory_click_record()
                        .await
                        .unwrap()
                        .unwrap()
                        .stage,
                    Stage::Pending
                );
                let mut p = vec![];
                put_slot(&mut p, &native(&record.prediction.cursor.value));
                f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &p).await;
                let complete =
                    crate::client::tests::common_pickup_complete_scenario(&client, record.id).await;
                assert!(complete.legacy_reply.is_none());
            }
            assert!(matches!(
                client
                    .player_state()
                    .await
                    .unwrap()
                    .inventory
                    .cursor
                    .unwrap()
                    .value,
                crate::client::SlotKnowledge::Empty
            ));
            f.stop().await;
            assert_eq!(
                client
                    .creative()
                    .inventory_click_record()
                    .await
                    .unwrap()
                    .unwrap()
                    .stage,
                Stage::ObservedClicked
            );
        }
    }
}
#[tokio::test]
async fn ordinary_pickup_cancelled_before_full_write_retains_unsent_intent_and_latches_later_values()
 {
    use crate::client::inventory::{
        InventoryClickButton as Button, InventoryClickSource as Source,
        InventoryClickStage as Stage,
    };
    let mut f = CommonFixture::new().await;
    let client = f.client();
    let session = f.api.bot.session.clone();
    let writer = session.writer.lock().await;
    let survival = client.survival();
    let mut send = Box::pin(survival.click_inventory(Source::Player, 9, Button::Left));
    std::future::poll_fn(|cx| {
        assert!(std::future::Future::poll(send.as_mut(), cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(send);
    drop(writer);
    let retained = client
        .survival()
        .inventory_click_record()
        .await
        .unwrap()
        .unwrap();
    assert!(!retained.send.dispatched);
    assert_eq!(retained.stage, Stage::Pending);
    assert!(
        client
            .survival()
            .click_inventory(Source::Player, 9, Button::Left)
            .await
            .is_err()
    );
    assert!(
        timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    f.slot(9, InventorySlot::Empty).await;
    let mut p = vec![];
    put_slot(&mut p, &plain("stone", 32));
    f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &p).await;
    let record = client
        .survival()
        .inventory_click_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.id, retained.id);
    assert!(!record.send.dispatched);
    assert_eq!(record.stage, Stage::RequiresInspection);
    assert!(record.source_receipt.is_none() && record.cursor_receipt.is_none());
    f.stop().await;
}
#[tokio::test]
async fn ordinary_pickup_native_bundle_override_refuses_before_record_or_io_and_default_remains_usable()
 {
    use crate::client::inventory::{
        InventoryClickButton as Button, InventoryClickSource as Source,
    };
    let mut f = CommonFixture::new().await;
    let client = f.client();
    let mut p = vec![];
    put_slot(&mut p, &plain("bundle", 1));
    f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &p).await;
    assert_eq!(
        client
            .survival()
            .click_inventory(Source::Player, 9, Button::Left)
            .await
            .unwrap_err()
            .kind(),
        crate::ErrorKind::Unsupported
    );
    assert!(
        client
            .survival()
            .inventory_click_record()
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &[0])
        .await;
    let record = client
        .survival()
        .click_inventory(Source::Player, 9, Button::Right)
        .await
        .unwrap();
    assert!(record.send.dispatched);
    assert_eq!(
        read_packet(&mut f.peer, None).await.unwrap().0,
        ids::play_serverbound::WINDOW_CLICK
    );
    f.stop().await;
}
#[tokio::test]
async fn ordinary_pickup_reused_numeric_opening_latches_conflict_even_when_expected_values_return()
{
    use crate::client::inventory::{
        InventoryClickButton as Button, InventoryClickSource as Source,
        InventoryClickStage as Stage,
    };
    let mut f = CommonFixture::new().await;
    let screen = f.open_swap_container().await;
    let client = f.client();
    let record = client
        .survival()
        .click_inventory(Source::Container { screen }, 0, Button::Right)
        .await
        .unwrap();
    read_packet(&mut f.peer, None).await.unwrap();
    let new = f.open_swap_container().await;
    assert_ne!(screen, new);
    f.container_slot(0, plain("stone", 1)).await;
    let mut p = vec![];
    put_slot(&mut p, &plain("stone", 2));
    f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &p).await;
    let retained = client
        .survival()
        .inventory_click_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained.id, record.id);
    assert_eq!(retained.stage, Stage::RequiresInspection);
    f.stop().await;
}
#[tokio::test]
async fn shift_transfer_same_consumer_both_modes_storage_player_equipment_and_old_cursor_ordinal() {
    use crate::client::{
        GameMode as Mode,
        inventory::{InventorySource as Source, InventoryTransferStage as Stage},
    };
    for mode in [Mode::Survival, Mode::Creative] {
        for scenario in [0, 1, 2, 3, 4] {
            let mut f = CommonFixture::new().await;
            let source = if scenario == 0 {
                Source::Container {
                    screen: f.open_swap_container().await,
                }
            } else {
                Source::Player
            };
            if scenario >= 2 {
                f.slot(
                    9,
                    match scenario {
                        2 => plain("diamond_helmet", 1),
                        3 => plain("carved_pumpkin", 7),
                        _ => plain("bundle", 1),
                    },
                )
                .await;
            }
            if mode == Mode::Creative {
                let mut p = vec![3];
                p.extend(1f32.to_be_bytes());
                f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &p)
                    .await;
            }
            let client = f.client();
            let record = crate::client::tests::common_transfer_start_scenario(
                &client,
                mode,
                source,
                if scenario == 0 { 0 } else { 9 },
            )
            .await;
            let (id, p) = read_packet(&mut f.peer, None).await.unwrap();
            assert_eq!(id, ids::play_serverbound::WINDOW_CLICK);
            assert_eq!(p, super::transfer::payload(&record).unwrap());
            if scenario == 0 {
                assert!(
                    record
                        .changed_slots
                        .iter()
                        .any(|s| s.slot == 62 && s.player_slot == Some(44))
                );
            }
            if scenario == 2 || scenario == 3 {
                assert!(record.changed_slots.iter().any(|s| s.slot == 5));
            }
            for (i, change) in record.changed_slots.iter().enumerate() {
                let value = match &change.prediction.value {
                    crate::client::SlotKnowledge::Empty => InventorySlot::Empty,
                    crate::client::SlotKnowledge::Item { item } => {
                        plain(&item.name, item.count as u8)
                    }
                    _ => unreachable!(),
                };
                if scenario == 0 {
                    f.container_slot(change.slot, value).await;
                } else {
                    f.slot(change.slot, value).await;
                }
                if i + 1 < record.changed_slots.len() {
                    assert_eq!(
                        client
                            .survival()
                            .inventory_transfer_record()
                            .await
                            .unwrap()
                            .unwrap()
                            .stage,
                        Stage::Pending
                    );
                }
            }
            let complete =
                crate::client::tests::common_transfer_complete_scenario(&client, record.id).await;
            assert_eq!(
                complete.cursor_inspected.unwrap().source,
                record.cursor_before.source
            );
            assert!(complete.legacy_reply.is_none() && complete.legacy_return_prediction.is_none());
            f.stop().await;
            assert_eq!(
                client
                    .creative()
                    .inventory_transfer_record()
                    .await
                    .unwrap()
                    .unwrap()
                    .stage,
                Stage::ObservedTransferred
            );
        }
    }
}

#[tokio::test]
async fn shift_transfer_cancelled_before_full_write_retains_intent_and_cannot_confirm_from_other_actor_values()
 {
    use crate::client::inventory::{InventorySource as Source, InventoryTransferStage as Stage};
    let mut f = CommonFixture::new().await;
    let client = f.client();
    let session = f.api.bot.session.clone();
    let writer = session.writer.lock().await;
    let survival = client.survival();
    let mut send = Box::pin(survival.transfer_inventory(Source::Player, 9));
    std::future::poll_fn(|cx| {
        assert!(std::future::Future::poll(send.as_mut(), cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(send);
    drop(writer);
    let retained = client
        .survival()
        .inventory_transfer_record()
        .await
        .unwrap()
        .unwrap();
    assert!(!retained.send.dispatched);
    assert_eq!(retained.stage, Stage::Pending);
    assert!(
        client
            .survival()
            .transfer_inventory(Source::Player, 9)
            .await
            .is_err()
    );
    assert!(
        timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    for c in &retained.changed_slots {
        let value = match &c.prediction.value {
            crate::client::SlotKnowledge::Empty => InventorySlot::Empty,
            crate::client::SlotKnowledge::Item { item } => plain(&item.name, item.count as u8),
            _ => unreachable!(),
        };
        f.slot(c.slot, value).await;
    }
    let current = client
        .survival()
        .inventory_transfer_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.id, retained.id);
    assert_eq!(current.stage, Stage::RequiresInspection);
    assert!(!current.send.dispatched);
    assert!(current.changed_slots.iter().all(|c| c.receipt.is_none()));
    f.stop().await;
}
#[tokio::test]
async fn shift_transfer_partial_merge_needs_both_actual_receipts_without_cursor_packet() {
    use crate::client::inventory::{InventorySource as Source, InventoryTransferStage as Stage};
    let mut f = CommonFixture::new().await;
    f.slot(36, plain("stone", 63)).await;
    for i in 37..45 {
        f.slot(i, plain("dirt", 64)).await;
    }
    let client = f.client();
    let record = client
        .survival()
        .transfer_inventory(Source::Player, 9)
        .await
        .unwrap();
    read_packet(&mut f.peer, None).await.unwrap();
    assert_eq!(record.changed_slots.len(), 2);
    f.slot(9, plain("stone", 31)).await;
    assert_eq!(
        client
            .survival()
            .inventory_transfer_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        Stage::Pending
    );
    f.slot(36, plain("stone", 64)).await;
    crate::client::tests::common_transfer_complete_scenario(&client, record.id).await;
    f.stop().await;
}
#[tokio::test]
async fn shift_transfer_reused_numeric_storage_opening_stays_uncertain_after_values_return() {
    use crate::client::inventory::{InventorySource as Source, InventoryTransferStage as Stage};
    let mut f = CommonFixture::new().await;
    let screen = f.open_swap_container().await;
    let client = f.client();
    let record = client
        .survival()
        .transfer_inventory(Source::Container { screen }, 0)
        .await
        .unwrap();
    read_packet(&mut f.peer, None).await.unwrap();
    assert_ne!(f.open_swap_container().await, screen);
    f.container_slot(0, InventorySlot::Empty).await;
    f.container_slot(62, plain("stone", 3)).await;
    let retained = client
        .survival()
        .inventory_transfer_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained.id, record.id);
    assert_eq!(retained.stage, Stage::RequiresInspection);
    f.stop().await;
}

#[tokio::test]
async fn cursor_return_close_same_consumer_both_modes_waits_for_each_actual_step_before_close() {
    use crate::client::{GameMode as Mode, inventory::InventoryClickStage};
    for mode in [Mode::Survival, Mode::Creative] {
        let mut f = CommonFixture::new().await;
        let screen = f.open_swap_container().await;
        let mut health = Vec::from(20f32.to_be_bytes());
        put_varint(&mut health, 20);
        health.extend(5f32.to_be_bytes());
        f.receive(ids::play_clientbound::UPDATE_HEALTH, &health)
            .await;
        f.container_slot(27, plain("stone", 63)).await;
        let mut cursor = Vec::new();
        put_slot(&mut cursor, &plain("stone", 5));
        f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &cursor)
            .await;
        if mode == Mode::Creative {
            let mut p = vec![3];
            p.extend(1f32.to_be_bytes());
            f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &p)
                .await;
        }
        let client = f.client();
        let waiter = crate::client::tests::common_cursor_close_start(&client, mode, screen);
        for index in 0..2 {
            let (id, payload) = timeout(Duration::from_secs(1), read_packet(&mut f.peer, None))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(id, ids::play_serverbound::WINDOW_CLICK);
            let record =
                crate::client::tests::common_cursor_close_retained(&client, index + 1).await;
            let step = &record.return_steps[index];
            assert_eq!(payload, super::click::payload(step).unwrap());
            assert_eq!(step.source_slot, 27 + index as u16);
            assert!(client.survival().swap_hotbar(9, 0).await.is_err());
            assert!(f.api.swap_player_hotbar(9, 0).await.is_err());
            assert!(
                timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
                    .await
                    .is_err()
            );
            f.container_slot(
                step.source_slot,
                plain("stone", if index == 0 { 64 } else { 4 }),
            )
            .await;
            assert!(
                timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
                    .await
                    .is_err()
            );
            let mut cursor = Vec::new();
            put_slot(
                &mut cursor,
                &if index == 0 {
                    plain("stone", 4)
                } else {
                    InventorySlot::Empty
                },
            );
            f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &cursor)
                .await;
        }
        assert_eq!(
            timeout(Duration::from_secs(1), read_packet(&mut f.peer, None))
                .await
                .unwrap()
                .unwrap(),
            (ids::play_serverbound::CLOSE_WINDOW, vec![3])
        );
        let complete = waiter.await.unwrap().unwrap();
        assert!(complete.dispatched);
        assert!(
            complete
                .return_steps
                .iter()
                .all(|s| s.stage == InventoryClickStage::ObservedClicked)
        );
        assert!(
            complete
                .return_steps
                .iter()
                .all(|s| s.source_receipt.is_some() && s.cursor_receipt.is_some())
        );
        assert!(complete.server_close_sequence.is_none());
        assert!(
            timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
                .await
                .is_err()
        );
        f.stop().await;
    }
}

#[test]
fn all_default_cursor_return_hashes_match_original_native_packet_codecs() {
    let cases: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../data/client_api/cursor_return_packets-1.21.11.json"
    ))
    .unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 3008);
    for case in cases.as_array().unwrap() {
        let item = default_item(case["item"].as_str().unwrap(), 1).unwrap();
        let mut payload = vec![3, 7];
        payload.extend((case["slot"].as_u64().unwrap() as i16).to_be_bytes());
        payload.extend([0, 0, 0]);
        put_default_cursor_hash(&mut payload, &InventorySlot::Item { item }).unwrap();
        assert_eq!(hex::encode(payload), case["payload_hex"].as_str().unwrap());
    }
}

#[tokio::test]
async fn actual_position_is_published_after_teleport_confirmation_before_normal_look() {
    let mut f = CommonFixture::new().await;
    let mut mode = vec![3];
    mode.extend(1f32.to_be_bytes());
    f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &mode)
        .await;
    let session = f.api.bot.session.clone();
    let writer = session.writer.lock().await;
    let mut position = vec![77];
    for v in [8.5f64, 65., 8.5, 0., 0., 0.] {
        position.extend(v.to_be_bytes());
    }
    for v in [0f32, 0.] {
        position.extend(v.to_be_bytes());
    }
    position.extend(0u32.to_be_bytes());
    write_packet(
        &mut f.peer,
        None,
        ids::play_clientbound::POSITION,
        &position,
    )
    .await
    .unwrap();
    timeout(Duration::from_secs(1), async {
        while session.state.try_lock().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let client = f.client();
    assert!(
        timeout(Duration::from_millis(20), client.player_state())
            .await
            .is_err()
    );
    let creative = client.creative();
    let look = tokio::spawn(async move { creative.look([4., 0.]).await });
    assert!(
        timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    drop(writer);
    assert_eq!(
        read_packet(&mut f.peer, None).await.unwrap(),
        (ids::play_serverbound::TELEPORT_CONFIRM, vec![77])
    );
    assert_eq!(
        read_packet(&mut f.peer, None).await.unwrap().0,
        ids::play_serverbound::POSITION_LOOK
    );
    assert_eq!(
        read_packet(&mut f.peer, None).await.unwrap().0,
        ids::play_serverbound::LOOK
    );
    look.await.unwrap().unwrap();
    assert_eq!(
        client
            .player_state()
            .await
            .unwrap()
            .received_pose
            .unwrap()
            .position,
        [8.5, 65., 8.5]
    );
    f.stop().await;
}

#[test]
fn component_observation_cannot_enter_native_default_swap_or_cursor_hash() {
    let corpus = super::super::component_tests::corpus();
    for sample in corpus["stacks"].as_array().unwrap() {
        let bytes = hex::decode(sample["stack_hex"].as_str().unwrap()).unwrap();
        let value = slot(&mut Reader::new(&bytes)).unwrap().unwrap();
        if !matches!(value, InventorySlot::ItemWithComponents { .. }) {
            continue;
        }
        let mut state = super::super::component_tests::baseline();
        let full = sample["packets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["label"] == "player_full")
            .unwrap();
        super::super::receive(
            &mut state,
            ids::play_clientbound::WINDOW_ITEMS,
            &hex::decode(full["payload_hex"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        state.sequence = 14;
        super::super::receive(&mut state, ids::play_clientbound::SET_CURSOR_ITEM, &[0]).unwrap();
        let result = prepare(&state.operations.inventory, 71, 14, 9, 0);
        assert_eq!(result.unwrap_err().kind(), crate::ErrorKind::Unsupported);
        assert!(state.operations.inventory.pending_swap.is_none());
        let mut bytes = vec![41];
        assert_eq!(
            put_default_cursor_hash(&mut bytes, &value)
                .unwrap_err()
                .kind(),
            crate::ErrorKind::Unsupported
        );
        assert_eq!(bytes, [41]);
    }
}

fn component_swap_item(count: i32, capacity: i32, extra: bool) -> InventorySlot {
    use crate::client::{ItemComponent, ItemComponentPatch, registry::Registry};
    let registry = Registry::for_version(MinecraftVersion::Java1_21_11);
    let mut bytes = Vec::new();
    put_varint(&mut bytes, capacity);
    let mut added = vec![ItemComponent {
        definition: registry.item_component("minecraft:max_stack_size").unwrap(),
        bytes,
    }];
    if extra {
        // Explicit prototype value; native equivalent to its omission.
        added.push(ItemComponent {
            definition: registry.item_component("minecraft:rarity").unwrap(),
            bytes: vec![0],
        });
    }
    InventorySlot::ItemWithComponents {
        item: PlainItem {
            name: "minecraft:stone".into(),
            item_id: registry.item("minecraft:stone").unwrap().id.value(),
            count,
        },
        components: ItemComponentPatch {
            added,
            removed: vec![],
        },
    }
}
#[tokio::test]
async fn data_swap_same_consumer_preserves_components_capacity_and_canonical_receipts_in_both_modes()
 {
    use crate::client::GameMode as Mode;
    let mut f = CommonFixture::new().await;
    f.api.bot.session.state.lock().await.registries.finish();
    let client = f.client();
    for mode in [Mode::Survival, Mode::Creative] {
        let mut packet = vec![3];
        packet.extend((if mode == Mode::Creative { 1f32 } else { 0f32 }).to_be_bytes());
        f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &packet)
            .await;
        // A real received70-count stack is valid with capacity99, beyond default64.
        f.slot(9, component_swap_item(70, 99, true)).await;
        f.slot(36, plain("dirt", 12)).await;
        let before = client.received_inventory().await.unwrap();
        let record = crate::client::tests::common_swap_start_scenario(&client, mode, 9, 0).await;
        let (id, payload) = read_packet(&mut f.peer, None).await.unwrap();
        assert_eq!(id, ids::play_serverbound::WINDOW_CLICK);
        assert!(payload.ends_with(&[0, 9, 0, 2, 0, 0]));
        f.slot(9, plain("dirt", 12)).await;
        crate::client::tests::common_swap_pending_scenario(&client).await;
        f.slot(36, component_swap_item(70, 99, false)).await;
        let complete =
            crate::client::tests::common_data_swap_completed_scenario(&client, record.id, &before)
                .await;
        assert_ne!(
            complete.hotbar_receipt.unwrap().value,
            record.source_before.value
        );
    }
    f.stop().await;
}
#[tokio::test]
async fn data_swap_refuses_effective_overstack_and_latches_changed_component_receipts() {
    use crate::client::inventory::InventorySwapStage;
    let mut f = CommonFixture::new().await;
    f.api.bot.session.state.lock().await.registries.finish();
    let client = f.client();
    f.slot(9, component_swap_item(3, 2, false)).await;
    assert!(client.survival().swap_hotbar(9, 0).await.is_err());
    assert!(
        timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    f.slot(9, component_swap_item(2, 2, false)).await;
    let record = client.survival().swap_hotbar(9, 0).await.unwrap();
    read_packet(&mut f.peer, None).await.unwrap();
    f.slot(36, component_swap_item(2, 3, false)).await;
    assert_eq!(
        client
            .survival()
            .inventory_swap_record()
            .await
            .unwrap()
            .unwrap()
            .stage,
        InventorySwapStage::RequiresInspection
    );
    f.slot(36, component_swap_item(2, 2, false)).await;
    f.slot(9, plain("dirt", 12)).await;
    let retained = client
        .survival()
        .inventory_swap_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained.id, record.id);
    assert_eq!(retained.stage, InventorySwapStage::RequiresInspection);
    f.stop().await;
}

#[tokio::test]
async fn data_pickup_same_consumer_both_modes_keeps_data_and_requires_fresh_resync_cursor() {
    use crate::client::{
        GameMode as Mode,
        inventory::{
            InventoryClickButton as Button, InventoryClickSource as Source,
            InventoryClickStage as Stage,
        },
    };
    let mut f = CommonFixture::new().await;
    f.api.bot.session.state.lock().await.registries.finish();
    let client = f.client();
    for mode in [Mode::Survival, Mode::Creative] {
        let mut packet = vec![3];
        packet.extend((if mode == Mode::Creative { 1f32 } else { 0f32 }).to_be_bytes());
        f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &packet)
            .await;
        f.slot(9, component_swap_item(7, 16, true)).await;
        let original = client.received_inventory().await.unwrap();
        for (button, source_count, cursor_count) in [
            (Button::Right, 3, 4),
            (Button::Right, 4, 3),
            (Button::Left, 7, 0),
        ] {
            let record = crate::client::tests::common_pickup_start_scenario(
                &client,
                mode,
                Source::Player,
                9,
                button,
            )
            .await;
            assert!(record.send.request_full_resync);
            assert_ne!(
                record.send.sent_screen_revision,
                record.send.screen_revision
            );
            let (id, payload) = read_packet(&mut f.peer, None).await.unwrap();
            assert_eq!(id, ids::play_serverbound::WINDOW_CLICK);
            assert_eq!(payload, super::click::payload(&record).unwrap());
            assert_eq!(payload.last(), Some(&0)); // resync marker, not actual data cursor hash
            f.slot(9, component_swap_item(source_count, 16, false))
                .await;
            assert_eq!(
                client
                    .survival()
                    .inventory_click_record()
                    .await
                    .unwrap()
                    .unwrap()
                    .stage,
                Stage::Pending
            );
            let mut cursor = Vec::new();
            put_slot(
                &mut cursor,
                &if cursor_count == 0 {
                    InventorySlot::Empty
                } else {
                    component_swap_item(cursor_count, 16, false)
                },
            );
            f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &cursor)
                .await;
            crate::client::tests::common_data_pickup_complete_scenario(
                &client,
                record.id,
                &original,
                (source_count as u32, cursor_count as u32),
            )
            .await;
        }
    }
    f.stop().await;
}
#[tokio::test]
async fn crafting_input_same_consumer_modes_requires_fresh_source_and_cursor_not_result_display() {
    use crate::client::{
        GameMode as Mode,
        inventory::{
            InventoryClickButton as Button, InventoryClickStage as Stage, InventorySource as Source,
        },
    };
    let mut f = CommonFixture::new().await;
    f.api.bot.session.state.lock().await.registries.finish();
    let client = f.client();
    for mode in [Mode::Survival, Mode::Creative] {
        let mut change = vec![3];
        change.extend((if mode == Mode::Creative { 1f32 } else { 0f32 }).to_be_bytes());
        f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &change)
            .await;
        f.slot(1, InventorySlot::Empty).await;
        let mut cursor = Vec::new();
        put_slot(&mut cursor, &plain("oak_planks", 3));
        f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &cursor)
            .await;
        let record =
            crate::client::tests::common_crafting_input_start_scenario(&client, mode).await;
        let (id, payload) = read_packet(&mut f.peer, None).await.unwrap();
        assert_eq!(id, ids::play_serverbound::WINDOW_CLICK);
        assert_eq!(payload, super::click::payload(&record).unwrap());
        f.slot(0, plain("oak_button", 1)).await;
        assert_eq!(
            client
                .survival()
                .inventory_click_record()
                .await
                .unwrap()
                .unwrap()
                .stage,
            Stage::Pending
        );
        f.slot(1, plain("oak_planks", 1)).await;
        assert_eq!(
            client
                .survival()
                .inventory_click_record()
                .await
                .unwrap()
                .unwrap()
                .stage,
            Stage::Pending
        );
        cursor.clear();
        put_slot(&mut cursor, &plain("oak_planks", 2));
        f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &cursor)
            .await;
        crate::client::tests::common_crafting_input_complete_scenario(&client, record.id).await;
        assert!(
            client
                .survival()
                .click_inventory(Source::Player, 0, Button::Left)
                .await
                .is_err()
        );
        assert!(
            timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
                .await
                .is_err()
        );
    }
    f.stop().await;
}

#[tokio::test]
async fn data_pickup_player_full_resync_cursor_requires_completed_matching_close() {
    let mut f = CommonFixture::new().await;
    let opening = f.open_swap_container().await;
    let client = f.client();
    let packet = {
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
        inventory.slots.fill(InventorySlot::Empty);
        inventory.cursor = plain("stone", 4);
        full(&inventory, 130)
    };
    // A player-menu packet must not replace an active foreign menu's cursor.
    f.receive(ids::play_clientbound::WINDOW_ITEMS, &packet)
        .await;
    assert_eq!(
        client.screen_state().await.unwrap().cursor.unwrap().value,
        crate::client::SlotKnowledge::Empty
    );
    client.survival().close_container(opening).await.unwrap();
    read_packet(&mut f.peer, None).await.unwrap();
    f.api
        .bot
        .session
        .state
        .lock()
        .await
        .common_container_close
        .as_mut()
        .unwrap()
        .dispatched = false;
    f.receive(ids::play_clientbound::WINDOW_ITEMS, &packet)
        .await;
    assert_eq!(
        client.screen_state().await.unwrap().cursor.unwrap().value,
        crate::client::SlotKnowledge::Empty
    );
    f.api
        .bot
        .session
        .state
        .lock()
        .await
        .common_container_close
        .as_mut()
        .unwrap()
        .dispatched = true;
    f.receive(ids::play_clientbound::WINDOW_ITEMS, &packet)
        .await;
    let actual = client.received_inventory().await.unwrap();
    assert_eq!(actual.cursor().unwrap().item().unwrap().stack().count, 4);
    assert_eq!(
        client.player_state().await.unwrap().inventory.window_id,
        Some(0)
    );
    // A later opening cannot reuse the old close's admission.
    let next = f.open_swap_container().await;
    assert_ne!(next, opening);
    f.receive(ids::play_clientbound::WINDOW_ITEMS, &packet)
        .await;
    assert_eq!(
        client.screen_state().await.unwrap().cursor.unwrap().value,
        crate::client::SlotKnowledge::Empty
    );
    f.stop().await;
}

#[tokio::test]
async fn data_transfer_same_consumer_both_modes_partial_merge_and_split_return_preserves_data() {
    use crate::client::{
        GameMode as Mode,
        inventory::{InventorySource as Source, InventoryTransferStage as Stage},
    };
    for mode in [Mode::Survival, Mode::Creative] {
        let mut f = CommonFixture::new().await;
        f.api.bot.session.state.lock().await.registries.finish();
        if mode == Mode::Creative {
            let mut p = vec![3];
            p.extend(1f32.to_be_bytes());
            f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &p)
                .await;
        }
        f.slot(9, component_swap_item(7, 16, true)).await;
        f.slot(36, component_swap_item(12, 16, false)).await;
        for i in 37..45 {
            f.slot(i, plain("dirt", 64)).await;
        }
        let client = f.client();
        let original = client.received_inventory().await.unwrap();
        for (source, updates, expected) in [
            (9, vec![(9, 3), (36, 16)], vec![(9, 3), (36, 16)]),
            (
                36,
                vec![(36, 0), (9, 16), (10, 3)],
                vec![(36, 0), (9, 16), (10, 3)],
            ),
        ] {
            let record = crate::client::tests::common_transfer_start_scenario(
                &client,
                mode,
                Source::Player,
                source,
            )
            .await;
            let (id, packet) = read_packet(&mut f.peer, None).await.unwrap();
            assert_eq!(id, ids::play_serverbound::WINDOW_CLICK);
            assert_eq!(packet, super::transfer::payload(&record).unwrap());
            assert!(!record.send.request_full_resync);
            for (ordinal, (slot, count)) in updates.iter().enumerate() {
                f.slot(
                    *slot,
                    if *count == 0 {
                        InventorySlot::Empty
                    } else {
                        component_swap_item(*count, 16, false)
                    },
                )
                .await;
                if ordinal + 1 < updates.len() {
                    assert_eq!(
                        client
                            .survival()
                            .inventory_transfer_record()
                            .await
                            .unwrap()
                            .unwrap()
                            .stage,
                        Stage::Pending
                    );
                }
            }
            crate::client::tests::common_data_transfer_complete_scenario(
                &client, record.id, &original, &expected,
            )
            .await;
        }
        f.stop().await;
    }
}
#[tokio::test]
async fn data_transfer_fresh_capacity_conflict_stays_latched_after_correct_data_returns() {
    use crate::client::inventory::{InventorySource as Source, InventoryTransferStage as Stage};
    let mut f = CommonFixture::new().await;
    f.api.bot.session.state.lock().await.registries.finish();
    f.slot(9, component_swap_item(7, 16, true)).await;
    let client = f.client();
    let record = client
        .survival()
        .transfer_inventory(Source::Player, 9)
        .await
        .unwrap();
    read_packet(&mut f.peer, None).await.unwrap();
    f.slot(36, component_swap_item(7, 15, false)).await;
    f.slot(36, component_swap_item(7, 16, false)).await;
    f.slot(9, InventorySlot::Empty).await;
    let actual = client
        .survival()
        .inventory_transfer_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(actual.id, record.id);
    assert_eq!(actual.stage, Stage::RequiresInspection);
    assert!(actual.requires_inspection.is_some());
    f.stop().await;
}

#[tokio::test]
async fn held_cursor_transfer_same_consumer_modes_requests_real_resync_and_preserves_normalized_cursor()
 {
    use crate::client::{
        GameMode as Mode, ItemData, SlotKnowledge,
        inventory::{InventorySource as Source, InventoryTransferStage as Stage},
    };
    for mode in [Mode::Survival, Mode::Creative] {
        let mut f = CommonFixture::new().await;
        crate::client::inventory::armor_tests::install(
            &mut f.api.bot.session.state.lock().await.registries,
            MinecraftVersion::Java1_21_11,
            None,
        );
        if mode == Mode::Creative {
            let mut p = vec![3];
            p.extend(1f32.to_be_bytes());
            f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &p)
                .await;
        }
        let held = |reversed| {
            let SlotKnowledge::Item { item } =
                crate::client::inventory::held_cursor_tests::data_cursor(
                    MinecraftVersion::Java1_21_11,
                    reversed,
                )
            else {
                unreachable!()
            };
            let ItemData::ModernComponents { patch } = item.data else {
                unreachable!()
            };
            InventorySlot::ItemWithComponents {
                item: PlainItem {
                    name: item.name,
                    item_id: item.id.value(),
                    count: item.count as i32,
                },
                components: patch,
            }
        };
        f.slot(9, plain("stone", 7)).await;
        f.slot(36, InventorySlot::Empty).await;
        let mut cursor = Vec::new();
        put_slot(&mut cursor, &held(false));
        f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &cursor)
            .await;
        let client = f.client();
        let before = client.received_inventory().await.unwrap();
        let record =
            crate::client::tests::common_transfer_start_scenario(&client, mode, Source::Player, 9)
                .await;
        assert!(record.send.request_full_resync);
        assert_ne!(
            record.send.screen_revision,
            record.send.sent_screen_revision
        );
        assert_eq!(
            read_packet(&mut f.peer, None).await.unwrap(),
            (
                ids::play_serverbound::WINDOW_CLICK,
                vec![0, 0, 0, 9, 0, 1, 0, 0]
            )
        );
        cursor.clear();
        put_slot(&mut cursor, &held(true));
        f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &cursor)
            .await;
        f.slot(9, InventorySlot::Empty).await;
        assert_eq!(
            client
                .survival()
                .inventory_transfer_record()
                .await
                .unwrap()
                .unwrap()
                .stage,
            Stage::Pending
        );
        f.slot(36, plain("stone", 7)).await;
        crate::client::tests::common_held_cursor_transfer_complete_scenario(
            &client, record.id, &before,
        )
        .await;
        f.stop().await;
    }
}
#[tokio::test]
async fn armor_transfer_same_consumer_modes_preserves_data_and_requires_both_actual_slots() {
    use crate::client::{
        GameMode as Mode, ItemData, SlotKnowledge,
        inventory::{InventorySource as Source, InventoryTransferStage as Stage},
    };
    for mode in [Mode::Survival, Mode::Creative] {
        for binding in [false, true] {
            let mut f = CommonFixture::new().await;
            crate::client::inventory::armor_tests::install(
                &mut f.api.bot.session.state.lock().await.registries,
                MinecraftVersion::Java1_21_11,
                None,
            );
            if mode == Mode::Creative {
                let mut p = vec![3];
                p.extend(1f32.to_be_bytes());
                f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &p)
                    .await;
            }
            let SlotKnowledge::Item { item } = crate::client::inventory::armor_tests::fixture(
                MinecraftVersion::Java1_21_11,
                binding,
            ) else {
                panic!("missing native helmet")
            };
            let ItemData::ModernComponents { patch } = item.data else {
                panic!("missing native components")
            };
            let value = InventorySlot::ItemWithComponents {
                item: PlainItem {
                    name: item.name,
                    item_id: item.id.value(),
                    count: 1,
                },
                components: patch,
            };
            f.slot(5, value.clone()).await;
            f.slot(9, InventorySlot::Empty).await;
            let client = f.client();
            let before = client.received_inventory().await.unwrap();
            if binding && mode == Mode::Survival {
                let error = client
                    .survival()
                    .transfer_inventory(Source::Player, 5)
                    .await
                    .unwrap_err();
                assert_eq!(error.kind(), crate::ErrorKind::InvalidInput);
                assert!(error.to_string().contains("no effect"));
                assert!(
                    client
                        .survival()
                        .inventory_transfer_record()
                        .await
                        .unwrap()
                        .is_none()
                );
                assert!(
                    timeout(Duration::from_millis(15), read_packet(&mut f.peer, None))
                        .await
                        .is_err()
                );
            } else {
                let record = crate::client::tests::common_transfer_start_scenario(
                    &client,
                    mode,
                    Source::Player,
                    5,
                )
                .await;
                assert_eq!(
                    read_packet(&mut f.peer, None).await.unwrap().0,
                    ids::play_serverbound::WINDOW_CLICK
                );
                f.slot(5, InventorySlot::Empty).await;
                assert_eq!(
                    client
                        .survival()
                        .inventory_transfer_record()
                        .await
                        .unwrap()
                        .unwrap()
                        .stage,
                    Stage::Pending
                );
                f.slot(9, value).await;
                crate::client::tests::common_data_transfer_complete_from_scenario(
                    &client,
                    record.id,
                    &before,
                    5,
                    &[(5, 0), (9, 1)],
                )
                .await;
            }
            f.stop().await;
        }
    }
}
#[tokio::test]
async fn modified_equipment_transfer_same_consumer_both_modes_requires_each_actual_destination() {
    use crate::client::{
        GameMode as Mode,
        inventory::{InventorySource as Source, InventoryTransferStage as Stage},
        registry::Registry,
    };
    use std::io::Read;
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(
        &include_bytes!(
            "../../../../../../data/client_api/equipment_transfer_cases-1.21.11.json.gz"
        )[..],
    )
    .read_to_end(&mut bytes)
    .unwrap();
    let cases: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let native = cases["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| {
            let request = &case["request"];
            request["item"] == "minecraft:stone"
                && request["slot"] == 9
                && request["occupied"] == false
                && request["components"]["minecraft:equippable"]
                    == serde_json::json!({"slot":"head"})
        })
        .unwrap();
    let wire = hex::decode(native["input_hex"].as_str().unwrap()).unwrap();
    let mut reader = crate::versions::java_1_21_11::wire::Reader::new(&wire);
    assert_eq!(reader.varint().unwrap(), 3);
    let item_id = reader.varint().unwrap();
    let patch = crate::versions::java_1_21_11::item_components::read_patch(&mut reader)
        .unwrap()
        .unwrap();
    reader.end().unwrap();
    let item = |count| InventorySlot::ItemWithComponents {
        item: PlainItem {
            name: Registry::for_version(MinecraftVersion::Java1_21_11)
                .item_by_native_id(item_id)
                .unwrap()
                .name,
            item_id,
            count,
        },
        components: patch.clone(),
    };
    for mode in [Mode::Survival, Mode::Creative] {
        let mut f = CommonFixture::new().await;
        f.api.bot.session.state.lock().await.registries.finish();
        if mode == Mode::Creative {
            let mut p = vec![3];
            p.extend(1f32.to_be_bytes());
            f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &p)
                .await;
        }
        f.slot(9, item(3)).await;
        f.slot(36, InventorySlot::Empty).await;
        let client = f.client();
        let before = client.received_inventory().await.unwrap();
        let record =
            crate::client::tests::common_transfer_start_scenario(&client, mode, Source::Player, 9)
                .await;
        assert_eq!(record.changed_slots.len(), 3);
        let (id, packet) = read_packet(&mut f.peer, None).await.unwrap();
        assert_eq!(id, ids::play_serverbound::WINDOW_CLICK);
        assert_eq!(packet, super::transfer::payload(&record).unwrap());
        for (ordinal, slot) in [5, 9, 36].into_iter().enumerate() {
            f.slot(
                slot,
                match slot {
                    5 => item(1),
                    9 => InventorySlot::Empty,
                    36 => item(2),
                    _ => unreachable!(),
                },
            )
            .await;
            if ordinal < 2 {
                assert_eq!(
                    client
                        .survival()
                        .inventory_transfer_record()
                        .await
                        .unwrap()
                        .unwrap()
                        .stage,
                    Stage::Pending
                );
            }
        }
        crate::client::tests::common_data_transfer_complete_scenario(
            &client,
            record.id,
            &before,
            &[(5, 1), (9, 0), (36, 2)],
        )
        .await;
        f.stop().await;
    }
}

#[tokio::test]
async fn data_transfer_damage_stackability_and_removed_equipment_obey_native_no_effect() {
    use crate::client::{ItemComponent, inventory::InventorySource as Source, registry::Registry};
    let mut f = CommonFixture::new().await;
    f.api.bot.session.state.lock().await.registries.finish();
    let registry = Registry::for_version(MinecraftVersion::Java1_21_11);
    let damaged = |count| {
        let InventorySlot::ItemWithComponents {
            item,
            mut components,
        } = component_swap_item(count, 16, false)
        else {
            unreachable!()
        };
        for (name, value) in [("minecraft:max_damage", 16), ("minecraft:damage", 7)] {
            components.added.push(ItemComponent {
                definition: registry.item_component(name).unwrap(),
                bytes: vec![value],
            });
        }
        InventorySlot::ItemWithComponents { item, components }
    };
    f.slot(9, damaged(7)).await;
    f.slot(36, damaged(12)).await;
    for slot in 37..45 {
        f.slot(slot, plain("dirt", 64)).await;
    }
    let client = f.client();
    let received = client.received_inventory().await.unwrap();
    let properties = received
        .slot(9)
        .unwrap()
        .unwrap()
        .item()
        .unwrap()
        .stack()
        .properties()
        .unwrap();
    assert!(properties.damaged && !properties.stackable);
    let error = client
        .survival()
        .transfer_inventory(Source::Player, 9)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), crate::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("no effect"));
    f.slot(
        9,
        InventorySlot::ItemWithComponents {
            item: PlainItem {
                name: "minecraft:carved_pumpkin".into(),
                item_id: registry
                    .item("minecraft:carved_pumpkin")
                    .unwrap()
                    .id
                    .value(),
                count: 7,
            },
            components: crate::client::ItemComponentPatch {
                added: vec![],
                removed: vec![registry.item_component("minecraft:equippable").unwrap()],
            },
        },
    )
    .await;
    let error = client
        .survival()
        .transfer_inventory(Source::Player, 9)
        .await
        .unwrap_err();
    // Removed equippable routes to the full hotbar rather than the empty head slot.
    assert_eq!(error.kind(), crate::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("no effect"), "{error}");
    assert!(
        client
            .survival()
            .inventory_transfer_record()
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(10), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    f.stop().await;
}

#[tokio::test]
async fn data_cursor_close_same_consumer_both_modes_waits_for_each_actual_data_step() {
    use crate::client::{GameMode as Mode, inventory::InventoryClickStage};
    for mode in [Mode::Survival, Mode::Creative] {
        let mut f = CommonFixture::new().await;
        f.api.bot.session.state.lock().await.registries.finish();
        let screen = f.open_swap_container().await;
        let mut health = Vec::from(20f32.to_be_bytes());
        put_varint(&mut health, 20);
        health.extend(5f32.to_be_bytes());
        f.receive(ids::play_clientbound::UPDATE_HEALTH, &health)
            .await;
        f.container_slot(27, component_swap_item(13, 16, false))
            .await;
        let mut cursor = Vec::new();
        put_slot(&mut cursor, &component_swap_item(5, 16, true));
        f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &cursor)
            .await;
        if mode == Mode::Creative {
            let mut p = vec![3];
            p.extend(1f32.to_be_bytes());
            f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &p)
                .await;
        }
        let client = f.client();
        let before = client.received_inventory().await.unwrap();
        let waiter = crate::client::tests::common_cursor_close_start(&client, mode, screen);
        for index in 0..2 {
            let (id, payload) = timeout(Duration::from_secs(1), read_packet(&mut f.peer, None))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(id, ids::play_serverbound::WINDOW_CLICK);
            let record =
                crate::client::tests::common_cursor_close_retained(&client, index + 1).await;
            let step = &record.return_steps[index];
            assert_eq!(payload, super::click::payload(step).unwrap());
            assert_eq!(step.source_slot, 27 + index as u16);
            assert!(client.survival().swap_hotbar(9, 0).await.is_err());
            assert!(f.api.swap_player_hotbar(9, 0).await.is_err());
            assert!(
                timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
                    .await
                    .is_err()
            );
            f.container_slot(
                step.source_slot,
                component_swap_item(if index == 0 { 16 } else { 2 }, 16, false),
            )
            .await;
            assert!(
                timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
                    .await
                    .is_err()
            );
            let mut cursor = Vec::new();
            put_slot(
                &mut cursor,
                &if index == 0 {
                    component_swap_item(2, 16, false)
                } else {
                    InventorySlot::Empty
                },
            );
            f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &cursor)
                .await;
        }
        assert_eq!(
            timeout(Duration::from_secs(1), read_packet(&mut f.peer, None))
                .await
                .unwrap()
                .unwrap(),
            (ids::play_serverbound::CLOSE_WINDOW, vec![3])
        );
        let complete = waiter.await.unwrap().unwrap();
        crate::client::tests::common_data_cursor_close_complete_scenario(
            &client,
            &complete,
            &before,
            &[(9, 16), (10, 2)],
        )
        .await;
        assert!(complete.dispatched);
        assert!(
            complete
                .return_steps
                .iter()
                .all(|s| s.stage == InventoryClickStage::ObservedClicked)
        );
        assert!(
            complete
                .return_steps
                .iter()
                .all(|s| s.source_receipt.is_some() && s.cursor_receipt.is_some())
        );
        assert!(complete.server_close_sequence.is_none());
        assert!(
            timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
                .await
                .is_err()
        );
        f.stop().await;
    }
}

#[tokio::test]
async fn data_cursor_close_no_capacity_refuses_before_any_return_or_close_frame() {
    let mut f = CommonFixture::new().await;
    f.api.bot.session.state.lock().await.registries.finish();
    let screen = f.open_swap_container().await;
    let mut health = Vec::from(20f32.to_be_bytes());
    put_varint(&mut health, 20);
    health.extend(5f32.to_be_bytes());
    f.receive(ids::play_clientbound::UPDATE_HEALTH, &health)
        .await;
    for slot in 27..63 {
        f.container_slot(slot, component_swap_item(16, 16, false))
            .await;
    }
    let mut cursor = Vec::new();
    put_slot(&mut cursor, &component_swap_item(5, 16, true));
    f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &cursor)
        .await;
    let client = f.client();
    let error = client.survival().close_container(screen).await.unwrap_err();
    assert_eq!(error.kind(), crate::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("sufficient"));
    assert!(
        client
            .survival()
            .container_close_record()
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    assert_eq!(
        client
            .received_inventory()
            .await
            .unwrap()
            .cursor()
            .unwrap()
            .item()
            .unwrap()
            .stack()
            .count,
        5
    );
    f.stop().await;
}
#[tokio::test]
async fn data_cursor_close_conflicting_fields_do_not_heal_or_submit_close() {
    let mut f = CommonFixture::new().await;
    f.api.bot.session.state.lock().await.registries.finish();
    let screen = f.open_swap_container().await;
    let mut health = Vec::from(20f32.to_be_bytes());
    put_varint(&mut health, 20);
    health.extend(5f32.to_be_bytes());
    f.receive(ids::play_clientbound::UPDATE_HEALTH, &health)
        .await;
    f.container_slot(27, component_swap_item(13, 16, false))
        .await;
    let mut cursor = Vec::new();
    put_slot(&mut cursor, &component_swap_item(5, 16, true));
    f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &cursor)
        .await;
    let client = f.client();
    let waiter = crate::client::tests::common_cursor_close_start(
        &client,
        crate::client::GameMode::Survival,
        screen,
    );
    assert_eq!(
        read_packet(&mut f.peer, None).await.unwrap().0,
        ids::play_serverbound::WINDOW_CLICK
    );
    f.container_slot(27, component_swap_item(16, 15, false))
        .await;
    f.container_slot(27, component_swap_item(16, 16, false))
        .await;
    f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &[0])
        .await;
    assert!(waiter.await.unwrap().is_err());
    let record = client
        .survival()
        .container_close_record()
        .await
        .unwrap()
        .unwrap();
    assert!(record.requires_inspection.is_some() && !record.dispatched);
    assert_eq!(record.return_steps.len(), 1);
    assert!(
        timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    f.stop().await;
}

#[tokio::test]
async fn crafting_take_cancelled_caller_retains_owned_write_and_prompt_history() {
    use crate::client::crafting::CraftingTakeStage;
    let mut f = CommonFixture::new().await;
    f.api.bot.session.state.lock().await.registries.finish();
    for slot in 0..=4 {
        f.slot(
            slot,
            if slot == 0 {
                plain("stick", 4)
            } else {
                InventorySlot::Empty
            },
        )
        .await;
    }
    let mut cursor = Vec::new();
    put_slot(&mut cursor, &InventorySlot::Empty);
    f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &cursor)
        .await;
    let client = f.client();
    let grid = client.received_crafting().await.unwrap().unwrap();
    let bot = f.api.bot.clone();
    let writer = bot.session.writer.lock().await;
    let ops = client.survival();
    let waiter = tokio::spawn(async move { ops.take_crafting_result(&grid).await });
    let pending = timeout(Duration::from_secs(1), async {
        loop {
            if let Some(r) = client.survival().crafting_take_record().await.unwrap() {
                break r;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!pending.send.dispatched);
    assert!(pending.after.is_none());
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    drop(writer);
    let (id, payload) = timeout(Duration::from_secs(1), read_packet(&mut f.peer, None))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(id, ids::play_serverbound::WINDOW_CLICK);
    assert_eq!(payload, super::crafting::payload(&pending).unwrap());
    let sent = client
        .survival()
        .crafting_take_record()
        .await
        .unwrap()
        .unwrap();
    assert!(sent.send.dispatched);
    assert_eq!(sent.id, pending.id);
    assert!(
        client
            .survival()
            .take_crafting_result(&sent.before)
            .await
            .is_err()
    );
    assert!(
        timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    f.stop().await;
    let retained = client
        .survival()
        .crafting_take_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained.stage, CraftingTakeStage::RequiresInspection);
    assert_eq!(retained.id, pending.id);
}
