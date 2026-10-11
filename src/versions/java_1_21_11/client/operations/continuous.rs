//! Continuous control on 1.21.11: one Client-owned task ticks the shared engine
//! every 50 ms and writes the client's input, sprint command and position.
use super::geometry::GeometryView;
use super::*;
use crate::client::control::{ControlRecord, ControlSession, ControlStatus, Controls, Received};
use crate::client::physics::Environment;
use crate::protocol::put_varint;
use std::time::Duration;

fn attribute(name: &str) -> Option<i32> {
    crate::client::physics::blocks::table(crate::MinecraftVersion::Java1_21_11)
        .attribute_ids
        .get(name)
        .copied()
}

/// The connection's continuous control session and its task identity.
#[derive(Default)]
pub(in crate::versions::java_1_21_11::client) struct ContinuousControl {
    pub session: Option<ControlSession>,
    next_id: u64,
    generation: Option<u64>,
    model_attempt: Option<u64>,
    // Complete local disable wins only over older abilities in the same world.
    flight_disabled: Option<(u64, u64)>,
}

impl ContinuousControl {
    pub(super) fn observe_abilities(&mut self, flags: u8, sequence: u64) {
        if let Some(session) = &mut self.session {
            session.observe_abilities(flags, sequence);
        }
    }
    pub(super) fn observe_mode(&mut self, mode: GameMode, sequence: u64) {
        if let Some(session) = &mut self.session {
            session.observe_mode(mode, sequence);
        }
    }
    /// Whether a session currently owns the player's movement.
    pub(in crate::versions::java_1_21_11::client) fn active(&self) -> bool {
        self.session.as_ref().is_some_and(ControlSession::running)
    }

    #[cfg(test)]
    pub(in crate::versions::java_1_21_11::client) fn seed_session(
        &mut self,
        session: ControlSession,
        generation: u64,
    ) {
        self.session = Some(session);
        self.generation = Some(generation);
    }
}

/// A release was sent: the running session stops applying the item-use slowdown.
pub(in crate::versions::java_1_21_11::client) fn item_released(state: &mut State) {
    let received = received(state);
    if let Some(session) = state.control.session.as_mut().filter(|s| s.running()) {
        session.item_released(&received);
    }
}

fn effect(state: &State, name: &str) -> Option<i32> {
    let id = *crate::client::physics::blocks::table(crate::MinecraftVersion::Java1_21_11)
        .effect_ids
        .get(name)?;
    state
        .operations
        .local_player
        .effect_updates
        .get(&id)
        .map(|e| e.amplifier)
}

fn value(v: &Option<AttributeValue>, default: f64) -> f64 {
    v.as_ref().map_or(default, |a| a.value)
}

/// Received facts the engine needs, from the own-player projection.
fn received(state: &State) -> Received {
    let p = &state.operations.local_player;
    let mut environment = Environment::defaults(crate::MinecraftVersion::Java1_21_11);
    if let Some(speed) =
        attribute("minecraft:movement_speed").and_then(|id| p.received_attributes.get(&id))
    {
        environment.movement_speed_base = speed.base;
        environment.movement_speed_modifiers = speed
            .modifiers
            .iter()
            .filter(|m| m.id != "minecraft:sprinting")
            .cloned()
            .collect();
    }
    environment.jump_strength = value(&p.jump_strength, environment.jump_strength);
    environment.step_height = value(&p.step_height, environment.step_height);
    environment.gravity = value(&p.gravity, environment.gravity);
    environment.sneaking_speed = value(&p.sneaking_speed, environment.sneaking_speed);
    environment.movement_efficiency = value(&p.movement_efficiency, 0.0);
    if let Some(a) = attribute("minecraft:water_movement_efficiency")
        .and_then(|id| p.received_attributes.get(&id))
    {
        let modifiers: Vec<_> = a.modifiers.iter().collect();
        let v = crate::client::physics::attribute_value(
            crate::MinecraftVersion::Java1_21_11,
            a.base,
            &modifiers,
        );
        environment.water_movement_efficiency = v.clamp(0.0, 1.0);
    }
    environment.jump_boost = effect(state, "minecraft:jump_boost").map(|a| a as u8);
    environment.slow_falling = effect(state, "minecraft:slow_falling").is_some();
    environment.levitation = effect(state, "minecraft:levitation").is_some();
    environment.blindness = effect(state, "minecraft:blindness").is_some();
    environment.weaving = effect(state, "minecraft:weaving").is_some();
    environment.dolphins_grace = effect(state, "minecraft:dolphins_grace").is_some();
    environment.food_level = p.health.as_ref().map_or(20, |h| h.food);
    environment.may_fly = state.operations.abilities.is_some_and(|a| a & 4 != 0);
    environment.fast_lava = state
        .world
        .dimension
        .as_ref()
        .is_some_and(|d| d.0 == "minecraft:the_nether");
    let using_item = super::common_player_in_state(state, 0, false)
        .ok()
        .and_then(|player| {
            crate::client::item_use::received_use(
                player.using_item.as_ref(),
                player.selected_hotbar.as_ref().map(|s| s.value),
                &player.inventory.slots,
            )
        });
    let abilities_sequence = state.context.abilities.as_ref().and_then(|a| {
        environment.may_fly = a.value.may_fly();
        environment.ability_flying_speed = Some(a.value.flying_speed);
        match a.source {
            crate::client::ValueSource::Received { sequence } => Some(sequence),
            _ => None,
        }
    });
    Received {
        abilities_sequence,
        environment,
        using_item,
        pose: state
            .motion
            .received_pose
            .as_ref()
            .map(|p| (p.receive_sequence, p.position, p.velocity)),
        velocity: p.velocity.as_ref().map(|v| (v.receive_sequence, v.value)),
    }
}

/// Why the session must end, from facts outside the engine's scope.
fn stop_reason(
    state: &State,
    generation: u64,
    mode: GameMode,
    kind: crate::client::control::ControlKind,
) -> Option<&'static str> {
    if state.loading.generation != generation {
        Some("world changed (respawn, dimension or reconfiguration)")
    } else if state.operations.game_mode != Some(mode)
        || state
            .control
            .session
            .as_ref()
            .is_some_and(|s| s.running() && s.mode_change_sequence.is_some())
    {
        Some("game mode changed")
    } else if (kind == crate::client::control::ControlKind::Ground
        && (state.operations.requested_flying
            || (state.operations.abilities.is_some_and(|a| a & 2 != 0)
                && !state.control.flight_disabled.is_some_and(|(world, after)| {
                    world == generation
                        && state
                            .operations
                            .abilities_sequence
                            .is_some_and(|sequence| sequence <= after)
                }))))
        || state
            .control
            .session
            .as_ref()
            .is_some_and(|s| s.running() && s.flight_change_sequence.is_some())
    {
        Some("flying")
    } else if kind == crate::client::control::ControlKind::Flight
        && crate::client::control::flight_admission(
            state.context.abilities.as_ref(),
            state.operations.requested_flying,
            state.control.flight_disabled.is_some_and(|(world, after)| {
                world == generation
                    && state
                        .operations
                        .abilities_sequence
                        .is_some_and(|sequence| sequence <= after)
            }),
        )
        .is_some()
    {
        crate::client::control::flight_admission(
            state.context.abilities.as_ref(),
            state.operations.requested_flying,
            state.control.flight_disabled.is_some_and(|(world, after)| {
                world == generation
                    && state
                        .operations
                        .abilities_sequence
                        .is_some_and(|sequence| sequence <= after)
            }),
        )
    } else if state.operations.local_player.scale.map(|s| s.value) != Some(1.0)
        || state
            .control
            .session
            .as_ref()
            .is_some_and(|s| s.running() && s.scale_change_sequence.is_some())
    {
        Some("unmodeled player scale")
    } else if state.vehicles.motion_interrupted()
        || state.operations.local_player.motion_interruption.is_some()
    {
        Some("vehicle or explosion motion outside the engine")
    } else if state
        .operations
        .local_player
        .health
        .as_ref()
        .is_some_and(|h| h.health <= 0.0)
    {
        Some("player died")
    } else {
        None
    }
}

fn sprint_command(entity: i32, start: bool) -> Vec<u8> {
    let mut payload = Vec::with_capacity(8);
    put_varint(&mut payload, entity);
    // ServerboundPlayerCommandPacket.Action: START_SPRINTING 1, STOP_SPRINTING 2.
    put_varint(&mut payload, if start { 1 } else { 2 });
    put_varint(&mut payload, 0);
    payload
}

fn position_payload(
    position: [f64; 3],
    rotation: [f32; 2],
    on_ground: bool,
    horizontal: bool,
) -> Vec<u8> {
    let mut payload = Vec::with_capacity(33);
    for v in position {
        payload.extend(v.to_be_bytes());
    }
    for v in rotation {
        payload.extend(v.to_be_bytes());
    }
    payload.push(u8::from(on_ground) | (u8::from(horizontal) << 1));
    payload
}

impl Operations {
    async fn write_release(&self, session: &ControlSession, entity: Option<i32>) -> Result<()> {
        let (_, sprint, input) = session.release();
        if let Some(bits) = input {
            self.bot
                .session
                .send(ids::play_serverbound::PLAYER_INPUT, &[bits])
                .await?;
        }
        if let (Some(false), Some(entity)) = (sprint, entity) {
            self.bot
                .session
                .send(
                    ids::play_serverbound::ENTITY_ACTION,
                    &sprint_command(entity, false),
                )
                .await?;
        }
        Ok(())
    }

    async fn stop_control_owned(
        &self,
        id: u64,
        mode: Option<GameMode>,
    ) -> Result<Option<ControlRecord>> {
        let mut state = self.bot.session.state.lock().await;
        let entity = state.operations.local_player.entity_id;
        let Some(mut session) = state.control.session.take() else {
            return Ok(None);
        };
        if mode.is_some_and(|mode| session.mode != mode) {
            state.control.session = Some(session);
            return Err(invalid("selected control belongs to a different mode"));
        }
        if session.id != id {
            state.control.session = Some(session);
            return Ok(None);
        }
        let old_world = state.control.generation != Some(state.loading.generation);
        let release = if !old_world && session.running() && self.ready(&state).is_ok() {
            self.write_release(&session, entity).await
        } else {
            Ok(())
        };
        if session.running() {
            session.status = ControlStatus::Stopped {
                reason: match &release {
                    Ok(()) if old_world => "world changed (respawn or dimension)".into(),
                    Ok(()) => "stopped by request".into(),
                    Err(e) => format!("stopped; release write failed: {e}"),
                },
            };
        }
        let record = session.record();
        state.control.session = Some(session);
        self.bot.session.changed.notify_waiters();
        release.map(|()| Some(record))
    }

    async fn run_control(
        &self,
        id: u64,
        generation: u64,
        mode: GameMode,
        kind: crate::client::control::ControlKind,
    ) {
        let mut interval = tokio::time::interval(Duration::from_millis(50));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            let mut state = self.bot.session.state.lock().await;
            if state
                .control
                .session
                .as_ref()
                .is_none_or(|s| s.id != id || !s.running())
            {
                return;
            }
            let stop = match self.ready(&state) {
                Err(e) => Some(e.to_string()),
                Ok(()) => stop_reason(&state, generation, mode, kind).map(str::to_owned),
            };
            if let Some(reason) = stop {
                // Send releases only in the original, ready world.
                let release =
                    if self.ready(&state).is_ok() && state.loading.generation == generation {
                        self.write_release(
                            state.control.session.as_ref().unwrap(),
                            state.operations.local_player.entity_id,
                        )
                        .await
                    } else {
                        Ok(())
                    };
                state.control.session.as_mut().unwrap().status = ControlStatus::Stopped {
                    reason: match release {
                        Ok(()) => reason,
                        Err(error) => format!("{reason}; release write failed: {error}"),
                    },
                };
                self.bot.session.changed.notify_waiters();
                return;
            }
            let tick = self.bot.session.started.elapsed().as_millis() as u64 / 50;
            let State {
                reconstruction,
                world,
                ..
            } = &mut *state;
            reconstruction.advance(world, tick);
            let mut session = state.control.session.take().unwrap();
            let received = received(&state);
            let result = if state.reconstruction.issue.is_some()
                || !state.reconstruction.recovery_chunks.is_empty()
            {
                session.status = ControlStatus::Paused {
                    reason: "piston reconstruction incomplete".into(),
                };
                Ok(None)
            } else {
                session.step(&received, &mut |p| state.block(p))
            };
            let output = match result {
                Ok(output) => output,
                Err(error) => {
                    session.status = ControlStatus::Stopped {
                        reason: error.to_string(),
                    };
                    state.control.session = Some(session);
                    self.bot.session.changed.notify_waiters();
                    return;
                }
            };
            let Some(output) = output else {
                state.control.session = Some(session);
                continue;
            };
            let entity = state.operations.local_player.entity_id;
            let sequence = state.sequence;
            if state
                .motion
                .begin(generation, sequence, output.position, output.rotation)
                .is_err()
            {
                session.status = ControlStatus::Stopped {
                    reason: "position attempt IDs exhausted".into(),
                };
                state.control.session = Some(session);
                return;
            }
            let landed =
                kind == crate::client::control::ControlKind::Flight && session.body.flight_landed;
            if landed {
                session.flight_disable = Some(crate::client::control::FlightDisable {
                    after_sequence: state.sequence,
                    dispatched: false,
                });
            }
            let written: Result<()> = async {
                if landed {
                    self.bot
                        .session
                        .send(ids::play_serverbound::ABILITIES, &[0])
                        .await?;
                    state.operations.requested_flying = false;
                    session.flight_disable.as_mut().unwrap().dispatched = true;
                    state.control.flight_disabled = Some((generation, state.sequence));
                }
                if let Some(bits) = output.input {
                    self.bot
                        .session
                        .send(ids::play_serverbound::PLAYER_INPUT, &[bits])
                        .await?;
                }
                if let (Some(start), Some(entity)) = (output.sprint, entity) {
                    self.bot
                        .session
                        .send(
                            ids::play_serverbound::ENTITY_ACTION,
                            &sprint_command(entity, start),
                        )
                        .await?;
                }
                let payload = position_payload(
                    output.position,
                    output.rotation,
                    output.on_ground,
                    output.horizontal_collision,
                );
                self.bot
                    .session
                    .send(ids::play_serverbound::POSITION_LOOK, &payload)
                    .await
            }
            .await;
            match written {
                Ok(()) => {
                    state.position = Some(output.position);
                    state.rotation = output.rotation;
                    state.motion.dispatched();
                    state.motion.on_ground = Some(output.on_ground);
                    session.dispatched(&output);
                    if landed {
                        session.status = ControlStatus::Stopped {
                            reason: "creative flight model landed".into(),
                        };
                        if let Err(error) = self.write_release(&session, entity).await {
                            session.status = ControlStatus::Stopped {
                                reason: format!(
                                    "creative flight model landed; release write failed: {error}"
                                ),
                            };
                        }
                    }
                    state.control.model_attempt =
                        state.motion.last_submission.as_ref().map(|s| s.attempt_id);
                    state.control.session = Some(session);
                    self.bot.session.changed.notify_waiters();
                    if landed {
                        return;
                    }
                }
                Err(error) => {
                    session.status = ControlStatus::Stopped {
                        reason: format!("write failed; inspect before reuse: {error}"),
                    };
                    state.control.session = Some(session);
                    return;
                }
            }
        }
    }
}

impl Operations {
    async fn start_control_inner(
        &self,
        mode: GameMode,
        initial_flight_controls: Option<Controls>,
    ) -> Result<ControlRecord> {
        let kind = if initial_flight_controls.is_some() {
            crate::client::control::ControlKind::Flight
        } else {
            crate::client::control::ControlKind::Ground
        };
        match kind {
            crate::client::control::ControlKind::Ground => {
                crate::client::control::validate_ground_mode(mode)?
            }
            crate::client::control::ControlKind::Flight
                if matches!(mode, GameMode::Creative | GameMode::Spectator) => {}
            _ => return Err(invalid("held flight requires Creative or Spectator mode")),
        }
        if let Some(controls) = initial_flight_controls {
            crate::client::operations::validate_rotation([controls.yaw, controls.pitch])?;
            if !(-1..=1).contains(&controls.forward) || !(-1..=1).contains(&controls.strafe) {
                return Err(invalid("controls out of range"));
            }
        }
        let mut state = self.bot.session.state.lock().await;
        self.ready(&state)?;
        if state.operations.game_mode != Some(mode) {
            return Err(invalid(
                "continuous control requires matching received mode",
            ));
        }

        if state.control.active() {
            return Err(invalid("a control session is already running"));
        }
        if state
            .survival_motion
            .as_ref()
            .is_some_and(|r| r.status == SurvivalMotionStatus::Running)
        {
            return Err(invalid("a finite motion run is in progress"));
        }
        let generation = state.loading.generation;
        if let Some(reason) = stop_reason(&state, generation, mode, kind) {
            return Err(invalid(reason));
        }
        let Some(position) = state.position else {
            return Err(invalid("continuous control requires a received position"));
        };
        if state
            .motion
            .received_pose
            .as_ref()
            .is_none_or(|p| p.generation != generation)
        {
            return Err(invalid("continuous control requires a received pose"));
        }
        let received = received(&state);
        if matches!(
            state.motion.position_basis,
            super::super::motion::PositionBasis::Unavailable
                | super::super::motion::PositionBasis::PendingSubmission
        ) {
            return Err(invalid(
                "continuous control requires a resolved position model",
            ));
        }
        let previous = state
            .control
            .session
            .as_ref()
            .filter(|_| state.control.generation == Some(generation));
        let attempt = state.motion.last_submission.as_ref().map(|s| s.attempt_id);
        if let Some(previous) = previous {
            if (previous.body.position != position || attempt != state.control.model_attempt)
                && !previous.has_new_pose(&received)
            {
                return Err(invalid(
                    "current model momentum unavailable after unrelated local movement",
                ));
            }
        }
        let owned_model = previous.filter(|_| attempt == state.control.model_attempt);
        if owned_model.is_none()
            && state.motion.position_basis == super::super::motion::PositionBasis::Submitted
        {
            return Err(invalid(
                "position was submitted outside this control model; receive a fresh pose first",
            ));
        }
        if owned_model.is_none()
            && received.pose.is_some_and(|(pose_sequence, _, velocity)| {
                velocity.is_none() && received.velocity.is_none_or(|v| v.0 < pose_sequence)
            })
        {
            return Err(invalid("received velocity baseline is unavailable"));
        }
        let id = state.control.next_id + 1;
        let rotation = state.rotation;
        let mut session = if let Some(previous) = owned_model {
            previous.resume(id, rotation, &received)
        } else {
            let mut session = ControlSession::new(
                crate::MinecraftVersion::Java1_21_11,
                id,
                position,
                &received,
            );
            session.controls.yaw = rotation[0];
            session.controls.pitch = rotation[1];
            session
        };
        if let Some(previous) = previous {
            session.inherit_item_release(previous);
        }
        session.mode = mode;
        session.kind = kind;
        if let Some(controls) = initial_flight_controls {
            session.controls = controls;
        }
        state.control.next_id = id;
        state.control.generation = Some(generation);
        state.control.model_attempt = attempt;
        let record = session.record();
        state.control.session = Some(session);
        drop(state);
        let ops = self.clone();
        tokio::spawn(async move { ops.run_control(id, generation, mode, kind).await });
        Ok(record)
    }
}

impl crate::client::adapter::ControlOps for Operations {
    async fn start_control(&self, mode: GameMode) -> Result<ControlRecord> {
        self.start_control_inner(mode, None).await
    }

    async fn start_flight_control(
        &self,
        mode: GameMode,
        controls: Controls,
    ) -> Result<ControlRecord> {
        self.start_control_inner(mode, Some(controls)).await
    }

    async fn set_controls(
        &self,
        mode: GameMode,
        session_id: Option<u64>,
        controls: Controls,
    ) -> Result<ControlRecord> {
        crate::client::operations::validate_rotation([controls.yaw, controls.pitch])?;
        if !(-1..=1).contains(&controls.forward) || !(-1..=1).contains(&controls.strafe) {
            return Err(invalid("controls out of range"));
        }
        let mut state = self.bot.session.state.lock().await;
        self.ready(&state)?;
        if state.operations.game_mode != Some(mode) {
            return Err(invalid("controls require matching received mode"));
        }
        if let Some(reason) = stop_reason(
            &state,
            state.loading.generation,
            mode,
            state
                .control
                .session
                .as_ref()
                .map_or(crate::client::control::ControlKind::Ground, |s| s.kind),
        ) {
            return Err(invalid(reason));
        }
        if state.control.generation != Some(state.loading.generation) {
            return Err(invalid("control belongs to a previous world"));
        }
        match state.control.session.as_mut() {
            Some(session) if session.running() && session.mode == mode => {
                if session_id.is_some_and(|id| session.id != id) {
                    return Err(invalid("selected control session was replaced"));
                }
                session.controls = controls;
                Ok(session.record())
            }
            _ => Err(invalid("no running control session")),
        }
    }

    #[cfg(test)]
    async fn stop_control(&self) -> Result<Option<ControlRecord>> {
        let state = self.bot.session.state.lock().await;
        let Some(session) = state.control.session.as_ref() else {
            return Ok(None);
        };
        let id = session.id;
        drop(state);
        let owner = self.clone();
        tokio::spawn(async move { owner.stop_control_owned(id, None).await })
            .await
            .map_err(|error| invalid(&format!("control stop task failed: {error}")))?
    }

    #[cfg(test)]
    async fn stop_control_for(&self, session_id: u64) -> Result<ControlRecord> {
        let owner = self.clone();
        tokio::spawn(async move { owner.stop_control_owned(session_id, None).await })
            .await
            .map_err(|error| invalid(&format!("control stop task failed: {error}")))??
            .ok_or_else(|| invalid("selected control session is missing or was replaced"))
    }

    async fn stop_control_in_mode(
        &self,
        mode: GameMode,
        session_id: Option<u64>,
    ) -> Result<Option<ControlRecord>> {
        let state = self.bot.session.state.lock().await;
        let Some(session) = state.control.session.as_ref() else {
            return Ok(None);
        };
        if session.mode != mode || session_id.is_some_and(|id| session.id != id) {
            return Err(invalid(
                "selected control was replaced or belongs to a different mode",
            ));
        }
        let id = session.id;
        drop(state);
        let owner = self.clone();
        tokio::spawn(async move { owner.stop_control_owned(id, Some(mode)).await })
            .await
            .map_err(|error| invalid(&format!("control stop task failed: {error}")))?
    }

    async fn request_ground_jump(
        &self,
        mode: GameMode,
        session_id: u64,
    ) -> Result<crate::client::control::GroundJumpRequest> {
        let mut state = self.bot.session.state.lock().await;
        self.ready(&state)?;
        if state.operations.game_mode != Some(mode) {
            return Err(invalid("ground jump requires matching received mode"));
        }
        let generation = state.loading.generation;
        if state.control.generation != Some(generation) {
            return Err(invalid("ground jump control belongs to a previous world"));
        }
        if let Some(reason) = stop_reason(
            &state,
            generation,
            mode,
            state
                .control
                .session
                .as_ref()
                .map_or(crate::client::control::ControlKind::Ground, |s| s.kind),
        ) {
            return Err(invalid(reason));
        }
        state
            .control
            .session
            .as_mut()
            .filter(|s| s.mode == mode)
            .ok_or_else(|| invalid("no running control session in the expected mode"))?
            .request_ground_jump(session_id)
    }

    async fn control_record(&self) -> Result<Option<ControlRecord>> {
        Ok(self
            .bot
            .session
            .state
            .lock()
            .await
            .control
            .session
            .as_ref()
            .map(ControlSession::record))
    }
}

#[cfg(test)]
mod equipment_tests {
    use super::*;
    #[test]
    fn received_water_modifiers_match_original_native_attribute_and_preserve_receipt() {
        let oracle: serde_json::Value = serde_json::from_reader(flate2::read::GzDecoder::new(
            &include_bytes!("../../../../../data/client_api/equipment_movement_oracle.json.gz")[..],
        ))
        .unwrap();
        for name in [
            "modern_water_default",
            "modern_water_mixed",
            "modern_water_clamp",
        ] {
            let scenario = oracle["scenarios"]
                .as_array()
                .unwrap()
                .iter()
                .find(|s| s["name"] == name)
                .unwrap();
            let original = oracle["results"]["1.21.11"]
                .as_array()
                .unwrap()
                .iter()
                .find(|s| s["name"] == name)
                .unwrap();
            let mut state = State {
                phase: super::super::Phase::Play,
                ..Default::default()
            };
            state.operations.local_player = LocalPlayerState::spawned(7);
            let mut packet = vec![7, 1];
            put_varint(
                &mut packet,
                attribute("minecraft:water_movement_efficiency").unwrap(),
            );
            packet.extend(
                scenario["attributes"]["minecraft:water_movement_efficiency"]
                    .as_f64()
                    .unwrap()
                    .to_be_bytes(),
            );
            let mods = scenario["attribute_modifiers"].as_array().unwrap();
            put_varint(&mut packet, mods.len() as i32);
            for m in mods {
                crate::protocol::put_string(&mut packet, m["id"].as_str().unwrap());
                packet.extend(m["amount"].as_f64().unwrap().to_be_bytes());
                packet.push(match m["operation"].as_str().unwrap() {
                    "ADD_VALUE" => 0,
                    "ADD_MULTIPLIED_BASE" => 1,
                    _ => 2,
                });
            }
            state
                .receive(ids::play_clientbound::ENTITY_UPDATE_ATTRIBUTES, &packet, 64)
                .unwrap();
            let receipt = state.operations.local_player.received_attributes.clone();
            let input = received(&state);
            assert_eq!(
                input.environment.water_movement_efficiency,
                original["initial"]["water_movement_efficiency"]
                    .as_str()
                    .unwrap()
                    .parse::<f64>()
                    .unwrap(),
                "{name}"
            );
            assert_eq!(state.operations.local_player.received_attributes, receipt);
        }
    }
}
