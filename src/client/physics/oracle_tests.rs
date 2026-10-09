//! The engine against trajectories of the official movement code
//! (`scripts/movement_oracle`, docs/movement-oracle.md).
use super::{Body, Controls, Environment, Modifier, ModifierOperation, tick};
use crate::{MinecraftVersion, NativeBlockState};
use serde_json::Value;
use std::collections::BTreeMap;

const ORIGIN: [i32; 3] = [1024, 100, 1024];

pub(super) fn oracle() -> Value {
    serde_json::from_reader(flate2::read::GzDecoder::new(
        &include_bytes!("../../../data/client_api/movement_oracle.json.gz")[..],
    ))
    .unwrap()
}

pub(super) fn fluid_control_oracle() -> Value {
    serde_json::from_reader(flate2::read::GzDecoder::new(
        &include_bytes!("../../../data/client_api/fluid_control_oracle.json.gz")[..],
    ))
    .unwrap()
}

/// Oracle numbers are exact decimal strings.
fn exact(value: &Value) -> f64 {
    value.as_str().unwrap().parse().unwrap()
}

pub(super) fn state(text: &str) -> NativeBlockState {
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

pub(super) fn world(scenario: &Value, result: &Value) -> BTreeMap<[i32; 3], NativeBlockState> {
    let mut cells = BTreeMap::new();
    for fill in scenario["blocks"].as_array().unwrap() {
        let b: Vec<i64> = (0..6).map(|i| fill[i].as_i64().unwrap()).collect();
        for x in b[0]..=b[3] {
            for y in b[1]..=b[4] {
                for z in b[2]..=b[5] {
                    let text = fill[6].as_str().unwrap();
                    let mut resolved = state(text);
                    // The official server's complete properties for this state.
                    for (k, v) in result["states"][text].as_object().unwrap() {
                        resolved
                            .properties
                            .insert(k.clone(), v.as_str().unwrap().to_owned());
                    }
                    cells.insert([x as i32, y as i32, z as i32], resolved);
                }
            }
        }
    }
    cells
}

fn environment(version: MinecraftVersion, scenario: &Value, initial: &Value) -> Environment {
    let mut env = Environment::defaults(version);
    if let Some(level) = initial["depth_strider"].as_u64() {
        env.depth_strider = level as u8;
    }
    if let Some(blocks) = initial["legacy_soul_speed_blocks"].as_array() {
        env.legacy_soul_speed_blocks = Some(
            blocks
                .iter()
                .map(|b| b.as_str().unwrap().to_owned())
                .collect(),
        );
    }
    env.movement_speed_base = exact(&initial["movement_speed_base"]);
    env.movement_speed_modifiers = initial["movement_speed_modifiers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| Modifier {
            id: m["id"].as_str().unwrap().to_owned(),
            operation: match m["operation"].as_str().unwrap() {
                "ADDITION" | "ADD_VALUE" => ModifierOperation::Addition,
                "MULTIPLY_BASE" | "ADD_MULTIPLIED_BASE" => ModifierOperation::MultiplyBase,
                _ => ModifierOperation::MultiplyTotal,
            },
            amount: exact(&m["amount"]),
        })
        .collect();
    for (key, slot) in [
        ("jump_strength", &mut env.jump_strength),
        ("step_height", &mut env.step_height),
        ("gravity", &mut env.gravity),
        ("sneaking_speed", &mut env.sneaking_speed),
        ("movement_efficiency", &mut env.movement_efficiency),
        (
            "water_movement_efficiency",
            &mut env.water_movement_efficiency,
        ),
    ] {
        if !initial[key].is_null() {
            *slot = exact(&initial[key]);
        }
    }
    if let Some(effects) = scenario["effects"].as_object() {
        for (name, level) in effects {
            match name.as_str() {
                "minecraft:jump_boost" => env.jump_boost = Some(level.as_u64().unwrap() as u8),
                "minecraft:slow_falling" => env.slow_falling = true,
                "minecraft:blindness" => env.blindness = true,
                "minecraft:weaving" => env.weaving = true,
                "minecraft:dolphins_grace" => env.dolphins_grace = true,
                _ => {}
            }
        }
    }
    env
}

/// Run one scenario; Ok(None) when every tick matches, Ok(Some(why)) on the
/// first difference, Err when the engine refuses the scenario.
pub(super) fn compare(
    version: MinecraftVersion,
    scenario: &Value,
    result: &Value,
) -> Result<Option<String>, String> {
    let cells = world(scenario, result);
    let air = state("minecraft:air");
    let start: Vec<f64> = (0..3)
        .map(|i| scenario["start"][i].as_f64().unwrap())
        .collect();
    let mut env = environment(version, scenario, &result["initial"]);
    let mut body = Body::new(std::array::from_fn(|i| start[i] + f64::from(ORIGIN[i])));
    body.on_ground = scenario["on_ground"].as_bool().unwrap_or(true);
    // The oracle's player is a freshly created entity.
    body.first_tick = true;
    let mut block_at = |p: [i32; 3]| {
        let relative: [i32; 3] = std::array::from_fn(|i| p[i] - ORIGIN[i]);
        Ok(cells.get(&relative).cloned().unwrap_or_else(|| air.clone()))
    };
    assert!(!scenario["ticks"].as_array().unwrap().is_empty());
    assert_eq!(
        scenario["ticks"].as_array().unwrap().len(),
        result["frames"].as_array().unwrap().len()
    );
    for (index, (t, expected)) in scenario["ticks"]
        .as_array()
        .unwrap()
        .iter()
        .zip(result["frames"].as_array().unwrap())
        .enumerate()
    {
        let controls = Controls {
            forward: t["forward"].as_i64().unwrap_or(0) as i8,
            strafe: t["strafe"].as_i64().unwrap_or(0) as i8,
            jump: t["jump"].as_bool().unwrap_or(false),
            sneak: t["sneak"].as_bool().unwrap_or(false),
            sprint: t["sprint"].as_bool().unwrap_or(false),
            yaw: t["yaw"].as_f64().unwrap() as f32,
            pitch: t["pitch"].as_f64().unwrap_or(0.0) as f32,
        };
        env.using_item = t["using"].as_str().map(|name| {
            if version == MinecraftVersion::Java1_16_1 {
                super::ItemUse::DEFAULT
            } else {
                let item = crate::client::registry::Registry::for_version(version)
                    .item(name)
                    .unwrap();
                crate::client::item_use::prototype_use_effects(item.id.value()).unwrap()
            }
        });
        tick(version, &mut body, &env, controls, &mut block_at)
            .map_err(|e| format!("tick {index}: {e}"))?;
        if scenario["compare_fall_distance"].as_bool() == Some(true)
            && body.fall_distance != exact(&expected["fall_distance"])
        {
            return Ok(Some(format!(
                "tick {index}: fall distance {} vs {}",
                body.fall_distance, expected["fall_distance"]
            )));
        }
        for (axis, origin) in ORIGIN.iter().enumerate() {
            let position = body.position[axis] - f64::from(*origin);
            let want = exact(&expected["position"][axis]);
            let velocity = exact(&expected["velocity"][axis]);
            if position != want || body.velocity[axis] != velocity {
                return Ok(Some(format!(
                    "tick {index} axis {axis}: position {position} vs {want}, velocity {} vs {velocity}",
                    body.velocity[axis]
                )));
            }
        }
        for (name, ours) in [
            ("on_ground", body.on_ground),
            ("horizontal_collision", body.horizontal_collision),
            ("sprinting", body.sprinting),
            ("crouching", body.crouching),
            ("in_water", body.in_water),
            ("swimming", body.swimming),
        ] {
            if expected[name].as_bool().unwrap() != ours {
                return Ok(Some(format!("tick {index}: {name} {ours}")));
            }
        }
    }
    Ok(None)
}

#[test]
fn engine_reproduces_official_trajectories() {
    let data = oracle();
    let mut report = Vec::new();
    let mut failed = false;
    for (version, key) in [
        (MinecraftVersion::Java1_16_1, "1.16.1"),
        (MinecraftVersion::Java1_21_11, "1.21.11"),
    ] {
        for result in data["results"][key].as_array().unwrap() {
            let name = result["name"].as_str().unwrap();
            let scenario = data["scenarios"]
                .as_array()
                .unwrap()
                .iter()
                .find(|s| {
                    s["name"] == name
                        && s["versions"]
                            .as_array()
                            .is_none_or(|v| v.iter().any(|v| v == key))
                })
                .unwrap();
            match compare(version, scenario, result) {
                Ok(None) => report.push(format!("ok      {key} {name}")),
                Ok(Some(why)) => {
                    failed = true;
                    report.push(format!("DIFFER  {key} {name}: {why}"));
                }
                Err(why) => report.push(format!("refused {key} {name}: {why}")),
            }
        }
    }
    println!("{}", report.join("\n"));
    assert!(!failed, "{}", report.join("\n"));
}

#[test]
fn bubble_columns_reproduce_every_official_tick_without_refusals() {
    let data = fluid_control_oracle();
    let mut failures = Vec::new();
    let mut compared = 0;
    for (version, key) in [
        (MinecraftVersion::Java1_16_1, "1.16.1"),
        (MinecraftVersion::Java1_21_11, "1.21.11"),
    ] {
        let scenarios = data["scenarios"].as_array().unwrap();
        let results = data["results"][key].as_array().unwrap();
        assert_eq!(scenarios.len(), results.len());
        for (scenario, result) in scenarios.iter().zip(results) {
            assert_eq!(scenario["name"], result["name"]);
            if scenario["boat"].as_bool() == Some(true) {
                continue;
            }
            match compare(version, scenario, result) {
                Ok(None) => compared += 1,
                other => failures.push(format!("{key} {}: {other:?}", scenario["name"])),
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(compared, 42);
}

#[test]
fn climbing_reproduces_every_official_tick_without_refusals() {
    let data: Value = serde_json::from_reader(flate2::read::GzDecoder::new(
        &include_bytes!("../../../data/client_api/climbing_oracle.json.gz")[..],
    ))
    .unwrap();
    let mut failures = Vec::new();
    let mut compared = 0;
    for (version, key, count) in [
        (MinecraftVersion::Java1_16_1, "1.16.1", 42),
        (MinecraftVersion::Java1_21_11, "1.21.11", 46),
    ] {
        let results = data["results"][key].as_array().unwrap();
        assert_eq!(results.len(), count);
        let scenarios: Vec<_> = data["scenarios"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| {
                s["versions"]
                    .as_array()
                    .is_none_or(|versions| versions.iter().any(|v| v == key))
            })
            .collect();
        assert_eq!(scenarios.len(), count);
        for (scenario, result) in scenarios.into_iter().zip(results) {
            let name = result["name"].as_str().unwrap();
            assert_eq!(scenario["name"], result["name"]);
            match compare(version, scenario, result) {
                Ok(None) => compared += 1,
                other => failures.push(format!("{key} {name}: {other:?}")),
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(compared, 88);
}

#[test]
fn received_equipment_inputs_match_original_native_player_movement() {
    let oracle: Value = serde_json::from_reader(flate2::read::GzDecoder::new(
        &include_bytes!("../../../data/client_api/equipment_movement_oracle.json.gz")[..],
    ))
    .unwrap();
    let mut runs = 0;
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let name = if version == MinecraftVersion::Java1_16_1 {
            "1.16.1"
        } else {
            "1.21.11"
        };
        for result in oracle["results"][name].as_array().unwrap() {
            let scenario = oracle["scenarios"]
                .as_array()
                .unwrap()
                .iter()
                .find(|s| s["name"] == result["name"])
                .unwrap();
            assert_eq!(
                compare(version, scenario, result).unwrap(),
                None,
                "{}",
                result["name"]
            );
            runs += 1;
        }
    }
    assert_eq!(runs, 11);
}
