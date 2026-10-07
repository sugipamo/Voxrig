//! Shared physics engine for the client player's own movement (design:
//! docs/physics-design.md). One `tick` reproduces one native client tick of the
//! local player; version differences come from `PhysicsRules`. Pure: no I/O.
//!
//! Scope (P3): any terrain whose collision shapes are pure functions of the block
//! state, block friction/speed/jump factors, slime and bed bounces, cobwebs and
//! berry bushes, sneaking (including edge back-off), sprinting, movement
//! attributes and effects. Outside it the tick returns `ErrorKind::Unsupported`
//! before changing state: fluids, climbing, flying, riding, levitation, honey
//! wall sliding and anything not reviewed in `blocks`.
// Wired into the finite-motion and continuous-control APIs in the next stages.
#![cfg_attr(not(test), allow(dead_code))]
pub(crate) mod blocks;
pub(crate) mod collision;
#[cfg(test)]
mod oracle_tests;

use crate::versions::table::{PhysicsGeneration, PhysicsRules};
use crate::{MinecraftVersion, NativeBlockState, Result};
use blocks::{Effect, unsupported};
use collision::Geometry;
use std::collections::HashMap;

/// Keys held during one tick, as the client's input sees them.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Controls {
    /// -1 backwards, 0 released, 1 forwards.
    pub forward: i8,
    /// -1 right, 0 released, 1 left.
    pub strafe: i8,
    pub jump: bool,
    pub sneak: bool,
    pub sprint: bool,
    /// Body yaw in degrees.
    pub yaw: f32,
    /// Pitch in degrees, -90..90.
    pub pitch: f32,
}

/// Attribute modifier operation, in native order of application.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ModifierOperation {
    Addition,
    MultiplyBase,
    MultiplyTotal,
}

/// One received attribute modifier. `id` is the version's modifier identity
/// (a UUID string in 1.16.1, a namespaced identifier in 1.21.11).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Modifier {
    pub id: String,
    pub operation: ModifierOperation,
    pub amount: f64,
}

/// Received player facts the movement depends on. Values are attribute values
/// as the server sent them; the engine adds only the client's own sprint modifier.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Environment {
    pub movement_speed_base: f64,
    /// Received movement-speed modifiers in arrival order (without sprinting).
    pub movement_speed_modifiers: Vec<Modifier>,
    pub jump_strength: f64,
    pub step_height: f64,
    pub gravity: f64,
    pub sneaking_speed: f64,
    pub movement_efficiency: f64,
    pub jump_boost: Option<u8>,
    pub slow_falling: bool,
    pub levitation: bool,
    pub blindness: bool,
    /// Weaving (1.21.11): weaker cobweb slowdown.
    pub weaving: bool,
    pub food_level: i32,
    pub may_fly: bool,
}

impl Environment {
    /// Native defaults of a fresh survival player.
    pub fn defaults(version: MinecraftVersion) -> Self {
        let c = &version.table().physics;
        Self {
            movement_speed_base: f64::from(c.base_movement_speed),
            movement_speed_modifiers: Vec::new(),
            jump_strength: f64::from(c.jump_velocity),
            step_height: f64::from(c.step_height),
            gravity: c.gravity,
            sneaking_speed: 0.3,
            movement_efficiency: 0.0,
            jump_boost: None,
            slow_falling: false,
            levitation: false,
            blindness: false,
            weaving: false,
            food_level: 20,
            may_fly: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Pose {
    Standing,
    Crouching,
}

/// The engine's player state between ticks.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Body {
    pub position: [f64; 3],
    /// Collision box (persisted separately where the version moves the box).
    pub bounds: [f64; 6],
    pub velocity: [f64; 3],
    pub on_ground: bool,
    pub horizontal_collision: bool,
    pub minor_horizontal_collision: bool,
    pub vertical_collision: bool,
    pub supporting_block: Option<[i32; 3]>,
    on_ground_no_blocks: bool,
    pub fall_distance: f64,
    stuck: [f64; 3],
    no_jump_delay: u8,
    pub sprinting: bool,
    pub crouching: bool,
    pub pose: Pose,
    keys: Controls,
    /// Modern: normalized key vector; legacy: input impulses.
    move_vector: [f32; 2],
    /// Legacy Player.flyingSpeed.
    flying_speed: f32,
    yaw: f32,
    pitch: f32,
}

const WIDTH: f32 = 0.6;
fn height(pose: Pose) -> f32 {
    match pose {
        Pose::Standing => 1.8,
        Pose::Crouching => 1.5,
    }
}

/// EntityDimensions.makeBoundingBox.
fn make_box(p: [f64; 3], pose: Pose) -> [f64; 6] {
    let half = f64::from(WIDTH / 2.0);
    [
        p[0] - half,
        p[1],
        p[2] - half,
        p[0] + half,
        p[1] + f64::from(height(pose)),
        p[2] + half,
    ]
}

fn deflate(b: [f64; 6], d: f64) -> [f64; 6] {
    [b[0] + d, b[1] + d, b[2] + d, b[3] - d, b[4] - d, b[5] - d]
}

fn shifted(b: [f64; 6], d: [f64; 3]) -> [f64; 6] {
    std::array::from_fn(|i| b[i] + d[i % 3])
}

fn floor(v: f64) -> i32 {
    v.floor() as i32
}

impl Body {
    /// A player standing still at `position` with the given received speed facts.
    pub fn new(position: [f64; 3]) -> Self {
        Self {
            position,
            bounds: make_box(position, Pose::Standing),
            velocity: [0.0; 3],
            on_ground: true,
            horizontal_collision: false,
            minor_horizontal_collision: false,
            vertical_collision: false,
            supporting_block: None,
            on_ground_no_blocks: false,
            fall_distance: 0.0,
            stuck: [0.0; 3],
            no_jump_delay: 0,
            sprinting: false,
            crouching: false,
            pose: Pose::Standing,
            keys: Controls::default(),
            move_vector: [0.0; 2],
            flying_speed: 0.02,
            yaw: 0.0,
            pitch: 0.0,
        }
    }
}

/// Cached block facts around the player for one tick.
struct Level<'a, F> {
    version: MinecraftVersion,
    block_at: &'a mut F,
    cache: HashMap<[i32; 3], (&'static blocks::State, &'static blocks::Block)>,
    states: HashMap<[i32; 3], NativeBlockState>,
}

impl<F: FnMut([i32; 3]) -> Result<NativeBlockState>> Level<'_, F> {
    fn get(&mut self, p: [i32; 3]) -> Result<(&'static blocks::State, &'static blocks::Block)> {
        if let Some(found) = self.cache.get(&p) {
            return Ok(*found);
        }
        let state = (self.block_at)(p)?;
        let found = blocks::lookup(self.version, &state)?;
        self.cache.insert(p, found);
        self.states.insert(p, state);
        Ok(found)
    }
    fn state(&mut self, p: [i32; 3]) -> Result<&NativeBlockState> {
        self.get(p)?;
        Ok(&self.states[&p])
    }
    fn block(&mut self, p: [i32; 3]) -> Result<&'static blocks::Block> {
        Ok(self.get(p)?.1)
    }
    fn is_air(&mut self, p: [i32; 3]) -> Result<bool> {
        Ok(matches!(
            self.block(p)?.name.as_str(),
            "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
        ))
    }
    /// Collision boxes of one cell in world coordinates; refuses unreviewed shapes.
    fn shape(&mut self, p: [i32; 3]) -> Result<Vec<[f64; 6]>> {
        let (state, block) = self.get(p)?;
        if let Effect::Unsupported(why) = block.shape {
            return Err(unsupported(format!("{why}: {}", block.name)));
        }
        if state.positional {
            return Err(unsupported(format!("positional shape: {}", block.name)));
        }
        let boxes = &blocks::table(self.version).shapes[usize::from(state.shape)];
        Ok(boxes
            .iter()
            .map(|b| std::array::from_fn(|i| b[i] + f64::from(p[i % 3])))
            .collect())
    }
    /// Shapes of every cell a box can touch (including neighbours whose shapes
    /// extend beyond their cell, such as fences).
    fn geometry(&mut self, region: [f64; 6]) -> Result<Geometry> {
        let mut geometry = Geometry::default();
        for x in floor(region[0] - 1e-7) - 1..=floor(region[3] + 1e-7) + 1 {
            for y in floor(region[1] - 1e-7) - 1..=floor(region[4] + 1e-7) + 1 {
                for z in floor(region[2] - 1e-7) - 1..=floor(region[5] + 1e-7) + 1 {
                    geometry.push(self.shape([x, y, z])?);
                }
            }
        }
        Ok(geometry)
    }
    /// Level.noCollision for blocks: no shape overlaps the box with positive volume.
    fn no_collision(&mut self, b: [f64; 6]) -> Result<bool> {
        Ok(!self.geometry(b)?.iter().any(|c| intersects(*c, b)))
    }
    fn suffocating(&mut self, p: [i32; 3]) -> Result<bool> {
        Ok(self.get(p)?.0.suffocating)
    }
}

fn intersects(a: [f64; 6], b: [f64; 6]) -> bool {
    (0..3).all(|i| a[i] < b[i + 3] && a[i + 3] > b[i])
}

/// Movement speed attribute, including the client's sprint modifier.
fn movement_speed(version: MinecraftVersion, env: &Environment, sprinting: bool) -> f64 {
    let sprint = sprinting.then(|| Modifier {
        id: version.table().physics_rules.sprint_modifier.into(),
        operation: ModifierOperation::MultiplyTotal,
        amount: f64::from(0.3f32),
    });
    let modifiers: Vec<&Modifier> = env
        .movement_speed_modifiers
        .iter()
        .chain(sprint.as_ref())
        .collect();
    let ordered = collision::attribute_order(version, &modifiers);
    let mut value = env.movement_speed_base;
    for m in ordered
        .iter()
        .filter(|m| m.operation == ModifierOperation::Addition)
    {
        value += m.amount;
    }
    let base = value;
    for m in ordered
        .iter()
        .filter(|m| m.operation == ModifierOperation::MultiplyBase)
    {
        value += base * m.amount;
    }
    for m in ordered
        .iter()
        .filter(|m| m.operation == ModifierOperation::MultiplyTotal)
    {
        value *= 1.0 + m.amount;
    }
    value.clamp(0.0, 1024.0)
}

fn trig(version: MinecraftVersion, degrees: f32, cosine: bool) -> f32 {
    crate::client::survival::model::trig(version, degrees * (std::f32::consts::PI / 180.0), cosine)
}

/// Advance the player by one native client tick. On error nothing is changed.
pub(crate) fn tick(
    version: MinecraftVersion,
    body: &mut Body,
    env: &Environment,
    controls: Controls,
    block_at: &mut impl FnMut([i32; 3]) -> Result<NativeBlockState>,
) -> Result<()> {
    if !controls.yaw.is_finite()
        || !(-90.0..=90.0).contains(&controls.pitch)
        || !(-1..=1).contains(&controls.forward)
        || !(-1..=1).contains(&controls.strafe)
    {
        return Err(crate::client::registry::invalid("controls out of range"));
    }
    if env.levitation {
        return Err(unsupported("levitation".into()));
    }
    let mut level = Level {
        version,
        block_at,
        cache: HashMap::new(),
        states: HashMap::new(),
    };
    let mut next = body.clone();
    Tick {
        version,
        rules: &version.table().physics_rules,
        env,
        body: &mut next,
        level: &mut level,
    }
    .run(controls)?;
    *body = next;
    Ok(())
}

struct Tick<'a, 'w, F> {
    version: MinecraftVersion,
    rules: &'static PhysicsRules,
    env: &'a Environment,
    body: &'a mut Body,
    level: &'a mut Level<'w, F>,
}

impl<F: FnMut([i32; 3]) -> Result<NativeBlockState>> Tick<'_, '_, F> {
    fn legacy(&self) -> bool {
        self.rules.generation == PhysicsGeneration::Legacy
    }

    fn run(&mut self, controls: Controls) -> Result<()> {
        self.body.yaw = controls.yaw;
        self.body.pitch = controls.pitch;
        self.check_scope()?;
        // LocalPlayer.aiStep (client input handling).
        let old_position = self.body.position;
        self.client_input(controls)?;
        // LivingEntity.aiStep.
        let b = &mut *self.body;
        b.no_jump_delay = b.no_jump_delay.saturating_sub(1);
        let c = &self.version.table().physics;
        let mut v = b.velocity;
        match self.rules.small_velocity {
            crate::versions::table::SmallVelocity::PerAxis => {
                for axis in [0, 2] {
                    if v[axis].abs() < c.small_velocity {
                        v[axis] = 0.0;
                    }
                }
            }
            crate::versions::table::SmallVelocity::HorizontalLength => {
                if v[0] * v[0] + v[2] * v[2] < 9e-6 {
                    v[0] = 0.0;
                    v[2] = 0.0;
                }
            }
        }
        if v[1].abs() < c.small_velocity {
            v[1] = 0.0;
        }
        b.velocity = v;
        let (mut xxa, mut zza) = if self.legacy() {
            (self.body.move_vector[0], self.body.move_vector[1])
        } else {
            self.modify_input()
        };
        let jumping = self.body.keys.jump;
        if jumping {
            if self.body.on_ground && self.body.no_jump_delay == 0 {
                self.jump_from_ground()?;
                self.body.no_jump_delay = 10;
            }
        } else {
            self.body.no_jump_delay = 0;
        }
        if self.legacy() {
            xxa *= 0.98;
            zza *= 0.98;
        }
        self.travel([f64::from(xxa), 0.0, f64::from(zza)])?;
        if !self.legacy() {
            self.apply_effects_from_blocks(old_position)?;
        }
        // Player.aiStep tail.
        if self.legacy() {
            self.body.flying_speed = if self.body.sprinting {
                (0.02f32 as f64 + 0.005999999865889549) as f32
            } else {
                0.02
            };
        }
        // Player.tick tail.
        self.update_pose()
    }

    /// Refuse fluids, climbing and blocks the engine does not reproduce.
    fn check_scope(&mut self) -> Result<()> {
        let b = self.body.bounds;
        for x in floor(b[0] - 0.01)..=floor(b[3] + 0.01) {
            for y in floor(b[1] - 0.01)..=floor(b[4] + 0.01) {
                for z in floor(b[2] - 0.01)..=floor(b[5] + 0.01) {
                    let (state, block) = self.level.get([x, y, z])?;
                    if state.fluid {
                        return Err(unsupported(format!("fluid: {}", block.name)));
                    }
                }
            }
        }
        let feet = self.block_position();
        let block = self.level.block(feet)?;
        if block.climbable || block.trapdoor && self.trapdoor_ladder(feet)? {
            return Err(unsupported(format!("climbing: {}", block.name)));
        }
        Ok(())
    }

    /// LivingEntity.trapdoorUsableAsLadder: open, above a ladder facing the same way.
    fn trapdoor_ladder(&mut self, at: [i32; 3]) -> Result<bool> {
        let trapdoor = self.level.state(at)?.clone();
        if trapdoor.properties.get("open").map(String::as_str) != Some("true") {
            return Ok(false);
        }
        let below = self.level.state([at[0], at[1] - 1, at[2]])?;
        Ok(below.name == "minecraft:ladder"
            && below.properties.get("facing") == trapdoor.properties.get("facing"))
    }

    fn block_position(&self) -> [i32; 3] {
        self.body.position.map(floor)
    }

    fn fits(&mut self, pose: Pose) -> Result<bool> {
        let b = if self.legacy() {
            // Entity.getBoundingBoxForPose: float half width.
            let half = WIDTH / 2.0;
            let p = self.body.position;
            [
                p[0] - f64::from(half),
                p[1],
                p[2] - f64::from(half),
                p[0] + f64::from(half),
                p[1] + f64::from(height(pose)),
                p[2] + f64::from(half),
            ]
        } else {
            make_box(self.body.position, pose)
        };
        self.level.no_collision(deflate(b, 1e-7))
    }

    fn client_input(&mut self, controls: Controls) -> Result<()> {
        let sneaking_before = self.body.keys.sneak;
        if self.legacy() {
            let had_impulse = self.enough_impulse_to_sprint();
            self.body.crouching =
                self.fits(Pose::Crouching)? && (sneaking_before || !self.fits(Pose::Standing)?);
            self.body.keys = controls;
            let slow = self.body.crouching;
            let mut forward = f32::from(controls.forward);
            let mut left = f32::from(controls.strafe);
            if slow {
                left = (f64::from(left) * 0.3) as f32;
                forward = (f64::from(forward) * 0.3) as f32;
            }
            self.body.move_vector = [left, forward];
            self.push_out_legacy()?;
            let food = self.env.food_level > 6 || self.env.may_fly;
            let b = &*self.body;
            let possible = food && !self.env.blindness && controls.sprint;
            if b.on_ground
                && !sneaking_before
                && !had_impulse
                && self.enough_impulse_to_sprint()
                && !b.sprinting
                && possible
            {
                self.body.sprinting = true;
            }
            if !self.body.sprinting && self.enough_impulse_to_sprint() && possible {
                self.body.sprinting = true;
            }
            if self.body.sprinting {
                let no_impulse = self.body.move_vector[1] <= 1.0e-5 || !food;
                if no_impulse || self.body.horizontal_collision {
                    self.body.sprinting = false;
                }
            }
        } else {
            self.body.crouching =
                self.fits(Pose::Crouching)? && (sneaking_before || !self.fits(Pose::Standing)?);
            self.body.keys = controls;
            let (x, y) = (f32::from(controls.strafe), f32::from(controls.forward));
            let length = (x * x + y * y).sqrt();
            self.body.move_vector = if length < 1.0e-4 {
                [0.0, 0.0]
            } else {
                [x / length, y / length]
            };
            self.push_out_modern()?;
            let possible = !self.env.blindness && (self.env.food_level > 6 || self.env.may_fly);
            let forward = self.body.move_vector[1] > 1.0e-5;
            if !self.body.sprinting
                && forward
                && possible
                && !self.body.crouching
                && controls.sprint
            {
                self.body.sprinting = true;
            }
            if self.body.sprinting
                && (!possible
                    || !forward
                    || self.body.horizontal_collision && !self.body.minor_horizontal_collision)
            {
                self.body.sprinting = false;
            }
        }
        Ok(())
    }

    fn enough_impulse_to_sprint(&self) -> bool {
        self.body.move_vector[1] >= 0.8
    }

    /// LocalPlayer.modifyInput: scale, sneak factor, then square-normalize.
    fn modify_input(&self) -> (f32, f32) {
        let [x, y] = self.body.move_vector;
        if x * x + y * y == 0.0 {
            return (x, y);
        }
        let (mut x, mut y) = (x * 0.98, y * 0.98);
        if self.body.crouching {
            let s = self.env.sneaking_speed as f32;
            x *= s;
            y *= s;
        }
        let length = (x * x + y * y).sqrt();
        if length <= 0.0 {
            return (x, y);
        }
        let (ux, uy) = (x * (1.0 / length), y * (1.0 / length));
        let (ax, ay) = (ux.abs(), uy.abs());
        let ratio = if ay > ax { ax / ay } else { ay / ax };
        let scale = (length * (1.0 + ratio * ratio).sqrt()).min(1.0);
        (ux * scale, uy * scale)
    }

    // LocalPlayer.moveTowardsClosestSpace.
    fn push_out_modern(&mut self) -> Result<()> {
        let w = f64::from(WIDTH) * 0.35;
        let p = self.body.position;
        for (x, z) in [
            (p[0] - w, p[2] + w),
            (p[0] - w, p[2] - w),
            (p[0] + w, p[2] - w),
            (p[0] + w, p[2] + w),
        ] {
            let cell = [floor(x), floor(self.body.position[1]), floor(z)];
            if !self.suffocates_modern(cell)? {
                continue;
            }
            let (dx, dz) = (x - f64::from(cell[0]), z - f64::from(cell[2]));
            let mut best: Option<(usize, f64)> = None;
            let mut nearest = f64::MAX;
            for (axis, sign) in [(0, -1.0), (0, 1.0), (2, -1.0), (2, 1.0)] {
                let along = if axis == 0 { dx } else { dz };
                let distance = if sign > 0.0 { 1.0 - along } else { along };
                let mut neighbour = cell;
                neighbour[axis] += sign as i32;
                if distance < nearest && !self.suffocates_modern(neighbour)? {
                    nearest = distance;
                    best = Some((axis, sign));
                }
            }
            if let Some((axis, sign)) = best {
                self.body.velocity[axis] = 0.1 * sign;
            }
        }
        Ok(())
    }

    fn suffocates_modern(&mut self, cell: [i32; 3]) -> Result<bool> {
        let b = self.body.bounds;
        let column = deflate(
            [
                f64::from(cell[0]),
                b[1],
                f64::from(cell[2]),
                f64::from(cell[0]) + 1.0,
                b[4],
                f64::from(cell[2]) + 1.0,
            ],
            1e-7,
        );
        // CollisionGetter.collidesWithSuffocatingBlock.
        for x in floor(column[0] - 1e-7) - 1..=floor(column[3] + 1e-7) + 1 {
            for y in floor(column[1] - 1e-7) - 1..=floor(column[4] + 1e-7) + 1 {
                for z in floor(column[2] - 1e-7) - 1..=floor(column[5] + 1e-7) + 1 {
                    if self.level.suffocating([x, y, z])?
                        && self
                            .level
                            .shape([x, y, z])?
                            .iter()
                            .any(|s| intersects(*s, column))
                    {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }

    // LocalPlayer.checkInBlock.
    fn push_out_legacy(&mut self) -> Result<()> {
        let w = f64::from(WIDTH) * 0.35;
        let p = self.body.position;
        let y = p[1] + 0.5;
        for (x, z) in [
            (p[0] - w, p[2] + w),
            (p[0] - w, p[2] - w),
            (p[0] + w, p[2] - w),
            (p[0] + w, p[2] + w),
        ] {
            let cell = [floor(x), floor(y), floor(z)];
            if !self.blocked_legacy(cell)? {
                continue;
            }
            let (dx, dz) = (x - f64::from(cell[0]), z - f64::from(cell[2]));
            let mut best = None;
            let mut nearest = 9999.0;
            for (axis, sign, distance) in
                [(0, -1, dx), (0, 1, 1.0 - dx), (2, -1, dz), (2, 1, 1.0 - dz)]
            {
                let mut neighbour = cell;
                neighbour[axis] += sign;
                if !self.blocked_legacy(neighbour)? && distance < nearest {
                    nearest = distance;
                    best = Some((axis, sign));
                }
            }
            if let Some((axis, sign)) = best {
                self.body.velocity[axis] = 0.1 * f64::from(sign);
            }
        }
        Ok(())
    }

    fn blocked_legacy(&mut self, cell: [i32; 3]) -> Result<bool> {
        let b = self.body.bounds;
        for y in floor(b[1])..b[4].ceil() as i32 {
            if self.level.suffocating([cell[0], y, cell[2]])? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn jump_factor_position(&mut self) -> [i32; 3] {
        self.below_affecting_movement()
    }

    fn jump_from_ground(&mut self) -> Result<()> {
        let feet = self.block_position();
        let below = self.jump_factor_position();
        let feet_factor = self.level.block(feet)?.jump_factor;
        let below_factor = self.level.block(below)?.jump_factor;
        let factor = if feet_factor == 1.0 {
            below_factor
        } else {
            feet_factor
        };
        let boost = self
            .env
            .jump_boost
            .map_or(0.0, |a| 0.1 * (f32::from(a) + 1.0));
        let rad = self.body.yaw * (std::f32::consts::PI / 180.0);
        let (s, c) = (
            crate::client::survival::model::trig(self.version, rad, false),
            crate::client::survival::model::trig(self.version, rad, true),
        );
        if self.legacy() {
            let power = 0.42f32 * factor + boost;
            self.body.velocity[1] = f64::from(power);
            if self.body.sprinting {
                self.body.velocity[0] += f64::from(-s * 0.2f32);
                self.body.velocity[2] += f64::from(c * 0.2f32);
            }
        } else {
            let power = self.env.jump_strength as f32 * 1.0 * factor + boost;
            if power <= 1.0e-5 {
                return Ok(());
            }
            self.body.velocity[1] = f64::from(power).max(self.body.velocity[1]);
            if self.body.sprinting {
                self.body.velocity[0] += f64::from(-s) * 0.2;
                self.body.velocity[2] += f64::from(c) * 0.2;
            }
        }
        Ok(())
    }

    /// Block whose friction/speed/jump factor applies below the player.
    fn below_affecting_movement(&mut self) -> [i32; 3] {
        if self.legacy() {
            let p = self.body.position;
            [
                floor(p[0]),
                floor(self.body.bounds[1] - 0.5000001),
                floor(p[2]),
            ]
        } else {
            self.on_pos(0.500001f32)
                .unwrap_or_else(|_| self.block_position())
        }
    }

    /// Modern Entity.getOnPos(offset).
    fn on_pos(&mut self, offset: f32) -> Result<[i32; 3]> {
        let p = self.body.position;
        let below = floor(p[1] - f64::from(offset));
        let Some(main) = self.body.supporting_block else {
            return Ok([floor(p[0]), below, floor(p[2])]);
        };
        if offset <= 1.0e-5 {
            return Ok(main);
        }
        let block = self.level.block(main)?;
        if (offset > 0.5 || !block.fences) && !block.walls && !block.fence_gate {
            Ok([main[0], below, main[2]])
        } else {
            Ok(main)
        }
    }

    /// Block under the feet for falls and stepping.
    fn landing_position(&mut self) -> Result<[i32; 3]> {
        if self.legacy() {
            let p = self.body.position;
            let pos = [floor(p[0]), floor(p[1] - f64::from(0.2f32)), floor(p[2])];
            if self.level.is_air(pos)? {
                let below = [pos[0], pos[1] - 1, pos[2]];
                let block = self.level.block(below)?;
                if block.fences || block.walls || block.fence_gate {
                    return Ok(below);
                }
            }
            Ok(pos)
        } else {
            self.on_pos(0.2)
        }
    }

    fn travel(&mut self, input: [f64; 3]) -> Result<()> {
        let below = self.below_affecting_movement();
        let block_friction = self.level.block(below)?.friction;
        let legacy = self.legacy();
        let friction = if legacy || self.body.on_ground {
            block_friction
        } else {
            1.0
        };
        let drag = if legacy {
            if self.body.on_ground {
                friction * 0.91
            } else {
                0.91
            }
        } else {
            friction * 0.91
        };
        // Legacy chooses gravity before moving.
        let mut gravity = self.env.gravity;
        if legacy && self.body.velocity[1] <= 0.0 && self.env.slow_falling {
            gravity = 0.01;
            self.body.fall_distance = 0.0;
        }
        let speed = if self.body.on_ground {
            // Player.getSpeed reads the attribute live.
            movement_speed(self.version, self.env, self.body.sprinting) as f32
                * (0.21600002f32 / (friction * friction * friction))
        } else if legacy {
            self.body.flying_speed
        } else if self.body.sprinting {
            0.025999999
        } else {
            0.02
        };
        let a = self.input_vector(input, speed);
        for (axis, value) in a.into_iter().enumerate() {
            self.body.velocity[axis] += value;
        }
        let motion = self.body.velocity;
        self.do_move(motion)?;
        let v = self.body.velocity;
        let mut y = v[1];
        if !legacy && v[1] <= 0.0 && self.env.slow_falling {
            gravity = gravity.min(0.01);
        }
        y -= gravity;
        self.body.velocity = [
            v[0] * f64::from(drag),
            y * f64::from(0.98f32),
            v[2] * f64::from(drag),
        ];
        Ok(())
    }

    /// Entity.getInputVector.
    fn input_vector(&self, input: [f64; 3], speed: f32) -> [f64; 3] {
        let length = input.iter().map(|v| v * v).sum::<f64>();
        if length < 1.0e-7 {
            return [0.0; 3];
        }
        let v = if length > 1.0 {
            let l = if self.rules.f32_input_length {
                f64::from(length.sqrt() as f32)
            } else {
                length.sqrt()
            };
            input.map(|c| c / l)
        } else {
            input
        };
        let v = v.map(|c| c * f64::from(speed));
        let (s, c) = (
            f64::from(trig(self.version, self.body.yaw, false)),
            f64::from(trig(self.version, self.body.yaw, true)),
        );
        [v[0] * c - v[2] * s, v[1], v[2] * c + v[0] * s]
    }

    fn step_height(&self) -> f32 {
        self.env.step_height as f32
    }

    fn do_move(&mut self, mut motion: [f64; 3]) -> Result<()> {
        let legacy = self.legacy();
        let stuck = self.body.stuck;
        if stuck.iter().map(|v| v * v).sum::<f64>() > 1.0e-7 {
            motion = std::array::from_fn(|i| motion[i] * stuck[i]);
            self.body.stuck = [0.0; 3];
            self.body.velocity = [0.0; 3];
        }
        motion = self.back_off_from_edge(motion)?;
        let region = {
            let b = self.body.bounds;
            let step = f64::from(self.step_height());
            [
                b[0] + motion[0].min(0.0),
                b[1] + motion[1].min(0.0) - step,
                b[2] + motion[2].min(0.0),
                b[3] + motion[0].max(0.0),
                b[4] + motion[1].max(0.0) + step,
                b[5] + motion[2].max(0.0),
            ]
        };
        let geometry = self.level.geometry(region)?;
        let collided = collision::collide(
            self.version,
            self.body.bounds,
            motion,
            &geometry,
            self.body.on_ground,
            self.step_height(),
        );
        let moved2 = collided.iter().map(|v| v * v).sum::<f64>();
        let motion2 = motion.iter().map(|v| v * v).sum::<f64>();
        let apply = moved2 > 1.0e-7 || (!legacy && motion2 - moved2 < 1.0e-7);
        if apply {
            if !legacy && self.body.fall_distance != 0.0 && moved2 >= 1.0 {
                return Err(unsupported("fall-distance reset sweep".into()));
            }
            if self.rules.position_from_bounds {
                self.body.bounds = shifted(self.body.bounds, collided);
                let b = self.body.bounds;
                self.body.position = [(b[0] + b[3]) / 2.0, b[1], (b[2] + b[5]) / 2.0];
            } else {
                self.body.position = std::array::from_fn(|i| self.body.position[i] + collided[i]);
                self.body.bounds = make_box(self.body.position, self.body.pose);
            }
        }
        let equal = |a: f64, b: f64| (b - a).abs() < f64::from(1.0e-5f32);
        let x_hit = !equal(motion[0], collided[0]);
        let z_hit = !equal(motion[2], collided[2]);
        self.body.horizontal_collision = x_hit || z_hit;
        self.body.vertical_collision = motion[1] != collided[1];
        self.body.on_ground = self.body.vertical_collision && motion[1] < 0.0;
        if !legacy {
            self.check_supporting_block(collided)?;
            self.body.minor_horizontal_collision =
                self.body.horizontal_collision && self.horizontal_collision_minor(collided);
        }
        let landing = self.landing_position()?;
        let landing_block = self.level.block(landing)?;
        // checkFallDamage: only the fall distance matters to movement.
        if legacy {
            if self.body.on_ground {
                self.body.fall_distance = 0.0;
            } else if collided[1] < 0.0 {
                self.body.fall_distance =
                    f64::from((self.body.fall_distance as f32 as f64 - collided[1]) as f32);
            }
        } else {
            if collided[1] < 0.0 {
                self.body.fall_distance -= collided[1];
            }
            if self.body.on_ground {
                self.body.fall_distance = 0.0;
            }
        }
        let v = self.body.velocity;
        if legacy {
            if motion[0] != collided[0] {
                self.body.velocity[0] = 0.0;
            }
            if motion[2] != collided[2] {
                self.body.velocity[2] = 0.0;
            }
        } else if self.body.horizontal_collision {
            self.body.velocity = [
                if x_hit { 0.0 } else { v[0] },
                v[1],
                if z_hit { 0.0 } else { v[2] },
            ];
        }
        if motion[1] != collided[1] {
            self.after_fall_on(landing_block)?;
        }
        if legacy {
            if self.body.on_ground && !self.body.keys.sneak {
                self.step_on(landing_block)?;
            }
            self.inside_blocks_legacy()?;
        }
        let factor = self.speed_factor()?;
        self.body.velocity[0] *= f64::from(factor);
        self.body.velocity[2] *= f64::from(factor);
        Ok(())
    }

    fn after_fall_on(&mut self, block: &blocks::Block) -> Result<()> {
        let v = &mut self.body.velocity;
        match block.after_fall_on {
            Effect::None => v[1] = 0.0,
            Effect::Slime | Effect::Bed if self.body.keys.sneak => v[1] = 0.0,
            Effect::Slime => {
                if v[1] < 0.0 {
                    v[1] = -v[1] * 1.0;
                }
            }
            Effect::Bed => {
                if v[1] < 0.0 {
                    v[1] = -v[1] * f64::from(0.66f32) * 1.0;
                }
            }
            other => {
                return Err(unsupported(format!("landing on {}: {other:?}", block.name)));
            }
        }
        Ok(())
    }

    fn step_on(&mut self, block: &blocks::Block) -> Result<()> {
        match block.step_on {
            Effect::None => Ok(()),
            Effect::Slime => {
                let y = self.body.velocity[1].abs();
                if y < 0.1 && !self.body.keys.sneak {
                    let f = 0.4 + y * 0.2;
                    self.body.velocity[0] *= f;
                    self.body.velocity[2] *= f;
                }
                Ok(())
            }
            other => Err(unsupported(format!(
                "stepping on {}: {other:?}",
                block.name
            ))),
        }
    }

    fn speed_factor(&mut self) -> Result<f32> {
        let feet = self.level.block(self.block_position())?;
        let own = feet.speed_factor;
        let below = self.below_affecting_movement();
        let base = if own == 1.0 {
            self.level.block(below)?.speed_factor
        } else {
            own
        };
        if self.legacy() {
            Ok(base)
        } else {
            let t = self.env.movement_efficiency as f32;
            Ok(base + t * (1.0 - base))
        }
    }

    fn inside_effect(&mut self, cell: [i32; 3], certain: bool) -> Result<()> {
        let block = self.level.block(cell)?;
        match block.inside {
            Effect::None => Ok(()),
            Effect::Stuck if !certain => {
                Err(unsupported(format!("possible contact with {}", block.name)))
            }
            Effect::Stuck => {
                let stuck = if block.name == "minecraft:cobweb" {
                    if self.env.weaving {
                        [0.5, 0.25, 0.5]
                    } else {
                        [0.25, f64::from(0.05f32), 0.25]
                    }
                } else {
                    [f64::from(0.8f32), 0.75, f64::from(0.8f32)]
                };
                self.body.fall_distance = 0.0;
                self.body.stuck = stuck;
                Ok(())
            }
            Effect::Honey => {
                let p = self.body.position;
                let half = 0.4375 + f64::from(WIDTH / 2.0);
                let sliding = !self.body.on_ground
                    && p[1] <= f64::from(cell[1]) + 0.9375 - 1.0e-7
                    && ((f64::from(cell[0]) + 0.5 - p[0]).abs() + 1.0e-7 > half
                        || (f64::from(cell[2]) + 0.5 - p[2]).abs() + 1.0e-7 > half);
                if sliding {
                    Err(unsupported("honey wall sliding".into()))
                } else {
                    Ok(())
                }
            }
            other => Err(unsupported(format!("inside {}: {other:?}", block.name))),
        }
    }

    fn inside_blocks_legacy(&mut self) -> Result<()> {
        let b = self.body.bounds;
        let lo = [
            floor(b[0] + 0.001),
            floor(b[1] + 0.001),
            floor(b[2] + 0.001),
        ];
        let hi = [
            floor(b[3] - 0.001),
            floor(b[4] - 0.001),
            floor(b[5] - 0.001),
        ];
        let mut seen_stuck: Option<&str> = None;
        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    let name = self.level.block([x, y, z])?.name.as_str();
                    if self.level.block([x, y, z])?.inside == Effect::Stuck {
                        if seen_stuck.is_some_and(|s| s != name) {
                            return Err(unsupported("mixed cobweb and berry bush".into()));
                        }
                        seen_stuck = Some(name);
                    }
                    self.inside_effect([x, y, z], true)?;
                }
            }
        }
        Ok(())
    }

    /// Modern Entity.applyEffectsFromBlocks for the single self-movement of a tick.
    fn apply_effects_from_blocks(&mut self, old_position: [f64; 3]) -> Result<()> {
        if self.body.on_ground {
            let landing = self.landing_position()?;
            let block = self.level.block(landing)?;
            self.step_on(block)?;
        }
        // Cells the swept box certainly visits (start and end boxes of each axis
        // step) versus cells it might visit (the hull between them).
        let from = old_position;
        let to = self.body.position;
        let delta: [f64; 3] = std::array::from_fn(|i| to[i] - from[i]);
        let order = if delta[0].abs() < delta[2].abs() {
            [1, 2, 0]
        } else {
            [1, 0, 2]
        };
        let pose = self.body.pose;
        let inside_box = |p: [f64; 3]| deflate(make_box(p, pose), f64::from(1.0e-5f32));
        let mut certain = vec![inside_box(from)];
        let mut possible = Vec::new();
        let mut at = from;
        for axis in order {
            if delta[axis] != 0.0 {
                let start = inside_box(at);
                at[axis] += delta[axis];
                let end = inside_box(at);
                certain.push(end);
                possible.push(std::array::from_fn(|i| {
                    if i < 3 {
                        start[i].min(end[i])
                    } else {
                        start[i].max(end[i])
                    }
                }));
            }
        }
        let cells = |b: &[f64; 6]| {
            let lo = [floor(b[0]), floor(b[1]), floor(b[2])];
            let hi = [floor(b[3]), floor(b[4]), floor(b[5])];
            (lo[0]..=hi[0]).flat_map(move |x| {
                (lo[1]..=hi[1]).flat_map(move |y| (lo[2]..=hi[2]).map(move |z| [x, y, z]))
            })
        };
        let mut sure = std::collections::BTreeSet::new();
        for b in &certain {
            sure.extend(cells(b));
        }
        let mut maybe = std::collections::BTreeSet::new();
        for b in &possible {
            maybe.extend(cells(b).filter(|c| !sure.contains(c)));
        }
        let mut stuck_kinds = std::collections::BTreeSet::new();
        for cell in sure.iter().chain(maybe.iter()) {
            if self.level.block(*cell)?.inside == Effect::Stuck {
                stuck_kinds.insert(self.level.block(*cell)?.name.as_str());
            }
        }
        if stuck_kinds.len() > 1 {
            return Err(unsupported("mixed cobweb and berry bush".into()));
        }
        for cell in sure {
            self.inside_effect(cell, true)?;
        }
        for cell in maybe {
            self.inside_effect(cell, false)?;
        }
        Ok(())
    }

    /// Modern Entity.checkSupportingBlock.
    fn check_supporting_block(&mut self, movement: [f64; 3]) -> Result<()> {
        if !self.body.on_ground {
            self.body.on_ground_no_blocks = false;
            self.body.supporting_block = None;
            return Ok(());
        }
        let b = self.body.bounds;
        let slab = [b[0], b[1] - 1.0e-6, b[2], b[3], b[1], b[5]];
        let mut found = self.find_supporting_block(slab)?;
        if found.is_some() || self.body.on_ground_no_blocks {
            self.body.supporting_block = found;
        } else {
            found = self.find_supporting_block(shifted(slab, [-movement[0], 0.0, -movement[2]]))?;
            self.body.supporting_block = found;
        }
        self.body.on_ground_no_blocks = found.is_none();
        Ok(())
    }

    fn find_supporting_block(&mut self, region: [f64; 6]) -> Result<Option<[i32; 3]>> {
        let p = self.body.position;
        let mut best: Option<([i32; 3], f64)> = None;
        for x in floor(region[0] - 1e-7) - 1..=floor(region[3] + 1e-7) + 1 {
            for y in floor(region[1] - 1e-7) - 1..=floor(region[4] + 1e-7) + 1 {
                for z in floor(region[2] - 1e-7) - 1..=floor(region[5] + 1e-7) + 1 {
                    if !self
                        .level
                        .shape([x, y, z])?
                        .iter()
                        .any(|s| intersects(*s, region))
                    {
                        continue;
                    }
                    let d = (f64::from(x) + 0.5 - p[0]).powi(2)
                        + (f64::from(y) + 0.5 - p[1]).powi(2)
                        + (f64::from(z) + 0.5 - p[2]).powi(2);
                    let later = |a: [i32; 3], b: [i32; 3]| (a[1], a[2], a[0]) < (b[1], b[2], b[0]);
                    if best.is_none_or(|(pos, bd)| d < bd || d == bd && later(pos, [x, y, z])) {
                        best = Some(([x, y, z], d));
                    }
                }
            }
        }
        Ok(best.map(|(pos, _)| pos))
    }

    /// LocalPlayer.isHorizontalCollisionMinor.
    fn horizontal_collision_minor(&self, movement: [f64; 3]) -> bool {
        let (xxa, zza) = self.modify_input();
        let (xxa, zza) = (f64::from(xxa), f64::from(zza));
        let (s, c) = (
            f64::from(trig(self.version, self.body.yaw, false)),
            f64::from(trig(self.version, self.body.yaw, true)),
        );
        let (x, z) = (xxa * c - zza * s, zza * c + xxa * s);
        let input = x * x + z * z;
        let moved = movement[0] * movement[0] + movement[2] * movement[2];
        if input < f64::from(1.0e-5f32) || moved < f64::from(1.0e-5f32) {
            return false;
        }
        ((x * movement[0] + z * movement[2]) / (input * moved).sqrt()).acos()
            < f64::from(0.13962634f32)
    }

    fn back_off_from_edge(&mut self, motion: [f64; 3]) -> Result<[f64; 3]> {
        if !self.body.keys.sneak {
            return Ok(motion);
        }
        let step = f64::from(self.step_height());
        if self.legacy() {
            if !self.body.on_ground {
                return Ok(motion);
            }
            let b = self.body.bounds;
            let free = |level: &mut Level<'_, F>, dx: f64, dz: f64| {
                level.no_collision(shifted(b, [dx, -step, dz]))
            };
            let toward = |v: f64| {
                if (-0.05..0.05).contains(&v) {
                    0.0
                } else if v > 0.0 {
                    v - 0.05
                } else {
                    v + 0.05
                }
            };
            let (mut x, mut z) = (motion[0], motion[2]);
            while x != 0.0 && free(self.level, x, 0.0)? {
                x = toward(x);
            }
            while z != 0.0 && free(self.level, 0.0, z)? {
                z = toward(z);
            }
            while x != 0.0 && z != 0.0 && free(self.level, x, z)? {
                x = toward(x);
                z = toward(z);
            }
            return Ok([x, motion[1], z]);
        }
        if motion[1] > 0.0 {
            return Ok(motion);
        }
        let fall = self.body.fall_distance;
        let above_ground =
            self.body.on_ground || fall < step && !self.can_fall(0.0, 0.0, step - fall)?;
        if !above_ground {
            return Ok(motion);
        }
        let (mut x, mut z) = (motion[0], motion[2]);
        let (sx, sz) = (x.signum() * 0.05, z.signum() * 0.05);
        while x != 0.0 && self.can_fall(x, 0.0, step)? {
            if x.abs() <= 0.05 {
                x = 0.0;
                break;
            }
            x -= sx;
        }
        while z != 0.0 && self.can_fall(0.0, z, step)? {
            if z.abs() <= 0.05 {
                z = 0.0;
                break;
            }
            z -= sz;
        }
        while x != 0.0 && z != 0.0 && self.can_fall(x, z, step)? {
            if x.abs() <= 0.05 {
                x = 0.0
            } else {
                x -= sx
            }
            if z.abs() <= 0.05 { z = 0.0 } else { z -= sz }
        }
        Ok([x, motion[1], z])
    }

    fn can_fall(&mut self, dx: f64, dz: f64, depth: f64) -> Result<bool> {
        let b = self.body.bounds;
        self.level.no_collision([
            b[0] + 1.0e-7 + dx,
            b[1] - depth - 1.0e-7,
            b[2] + 1.0e-7 + dz,
            b[3] - 1.0e-7 + dx,
            b[1],
            b[5] - 1.0e-7 + dz,
        ])
    }

    /// Player.updatePlayerPose and Entity.refreshDimensions.
    fn update_pose(&mut self) -> Result<()> {
        // Swimming pose is only reachable through fluids or tight spaces.
        if !self.fits_swimming()? {
            return Ok(());
        }
        let desired = if self.body.keys.sneak {
            Pose::Crouching
        } else {
            Pose::Standing
        };
        let pose = if self.fits(desired)? {
            desired
        } else if self.fits(Pose::Crouching)? {
            Pose::Crouching
        } else {
            return Err(unsupported("crawling pose".into()));
        };
        if pose != self.body.pose {
            self.body.pose = pose;
            if self.legacy() {
                let b = self.body.bounds;
                self.body.bounds = [
                    b[0],
                    b[1],
                    b[2],
                    b[0] + f64::from(WIDTH),
                    b[1] + f64::from(height(pose)),
                    b[2] + f64::from(WIDTH),
                ];
            } else {
                self.body.bounds = make_box(self.body.position, pose);
            }
        }
        Ok(())
    }

    fn fits_swimming(&mut self) -> Result<bool> {
        let p = self.body.position;
        let half = f64::from(WIDTH / 2.0);
        let b = [
            p[0] - half,
            p[1],
            p[2] - half,
            p[0] + half,
            p[1] + f64::from(0.6f32),
            p[2] + half,
        ];
        self.level.no_collision(deflate(b, 1e-7))
    }
}
