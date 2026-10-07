//! Trajectories from the official movement code (`scripts/movement_oracle`).
use super::model::{Model, predict};
use super::{SurvivalControl, SurvivalInput};
use crate::{MinecraftVersion, NativeBlockState};
use serde_json::Value;
use std::collections::BTreeMap;

const ORIGIN: [i32; 3] = [1024, 100, 1024];

fn oracle() -> Value {
    serde_json::from_str(include_str!(
        "../../../data/client_api/movement_oracle.json"
    ))
    .unwrap()
}

fn state(text: &str) -> NativeBlockState {
    let (name, properties) = match text.split_once('[') {
        None => (text, BTreeMap::new()),
        Some((name, rest)) => (
            name,
            rest.trim_end_matches(']')
                .split(',')
                .map(|pair| {
                    let (k, v) = pair.split_once('=').unwrap();
                    (k.to_owned(), v.to_owned())
                })
                .collect(),
        ),
    };
    NativeBlockState {
        name: name.to_owned(),
        properties,
    }
}

/// Oracle values are exact decimal strings; `str::parse` rounds correctly.
fn exact(value: &Value) -> f64 {
    value.as_str().unwrap().parse().unwrap()
}

fn world(scenario: &Value) -> BTreeMap<[i32; 3], NativeBlockState> {
    let mut cells = BTreeMap::new();
    for fill in scenario["blocks"].as_array().unwrap() {
        let b: Vec<i64> = (0..6).map(|i| fill[i].as_i64().unwrap()).collect();
        for x in b[0]..=b[3] {
            for y in b[1]..=b[4] {
                for z in b[2]..=b[5] {
                    cells.insert(
                        [x as i32, y as i32, z as i32],
                        state(fill[6].as_str().unwrap()),
                    );
                }
            }
        }
    }
    cells
}

/// Compare one scenario; returns the first differing tick, if any.
fn compare(version: MinecraftVersion, scenario: &Value, frames: &Value) -> Result<(), String> {
    let cells = world(scenario);
    let start: Vec<f64> = (0..3)
        .map(|i| scenario["start"][i].as_f64().unwrap())
        .collect();
    let mut model = Model::new(
        version,
        std::array::from_fn(|i| start[i] + f64::from(ORIGIN[i])),
    );
    let controls: Vec<SurvivalControl> = scenario["ticks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| SurvivalControl {
            yaw: t["yaw"].as_f64().unwrap() as f32,
            input: SurvivalInput {
                forward: t["forward"].as_i64().unwrap_or(0) as i8,
                strafe: t["strafe"].as_i64().unwrap_or(0) as i8,
                jump: t["jump"].as_bool().unwrap_or(false),
            },
        })
        .collect();
    let air = state("minecraft:air");
    let predicted = predict(
        |p| {
            let relative = std::array::from_fn(|i| p[i] - ORIGIN[i]);
            Ok(cells.get(&relative).cloned().unwrap_or_else(|| air.clone()))
        },
        &mut model,
        &controls,
    )
    .map_err(|e| format!("model refused: {e}"))?;
    for (tick, (frame, expected)) in predicted.iter().zip(frames.as_array().unwrap()).enumerate() {
        for axis in 0..3 {
            let position = frame.position[axis] - f64::from(ORIGIN[axis]);
            let want = exact(&expected["position"][axis]);
            let velocity = exact(&expected["velocity"][axis]);
            if position != want || frame.velocity[axis] != velocity {
                return Err(format!(
                    "tick {tick} axis {axis}: position {position} vs {want}, velocity {} vs {velocity}",
                    frame.velocity[axis]
                ));
            }
        }
        if frame.on_ground != expected["on_ground"].as_bool().unwrap() {
            return Err(format!("tick {tick}: on_ground"));
        }
    }
    Ok(())
}

#[test]
fn existing_dry_model_reproduces_official_baseline_trajectories() {
    let data = oracle();
    let baseline = [
        "walk",
        "walk_diagonal_yaw",
        "back_strafe",
        "jump_in_place",
        "walk_hold_jump",
        "walk_into_wall",
        "step_onto_slab",
        "walk_off_edge",
    ];
    let mut failures = Vec::new();
    for (version, key) in [
        (MinecraftVersion::Java1_16_1, "1.16.1"),
        (MinecraftVersion::Java1_21_11, "1.21.11"),
    ] {
        let results = data["results"][key].as_array().unwrap();
        for name in baseline {
            let scenario = data["scenarios"]
                .as_array()
                .unwrap()
                .iter()
                .find(|s| s["name"] == name)
                .unwrap();
            let result = results.iter().find(|r| r["name"] == name).unwrap();
            if let Err(e) = compare(version, scenario, &result["frames"]) {
                failures.push(format!("{key} {name}: {e}"));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
