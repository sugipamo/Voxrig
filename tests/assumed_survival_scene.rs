//! Public read-only boundary; no connection or private received state.
use std::collections::BTreeMap;
use voxrig::checked_survival::{
    AssumedSurvivalScene, AssumedSurvivalStart, HypotheticalSceneSource, SurvivalControl,
    SurvivalInput, SurvivalMotionContract, TerminalClearance,
};
use voxrig::{BlockFace, NativeBlockState, Region};
fn block(name: &str) -> NativeBlockState {
    NativeBlockState {
        name: format!("minecraft:{name}"),
        properties: Default::default(),
    }
}
fn input() -> (
    Region,
    BTreeMap<[i32; 3], NativeBlockState>,
    AssumedSurvivalStart,
) {
    let region = Region {
        min: [-4, -2, -4],
        max: [4, 4, 4],
    };
    let mut blocks = BTreeMap::new();
    for x in -4..=4 {
        for y in -2..=4 {
            for z in -4..=4 {
                blocks.insert([x, y, z], block(if y < 0 { "stone" } else { "air" }));
            }
        }
    }
    (
        region,
        blocks,
        AssumedSurvivalStart {
            dimension: "minecraft:overworld".into(),
            position: [0.5, 0.0, 0.5],
            velocity: [0.0; 3],
            planning_reserve: [0.0; 3],
        },
    )
}
#[test]
fn complete_geometry_and_bounded_grounded_start_are_required() {
    let (region, blocks, start) = input();
    let mut missing = blocks.clone();
    missing.remove(&[3, 3, 3]);
    assert!(AssumedSurvivalScene::new(region, missing, start.clone()).is_err());
    let mut replaced = blocks.clone();
    replaced.remove(&[3, 3, 3]);
    replaced.insert([5, 3, 3], block("air"));
    assert!(AssumedSurvivalScene::new(region, replaced, start.clone()).is_err());
    let mut dynamic = blocks.clone();
    dynamic.insert([3, 3, 3], block("water"));
    assert!(AssumedSurvivalScene::new(region, dynamic, start.clone()).is_err());
    for invalid in [
        AssumedSurvivalStart {
            position: [f64::NAN, 0.0, 0.5],
            ..start.clone()
        },
        AssumedSurvivalStart {
            position: [0.5, 1.0, 0.5],
            ..start.clone()
        },
        AssumedSurvivalStart {
            planning_reserve: [-0.1, 0.0, 0.0],
            ..start.clone()
        },
        AssumedSurvivalStart {
            velocity: [f64::INFINITY, 0.0, 0.0],
            ..start.clone()
        },
    ] {
        assert!(AssumedSurvivalScene::new(region, blocks.clone(), invalid).is_err());
    }
    assert!(
        AssumedSurvivalScene::new(
            Region {
                max: [i32::MAX; 3],
                ..region
            },
            blocks.clone(),
            start.clone()
        )
        .is_err()
    );
    assert!(AssumedSurvivalScene::new(region, blocks, start).is_ok());
}
#[test]
fn native_checks_and_fork_isolation_keep_assumed_provenance() {
    let (region, blocks, start) = input();
    let scene = AssumedSurvivalScene::new(region, blocks, start.clone()).unwrap();
    let scenario = scene.scenario_with_motion_contract(SurvivalMotionContract::Predicted);
    assert!(scenario.block([5, 0, 0]).is_err());
    let placement = scenario
        .preview_cube_placement(
            [2, -1, 0],
            BlockFace::Up,
            [-90.0, (1.62_f64 / 2.0).atan().to_degrees() as f32],
            "minecraft:stone",
        )
        .unwrap();
    let edited = scenario.after_edits(&[placement.edit]).unwrap();
    assert_eq!(scenario.block([2, 0, 0]).unwrap(), block("air"));
    assert_eq!(edited.block([2, 0, 0]).unwrap(), block("stone"));
    let idle = [SurvivalControl {
        yaw: 0.0,
        input: Default::default(),
    }; 3];
    let preview = scenario.preview_path(&idle).unwrap();
    assert!(!preview.shares_origin(&edited.preview_path(&idle).unwrap()));
    assert!(
        matches!(&preview.source, HypotheticalSceneSource::Assumed { start: s } if *s == start)
    );
    assert!(preview.source.captured().is_none());
    let (reset, _) = edited.after_expected_reconnect().unwrap();
    assert!(
        reset
            .preview_path(&idle)
            .unwrap()
            .source
            .captured()
            .is_none()
    );
    assert_eq!(scene.source(), &start);
}

#[test]
fn movement_transition_preserves_native_continuation_and_refuses_unsafe_stops() {
    let (region, blocks, start) = input();
    let scene = AssumedSurvivalScene::new(region, blocks, start).unwrap();
    let scenario = scene.scenario_with_motion_contract(SurvivalMotionContract::Predicted);
    let controls: Vec<_> = (0..28)
        .map(|tick| SurvivalControl {
            yaw: -90.0,
            input: SurvivalInput {
                forward: i8::from(tick < 4),
                ..Default::default()
            },
        })
        .collect();
    let expected = scenario.preview_path(&controls).unwrap();
    let expected_next = scenario.after_path(&controls).unwrap();
    let transition = scenario.preview_path_transition(&controls).unwrap();
    assert!(scenario.matches_preview(transition.preview()));
    assert_eq!(transition.preview().frames, expected.frames);
    let (preview, next) = transition.into_parts().unwrap();
    assert_eq!(preview.initial_frame, expected.initial_frame);
    assert_eq!(preview.terminal_clearance, expected.terminal_clearance);
    assert_eq!(next.position(), expected_next.position());
    assert!(!next.matches_preview(&preview));
    assert_eq!(scenario.position(), scene.source().position);
    let jump = [SurvivalControl {
        yaw: 0.0,
        input: SurvivalInput {
            jump: true,
            ..Default::default()
        },
    }; 3];
    assert_eq!(
        next.preview_path(&jump).unwrap().frames,
        expected_next.preview_path(&jump).unwrap().frames
    );
    let mut combined = controls.clone();
    combined.extend(jump);
    let uninterrupted = scenario.preview_path(&combined).unwrap();
    let continued = next.preview_path(&jump).unwrap();
    for (actual, expected) in continued
        .frames
        .iter()
        .zip(&uninterrupted.frames[controls.len()..])
    {
        let mut expected = expected.clone();
        expected.tick = actual.tick;
        assert_eq!(actual, &expected);
    }
    let unsafe_stop = scenario.preview_path_transition(&controls[..1]).unwrap();
    assert!(matches!(
        unsafe_stop.preview().terminal_clearance,
        TerminalClearance::RequiresReplan { .. }
    ));
    assert!(unsafe_stop.into_parts().is_err());
    assert_eq!(
        scenario.preview_path(&controls).unwrap().frames,
        expected.frames
    );
}

#[test]
fn removal_transition_is_atomic_and_keeps_support_admission() {
    let (region, mut blocks, start) = input();
    blocks.insert([2, 0, 0], block("stone"));
    let scene = AssumedSurvivalScene::new(region, blocks, start).unwrap();
    let scenario = scene.scenario_with_motion_contract(SurvivalMotionContract::Predicted);
    let rotation = [-90.0, (1.12_f64 / 1.5).atan().to_degrees() as f32];
    let expected = scenario
        .preview_cube_removal([2, 0, 0], BlockFace::West, rotation)
        .unwrap();
    let (edit, next) = scenario
        .preview_cube_removal_with_successor([2, 0, 0], BlockFace::West, rotation)
        .unwrap();
    assert_eq!(edit.position, expected.position);
    assert_eq!(edit.before, expected.before);
    assert_eq!(edit.after, expected.after);
    assert_eq!(next.block([2, 0, 0]).unwrap(), block("air"));
    assert_eq!(scenario.block([2, 0, 0]).unwrap(), block("stone"));
    assert!(
        scenario
            .preview_cube_removal_with_successor([0, -1, 0], BlockFace::Up, [0.0, 90.0])
            .is_err()
    );
    assert_eq!(scenario.block([0, -1, 0]).unwrap(), block("stone"));
    assert!(
        scenario
            .preview_cube_removal_with_successor([2, 0, 0], BlockFace::East, rotation)
            .is_err()
    );
}
