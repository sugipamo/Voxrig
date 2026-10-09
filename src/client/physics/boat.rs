//! Mounted boat motion using the pinned Boat/AbstractBoat movement rules.
//! Surface/submerged/flowing water, air and audited block collision. Bubble
//! columns and entity collisions require separate received handling.
use super::*;
use crate::client::vehicle::VehicleInput;

const WIDTH: f64 = 1.375;
const HEIGHT: f64 = 0.5625;

/// Predicted boat state, distinct from any received vehicle position.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct BoatFrame {
    /// Predicted feet coordinates.
    pub position: [f64; 3],
    /// Velocity carried into the next tick.
    pub velocity: [f64; 3],
    /// Predicted body yaw and pitch in degrees.
    pub rotation: [f32; 2],
    /// Native angular inertia, in degrees per tick.
    pub angular_velocity: f32,
    /// Downward block collision this tick.
    pub on_ground: bool,
    /// Predicted surface-water or submerged-water contact.
    pub in_water: bool,
    /// Paddle states produced by the held keys.
    pub paddles: [bool; 2],
    pub(crate) status: Option<Status>,
    pub(crate) bounds: [f64; 6],
    pub(crate) last_vertical_movement: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Status {
    Water,
    UnderWater,
    UnderFlowingWater,
    Air,
    Land,
}

impl BoatFrame {
    pub(crate) fn new(position: [f64; 3], rotation: [f32; 2], velocity: [f64; 3]) -> Self {
        Self {
            position,
            rotation,
            velocity,
            angular_velocity: 0.0,
            on_ground: false,
            in_water: false,
            paddles: [false; 2],
            status: None,
            bounds: Self::box_at(position),
            last_vertical_movement: 0.0,
        }
    }
    fn bounds(&self) -> [f64; 6] {
        self.bounds
    }
    fn box_at(position: [f64; 3]) -> [f64; 6] {
        let [x, y, z] = position;
        [
            x - WIDTH / 2.0,
            y,
            z - WIDTH / 2.0,
            x + WIDTH / 2.0,
            y + HEIGHT,
            z + WIDTH / 2.0,
        ]
    }
}

/// Refuse before changing the retained frame if the world cannot be predicted.
pub(crate) fn tick(
    version: MinecraftVersion,
    frame: &BoatFrame,
    input: VehicleInput,
    block_at: &mut impl FnMut([i32; 3]) -> Result<NativeBlockState>,
) -> Result<BoatFrame> {
    let mut level = Level {
        version,
        block_at,
        cache: HashMap::new(),
        states: HashMap::new(),
    };
    let mut next = frame.clone();
    let b = frame.bounds();
    let context = CollisionContext {
        bottom: b[1],
        descending: false,
    };
    let mut water_level = f64::NEG_INFINITY;
    let mut water = false;
    let mut submerged = None;
    for x in floor(b[0])..b[3].ceil() as i32 {
        for z in floor(b[2])..b[5].ceil() as i32 {
            for y in floor(b[1])..(b[4] + 0.001).ceil() as i32 {
                let p = [x, y, z];
                if level.block(p)?.name == "minecraft:bubble_column" {
                    return Err(blocks::unsupported("boat bubble columns".into()));
                }
                if let Some(fluid) = level.fluid(p)? {
                    if fluid.lava {
                        return Err(blocks::unsupported("boat lava".into()));
                    }
                    let surface = f64::from(y as f32 + level.fluid_height(p, fluid)?);
                    if y >= floor(b[4]) && surface > b[4] + 0.001 {
                        if fluid.amount != 8 || fluid.falling {
                            submerged = Some(Status::UnderFlowingWater);
                        } else if submerged.is_none() {
                            submerged = Some(Status::UnderWater);
                        }
                    }
                    if y < (b[1] + 0.001).ceil() as i32 {
                        water_level = water_level.max(surface);
                        water |= surface > b[1];
                    }
                }
            }
        }
    }
    let contact = [b[0], b[1] - 0.001, b[2], b[3], b[1], b[5]];
    let mut friction = 0.0f32;
    let mut count = 0u32;
    if !water {
        for x in floor(contact[0])..contact[3].ceil() as i32 {
            for y in floor(contact[1])..contact[4].ceil() as i32 {
                for z in floor(contact[2])..contact[5].ceil() as i32 {
                    let p = [x, y, z];
                    if level
                        .shape(p, context)?
                        .iter()
                        .any(|s| intersects(*s, contact))
                    {
                        friction += level.block(p)?.friction;
                        count += 1;
                    }
                }
            }
        }
    }
    let mut status = if let Some(status) = submerged {
        status
    } else if water {
        Status::Water
    } else if count > 0 {
        Status::Land
    } else {
        Status::Air
    };
    // Entity.baseTick runs before floatBoat. Reuse its audited fluid math with
    // the boat's box and the native non-player normalization of the current.
    let mut fluid_body = Body::new(next.position);
    fluid_body.bounds = b;
    fluid_body.velocity = next.velocity;
    Tick {
        version,
        rules: &version.table().physics_rules,
        env: &Environment::defaults(version),
        body: &mut fluid_body,
        level: &mut level,
        movement_order: [1, 0, 2],
    }
    .fluid_push(false, 0.014, false)?;
    next.velocity = fluid_body.velocity;
    let gravity = if status == Status::UnderFlowingWater {
        0.0007
    } else if version == MinecraftVersion::Java1_16_1 {
        f64::from(0.04f32)
    } else {
        0.04
    };
    let drag = if status == Status::UnderWater {
        0.45f32
    } else if status == Status::Land {
        friction / count as f32
    } else {
        0.9f32
    };
    // Boat.floatBoat/AbstractBoat.floatBoat: air-to-water snaps onto the surface.
    if frame.status == Some(Status::Air) && !matches!(status, Status::Air | Status::Land) {
        let end = (b[4] - frame.last_vertical_movement).ceil() as i32;
        let mut above = (end + 1) as f32;
        for y in floor(b[4])..end {
            let mut height = 0.0f32;
            for x in floor(b[0])..b[3].ceil() as i32 {
                for z in floor(b[2])..b[5].ceil() as i32 {
                    let p = [x, y, z];
                    if let Some(fluid) = level.fluid(p)? {
                        if !fluid.lava {
                            height = height.max(level.fluid_height(p, fluid)?);
                        }
                    }
                }
            }
            if height < 1.0 {
                above = y as f32 + height;
                break;
            }
        }
        let position = [
            next.position[0],
            f64::from(above - HEIGHT as f32) + 0.101,
            next.position[2],
        ];
        let target = BoatFrame::box_at(position);
        let no_collision = version == MinecraftVersion::Java1_16_1
            || !level
                .geometry(target, context)?
                .iter()
                .any(|s| intersects(*s, target));
        if no_collision {
            next.position = position;
            next.bounds = target;
            next.velocity[1] = 0.0;
        }
        status = Status::Water;
    } else {
        next.velocity[0] *= f64::from(drag);
        next.velocity[2] *= f64::from(drag);
        next.angular_velocity *= drag;
        next.velocity[1] -= gravity;
        let buoyancy = match status {
            Status::Water => (water_level - frame.position[1]) / HEIGHT,
            Status::UnderWater => f64::from(0.01f32),
            _ => 0.0,
        };
        if buoyancy > 0.0 {
            let lift = if version == MinecraftVersion::Java1_16_1 {
                0.06153846016296973
            } else {
                0.04 / 0.65
            };
            next.velocity[1] = (next.velocity[1] + buoyancy * lift) * 0.75;
        }
    }
    // All input/rotation arithmetic remains native float arithmetic.
    let left = input.strafe > 0;
    let right = input.strafe < 0;
    let forward = input.forward > 0;
    let backward = input.forward < 0;
    next.angular_velocity += if left {
        -1.0
    } else if right {
        1.0
    } else {
        0.0
    };
    next.rotation[0] += next.angular_velocity;
    let mut acceleration = if (left != right) && !forward && !backward {
        0.005f32
    } else {
        0.0
    };
    if forward {
        acceleration += 0.04f32;
    }
    if backward {
        acceleration -= 0.005f32;
    }
    next.velocity[0] += f64::from(trig(version, -next.rotation[0], false) * acceleration);
    next.velocity[2] += f64::from(trig(version, next.rotation[0], true) * acceleration);
    next.paddles = [right && !left || forward, left && !right || forward];
    if next
        .velocity
        .iter()
        .any(|v| !v.is_finite() || v.abs() > 1.0)
    {
        return Err(blocks::unsupported(
            "boat step exceeds bounded movement".into(),
        ));
    }
    let b = next.bounds();
    let region = std::array::from_fn(|i| {
        b[i] + if i < 3 {
            next.velocity[i].min(0.0)
        } else {
            next.velocity[i - 3].max(0.0)
        }
    });
    // Boat.move also invokes block movement hooks. Refuse hooks that have only
    // been reproduced for a walking player, before retaining or sending a step.
    for x in floor(region[0])..region[3].ceil() as i32 {
        for y in floor(region[1] - 0.01)..region[4].ceil() as i32 {
            for z in floor(region[2])..region[5].ceil() as i32 {
                let block = level.block([x, y, z])?;
                let ordinary_water =
                    block.name == "minecraft:water" && block.inside == blocks::Effect::Liquid;
                if (block.inside != blocks::Effect::None && !ordinary_water)
                    || block.step_on != blocks::Effect::None
                    || block.after_fall_on != blocks::Effect::None
                    || block.speed_factor != 1.0
                {
                    return Err(blocks::unsupported(format!(
                        "boat movement hook on {}",
                        block.name
                    )));
                }
            }
        }
    }
    let geometry = level.geometry(
        region,
        CollisionContext {
            bottom: b[1],
            descending: false,
        },
    )?;
    let velocity = next.velocity;
    let moved = collision::collide(version, b, velocity, &geometry, frame.on_ground, 0.0);
    next.on_ground = moved[1] != next.velocity[1] && next.velocity[1] < 0.0;
    let moved2 = moved.iter().map(|v| v * v).sum::<f64>();
    let motion2 = velocity.iter().map(|v| v * v).sum::<f64>();
    if moved2 > 1e-7 || (version == MinecraftVersion::Java1_21_11 && motion2 - moved2 < 1e-7) {
        if version == MinecraftVersion::Java1_16_1 {
            next.bounds = std::array::from_fn(|i| b[i] + moved[i % 3]);
            next.position = [
                (next.bounds[0] + next.bounds[3]) * 0.5,
                next.bounds[1],
                (next.bounds[2] + next.bounds[5]) * 0.5,
            ];
        } else {
            next.position = std::array::from_fn(|i| next.position[i] + moved[i]);
            next.bounds = BoatFrame::box_at(next.position);
        }
    }
    let hit = |i: usize| {
        if version == MinecraftVersion::Java1_16_1 {
            moved[i] != velocity[i]
        } else {
            (moved[i] - velocity[i]).abs() >= f64::from(1e-5f32)
        }
    };
    if hit(0) {
        next.velocity[0] = 0.0;
    }
    if hit(2) {
        next.velocity[2] = 0.0;
        if version == MinecraftVersion::Java1_16_1 {
            next.velocity[0] = velocity[0];
        }
    }
    if moved[1] != velocity[1] {
        next.velocity[1] = 0.0;
    }
    next.last_vertical_movement = moved[1];
    next.status = Some(status);
    next.in_water = matches!(
        status,
        Status::Water | Status::UnderWater | Status::UnderFlowingWater
    );
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn unimplemented_boat_hooks_refuse_without_mutating_seed() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            for name in [
                "minecraft:slime_block",
                "minecraft:honey_block",
                "minecraft:cobweb",
                "minecraft:bubble_column",
            ] {
                let initial = BoatFrame::new([0.5, 65.0, 0.5], [0.0; 2], [0.0; 3]);
                let saved = initial.clone();
                let hazard = super::super::oracle_tests::state(name);
                let air = super::super::oracle_tests::state("minecraft:air");
                let mut blocks = |p: [i32; 3]| {
                    Ok(
                        if p[1] == 64 || (p[1] == 65 && name == "minecraft:bubble_column") {
                            hazard.clone()
                        } else {
                            air.clone()
                        },
                    )
                };
                assert!(
                    tick(version, &initial, VehicleInput::default(), &mut blocks).is_err(),
                    "{version:?} {name}"
                );
                assert_eq!(initial, saved);
            }
        }
    }

    #[test]
    fn every_supported_boat_tick_matches_unchanged_official_methods() {
        let oracle: Value = serde_json::from_reader(flate2::read::GzDecoder::new(
            &include_bytes!("../../../data/client_api/boat_oracle.json.gz")[..],
        ))
        .unwrap();
        compare_boat_ticks(&oracle, 17);
    }

    #[test]
    fn submerged_and_flowing_boats_match_unchanged_official_methods() {
        compare_boat_ticks(&super::super::oracle_tests::fluid_control_oracle(), 33);
    }

    fn compare_boat_ticks(oracle: &Value, count: usize) {
        let origin = [1024, 100, 1024];
        let exact = |v: &Value| v.as_str().unwrap().parse::<f64>().unwrap();
        let mut differences = Vec::new();
        for (key, version) in [
            ("1.16.1", MinecraftVersion::Java1_16_1),
            ("1.21.11", MinecraftVersion::Java1_21_11),
        ] {
            let results = oracle["results"][key].as_array().unwrap();
            assert_eq!(results.len(), oracle["scenarios"].as_array().unwrap().len());
            let mut compared = 0;
            for (scenario, result) in oracle["scenarios"].as_array().unwrap().iter().zip(results) {
                assert_eq!(scenario["name"], result["name"]);
                if scenario["boat"].as_bool() != Some(true) {
                    continue;
                }
                compared += 1;
                let cells = super::super::oracle_tests::world(scenario, result);
                let air = super::super::oracle_tests::state("minecraft:air");
                let position = std::array::from_fn(|i| {
                    scenario["start"][i].as_f64().unwrap() + f64::from(origin[i])
                });
                let mut frame = BoatFrame::new(
                    position,
                    [scenario["yaw"].as_f64().unwrap_or(0.0) as f32, 0.0],
                    [0.0; 3],
                );
                let mut block = |p: [i32; 3]| {
                    Ok(cells
                        .get(&std::array::from_fn(|i| p[i] - origin[i]))
                        .cloned()
                        .unwrap_or_else(|| air.clone()))
                };
                let controls = scenario["ticks"].as_array().unwrap();
                let frames = result["frames"].as_array().unwrap();
                assert_eq!(controls.len(), frames.len());
                for (index, (input, expected)) in controls.iter().zip(frames).enumerate() {
                    let input = VehicleInput {
                        forward: input["forward"].as_i64().unwrap_or(0) as i8,
                        strafe: input["strafe"].as_i64().unwrap_or(0) as i8,
                        jump: false,
                    };
                    frame = tick(version, &frame, input, &mut block).unwrap();
                    let position = frame.position;
                    let mismatch = (0..3).find(|&i| {
                        position[i] - f64::from(origin[i]) != exact(&expected["position"][i])
                            || frame.velocity[i] != exact(&expected["velocity"][i])
                    });
                    if let Some(axis) = mismatch {
                        differences.push(format!(
                            "{key} {} tick {index} axis {axis}: p {} vs {}, v {} vs {}",
                            scenario["name"],
                            position[axis] - f64::from(origin[axis]),
                            expected["position"][axis],
                            frame.velocity[axis],
                            expected["velocity"][axis]
                        ));
                        break;
                    }
                    assert_eq!(
                        frame.rotation,
                        std::array::from_fn(|i| exact(&expected["rotation"][i]) as f32),
                        "{key} {} tick {index}",
                        scenario["name"]
                    );
                    assert_eq!(
                        frame.angular_velocity,
                        exact(&expected["angular_velocity"]) as f32
                    );
                    assert_eq!(frame.on_ground, expected["on_ground"].as_bool().unwrap());
                    assert_eq!(frame.in_water, expected["in_water"].as_bool().unwrap());
                    if let Some(status) = expected["water_status"].as_str() {
                        let actual = match frame.status.unwrap() {
                            Status::Water => "IN_WATER",
                            Status::UnderWater => "UNDER_WATER",
                            Status::UnderFlowingWater => "UNDER_FLOWING_WATER",
                            Status::Air => "IN_AIR",
                            Status::Land => "ON_LAND",
                        };
                        assert_eq!(actual, status, "{key} {} tick {index}", scenario["name"]);
                    }
                    assert_eq!(
                        frame.paddles,
                        std::array::from_fn(|i| expected["paddles"][i].as_bool().unwrap())
                    );
                }
            }
            assert_eq!(compared, count);
        }
        assert!(differences.is_empty(), "{}", differences.join("\n"));
    }
}
