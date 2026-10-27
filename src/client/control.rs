//! Continuous control (P7): held keys and per-tick client physics, shared by
//! both adapters. Contract: docs/common-control.md.
//!
//! The session runs the shared engine (`client::physics`) once per 50 ms tick
//! on the client's clock and sends what the client sends. A position packet is a
//! submission, never server acceptance; received corrections and velocity are
//! applied as the vanilla client applies them and are counted separately.
use crate::client::physics::{self, Body, Environment};

#[path = "control_changes.rs"]
mod changes;
use crate::{MinecraftVersion, NativeBlockState, Result};
pub use changes::{
    ControlChange, ControlChangeGap, ControlChangeLog, ControlCorrection, ControlCorrectionChange,
    ControlCorrectionLog, ControlDegradation, ControlSubmission, DegradationPhase,
    MAX_RETAINED_CONTROL_CHANGES,
};
pub(crate) use changes::{
    ControlChangeLedger, ControlChangeSource, ControlCorrectionLedger, ControlCorrectionSource,
};

pub use crate::client::physics::{Controls, ItemUse, Modifier, ModifierOperation, Pose};

#[path = "control_states.rs"]
mod states;
pub(crate) use states::ControlStateTracker;
pub use states::{
    ControlStateChange, ControlStateEvent, ControlStateLog, ControlStopCause, ControlTransition,
};

/// Maximum speculative walking ticks before a new pose or usable terrain is required.
pub const MAX_UNPROVEN_WALK_TICKS: u16 = 20;

/// Warning retained by best-effort walking, never a support proof or server ACK.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct TerrainDiagnostic {
    /// Original terrain prediction failure, including its diagnostic chain.
    pub reason: String,
    /// Speculative ticks since the last received correction or usable terrain tick.
    pub unproven_ticks: u16,
    /// No more speculative frames until fresh evidence re-anchors this owner.
    pub waiting_for_evidence: bool,
}

/// Additional walking diagnostics for a selected owner, without changing the
/// existing ControlRecord layout. Never establishes server acceptance.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct WalkingDiagnostics {
    /// Latest terrain warning, retained after recovery and stopping.
    pub terrain: Option<TerrainDiagnostic>,
    /// Present for a connection-owned finite best-effort route.
    pub path: Option<ControlPathProgress>,
}

/// Connection-owned finite best-effort path progress; dispatch is not acceptance.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ControlPathProgress {
    /// Original number of inputs, at most 120.
    pub total_ticks: u16,
    /// Inputs completely written, each exactly once.
    pub dispatched_ticks: u16,
}

pub(crate) fn validate_ground_mode(mode: super::GameMode) -> Result<()> {
    if mode == super::GameMode::Spectator {
        return Err(crate::Error::new(
            crate::ErrorKind::Unsupported,
            anyhow::anyhow!("spectator requires a separate flight model"),
        ));
    }
    Ok(())
}

/// Explicit model selected at admission, never converted by a mode change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlKind {
    /// Ordinary ground/fluid/climbing controls.
    Ground,
    /// Creative or Spectator held flight, with original abilities speed.
    Flight,
    /// Elytra glide; starts only after an actual accepted metadata flag.
    Glide,
}

/// Converted inputs to the held-flight model, not current flight or an ACK.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct FlightParameters {
    /// Flight speed from the original abilities receipt.
    pub ability_flying_speed: f32,
    /// Original receive ordinal supplying that speed.
    pub abilities_sequence: u64,
    /// The admitted Spectator model uses native noPhysics.
    pub spectator: bool,
}

/// Creative landing's owned abilities write, separate from received abilities.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct FlightDisable {
    /// Latest original receive ordinal under the write's state boundary.
    pub after_sequence: u64,
    /// The complete disable packet was written; this is not a server ACK.
    pub dispatched: bool,
}

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
    /// Numerical Elytra model, not the actual fall-flying metadata.
    pub gliding: bool,
    /// Original float scale used by this numerical frame, not a receipt.
    pub model_scale: f32,
    /// Numerical collision bounds, minimum XYZ then maximum XYZ.
    pub bounds: [f64; 6],
    /// Numerical eye position for this pose and scale.
    pub eye_position: [f64; 3],
    /// Native model flight flag after the tick; landing can clear it locally.
    pub flying: bool,
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
    /// Frozen remote-model degradation on Java 1.16.1; independent of status,
    /// movement refusal and server acceptance. None on adapters without this log.
    pub degradation: Option<ControlDegradation>,
    /// Structured cause retained when contact prediction stops this owner.
    pub movement_refusal: Option<crate::MovementRefusal>,
    /// Owned Elytra request and actual receipts; None for other control kinds.
    pub glide: Option<super::glide::GlideRecord>,
    /// Latest automatic entity contact frame, at this owner's explicit model
    /// origin. Numerical feet/velocity are separate from received targets.
    pub entity_contacts: Option<super::EntityContactFrame>,
    /// Model explicitly selected when this owner was created.
    pub kind: ControlKind,
    /// Last converted held-flight parameters, retained when the owner stops.
    pub flight_parameters: Option<FlightParameters>,
    /// Owned landing toggle, retained even if a subsequent movement write fails.
    pub flight_disable: Option<FlightDisable>,
    /// Expected received mode at admission; changing mode stops this owner.
    pub mode: super::GameMode,
    /// First actual mode-change packet that disagreed with this owner, even if
    /// a later packet restored the mode before a local tick. Not a server tick.
    pub mode_change_sequence: Option<u64>,
    /// First original abilities packet contradicting this model: flight starts
    /// for Ground, or flight/permission is removed for Flight. Later restoration
    /// cannot resume the interrupted owner.
    pub flight_change_sequence: Option<u64>,
    /// First original own-player scale receipt differing from this owner's origin.
    /// Java 1.16.1 has no scale attribute. Restoration does not resume this owner.
    pub scale_change_sequence: Option<u64>,
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
    /// Received explosion knockback impulses added once to this Ground model.
    pub explosion_impulses: u64,
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

/// Validate original abilities before acquiring a flight owner. The locally
/// requested flag is independent of receipt provenance and never grants flight.
pub(crate) fn flight_admission(
    abilities: Option<&super::ObservedValue<super::PlayerAbilities>>,
    requested: bool,
    locally_disabled: bool,
) -> Option<&'static str> {
    let Some(abilities) =
        abilities.filter(|a| matches!(a.source, super::ValueSource::Received { .. }))
    else {
        return Some("flight requires current-world received abilities");
    };
    if !abilities.value.may_fly() {
        return Some("flight permission unavailable");
    }
    if !abilities.value.flying_speed.is_finite() || abilities.value.flying_speed < 0.0 {
        return Some("flight speed is unavailable or invalid");
    }
    if !(requested || abilities.value.flying() && !locally_disabled) {
        return Some("flight must first be explicitly enabled");
    }
    None
}

/// Received facts sampled under the adapter's state lock before a tick.
pub(crate) struct Received {
    /// Source of the original abilities speed, under the same state boundary.
    pub abilities_sequence: Option<u64>,
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
    pub(crate) contact_change_source: Option<ControlChangeSource>,
    pub(crate) correction_source: Option<ControlCorrectionSource>,
    /// Latest written position, superseded by a later received pose.
    submitted: Option<ControlSubmission>,
    /// Latest logged pose receipt, and one received but not yet applied.
    logged_pose: Option<u64>,
    observed_pose: Option<(u64, [f64; 3])>,
    degradation: Option<ControlDegradation>,
    terrain_diagnostic: Option<TerrainDiagnostic>,
    unproven_ticks: u16,
    path: Option<Vec<Controls>>,
    path_dispatched: usize,
    path_deadline: Option<std::time::Instant>,
    pub(crate) movement_refusal: Option<crate::MovementRefusal>,
    pub(crate) glide: Option<super::glide::GlideRecord>,
    pub(crate) best_effort_contacts: bool,
    pub(crate) contact_diagnostic: Option<crate::MovementRefusal>,
    pub(crate) contacts: Option<super::entity::owned_contacts::OwnedContacts>,
    pub(crate) entity_contacts: Option<super::EntityContactFrame>,
    pub(crate) contact_team: Option<(String, super::ui::TeamCollision)>,
    pub kind: ControlKind,
    flight_parameters: Option<FlightParameters>,
    pub flight_disable: Option<FlightDisable>,
    pub mode: super::GameMode,
    pub mode_change_sequence: Option<u64>,
    pub flight_change_sequence: Option<u64>,
    pub scale_change_sequence: Option<u64>,
    admitted_scale: Option<f64>,
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
    pub explosion_impulses: u64,
    last_pose: Option<u64>,
    last_velocity: Option<u64>,
    last_explosion: Option<u64>,
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
        mut body: Body,
        rotation: [f32; 2],
        received: &Received,
    ) -> Self {
        if let Some(scale) = received.environment.player_scale {
            body.set_scale(scale as f32);
        }
        Self {
            contact_change_source: None,
            correction_source: None,
            submitted: None,
            logged_pose: received.pose.map(|p| p.0),
            observed_pose: None,
            degradation: None,
            terrain_diagnostic: None,
            unproven_ticks: 0,
            path: None,
            path_dispatched: 0,
            path_deadline: None,
            best_effort_contacts: false,
            contact_diagnostic: None,
            contacts: None,
            movement_refusal: None,
            entity_contacts: None,
            contact_team: None,
            kind: ControlKind::Ground,
            flight_parameters: None,
            flight_disable: None,
            mode: super::GameMode::Survival,
            mode_change_sequence: None,
            flight_change_sequence: None,
            scale_change_sequence: None,
            glide: None,
            admitted_scale: received.environment.player_scale,
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
            explosion_impulses: 0,
            last_pose: received.pose.map(|p| p.0),
            last_velocity: received.velocity.map(|v| v.0),
            last_explosion: None,
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
        next.last_explosion = self.last_explosion;
        next.released_use = self.released_use;
        next.logged_pose = received.pose.map(|p| p.0).max(self.logged_pose);
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
        if self.kind != ControlKind::Ground {
            return Err(crate::Error::new(
                crate::ErrorKind::Unsupported,
                anyhow::anyhow!("held flight uses vertical keys, not ground requests"),
            ));
        }
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
            degradation: self.degradation.clone(),
            movement_refusal: self.movement_refusal.clone(),
            glide: self.glide.clone(),
            entity_contacts: self.entity_contacts.clone(),
            kind: self.kind,
            flight_parameters: self.flight_parameters.clone(),
            flight_disable: self.flight_disable.clone(),
            mode: self.mode,
            mode_change_sequence: self.mode_change_sequence,
            flight_change_sequence: self.flight_change_sequence,
            scale_change_sequence: self.scale_change_sequence,
            movement_equipment: self.movement_equipment.clone(),
            session_id: self.id,
            status: self.status.clone(),
            controls: self.controls,
            dispatched_ticks: self.dispatched_ticks,
            frame: self.frame.clone(),
            corrections: self.corrections,
            velocity_updates: self.velocity_updates,
            explosion_impulses: self.explosion_impulses,
            ground_jump,
        }
    }

    pub(crate) fn running(&self) -> bool {
        !matches!(self.status, ControlStatus::Stopped { .. })
    }

    pub(crate) fn observe_mode(&mut self, mode: super::GameMode, sequence: u64) {
        if self.running() && mode != self.mode {
            self.mode_change_sequence.get_or_insert(sequence);
        }
    }

    pub(crate) fn observe_abilities(&mut self, flags: u8, sequence: u64) {
        let mismatch = match self.kind {
            ControlKind::Ground | ControlKind::Glide => flags & 2 != 0,
            ControlKind::Flight => flags & 6 != 6,
        };
        if self.running() && mismatch {
            self.flight_change_sequence.get_or_insert(sequence);
        }
    }

    pub(crate) fn glide_stop_reason(&self, received: &Received) -> Option<&'static str> {
        let g = self.glide.as_ref()?;
        if g.ended_sequence.is_some() {
            Some("actual glide ended")
        } else if g.equipment_invalidated_sequence.is_some() {
            Some("glider equipment unavailable or unusable")
        } else if self.has_new_pose(received) {
            Some("glide correction requires a new owner")
        } else if received.environment.levitation {
            Some("glide levitation is unsupported")
        } else if received.environment.player_scale != Some(1.0) {
            Some("glide nonunit scale is unqualified")
        } else if g.waiting_ticks >= 40 {
            Some("glide start unconfirmed; inspect before a new explicit request")
        } else {
            None
        }
    }
    /// No model tick or position write is allowed before an actual flag receipt.
    pub(crate) fn glide_waiting(&mut self) -> bool {
        let Some(g) = &mut self.glide else {
            return false;
        };
        if g.accepted_sequence.is_none() {
            g.waiting_ticks = g.waiting_ticks.saturating_add(1);
            true
        } else {
            if self.frame.is_none() {
                self.body.admit_glide_pose();
            }
            false
        }
    }

    pub(crate) fn observe_gliding(&mut self, value: bool, sequence: u64) {
        if !self.running() {
            return;
        }
        let Some(glide) = &mut self.glide else {
            return;
        };
        glide.actual = Some(crate::client::received(value, sequence));
        if sequence <= glide.after_sequence {
            return;
        }
        if value && glide.dispatched && glide.accepted_sequence.is_none() {
            glide.accepted_sequence = Some(sequence);
        } else if !value && glide.accepted_sequence.is_some() && glide.ended_sequence.is_none() {
            glide.ended_sequence = Some(sequence);
        }
    }

    pub(crate) fn observe_glider_equipment(&mut self, available: bool, sequence: u64) {
        if self.running() {
            if let Some(glide) = &mut self.glide {
                if !available && glide.equipment_invalidated_sequence.is_none() {
                    glide.equipment_invalidated_sequence = Some(sequence);
                }
            }
        }
    }

    pub(crate) fn observe_scale(&mut self, value: f64, sequence: u64) {
        if self.running() && Some(value) != self.admitted_scale {
            self.scale_change_sequence.get_or_insert(sequence);
        }
    }

    /// Apply receipts newer than the session has seen, as the client would.
    pub(crate) fn apply_received(&mut self, received: &Received) {
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

    /// Private, ordered receive input, independent of the observation history.
    /// The adapter holds the state lock and fences the world/model lifetime.
    pub(crate) fn ground_explosion(
        &mut self,
        sequence: u64,
        impulse: Option<[f64; 3]>,
        received: &Received,
    ) -> bool {
        if self.kind != ControlKind::Ground {
            return false;
        }
        if self.last_explosion.is_some_and(|seen| sequence <= seen) {
            return true;
        }
        self.apply_received(received);
        if let Some(impulse) = impulse {
            let velocity = std::array::from_fn(|axis| self.body.velocity[axis] + impulse[axis]);
            if velocity.iter().any(|v| !v.is_finite()) {
                return false;
            }
            self.body.velocity = velocity;
            self.explosion_impulses += 1;
        }
        self.last_explosion = Some(sequence);
        true
    }

    /// Log a received own pose at its receive boundary, before any tick applies
    /// it, so several corrections between two ticks are each retained. The
    /// adapter calls this under the state that published the pose; application
    /// (and the `corrections` counter) still follows `Received.pose`.
    pub(crate) fn observe_pose(
        &mut self,
        sequence: u64,
        position: [f64; 3],
        velocity: Option<[f64; 3]>,
    ) {
        if !self.running() || self.last_pose.is_some_and(|seen| sequence <= seen) {
            return;
        }
        let model = self
            .observed_pose
            .filter(|(seen, _)| self.last_pose.is_none_or(|applied| *seen > applied))
            .map_or(self.body.position, |(_, p)| p);
        self.log_correction(sequence, model, position, velocity);
        self.observed_pose = Some((sequence, position));
    }

    fn log_correction(
        &mut self,
        sequence: u64,
        model: [f64; 3],
        position: [f64; 3],
        velocity: Option<[f64; 3]>,
    ) {
        if self.logged_pose.is_some_and(|seen| sequence <= seen) {
            return;
        }
        self.logged_pose = Some(sequence);
        let Some(source) = &self.correction_source else {
            return;
        };
        let delta: [f64; 3] = std::array::from_fn(|axis| position[axis] - model[axis]);
        source.ledger.record(ControlCorrection {
            session: source.session,
            session_id: self.id,
            receive_sequence: sequence,
            model_tick: self.tick,
            submitted: self.submitted,
            model_position: model,
            received_position: position,
            delta,
            distance: delta.iter().map(|d| d * d).sum::<f64>().sqrt(),
            received_velocity: velocity,
        });
    }

    fn apply_pose(&mut self, pose: Option<(u64, [f64; 3], Option<[f64; 3]>)>) {
        if let Some((sequence, position, velocity)) = pose {
            // Fallback for a pose the receive path did not observe.
            self.log_correction(sequence, self.body.position, position, velocity);
            self.terrain_evidence();
            // A corrected local origin cannot reuse remote numerical state
            // advanced against the old origin. Strict owners retain their
            // receipt-based behavior; opted-in owners keep their held input.
            if self.best_effort_contacts && self.contacts.is_some() {
                let mut cause = crate::MovementRefusal::permanent(
                    "received pose correction changed the contact origin".into(),
                );
                let context = cause.context_mut();
                context.owner_id = Some(self.id);
                context.owner_state = crate::MovementOwnerState::Invalidated;
                context.receive_sequence = Some(sequence);
                context.model_tick = Some(self.tick);
                self.omit_contacts_at(&cause.into_error(), DegradationPhase::Correction);
            }
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

    /// Discard an unprovable remote model only for explicitly opted-in ground owners.
    /// The original local body is retained; staged contact ticks never commit on error.
    pub(crate) fn omit_contacts(&mut self, error: &crate::Error) -> bool {
        self.omit_contacts_at(error, DegradationPhase::Refresh)
    }

    pub(crate) fn omit_contacts_at(
        &mut self,
        error: &crate::Error,
        phase: DegradationPhase,
    ) -> bool {
        if !self.best_effort_contacts || self.kind != ControlKind::Ground {
            return false;
        }
        if self.degradation.is_none() {
            self.contact_diagnostic = Some(crate::MovementRefusal::RemoteStateDropped {
                refusal: Box::new(
                    error
                        .movement_refusal()
                        .cloned()
                        .unwrap_or_else(|| crate::MovementRefusal::permanent(error.to_string())),
                ),
            });
            if let Some(source) = &self.contact_change_source {
                let degradation = ControlDegradation {
                    session: source.session,
                    session_id: self.id,
                    receive_sequence: if phase == DegradationPhase::Correction {
                        error
                            .movement_refusal()
                            .and_then(|r| r.context().receive_sequence)
                            .unwrap_or(source.receive_sequence)
                    } else {
                        source.receive_sequence
                    },
                    model_tick: self.tick,
                    phase,
                    reason: self
                        .contact_diagnostic
                        .clone()
                        .expect("omitted contact reason"),
                };
                if phase != DegradationPhase::Admission {
                    source.ledger.record(degradation.clone());
                }
                self.degradation = Some(degradation);
            }
        }
        self.contacts = None;
        self.entity_contacts = None;
        self.contact_team = None;
        true
    }

    // Initial admission can still fail at movement binding. Publish only after
    // that succeeds, while the adapter still owns the application gate.
    pub(crate) fn publish_admission_degradation(&self) {
        if let (Some(source), Some(degradation)) = (&self.contact_change_source, &self.degradation)
        {
            if degradation.phase == DegradationPhase::Admission {
                source.ledger.record(degradation.clone());
            }
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
        if self
            .path_deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline)
        {
            return Err(crate::Error::new(
                crate::ErrorKind::Timeout,
                anyhow::anyhow!("best-effort path evidence deadline expired"),
            ));
        }
        if let Some(path) = &self.path {
            self.controls = path[self.path_dispatched];
        }
        // Adapter reconstruction may pause outside step(). A queued request
        // must never survive that gap and trigger after terrain becomes usable.
        if matches!(self.status, ControlStatus::Paused { .. }) {
            self.cancel_ground_jump("control was paused before the next physics tick");
        }
        if self.kind == ControlKind::Glide
            && self.glide.as_ref().is_none_or(|g| {
                !g.dispatched
                    || g.accepted_sequence.is_none()
                    || g.ended_sequence.is_some()
                    || g.equipment_invalidated_sequence.is_some()
            })
        {
            return Err(crate::client::registry::invalid(
                "glide tick requires the same confirmed actual glide owner",
            ));
        }
        let ground_requested = self
            .ground_jump
            .as_ref()
            .is_some_and(|r| r.status == GroundJumpStatus::Queued);
        self.cancel_ground_jump("next physics tick could not be predicted");
        self.apply_received(received);
        let mut environment = received.environment.clone();
        if self.kind == ControlKind::Flight {
            environment.flying = true;
            environment.spectator = self.mode == super::GameMode::Spectator;
            self.flight_parameters = environment
                .ability_flying_speed
                .zip(received.abilities_sequence)
                .map(|(speed, sequence)| FlightParameters {
                    ability_flying_speed: speed,
                    abilities_sequence: sequence,
                    spectator: environment.spectator,
                });
        }
        if self.kind == ControlKind::Glide {
            environment.gliding = true;
        }
        self.movement_equipment = MovementEquipment::from(&environment);
        environment.using_item = match &received.using_item {
            Some((sequence, _)) if self.released_use == Some(*sequence) => None,
            Some((_, Ok(using))) => *using,
            Some((_, Err(reason))) => {
                if self.kind == ControlKind::Glide {
                    self.status = ControlStatus::Stopped {
                        reason: reason.clone(),
                    };
                    return Err(crate::client::registry::invalid(reason));
                }
                self.status = ControlStatus::Paused {
                    reason: reason.clone(),
                };
                return Ok(None);
            }
            None => None,
        };
        self.tick += 1;
        let controls = self.controls;
        let contact_bodies = self.contacts.as_ref().is_some_and(|c| c.has_bodies());
        let physics_result = if let Some(contacts) = &self.contacts {
            let mut next = contacts.clone();
            let mut body = self.body.clone();
            match next.tick_player(
                &mut body,
                &environment,
                controls,
                ground_requested,
                self.contact_team.as_ref(),
                block_at,
            ) {
                Ok((outcome, frame)) => {
                    self.body = body;
                    self.contacts = Some(next);
                    self.entity_contacts = Some(frame);
                    Ok(outcome)
                }
                Err(error) if self.omit_contacts_at(&error, DegradationPhase::Prediction) => {
                    physics::tick_with_ground_jump(
                        self.version,
                        &mut self.body,
                        &environment,
                        controls,
                        ground_requested,
                        block_at,
                    )
                }
                Err(error) => Err(error),
            }
        } else {
            physics::tick_with_ground_jump(
                self.version,
                &mut self.body,
                &environment,
                controls,
                ground_requested,
                block_at,
            )
        };
        let ground_outcome = match physics_result {
            Ok(outcome) => {
                self.terrain_evidence();
                self.status = ControlStatus::Running;
                outcome
            }
            Err(error)
                if self.best_effort_contacts
                    && self.kind == ControlKind::Ground
                    && error
                        .diagnostic()
                        .downcast_ref::<physics::UnprovenTerrain>()
                        .is_some() =>
            {
                self.terrain_diagnostic = Some(TerrainDiagnostic {
                    reason: format!("{:#}", error.diagnostic()),
                    unproven_ticks: self.unproven_ticks,
                    waiting_for_evidence: self.unproven_ticks >= MAX_UNPROVEN_WALK_TICKS,
                });
                if self.unproven_ticks >= MAX_UNPROVEN_WALK_TICKS {
                    self.status = ControlStatus::Paused {
                        reason: "bounded walking awaits fresh pose or usable terrain".into(),
                    };
                    return Ok(None);
                }
                self.omit_contacts_at(&error, DegradationPhase::Terrain);
                self.body
                    .walk_unproven(self.version, controls, &environment)?;
                self.unproven_ticks += 1;
                self.terrain_diagnostic.as_mut().unwrap().unproven_ticks = self.unproven_ticks;
                self.status = ControlStatus::Running;
                None
            }
            // Unsupported terrain, unloaded cells: hold position and retry.
            Err(error) => {
                if (contact_bodies && self.contacts.is_some())
                    || self.kind == ControlKind::Glide
                    || error.movement_refusal().is_some_and(|refusal| {
                        refusal.unsupported_kind()
                            && refusal.context().entity.is_some()
                            && refusal.geometry().is_some()
                    })
                {
                    self.status = ControlStatus::Stopped {
                        reason: error.to_string(),
                    };
                    self.contacts = None;
                    return Err(error);
                }
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
            gliding: self.body.gliding,
            model_scale: b.scale,
            bounds: b.bounds,
            eye_position: [
                b.position[0],
                b.position[1] + f64::from(b.eye_height()),
                b.position[2],
            ],
            flying: environment.flying && !b.flight_landed,
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
        self.dispatched_ticks += 1;
        self.submitted = Some(ControlSubmission {
            model_tick: self.tick,
            position: output.position,
        });
        if self.path.is_some() {
            self.path_dispatched += 1;
        }
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

    pub(crate) fn walking_diagnostics(&self) -> WalkingDiagnostics {
        WalkingDiagnostics {
            terrain: self.terrain_diagnostic.clone(),
            path: self.path.as_ref().map(|path| ControlPathProgress {
                total_ticks: path.len() as u16,
                dispatched_ticks: self.path_dispatched as u16,
            }),
        }
    }

    /// A degraded terrain source is still diagnostic when its received static
    /// cells suffice for the ordinary physics calculation.
    pub(crate) fn terrain_warning(&mut self, reason: &str) {
        self.terrain_diagnostic = Some(TerrainDiagnostic {
            reason: reason.into(),
            unproven_ticks: self.unproven_ticks,
            waiting_for_evidence: false,
        });
    }

    fn terrain_evidence(&mut self) {
        self.unproven_ticks = 0;
        if let Some(diagnostic) = &mut self.terrain_diagnostic {
            diagnostic.unproven_ticks = 0;
            diagnostic.waiting_for_evidence = false;
        }
    }

    pub(crate) fn inherit_terrain_budget(&mut self, previous: &Self) {
        if self.last_pose == previous.last_pose {
            self.unproven_ticks = previous.unproven_ticks;
            self.terrain_diagnostic = previous.terrain_diagnostic.clone();
        }
    }

    pub(crate) fn set_path(&mut self, path: &[super::survival::SurvivalControl]) {
        self.path = Some(
            path.iter()
                .map(|c| Controls {
                    forward: c.input.forward,
                    strafe: c.input.strafe,
                    jump: c.input.jump,
                    yaw: c.yaw,
                    pitch: self.controls.pitch,
                    ..Default::default()
                })
                .collect(),
        );
        self.controls = self.path.as_ref().unwrap()[0];
        self.path_deadline = Some(
            std::time::Instant::now()
                + std::time::Duration::from_millis(
                    (path.len() as u64 + u64::from(MAX_UNPROVEN_WALK_TICKS)) * 50 + 1000,
                ),
        );
    }

    pub(crate) fn path_active(&self) -> bool {
        self.path.is_some()
    }

    pub(crate) fn path_complete(&self) -> bool {
        self.path
            .as_ref()
            .is_some_and(|path| self.path_dispatched == path.len())
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
            abilities_sequence: None,
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
    fn stopped_record_retains_typed_movement_refusal() {
        let mut session = glide_session();
        let detail = crate::MovementRefusal::permanent("contact history gap".into());
        session.movement_refusal = Some(detail.clone());
        session.status = ControlStatus::Stopped {
            reason: detail.to_string(),
        };
        let record = session.record();
        assert_eq!(record.movement_refusal, Some(detail));
        assert!(matches!(record.status, ControlStatus::Stopped { .. }));
    }

    #[test]
    fn scaled_origin_is_installed_before_contact_ticks_and_latches_any_change() {
        let mut facts = received(Some((1, [0.5, 64., 0.5], Some([0.; 3]))), None);
        facts.environment.player_scale = Some(2.0);
        let mut session =
            ControlSession::new(MinecraftVersion::Java1_21_11, 1, [0.5, 64., 0.5], &facts);
        assert_eq!(session.body.scale, 2.0);
        assert_eq!(
            session.body.bounds[3] - session.body.bounds[0],
            f64::from(0.6f32 * 2.0)
        );
        session.observe_scale(2.0, 10);
        assert_eq!(session.scale_change_sequence, None);
        session.observe_scale(1.0, 11);
        session.observe_scale(2.0, 12);
        assert_eq!(session.scale_change_sequence, Some(11));
        session.status = ControlStatus::Stopped {
            reason: "scale origin changed".into(),
        };
        let fresh = session.resume(2, [0.; 2], &facts);
        assert_eq!(fresh.scale_change_sequence, None);
        assert_eq!(fresh.body.scale, 2.0);
        facts.environment.player_scale = Some(0.5);
        let smaller = session.resume(3, [0.; 2], &facts);
        assert_eq!(smaller.body.scale, 0.5);
        assert_eq!(
            smaller.body.bounds[4] - smaller.body.bounds[1],
            f64::from(1.8f32 * 0.5)
        );
        assert_eq!(smaller.body.velocity, session.body.velocity);
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
    fn every_received_correction_is_logged_with_submission_and_delta() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let start = received(Some((5, [0.5, 64.0, 0.5], None)), None);
            let mut session = ControlSession::new(version, 3, [0.5, 64.0, 0.5], &start);
            let ledger = std::sync::Arc::new(ControlCorrectionLedger::default());
            let stamp = crate::client::SessionStamp {
                version,
                connection_id: 7,
                world_generation: 2,
            };
            session.correction_source = Some(ControlCorrectionSource {
                ledger: ledger.clone(),
                session: stamp,
            });
            let mut blocks = world(false);
            session.controls.forward = 1;
            let output = session.step(&start, &mut blocks).unwrap().unwrap();
            session.dispatched(&output);
            let submitted = ControlSubmission {
                model_tick: session.tick,
                position: output.position,
            };
            // A stale or start-boundary receipt is never a correction.
            session.observe_pose(5, [9.0, 64.0, 9.0], None);
            assert_eq!(ledger.after(0).unwrap().corrections, vec![]);
            // Two packets between ticks: latest-only Received.pose collapses
            // them, the receive-boundary log keeps both in order.
            session.observe_pose(8, [0.5, 64.0, 0.5], Some([0.0; 3]));
            session.observe_pose(9, [2.5, 65.0, 0.5], None);
            session.observe_pose(9, [2.5, 65.0, 0.5], None);
            let latest = received(Some((9, [2.5, 65.0, 0.5], None)), None);
            session.step(&latest, &mut blocks).unwrap();
            assert_eq!(session.corrections, 1, "applied count is unchanged");
            assert_eq!(session.status, ControlStatus::Running);
            let log = ledger.after(0).unwrap();
            assert_eq!(log.sequence, 2);
            let [first, second] = &log.corrections[..] else {
                panic!("{log:?}");
            };
            assert_eq!((first.sequence, second.sequence), (1, 2));
            let first = &first.correction;
            assert_eq!((first.session, first.session_id), (stamp, 3));
            assert_eq!((first.receive_sequence, first.model_tick), (8, 1));
            assert_eq!(first.submitted, Some(submitted));
            assert_eq!(first.model_position, output.position);
            assert_eq!(first.received_position, [0.5, 64.0, 0.5]);
            assert_eq!(first.received_velocity, Some([0.0; 3]));
            let pushed_back = output.position[2] - 0.5;
            assert!(pushed_back > 0.0);
            assert!((first.delta[2] + pushed_back).abs() < 1e-12);
            assert!(
                (first.distance - first.delta.iter().map(|d| d * d).sum::<f64>().sqrt()).abs()
                    < 1e-12
            );
            let second = &second.correction;
            assert_eq!(second.receive_sequence, 9);
            assert_eq!(second.model_position, [0.5, 64.0, 0.5], "after the first");
            assert_eq!(
                (second.delta, second.distance),
                ([2.0, 1.0, 0.0], 5f64.sqrt())
            );
            assert_eq!(second.received_velocity, None);
            // A pose that reached only Received (no receive hook) is still
            // logged once when applied; it is not logged again by observe.
            let unobserved = received(Some((12, [4.5, 65.0, 0.5], None)), None);
            session.step(&unobserved, &mut blocks).unwrap();
            session.observe_pose(12, [4.5, 65.0, 0.5], None);
            session.step(&unobserved, &mut blocks).unwrap();
            let log = ledger.after(2).unwrap();
            assert_eq!(log.corrections.len(), 1);
            assert_eq!(log.corrections[0].correction.receive_sequence, 12);
            assert_eq!(session.corrections, 2);
            // Stopped owners are never relabelled by later receipts.
            session.status = ControlStatus::Stopped {
                reason: "test".into(),
            };
            session.observe_pose(13, [0.5, 64.0, 0.5], None);
            assert_eq!(ledger.after(0).unwrap().sequence, 3);
        }
    }

    #[test]
    fn correction_log_reports_eviction_and_future_cursor() {
        let ledger = ControlCorrectionLedger::default();
        let stamp = crate::client::SessionStamp {
            version: MinecraftVersion::Java1_21_11,
            connection_id: 1,
            world_generation: 1,
        };
        for sequence in 0..MAX_RETAINED_CONTROL_CHANGES as u64 + 3 {
            ledger.record(ControlCorrection {
                session: stamp,
                session_id: 1,
                receive_sequence: sequence,
                model_tick: 0,
                submitted: None,
                model_position: [0.0; 3],
                received_position: [0.0; 3],
                delta: [0.0; 3],
                distance: 0.0,
                received_velocity: None,
            });
        }
        let error = ledger.after(0).unwrap_err();
        assert_eq!(error.kind(), crate::ErrorKind::State);
        let gap = error
            .diagnostic()
            .downcast_ref::<ControlChangeGap>()
            .copied()
            .unwrap();
        assert_eq!((gap.missed, gap.resume_after, gap.sequence), (3, 3, 259));
        let retained = ledger.after(gap.resume_after).unwrap();
        assert_eq!(retained.corrections.len(), MAX_RETAINED_CONTROL_CHANGES);
        assert_eq!(retained.corrections[0].correction.receive_sequence, 3);
        assert_eq!(
            ledger.after(260).unwrap_err().kind(),
            crate::ErrorKind::InvalidInput
        );
        assert_eq!(ledger.after(259).unwrap().corrections, vec![]);
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
    fn unqualified_spawn_keeps_control_and_received_knockback_alive() {
        use crate::client::{
            SessionStamp,
            entity::{NativeSpawn, NativeSpawnMotion, SpawnLedger, owned_contacts::OwnedContacts},
        };
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let mut ledger = SpawnLedger::default();
            ledger.contact_local_player(99);
            ledger.history_context(version, 0, 1);
            let stamp = SessionStamp {
                version,
                connection_id: 1,
                world_generation: 0,
            };
            let start = received(Some((5, [0.5, 64., 0.5], Some([0.; 3]))), None);
            let mut control = ControlSession::new(version, 8, [0.5, 64., 0.5], &start);
            control.contacts = Some(OwnedContacts::new(stamp, 8, 99, &ledger, 1).unwrap());
            let output = control.step(&start, &mut world(false)).unwrap().unwrap();
            control.dispatched(&output);
            ledger.history_context(version, 0, 2);
            ledger
                .insert(
                    version,
                    NativeSpawn {
                        id: 8,
                        uuid: None,
                        type_id: None,
                        dedicated_type_name: Some("horse"),
                        position: [600., 64., 0.5],
                        living: Some(true),
                    },
                    2,
                    16,
                )
                .unwrap();
            ledger.initialize_motion(
                8,
                NativeSpawnMotion {
                    position: Some([600., 64., 0.5]),
                    rotation: Some([0.; 2]),
                    ..Default::default()
                },
                2,
            );
            control
                .contacts
                .as_mut()
                .unwrap()
                .refresh(8, &ledger, 2)
                .unwrap();
            let knockback = received(
                Some((5, [0.5, 64., 0.5], Some([0.; 3]))),
                Some((6, [0.25, 0.42, 0.])),
            );
            let output = control
                .step(&knockback, &mut world(false))
                .unwrap()
                .unwrap();
            assert!(control.body.position[0] > 0.5);
            assert!(control.body.position[1] > 64.);
            assert_eq!(control.velocity_updates, 1);
            assert!(control.contacts.is_some());
            assert!(control.entity_contacts.as_ref().unwrap().bodies.is_empty());
            control.dispatched(&output);
            assert_eq!(control.dispatched_ticks, 2);
        }
    }

    #[test]
    fn bounded_contact_passes_through_intersecting_sweep_before_dispatch() {
        use crate::client::{
            SessionStamp,
            entity::{NativeSpawn, NativeSpawnMotion, SpawnLedger, owned_contacts::OwnedContacts},
        };
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let mut ledger = SpawnLedger::default();
            ledger.contact_local_player(99);
            ledger.history_context(version, 0, 1);
            let stamp = SessionStamp {
                version,
                connection_id: 1,
                world_generation: 0,
            };
            let start = received(Some((5, [0.5, 64., 0.5], Some([0.; 3]))), None);
            let mut control = ControlSession::new(version, 8, [0.5, 64., 0.5], &start);
            control.contacts = Some(OwnedContacts::new(stamp, 8, 99, &ledger, 1).unwrap());
            let output = control.step(&start, &mut world(false)).unwrap().unwrap();
            control.dispatched(&output);
            ledger.history_context(version, 0, 2);
            ledger
                .insert(
                    version,
                    NativeSpawn {
                        id: 8,
                        uuid: None,
                        type_id: None,
                        dedicated_type_name: Some("spider"),
                        position: [8., 64., 0.5],
                        living: Some(true),
                    },
                    2,
                    16,
                )
                .unwrap();
            ledger.initialize_motion(
                8,
                NativeSpawnMotion {
                    position: Some([8., 64., 0.5]),
                    rotation: Some([0.; 2]),
                    ..Default::default()
                },
                2,
            );
            control
                .contacts
                .as_mut()
                .unwrap()
                .refresh(8, &ledger, 2)
                .unwrap();
            // A stationary tick is unrelated and may produce a frame.
            let output = control.step(&start, &mut world(false)).unwrap().unwrap();
            control.dispatched(&output);
            // Intersecting an unqualified envelope keeps control dispatchable.
            control.body.velocity = [10., 0., 0.];
            let output = control.step(&start, &mut world(false)).unwrap().unwrap();
            assert!(control.body.position[0] > 8.);
            assert!(control.contacts.is_some());
            assert!(control.entity_contacts.as_ref().unwrap().bodies.is_empty());
            control.dispatched(&output);
            assert_eq!(control.dispatched_ticks, 3);
        }
    }

    #[test]
    fn best_effort_unknown_walk_is_bounded_and_same_pose_cannot_replenish_it() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let start = received(Some((5, [0.5, 64.0, 0.5], Some([0.0; 3]))), None);
            let mut session = ControlSession::new(version, 7, [0.5, 64.0, 0.5], &start);
            session.best_effort_contacts = true;
            session.controls.forward = 1;
            session.controls.strafe = 1;
            session.controls.jump = true;
            let mut unknown = |_| Err(crate::error::state_error("unloaded section"));
            for _ in 0..MAX_UNPROVEN_WALK_TICKS {
                let before = session.body.position;
                let output = session.step(&start, &mut unknown).unwrap().unwrap();
                assert!(!output.on_ground);
                assert!(
                    (output.position[0] - before[0]).hypot(output.position[2] - before[2])
                        <= 0.100000001
                );
                assert_eq!(output.position[1], before[1]); // No invented floor or jump.
                session.dispatched(&output);
            }
            let position = session.body.position;
            for _ in 0..30 {
                assert_eq!(session.step(&start, &mut unknown).unwrap(), None);
            }
            assert_eq!(session.body.position, position);
            assert_eq!(session.dispatched_ticks, 20); // Paused attempts are not writes.
            assert!(
                session
                    .walking_diagnostics()
                    .terrain
                    .unwrap()
                    .waiting_for_evidence
            );
            let mut next = session.resume(8, [0.0; 2], &start);
            next.best_effort_contacts = true;
            next.inherit_terrain_budget(&session);
            assert_eq!(next.step(&start, &mut unknown).unwrap(), None);
            let corrected = received(Some((6, [2.5, 64.0, 0.5], Some([0.0; 3]))), None);
            let output = session.step(&corrected, &mut unknown).unwrap().unwrap();
            assert!((output.position[0] - 2.5).abs() <= 0.1);
            assert!((output.position[2] - 0.5).abs() <= 0.1);
            assert_eq!(session.id, 7);
            assert_eq!(session.corrections, 1);
            session.dispatched(&output);
            assert_eq!(session.dispatched_ticks, 21);
            assert_eq!(
                session
                    .walking_diagnostics()
                    .terrain
                    .unwrap()
                    .unproven_ticks,
                1
            );
            assert!(
                session
                    .step(&corrected, &mut world(false))
                    .unwrap()
                    .is_some()
            );
            assert_eq!(
                session
                    .walking_diagnostics()
                    .terrain
                    .unwrap()
                    .unproven_ticks,
                0
            );
        }
    }

    #[test]
    fn best_effort_unmodeled_shape_and_missing_collision_warn_but_inputs_do_not() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            for name in ["minecraft:moving_piston", "minecraft:unregistered_shape"] {
                let start = received(Some((5, [0.5, 64.0, 0.5], Some([0.0; 3]))), None);
                let mut session = ControlSession::new(version, 1, [0.5, 64.0, 0.5], &start);
                session.best_effort_contacts = true;
                session.controls.forward = 1;
                let mut terrain = |_: [i32; 3]| {
                    Ok(NativeBlockState {
                        name: name.into(),
                        properties: Default::default(),
                    })
                };
                assert!(session.step(&start, &mut terrain).unwrap().is_some());
                assert_eq!(session.status, ControlStatus::Running);
                assert!(session.walking_diagnostics().terrain.is_some());
                assert!(session.movement_refusal.is_none());
            }
            for invalid in 0..4 {
                let mut start = received(Some((5, [0.5, 64.0, 0.5], Some([0.0; 3]))), None);
                let mut session = ControlSession::new(version, 1, [0.5, 64.0, 0.5], &start);
                session.best_effort_contacts = true;
                match invalid {
                    0 => session.controls.yaw = f32::NAN,
                    1 => session.controls.forward = 2,
                    2 => start.environment.equipment_unavailable = true,
                    _ => start.environment.player_scale = None,
                }
                assert_eq!(session.step(&start, &mut world(false)).unwrap(), None);
                assert!(session.walking_diagnostics().terrain.is_none());
                assert_eq!(session.dispatched_ticks, 0);
            }
        }
    }

    #[test]
    fn best_effort_finite_route_retains_progress_across_correction_and_has_deadline() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let start = received(Some((5, [0.5, 64.0, 0.5], Some([0.0; 3]))), None);
            let mut session = ControlSession::new(version, 7, [0.5, 64.0, 0.5], &start);
            session.best_effort_contacts = true;
            let path: Vec<_> = [0.0, 90.0, 180.0]
                .into_iter()
                .map(|yaw| super::super::survival::SurvivalControl {
                    yaw,
                    input: super::super::survival::SurvivalInput {
                        forward: 1,
                        ..Default::default()
                    },
                })
                .collect();
            session.set_path(&path);
            let mut unknown = |_| Err(crate::error::state_error("missing collision data"));
            for (i, control) in path.iter().enumerate() {
                let r = if i == 1 {
                    received(Some((6, [2.5, 64.0, 0.5], Some([0.0; 3]))), None)
                } else {
                    received(Some((5, [0.5, 64.0, 0.5], Some([0.0; 3]))), None)
                };
                let output = session.step(&r, &mut unknown).unwrap().unwrap();
                assert_eq!(output.rotation[0], control.yaw);
                session.dispatched(&output);
                assert_eq!(session.path_complete(), i == 2);
                assert_eq!(
                    session.walking_diagnostics().path.unwrap().dispatched_ticks,
                    (i + 1) as u16
                );
            }
            assert_eq!(session.corrections, 1);
            let mut expired = ControlSession::new(version, 8, [0.5, 64.0, 0.5], &start);
            expired.set_path(&path);
            expired.path_deadline = Some(std::time::Instant::now());
            assert_eq!(
                expired.step(&start, &mut unknown).unwrap_err().kind(),
                crate::ErrorKind::Timeout
            );
            assert_eq!(expired.dispatched_ticks, 0);
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
    fn glide_session() -> ControlSession {
        use crate::client::{ItemData, ItemStack, SlotKnowledge, registry::Registry};
        let version = MinecraftVersion::Java1_21_11;
        let native = Registry::for_version(version)
            .item("minecraft:elytra")
            .unwrap();
        let equipment = super::super::glide::equipment(Some(&crate::client::received(
            SlotKnowledge::Item {
                item: ItemStack {
                    id: native.id,
                    name: native.name.clone(),
                    count: 1,
                    data: ItemData::Default,
                },
            },
            10,
        )))
        .unwrap();
        let mut s = ControlSession::new(
            version,
            1,
            [0.5, 78., 0.5],
            &received(Some((11, [0.5, 78., 0.5], Some([0.; 3]))), None),
        );
        s.kind = ControlKind::Glide;
        s.glide = Some(super::super::glide::GlideRecord {
            after_sequence: 11,
            dispatched: false,
            accepted_sequence: None,
            ended_sequence: None,
            actual: None,
            equipment,
            equipment_invalidated_sequence: None,
            waiting_ticks: 0,
        });
        s
    }
    #[test]
    fn glide_request_write_never_substitutes_for_an_actual_flag() {
        let mut s = glide_session();
        let body = s.body.clone();
        let r = received(Some((11, [0.5, 78., 0.5], Some([0.; 3]))), None);
        s.observe_gliding(true, 12);
        assert_eq!(s.glide.as_ref().unwrap().accepted_sequence, None);
        s.glide.as_mut().unwrap().dispatched = true;
        assert!(s.step(&r, &mut world(false)).is_err());
        assert_eq!(s.body, body);
        s.observe_gliding(true, 11);
        assert_eq!(s.glide.as_ref().unwrap().accepted_sequence, None);
        s.observe_gliding(true, 13);
        assert_eq!(s.glide.as_ref().unwrap().accepted_sequence, Some(13));
        assert!(!s.glide_waiting());
        assert_eq!(s.body.pose, Pose::FallFlying);
        assert!(s.step(&r, &mut world(false)).unwrap().is_some());
    }
    #[test]
    fn actual_glide_end_and_equipment_loss_latch_before_restoration() {
        let mut s = glide_session();
        s.glide.as_mut().unwrap().dispatched = true;
        s.observe_gliding(true, 12);
        s.observe_gliding(false, 13);
        s.observe_gliding(true, 14);
        assert_eq!(s.glide.as_ref().unwrap().ended_sequence, Some(13));
        assert_eq!(
            s.glide_stop_reason(&received(Some((11, [0.5, 78., 0.5], Some([0.; 3]))), None)),
            Some("actual glide ended")
        );
        s.observe_glider_equipment(false, 15);
        s.observe_glider_equipment(true, 16);
        assert_eq!(
            s.glide.as_ref().unwrap().equipment_invalidated_sequence,
            Some(15)
        );
        let snapshot = s.body.clone();
        assert!(s.step(&received(None, None), &mut world(false)).is_err());
        assert_eq!(s.body, snapshot);
    }
    #[test]
    fn unconfirmed_glide_has_bounded_wait_and_correction_ends_its_origin() {
        let mut s = glide_session();
        s.glide.as_mut().unwrap().dispatched = true;
        for _ in 0..40 {
            assert!(s.glide_waiting());
        }
        assert!(
            s.glide_stop_reason(&received(Some((11, [0.5, 78., 0.5], Some([0.; 3]))), None))
                .unwrap()
                .contains("unconfirmed")
        );
        let mut s = glide_session();
        s.glide.as_mut().unwrap().dispatched = true;
        s.observe_gliding(true, 12);
        assert_eq!(
            s.glide_stop_reason(&received(Some((13, [0.5, 79., 0.5], Some([0.; 3]))), None)),
            Some("glide correction requires a new owner")
        );
        s.status = ControlStatus::Stopped {
            reason: "disconnected".into(),
        };
        let old = s.record();
        s.observe_gliding(false, 14);
        s.observe_glider_equipment(false, 15);
        assert_eq!(s.record(), old);
    }

    #[test]
    fn glide_unknown_terrain_ends_origin_without_resuming_after_recovery() {
        let mut s = glide_session();
        s.glide.as_mut().unwrap().dispatched = true;
        s.observe_gliding(true, 12);
        assert!(!s.glide_waiting());
        let r = received(Some((11, [0.5, 78., 0.5], Some([0.; 3]))), None);
        let body = s.body.clone();
        assert!(
            s.step(&r, &mut |_| Err(crate::client::registry::invalid(
                "cell unreceived"
            )))
            .is_err()
        );
        assert_eq!(s.body, body);
        assert!(matches!(s.status, ControlStatus::Stopped { .. }));
        let retained = s.record();
        assert!(s.step(&r, &mut world(false)).unwrap().is_none());
        assert_eq!(s.record(), retained);
    }
}
