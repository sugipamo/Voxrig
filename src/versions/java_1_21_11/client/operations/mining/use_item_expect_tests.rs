use super::*;
use crate::client::{Hand, SlotKnowledge};

#[tokio::test]
async fn use_item_expect_checks_identity_after_lock_and_preserves_dispatch_receipts() {
    for mode in [GameMode::Survival, GameMode::Creative] {
        for hand in [Hand::Main, Hand::Off] {
            let mut f = Fixture::new().await;
            let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
            let item = client.registry().item("minecraft:dirt").unwrap().id;
            f.session.state.lock().await.operations.game_mode = Some(mode);
            f.api.select_hotbar(0).await.unwrap();
            read_packet(&mut f.peer, None).await.unwrap();
            let slot = if hand == Hand::Main { 36 } else { 45 };
            let mut payload = vec![0, 2, 0, slot, 1];
            put_varint(&mut payload, item.value());
            payload.extend([0, 0]);
            f.receive(ids::play_clientbound::SET_SLOT, &payload).await;
            let use_expected = || async {
                match mode {
                    GameMode::Survival => client.survival().use_item_expect(hand, 0, item).await,
                    _ => client.creative().use_item_expect(hand, 0, item).await,
                }
            };
            let sent = use_expected().await.unwrap();
            assert_eq!(sent.version, MinecraftVersion::Java1_21_11);
            let (id, bytes) = read_packet(&mut f.peer, None).await.unwrap();
            assert_eq!(id, ids::play_serverbound::USE_ITEM);
            assert_eq!(bytes[0], hand as u8);
            assert!(sent.interaction_sequence.is_some());
            crate::client::item_use::expect_test_contract(&client, hand, item).await;
            assert_eq!(
                read_packet(&mut f.peer, None).await.unwrap().0,
                ids::play_serverbound::USE_ITEM
            );
            // Poll while locked, then change the selection before validation runs.
            let mut gate = f.session.state.lock().await;
            let mut pending = Box::pin(use_expected());
            assert!(
                timeout(Duration::from_millis(20), pending.as_mut())
                    .await
                    .is_err()
            );
            gate.operations.selected_hotbar.as_mut().unwrap().slot = 1;
            drop(gate);
            let error = pending.await.unwrap_err();
            assert_eq!(error.kind(), ErrorKind::State);
            let refusal = error.use_item_refusal().unwrap();
            assert_eq!(refusal.expected_selected_hotbar, 0);
            assert_eq!(refusal.observed_selected_hotbar, Some(1));
            assert!(
                timeout(Duration::from_millis(30), read_packet(&mut f.peer, None))
                    .await
                    .is_err()
            );
            // Same selected slot, changed item: verify the slot under the same lock.
            let mut gate = f.session.state.lock().await;
            gate.operations.selected_hotbar.as_mut().unwrap().slot = 0;
            let mut pending = Box::pin(use_expected());
            assert!(
                timeout(Duration::from_millis(20), pending.as_mut())
                    .await
                    .is_err()
            );
            super::super::super::receive(
                &mut gate,
                ids::play_clientbound::SET_SLOT,
                &[0, 3, 0, slot, 0],
            )
            .unwrap();
            drop(gate);
            let error = pending.await.unwrap_err();
            assert_eq!(
                error.use_item_refusal().unwrap().observed_item,
                Some(SlotKnowledge::Empty)
            );
            assert!(
                timeout(Duration::from_millis(30), read_packet(&mut f.peer, None))
                    .await
                    .is_err()
            );
            // An unresolved submitted selection is unknown identity, with typed detail.
            f.session
                .state
                .lock()
                .await
                .operations
                .selected_hotbar
                .as_mut()
                .unwrap()
                .dispatched = false;
            assert_eq!(
                use_expected()
                    .await
                    .unwrap_err()
                    .use_item_refusal()
                    .unwrap()
                    .observed_selected_hotbar,
                None
            );
            f.stop().await;
        }
    }
}
