use super::*;
use crate::client::{self as api, Hand, SlotKnowledge};
use tokio::time::timeout;

#[tokio::test]
async fn use_on_block_expect_checks_identity_inside_dispatch_lock() {
    for mode in [api::GameMode::Survival, api::GameMode::Creative] {
        for hand in [Hand::Main, Hand::Off] {
            let (bot, mut packets, release, server) =
                super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
            let client = crate::Client::from_java_1_16_1(bot.clone());
            let item = client.registry().item("minecraft:dirt").unwrap().id;
            let other = client.registry().item("minecraft:stone").unwrap().id;
            bot.survival.write().await.game_mode = Some(if mode == api::GameMode::Survival {
                0
            } else {
                1
            });
            bot.select_hotbar(0).await.unwrap();
            assert_eq!(packets.recv().await.unwrap().0, 0x24);
            let slot = if hand == Hand::Main { 36i16 } else { 45 };
            let mut payload = vec![0];
            payload.extend(slot.to_be_bytes());
            payload.push(1);
            put_varint(&mut payload, item.value());
            payload.extend([1, 0]);
            bot.apply_packet(0x16, payload).await.unwrap();
            let use_expected = |selected_hotbar, item| {
                let client = &client;
                async move {
                    match mode {
                        api::GameMode::Survival => {
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
            assert_eq!(sent.version, crate::MinecraftVersion::Java1_16_1);
            let (id, bytes) = packets.recv().await.unwrap();
            assert_eq!(id, 0x2d);
            let mut expected_packet = vec![hand as u8];
            expected_packet.extend(BlockPos { x: 1, y: 0, z: 0 }.packed().to_be_bytes());
            expected_packet.push(crate::BlockFace::Up as u8);
            for v in [0.5f32; 3] {
                expected_packet.extend(v.to_be_bytes());
            }
            expected_packet.push(0);
            assert_eq!(sent.interaction_sequence, None);
            assert_eq!(bytes, expected_packet);
            let baseline = client.player_state().await.unwrap();
            let received = baseline.inventory.slots[slot as usize].clone().unwrap();
            for case in [
                "slot",
                "item",
                "empty",
                "unavailable",
                "unknown_item",
                "predicted_item",
                "unknown_selection",
                "unsubmitted_selection",
            ] {
                // The caller observed a matching identity before starting dispatch.
                let gate = bot.coherent_state_gate.lock().await;
                bot.inventory.write().await.selected_hotbar = 0;
                bot.common_receipts.lock().await.selected_hotbar = Some(api::received(0, 100));
                bot.common_receipts.lock().await.inventory.slots[slot as usize] =
                    Some(received.clone());
                drop(gate);
                crate::client::item_use::validate_expected(
                    &client.player_state().await.unwrap(),
                    hand,
                    0,
                    item,
                )
                .unwrap();
                let gate = bot.coherent_state_gate.lock().await;
                let mut pending = Box::pin(use_expected(0, item));
                assert!(
                    timeout(Duration::from_millis(20), pending.as_mut())
                        .await
                        .is_err()
                );
                let mut observed = Some(received.clone());
                match case {
                    "slot" => {
                        bot.inventory.write().await.selected_hotbar = 1;
                        bot.common_receipts.lock().await.selected_hotbar =
                            Some(api::received(1, 101));
                    }
                    "item" => {
                        if let SlotKnowledge::Item { item } = &mut observed.as_mut().unwrap().value
                        {
                            item.id = other;
                        }
                    }
                    "empty" => observed.as_mut().unwrap().value = SlotKnowledge::Empty,
                    "unavailable" => observed.as_mut().unwrap().value = SlotKnowledge::Unavailable,
                    "unknown_item" => observed = None,
                    "predicted_item" => {
                        observed.as_mut().unwrap().source = crate::client::ValueSource::Predicted
                    }
                    "unknown_selection" => bot.common_receipts.lock().await.selected_hotbar = None,
                    "unsubmitted_selection" => bot.inventory.write().await.selected_hotbar = 2,
                    _ => unreachable!(),
                }
                bot.common_receipts.lock().await.inventory.slots[slot as usize] = observed;
                drop(gate);
                let error = pending.await.unwrap_err();
                assert_eq!(error.kind(), crate::ErrorKind::State, "{case}");
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
                if case == "unknown_item" || case == "predicted_item" {
                    assert_eq!(refusal.observed_item, None);
                }
                assert!(
                    timeout(Duration::from_millis(30), packets.recv())
                        .await
                        .is_err(),
                    "{case}"
                );
            }
            // Unchecked use keeps its original semantics with unknown identity.
            let sent_unchecked = match mode {
                api::GameMode::Survival => {
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
            assert_eq!(packets.recv().await.unwrap().0, 0x2d);
            release.send(()).unwrap();
            drop(client);
            drop(bot);
            server.await.unwrap();
        }
    }
}
