use super::*;
use crate::client::{Hand, SlotKnowledge};

#[tokio::test]
async fn use_on_block_expect_checks_identity_inside_dispatch_lock() {
    for mode in [GameMode::Survival, GameMode::Creative] {
        for hand in [Hand::Main, Hand::Off] {
            let mut f = Fixture::new().await;
            let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
            let item = client.registry().item("minecraft:dirt").unwrap().id;
            let other = client.registry().item("minecraft:stone").unwrap().id;
            f.session.state.lock().await.operations.game_mode = Some(mode);
            f.api.select_hotbar(0).await.unwrap();
            read_packet(&mut f.peer, None).await.unwrap();
            let slot = if hand == Hand::Main { 36 } else { 45 };
            let mut payload = vec![0, 2, 0, slot, 1];
            put_varint(&mut payload, item.value());
            payload.extend([0, 0]);
            f.receive(ids::play_clientbound::SET_SLOT, &payload).await;
            let use_expected = |selected_hotbar, item| {
                let client = &client;
                async move {
                    match mode {
                        GameMode::Survival => {
                            client
                                .survival()
                                .use_on_block_expect(
                                    [1, 0, 0],
                                    crate::BlockFace::Up,
                                    [0.5; 3],
                                    hand,
                                    selected_hotbar,
                                    item,
                                )
                                .await
                        }
                        _ => {
                            client
                                .creative()
                                .use_on_block_expect(
                                    [1, 0, 0],
                                    crate::BlockFace::Up,
                                    [0.5; 3],
                                    hand,
                                    selected_hotbar,
                                    item,
                                )
                                .await
                        }
                    }
                }
            };
            let other_version = if client.version() == crate::MinecraftVersion::Java1_16_1 {
                crate::MinecraftVersion::Java1_21_11
            } else {
                crate::MinecraftVersion::Java1_16_1
            };
            let foreign = crate::client::registry::Registry::for_version(other_version)
                .item("minecraft:dirt")
                .unwrap()
                .id;
            let non_item = client.registry().block_state_id(0).unwrap();
            for (selected, expected) in [(9, item), (0, foreign), (0, non_item)] {
                let error = use_expected(selected, expected).await.unwrap_err();
                assert_eq!(error.kind(), crate::ErrorKind::InvalidInput);
                assert!(error.use_item_refusal().is_none());
            }
            let sent = use_expected(0, item).await.unwrap();
            assert_eq!(sent.version, MinecraftVersion::Java1_21_11);
            let (id, bytes) = read_packet(&mut f.peer, None).await.unwrap();
            assert_eq!(id, ids::play_serverbound::BLOCK_PLACE);
            let mut expected_packet = vec![hand as u8];
            expected_packet.extend(pack_position([1, 0, 0]).to_be_bytes());
            expected_packet.push(crate::BlockFace::Up as u8);
            for v in [0.5f32; 3] {
                expected_packet.extend(v.to_be_bytes());
            }
            expected_packet.extend([0, 0]);
            put_varint(&mut expected_packet, sent.interaction_sequence.unwrap());
            assert_eq!(bytes, expected_packet);
            for case in [
                "slot",
                "item",
                "empty",
                "unavailable",
                "unknown_item",
                "unsubmitted_selection",
            ] {
                // The caller observed a matching identity before starting dispatch.
                let mut gate = f.session.state.lock().await;
                gate.operations.selected_hotbar.as_mut().unwrap().slot = 0;
                gate.operations.selected_hotbar.as_mut().unwrap().dispatched = true;
                let mut reset = vec![0, 3, 0, slot, 1];
                put_varint(&mut reset, item.value());
                reset.extend([0, 0]);
                super::super::super::receive(&mut gate, ids::play_clientbound::SET_SLOT, &reset)
                    .unwrap();
                drop(gate);
                crate::client::item_use::validate_expected(
                    &client.player_state().await.unwrap(),
                    hand,
                    0,
                    item,
                )
                .unwrap();
                let mut gate = f.session.state.lock().await;
                let mut pending = Box::pin(use_expected(0, item));
                assert!(
                    timeout(Duration::from_millis(20), pending.as_mut())
                        .await
                        .is_err()
                );
                match case {
                    "slot" => gate.operations.selected_hotbar.as_mut().unwrap().slot = 1,
                    "item" => {
                        let mut update = vec![0, 4, 0, slot, 1];
                        put_varint(&mut update, other.value());
                        update.extend([0, 0]);
                        super::super::super::receive(
                            &mut gate,
                            ids::play_clientbound::SET_SLOT,
                            &update,
                        )
                        .unwrap();
                    }
                    "empty" => {
                        super::super::super::receive(
                            &mut gate,
                            ids::play_clientbound::SET_SLOT,
                            &[0, 4, 0, slot, 0],
                        )
                        .unwrap();
                    }
                    "unavailable" => {
                        gate.operations.inventory.slots[slot as usize] = InventorySlot::Unavailable
                    }
                    "unknown_item" => {
                        gate.operations.inventory.slot_sequences[slot as usize] = None
                    }
                    "unsubmitted_selection" => {
                        gate.operations.selected_hotbar.as_mut().unwrap().dispatched = false
                    }
                    _ => unreachable!(),
                }
                drop(gate);
                let error = pending.await.unwrap_err();
                assert_eq!(error.kind(), ErrorKind::State, "{case}");
                let refusal = error.use_item_refusal().unwrap();
                assert_eq!(refusal.hand, hand);
                assert_eq!(refusal.expected_selected_hotbar, 0);
                assert_eq!(refusal.expected_item, item);
                assert_eq!(
                    refusal.observed_selected_hotbar,
                    if case == "slot" {
                        Some(1)
                    } else if case.ends_with("selection") {
                        None
                    } else {
                        Some(0)
                    }
                );
                if case == "item" {
                    assert!(
                        matches!(&refusal.observed_item, Some(SlotKnowledge::Item { item }) if item.id == other)
                    );
                }
                if case == "unknown_item" {
                    assert_eq!(refusal.observed_item, None);
                }
                assert!(
                    timeout(Duration::from_millis(30), read_packet(&mut f.peer, None))
                        .await
                        .is_err(),
                    "{case}"
                );
            }
            // The unchecked API allows an unknown held item, while its existing
            // admission still requires any submitted selection to be resolved.
            let mut gate = f.session.state.lock().await;
            gate.operations.selected_hotbar.as_mut().unwrap().dispatched = true;
            gate.operations.inventory.slot_sequences[slot as usize] = None;
            drop(gate);
            let sent_unchecked = match mode {
                GameMode::Survival => {
                    client
                        .survival()
                        .use_on_block([1, 0, 0], crate::BlockFace::Up, [0.5; 3], hand)
                        .await
                }
                _ => {
                    client
                        .creative()
                        .use_on_block([1, 0, 0], crate::BlockFace::Up, [0.5; 3], hand)
                        .await
                }
            }
            .unwrap();
            assert_eq!(
                sent_unchecked.interaction_sequence,
                sent.interaction_sequence.map(|s| s + 1)
            );
            assert_eq!(
                read_packet(&mut f.peer, None).await.unwrap().0,
                ids::play_serverbound::BLOCK_PLACE
            );
            f.stop().await;
        }
    }
}
