use super::*;
use crate::client::{
    GameMode,
    inventory::{InventorySource, InventorySwapRecord, InventorySwapStage as Stage},
};

async fn fixture(
    table: bool,
    mode: GameMode,
    input: u16,
    hotbar: u8,
    occupied: bool,
) -> (CommonFixture, InventorySource) {
    let mut f = CommonFixture::new().await;
    crate::client::inventory::armor_tests::install(
        &mut f.api.bot.session.state.lock().await.registries,
        MinecraftVersion::Java1_21_11,
        None,
    );
    if mode == GameMode::Creative {
        let mut p = vec![3];
        p.extend(1f32.to_be_bytes());
        f.receive(ids::play_clientbound::GAME_STATE_CHANGE, &p)
            .await;
    }
    if table {
        f.receive(ids::play_clientbound::OPEN_WINDOW, &[3, 12, 10, 0])
            .await;
    }
    let mut full = vec![if table { 3 } else { 0 }, 7, 46];
    for index in 0..46 {
        put_slot(
            &mut full,
            &if index == usize::from(input) && occupied {
                component_swap_item(7, 16, true)
            } else if index == (if table { 37 } else { 36 }) + usize::from(hotbar) {
                plain("oak_planks", 3)
            } else {
                InventorySlot::Empty
            },
        );
    }
    put_slot(&mut full, &InventorySlot::Empty);
    f.receive(ids::play_clientbound::WINDOW_ITEMS, &full).await;
    let source = if table {
        InventorySource::Container {
            screen: f.client().screen_state().await.unwrap().screen.unwrap().id,
        }
    } else {
        InventorySource::Player
    };
    (f, source)
}
async fn start(
    client: &crate::Client,
    mode: GameMode,
    source: InventorySource,
    input: u16,
    hotbar: u8,
) -> crate::Result<InventorySwapRecord> {
    match mode {
        GameMode::Creative => {
            client
                .creative()
                .swap_crafting_input_hotbar(source, input, hotbar)
                .await
        }
        _ => {
            client
                .survival()
                .swap_crafting_input_hotbar(source, input, hotbar)
                .await
        }
    }
}
async fn update(f: &mut CommonFixture, table: bool, slot: u16, value: InventorySlot) {
    if table {
        f.container_slot(slot, value).await;
    } else {
        f.slot(slot, value).await;
    }
}

#[tokio::test]
async fn crafting_input_swap_packets_all_inputs_modes_empty_occupied_hotbar_endpoints() {
    for table in [false, true] {
        for mode in [GameMode::Survival, GameMode::Creative] {
            for input in 1..=if table { 9 } else { 4 } {
                for hotbar in [0, 8] {
                    for occupied in [false, true] {
                        let (mut f, source) = fixture(table, mode, input, hotbar, occupied).await;
                        let client = f.client();
                        let record = start(&client, mode, source, input, hotbar).await.unwrap();
                        assert!(record.send.dispatched);
                        assert_eq!(record.send.screen_revision, Some(7));
                        let mut expected = vec![if table { 3 } else { 0 }, 0];
                        expected.extend(input.to_be_bytes());
                        expected.extend([hotbar, 2, 0, 0]);
                        assert_eq!(
                            read_packet(&mut f.peer, None).await.unwrap(),
                            (ids::play_serverbound::WINDOW_CLICK, expected)
                        );
                        // Result recomputation does not establish either destination.
                        update(&mut f, table, 0, plain("oak_button", 1)).await;
                        let pending = client
                            .survival()
                            .inventory_swap_record()
                            .await
                            .unwrap()
                            .unwrap();
                        assert_eq!(pending.stage, Stage::Pending);
                        assert!(
                            pending.source_receipt.is_none() && pending.hotbar_receipt.is_none()
                        );
                        // Both receive orders, with semantic normalization of component data.
                        let updates = [
                            (input, plain("oak_planks", 3)),
                            (
                                record.hotbar_screen_slot,
                                if occupied {
                                    component_swap_item(7, 16, false)
                                } else {
                                    InventorySlot::Empty
                                },
                            ),
                        ];
                        let order = if hotbar == 0 { [0, 1] } else { [1, 0] };
                        for (step, index) in order.into_iter().enumerate() {
                            let (slot, value) = &updates[index];
                            update(&mut f, table, *slot, value.clone()).await;
                            let current = client
                                .survival()
                                .inventory_swap_record()
                                .await
                                .unwrap()
                                .unwrap();
                            assert_eq!(current.id, record.id);
                            assert_eq!(
                                current.stage,
                                if step == 0 {
                                    Stage::Pending
                                } else {
                                    Stage::ObservedSwapped
                                },
                                "{current:?}"
                            );
                            if step == 1 {
                                for receipt in [current.source_receipt, current.hotbar_receipt] {
                                    assert!(
                                        matches!(receipt.unwrap().source,crate::client::ValueSource::Received {sequence} if sequence > record.send.after_sequence)
                                    );
                                }
                            }
                        }
                        assert_eq!(
                            client
                                .player_state()
                                .await
                                .unwrap()
                                .inventory
                                .cursor
                                .unwrap()
                                .value,
                            record.initial.inventory.cursor.unwrap().value
                        );
                        // Completed receive history freezes independently of later changes.
                        update(&mut f, table, input, InventorySlot::Empty).await;
                        client.disconnect().await.unwrap();
                        assert_eq!(
                            client
                                .survival()
                                .inventory_swap_record()
                                .await
                                .unwrap()
                                .unwrap()
                                .stage,
                            Stage::ObservedSwapped
                        );
                        f.stop().await;
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn crafting_input_swap_packets_recover_data_into_empty_hotbar() {
    for table in [false, true] {
        for mode in [GameMode::Survival, GameMode::Creative] {
            for hotbar in [0, 8] {
                let (mut f, source) = fixture(table, mode, 1, hotbar, true).await;
                let destination = (if table { 37 } else { 36 }) + u16::from(hotbar);
                update(&mut f, table, destination, InventorySlot::Empty).await;
                let client = f.client();
                let sent = start(&client, mode, source, 1, hotbar).await.unwrap();
                assert_eq!(
                    sent.hotbar_before.value,
                    crate::client::SlotKnowledge::Empty
                );
                let revision = if table { 6 } else { 130 };
                assert_eq!(sent.send.screen_revision, Some(revision));
                let mut expected = vec![if table { 3 } else { 0 }];
                put_varint(&mut expected, if revision == 0 { 1 } else { 0 });
                expected.extend(1u16.to_be_bytes());
                expected.extend([hotbar, 2, 0, 0]);
                assert_eq!(
                    read_packet(&mut f.peer, None).await.unwrap(),
                    (ids::play_serverbound::WINDOW_CLICK, expected)
                );
                let updates = [
                    (1, InventorySlot::Empty),
                    (destination, component_swap_item(7, 16, false)),
                ];
                let order = if hotbar == 0 { [0, 1] } else { [1, 0] };
                for (step, index) in order.into_iter().enumerate() {
                    let (slot, value) = &updates[index];
                    update(&mut f, table, *slot, value.clone()).await;
                    let current = client
                        .survival()
                        .inventory_swap_record()
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(current.id, sent.id);
                    assert_eq!(
                        current.stage,
                        if step == 0 {
                            Stage::Pending
                        } else {
                            Stage::ObservedSwapped
                        }
                    );
                    if step == 1 {
                        assert_eq!(
                            current.source_receipt.as_ref().unwrap().value,
                            crate::client::SlotKnowledge::Empty
                        );
                        for receipt in [current.source_receipt, current.hotbar_receipt] {
                            assert!(matches!(receipt.unwrap().source,
                                crate::client::ValueSource::Received { sequence }
                                    if sequence > sent.send.after_sequence));
                        }
                    }
                }
                assert_eq!(
                    client.player_state().await.unwrap().inventory.cursor,
                    sent.initial.inventory.cursor
                );
                f.fence_response().await;
                assert!(
                    timeout(Duration::from_millis(15), read_packet(&mut f.peer, None))
                        .await
                        .is_err()
                );
                f.stop().await;
            }
        }
    }
}

#[tokio::test]
async fn crafting_input_swap_capture_rejects_preopening_and_future_receipts() {
    use crate::client::{ValueSource, inventory as contract};
    let (mut f, source) = fixture(true, GameMode::Survival, 1, 8, true).await;
    let client = f.client();
    let initial = client.player_state().await.unwrap();
    let screen = client.screen_state().await.unwrap().screen.unwrap();
    let owner = f
        .api
        .bot
        .session
        .state
        .lock()
        .await
        .registries
        .capture(initial.session, initial.receive_sequence);
    for sequence in [
        screen.id.opened_sequence() - 1,
        initial.receive_sequence + 1,
    ] {
        for field in 0..4 {
            let mut initial = initial.clone();
            let mut screen = screen.clone();
            let receipt = ValueSource::Received { sequence };
            match field {
                0 => screen.slots[1].as_mut().unwrap().source = receipt,
                1 => {
                    // Keep both aliases coherent: the failure must be opening ownership.
                    screen.slots[45].as_mut().unwrap().source = receipt;
                    initial.inventory.slots[44].as_mut().unwrap().source = receipt;
                }
                2 => initial.inventory.cursor.as_mut().unwrap().source = receipt,
                _ => screen.revision.as_mut().unwrap().source = receipt,
            }
            let error = contract::prepare_received_source(
                (initial, owner.clone()),
                GameMode::Survival,
                contract::InventorySwapSource::CraftingInput { source },
                1,
                8,
                1,
                Some(screen),
            )
            .unwrap_err();
            assert_eq!(
                error.to_string(),
                if field == 3 {
                    "crafting SWAP received screen revision unavailable"
                } else {
                    "crafting SWAP predecessor outside original capture/opening"
                }
            );
        }
    }
    assert!(
        client
            .survival()
            .inventory_swap_record()
            .await
            .unwrap()
            .is_none()
    );
    f.fence_response().await;
    assert!(
        timeout(Duration::from_millis(15), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    // Rejected captures did not acquire the inventory owner.
    start(&client, GameMode::Survival, source, 1, 8)
        .await
        .unwrap();
    read_packet(&mut f.peer, None).await.unwrap();
    f.stop().await;
}

#[tokio::test]
async fn crafting_input_swap_packets_reject_result_appended_equipment_and_invalid_hotbar_before_intent()
 {
    for table in [false, true] {
        let (mut f, source) = fixture(table, GameMode::Survival, 1, 0, true).await;
        let client = f.client();
        for input in if table {
            vec![0, 10, 36, 37, 45]
        } else {
            vec![0, 5, 8, 9, 36, 45]
        } {
            assert!(
                start(&client, GameMode::Survival, source, input, 0)
                    .await
                    .is_err()
            );
        }
        assert!(
            start(&client, GameMode::Survival, source, 1, 9)
                .await
                .is_err()
        );
        assert!(
            client
                .survival()
                .inventory_swap_record()
                .await
                .unwrap()
                .is_none()
        );
        f.fence_response().await;
        assert!(
            timeout(Duration::from_millis(15), read_packet(&mut f.peer, None))
                .await
                .is_err()
        );
        start(&client, GameMode::Survival, source, 1, 0)
            .await
            .unwrap();
        read_packet(&mut f.peer, None).await.unwrap();
        // Every other inventory entrance shares the unresolved owner.
        assert!(client.survival().swap_hotbar(9, 8).await.is_err());
        assert!(
            client
                .survival()
                .transfer_inventory(source, 1)
                .await
                .is_err()
        );
        assert!(
            client
                .survival()
                .click_inventory(
                    source,
                    1,
                    crate::client::inventory::InventoryClickButton::Left
                )
                .await
                .is_err()
        );
        f.stop().await;
    }
}

#[tokio::test]
async fn crafting_input_swap_packets_cursor_and_destination_conflicts_never_heal() {
    for table in [false, true] {
        for conflict in 0..4 {
            let (mut f, source) = fixture(table, GameMode::Survival, 1, 8, true).await;
            let client = f.client();
            let sent = start(&client, GameMode::Survival, source, 1, 8)
                .await
                .unwrap();
            read_packet(&mut f.peer, None).await.unwrap();
            match conflict {
                0 => {
                    let mut p = Vec::new();
                    put_slot(&mut p, &plain("dirt", 1));
                    f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &p).await;
                    f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &[0])
                        .await;
                }
                1 => {
                    update(&mut f, table, 1, plain("oak_planks", 3)).await;
                    update(&mut f, table, 1, component_swap_item(7, 16, true)).await;
                }
                2 => update(&mut f, table, 1, plain("dirt", 1)).await,
                _ => update(&mut f, table, 1, component_swap_item(7, 32, false)).await,
            }
            update(&mut f, table, 1, plain("oak_planks", 3)).await;
            update(
                &mut f,
                table,
                sent.hotbar_screen_slot,
                component_swap_item(7, 16, false),
            )
            .await;
            let record = client
                .survival()
                .inventory_swap_record()
                .await
                .unwrap()
                .unwrap();
            assert_eq!(record.id, sent.id);
            assert_eq!(record.stage, Stage::RequiresInspection);
            assert!(
                start(&client, GameMode::Survival, source, 1, 8)
                    .await
                    .is_err()
            );
            f.fence_response().await;
            assert!(
                timeout(Duration::from_millis(15), read_packet(&mut f.peer, None))
                    .await
                    .is_err()
            );
            f.stop().await;
        }
    }
}

#[tokio::test]
async fn crafting_input_swap_packets_close_and_reopen_numeric_id_never_rebind() {
    for close in [false, true] {
        let (mut f, source) = fixture(true, GameMode::Survival, 9, 0, true).await;
        let client = f.client();
        let sent = start(&client, GameMode::Survival, source, 9, 0)
            .await
            .unwrap();
        read_packet(&mut f.peer, None).await.unwrap();
        if close {
            f.receive(ids::play_clientbound::CLOSE_WINDOW, &[3]).await;
        }
        f.receive(ids::play_clientbound::OPEN_WINDOW, &[3, 12, 10, 0])
            .await;
        let mut full = vec![3, 8, 46];
        for slot in 0..46 {
            put_slot(
                &mut full,
                &if slot == 9 {
                    plain("oak_planks", 3)
                } else if slot == 37 {
                    component_swap_item(7, 16, false)
                } else {
                    InventorySlot::Empty
                },
            );
        }
        full.push(0);
        f.receive(ids::play_clientbound::WINDOW_ITEMS, &full).await;
        let new = client.screen_state().await.unwrap().screen.unwrap().id;
        let InventorySource::Container { screen: old } = source else {
            unreachable!()
        };
        assert_ne!(new, old);
        let current = client
            .survival()
            .inventory_swap_record()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.id, sent.id);
        assert_eq!(current.stage, Stage::RequiresInspection);
        assert!(
            start(
                &client,
                GameMode::Survival,
                InventorySource::Container { screen: new },
                9,
                0
            )
            .await
            .is_err()
        );
        f.stop().await;
    }
}

#[tokio::test]
async fn crafting_input_swap_packets_cancel_before_writer_retains_unsent_intent() {
    for table in [false, true] {
        let (mut f, source) = fixture(table, GameMode::Survival, 1, 0, true).await;
        let client = f.client();
        let session = f.api.bot.session.clone();
        let writer = session.writer.lock().await;
        let ops = client.survival();
        let waiter =
            tokio::spawn(async move { ops.swap_crafting_input_hotbar(source, 1, 0).await });
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
        update(&mut f, table, 1, plain("oak_planks", 3)).await;
        update(
            &mut f,
            table,
            if table { 37 } else { 36 },
            component_swap_item(7, 16, false),
        )
        .await;
        let current = client
            .survival()
            .inventory_swap_record()
            .await
            .unwrap()
            .unwrap();
        assert!(!current.send.dispatched);
        assert_eq!(current.stage, Stage::Pending);
        assert!(
            start(&client, GameMode::Survival, source, 1, 0)
                .await
                .is_err()
        );
        f.fence_response().await;
        assert!(
            timeout(Duration::from_millis(15), read_packet(&mut f.peer, None))
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
            Stage::RequiresInspection
        );
        f.stop().await;
    }
}

#[tokio::test]
async fn crafting_input_swap_packets_partial_write_cancellation_is_uncertain_and_never_retries() {
    for table in [false, true] {
        let (f, source) = fixture(table, GameMode::Survival, 1, 0, true).await;
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
        let waiter =
            tokio::spawn(async move { ops.swap_crafting_input_hotbar(source, 1, 0).await });
        control.until(|writer| writer.bytes().len() == 1).await;
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        let record = client
            .survival()
            .inventory_swap_record()
            .await
            .unwrap()
            .unwrap();
        assert!(!record.send.dispatched);
        assert_eq!(record.stage, Stage::RequiresInspection);
        assert!(
            start(&client, GameMode::Survival, source, 1, 0)
                .await
                .is_err()
        );
        assert!(f.api.bot.session.ledger.cut(false).wait().await.is_err());
        assert_eq!(control.bytes().len(), 1);
        f.stop().await;
    }
}

#[tokio::test]
async fn crafting_input_swap_packets_missing_revision_capacity_cursor_and_old_closed_opening_send_nothing()
 {
    for table in [false, true] {
        for invalid in 0..6 {
            let (mut f, source) = fixture(table, GameMode::Survival, 1, 0, true).await;
            let client = f.client();
            match invalid {
                0 => {
                    let mut state = f.api.bot.session.state.lock().await;
                    if table {
                        state
                            .operations
                            .inventory
                            .container
                            .as_mut()
                            .unwrap()
                            .revision = None;
                    } else {
                        state.operations.inventory.screen_revision = None;
                    }
                }
                1 => {
                    update(
                        &mut f,
                        table,
                        if table { 37 } else { 36 },
                        component_swap_item(17, 16, false),
                    )
                    .await
                }
                2 => {
                    let mut p = Vec::new();
                    put_slot(&mut p, &plain("dirt", 1));
                    f.receive(ids::play_clientbound::SET_CURSOR_ITEM, &p).await;
                }
                3 => {
                    if let InventorySource::Container { screen } = source {
                        client.survival().close_container(screen).await.unwrap();
                        read_packet(&mut f.peer, None).await.unwrap();
                    } else {
                        f.slot(1, component_swap_item(17, 16, false)).await;
                    }
                }
                4 => update(&mut f, table, 1, plain("oak_planks", 3)).await,
                _ => {
                    update(&mut f, table, 1, InventorySlot::Empty).await;
                    update(
                        &mut f,
                        table,
                        if table { 37 } else { 36 },
                        InventorySlot::Empty,
                    )
                    .await;
                }
            }
            assert!(
                start(&client, GameMode::Survival, source, 1, 0)
                    .await
                    .is_err()
            );
            assert!(
                client
                    .survival()
                    .inventory_swap_record()
                    .await
                    .unwrap()
                    .is_none()
            );
            f.fence_response().await;
            assert!(
                timeout(Duration::from_millis(15), read_packet(&mut f.peer, None))
                    .await
                    .is_err()
            );
            f.stop().await;
        }
    }
}
