use super::*;
use crate::client::{
    adapter::PlacementOps,
    survival::{PlacementStage, SlabFluidState},
};

fn water(level: u8) -> crate::NativeBlockState {
    crate::NativeBlockState {
        name: "minecraft:water".into(),
        properties: [("level".into(), level.to_string())].into(),
    }
}
fn wet(kind: &str) -> crate::NativeBlockState {
    let mut block = oak(kind);
    block.properties.insert("waterlogged".into(), "true".into());
    block
}
async fn start(
    f: &mut Fixture,
    before: crate::NativeBlockState,
    merge: bool,
    count: i32,
) -> crate::client::survival::FluidSlabPlacementRecord {
    slab_stack(f, count).await;
    let target = if merge { SUPPORT } else { TARGET };
    replacement_block(f, target, &before).await;
    let sent = f
        .api
        .place_slab_with_fluid(
            SUPPORT,
            if merge {
                crate::BlockFace::Up
            } else {
                crate::BlockFace::West
            },
            target,
            before,
        )
        .await
        .unwrap();
    let (id, payload) = f.expected_packet("fluid slab single BLOCK_PLACE").await;
    assert_eq!(id, ids::play_serverbound::BLOCK_PLACE);
    let intent = f
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
    assert_eq!(payload, packet(&intent));
    assert!(sent.placement.send.dispatched);
    assert_eq!(sent.placement.merges_clicked, merge);
    assert_eq!(
        sent.source.receive_sequence,
        sent.placement.send.after_sequence
    );
    assert_eq!(
        sent.source.connection_id,
        sent.placement.id.session().connection_id
    );
    sent
}
#[test]
fn fluid_slab_original_context_all_water_levels_wet_halves_and_exclusions() {
    use std::io::Read;
    let mut text = String::new();
    flate2::read::GzDecoder::new(
        &include_bytes!("../../../../../../../data/client_api/fluid_slab_placement.json.gz")[..],
    )
    .read_to_string(&mut text)
    .unwrap();
    let rows: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
    assert_eq!(rows.len(), 5328);
    let mut levels = std::collections::BTreeSet::new();
    let mut checked = 0;
    for row in rows {
        let input: crate::NativeBlockState = serde_json::from_value(row["input"].clone()).unwrap();
        if let Some(fluid) = wet_slab::fluid(&input) {
            let oracle = &row["input_fluid"];
            match fluid {
                SlabFluidState::Empty => assert_eq!(oracle["native_type"], "minecraft:empty"),
                SlabFluidState::Water {
                    source,
                    falling,
                    amount,
                } => {
                    assert_eq!(
                        oracle["native_type"],
                        if source {
                            "minecraft:water"
                        } else {
                            "minecraft:flowing_water"
                        }
                    );
                    assert_eq!(oracle["source"], source);
                    assert_eq!(oracle["falling"], falling);
                    assert_eq!(oracle["amount"], amount);
                    if input.name == "minecraft:water" {
                        levels.insert(input.properties["level"].clone());
                    }
                }
            }
        } else {
            assert!(!wet_slab::destination(&input));
        }
        let clicked: crate::NativeBlockState =
            serde_json::from_value(row["clicked"].clone()).unwrap();
        if !(clicked == native("stone") || slab::kind_with_fluid(&clicked).is_some()) {
            continue;
        }
        let face = row["face"].as_u64().unwrap() as u8;
        let y = row["hit_y"].as_f64().unwrap() as f32;
        let merges = slab::replace_clicked(&clicked, face, y);
        assert_eq!(row["replace_clicked"], merges, "{row}");
        let d = offset(face);
        let destination = if merges {
            SUPPORT
        } else {
            [2 + d[0], 2 + d[1], d[2]]
        };
        assert_eq!(row["destination"], serde_json::json!(destination));
        let before = crate::versions::java_1_21_11::native_state(
            row["destination_state"].as_i64().unwrap() as i32,
        )
        .unwrap();
        if !wet_slab::destination(&before) {
            continue;
        }
        assert_eq!(row["can_place"], true, "{row}");
        let expected = slab::expected_with_fluid(&before, face, y);
        assert_eq!(
            row["placed_state"],
            serde_json::to_value(&expected).unwrap(),
            "{row}"
        );
        let expected_fluid = wet_slab::fluid(&expected).unwrap();
        assert_eq!(
            row["placed_fluid"]["native_type"],
            if expected_fluid == SlabFluidState::Empty {
                "minecraft:empty"
            } else {
                "minecraft:water"
            }
        );
        checked += 1;
    }
    assert_eq!(levels.len(), 16);
    assert!(checked > 1000, "{checked}");
}
#[tokio::test]
async fn fluid_slab_source_flowing_falling_dry_and_wet_merge_all_receipt_orders_freeze() {
    for (before, merge) in [
        (water(0), false),
        (water(1), false),
        (water(7), false),
        (water(8), false),
        (water(15), false),
        (native("air"), false),
        (wet("bottom"), true),
        (wet("bottom"), false),
        (oak("bottom"), false),
    ] {
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            let mut f = Fixture::new().await;
            let sent = start(&mut f, before.clone(), merge, 1).await;
            let r = &sent.placement;
            let expected = if merge || before.name == "minecraft:oak_slab" {
                oak("double")
            } else if before == water(0) {
                wet("top")
            } else {
                oak("top")
            };
            assert_eq!(r.expected, expected);
            assert_eq!(sent.before_fluid, wet_slab::fluid(&before).unwrap());
            assert!(f.api.slab_placement_record().await.unwrap().is_none());
            assert!(f.api.placement_record().await.unwrap().is_none());
            assert!(
                f.api
                    .place_slab_with_fluid(SUPPORT, crate::BlockFace::West, TARGET, native("air"))
                    .await
                    .is_err()
            );
            for (index, receipt) in order.into_iter().enumerate() {
                match receipt {
                    0 => replacement_block(&mut f, r.target, &r.expected).await,
                    1 => slab_stack(&mut f, 0).await,
                    2 => f.ack(r.send.interaction_sequence.unwrap()).await,
                    _ => unreachable!(),
                }
                let state = f.api.bot.session.state.lock().await;
                assert_eq!(
                    state.placement.as_ref().unwrap().observation.is_some(),
                    index == 2,
                    "{before:?} {order:?} {index}"
                );
            }
            let frozen = {
                let state = f.api.bot.session.state.lock().await;
                serde_json::to_value(wet_slab::record_in(&state).unwrap().unwrap()).unwrap()
            };
            replacement_block(&mut f, r.target, &water(2)).await;
            f.stack(12).await;
            f.ack(999).await;
            let complete = f.api.fluid_slab_placement_record().await.unwrap().unwrap();
            assert_eq!(complete.placement.stage, PlacementStage::ObservedPlaced);
            assert_eq!(
                complete.placement.material_receipt.as_ref().unwrap().value,
                crate::client::SlotKnowledge::Empty
            );
            assert_eq!(serde_json::to_value(complete).unwrap(), frozen);
            assert!(
                timeout(Duration::from_millis(5), read_packet(&mut f.peer, None))
                    .await
                    .is_err()
            );
            f.stop().await;
        }
    }
}
#[tokio::test]
async fn fluid_slab_rejects_unaudited_fluid_family_wrong_predecessor_and_keeps_dry_api() {
    for case in [
        "lava",
        "bubble_column",
        "spruce",
        "double",
        "stair",
        "wrong_before",
        "wrong_target",
        "material",
        "dry_source",
        "dry_wet",
    ] {
        let mut f = Fixture::new().await;
        slab_stack(&mut f, 1).await;
        let before = match case {
            "lava" => crate::NativeBlockState {
                name: "minecraft:lava".into(),
                properties: [("level".into(), "0".into())].into(),
            },
            "bubble_column" => crate::NativeBlockState {
                name: "minecraft:bubble_column".into(),
                properties: [("drag".into(), "false".into())].into(),
            },
            "spruce" => {
                let mut s = wet("bottom");
                s.name = "minecraft:spruce_slab".into();
                s
            }
            "double" => wet("double"),
            "stair" => crate::NativeBlockState {
                name: "minecraft:oak_stairs".into(),
                properties: [
                    ("facing".into(), "east".into()),
                    ("half".into(), "bottom".into()),
                    ("shape".into(), "straight".into()),
                    ("waterlogged".into(), "false".into()),
                ]
                .into(),
            },
            "dry_wet" => wet("top"),
            _ => water(0),
        };
        replacement_block(&mut f, TARGET, &before).await;
        if case == "material" {
            f.stack(1).await;
        }
        let requested = if case == "wrong_before" {
            water(1)
        } else {
            before.clone()
        };
        let target = if case == "wrong_target" {
            SUPPORT
        } else {
            TARGET
        };
        let result = if case.starts_with("dry_") {
            f.api
                .place_slab(SUPPORT, crate::BlockFace::West, target, requested)
                .await
                .map(|_| ())
        } else {
            f.api
                .place_slab_with_fluid(SUPPORT, crate::BlockFace::West, target, requested)
                .await
                .map(|_| ())
        };
        assert!(result.is_err(), "{case}");
        assert!(f.api.fluid_slab_placement_record().await.unwrap().is_none());
        assert!(
            timeout(Duration::from_millis(5), read_packet(&mut f.peer, None))
                .await
                .is_err()
        );
        f.stop().await;
    }
    for mut block in [water(0), wet("top")] {
        block.properties.insert("unknown".into(), "true".into());
        assert!(wet_slab::fluid(&block).is_none());
    }
    assert!(
        wet_slab::fluid(&crate::NativeBlockState {
            name: "minecraft:unknown_fluid".into(),
            properties: Default::default()
        })
        .is_none()
    );
}
#[tokio::test]
async fn fluid_slab_missing_stale_conflicting_unload_and_neighbor_receipts() {
    for case in [
        "block",
        "material",
        "ack",
        "stale_ack",
        "wrong_wet",
        "flow_before_target",
        "target_restore",
        "slot_restore",
        "unload",
        "selection",
        "support",
        "neighbor",
    ] {
        let mut f = Fixture::new().await;
        if case == "stale_ack" {
            f.ack(100).await;
        }
        let sent = start(&mut f, water(0), false, 2).await;
        let r = sent.placement;
        match case {
            "wrong_wet" => replacement_block(&mut f, TARGET, &oak("top")).await,
            "flow_before_target" => replacement_block(&mut f, TARGET, &water(1)).await,
            "support" => f.block(SUPPORT, "air").await,
            "unload" => {
                f.receive(ids::play_clientbound::UNLOAD_CHUNK, &[0; 8])
                    .await
            }
            "selection" => f.receive(ids::play_clientbound::HELD_ITEM_SLOT, &[1]).await,
            "neighbor" => replacement_block(&mut f, [1, 2, 1], &water(3)).await,
            _ => (),
        }
        if case != "block" && case != "unload" {
            replacement_block(&mut f, TARGET, &r.expected).await;
        }
        if case == "target_restore" {
            replacement_block(&mut f, TARGET, &r.before).await;
            replacement_block(&mut f, TARGET, &r.expected).await;
        }
        if case != "material" {
            slab_stack(&mut f, 1).await;
        }
        if case == "slot_restore" {
            slab_stack(&mut f, 2).await;
            slab_stack(&mut f, 1).await;
        }
        if !matches!(case, "ack" | "stale_ack") {
            f.ack(r.send.interaction_sequence.unwrap()).await;
        }
        let result = f
            .api
            .fluid_slab_placement_record()
            .await
            .unwrap()
            .unwrap()
            .placement;
        assert_eq!(
            result.stage,
            if case == "neighbor" {
                PlacementStage::ObservedPlaced
            } else if matches!(case, "block" | "material" | "ack" | "stale_ack") {
                PlacementStage::Pending
            } else {
                PlacementStage::RequiresInspection
            },
            "{case} {result:?}"
        );
        assert_eq!(sent.source.receive_sequence, r.send.after_sequence);
        f.stop().await;
    }
}
#[tokio::test]
async fn fluid_slab_cancelled_before_writer_and_partial_write_keep_intent_without_replay() {
    for partial in [false, true] {
        let mut f = Fixture::new().await;
        slab_stack(&mut f, 1).await;
        replacement_block(&mut f, TARGET, &water(0)).await;
        let session = f.api.bot.session.clone();
        let control = crate::client::ledger::testing::Control::default();
        if partial {
            session
                .writer
                .lock()
                .await
                .stream
                .test_replace(control.writer());
            control.release(1, false);
        }
        let writer = if partial {
            None
        } else {
            Some(session.writer.lock().await)
        };
        if partial {
            let client = crate::Client::from_java_1_21_11(f.api.bot.clone());
            let survival = client.survival();
            let waiter = tokio::spawn(async move {
                survival
                    .place_slab_with_fluid(SUPPORT, crate::BlockFace::West, TARGET, water(0))
                    .await
            });
            control.until(|w| w.bytes().len() == 1).await;
            waiter.abort();
            assert!(waiter.await.unwrap_err().is_cancelled());
        } else {
            assert!(
                timeout(
                    Duration::from_millis(30),
                    f.api
                        .place_slab_with_fluid(SUPPORT, crate::BlockFace::West, TARGET, water(0))
                )
                .await
                .is_err()
            );
        }
        drop(writer);
        let sent = f.api.fluid_slab_placement_record().await.unwrap().unwrap();
        assert!(!sent.placement.send.dispatched);
        if !partial {
            replacement_block(&mut f, TARGET, &sent.placement.expected).await;
            slab_stack(&mut f, 0).await;
            f.ack(sent.placement.send.interaction_sequence.unwrap())
                .await;
        }
        assert_ne!(
            f.api
                .fluid_slab_placement_record()
                .await
                .unwrap()
                .unwrap()
                .placement
                .stage,
            PlacementStage::ObservedPlaced
        );
        assert!(
            f.api
                .place_slab_with_fluid(SUPPORT, crate::BlockFace::West, TARGET, water(0))
                .await
                .is_err()
        );
        if partial {
            assert_eq!(sent.placement.stage, PlacementStage::RequiresInspection);
            assert_eq!(control.bytes().len(), 1);
        } else {
            assert!(
                timeout(Duration::from_millis(5), read_packet(&mut f.peer, None))
                    .await
                    .is_err()
            );
        }
        f.stop().await;
    }
}
#[tokio::test]
async fn fluid_slab_history_is_separate_from_dry_slab_and_survives_next_cube_and_close() {
    let mut f = Fixture::new().await;
    let dry = slab_start(&mut f, false, 1).await;
    replacement_block(&mut f, TARGET, &dry.expected).await;
    slab_stack(&mut f, 0).await;
    f.ack(dry.send.interaction_sequence.unwrap()).await;
    let dry = serde_json::to_value(f.api.slab_placement_record().await.unwrap()).unwrap();
    let wet = start(&mut f, water(0), false, 1).await.placement;
    replacement_block(&mut f, TARGET, &wet.expected).await;
    slab_stack(&mut f, 0).await;
    f.ack(wet.send.interaction_sequence.unwrap()).await;
    let wet = serde_json::to_value(f.api.fluid_slab_placement_record().await.unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(f.api.slab_placement_record().await.unwrap()).unwrap(),
        dry
    );
    f.block(TARGET, "air").await;
    f.stack(3).await;
    f.start().await;
    assert_eq!(
        serde_json::to_value(f.api.fluid_slab_placement_record().await.unwrap()).unwrap(),
        wet
    );
    let api = f.api.clone();
    f.stop().await;
    assert_eq!(
        serde_json::to_value(api.fluid_slab_placement_record().await.unwrap()).unwrap(),
        wet
    );
    assert_eq!(
        serde_json::to_value(api.slab_placement_record().await.unwrap()).unwrap(),
        dry
    );
}

#[test]
fn fluid_slab_halo_preserves_qualified_dry_terrain_and_refuses_body_fluid() {
    let own = crate::client::survival::model::body([0.5, 1., 0.5]);
    let grass = crate::NativeBlockState {
        name: "minecraft:grass_block".into(),
        properties: [("snowy".into(), "false".into())].into(),
    };
    assert_eq!(wet_slab::fluid(&grass), Some(SlabFluidState::Empty));
    assert!(fluid_slab_collision(&grass, [0, 0, 0], own).is_ok());
    let mut foreign = oak("bottom");
    foreign.name = "minecraft:spruce_slab".into();
    assert!(fluid_slab_collision(&foreign, [1, 0, 0], own).is_ok());
    assert!(!wet_slab::destination(&foreign));
    assert!(fluid_slab_collision(&water(0), [1, 1, 0], own).is_ok());
    assert!(fluid_slab_collision(&water(0), [0, 1, 0], own).is_err());
    assert!(fluid_slab_collision(&wet("bottom"), [0, 1, 0], own).is_err());
}
