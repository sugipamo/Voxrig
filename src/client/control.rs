//! Continuous control (P7): held keys and per-tick client physics, shared by
//! both adapters. Contract: docs/common-control.md.
//!
//! The session runs the shared engine (`client::physics`) once per 50 ms tick
//! on the client's clock and sends what the client sends. A position packet is a
//! submission, never server acceptance; received corrections and velocity are
//! applied as the vanilla client applies them and are counted separately.
use crate::client::physics::{self, Body, Environment};
use crate::{MinecraftVersion, NativeBlockState, Result};

pub use crate::client::physics::{Controls, ItemUse, Modifier, ModifierOperation, Pose};

/// Session state.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ControlStatus {
    /// Ticking and sending.
    Running,
    /// The last tick could not be predicted; nothing is sent while paused and
    /// each tick retries from the same state.
    Paused {
        /// Why the engine could not predict the tick.
        reason: String,
    },
    /// Ended; a new session must be started explicitly.
    Stopped {
        /// Why the session ended.
        reason: String,
    },
}

/// Converted equipment inputs sampled for this local movement model.
/// These are diagnostic model parameters, not original receipts or action permissions.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct MovementEquipment {
    /// Legacy Depth Strider level; the water calculation caps its effect at three.
    pub depth_strider: u8,
    /// Legacy Soul Speed target block names from the received tag; empty stays empty.
    pub soul_speed_blocks: Option<Vec<String>>,
    /// Modern effective received water-movement-efficiency attribute.
    pub water_movement_efficiency: f64,
    /// Modern effective received ground-movement-efficiency attribute.
    pub movement_efficiency: f64,
    /// False when a supplied equipment value/tag could not be interpreted.
    pub available: bool,
}
impl From<&Environment> for MovementEquipment {
    fn from(env: &Environment) -> Self {
        Self {
            depth_strider: env.depth_strider,
            soul_speed_blocks: env.legacy_soul_speed_blocks.clone(),
            water_movement_efficiency: env.water_movement_efficiency,
            movement_efficiency: env.movement_efficiency,
            available: !env.equipment_unavailable,
        }
    }
}

/// Result of evaluating a single ground request in the native client model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GroundJumpOutcome {
    /// A ground impulse was predicted; this is not server acceptance.
    Applied,
    /// The model's ground flag was false, including an unsettled initial model.
    Airborne,
    /// Deep water or lava requires held swimming input instead.
    InFluid,
    /// Native jump power was too small to apply an impulse (modern attribute).
    NoJumpPower,
    /// The caller already holds jump; the request adds no second impulse.
    AlreadyHeld,
}

/// Local queue, model evaluation and dispatch are separate boundaries.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum GroundJumpStatus {
    /// Owned by the connection, to be consumed at the next physics tick.
    Queued,
    /// Consumed once, including when the model could not jump.
    Evaluated {
        /// Control tick that evaluated it.
        tick: u64,
        /// Client-model outcome, never proof of a server jump.
        outcome: GroundJumpOutcome,
        /// That tick's movement was completely written.
        dispatched: bool,
    },
    /// No predictable next tick, or the session ended before evaluation.
    Cancelled {
        /// Why the queued request was discarded without a later retry.
        reason: String,
    },
}

/// Latest once-only ground request. IDs are local to this connection/session.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct GroundJumpRequest {
    /// Session selected by the caller, fencing stale requests after replacement.
    pub session_id: u64,
    /// Monotonic request ID within that session; pending requests coalesce.
    pub request_id: u64,
    /// Queue, model and packet boundaries.
    pub status: GroundJumpStatus,
}

/// Engine state after one tick: a prediction, not a received position.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ControlFrame {
    /// Session tick that produced this frame.
    pub tick: u64,
    /// Predicted feet position.
    pub position: [f64; 3],
    /// Velocity for the next tick.
    pub velocity: [f64; 3],
    /// Predicted downward collision.
    pub on_ground: bool,
    /// Predicted X/Z obstruction.
    pub horizontal_collision: bool,
    /// Predicted pose (its box height).
    pub pose: Pose,
    /// Client sprint state (sent as a player command when it changes).
    pub sprinting: bool,
    /// Client crouching state.
    pub crouching: bool,
    /// Touching water.
    pub in_water: bool,
    /// Swimming (sprinting under water).
    pub swimming: bool,
    /// Eye position in water (`Entity.isEyeInFluid(WATER)` for the predicted pose).
    pub eye_in_water: bool,
    /// Item-use slowdown applied this tick (from the received item-use flags).
    pub using_item: Option<ItemUse>,
}

/// Observable record of the connection's control session.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ControlRecord {
    /// Latest converted equipment parameters sampled by this local model.
    /// Retained after stopping; never current equipment or a server acknowledgement.
    pub movement_equipment: MovementEquipment,
    /// Process-local session identity.
    pub session_id: u64,
    /// Running, paused or stopped.
    pub status: ControlStatus,
    /// Keys currently held; they apply from the next tick.
    pub controls: Controls,
    /// Ticks whose packets were completely written.
    pub dispatched_ticks: u64,
    /// Latest predicted frame (`None` before the first tick).
    pub frame: Option<ControlFrame>,
    /// Received position corrections applied to the session.
    pub corrections: u64,
    /// Received own-velocity updates applied to the session.
    pub velocity_updates: u64,
    /// Latest ground request, retained until another request or session replaces it.
    pub ground_jump: Option<GroundJumpRequest>,
}

/// One received attribute: base value and modifiers in arrival order.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ReceivedAttribute {
    /// Received base value.
    pub base: f64,
    /// Received modifiers in arrival order.
    pub modifiers: Vec<Modifier>,
}

/// Received facts sampled under the adapter's state lock before a tick.
pub(crate) struct Received {
    pub environment: Environment,
    /// Latest received pose (receive sequence, position, optional velocity).
    pub pose: Option<(u64, [f64; 3], Option<[f64; 3]>)>,
    /// Latest received own velocity (receive sequence, value).
    pub velocity: Option<(u64, [f64; 3])>,
    /// Latest received item-use flags and the effects of the item in use.
    pub using_item: Option<crate::client::item_use::ReceivedUse>,
}

/// Packets to write for one tick, in order.
#[derive(Debug, PartialEq)]
pub(crate) struct Output {
    /// Sneak changed (legacy entity action; modern carries it in the input).
    pub sneak: Option<bool>,
    /// Sprint changed (player command).
    pub sprint: Option<bool>,
    /// Modern input packet bits, when they changed.
    pub input: Option<u8>,
    pub position: [f64; 3],
    pub rotation: [f32; 2],
    pub on_ground: bool,
    pub horizontal_collision: bool,
}

pub(crate) struct ControlSession {
    movement_equipment: MovementEquipment,
    pub version: MinecraftVersion,
    pub id: u64,
    pub status: ControlStatus,
    pub controls: Controls,
    pub body: Body,
    pub tick: u64,
    pub dispatched_ticks: u64,
    pub frame: Option<ControlFrame>,
    pub corrections: u64,
    pub velocity_updates: u64,
    last_pose: Option<u64>,
    last_velocity: Option<u64>,
    sent_sneak: bool,
    sent_sprint: bool,
    sent_input: Option<u8>,
    /// Receive sequence of the item-use flags current when use was released locally.
    released_use: Option<u64>,
    ground_jump: Option<GroundJumpRequest>,
    next_ground_jump: u64,
}

impl ControlSession {
    /// Start from the latest received pose; earlier receipts are already applied.
    pub(crate) fn new(
        version: MinecraftVersion,
        id: u64,
        position: [f64; 3],
        received: &Received,
    ) -> Self {
        let mut body = Body::new(position);
        body.on_ground = false;
        if let Some((_, _, Some(v))) = received.pose {
            body.velocity = v;
        }
        if let Some((sequence, value)) = received.velocity {
            if received.pose.is_none_or(|(pose_sequence, _, velocity)| {
                velocity.is_none() || sequence > pose_sequence
            }) {
                body.velocity = value;
            }
        }
        Self::from_model(version, id, body, [0.0; 2], received)
    }

    /// Resume from the SDK's current local model, never from retained packets.
    /// The adapter has already applied the captured receipt boundary to this
    /// model. Neither its velocity nor its ground flag is a server receipt.
    pub(crate) fn from_model(
        version: MinecraftVersion,
        id: u64,
        body: Body,
        rotation: [f32; 2],
        received: &Received,
    ) -> Self {
        Self {
            movement_equipment: MovementEquipment::from(&received.environment),
            version,
            id,
            status: ControlStatus::Running,
            controls: Controls {
                yaw: rotation[0],
                pitch: rotation[1],
                ..Default::default()
            },
            body,
            tick: 0,
            dispatched_ticks: 0,
            frame: None,
            corrections: 0,
            velocity_updates: 0,
            last_pose: received.pose.map(|p| p.0),
            last_velocity: received.velocity.map(|v| v.0),
            sent_sneak: false,
            sent_sprint: false,
            sent_input: None,
            released_use: None,
            ground_jump: None,
            next_ground_jump: 0,
        }
    }

    /// Retain stopped model momentum and consume only subsequently received
    /// corrections/velocities. The adapter must check the world generation and
    /// exclude intervening unrelated local movement before calling this.
    pub(crate) fn resume(&self, id: u64, rotation: [f32; 2], received: &Received) -> Self {
        let mut next = Self::from_model(self.version, id, self.body.clone(), rotation, received);
        next.last_pose = self.last_pose;
        next.last_velocity = self.last_velocity;
        next.released_use = self.released_use;
        next.apply_received(received);
        // The start boundary is part of the initial seed, just as it is for a
        // fresh model. Counters describe updates after that boundary.
        next.corrections = 0;
        next.velocity_updates = 0;
        next
    }

    pub(crate) fn inherit_item_release(&mut self, previous: &Self) {
        self.released_use = previous.released_use;
    }

    pub(crate) fn has_new_pose(&self, received: &Received) -> bool {
        received
            .pose
            .is_some_and(|p| self.last_pose.is_none_or(|seen| p.0 > seen))
    }

    /// The client stops using an item as soon as it releases it; the received flags
    /// current at that moment no longer apply.
    pub(crate) fn item_released(&mut self, received: &Received) {
        self.released_use = received.using_item.as_ref().map(|u| u.0);
    }

    pub(crate) fn request_ground_jump(&mut self, session_id: u64) -> Result<GroundJumpRequest> {
        if self.id != session_id || self.status != ControlStatus::Running {
            return Err(crate::client::registry::invalid(
                "ground jump requires the same running control session",
            ));
        }
        if let Some(request) = &self.ground_jump {
            if request.status == GroundJumpStatus::Queued {
                return Ok(request.clone());
            }
        }
        self.next_ground_jump = self
            .next_ground_jump
            .checked_add(1)
            .ok_or_else(|| crate::client::registry::invalid("ground jump request IDs exhausted"))?;
        let request = GroundJumpRequest {
            session_id,
            request_id: self.next_ground_jump,
            status: GroundJumpStatus::Queued,
        };
        self.ground_jump = Some(request.clone());
        Ok(request)
    }

    fn cancel_ground_jump(&mut self, reason: &str) {
        if let Some(request) = self
            .ground_jump
            .as_mut()
            .filter(|r| r.status == GroundJumpStatus::Queued)
        {
            request.status = GroundJumpStatus::Cancelled {
                reason: reason.into(),
            };
        }
    }

    pub(crate) fn record(&self) -> ControlRecord {
        let mut ground_jump = self.ground_jump.clone();
        if let Some(request) = ground_jump
            .as_mut()
            .filter(|r| r.status == GroundJumpStatus::Queued)
        {
            let reason = match &self.status {
                ControlStatus::Running => None,
                ControlStatus::Paused { reason } | ControlStatus::Stopped { reason } => {
                    Some(reason)
                }
            };
            if let Some(reason) = reason {
                request.status = GroundJumpStatus::Cancelled {
                    reason: reason.clone(),
                };
            }
        }
        ControlRecord {
            movement_equipment: self.movement_equipment.clone(),
            session_id: self.id,
            status: self.status.clone(),
            controls: self.controls,
            dispatched_ticks: self.dispatched_ticks,
            frame: self.frame.clone(),
            corrections: self.corrections,
            velocity_updates: self.velocity_updates,
            ground_jump,
        }
    }

    pub(crate) fn running(&self) -> bool {
        !matches!(self.status, ControlStatus::Stopped { .. })
    }

    /// Apply receipts newer than the session has seen, as the client would.
    fn apply_received(&mut self, received: &Received) {
        let pose = received
            .pose
            .filter(|p| self.last_pose.is_none_or(|seen| p.0 > seen));
        let velocity = received
            .velocity
            .filter(|v| self.last_velocity.is_none_or(|seen| v.0 > seen));
        // A newer teleport can reset velocity. Apply the two latest receipts in
        // arrival order, rather than replaying an older knockback after it.
        if velocity.is_some_and(|v| pose.is_some_and(|p| v.0 < p.0)) {
            self.apply_velocity(velocity);
            self.apply_pose(pose);
        } else {
            self.apply_pose(pose);
            self.apply_velocity(velocity);
        }
    }

    fn apply_pose(&mut self, pose: Option<(u64, [f64; 3], Option<[f64; 3]>)>) {
        if let Some((sequence, position, velocity)) = pose {
            self.last_pose = Some(sequence);
            self.body.teleport(self.version, position);
            if let Some(v) = velocity {
                self.body.velocity = v;
            }
            self.corrections += 1;
        }
    }

    fn apply_velocity(&mut self, velocity: Option<(u64, [f64; 3])>) {
        if let Some((sequence, value)) = velocity {
            self.last_velocity = Some(sequence);
            self.body.velocity = value;
            self.velocity_updates += 1;
        }
    }

    /// Run one tick. `Ok(None)` while paused (nothing to send).
    pub(crate) fn step(
        &mut self,
        received: &Received,
        block_at: &mut impl FnMut([i32; 3]) -> Result<NativeBlockState>,
    ) -> Result<Option<Output>> {
        if !self.running() {
            return Ok(None);
        }
        // Adapter reconstruction may pause outside step(). A queued request
        // must never survive that gap and trigger after terrain becomes usable.
        if matches!(self.status, ControlStatus::Paused { .. }) {
            self.cancel_ground_jump("control was paused before the next physics tick");
        }
        let ground_requested = self
            .ground_jump
            .as_ref()
            .is_some_and(|r| r.status == GroundJumpStatus::Queued);
        self.cancel_ground_jump("next physics tick could not be predicted");
        self.apply_received(received);
        let mut environment = received.environment.clone();
        self.movement_equipment = MovementEquipment::from(&environment);
        environment.using_item = match &received.using_item {
            Some((sequence, _)) if self.released_use == Some(*sequence) => None,
            Some((_, Ok(using))) => *using,
            Some((_, Err(reason))) => {
                self.status = ControlStatus::Paused {
                    reason: reason.clone(),
                };
                return Ok(None);
            }
            None => None,
        };
        self.tick += 1;
        let controls = self.controls;
        let ground_outcome = match physics::tick_with_ground_jump(
            self.version,
            &mut self.body,
            &environment,
            controls,
            ground_requested,
            block_at,
        ) {
            Ok(outcome) => {
                self.status = ControlStatus::Running;
                outcome
            }
            // Unsupported terrain, unloaded cells: hold position and retry.
            Err(error) => {
                self.status = ControlStatus::Paused {
                    reason: error.to_string(),
                };
                return Ok(None);
            }
        };
        if let Some(outcome) = ground_outcome {
            self.ground_jump.as_mut().unwrap().status = GroundJumpStatus::Evaluated {
                tick: self.tick,
                outcome,
                dispatched: false,
            };
        }
        let b = &self.body;
        self.frame = Some(ControlFrame {
            tick: self.tick,
            position: b.position,
            velocity: b.velocity,
            on_ground: b.on_ground,
            horizontal_collision: b.horizontal_collision,
            pose: b.pose,
            sprinting: b.sprinting,
            crouching: b.crouching,
            in_water: b.in_water,
            swimming: b.swimming,
            eye_in_water: b.eye_in_water,
            using_item: environment.using_item,
        });
        let modern = self.version == MinecraftVersion::Java1_21_11;
        let input = modern.then(|| {
            input_bits(controls)
                | if ground_outcome == Some(GroundJumpOutcome::Applied) {
                    1 << 4
                } else {
                    0
                }
        });
        let output = Output {
            sneak: (controls.sneak != self.sent_sneak).then_some(controls.sneak),
            sprint: (b.sprinting != self.sent_sprint).then_some(b.sprinting),
            input: input.filter(|bits| Some(*bits) != self.sent_input),
            position: b.position,
            rotation: [controls.yaw, controls.pitch],
            on_ground: b.on_ground,
            horizontal_collision: b.horizontal_collision,
        };
        Ok(Some(output))
    }

    /// Record that a tick's packets were completely written.
    pub(crate) fn dispatched(&mut self, output: &Output) {
        if let Some(s) = output.sneak {
            self.sent_sneak = s;
        }
        if let Some(s) = output.sprint {
            self.sent_sprint = s;
        }
        if let Some(bits) = output.input {
            self.sent_input = Some(bits);
        }
        self.dispatched_ticks = self.tick;
        if let Some(request) = self.ground_jump.as_mut() {
            if let GroundJumpStatus::Evaluated {
                tick, dispatched, ..
            } = &mut request.status
            {
                if *tick == self.tick {
                    *dispatched = true;
                }
            }
        }
    }

    /// Packets that release held sprint/sneak when the session stops.
    pub(crate) fn release(&self) -> (Option<bool>, Option<bool>, Option<u8>) {
        let modern = self.version == MinecraftVersion::Java1_21_11;
        (
            self.sent_sneak.then_some(false),
            self.sent_sprint.then_some(false),
            (modern && self.sent_input.is_some_and(|b| b != 0)).then_some(0),
        )
    }
}

/// Modern `Input` flags: forward, backward, left, right, jump, shift, sprint.
fn input_bits(c: Controls) -> u8 {
    u8::from(c.forward > 0)
        | (u8::from(c.forward < 0) << 1)
        | (u8::from(c.strafe > 0) << 2)
        | (u8::from(c.strafe < 0) << 3)
        | (u8::from(c.jump) << 4)
        | (u8::from(c.sneak) << 5)
        | (u8::from(c.sprint) << 6)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn received(
        pose: Option<(u64, [f64; 3], Option<[f64; 3]>)>,
        velocity: Option<(u64, [f64; 3])>,
    ) -> Received {
        Received {
            environment: Environment::defaults(MinecraftVersion::Java1_21_11),
            pose,
            velocity,
            using_item: None,
        }
    }

    fn world(ladder: bool) -> impl FnMut([i32; 3]) -> Result<NativeBlockState> {
        move |p| {
            let name = if p[1] == 63 {
                "minecraft:stone"
            } else if ladder && p == [0, 64, 0] {
                return Ok(NativeBlockState {
                    name: "minecraft:ladder".into(),
                    properties: [
                        ("facing".into(), "north".into()),
                        ("waterlogged".into(), "false".into()),
                    ]
                    .into_iter()
                    .collect(),
                });
            } else {
                "minecraft:air"
            };
            Ok(NativeBlockState {
                name: name.into(),
                properties: Default::default(),
            })
        }
    }

    #[test]
    fn ground_request_preserves_keys_coalesces_and_matches_one_native_ground_jump() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let start = received(Some((5, [0.5, 64., 0.5], Some([0.; 3]))), None);
            let mut session = ControlSession::new(version, 8, [0.5, 64., 0.5], &start);
            session.controls = Controls {
                forward: 1,
                sneak: true,
                yaw: 37.,
                pitch: -12.,
                ..Default::default()
            };
            for _ in 0..2 {
                let settled = session.step(&start, &mut world(false)).unwrap().unwrap();
                session.dispatched(&settled);
            }
            let held = session.controls;
            let mut reference = session.body.clone();
            let queued = session.request_ground_jump(8).unwrap();
            assert_eq!(queued.status, GroundJumpStatus::Queued);
            assert_eq!(session.request_ground_jump(8).unwrap(), queued);
            assert!(session.request_ground_jump(7).is_err());
            assert_eq!(session.controls, held);
            let mut peaks = 0;
            let mut rising = false;
            for tick in 0..32 {
                let mut controls = held;
                controls.jump = tick == 0;
                physics::tick(
                    version,
                    &mut reference,
                    &start.environment,
                    controls,
                    &mut world(false),
                )
                .unwrap();
                let output = session.step(&start, &mut world(false)).unwrap().unwrap();
                assert_eq!(session.body.position, reference.position);
                assert_eq!(session.body.velocity, reference.velocity);
                assert_eq!(session.controls, held);
                if tick == 0 {
                    assert!(matches!(
                        session.record().ground_jump.unwrap().status,
                        GroundJumpStatus::Evaluated {
                            outcome: GroundJumpOutcome::Applied,
                            dispatched: false,
                            ..
                        }
                    ));
                    assert_eq!(
                        output.input,
                        (version == MinecraftVersion::Java1_21_11).then_some(0x31)
                    );
                } else if tick == 1 {
                    assert_eq!(
                        output.input,
                        (version == MinecraftVersion::Java1_21_11).then_some(0x21)
                    );
                }
                let next_rising = session.body.velocity[1] > 0.;
                if next_rising && !rising {
                    peaks += 1;
                }
                rising = next_rising;
                session.dispatched(&output);
            }
            assert_eq!(peaks, 1);
            assert!(session.body.on_ground);
            assert!(matches!(
                session.record().ground_jump.unwrap().status,
                GroundJumpStatus::Evaluated {
                    tick: 3,
                    outcome: GroundJumpOutcome::Applied,
                    dispatched: true
                }
            ));
        }
    }

    #[test]
    fn airborne_ground_request_is_consumed_before_landing() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let start = received(Some((5, [0.5, 68., 0.5], Some([0.; 3]))), None);
            let mut session = ControlSession::new(version, 1, [0.5, 68., 0.5], &start);
            session.request_ground_jump(1).unwrap();
            for _ in 0..24 {
                let output = session.step(&start, &mut world(false)).unwrap().unwrap();
                assert!(session.body.velocity[1] <= 0.);
                session.dispatched(&output);
            }
            assert!(session.body.on_ground);
            assert!(matches!(
                session.record().ground_jump.unwrap().status,
                GroundJumpStatus::Evaluated {
                    tick: 1,
                    outcome: GroundJumpOutcome::Airborne,
                    dispatched: true
                }
            ));
            assert_eq!(session.request_ground_jump(1).unwrap().request_id, 2);
            session.step(&start, &mut world(false)).unwrap();
            assert!(session.body.velocity[1] > 0.);
        }
    }

    #[test]
    fn ground_request_does_not_change_held_swimming_or_climbing() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            for water in [false, true] {
                let start = received(Some((5, [0.5, 64., 0.5], Some([0.; 3]))), None);
                let mut session = ControlSession::new(version, 1, [0.5, 64., 0.5], &start);
                session.controls = Controls {
                    jump: true,
                    yaw: 37.,
                    pitch: -12.,
                    ..Default::default()
                };
                let mut dry = world(!water);
                let mut blocks = |p: [i32; 3]| {
                    if water && (64..=70).contains(&p[1]) {
                        Ok(NativeBlockState {
                            name: "minecraft:water".into(),
                            properties: [("level".into(), "0".into())].into_iter().collect(),
                        })
                    } else {
                        dry(p)
                    }
                };
                let output = session.step(&start, &mut blocks).unwrap().unwrap();
                session.dispatched(&output);
                let mut reference = session.body.clone();
                session.request_ground_jump(1).unwrap();
                physics::tick(
                    version,
                    &mut reference,
                    &start.environment,
                    session.controls,
                    &mut blocks,
                )
                .unwrap();
                let output = session.step(&start, &mut blocks).unwrap().unwrap();
                assert_eq!(session.body, reference);
                assert!(session.controls.jump);
                assert!(matches!(
                    session.record().ground_jump.unwrap().status,
                    GroundJumpStatus::Evaluated {
                        outcome: GroundJumpOutcome::AlreadyHeld,
                        ..
                    }
                ));
                assert_eq!(output.input, None);
            }
        }
    }

    #[test]
    fn ground_request_reports_received_zero_jump_power_without_inventing_an_impulse() {
        let version = MinecraftVersion::Java1_21_11;
        let mut start = received(Some((5, [0.5, 64., 0.5], Some([0.; 3]))), None);
        start.environment.jump_strength = 0.;
        let mut session = ControlSession::new(version, 1, [0.5, 64., 0.5], &start);
        for _ in 0..2 {
            let output = session.step(&start, &mut world(false)).unwrap().unwrap();
            session.dispatched(&output);
        }
        session.request_ground_jump(1).unwrap();
        let output = session.step(&start, &mut world(false)).unwrap().unwrap();
        assert_eq!(session.body.position[1], 64.);
        assert!(session.body.on_ground);
        assert_eq!(output.input, None);
        assert!(matches!(
            session.record().ground_jump.unwrap().status,
            GroundJumpStatus::Evaluated {
                outcome: GroundJumpOutcome::NoJumpPower,
                dispatched: false,
                ..
            }
        ));
    }

    #[test]
    fn ground_request_is_discarded_on_pause_stop_or_replacement() {
        let start = received(Some((5, [0.5, 64., 0.5], Some([0.; 3]))), None);
        let mut session =
            ControlSession::new(MinecraftVersion::Java1_16_1, 1, [0.5, 64., 0.5], &start);
        session.request_ground_jump(1).unwrap();
        let mut unavailable = |_| Err(crate::client::registry::invalid("unloaded"));
        assert!(session.step(&start, &mut unavailable).unwrap().is_none());
        assert!(session.request_ground_jump(1).is_err());
        assert!(matches!(
            session.record().ground_jump.unwrap().status,
            GroundJumpStatus::Cancelled { .. }
        ));
        for _ in 0..3 {
            session.step(&start, &mut world(false)).unwrap();
        }
        assert!(session.body.velocity[1] <= 0.);
        session.request_ground_jump(1).unwrap();
        session.status = ControlStatus::Stopped {
            reason: "world changed".into(),
        };
        assert!(matches!(
            session.record().ground_jump.unwrap().status,
            GroundJumpStatus::Cancelled { .. }
        ));
        assert!(session.request_ground_jump(1).is_err());
        let mut replacement = session.resume(2, [37., -12.], &start);
        assert!(replacement.record().ground_jump.is_none());
        assert!(replacement.request_ground_jump(1).is_err());
        assert!(
            replacement
                .step(&start, &mut world(false))
                .unwrap()
                .is_some()
        );
        assert!(replacement.body.velocity[1] <= 0.);
    }

    #[test]
    fn packets_follow_key_and_sprint_changes_only() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let start = received(Some((5, [0.5, 64.0, 0.5], None)), None);
            let mut session = ControlSession::new(version, 1, [0.5, 64.0, 0.5], &start);
            let mut blocks = world(false);
            let first = session.step(&start, &mut blocks).unwrap().unwrap();
            assert_eq!(first.sprint, None);
            assert_eq!(
                first.input,
                (version == MinecraftVersion::Java1_21_11).then_some(0)
            );
            session.dispatched(&first);
            session.controls = Controls {
                forward: 1,
                sprint: true,
                ..Default::default()
            };
            let run = session.step(&start, &mut blocks).unwrap().unwrap();
            assert_eq!(run.sprint, Some(true));
            session.dispatched(&run);
            let steady = session.step(&start, &mut blocks).unwrap().unwrap();
            assert_eq!((steady.sprint, steady.input), (None, None));
            session.dispatched(&steady);
            assert!(session.frame.as_ref().unwrap().position[2] > 0.5);
            assert_eq!(session.release().1, Some(false));
        }
    }

    #[test]
    fn item_use_follows_received_flags_and_local_release() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let mut start = received(Some((5, [0.5, 64.0, 0.5], None)), None);
            let mut session = ControlSession::new(version, 1, [0.5, 64.0, 0.5], &start);
            let mut blocks = world(false);
            session.controls = Controls {
                forward: 1,
                sprint: true,
                ..Default::default()
            };
            // The server reports a shield raised: slowed, and sprinting cannot start.
            start.using_item = Some((8, Ok(Some(ItemUse::DEFAULT))));
            session.step(&start, &mut blocks).unwrap().unwrap();
            let frame = session.frame.clone().unwrap();
            assert_eq!(frame.using_item, Some(ItemUse::DEFAULT));
            assert!(!frame.sprinting);
            // Released locally: the stale flags no longer apply.
            session.item_released(&start);
            session.step(&start, &mut blocks).unwrap().unwrap();
            assert_eq!(session.frame.as_ref().unwrap().using_item, None);
            assert!(session.frame.as_ref().unwrap().sprinting);
            // Newer flags apply again; an unresolved item pauses without sending.
            start.using_item = Some((9, Err("unknown".into())));
            assert!(session.step(&start, &mut blocks).unwrap().is_none());
            assert!(matches!(session.status, ControlStatus::Paused { .. }));
            start.using_item = Some((10, Ok(None)));
            assert!(session.step(&start, &mut blocks).unwrap().is_some());
            assert_eq!(session.status, ControlStatus::Running);
        }
    }

    #[test]
    fn received_corrections_and_velocity_apply_once() {
        let version = MinecraftVersion::Java1_21_11;
        let start = received(Some((5, [0.5, 64.0, 0.5], None)), None);
        let mut session = ControlSession::new(version, 1, [0.5, 64.0, 0.5], &start);
        let mut blocks = world(false);
        session.step(&start, &mut blocks).unwrap();
        let moved = received(
            Some((9, [3.5, 64.0, 3.5], Some([0.0; 3]))),
            Some((7, [0.0, 0.42, 0.0])),
        );
        session.step(&moved, &mut blocks).unwrap();
        session.step(&moved, &mut blocks).unwrap();
        assert_eq!((session.corrections, session.velocity_updates), (1, 1));
        let frame = session.frame.as_ref().unwrap();
        assert_eq!((frame.position[0], frame.position[2]), (3.5, 3.5));
        assert_eq!(
            frame.position[1], 64.0,
            "newer teleport resets older velocity"
        );
        let knockback = received(moved.pose, Some((10, [0.0, 0.42, 0.0])));
        session.step(&knockback, &mut blocks).unwrap();
        assert!(session.frame.as_ref().unwrap().position[1] > 64.0);
        assert_eq!((session.corrections, session.velocity_updates), (1, 2));
    }

    #[test]
    fn resume_preserves_falling_model_and_only_applies_new_receipts() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let mut start = received(
                Some((5, [0.5, 68.0, 0.5], Some([0.0; 3]))),
                Some((7, [0.08, -0.1, 0.0])),
            );
            start.environment = Environment::defaults(version);
            let mut original = ControlSession::new(version, 1, [0.5, 68.0, 0.5], &start);
            original.controls.yaw = 37.0;
            for _ in 0..3 {
                original.step(&start, &mut world(false)).unwrap();
            }
            let mut uninterrupted = ControlSession::from_model(
                version,
                1,
                original.body.clone(),
                [37.0, -12.0],
                &start,
            );
            let mut resumed = original.resume(2, [37.0, -12.0], &start);
            uninterrupted.controls.pitch = -12.0;
            assert!(!resumed.body.on_ground);
            for _ in 0..20 {
                let expected = uninterrupted
                    .step(&start, &mut world(false))
                    .unwrap()
                    .unwrap();
                let actual = resumed.step(&start, &mut world(false)).unwrap().unwrap();
                assert_eq!(actual.position, expected.position);
                assert_eq!(actual.rotation, [37.0, -12.0]);
                assert_eq!(actual.on_ground, expected.on_ground);
                assert_eq!(resumed.body.velocity, uninterrupted.body.velocity);
            }
            assert!(resumed.body.on_ground);
            assert_eq!((resumed.corrections, resumed.velocity_updates), (0, 0));
            // A new velocity received after stop supersedes the old model once.
            let next = received(start.pose, Some((9, [0.0, 0.42, 0.0])));
            let mut jumped = resumed.resume(3, [37.0, -12.0], &next);
            jumped.step(&next, &mut world(false)).unwrap();
            assert!(jumped.body.position[1] > 64.0);
            assert_eq!((jumped.corrections, jumped.velocity_updates), (0, 0));
            jumped.step(&next, &mut world(false)).unwrap();
            assert_eq!((jumped.corrections, jumped.velocity_updates), (0, 0));
            // A later correction resets the earlier knockback, including when
            // both arrived while control ownership was released.
            let corrected = received(Some((10, [2.5, 64.0, 2.5], Some([0.0; 3]))), next.velocity);
            let mut corrected = resumed.resume(4, [37.0, -12.0], &corrected);
            assert_eq!(corrected.body.position, [2.5, 64.0, 2.5]);
            assert_eq!(corrected.body.velocity, [0.0; 3]);
            assert_eq!((corrected.corrections, corrected.velocity_updates), (0, 0));
            corrected.step(&start, &mut world(false)).unwrap();
            assert_eq!(corrected.body.position[1], 64.0);
        }
    }

    #[test]
    fn starting_velocity_uses_the_latest_receipt() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            for (pose_sequence, pose_velocity, velocity_sequence, expected) in [
                (5, Some([0.0; 3]), 7, [0.0, 0.42, 0.0]),
                (7, Some([0.0; 3]), 5, [0.0; 3]),
                (7, None, 5, [0.0, 0.42, 0.0]),
            ] {
                let start = received(
                    Some((pose_sequence, [0.5, 64.0, 0.5], pose_velocity)),
                    Some((velocity_sequence, [0.0, 0.42, 0.0])),
                );
                let mut session = ControlSession::new(version, 1, [0.5, 64.0, 0.5], &start);
                assert_eq!(session.body.velocity, expected);
                session.step(&start, &mut world(false)).unwrap();
                assert_eq!((session.corrections, session.velocity_updates), (0, 0));
                assert_eq!(session.body.position[1] > 64.0, expected[1] > 0.0);
            }
        }
    }

    #[test]
    fn unsupported_terrain_pauses_without_output_and_resumes() {
        let version = MinecraftVersion::Java1_16_1;
        let start = received(Some((5, [0.5, 64.0, 0.5], None)), None);
        let mut session = ControlSession::new(version, 1, [0.5, 64.0, 0.5], &start);
        let mut ordinary = world(false);
        let mut unsupported = |p| {
            if p == [0, 64, 0] {
                Ok(NativeBlockState {
                    name: "minecraft:nether_portal".into(),
                    properties: [("axis".into(), "x".into())].into_iter().collect(),
                })
            } else {
                ordinary(p)
            }
        };
        assert_eq!(session.step(&start, &mut unsupported).unwrap(), None);
        assert!(matches!(session.status, ControlStatus::Paused { .. }));
        assert!(session.step(&start, &mut world(false)).unwrap().is_some());
        assert_eq!(session.status, ControlStatus::Running);
    }

    #[test]
    fn bubble_columns_keep_control_running_in_both_directions() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            for down in [false, true] {
                let start = received(Some((5, [0.5, 68.0, 0.5], None)), None);
                let mut session = ControlSession::new(version, 1, [0.5, 68.0, 0.5], &start);
                let mut ordinary = world(false);
                let mut column = |p: [i32; 3]| {
                    if p[0] == 0 && p[2] == 0 && (64..80).contains(&p[1]) {
                        Ok(NativeBlockState {
                            name: "minecraft:bubble_column".into(),
                            properties: [("drag".into(), down.to_string())].into_iter().collect(),
                        })
                    } else {
                        ordinary(p)
                    }
                };
                for _ in 0..4 {
                    let output = session.step(&start, &mut column).unwrap().unwrap();
                    session.dispatched(&output);
                    assert_eq!(session.status, ControlStatus::Running);
                }
                let y = session.frame.as_ref().unwrap().position[1];
                assert!(if down { y < 68.0 } else { y > 68.0 });
                assert_eq!(session.dispatched_ticks, 4);
            }
        }
    }

    #[test]
    fn ladders_keep_continuous_control_running_and_send_upward_motion() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let start = received(Some((5, [0.5, 64.0, 0.5], None)), None);
            let mut session = ControlSession::new(version, 1, [0.5, 64.0, 0.5], &start);
            session.controls = Controls {
                jump: true,
                ..Default::default()
            };
            let mut ladder = world(true);
            for _ in 0..3 {
                let output = session.step(&start, &mut ladder).unwrap().unwrap();
                session.dispatched(&output);
                assert_eq!(session.status, ControlStatus::Running);
            }
            assert_eq!(session.dispatched_ticks, 3);
            assert!(session.frame.as_ref().unwrap().position[1] > 64.0);
        }
    }
}
