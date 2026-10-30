use super::*;
use std::sync::atomic::AtomicUsize;

pub(crate) trait Fixture {
    fn client(&self) -> Client;
    async fn mode(&mut self, mode: GameMode);
    async fn slot(&mut self, slot: i16, item: Option<RegistryId>);
    async fn packet(&mut self) -> (i32, Vec<u8>);
    async fn silent(&mut self);
}
pub(crate) async fn dispatch(
    client: &Client,
    block: bool,
    hand: Hand,
    expected: Option<HeldItemExpectation>,
    check: Check,
) -> Result<GuardedUseResult> {
    let target = BlockUseTarget {
        position: [1, 0, 0],
        face: crate::BlockFace::Up,
        cursor: [0.5; 3],
    };
    if client.player_state().await?.game_mode == Some(GameMode::Creative) {
        if block {
            client
                .creative()
                .use_on_block_guarded(target, hand, expected, check)
                .await
        } else {
            client
                .creative()
                .use_item_guarded(hand, expected, check)
                .await
        }
    } else if block {
        client
            .survival()
            .use_on_block_guarded(target, hand, expected, check)
            .await
    } else {
        client
            .survival()
            .use_item_guarded(hand, expected, check)
            .await
    }
}
pub(crate) async fn contract(f: &mut impl Fixture) {
    let client = f.client();
    let item = client.registry().item("minecraft:dirt").unwrap().id;
    let ids = if client.version() == crate::MinecraftVersion::Java1_16_1 {
        [0x2e, 0x2d]
    } else {
        [0x40, 0x3f]
    };
    for mode in [GameMode::Survival, GameMode::Creative] {
        f.mode(mode).await;
        for hand in [Hand::Main, Hand::Off] {
            let slot = if hand == Hand::Main { 36 } else { 45 };
            f.slot(slot, Some(item)).await;
            for block in [false, true] {
                for reason in [
                    UseNotSentReason::GuardRejected,
                    UseNotSentReason::GuardPanicked,
                ] {
                    let calls = Arc::new(AtomicUsize::new(0));
                    let count = calls.clone();
                    let result = dispatch(&client, block, hand, Some(HeldItemExpectation { selected_hotbar: 0, item: Some(item) }), Box::new(move |cut| {
                        count.fetch_add(1, Ordering::SeqCst);
                        assert_eq!(cut.slot, Some(slot as usize));
                        assert!(matches!(&cut.held.as_ref().unwrap().value, SlotKnowledge::Item { item: i } if i.id == item));
                        if reason == UseNotSentReason::GuardPanicked { panic!("test guard panic"); }
                        false
                    })).await.unwrap();
                    assert_eq!(result.outcome(), super::super::OperationOutcome::NotSent);
                    assert!(
                        matches!(result, GuardedUseResult::NotSent { reason: actual, cut: Some(_) } if actual == reason)
                    );
                    assert_eq!(calls.load(Ordering::SeqCst), 1);
                    f.silent().await;
                }
                let calls = Arc::new(AtomicUsize::new(0));
                let count = calls.clone();
                let result = dispatch(
                    &client,
                    block,
                    hand,
                    None,
                    Box::new(move |_| {
                        count.fetch_add(1, Ordering::SeqCst);
                        true
                    }),
                )
                .await
                .unwrap();
                assert_eq!(
                    result.outcome(),
                    super::super::OperationOutcome::SentUnconfirmed
                );
                let GuardedUseResult::Sent { receipt, cut } = result else {
                    panic!("not sent");
                };
                assert_eq!(receipt.version, client.version());
                assert_eq!(cut.hand, hand);
                assert_eq!(calls.load(Ordering::SeqCst), 1);
                let (id, bytes) = f.packet().await;
                assert_eq!(id, ids[usize::from(block)]);
                assert_eq!(bytes[0], hand as u8);
                f.silent().await; // One invocation produced exactly one frame.
            }
            let target = BlockUseTarget {
                position: [1, 0, 0],
                face: crate::BlockFace::Up,
                cursor: [0.5; 3],
            };
            // Received item cannot satisfy explicit empty expectation; check is not called.
            let empty = HeldItemExpectation {
                selected_hotbar: 0,
                item: None,
            };
            let result = dispatch(
                &client,
                true,
                hand,
                Some(empty),
                Box::new(|_| panic!("mismatch must not invoke guard")),
            )
            .await
            .unwrap();
            assert!(matches!(
                result,
                GuardedUseResult::NotSent {
                    reason: UseNotSentReason::ExpectationMismatch,
                    ..
                }
            ));
            f.silent().await;
            f.slot(slot, None).await;
            let result = if mode == GameMode::Creative {
                client
                    .creative()
                    .use_on_block_expect_slot(target, hand, empty)
                    .await
            } else {
                client
                    .survival()
                    .use_on_block_expect_slot(target, hand, empty)
                    .await
            }
            .unwrap();
            let GuardedUseResult::Sent { cut, .. } = result else {
                panic!("empty expected send");
            };
            assert!(cut.matches(empty));
            for source in [
                super::super::ValueSource::Predicted,
                super::super::ValueSource::LocalCache,
                super::super::ValueSource::Submitted,
            ] {
                let mut unknown = cut.clone();
                unknown.held.as_mut().unwrap().source = source;
                assert!(!unknown.matches(empty));
            }
            let mut unknown = cut.clone();
            unknown.held = None;
            assert!(!unknown.matches(empty));
            let mut unknown = cut.clone();
            unknown.held.as_mut().unwrap().value = SlotKnowledge::Unavailable;
            assert!(!unknown.matches(empty));
            let mut unknown = cut.clone();
            unknown.player.selected_hotbar = None;
            assert!(!unknown.matches(empty));
            assert_eq!(f.packet().await.0, ids[1]);
            f.silent().await;
            let result = dispatch(
                &client,
                true,
                hand,
                Some(HeldItemExpectation {
                    selected_hotbar: 0,
                    item: Some(item),
                }),
                Box::new(|_| panic!("empty is not item")),
            )
            .await
            .unwrap();
            assert!(matches!(
                result,
                GuardedUseResult::NotSent {
                    reason: UseNotSentReason::ExpectationMismatch,
                    ..
                }
            ));
            f.silent().await;
            // Original expect still refuses empty; unchecked original still sends.
            let old = if mode == GameMode::Creative {
                client
                    .creative()
                    .use_on_block_expect(target.position, target.face, target.cursor, hand, 0, item)
                    .await
            } else {
                client
                    .survival()
                    .use_on_block_expect(target.position, target.face, target.cursor, hand, 0, item)
                    .await
            };
            assert!(old.unwrap_err().use_item_refusal().is_some());
            f.silent().await;
            let old = if mode == GameMode::Creative {
                client
                    .creative()
                    .use_on_block(target.position, target.face, target.cursor, hand)
                    .await
            } else {
                client
                    .survival()
                    .use_on_block(target.position, target.face, target.cursor, hand)
                    .await
            }
            .unwrap();
            assert_eq!(old.version, client.version());
            assert_eq!(f.packet().await.0, ids[1]);
        }
    }
}
