//! Constructed native packets exercise the connection-owned stair record.
use super::*;
use crate::client::{adapter::PlacementOps, survival::PlacementStage};
fn stair(facing: &str, half: &str, shape: &str) -> crate::NativeBlockState {
    crate::NativeBlockState {
        name: "minecraft:oak_stairs".into(),
        properties: [
            ("facing".into(), facing.into()),
            ("half".into(), half.into()),
            ("shape".into(), shape.into()),
            ("waterlogged".into(), "false".into()),
        ]
        .into(),
    }
}
async fn stack(f: &mut Fixture, count: i32) {
    let mut p = vec![0, 2];
    p.extend(36i16.to_be_bytes());
    put_varint(&mut p, count);
    if count > 0 {
        put_varint(&mut p, default_item("oak_stairs", 1).unwrap().item_id);
        p.extend([0, 0]);
    }
    f.receive(ids::play_clientbound::SET_SLOT, &p).await;
}
const NEIGHBOR: [i32; 3] = [1, 2, 1];
async fn start(f: &mut Fixture, count: i32) -> crate::client::survival::StairPlacementRecord {
    stack(f, count).await;
    let sent = f
        .api
        .place_stair(SUPPORT, crate::BlockFace::West, TARGET, native("air"))
        .await
        .unwrap();
    assert_eq!(sent.expected, stair("east", "top", "straight"));
    assert_eq!(sent.first_outline.position, SUPPORT);
    assert!(sent.neighbor_updates.is_empty());
    let (id, p) = f.expected_packet("stair BLOCK_PLACE").await;
    assert_eq!(id, ids::play_serverbound::BLOCK_PLACE);
    let mut r = Reader::new(&p);
    assert_eq!(r.varint().unwrap(), 0);
    assert_eq!(r.take(8).unwrap(), pack_position(SUPPORT).to_be_bytes());
    assert_eq!(r.varint().unwrap(), 4);
    for v in sent.cursor {
        assert_eq!(r.f32().unwrap(), v);
    }
    assert!(!r.bool().unwrap());
    assert!(!r.bool().unwrap());
    assert_eq!(r.varint().unwrap(), sent.send.interaction_sequence.unwrap());
    r.end().unwrap();
    sent
}
#[tokio::test]
async fn stair_all_receipt_orders_last_material_and_history_freeze_on_receive() {
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        for count in [1, 5] {
            let mut f = Fixture::new().await;
            let sent = start(&mut f, count).await;
            assert!(f.api.placement_record().await.unwrap().is_none());
            assert!(f.api.slab_placement_record().await.unwrap().is_none());
            assert!(f.api.select_hotbar(1).await.is_err());
            assert!(
                f.api
                    .place_stair(SUPPORT, crate::BlockFace::West, TARGET, native("air"))
                    .await
                    .is_err()
            );
            for (index, action) in order.into_iter().enumerate() {
                match action {
                    0 => replacement_block(&mut f, TARGET, &sent.expected).await,
                    1 => stack(&mut f, count - 1).await,
                    _ => f.ack(sent.send.interaction_sequence.unwrap()).await,
                }
                if index < 2 {
                    assert_eq!(
                        f.api.stair_placement_record().await.unwrap().unwrap().stage,
                        PlacementStage::Pending
                    );
                }
            }
            // Complete on receive, before the first completed getter.
            f.block(TARGET, "stone").await;
            f.stack(12).await;
            f.ack(100).await;
            let complete = f.api.stair_placement_record().await.unwrap().unwrap();
            assert_eq!(
                complete.stage,
                PlacementStage::ObservedPlaced,
                "{:?}",
                complete.requires_inspection
            );
            assert_eq!(complete.target_receipt.unwrap().value, sent.expected);
            assert!(complete.processing.unwrap().receive_sequence > sent.send.after_sequence);
            if count == 1 {
                assert!(matches!(
                    complete.material_receipt.unwrap().value,
                    crate::client::SlotKnowledge::Empty
                ));
            }
            f.stop().await;
        }
    }
}
#[tokio::test]
async fn stair_neighbor_receipt_is_required_separately_and_conflicts_are_latched() {
    for conflict in [false, true] {
        let mut f = Fixture::new().await;
        stack(&mut f, 1).await;
        // Same half, perpendicular south neighbor changes to outer_right.
        replacement_block(&mut f, NEIGHBOR, &stair("north", "top", "straight")).await;
        let sent = f
            .api
            .place_stair(SUPPORT, crate::BlockFace::West, TARGET, native("air"))
            .await
            .unwrap();
        assert_eq!(sent.expected, stair("east", "top", "straight"));
        assert_eq!(sent.neighbor_updates.len(), 1);
        let neighbor = sent.neighbor_updates[0].clone();
        assert_ne!(neighbor.before, neighbor.expected);
        f.expected_packet("stair").await;
        replacement_block(&mut f, TARGET, &sent.expected).await;
        stack(&mut f, 0).await;
        f.ack(sent.send.interaction_sequence.unwrap()).await;
        let pending = f.api.stair_placement_record().await.unwrap().unwrap();
        assert_eq!(pending.stage, PlacementStage::Pending);
        assert!(pending.neighbor_updates[0].receipt.is_none());
        if conflict {
            f.block(NEIGHBOR, "stone").await;
        }
        replacement_block(&mut f, NEIGHBOR, &neighbor.expected).await;
        let record = f.api.stair_placement_record().await.unwrap().unwrap();
        assert_eq!(
            record.stage,
            if conflict {
                PlacementStage::RequiresInspection
            } else {
                PlacementStage::ObservedPlaced
            }
        );
        if !conflict {
            let frozen = serde_json::to_value(&record).unwrap();
            f.block(NEIGHBOR, "air").await;
            assert_eq!(
                serde_json::to_value(f.api.stair_placement_record().await.unwrap().unwrap())
                    .unwrap(),
                frozen
            );
        }
        f.stop().await;
    }
}
#[tokio::test]
async fn stair_conflicting_diagonal_input_is_latched() {
    // A neighbor facing east reads its east/west cells, diagonal to the target.
    let mut f = Fixture::new().await;
    stack(&mut f, 1).await;
    replacement_block(&mut f, NEIGHBOR, &stair("east", "top", "straight")).await;
    let sent = f
        .api
        .place_stair(SUPPORT, crate::BlockFace::West, TARGET, native("air"))
        .await
        .unwrap();
    f.expected_packet("stair").await;
    let diagonal = [2, 2, 1];
    assert!(sent.shape_inputs.iter().any(|i| i.position == diagonal));
    f.block(diagonal, "stone").await;
    f.block(diagonal, "air").await;
    assert_eq!(
        f.api.stair_placement_record().await.unwrap().unwrap().stage,
        PlacementStage::RequiresInspection
    );
    f.stop().await;
}
#[tokio::test]
async fn stair_wrong_destination_material_wet_neighbor_and_cancelled_writer_do_not_resend() {
    for case in ["target", "before", "material", "wet"] {
        let mut f = Fixture::new().await;
        stack(&mut f, 1).await;
        if case == "material" {
            f.stack(1).await;
        }
        if case == "wet" {
            let mut wet = stair("north", "top", "straight");
            wet.properties.insert("waterlogged".into(), "true".into());
            replacement_block(&mut f, NEIGHBOR, &wet).await;
        }
        let result = f
            .api
            .place_stair(
                SUPPORT,
                crate::BlockFace::West,
                if case == "target" { [0, 2, 0] } else { TARGET },
                if case == "before" {
                    native("stone")
                } else {
                    native("air")
                },
            )
            .await;
        assert!(result.is_err(), "{case}");
        assert!(f.api.stair_placement_record().await.unwrap().is_none());
        assert!(
            timeout(Duration::from_millis(10), read_packet(&mut f.peer, None))
                .await
                .is_err()
        );
        f.stop().await;
    }
    let mut f = Fixture::new().await;
    stack(&mut f, 1).await;
    let writer = f.api.bot.session.writer.lock().await;
    assert!(
        timeout(
            Duration::from_millis(20),
            f.api
                .place_stair(SUPPORT, crate::BlockFace::West, TARGET, native("air"))
        )
        .await
        .is_err()
    );
    drop(writer);
    let intent = f.api.stair_placement_record().await.unwrap().unwrap();
    assert!(!intent.send.dispatched);
    replacement_block(&mut f, TARGET, &intent.expected).await;
    stack(&mut f, 0).await;
    f.ack(intent.send.interaction_sequence.unwrap()).await;
    assert_ne!(
        f.api.stair_placement_record().await.unwrap().unwrap().stage,
        PlacementStage::ObservedPlaced
    );
    assert!(
        timeout(Duration::from_millis(10), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    f.stop().await;
}

#[tokio::test]
async fn stair_partial_write_retains_inspection_and_never_replays() {
    use crate::client::survival::PlacementStage;
    let mut f = Fixture::new().await;
    stack(&mut f, 1).await;
    let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
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
    let survival = client.survival();
    let waiter = tokio::spawn(async move {
        survival
            .place_stair(SUPPORT, crate::BlockFace::West, TARGET, native("air"))
            .await
    });
    control.until(|w| w.bytes().len() == 1).await;
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    let record = client
        .survival()
        .stair_placement_record()
        .await
        .unwrap()
        .unwrap();
    assert!(!record.send.dispatched);
    assert_eq!(record.stage, PlacementStage::RequiresInspection);
    assert!(
        client
            .survival()
            .place_stair(SUPPORT, crate::BlockFace::West, TARGET, native("air"))
            .await
            .is_err()
    );
    assert!(f.api.bot.session.ledger.cut(false).wait().await.is_err());
    assert_eq!(control.bytes().len(), 1);
    f.stop().await;
}

#[tokio::test]
async fn stair_neighbor_all_receipt_orders_and_successor_history() {
    for a in 0..4 {
        for b in 0..4 {
            for c in 0..4 {
                for d in 0..4 {
                    let order = [a, b, c, d];
                    if (0..4).any(|i| (i + 1..4).any(|j| order[i] == order[j])) {
                        continue;
                    }
                    let mut f = Fixture::new().await;
                    stack(&mut f, 1).await;
                    replacement_block(&mut f, NEIGHBOR, &stair("north", "top", "straight")).await;
                    let sent = f
                        .api
                        .place_stair(SUPPORT, crate::BlockFace::West, TARGET, native("air"))
                        .await
                        .unwrap();
                    f.expected_packet("stairs").await;
                    let neighbor = sent.neighbor_updates[0].clone();
                    for (index, action) in order.into_iter().enumerate() {
                        match action {
                            0 => replacement_block(&mut f, TARGET, &sent.expected).await,
                            1 => stack(&mut f, 0).await,
                            2 => f.ack(sent.send.interaction_sequence.unwrap()).await,
                            _ => replacement_block(&mut f, NEIGHBOR, &neighbor.expected).await,
                        }
                        if index < 3 {
                            assert_eq!(
                                f.api.stair_placement_record().await.unwrap().unwrap().stage,
                                PlacementStage::Pending
                            );
                        }
                    }
                    // An adjacent receipt after completion cannot rewrite the completed one.
                    f.block(NEIGHBOR, "air").await;
                    let history = f.api.stair_placement_record().await.unwrap().unwrap();
                    assert_eq!(history.stage, PlacementStage::ObservedPlaced);
                    assert_eq!(
                        history.neighbor_updates[0].receipt.as_ref().unwrap().value,
                        neighbor.expected
                    );
                    let frozen = serde_json::to_value(history).unwrap();
                    f.block(TARGET, "air").await;
                    f.stack(5).await;
                    let cube = f
                        .api
                        .place_cube(SUPPORT, crate::BlockFace::West)
                        .await
                        .unwrap();
                    assert!(cube.send.dispatched);
                    f.expected_packet("next cube").await;
                    assert_eq!(
                        serde_json::to_value(
                            f.api.stair_placement_record().await.unwrap().unwrap()
                        )
                        .unwrap(),
                        frozen
                    );
                    f.stop().await;
                }
            }
        }
    }
}
#[tokio::test]
async fn stair_building_entity_after_send_and_stale_ack_cannot_confirm() {
    for entity in [false, true] {
        let mut f = Fixture::new().await;
        let sent = start(&mut f, 1).await;
        if entity {
            slab_fixed_entity(&mut f, "tnt", [1.5, 2.7, 0.5]).await;
        } else {
            let mut state = f.api.bot.session.state.lock().await;
            state.operations.ack = sent.send.interaction_sequence;
            state.operations.ack_receive_sequence = Some(sent.send.after_sequence);
        }
        replacement_block(&mut f, TARGET, &sent.expected).await;
        stack(&mut f, 0).await;
        assert_eq!(
            f.api.stair_placement_record().await.unwrap().unwrap().stage,
            if entity {
                PlacementStage::RequiresInspection
            } else {
                PlacementStage::Pending
            }
        );
        f.stop().await;
    }
}
#[tokio::test]
async fn stair_missing_diagonal_chunk_at_capture_and_later_chunk_replacement_are_not_air() {
    struct Cells(std::collections::BTreeMap<[i32; 3], crate::NativeBlockState>);
    impl GeometryView for Cells {
        fn block(&self, p: [i32; 3]) -> Result<crate::NativeBlockState> {
            self.0
                .get(&p)
                .cloned()
                .ok_or_else(|| unavailable("missing diagonal chunk"))
        }
    }
    let mut f = Fixture::new().await;
    let sent = start(&mut f, 1).await;
    let mut intent = f
        .api
        .bot
        .session
        .state
        .lock()
        .await
        .placement
        .as_ref()
        .unwrap()
        .intent
        .clone();
    intent.target = [15, 2, 15];
    let mut cells = Cells(Default::default());
    for x in 13..=17 {
        for z in 13..=17 {
            cells.0.insert([x, 2, z], native("air"));
        }
    }
    cells
        .0
        .insert([15, 2, 16], stair("east", "top", "straight"));
    let capture = stair::capture(&cells, &intent).unwrap();
    let diagonal = [16, 2, 16];
    assert!(capture.inputs.iter().any(|i| i.position == diagonal));
    cells.0.remove(&diagonal);
    assert!(stair::capture(&cells, &intent).is_err());
    {
        let mut state = f.api.bot.session.state.lock().await;
        state.common_placement.as_mut().unwrap().stairs = Some(capture);
        // This diagonal chunk contains neither original support nor target.
        placement_chunk_changed(&mut state, [1, 1]);
        assert!(
            state
                .placement
                .as_ref()
                .unwrap()
                .requires_inspection
                .is_some()
        );
    }
    assert_eq!(sent.stage, PlacementStage::Pending);
    f.stop().await;
}
