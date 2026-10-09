//! Mounted boat motion using the pinned Boat/AbstractBoat movement rules.
//! Surface/submerged/flowing water, bubble interiors, air, audited block
//! collision and nonliving terrain callbacks. Surface bubble launch/ejection
//! belongs to the server. Rigid entity boxes are explicit sampled model inputs.
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
    pub(crate) stuck: [f64; 3],
    pub(crate) supporting_block: Option<[i32; 3]>,
    pub(crate) on_ground_no_blocks: bool,
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
            stuck: [0.0; 3],
            supporting_block: None,
            on_ground_no_blocks: false,
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
    entity_boxes: &[[f64; 6]],
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
            || (!entity_boxes
                .iter()
                .any(|s| intersects(*s, deflate(target, -1e-7)))
                && !level
                    .geometry(target, context)?
                    .iter()
                    .any(|s| intersects(*s, target)));
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
    if next.velocity.iter().any(|v| !v.is_finite())
        || next.velocity[0].abs() > 1.0
        || next.velocity[2].abs() > 1.0
        || next.velocity[1].abs() > 3.0
    {
        return Err(blocks::unsupported(
            "boat step exceeds bounded movement".into(),
        ));
    }
    let mut motion = next.velocity;
    // Entity.move consumes the previous interior callback's multiplier once.
    // It scales the requested displacement and clears retained velocity before
    // collision/landing hooks. A new receipt does not erase that callback state.
    if next.stuck.iter().map(|v| v * v).sum::<f64>() > 1e-7 {
        motion = std::array::from_fn(|axis| motion[axis] * next.stuck[axis]);
        next.stuck = [0.0; 3];
        next.velocity = [0.0; 3];
    }
    let b = next.bounds();
    let region = std::array::from_fn(|i| {
        b[i] + if i < 3 {
            motion[i].min(0.0)
        } else {
            motion[i - 3].max(0.0)
        }
    });
    // Boat.move invokes Entity/block callbacks. Only audited nonliving hooks
    // are admitted; unknown hooks refuse before retaining or sending a step.
    for x in floor(region[0])..region[3].ceil() as i32 {
        for y in floor(region[1] - 0.01)..region[4].ceil() as i32 {
            for z in floor(region[2])..region[5].ceil() as i32 {
                let block = level.block([x, y, z])?;
                let ordinary_water = block.name == "minecraft:water"
                    && block.inside == blocks::Effect::Liquid
                    || block.name == "minecraft:bubble_column"
                        && block.inside == blocks::Effect::BubbleColumn;
                let inside = matches!(block.inside, blocks::Effect::None | blocks::Effect::Honey)
                    || ordinary_water
                    || block.inside == blocks::Effect::Stuck
                        && matches!(
                            block.name.as_str(),
                            "minecraft:cobweb" | "minecraft:sweet_berry_bush"
                        );
                if !inside
                    || !matches!(block.step_on, blocks::Effect::None | blocks::Effect::Slime)
                    || !matches!(
                        block.after_fall_on,
                        blocks::Effect::None | blocks::Effect::Slime | blocks::Effect::Bed
                    )
                {
                    return Err(blocks::unsupported(format!(
                        "boat movement hook on {}",
                        block.name
                    )));
                }
            }
        }
    }
    let mut geometry = level.geometry(
        region,
        CollisionContext {
            bottom: b[1],
            descending: false,
        },
    )?;
    let selected: Vec<_> = entity_boxes
        .iter()
        .copied()
        .filter(|s| intersects(*s, deflate(region, -1e-7)))
        .collect();
    geometry.prepend_entities(&selected);
    let velocity = next.velocity;
    let moved = collision::collide(version, b, motion, &geometry, frame.on_ground, 0.0);
    next.on_ground = moved[1] != motion[1] && motion[1] < 0.0;
    let moved2 = moved.iter().map(|v| v * v).sum::<f64>();
    let motion2 = motion.iter().map(|v| v * v).sum::<f64>();
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
            moved[i] != motion[i]
        } else {
            (moved[i] - motion[i]).abs() >= f64::from(1e-5f32)
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
    terrain_effect(
        &mut level,
        &mut next,
        TerrainPhase::Collision {
            movement: moved,
            vertical_hit: moved[1] != motion[1],
        },
    )?;
    next.last_vertical_movement = moved[1];
    if version == MinecraftVersion::Java1_16_1 {
        let bounds = next.bounds();
        for x in floor(bounds[0] + 0.001)..=floor(bounds[3] - 0.001) {
            for y in floor(bounds[1] + 0.001)..=floor(bounds[4] - 0.001) {
                for z in floor(bounds[2] + 0.001)..=floor(bounds[5] - 0.001) {
                    interior_effect(&mut level, &mut next, [x, y, z], true)?;
                }
            }
        }
        terrain_effect(&mut level, &mut next, TerrainPhase::Speed)?;
    } else {
        terrain_effect(&mut level, &mut next, TerrainPhase::Speed)?;
        let order = if motion[0].abs() < motion[2].abs() {
            [1, 2, 0]
        } else {
            [1, 0, 2]
        };
        // AbstractBoat.tick applies block effects twice. The first consumes the
        // self-movement steps; the second visits the stationary destination.
        for from in [frame.position, next.position] {
            terrain_effect(&mut level, &mut next, TerrainPhase::Step)?;
            for (cell, certain) in bubbles::visits(from, next.position, order, |p| {
                deflate(BoatFrame::box_at(p), f64::from(1.0e-5f32))
            })? {
                interior_effect(&mut level, &mut next, cell, certain)?;
            }
        }
    }
    next.status = Some(status);
    next.in_water = matches!(
        status,
        Status::Water | Status::UnderWater | Status::UnderFlowingWater
    );
    Ok(next)
}

enum TerrainPhase {
    Collision {
        movement: [f64; 3],
        vertical_hit: bool,
    },
    Step,
    Speed,
}

fn terrain_effect<F: FnMut([i32; 3]) -> Result<NativeBlockState>>(
    level: &mut Level<'_, F>,
    frame: &mut BoatFrame,
    phase: TerrainPhase,
) -> Result<()> {
    let version = level.version;
    let mut body = Body::new(frame.position);
    body.bounds = frame.bounds();
    body.velocity = frame.velocity;
    body.on_ground = frame.on_ground;
    body.supporting_block = frame.supporting_block;
    body.on_ground_no_blocks = frame.on_ground_no_blocks;
    let environment = Environment::defaults(version);
    let mut tick = Tick {
        version,
        rules: &version.table().physics_rules,
        env: &environment,
        body: &mut body,
        level,
        movement_order: [1, 0, 2],
    };
    match phase {
        TerrainPhase::Collision {
            movement,
            vertical_hit,
        } => {
            if version == MinecraftVersion::Java1_21_11 {
                // Same Entity support search and tie breaking, using the boat
                // bounds; no standing-player dimensions are substituted.
                tick.check_supporting_block(movement)?;
            }
            let position = tick.landing_position()?;
            let block = tick.level.block(position)?;
            if vertical_hit {
                let y = &mut tick.body.velocity[1];
                match block.after_fall_on {
                    blocks::Effect::None => *y = 0.0,
                    blocks::Effect::Slime if *y < 0.0 => *y = -*y * 0.8,
                    blocks::Effect::Bed if *y < 0.0 => *y = -*y * f64::from(0.66f32) * 0.8,
                    blocks::Effect::Slime | blocks::Effect::Bed => {}
                    other => return Err(blocks::unsupported(format!("boat landing: {other:?}"))),
                }
            }
            if version == MinecraftVersion::Java1_16_1 && tick.body.on_ground {
                tick.step_on(block)?;
            }
        }
        TerrainPhase::Step => {
            if tick.body.on_ground {
                let position = tick.landing_position()?;
                let block = tick.level.block(position)?;
                tick.step_on(block)?;
            }
        }
        TerrainPhase::Speed => {
            let feet = tick.level.block(tick.block_position())?;
            let own = feet.speed_factor;
            let factor = if matches!(
                feet.name.as_str(),
                "minecraft:water" | "minecraft:bubble_column"
            ) || own != 1.0
            {
                own
            } else {
                let below = if version == MinecraftVersion::Java1_16_1 {
                    [
                        floor(frame.position[0]),
                        floor(frame.bounds()[1] - 0.5000001),
                        floor(frame.position[2]),
                    ]
                } else {
                    tick.on_pos(0.500001f32)?
                };
                tick.level.block(below)?.speed_factor
            };
            tick.body.velocity[0] *= f64::from(factor);
            tick.body.velocity[2] *= f64::from(factor);
        }
    }
    frame.velocity = tick.body.velocity;
    frame.supporting_block = tick.body.supporting_block;
    frame.on_ground_no_blocks = tick.body.on_ground_no_blocks;
    Ok(())
}

fn interior_effect<F: FnMut([i32; 3]) -> Result<NativeBlockState>>(
    level: &mut Level<'_, F>,
    frame: &mut BoatFrame,
    cell: [i32; 3],
    certain: bool,
) -> Result<()> {
    match level.block(cell)?.inside {
        blocks::Effect::BubbleColumn => bubble_effect(level, frame, cell, certain)?,
        blocks::Effect::Stuck if level.block(cell)?.name == "minecraft:cobweb" => {
            // Native WebBlock applies to every Entity; Weaving is LivingEntity
            // only. Its modern callback ignores the intersection boolean.
            frame.stuck = [0.25, f64::from(0.05f32), 0.25];
        }
        blocks::Effect::Honey => {
            let position = frame.position;
            let old_y = if level.version == MinecraftVersion::Java1_16_1 {
                frame.velocity[1]
            } else {
                frame.velocity[1] / f64::from(0.98f32) + 0.08
            };
            let edge = 0.4375 + WIDTH / 2.0;
            if !frame.on_ground
                && position[1] <= f64::from(cell[1]) + 0.9375 - 1e-7
                && old_y < -0.08
                && ((f64::from(cell[0]) + 0.5 - position[0]).abs() + 1e-7 > edge
                    || (f64::from(cell[2]) + 0.5 - position[2]).abs() + 1e-7 > edge)
            {
                if old_y < -0.13 {
                    let ratio = -0.05 / old_y;
                    frame.velocity[0] *= ratio;
                    frame.velocity[2] *= ratio;
                }
                frame.velocity[1] = if level.version == MinecraftVersion::Java1_16_1 {
                    -0.05
                } else {
                    (-0.05 - 0.08) * f64::from(0.98f32)
                };
            }
        }
        // SweetBerryBushBlock's slowdown is LivingEntity-only. Ordinary water
        // and server-owned damage/fire callbacks add no boat interior velocity.
        _ => {}
    }
    Ok(())
}

fn bubble_effect<F: FnMut([i32; 3]) -> Result<NativeBlockState>>(
    level: &mut Level<'_, F>,
    frame: &mut BoatFrame,
    cell: [i32; 3],
    certain: bool,
) -> Result<()> {
    if !certain || level.block(cell)?.inside != blocks::Effect::BubbleColumn {
        return Ok(());
    }
    let above = [cell[0], cell[1] + 1, cell[2]];
    let surface = if level.version == MinecraftVersion::Java1_16_1 {
        level.is_air(above)?
    } else {
        level.fluid(above)?.is_none()
            && level
                .shape(
                    above,
                    CollisionContext {
                        bottom: f64::INFINITY,
                        descending: false,
                    },
                )?
                .is_empty()
    };
    // Boat overrides the surface hook: the server owns its timer,
    // launch and ejection. The client surface hook adds no impulse.
    if !surface {
        let down = level
            .state(cell)?
            .properties
            .get("drag")
            .map(String::as_str)
            == Some("true");
        frame.velocity[1] = if down {
            (-0.3f64).max(frame.velocity[1] - 0.03)
        } else {
            0.7f64.min(frame.velocity[1] + 0.06)
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn unimplemented_boat_hooks_refuse_without_mutating_seed() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            for name in ["minecraft:lava", "minecraft:moving_piston"] {
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
                    tick(version, &initial, VehicleInput::default(), &[], &mut blocks).is_err(),
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

    #[test]
    fn bubbles_and_velocity_changes_match_original_boat_movement() {
        let oracle: Value = serde_json::from_reader(flate2::read::GzDecoder::new(
            &include_bytes!("../../../data/client_api/boat_bubble_oracle.json.gz")[..],
        ))
        .unwrap();
        compare_boat_ticks(&oracle, 16);
    }

    #[test]
    fn special_block_hooks_match_original_nonliving_boat_callbacks() {
        let oracle: Value = serde_json::from_reader(flate2::read::GzDecoder::new(
            &include_bytes!("../../../data/client_api/boat_hooks_oracle.json.gz")[..],
        ))
        .unwrap();
        compare_boat_ticks(&oracle, 14);
    }

    #[test]
    fn rigid_vehicle_collisions_match_original_world_queries() {
        let oracle: Value = serde_json::from_reader(flate2::read::GzDecoder::new(
            &include_bytes!("../../../data/client_api/boat_collision_oracle.json.gz")[..],
        ))
        .unwrap();
        compare_boat_ticks(&oracle, 6);
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
                // Independently exported original Entity.getBoundingBox values.
                // Do not construct the expected shape with the SDK's dimensions.
                let entity_boxes: Vec<[f64; 6]> = result["collision_boxes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|b| std::array::from_fn(|i| exact(&b[i]) + f64::from(origin[i % 3])))
                    .collect();
                let frames = result["frames"].as_array().unwrap();
                assert_eq!(controls.len(), frames.len());
                for (index, (input, expected)) in controls.iter().zip(frames).enumerate() {
                    if let Some(value) = input["received_boat_velocity"].as_array() {
                        frame.velocity = std::array::from_fn(|axis| value[axis].as_f64().unwrap());
                    }
                    let input = VehicleInput {
                        forward: input["forward"].as_i64().unwrap_or(0) as i8,
                        strafe: input["strafe"].as_i64().unwrap_or(0) as i8,
                        jump: false,
                    };
                    frame = tick(version, &frame, input, &entity_boxes, &mut block).unwrap();
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
