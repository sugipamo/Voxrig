use super::*;
use std::sync::atomic::AtomicUsize;

pub(crate) trait Fixture {
    fn client(&self) -> Client;
    async fn mode(&mut self, mode: GameMode);
    async fn slot(&mut self, slot: i16, item: Option<RegistryId>);
    async fn packet(&mut self) -> (i32, Vec<u8>);
    async fn silent(&mut self);
    /// Received column with all-air terrain.
    async fn chunk(&mut self, column: [i32; 2]);
    async fn unload(&mut self, column: [i32; 2]);
    async fn block(&mut self, p: [i32; 3], state: i32);
    /// One multi-block packet; all cells share one section.
    async fn section(&mut self, cells: &[[i32; 3]], state: i32);
    /// Entity movement and world time, the dominant non-terrain traffic.
    async fn unrelated(&mut self);
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

/// Reads the cut's cells and player receive ordinal; the guard refuses, so no frame.
async fn cells(client: &Client, reads: &[[i32; 3]]) -> (Vec<([i32; 3], Option<u64>)>, u64) {
    let seen = Arc::new(std::sync::Mutex::new(None));
    let sink = seen.clone();
    let result = client
        .survival()
        .use_item_guarded_with_reads(Hand::Main, None, reads, move |cut| {
            *sink.lock().unwrap() = Some((cut.cells.clone(), cut.player.receive_sequence));
            false
        })
        .await
        .unwrap();
    let GuardedUseResult::NotSent {
        reason: UseNotSentReason::GuardRejected,
        cut: Some(cut),
    } = result
    else {
        panic!("guard did not run");
    };
    let (cells, sequence) = seen.lock().unwrap().take().unwrap();
    assert_eq!(cut.cells, cells);
    assert_eq!(cut.player.receive_sequence, sequence);
    for (_, ordinal) in &cells {
        // Frozen with the player observation: never ahead of its boundary.
        assert!(ordinal.is_none_or(|o| o <= sequence));
    }
    (cells, sequence)
}
pub(crate) async fn read_cells(f: &mut impl Fixture) {
    let client = f.client();
    f.mode(GameMode::Survival).await;
    let (target, halo, upper, far) = ([161, 2, 163], [162, 3, 163], [161, 40, 163], [200, 2, 50]);
    let outside = [161, 400, 163];
    let reads = [target, halo, upper, far, outside, target];
    let (before, _) = cells(&client, &reads).await;
    assert!(
        before.iter().all(|(_, o)| o.is_none()),
        "unloaded: {before:?}"
    );
    assert_eq!(
        before.iter().map(|(p, _)| *p).collect::<Vec<_>>(),
        reads,
        "request order and duplicates are kept"
    );
    f.chunk([10, 10]).await;
    f.chunk([12, 3]).await;
    let (loaded, at_load) = cells(&client, &reads).await;
    let load = loaded[0].1.expect("loaded cell");
    assert!(load > 0 && load <= at_load);
    assert_eq!(loaded[1].1, Some(load));
    assert_eq!(loaded[2].1, Some(load));
    assert_eq!(loaded[3].1.map(|o| o > load), Some(true), "own column load");
    assert_eq!(loaded[4].1, None, "outside the dimension");
    assert_eq!(loaded[5].1, Some(load));

    // A block update on a read cell advances exactly that cell.
    f.block(target, 1).await;
    let (changed, at_change) = cells(&client, &reads).await;
    assert_eq!(changed[0].1, Some(at_change), "same boundary as the player");
    assert_eq!(changed[5].1, Some(at_change));
    assert_eq!(changed[1].1, Some(load));
    assert_eq!(changed[2].1, Some(load));
    let far_load = changed[3].1;

    // Far block, entity and time traffic advance the global ordinal only.
    f.block([195, 2, 52], 1).await;
    f.block([161, 60, 163], 1).await;
    f.unrelated().await;
    let (quiet, at_quiet) = cells(&client, &reads).await;
    assert!(at_quiet > at_change);
    assert_eq!(quiet[0].1, Some(at_change));
    assert_eq!(quiet[1].1, Some(load));
    assert_eq!(quiet[2].1, Some(load));
    assert_eq!(quiet[3].1, far_load);

    // A multi-block change touching a read cell advances it, even unchanged.
    f.section(&[[160, 0, 160], halo, target], 1).await;
    let (multi, at_multi) = cells(&client, &reads).await;
    assert_eq!(multi[0].1, Some(at_multi));
    assert_eq!(multi[1].1, Some(at_multi));
    assert_eq!(multi[2].1, Some(load));

    // Unload forgets the column; a reload supersedes its earlier cells.
    f.unload([10, 10]).await;
    let (gone, _) = cells(&client, &reads).await;
    assert_eq!(gone[..3].iter().map(|c| c.1).collect::<Vec<_>>(), [None; 3]);
    assert_eq!(gone[3].1, far_load);
    f.chunk([10, 10]).await;
    let (reloaded, at_reload) = cells(&client, &reads).await;
    let reload = reloaded[0].1.unwrap();
    assert!(reload > at_multi && reload <= at_reload);
    assert_eq!(reloaded[1].1, Some(reload));

    // The read set is bounded: rejected before any wait, guard or frame.
    let many = vec![target; MAX_GUARDED_READS + 1];
    let error = client
        .survival()
        .use_item_guarded_with_reads(Hand::Main, None, &many, |_| panic!("over bound"))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), crate::ErrorKind::InvalidInput);
    let max = vec![target; MAX_GUARDED_READS];
    assert_eq!(cells(&client, &max).await.0.len(), MAX_GUARDED_READS);
    // Existing APIs carry no cells, and the block variant takes the same reads.
    let plain = client
        .survival()
        .use_item_guarded(Hand::Main, None, |cut| {
            assert!(cut.cells.is_empty());
            false
        })
        .await
        .unwrap();
    assert!(matches!(plain, GuardedUseResult::NotSent { cut: Some(c), .. } if c.cells.is_empty()));
    let block = client
        .survival()
        .use_on_block_guarded_with_reads(
            BlockUseTarget {
                position: [1, 0, 0],
                face: crate::BlockFace::Up,
                cursor: [0.5; 3],
            },
            Hand::Main,
            None,
            &[target],
            |_| false,
        )
        .await
        .unwrap();
    assert!(
        matches!(block, GuardedUseResult::NotSent { cut: Some(c), .. } if c.cells == [(target, Some(reload))])
    );
    f.silent().await;
}

type Captured = (
    Vec<([i32; 3], Option<u64>)>,
    Vec<(GuardedReadBox, Option<u64>)>,
    u64,
);
/// One gated cut with both cells and boxes; the guard refuses, so no frame.
async fn reads(client: &Client, cells: &[[i32; 3]], boxes: &[GuardedReadBox]) -> Captured {
    let seen = Arc::new(std::sync::Mutex::new(None));
    let sink = seen.clone();
    let request = Request {
        hand: Hand::Main,
        target: None,
        expected: None,
    };
    let result = client
        .guarded_use(
            Some(GameMode::Survival),
            request,
            Reads { cells, boxes },
            Box::new(move |cut| {
                let frozen = (cut.cells.clone(), cut.boxes.clone());
                *sink.lock().unwrap() = Some((frozen, cut.player.receive_sequence));
                false
            }),
        )
        .await
        .unwrap();
    let GuardedUseResult::NotSent {
        reason: UseNotSentReason::GuardRejected,
        cut: Some(cut),
    } = result
    else {
        panic!("guard did not run");
    };
    let ((cells, boxes), sequence) = seen.lock().unwrap().take().unwrap();
    assert_eq!((&cut.cells, &cut.boxes), (&cells, &boxes));
    for ordinal in cells.iter().map(|c| c.1).chain(boxes.iter().map(|b| b.1)) {
        // Frozen with the player observation: never ahead of its boundary.
        assert!(ordinal.is_none_or(|o| o <= sequence));
    }
    (cells, boxes, sequence)
}
async fn latest(client: &Client, boxes: &[GuardedReadBox]) -> (Vec<Option<u64>>, u64) {
    let (_, read, sequence) = reads(client, &[], boxes).await;
    assert_eq!(read.iter().map(|b| b.0).collect::<Vec<_>>(), boxes);
    (read.into_iter().map(|b| b.1).collect(), sequence)
}
/// A 4×4×4 box (64 cells) read in the same cut as all its cells: the box is
/// exactly the latest of its cells.
async fn consistent(client: &Client, b: GuardedReadBox) -> Option<u64> {
    let mut cells = Vec::new();
    for x in b.min[0]..=b.max[0] {
        for y in b.min[1]..=b.max[1] {
            for z in b.min[2]..=b.max[2] {
                cells.push([x, y, z]);
            }
        }
    }
    assert_eq!(cells.len(), MAX_GUARDED_READS);
    let (cells, boxes, _) = reads(client, &cells, &[b]).await;
    let max = cells.iter().try_fold(0, |m, c| c.1.map(|o| o.max(m)));
    assert_eq!(boxes, [(b, max)], "box is the latest of its cells");
    max
}
pub(crate) async fn read_boxes(f: &mut impl Fixture) {
    let client = f.client();
    f.mode(GameMode::Survival).await;
    let b = |min, max| GuardedReadBox { min, max };
    // Column (10, 10) is x/z 160..=175; column (11, 10) starts at x = 176.
    let halo = b([160, 0, 160], [164, 4, 164]);
    let straddle = b([174, 2, 160], [177, 4, 162]);
    let tall = b([165, 10, 165], [165, 40, 165]);
    let upper = b([165, 32, 165], [165, 40, 165]);
    let below = b([160, -70, 160], [160, 0, 160]);
    let small = b([160, 0, 160], [163, 3, 163]);
    let boxes = [halo, straddle, tall, upper, below];
    let (before, _) = latest(&client, &boxes).await;
    assert_eq!(before, [None; 5], "unloaded");

    f.chunk([10, 10]).await;
    let (loaded, at_load) = latest(&client, &boxes).await;
    let load = loaded[0].expect("loaded box");
    assert!(load <= at_load);
    assert_eq!(loaded, [Some(load), None, Some(load), Some(load), None]);
    f.chunk([11, 10]).await;
    let (both, _) = latest(&client, &boxes).await;
    let load2 = both[1].expect("both columns loaded");
    assert!(load2 > load, "latest column load wins");
    assert_eq!(consistent(&client, small).await, Some(load));

    // A block update inside a box advances it to the cut's boundary; one
    // outside does not, and neither does entity or time traffic.
    f.block([162, 2, 162], 1).await;
    let (changed, at_change) = latest(&client, &boxes).await;
    assert_eq!(changed[0], Some(at_change), "same boundary as the player");
    assert_eq!(changed[1..4], [Some(load2), Some(load), Some(load)]);
    f.block([166, 2, 166], 1).await;
    f.block([165, 41, 165], 1).await;
    f.block([178, 2, 160], 1).await;
    f.unrelated().await;
    let (quiet, at_quiet) = latest(&client, &boxes).await;
    assert!(at_quiet > at_change);
    assert_eq!(quiet, changed, "outside touches leave every box unchanged");
    assert_eq!(consistent(&client, small).await, Some(at_change));

    // Multi-block change with a cell inside the halo and the straddle box.
    f.section(&[[170, 3, 170], [160, 0, 160]], 1).await;
    let (multi, at_multi) = latest(&client, &boxes).await;
    assert_eq!(multi[0], Some(at_multi));
    assert_eq!(multi[1..4], [Some(load2), Some(load), Some(load)]);
    assert_eq!(consistent(&client, small).await, Some(at_multi));
    f.section(&[[176, 4, 162]], 1).await;
    let (multi2, at_multi2) = latest(&client, &boxes).await;
    assert_eq!(multi2[1], Some(at_multi2), "touched in the second column");
    assert_eq!(multi2[0], Some(at_multi));

    // More than 64 touched cells in a section collapse into its floor: every
    // box overlapping that section reports at least that packet, never earlier.
    let busy: Vec<[i32; 3]> = (0..65).map(|i| [166 + i % 10, 8, 166 + i / 10]).collect();
    f.section(&busy, 1).await;
    let (floor, at_floor) = latest(&client, &boxes).await;
    assert_eq!(
        floor[0],
        Some(at_floor),
        "conservative: halo shares section 0"
    );
    assert_eq!(floor[2], Some(at_floor), "tall box overlaps section 0");
    assert_eq!(floor[3], Some(load), "section 2 is untouched");
    assert_eq!(
        floor[1],
        Some(at_floor),
        "straddle overlaps the busy section"
    );
    assert_eq!(consistent(&client, small).await, Some(at_floor));

    // Chunk data inside a box supersedes and advances it.
    f.chunk([10, 10]).await;
    let (reloaded, at_reload) = latest(&client, &boxes).await;
    let reload = reloaded[0].unwrap();
    assert!(reload > at_floor && reload <= at_reload);
    assert_eq!(reloaded[..4], [Some(reload); 4]);
    assert_eq!(consistent(&client, small).await, Some(reload));

    // Unloading any part of a box makes the whole box unknown.
    f.unload([11, 10]).await;
    let (gone, _) = latest(&client, &boxes).await;
    assert_eq!(gone, [Some(reload), None, Some(reload), Some(reload), None]);

    // Bounds: rejected before any wait, guard or frame; never truncated.
    let section = b([0, 0, 0], [15, 15, 15]);
    let rejected = [
        vec![halo; MAX_GUARDED_BOXES + 1],
        vec![b([0, 0, 0], [16, 15, 15])],
        vec![b([0, 1, 0], [0, 0, 0])],
        vec![halo, b([i32::MIN; 3], [i32::MAX; 3])],
    ];
    for boxes in &rejected {
        let error = client
            .survival()
            .use_item_guarded_with_box_reads(Hand::Main, None, boxes, |_| panic!("over bound"))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), crate::ErrorKind::InvalidInput);
        let error = client
            .creative()
            .use_on_block_guarded_with_box_reads(
                BlockUseTarget {
                    position: [1, 0, 0],
                    face: crate::BlockFace::Up,
                    cursor: [0.5; 3],
                },
                Hand::Main,
                None,
                boxes,
                |_| panic!("over bound"),
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind(), crate::ErrorKind::InvalidInput);
    }
    f.silent().await;
    let max = vec![section; MAX_GUARDED_BOXES];
    assert_eq!(latest(&client, &max).await.0, [None; MAX_GUARDED_BOXES]);

    // Box reads leave cells empty, cell reads and existing APIs leave boxes
    // empty, and a passing guard sends exactly one use frame.
    let item = client
        .survival()
        .use_item_guarded_with_reads(Hand::Main, None, &[[162, 2, 162]], |cut| {
            assert_eq!(cut.cells.len(), 1);
            false
        })
        .await
        .unwrap();
    assert!(matches!(item, GuardedUseResult::NotSent { cut: Some(c), .. } if c.boxes.is_empty()));
    let plain = client
        .survival()
        .use_item_guarded(Hand::Main, None, |_| false)
        .await
        .unwrap();
    assert!(
        matches!(plain, GuardedUseResult::NotSent { cut: Some(c), .. } if c.boxes.is_empty() && c.cells.is_empty())
    );
    let block = client
        .survival()
        .use_on_block_guarded_with_box_reads(
            BlockUseTarget {
                position: [1, 0, 0],
                face: crate::BlockFace::Up,
                cursor: [0.5; 3],
            },
            Hand::Main,
            None,
            &[halo],
            |_| false,
        )
        .await
        .unwrap();
    assert!(
        matches!(block, GuardedUseResult::NotSent { cut: Some(c), .. } if c.cells.is_empty() && c.boxes == [(halo, Some(reload))])
    );
    f.silent().await;
    let sent = client
        .survival()
        .use_item_guarded_with_box_reads(Hand::Main, None, &[halo], |cut| cut.boxes[0].1.is_some())
        .await
        .unwrap();
    let GuardedUseResult::Sent { cut, .. } = sent else {
        panic!("box read guard passed but nothing was sent");
    };
    assert_eq!(cut.boxes, [(halo, Some(reload))]);
    f.packet().await;
    f.silent().await;
}
