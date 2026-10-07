//! Version-selected bounded dry-terrain motion rules, shared by adapter admission.
//! Native float/trig/input and collision fixtures are independently generated.
use super::{MAX_SURVIVAL_CONTROL_TICKS, PredictedMotionFrame, SurvivalControl, SurvivalInput};
use crate::versions::table::{SmallVelocity, StepSearch};
use crate::{Error, ErrorKind, MinecraftVersion, Result};

fn invalid(message: &str) -> Error {
    Error::new(ErrorKind::InvalidInput, anyhow::anyhow!("{message}"))
}
pub(crate) fn validate_controls(controls: &[SurvivalControl]) -> Result<()> {
    if controls.is_empty()
        || controls.len() > MAX_SURVIVAL_CONTROL_TICKS
        || controls.iter().any(|control| {
            !control.yaw.is_finite()
                || !(-1..=1).contains(&control.input.forward)
                || !(-1..=1).contains(&control.input.strafe)
        })
    {
        return Err(invalid("motion requires 1..120 bounded digital inputs"));
    }
    Ok(())
}
pub(crate) fn predict(
    mut block_at: impl FnMut([i32; 3]) -> Result<crate::NativeBlockState>,
    model: &mut Model,
    controls: &[SurvivalControl],
) -> Result<Vec<PredictedMotionFrame>> {
    validate_controls(controls)?;
    let origin_y = model.frame.position[1];
    model.frame.tick = 0;
    let mut frames = Vec::with_capacity(controls.len());
    for control in controls {
        let proposed = model.intent(control.input, control.yaw);
        if proposed.iter().any(|value| value.abs() > 1.0) {
            return Err(invalid("motion exceeds bounded dry preview step"));
        }
        let boxes = geometry(model.version, &mut block_at, model.frame.position, proposed)?;
        model.advance(control.input, proposed, &boxes);
        if model.frame.position[1] < origin_y - 3.0 {
            return Err(invalid("preview falls outside bounded construction height"));
        }
        frames.push(model.frame.clone());
    }
    Ok(frames)
}
pub(crate) fn trig(version: MinecraftVersion, angle: f32, cosine: bool) -> f32 {
    if version.table().physics_rules.modern_trig {
        let index = ((f64::from(angle) * 10430.378350470453 + if cosine { 16384.0 } else { 0.0 })
            as i64
            & 65535) as i32;
        (f64::from(index) / 10430.378350470453).sin() as f32
    } else {
        let index =
            ((angle * 10430.378f32 + if cosine { 16384.0f32 } else { 0.0f32 }) as i32) & 65535;
        (f64::from(index) * std::f64::consts::PI * 2.0 / 65536.0).sin() as f32
    }
}

pub(crate) const DRY_CUBES: &[&str] = &[
    "minecraft:stone",
    "minecraft:dirt",
    "minecraft:grass_block",
    "minecraft:cobblestone",
    "minecraft:oak_planks",
    "minecraft:spruce_planks",
    "minecraft:quartz_block",
    "minecraft:smooth_quartz",
    "minecraft:white_concrete",
    "minecraft:glass",
    "minecraft:andesite",
    "minecraft:granite",
];

pub(crate) fn body(p: [f64; 3]) -> [f64; 6] {
    let half = f64::from(0.6f32) / 2.0;
    [
        p[0] - half,
        p[1],
        p[2] - half,
        p[0] + half,
        p[1] + f64::from(1.8f32),
        p[2] + half,
    ]
}
/// Preserve each original VoxelShape boundary: native epsilon handling occurs
/// once per shape, rather than once per component of a stairs union.
#[derive(Default)]
pub(crate) struct CollisionGeometry {
    boxes: Vec<[f64; 6]>,
    shapes: Vec<std::ops::Range<usize>>,
}
impl std::ops::Deref for CollisionGeometry {
    type Target = [[f64; 6]];
    fn deref(&self) -> &Self::Target {
        &self.boxes
    }
}
impl CollisionGeometry {
    #[cfg(test)]
    pub(crate) fn joined(boxes: &[[f64; 6]]) -> Self {
        Self {
            boxes: boxes.to_vec(),
            shapes: std::iter::once(0..boxes.len()).collect(),
        }
    }
    fn groups(&self) -> impl Iterator<Item = &[[f64; 6]]> + Clone {
        self.shapes.iter().map(|range| &self.boxes[range.clone()])
    }
}
pub(crate) fn geometry(
    version: MinecraftVersion,
    mut block_at: impl FnMut([i32; 3]) -> Result<crate::NativeBlockState>,
    p: [f64; 3],
    motion: [f64; 3],
) -> Result<CollisionGeometry> {
    let a = body(p);
    let b = body(std::array::from_fn(|i| p[i] + motion[i]));
    let min: [i32; 3] = std::array::from_fn(|i| a[i].min(b[i]).floor() as i32 - 1);
    let max: [i32; 3] = std::array::from_fn(|i| a[i + 3].max(b[i + 3]).floor() as i32 + 1);
    let mut geometry = CollisionGeometry::default();
    for x in min[0]..=max[0] {
        for y in min[1]..=max[1] {
            for z in min[2]..=max[2] {
                let p = [x, y, z];
                let block = block_at(p)?;
                let start = geometry.boxes.len();
                for bounds in collision_shape(version, &block)? {
                    geometry.boxes.push(std::array::from_fn(|axis| {
                        bounds[axis] + f64::from(p[axis % 3])
                    }));
                }
                if geometry.boxes.len() > start {
                    geometry.shapes.push(start..geometry.boxes.len());
                }
            }
        }
    }
    Ok(geometry)
}
/// Default dry cubes or a complete originally registered dry slab/stair/rail state.
/// Fluids and unmodeled shape/effect semantics remain explicitly unsupported.
pub(crate) fn collision_shape(
    version: MinecraftVersion,
    block: &crate::NativeBlockState,
) -> Result<&'static [[f64; 6]]> {
    const CUBE: &[[f64; 6]] = &[[0., 0., 0., 1., 1., 1.]];
    if matches!(
        block.name.as_str(),
        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
    ) {
        return Ok(&[]);
    }
    if DRY_CUBES.contains(&block.name.as_str()) {
        return Ok(CUBE);
    }
    super::terrain::lookup(version, block)
        .map(|shape| shape.collision.as_slice())
        .ok_or_else(|| {
            Error::new(
                ErrorKind::Unsupported,
                anyhow::anyhow!(
                    "unsupported dry terrain state {} {:?}",
                    block.name,
                    block.properties
                ),
            )
        })
}
pub(crate) fn acceleration(
    version: MinecraftVersion,
    input: SurvivalInput,
    yaw: f32,
    speed: f32,
) -> [f64; 3] {
    let (constants, rules) = (&version.table().physics, &version.table().physics_rules);
    let drag = constants.input_drag;
    let (mut x, mut z) = if !rules.normalize_input {
        (
            f64::from(f32::from(input.strafe) * drag),
            f64::from(f32::from(input.forward) * drag),
        )
    } else {
        let (mut x, mut z) = (f32::from(input.strafe), f32::from(input.forward));
        let length = (x * x + z * z).sqrt();
        if length == 0.0 {
            return [0.0; 3];
        }
        x = (x / length) * drag;
        z = (z / length) * drag;
        let length = (x * x + z * z).sqrt();
        let (nx, nz) = (x * (1.0 / length), z * (1.0 / length));
        let ratio = nx.abs().min(nz.abs()) / nx.abs().max(nz.abs());
        let magnitude = (length * (1.0 + ratio * ratio).sqrt()).min(1.0);
        let (x, z) = (f64::from(nx * magnitude), f64::from(nz * magnitude));
        (x, z)
    };
    let length = x * x + z * z;
    if length < constants.min_input_sq {
        return [0.0; 3];
    }
    if length > 1.0 {
        let inverse = if rules.f32_input_length {
            1.0 / f64::from(length.sqrt() as f32)
        } else {
            1.0 / length.sqrt()
        };
        x *= inverse;
        z *= inverse;
    }
    x *= f64::from(speed);
    z *= f64::from(speed);
    let angle = yaw * (std::f32::consts::PI / 180.0);
    let (s, c) = (
        f64::from(trig(version, angle, false)),
        f64::from(trig(version, angle, true)),
    );
    [x * c - z * s, 0.0, z * c + x * s]
}
#[cfg(test)]
pub(crate) fn collide(bounds: [f64; 6], motion: [f64; 3], boxes: &[[f64; 6]]) -> [f64; 3] {
    collide_groups(bounds, motion, boxes.iter().map(std::slice::from_ref))
}
pub(crate) fn collide_geometry(
    bounds: [f64; 6],
    motion: [f64; 3],
    geometry: &CollisionGeometry,
) -> [f64; 3] {
    collide_groups(bounds, motion, geometry.groups())
}
fn collide_groups<'a>(
    mut bounds: [f64; 6],
    motion: [f64; 3],
    shapes: impl Iterator<Item = &'a [[f64; 6]]> + Clone,
) -> [f64; 3] {
    let mut result = [0.0; 3];
    let order = if motion[0].abs() < motion[2].abs() {
        [1, 2, 0]
    } else {
        [1, 0, 2]
    };
    for axis in order {
        let mut distance = motion[axis];
        if distance == 0.0 {
            continue;
        }
        for shape in shapes.clone() {
            if distance.abs() < 1e-7 {
                distance = 0.0;
                break;
            }
            let positive = distance > 0.0;
            for cube in shape {
                if (0..3).any(|i| {
                    i != axis
                        && (bounds[i] + 1e-7 >= cube[i + 3] || bounds[i + 3] - 1e-7 <= cube[i])
                }) {
                    continue;
                }
                let ahead = cube[axis] - bounds[axis + 3];
                let behind = cube[axis + 3] - bounds[axis];
                if positive && ahead >= -1e-7 {
                    distance = distance.min(ahead);
                } else if !positive && behind <= 1e-7 {
                    distance = distance.max(behind);
                }
            }
        }
        result[axis] = distance;
        bounds[axis] += distance;
        bounds[axis + 3] += distance;
    }
    result
}
// Native Entity.adjustMovementForCollisions step search, restricted to the
// already admitted dry terrain and native 0.6f step height.
fn collide_with_step(
    version: MinecraftVersion,
    bounds: [f64; 6],
    motion: [f64; 3],
    boxes: &CollisionGeometry,
    on_ground: bool,
) -> [f64; 3] {
    let height = version.table().physics.step_height;
    match version.table().physics_rules.step {
        StepSearch::TwoCandidate => {
            legacy_collide_with_step(bounds, motion, boxes, on_ground, height)
        }
        StepSearch::HeightScan => {
            modern_collide_with_step(bounds, motion, boxes, on_ground, height)
        }
    }
}
// Entity.collide's older two-candidate step search. Do not use the modern
// voxel-height search as a substitute for this version's native rules.
fn legacy_collide_with_step(
    bounds: [f64; 6],
    motion: [f64; 3],
    boxes: &CollisionGeometry,
    on_ground: bool,
    step_height: f32,
) -> [f64; 3] {
    let adjusted = collide_geometry(bounds, motion, boxes);
    if !(on_ground || (motion[1] < 0.0 && motion[1] != adjusted[1]))
        || (motion[0] == adjusted[0] && motion[2] == adjusted[2])
    {
        return adjusted;
    }
    let height = f64::from(step_height);
    let mut candidate = collide_geometry(bounds, [motion[0], height, motion[2]], boxes);
    let mut expanded = bounds;
    expanded[0] += motion[0].min(0.0);
    expanded[3] += motion[0].max(0.0);
    expanded[2] += motion[2].min(0.0);
    expanded[5] += motion[2].max(0.0);
    let up = collide_geometry(expanded, [0.0, height, 0.0], boxes);
    if up[1] < height {
        let alternate = collide_geometry(shifted(bounds, up), [motion[0], 0.0, motion[2]], boxes);
        let alternate = std::array::from_fn(|i| alternate[i] + up[i]);
        if horizontal_length(alternate) > horizontal_length(candidate) {
            candidate = alternate;
        }
    }
    if horizontal_length(candidate) > horizontal_length(adjusted) {
        let down = collide_geometry(
            shifted(bounds, candidate),
            [0.0, motion[1] - candidate[1], 0.0],
            boxes,
        );
        std::array::from_fn(|i| candidate[i] + down[i])
    } else {
        adjusted
    }
}
fn shifted(bounds: [f64; 6], delta: [f64; 3]) -> [f64; 6] {
    std::array::from_fn(|i| bounds[i] + delta[i % 3])
}
fn horizontal_length(delta: [f64; 3]) -> f64 {
    delta[0] * delta[0] + delta[2] * delta[2]
}
fn modern_collide_with_step(
    bounds: [f64; 6],
    motion: [f64; 3],
    boxes: &CollisionGeometry,
    on_ground: bool,
    step_height: f32,
) -> [f64; 3] {
    let adjusted = collide_geometry(bounds, motion, boxes);
    let downward = motion[1] < 0.0 && motion[1] != adjusted[1];
    if !(downward || on_ground) || (motion[0] == adjusted[0] && motion[2] == adjusted[2]) {
        return adjusted;
    }
    let mut base = bounds;
    if downward {
        base[1] += adjusted[1];
        base[4] += adjusted[1];
    }
    let mut scan = base;
    scan[0] += motion[0].min(0.0);
    scan[3] += motion[0].max(0.0);
    scan[2] += motion[2].min(0.0);
    scan[5] += motion[2].max(0.0);
    scan[4] += f64::from(step_height);
    if !downward {
        scan[1] -= f64::from(1e-5f32);
    }
    let mut heights: Vec<f32> = boxes
        .iter()
        .filter(|b| (0..3).all(|i| scan[i] < b[i + 3] && scan[i + 3] > b[i]))
        .flat_map(|b| [b[1], b[4]])
        .map(|y| (y - base[1]) as f32)
        .filter(|h| *h >= 0.0 && *h <= step_height && *h != adjusted[1] as f32)
        .collect();
    heights.sort_by(f32::total_cmp);
    heights.dedup();
    for height in heights {
        let mut candidate =
            collide_geometry(base, [motion[0], f64::from(height), motion[2]], boxes);
        if candidate[0] * candidate[0] + candidate[2] * candidate[2]
            > adjusted[0] * adjusted[0] + adjusted[2] * adjusted[2]
        {
            candidate[1] -= bounds[1] - base[1];
            return candidate;
        }
    }
    adjusted
}

#[derive(Clone, Debug)]
pub(crate) struct Model {
    version: MinecraftVersion,
    pub(crate) frame: PredictedMotionFrame,
    jump_cooldown: u8,
}
impl Model {
    pub(crate) fn initial_frame(&self) -> PredictedMotionFrame {
        PredictedMotionFrame {
            tick: 0,
            ..self.frame.clone()
        }
    }
    pub(crate) fn new(version: MinecraftVersion, position: [f64; 3]) -> Self {
        Self {
            version,
            frame: PredictedMotionFrame {
                tick: 0,
                position,
                velocity: [0.0; 3],
                on_ground: true,
                horizontal_collision: false,
                resting: true,
            },
            jump_cooldown: 0,
        }
    }
    pub(crate) fn intent(&mut self, input: SurvivalInput, yaw: f32) -> [f64; 3] {
        let (c, rules) = (
            &self.version.table().physics,
            &self.version.table().physics_rules,
        );
        self.jump_cooldown = self.jump_cooldown.saturating_sub(1);
        let mut v = self.frame.velocity;
        match rules.small_velocity {
            SmallVelocity::PerAxis => {
                if v[0].abs() < c.small_velocity {
                    v[0] = 0.0;
                }
                if v[2].abs() < c.small_velocity {
                    v[2] = 0.0;
                }
            }
            SmallVelocity::HorizontalLength => {
                if v[0] * v[0] + v[2] * v[2] < 9e-6 {
                    v[0] = 0.0;
                    v[2] = 0.0;
                }
            }
        }
        if v[1].abs() < c.small_velocity {
            v[1] = 0.0;
        }
        if input.jump && self.frame.on_ground && self.jump_cooldown == 0 {
            let jump = f64::from(c.jump_velocity);
            v[1] = if rules.jump_keeps_rising {
                v[1].max(jump)
            } else {
                jump
            };
            self.jump_cooldown = c.jump_cooldown_ticks;
        } else if !input.jump {
            self.jump_cooldown = 0;
        }
        // Native float evaluation: f32 throughout, widened once.
        let slip = c.default_slipperiness;
        let speed = if self.frame.on_ground {
            c.base_movement_speed * (c.ground_acceleration / (slip * slip * slip))
        } else {
            c.air_acceleration
        };
        let a = acceleration(self.version, input, yaw, speed);
        std::array::from_fn(|i| v[i] + a[i])
    }
    pub(crate) fn advance(
        &mut self,
        input: SurvivalInput,
        proposed: [f64; 3],
        boxes: &CollisionGeometry,
    ) {
        let adjusted = collide_with_step(
            self.version,
            body(self.frame.position),
            proposed,
            boxes,
            self.frame.on_ground,
        );
        let (c, rules) = (
            &self.version.table().physics,
            &self.version.table().physics_rules,
        );
        let friction = if self.frame.on_ground {
            c.default_slipperiness * c.air_friction
        } else {
            c.air_friction
        };
        let mut velocity = proposed;
        for axis in 0..3 {
            let changed = match rules.horizontal_collision_tolerance {
                Some(tolerance) if axis != 1 => {
                    (proposed[axis] - adjusted[axis]).abs() >= tolerance
                }
                _ => proposed[axis] != adjusted[axis],
            };
            if changed {
                velocity[axis] = 0.0;
            }
        }
        self.frame.tick += 1;
        let length2 = adjusted.iter().map(|v| v * v).sum::<f64>();
        let move_position = length2 > c.min_input_sq
            || (rules.move_when_nearly_stopped
                && proposed.iter().map(|v| v * v).sum::<f64>() - length2 < c.min_input_sq);
        if move_position {
            self.frame.position = std::array::from_fn(|i| self.frame.position[i] + adjusted[i]);
        }
        self.frame.velocity = [
            velocity[0] * f64::from(friction),
            (velocity[1] - c.gravity) * f64::from(c.vertical_drag),
            velocity[2] * f64::from(friction),
        ];
        self.frame.on_ground = proposed[1] < 0.0 && proposed[1] != adjusted[1];
        self.frame.horizontal_collision =
            (proposed[0] - adjusted[0]).abs() >= 1e-5 || (proposed[2] - adjusted[2]).abs() >= 1e-5;
        self.frame.resting = self.frame.on_ground
            && (!move_position || adjusted == [0.0; 3])
            && input.forward == 0
            && input.strafe == 0
            && !input.jump;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_motion_primitives_match_unmodified_native_methods() {
        let data: serde_json::Value = serde_json::from_str(include_str!(
            "../../../data/client_api/java_1_16_1_dry_movement.json"
        ))
        .unwrap();
        assert_eq!(
            data["standing_dimensions"][0].as_f64(),
            Some(f64::from(0.6f32))
        );
        assert_eq!(
            data["standing_dimensions"][1].as_f64(),
            Some(f64::from(1.8f32))
        );
        for material in DRY_CUBES {
            let values = &data["materials"][material];
            assert_eq!(values[0].as_f64(), Some(f64::from(0.6f32)));
            assert_eq!(values[1].as_f64(), Some(1.0));
            assert_eq!(values[2].as_f64(), Some(1.0));
            assert_eq!(
                data["material_shapes"][material],
                serde_json::json!([[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]])
            );
        }
        for case in data["inputs"].as_array().unwrap() {
            let input = SurvivalInput {
                strafe: case["strafe"].as_i64().unwrap() as i8,
                forward: case["forward"].as_i64().unwrap() as i8,
                jump: false,
            };
            let actual = acceleration(
                MinecraftVersion::Java1_16_1,
                input,
                case["yaw"].as_f64().unwrap() as f32,
                case["speed"].as_f64().unwrap() as f32,
            );
            for (i, value) in actual.into_iter().enumerate() {
                assert!(
                    (value - case["expected"][i].as_f64().unwrap()).abs() < 1e-10,
                    "legacy input {case}: {actual:?}"
                );
            }
        }
        for case in data["collisions"].as_array().unwrap() {
            let actual = collide(
                body(serde_json::from_value(case["position"].clone()).unwrap()),
                serde_json::from_value(case["motion"].clone()).unwrap(),
                &serde_json::from_value::<Vec<[f64; 6]>>(case["boxes"].clone()).unwrap(),
            );
            for (i, value) in actual.into_iter().enumerate() {
                assert!(
                    (value - case["expected"][i].as_f64().unwrap()).abs() < 1e-10,
                    "legacy collision {case}: {actual:?}"
                );
            }
        }
    }
}
