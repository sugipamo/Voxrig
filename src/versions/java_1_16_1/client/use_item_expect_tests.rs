use super::*;
use crate::client::{self as api, Hand, SlotKnowledge};
use tokio::time::timeout;

#[tokio::test]
async fn use_item_expect_checks_identity_after_lock_and_preserves_dispatch_receipts() {
    for mode in [api::GameMode::Survival, api::GameMode::Creative] {
        for hand in [Hand::Main, Hand::Off] {
            let (bot, mut packets, release, server) =
                super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
            let client = crate::Client::from_java_1_16_1(bot.clone());
            let item = client.registry().item("minecraft:dirt").unwrap().id;
            bot.survival.write().await.game_mode = Some(if mode == api::GameMode::Survival {
                0
            } else {
                1
            });
            bot.select_hotbar(0).await.unwrap();
            assert_eq!(packets.recv().await.unwrap(), (0x24, vec![0, 0]));
            assert_eq!(
                client
                    .player_state()
                    .await
                    .unwrap()
                    .selected_hotbar
                    .unwrap()
                    .source,
                api::ValueSource::Submitted
            );
            let slot = if hand == Hand::Main { 36i16 } else { 45 };
            let mut payload = vec![0];
            payload.extend(slot.to_be_bytes());
            payload.push(1);
            put_varint(&mut payload, item.value());
            payload.extend([1, 0]); // count 1, no NBT
            bot.apply_packet(0x16, payload).await.unwrap();
            let use_expected = || async {
                match mode {
                    api::GameMode::Survival => {
                        client.survival().use_item_expect(hand, 0, item).await
                    }
                    _ => client.creative().use_item_expect(hand, 0, item).await,
                }
            };
            let sent = use_expected().await.unwrap();
            assert_eq!(sent.version, crate::MinecraftVersion::Java1_16_1);
            assert_eq!(sent.interaction_sequence, None);
            assert_eq!(packets.recv().await.unwrap(), (0x2e, vec![hand as u8]));
            bot.use_item_expect(hand, 0, item).await.unwrap();
            assert_eq!(packets.recv().await.unwrap(), (0x2e, vec![hand as u8]));
            api::item_use::expect_test_contract(&client, hand, item).await;
            assert_eq!(packets.recv().await.unwrap(), (0x2e, vec![hand as u8]));
            let gate = bot.coherent_state_gate.lock().await;
            let mut pending = Box::pin(use_expected());
            assert!(
                timeout(Duration::from_millis(20), pending.as_mut())
                    .await
                    .is_err()
            );
            let mut native_pending = Box::pin(bot.use_item_expect(hand, 0, item));
            assert!(
                timeout(Duration::from_millis(20), native_pending.as_mut())
                    .await
                    .is_err()
            );
            bot.inventory.write().await.selected_hotbar = 1;
            bot.common_receipts.lock().await.selected_hotbar = Some(api::received(1, 100));
            drop(gate);
            let error = pending.await.unwrap_err();
            assert!(
                native_pending
                    .await
                    .unwrap_err()
                    .use_item_refusal()
                    .is_some()
            );
            assert_eq!(error.kind(), crate::ErrorKind::State);
            let refusal = error.use_item_refusal().unwrap();
            assert_eq!(refusal.expected_selected_hotbar, 0);
            assert_eq!(refusal.observed_selected_hotbar, Some(1));
            assert!(
                timeout(Duration::from_millis(30), packets.recv())
                    .await
                    .is_err()
            );
            let gate = bot.coherent_state_gate.lock().await;
            bot.inventory.write().await.selected_hotbar = 0;
            bot.common_receipts.lock().await.selected_hotbar = Some(api::received(0, 101));
            let mut pending = Box::pin(use_expected());
            assert!(
                timeout(Duration::from_millis(20), pending.as_mut())
                    .await
                    .is_err()
            );
            bot.common_receipts.lock().await.inventory.slots[slot as usize] =
                Some(api::received(SlotKnowledge::Empty, 102));
            drop(gate);
            let error = pending.await.unwrap_err();
            assert_eq!(
                error.use_item_refusal().unwrap().observed_item,
                Some(SlotKnowledge::Empty)
            );
            assert!(
                timeout(Duration::from_millis(30), packets.recv())
                    .await
                    .is_err()
            );
            // Native Bot shares the same identity contract.
            assert!(
                bot.use_item_expect(hand, 0, item)
                    .await
                    .unwrap_err()
                    .use_item_refusal()
                    .is_some()
            );
            // A compatibility-only native selection must not reuse a stale receipt.
            bot.inventory.write().await.selected_hotbar = 2;
            assert_eq!(
                use_expected()
                    .await
                    .unwrap_err()
                    .use_item_refusal()
                    .unwrap()
                    .observed_selected_hotbar,
                None
            );
            bot.common_receipts.lock().await.selected_hotbar = None;
            assert_eq!(
                use_expected()
                    .await
                    .unwrap_err()
                    .use_item_refusal()
                    .unwrap()
                    .observed_selected_hotbar,
                None
            );
            release.send(()).unwrap();
            drop(client);
            drop(bot);
            server.await.unwrap();
        }
    }
}
