use super::*;

fn world() -> World {
    let mut w = World::default();
    w.select_dimension(
        "minecraft:overworld".into(),
        super::super::world::Dimension::new(-64, 384).unwrap(),
    );
    for x in -1..=1 {
        for z in -1..=1 {
            w.seed_replay_cell([x * 16, 80, z * 16], 0);
        }
    }
    w
}
fn put(w: &mut World, p: Pos, s: NativeBlockState) {
    w.seed_replay_cell(p, super::super::state_id(&s).unwrap());
}
fn body(sticky: bool, dir: Direction) -> NativeBlockState {
    state(
        if sticky { "sticky_piston" } else { "piston" },
        &[("facing", dir.name()), ("extended", "false")],
    )
}
fn name(r: &Reconstruction, w: &World, p: Pos) -> String {
    r.cell(w, p).state.unwrap().name
}

#[test]
fn normal_and_sticky_carriers_complete_in_all_six_directions() {
    let p = [0, 80, 0];
    for dir in Direction::ALL {
        for sticky in [false, true] {
            let mut w = world();
            put(&mut w, p, body(sticky, dir));
            put(&mut w, dir.offset(p, 1), state("stone", &[]));
            let mut r = Reconstruction::default();
            let piston = if sticky {
                "minecraft:sticky_piston"
            } else {
                "minecraft:piston"
            };
            r.action(&w, p, Action::Extend, dir, piston, 5);
            assert!(r.issue.is_none(), "{:?}", r.issue);
            assert_eq!(r.moving.len(), 2);
            assert_eq!(r.moving[&dir.offset(p, 2)].role, CarrierRole::Payload);
            r.advance(&w, 1);
            assert_eq!(r.moving[&dir.offset(p, 2)].progress, MotionProgress::Half);
            assert_eq!(
                r.moving[&dir.offset(p, 2)].last_progress,
                MotionProgress::Start
            );
            r.advance(&w, 2);
            assert_eq!(r.moving[&dir.offset(p, 2)].progress, MotionProgress::Full);
            r.advance(&w, 7);
            assert_eq!(r.moving.len(), 2);
            r.advance(&w, 8);
            assert!(r.moving.is_empty());
            assert_eq!(name(&r, &w, dir.offset(p, 2)), "minecraft:stone");
            r.action(&w, p, Action::Retract, dir, piston, 6);
            assert!(r.issue.is_none(), "{:?}", r.issue);
            assert_eq!(r.moving[&p].role, CarrierRole::Body);
            assert_eq!(r.moving.len(), if sticky { 2 } else { 1 });
            r.advance(&w, 16);
            assert_eq!(name(&r, &w, p), piston);
            assert_eq!(
                name(&r, &w, dir.offset(p, 1)),
                if sticky {
                    "minecraft:stone"
                } else {
                    "minecraft:air"
                }
            );
            assert_eq!(
                name(&r, &w, dir.offset(p, 2)),
                if sticky {
                    "minecraft:air"
                } else {
                    "minecraft:stone"
                }
            );
            // Received cache is never overwritten by predicted motion.
            assert_eq!(
                super::super::native_state(w.block(dir.offset(p, 1)).unwrap())
                    .unwrap()
                    .name,
                "minecraft:stone"
            );
        }
    }
}

#[test]
fn forced_completion_and_drop_retraction_leave_payload_behind() {
    let p = [0, 80, 0];
    let dir = Direction::East;
    for ticks in [0, 1, 2, 7, 8] {
        for action in [Action::Retract, Action::Drop] {
            let mut w = world();
            put(&mut w, p, body(true, dir));
            put(&mut w, [1, 80, 0], state("stone", &[]));
            let mut r = Reconstruction::default();
            r.action(&w, p, Action::Extend, dir, "minecraft:sticky_piston", 1);
            r.advance(&w, ticks);
            r.action(&w, p, action, dir, "minecraft:sticky_piston", 2);
            assert!(r.issue.is_none(), "{:?}", r.issue);
            r.advance(&w, ticks + 8);
            let pulls = ticks == 8 && action == Action::Retract;
            assert_eq!(
                name(&r, &w, [1, 80, 0]),
                if pulls {
                    "minecraft:stone"
                } else {
                    "minecraft:air"
                }
            );
            assert_eq!(
                name(&r, &w, [2, 80, 0]),
                if pulls {
                    "minecraft:air"
                } else {
                    "minecraft:stone"
                }
            );
        }
    }
}

#[test]
fn native_update_cancels_carrier_so_late_completion_cannot_restore_it() {
    let p = [0, 80, 0];
    let mut w = world();
    put(&mut w, p, body(false, Direction::East));
    put(&mut w, [1, 80, 0], state("stone", &[]));
    let mut r = Reconstruction::default();
    r.action(
        &w,
        p,
        Action::Extend,
        Direction::East,
        "minecraft:piston",
        1,
    );
    put(&mut w, [2, 80, 0], state("air", &[]));
    r.received(&[([2, 80, 0], 0)]);
    r.advance(&w, 100);
    assert_eq!(name(&r, &w, [2, 80, 0]), "minecraft:air");
    assert!(!r.moving.contains_key(&[2, 80, 0]));
}

#[test]
fn unsupported_and_missing_dependencies_never_publish_partial_motion() {
    let p = [0, 80, 0];
    let mut w = world();
    put(&mut w, p, body(false, Direction::East));
    put(&mut w, [1, 80, 0], state("slime_block", &[]));
    let mut r = Reconstruction::default();
    r.action(
        &w,
        p,
        Action::Extend,
        Direction::East,
        "minecraft:piston",
        1,
    );
    assert!(matches!(
        r.issue,
        Some(ReconstructionIssue::UnsupportedBlock { .. })
    ));
    assert!(r.overlay.is_empty());
    assert!(r.cell(&w, p).state.is_none());
    let mut r = Reconstruction::default();
    r.action(
        &w,
        [1000, 80, 1000],
        Action::Extend,
        Direction::East,
        "minecraft:piston",
        1,
    );
    assert!(matches!(
        r.issue,
        Some(ReconstructionIssue::MissingBlock { .. })
    ));
}

#[test]
fn chunk_invalidation_and_unknown_carriers_are_explicit() {
    let p = [0, 80, 0];
    let mut w = world();
    put(&mut w, p, body(false, Direction::East));
    let mut r = Reconstruction::default();
    r.action(
        &w,
        p,
        Action::Extend,
        Direction::East,
        "minecraft:piston",
        1,
    );
    r.chunk_replaced([0, 0]);
    assert!(matches!(
        r.issue,
        Some(ReconstructionIssue::ChunkInvalidated { .. })
    ));
    let mut r = Reconstruction::default();
    let moving = super::super::state_id(&state(
        "moving_piston",
        &[("facing", "east"), ("type", "normal")],
    ))
    .unwrap();
    r.received(&[(p, moving)]);
    assert!(matches!(
        r.issue,
        Some(ReconstructionIssue::MissingCarrier { .. })
    ));
}

#[test]
fn known_carrier_can_finish_after_receiving_a_moving_state_packet() {
    let p = [0, 80, 0];
    let dest = [2, 80, 0];
    let mut w = world();
    put(&mut w, p, body(false, Direction::East));
    put(&mut w, [1, 80, 0], state("stone", &[]));
    let mut r = Reconstruction::default();
    r.action(
        &w,
        p,
        Action::Extend,
        Direction::East,
        "minecraft:piston",
        1,
    );
    let moving = state("moving_piston", &[("facing", "east"), ("type", "normal")]);
    let id = super::super::state_id(&moving).unwrap();
    put(&mut w, dest, moving);
    r.received(&[(dest, id)]);
    r.advance(&w, 8);
    assert_eq!(name(&r, &w, dest), "minecraft:stone");
    assert!(r.issue.is_none());
}

#[test]
fn shape_rules_preserve_all_native_stair_properties() {
    let mut w = world();
    let target = [2, 80, -1];
    put(&mut w, [0, 80, 0], body(false, Direction::East));
    put(&mut w, [1, 80, 0], state("stone", &[]));
    put(
        &mut w,
        [2, 80, 0],
        state(
            "cobblestone_stairs",
            &[
                ("facing", "west"),
                ("half", "top"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
    );
    put(
        &mut w,
        target,
        state(
            "quartz_stairs",
            &[
                ("facing", "north"),
                ("half", "top"),
                ("shape", "inner_left"),
                ("waterlogged", "false"),
            ],
        ),
    );
    let mut r = Reconstruction::default();
    r.action(
        &w,
        [0, 80, 0],
        Action::Extend,
        Direction::East,
        "minecraft:piston",
        42,
    );
    assert!(r.issue.is_none(), "{:?}", r.issue);
    let cell = r.cell(&w, target);
    assert_eq!(cell.state.unwrap().properties["shape"], "straight");
    assert!(matches!(
        cell.origin,
        StateOrigin::ClientUpdate {
            action_sequence: 42
        }
    ));
}

#[test]
fn complete_state_codec_roundtrips_every_bundled_state_and_rejects_missing_properties() {
    let definitions: serde_json::Value =
        serde_json::from_str(include_str!("../../../../data/java_1_21_11/blocks.json")).unwrap();
    let max = definitions.as_array().unwrap().last().unwrap()["maxStateId"]
        .as_i64()
        .unwrap() as i32;
    for id in 0..=max {
        let s = super::super::native_state(id).unwrap();
        assert_eq!(super::super::state_id(&s).unwrap(), id);
    }
    assert!(super::super::state_id(&state("piston", &[("facing", "east")])).is_err());
}
