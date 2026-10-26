//! Continuous control on 1.16.1. While a session runs, the Bot's own physics
//! loop is paused (`common_motion_pauses_physics`) and one Client-owned task
//! ticks the shared engine every 50 ms instead.
use super::{Bot, MotionState, Vec3};
use crate::client::GameMode;
use crate::client::control::{ControlRecord, ControlSession, ControlStatus, Controls, Received};
use crate::client::physics::{Body, Environment, Modifier, ModifierOperation};
use crate::error::state_error;
use crate::protocol::put_varint;
use crate::{MinecraftVersion, Result};
use std::sync::atomic::Ordering;
use std::time::Duration;

/// The connection's continuous control session.
#[derive(Default)]
pub(crate) struct MotionExecutor {
    pub(in crate::versions) finite: Option<crate::client::motion_executor::FiniteMotion>,
    session: Option<ControlSession>,
    next_id: u64,
    jump_work: Option<crate::client::ledger::WorkGuard>,
    generation: Option<u64>,
    model_tick: Option<u64>,
    flight_disabled: Option<(u64, u64)>,
}

impl MotionExecutor {
    pub(in crate::versions) fn install_finite(
        &mut self,
        finite: crate::client::motion_executor::FiniteMotion,
    ) {
        assert!(!self.active(), "motion executor already owned");
        self.session = None;
        self.finite = Some(finite);
    }

    pub(super) fn observe_gliding(&mut self, value: bool, sequence: u64) {
        if let Some(owner) = &mut self.session {
            owner.observe_gliding(value, sequence);
        }
    }

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
    pub(crate) fn active(&self) -> bool {
        self.finite.is_some() || self.session.as_ref().is_some_and(ControlSession::running)
    }
    /// A release was sent: the running session stops applying the item-use slowdown.
    pub(crate) fn item_released(&mut self, received: &Received) {
        if let Some(session) = self.session.as_mut().filter(|s| s.running()) {
            session.item_released(received);
        }
    }
}

/// A received 1.16.1 modifier in the shared form (UUID text identity).
pub(super) fn legacy_modifier(m: &super::super::survival::AttributeModifier) -> Modifier {
    Modifier {
        id: uuid_text(m.uuid),
        operation: match m.operation {
            0 => ModifierOperation::Addition,
            1 => ModifierOperation::MultiplyBase,
            _ => ModifierOperation::MultiplyTotal,
        },
        amount: m.amount,
    }
}

fn uuid_text(u: [u8; 16]) -> String {
    let h: String = u.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

impl Bot {
    /// Received facts for the engine, from the Bot's own-player projection.
    pub(super) async fn control_received(&self) -> Received {
        let survival = self.survival.read().await;
        let mut environment = Environment::defaults(MinecraftVersion::Java1_16_1);
        if let Some(speed) = survival
            .attributes
            .get("minecraft:generic.movement_speed")
            .or_else(|| survival.attributes.get("generic.movement_speed"))
        {
            environment.movement_speed_base = speed.base;
            let sprint = MinecraftVersion::Java1_16_1
                .table()
                .physics_rules
                .sprint_modifier;
            environment.movement_speed_modifiers = speed
                .modifiers
                .iter()
                .map(legacy_modifier)
                .filter(|m| m.id != sprint)
                .collect();
        }
        let ids = &crate::client::physics::blocks::table(MinecraftVersion::Java1_16_1).effect_ids;
        let effect = |name: &str| {
            ids.get(name)
                .and_then(|id| survival.effects.get(&(*id as i8)))
                .map(|e| e.amplifier)
        };
        environment.jump_boost = effect("minecraft:jump_boost").map(|a| a as u8);
        environment.slow_falling = effect("minecraft:slow_falling").is_some();
        environment.levitation = effect("minecraft:levitation").is_some();
        environment.blindness = effect("minecraft:blindness").is_some();
        environment.dolphins_grace = effect("minecraft:dolphins_grace").is_some();
        environment.food_level = survival.vitals.as_ref().map_or(20, |v| v.food);
        environment.may_fly = survival.flying_allowed;
        environment.fast_lava = survival.dimension.as_deref() == Some("minecraft:the_nether");
        drop(survival);
        let receipts = self.common_receipts.lock().await;
        crate::client::physics::equipment::legacy_boots(
            receipts.inventory.slots.get(8).and_then(Option::as_ref),
            &receipts.registries,
            &mut environment,
        );
        let pose = receipts
            .pose
            .as_ref()
            .map(|p| (p.receive_sequence, p.position, Some([0.0; 3])));
        let using_item = crate::client::item_use::received_use(
            receipts.using_item.as_ref(),
            receipts.selected_hotbar.as_ref().map(|s| s.value),
            &receipts.inventory.slots,
        );
        let abilities_sequence = receipts.context.abilities.as_ref().and_then(|a| {
            environment.may_fly = a.value.may_fly();
            environment.ability_flying_speed = Some(a.value.flying_speed);
            match a.source {
                crate::client::ValueSource::Received { sequence } => Some(sequence),
                _ => None,
            }
        });
        drop(receipts);
        let velocity = *self.own_velocity_receipt.lock().await;
        Received {
            abilities_sequence,
            environment,
            pose,
            velocity,
            using_item,
        }
    }

    async fn glide_equipment(&self) -> Result<crate::client::glide::GliderEquipment> {
        let receipts = self.common_receipts.lock().await;
        crate::client::glide::equipment(receipts.inventory.slots.get(6).and_then(Option::as_ref))
    }
    pub(super) async fn observe_common_glide_equipment(&self, sequence: u64) {
        if self
            .common_control
            .lock()
            .await
            .session
            .as_ref()
            .is_some_and(|s| s.running() && s.glide.is_some())
        {
            let available = self.glide_equipment().await.is_ok();
            if let Some(owner) = &mut self.common_control.lock().await.session {
                owner.observe_glider_equipment(available, sequence);
            }
        }
    }

    async fn control_stop_reason(
        &self,
        generation: u64,
        mode: GameMode,
        kind: crate::client::control::ControlKind,
    ) -> Option<&'static str> {
        let receipts = self.common_receipts.lock().await;
        if receipts.generation != generation {
            return Some("world changed (respawn or dimension)");
        }
        if kind != crate::client::control::ControlKind::Glide
            && receipts.fall_flying.as_ref().is_some_and(|v| v.value)
        {
            return Some("actual Elytra glide requires a separate owner");
        }
        if receipts.vehicles.motion_interrupted() {
            return Some("riding");
        }
        match kind {
            crate::client::control::ControlKind::Ground | crate::client::control::ControlKind::Glide if receipts.requested_flying => {
                return Some("flying");
            }
            crate::client::control::ControlKind::Flight => {
                if let Some(reason) = crate::client::control::flight_admission(
                    receipts.context.abilities.as_ref(),
                    receipts.requested_flying,
                    self.common_control.lock().await.flight_disabled.is_some_and(|(world,after)| world==generation && receipts.context.abilities.as_ref().is_some_and(|a|matches!(a.source,crate::client::ValueSource::Received {sequence} if sequence<=after))),
                ) {
                    return Some(reason);
                }
            }
            _ => {}
        }
        drop(receipts);
        if self
            .common_control
            .lock()
            .await
            .session
            .as_ref()
            .is_some_and(|s| s.running() && s.flight_change_sequence.is_some())
        {
            return Some("flying");
        }
        if self
            .common_control
            .lock()
            .await
            .session
            .as_ref()
            .is_some_and(|s| s.running() && s.mode_change_sequence.is_some())
        {
            return Some("game mode changed");
        }
        let survival = self.survival.read().await;
        if survival
            .game_mode
            .and_then(|id| GameMode::decode(id & 7).ok())
            != Some(mode)
        {
            Some("game mode changed")
        } else if kind != crate::client::control::ControlKind::Flight && survival.flying {
            Some("flying")
        } else if survival.vitals.as_ref().is_some_and(|v| v.health <= 0.0) {
            Some("player died")
        } else {
            None
        }
    }

    async fn control_entity_action(&self, action: i32) -> Result<()> {
        self.control_entity_action_in(action, crate::client::domain::Domain::Strict)
            .await
    }
    async fn control_entity_action_in(
        &self,
        action: i32,
        domain: crate::client::domain::Domain,
    ) -> Result<()> {
        let entity = self
            .player
            .lock()
            .await
            .entity_id
            .ok_or_else(|| state_error("Join Game entity ID is unavailable"))?;
        let mut payload = Vec::new();
        put_varint(&mut payload, entity);
        put_varint(&mut payload, action);
        put_varint(&mut payload, 0);
        self.send_in_domain(domain, 0x1c, &payload).await
    }

    async fn release_control(&self, session: &ControlSession) -> Result<()> {
        let domain = if session.best_effort_contacts {
            crate::client::domain::Domain::Motion
        } else {
            crate::client::domain::Domain::Strict
        };
        let (sneak, sprint, _) = session.release();
        if sprint.is_some() {
            self.control_entity_action_in(4, domain).await?;
        }
        if sneak.is_some() {
            self.control_entity_action_in(1, domain).await?;
        }
        Ok(())
    }

    // The connection owns release writes even if the caller stops awaiting them.
    async fn stop_control_owned(
        &self,
        id: u64,
        mode: Option<GameMode>,
    ) -> Result<Option<ControlRecord>> {
        let _gate = self.coherent_state_gate.lock().await;
        let generation = self.common_receipts.lock().await.generation;
        let mut control = self.common_control.lock().await;
        let old_world = control.generation != Some(generation);
        let Some(session) = control.session.as_mut().filter(|s| s.id == id) else {
            return Ok(None);
        };
        if mode.is_some_and(|mode| session.mode != mode) {
            return Err(state_error("selected control belongs to a different mode"));
        }
        let mut release = Ok(());
        if session.running() {
            session.status = ControlStatus::Stopped {
                reason: if old_world {
                    "world changed (respawn or dimension)".into()
                } else {
                    "stopped by request".into()
                },
            };
            if !old_world && !self.stopped.load(Ordering::Acquire) {
                release = self.release_control(session).await;
            }
            if let Err(error) = &release {
                session.status = ControlStatus::Stopped {
                    reason: format!("stopped; release write failed: {error}"),
                };
            }
        }
        if release.is_ok() {
            self.connection.ledger().retire_common_control(id);
        }
        release.map(|()| Some(session.record()))
    }

    async fn run_control(
        &self,
        id: u64,
        generation: u64,
        mode: GameMode,
        kind: crate::client::control::ControlKind,
        producer: crate::client::ledger::Producer,
    ) {
        let mut interval = tokio::time::interval(Duration::from_millis(50));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            let _gate = self.coherent_state_gate.lock().await;
            let queued = self.common_control.lock().await.jump_work.take();
            let queued_jump = queued.is_some();
            let work = match queued {
                Some(work) => work,
                None => match self.connection.ledger().try_work() {
                    Ok(work) => work,
                    Err(_) => break,
                },
            };
            let again = work
                .run_scoped(async {
                    if self.stopped.load(Ordering::Acquire) {
                        let mut control = self.common_control.lock().await;
                        if let Some(s) = control
                            .session
                            .as_mut()
                            .filter(|s| s.id == id && s.running())
                        {
                            s.status = ControlStatus::Stopped {
                                reason: "disconnected".into(),
                            };
                        }
                        return false;
                    }
                    let stop = self.control_stop_reason(generation, mode, kind).await;
                    let received = self.control_received().await;
                    let mut control = self.common_control.lock().await;
                    let Some(session) = control
                        .session
                        .as_mut()
                        .filter(|s| s.id == id && s.running())
                    else {
                        return false;
                    };
                    if let Some(reason) = stop.or_else(|| session.glide_stop_reason(&received)) {
                        session.status = ControlStatus::Stopped {
                            reason: reason.into(),
                        };
                        // A new world resets input; never send the old world's entity ID.
                        if reason != "world changed (respawn or dimension)" {
                            if let Err(error) = self.release_control(session).await {
                                session.status = ControlStatus::Stopped {
                                    reason: format!("{reason}; release write failed: {error}"),
                                };
                            }
                        }
                        return false;
                    }
                    if kind == crate::client::control::ControlKind::Glide {
                        if !session.glide.as_ref().unwrap().dispatched {
                            if self
                                .common_receipts
                                .lock()
                                .await
                                .fall_flying
                                .as_ref()
                                .is_some_and(|v| v.value)
                            {
                                session.status = ControlStatus::Stopped {
                                    reason:
                                        "actual glide preceded start dispatch; inspect before reuse"
                                            .into(),
                                };
                                return false;
                            }
                            if let Err(error) = self.control_entity_action(8).await {
                                session.status = ControlStatus::Stopped {
                                    reason: format!(
                                        "glide start write failed; inspect before reuse: {error}"
                                    ),
                                };
                                return false;
                            }
                            session.glide.as_mut().unwrap().dispatched = true;
                        }
                        if session.glide_waiting() {
                            return true;
                        }
                    }
                    session.apply_received(&received);
                    let contacts_ready: Result<()> = async {
                        if let Some(contacts) = &mut session.contacts {
                            let receipts = self.common_receipts.lock().await;
                            let sequence = self.protocol_packet_sequence.load(Ordering::Acquire);
                            let stamp = crate::client::SessionStamp {
                                version: MinecraftVersion::Java1_16_1,
                                connection_id: self.connection_id(),
                                world_generation: generation,
                            };
                            contacts.refresh_player(
                                id,
                                &receipts.entities,
                                sequence,
                                (&session.body, &received.environment),
                            )?;
                            let teams = self.common_teams.lock().await.capture(stamp, sequence);
                            let profiles = self
                                .common_player_list
                                .lock()
                                .await
                                .capture(stamp, sequence);
                            contacts.teams(
                                &teams,
                                &profiles,
                                crate::MovementOwnerState::Invalidated,
                            )?;
                            session.contact_team = crate::client::entity::owned_contacts::team(
                                &teams,
                                &self.login_profile.name,
                            );
                        }
                        Ok(())
                    }
                    .await;
                    if let Err(error) = contacts_ready.or_else(|error| {
                        if session.omit_contacts(&error) {
                            Ok(())
                        } else {
                            Err(error)
                        }
                    }) {
                        session.movement_refusal = error.movement_refusal().cloned();
                        session.contacts = None;
                        session.status = ControlStatus::Stopped {
                            reason: error.to_string(),
                        };
                        if let Err(release) = self.release_control(session).await {
                            session.status = ControlStatus::Stopped {
                                reason: format!("{error}; release write failed: {release}"),
                            };
                        }
                        return false;
                    }
                    let output = {
                        let world = self.world.lock().await;
                        session.step(&received, &mut |p| {
                            super::common_motion::legacy_motion_block(&world, p)
                        })
                    };
                    let output = match output {
                        Ok(Some(output)) => output,
                        Ok(None) => return true,
                        Err(error) => {
                            session.movement_refusal = error.movement_refusal().cloned();
                            session.status = ControlStatus::Stopped {
                                reason: error.to_string(),
                            };
                            if let Err(release) = self.release_control(session).await {
                                session.status = ControlStatus::Stopped {
                                    reason: format!("{error}; release write failed: {release}"),
                                };
                            }
                            return false;
                        }
                    };
                    let landed = kind == crate::client::control::ControlKind::Flight
                        && session.body.flight_landed;
                    if landed {
                        session.flight_disable = Some(crate::client::control::FlightDisable {
                            after_sequence: self.protocol_packet_sequence.load(Ordering::Acquire),
                            dispatched: false,
                        });
                    }
                    let glide_ended = kind == crate::client::control::ControlKind::Glide
                        && session.body.glide_ended;
                    let domain = if session.best_effort_contacts {
                        crate::client::domain::Domain::Motion
                    } else {
                        crate::client::domain::Domain::Strict
                    };
                    let written: Result<()> = async {
                        if landed {
                            self.send(0x1a, &[0]).await?;
                            self.common_receipts.lock().await.requested_flying = false;
                            self.survival.write().await.flying = false;
                            session.flight_disable.as_mut().unwrap().dispatched = true;
                        }
                        // LocalPlayer.sendPosition: sprint, then sneak, then movement.
                        if let Some(start) = output.sprint {
                            self.control_entity_action_in(if start { 3 } else { 4 }, domain)
                                .await?;
                        }
                        if let Some(press) = output.sneak {
                            self.control_entity_action_in(if press { 0 } else { 1 }, domain)
                                .await?;
                        }
                        let mut payload = Vec::with_capacity(33);
                        for v in output.position {
                            payload.extend(v.to_be_bytes());
                        }
                        for v in output.rotation {
                            payload.extend(v.to_be_bytes());
                        }
                        payload.push(u8::from(output.on_ground));
                        self.send_in_domain(domain, 0x13, &payload).await
                    }
                    .await;
                    if let Err(error) = written {
                        session.status = ControlStatus::Stopped {
                            reason: format!("write failed; inspect before reuse: {error}"),
                        };
                        return false;
                    }
                    session.dispatched(&output);
                    let path_complete = session.path_complete();
                    if path_complete {
                        session.status = ControlStatus::Stopped {
                            reason: "best-effort path fully dispatched; acceptance is separate"
                                .into(),
                        };
                        if let Err(error) = self.release_control(session).await {
                            session.status = ControlStatus::Stopped {
                                reason: format!("path release write failed: {error}"),
                            };
                        }
                    }

                    let frame = session.frame.clone().unwrap();
                    let fall_distance = session.body.fall_distance as f32;
                    let vertical_collision = session.body.vertical_collision;
                    if landed {
                        session.status = ControlStatus::Stopped {
                            reason: "creative flight model landed".into(),
                        };
                        if let Err(error) = self.release_control(session).await {
                            session.status = ControlStatus::Stopped {
                                reason: format!(
                                    "creative flight model landed; release write failed: {error}"
                                ),
                            };
                        }
                    }
                    if glide_ended {
                        session.status = ControlStatus::Stopped {
                            reason:
                                "glide model landed or reached a ladder; actual flag is separate"
                                    .into(),
                        };
                        if let Err(error) = self.release_control(session).await {
                            session.status = ControlStatus::Stopped {
                                reason: format!("glide model ended; release write failed: {error}"),
                            };
                        }
                    }
                    if landed {
                        control.flight_disabled = Some((
                            generation,
                            self.protocol_packet_sequence.load(Ordering::Acquire),
                        ));
                    }
                    drop(control);
                    let snapshot = {
                        let mut player = self.player.lock().await;
                        player.x = output.position[0];
                        player.y = output.position[1];
                        player.z = output.position[2];
                        player.yaw = output.rotation[0];
                        player.pitch = output.rotation[1];
                        player.on_ground = output.on_ground;
                        let mut receipts = self.common_receipts.lock().await;
                        receipts.ground_source = Some(crate::client::ValueSource::Predicted);
                        receipts.rotation_source = Some(crate::client::ValueSource::Submitted);
                        player.clone()
                    };
                    {
                        let mut motion = self.motion.lock().await;
                        let previous = **motion;
                        **motion = MotionState {
                            velocity: Vec3 {
                                x: frame.velocity[0],
                                y: frame.velocity[1],
                                z: frame.velocity[2],
                            },
                            collided_horizontal: frame.horizontal_collision,
                            collided_vertical: vertical_collision,
                            ticks: previous.ticks + 1,
                            fall_distance,
                        };
                        self.common_control.lock().await.model_tick = Some(motion.ticks);
                    }
                    self.physics.lock().await.record_movement(snapshot);
                    self.common_receipts.lock().await.position_source =
                        Some(crate::client::ValueSource::Predicted);
                    if landed || glide_ended || path_complete {
                        return false;
                    }
                    true
                })
                .await;
            if queued_jump {
                let completed = self
                    .common_control
                    .lock()
                    .await
                    .session
                    .as_ref()
                    .is_some_and(|s| {
                        matches!(
                            s.record().ground_jump.map(|r| r.status),
                            Some(crate::client::control::GroundJumpStatus::Evaluated {
                                dispatched: true,
                                ..
                            })
                        )
                    });
                if !completed {
                    self.connection
                        .ledger()
                        .fail(crate::client::ledger::Failure::TransactionUnconfirmed);
                }
            }
            if !again {
                break;
            }
        }
        if !self
            .common_control
            .lock()
            .await
            .session
            .as_ref()
            .is_some_and(|s| s.id == id && s.running())
        {
            self.connection.ledger().retire_common_control(id);
        }
        producer.acknowledge_park();
    }
}

impl Bot {
    pub(crate) async fn body_start_control(
        &self,
        mode: GameMode,
        world: crate::body::BodyWorld,
    ) -> Result<ControlRecord> {
        self.start_control_inner(mode, None, false, true, Some(world), None)
            .await
    }
    async fn start_control_inner(
        &self,
        mode: GameMode,
        initial_flight_controls: Option<Controls>,
        glide: bool,
        best_effort_contacts: bool,
        world: Option<crate::body::BodyWorld>,
        path: Option<&[crate::client::survival::SurvivalControl]>,
    ) -> Result<ControlRecord> {
        let admission = self.connection.ledger().try_work()?;
        admission
            .run_scoped(async {
                let kind = if glide {
                    crate::client::control::ControlKind::Glide
                } else if initial_flight_controls.is_some() {
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
                    crate::client::control::ControlKind::Glide if mode != GameMode::Spectator => {}
                    _ => {
                        return Err(state_error(
                            "selected control kind does not support this mode",
                        ));
                    }
                }
                if let Some(controls) = initial_flight_controls {
                    crate::client::operations::validate_rotation([controls.yaw, controls.pitch])?;
                    if !(-1..=1).contains(&controls.forward) || !(-1..=1).contains(&controls.strafe)
                    {
                        return Err(state_error("controls out of range"));
                    }
                }
                self.wait_until_ready().await?;
                let _gate = self.coherent_state_gate.lock().await;
                let generation = self.common_receipts.lock().await.generation;
                if world.is_some_and(|world| {
                    world
                        != (crate::body::BodyWorld {
                            connection: self.connection_id(),
                            generation,
                        })
                }) {
                    return Err(state_error("body world changed before control admission"));
                }

                if self
                    .survival
                    .read()
                    .await
                    .game_mode
                    .and_then(|id| GameMode::decode(id & 7).ok())
                    != Some(mode)
                {
                    return Err(state_error(
                        "continuous control requires matching received mode",
                    ));
                }
                if self.common_control.lock().await.active() {
                    return Err(state_error("a control session is already running"));
                }
                if best_effort_contacts {
                    self.common_domain_admission(
                        false,
                        None,
                        crate::client::domain::Domain::Motion,
                    )
                    .await?;
                } else if self.common_motion_pauses_physics().await
                    || self.common_native_physics_paused().await
                {
                    return Err(state_error("another movement operation is unresolved"));
                }
                if self.control().await != super::ControlState::default() {
                    return Err(state_error("native controls are held; clear them first"));
                }
                if self.jump_requested.load(Ordering::Acquire) {
                    return Err(state_error(
                        "a native ground jump is pending; wait for its physics tick",
                    ));
                }
                if let Some(reason) = self.control_stop_reason(generation, mode, kind).await {
                    return Err(state_error(reason));
                }
                if self.common_receipts.lock().await.pose.is_none() {
                    return Err(state_error("continuous control requires a received pose"));
                }
                let player = self.player.lock().await.clone();
                let motion = **self.motion.lock().await;
                let received = self.control_received().await;
                if glide
                    && self
                        .common_receipts
                        .lock()
                        .await
                        .fall_flying
                        .as_ref()
                        .is_some_and(|v| v.value)
                {
                    return Err(state_error(
                        "already fall-flying; new start request is not qualified",
                    ));
                }
                let equipment = if glide {
                    Some(self.glide_equipment().await?)
                } else {
                    None
                };
                let mut control = self.common_control.lock().await;
                control.next_id += 1;
                let id = control.next_id;
                let mut body = control
                    .session
                    .as_ref()
                    .filter(|_| {
                        control.generation == Some(generation)
                            && control.model_tick == Some(motion.ticks)
                    })
                    .map_or_else(
                        || Body::new([player.x, player.y, player.z]),
                        |s| s.body.clone(),
                    );
                body.teleport(MinecraftVersion::Java1_16_1, [player.x, player.y, player.z]);
                // Native packet application and the passive SDK loop already update
                // this model. Retained received velocity must not be replayed here.
                body.velocity = [motion.velocity.x, motion.velocity.y, motion.velocity.z];
                body.on_ground = player.on_ground;
                body.horizontal_collision = motion.collided_horizontal;
                body.vertical_collision = motion.collided_vertical;
                body.fall_distance = f64::from(motion.fall_distance);
                let mut session = ControlSession::from_model(
                    MinecraftVersion::Java1_16_1,
                    id,
                    body,
                    [player.yaw, player.pitch],
                    &received,
                );
                if control.generation == Some(generation) {
                    if let Some(previous) = &control.session {
                        // Keep local item release provenance across control ownership changes.
                        session.inherit_item_release(previous);
                        session.inherit_terrain_budget(previous);
                    }
                }
                control.generation = Some(generation);
                control.model_tick = Some(motion.ticks);
                if !glide
                    && control
                        .session
                        .as_ref()
                        .is_some_and(|p| p.kind == crate::client::control::ControlKind::Glide)
                {
                    if *self.local_pose.lock().await != Some(0) {
                        return Err(state_error(
                            "ground after glide requires actual standing pose",
                        ));
                    }
                    session.body.leave_glide_pose();
                }
                session.mode = mode;
                session.kind = kind;
                session.best_effort_contacts = best_effort_contacts;
                if let Some(path) = path {
                    session.set_path(path);
                }
                if let Some(controls) = initial_flight_controls {
                    session.controls = controls;
                }
                {
                    let receipts = self.common_receipts.lock().await;
                    let sequence = self.protocol_packet_sequence.load(Ordering::Acquire);
                    let stamp = crate::client::SessionStamp {
                        version: MinecraftVersion::Java1_16_1,
                        connection_id: self.connection_id(),
                        world_generation: generation,
                    };
                    if let Some(own) = player.entity_id {
                        let admission: Result<()> = async {
                            let mut contacts =
                                crate::client::entity::owned_contacts::OwnedContacts::for_player(
                                    (stamp, id, own),
                                    &receipts.entities,
                                    sequence,
                                    (&session.body, &received.environment),
                                    self.connection_options.max_contact_bodies,
                                    glide,
                                )?;
                            let teams = self.common_teams.lock().await.capture(stamp, sequence);
                            let profiles = self
                                .common_player_list
                                .lock()
                                .await
                                .capture(stamp, sequence);
                            contacts.teams(
                                &teams,
                                &profiles,
                                crate::MovementOwnerState::NotAdmitted,
                            )?;
                            session.contact_team = crate::client::entity::owned_contacts::team(
                                &teams,
                                &self.login_profile.name,
                            );
                            session.contacts = Some(contacts);
                            Ok(())
                        }
                        .await;
                        if let Err(error) = admission {
                            if !session.omit_contacts(&error) {
                                return Err(error);
                            }
                        }
                    } else if !receipts
                        .entities
                        .capture(stamp, sequence)
                        .entities
                        .is_empty()
                    {
                        return Err(state_error("contact model requires own login entity"));
                    }
                }
                if let Some(equipment) = equipment {
                    let world = self.world.lock().await;
                    crate::client::physics::glide_origin(
                        MinecraftVersion::Java1_16_1,
                        &session.body,
                        &mut |p| super::common_motion::legacy_motion_block(&world, p),
                    )?;
                    drop(world);

                    if session.body.on_ground
                        || session.body.in_water
                        || received.environment.levitation
                    {
                        return Err(state_error(
                            "glide requires an airborne non-water model without levitation",
                        ));
                    }
                    session.glide = Some(crate::client::glide::GlideRecord {
                        after_sequence: self.protocol_packet_sequence.load(Ordering::Acquire),
                        dispatched: false,
                        accepted_sequence: None,
                        ended_sequence: None,
                        actual: self.common_receipts.lock().await.fall_flying.clone(),
                        equipment,
                        equipment_invalidated_sequence: None,
                        waiting_ticks: 0,
                    });
                }
                let record = session.record();
                self.connection.ledger().bind_movement(
                    crate::client::SessionStamp {
                        version: MinecraftVersion::Java1_16_1,
                        connection_id: self.connection_generation().get(),
                        world_generation: generation,
                    },
                    mode,
                    Some(id),
                )?;
                control.session = Some(session);
                drop(control);
                let bot = self.clone();
                let producer = self
                    .connection
                    .ledger()
                    .register_producer()
                    .map_err(crate::client::ledger::finite::admission_error)?;
                self.connection.ledger().tasks().spawn(async move {
                    bot.run_control(id, generation, mode, kind, producer).await
                });
                Ok(record)
            })
            .await
    }
}

impl crate::client::adapter::ControlOps for Bot {
    async fn start_glide_control(
        &self,
        mode: GameMode,
        controls: Controls,
    ) -> Result<ControlRecord> {
        self.start_control_inner(mode, Some(controls), true, false, None, None)
            .await
    }

    async fn start_control(&self, mode: GameMode) -> Result<ControlRecord> {
        self.start_control_inner(mode, None, false, false, None, None)
            .await
    }

    async fn start_flight_control(
        &self,
        mode: GameMode,
        controls: Controls,
    ) -> Result<ControlRecord> {
        self.start_control_inner(mode, Some(controls), false, false, None, None)
            .await
    }

    async fn start_control_best_effort(&self, mode: GameMode) -> Result<ControlRecord> {
        self.start_control_inner(mode, None, false, true, None, None)
            .await
    }

    async fn start_path_best_effort(
        &self,
        mode: GameMode,
        controls: &[crate::client::survival::SurvivalControl],
    ) -> Result<ControlRecord> {
        crate::client::survival::model::validate_controls(controls)?;
        self.start_control_inner(mode, None, false, true, None, Some(controls))
            .await
    }

    async fn walking_diagnostics_for(
        &self,
        session_id: u64,
    ) -> Result<crate::client::control::WalkingDiagnostics> {
        let state = self.common_control.lock().await;
        let session = state
            .session
            .as_ref()
            .filter(|session| session.id == session_id)
            .ok_or_else(|| {
                crate::error::state_error("selected control session is missing or was replaced")
            })?;
        Ok(session.walking_diagnostics())
    }

    async fn contact_diagnostic_for(
        &self,
        session_id: u64,
    ) -> Result<Option<crate::MovementRefusal>> {
        let control = self.common_control.lock().await;
        let session = control
            .session
            .as_ref()
            .filter(|s| s.id == session_id)
            .ok_or_else(|| state_error("selected control session is missing or was replaced"))?;
        Ok(session.contact_diagnostic.clone())
    }

    async fn set_controls(
        &self,
        mode: GameMode,
        session_id: Option<u64>,
        controls: Controls,
    ) -> Result<ControlRecord> {
        let admission = self.connection.ledger().try_work()?;
        admission
            .run_scoped(async {
                crate::client::operations::validate_rotation([controls.yaw, controls.pitch])?;
                if !(-1..=1).contains(&controls.forward) || !(-1..=1).contains(&controls.strafe) {
                    return Err(state_error("controls out of range"));
                }
                self.wait_until_ready().await?;
                let _gate = self.coherent_state_gate.lock().await;
                if self
                    .survival
                    .read()
                    .await
                    .game_mode
                    .and_then(|id| GameMode::decode(id & 7).ok())
                    != Some(mode)
                {
                    return Err(state_error("controls require matching received mode"));
                }
                let generation = self.common_receipts.lock().await.generation;
                let kind = {
                    self.common_control
                        .lock()
                        .await
                        .session
                        .as_ref()
                        .map_or(crate::client::control::ControlKind::Ground, |s| s.kind)
                };
                if let Some(reason) = self.control_stop_reason(generation, mode, kind).await {
                    return Err(state_error(reason));
                }
                let received = self.control_received().await;
                let mut control = self.common_control.lock().await;
                if control.generation != Some(generation) {
                    return Err(state_error("control belongs to a previous world"));
                }
                match control.session.as_mut() {
                    Some(session) if session.running() && session.mode == mode => {
                        if session_id.is_some_and(|id| session.id != id) {
                            return Err(state_error("selected control session was replaced"));
                        }
                        if let Some(reason) = session.glide_stop_reason(&received) {
                            return Err(state_error(reason));
                        }
                        if session.path_active() {
                            return Err(crate::error::state_error("finite path owns its scheduled inputs; stop the selected owner first"));
                        }
                        session.controls = controls;
                        Ok(session.record())
                    }
                    _ => Err(state_error("no running control session")),
                }
            })
            .await
    }

    #[cfg(test)]
    async fn stop_control(&self) -> Result<Option<ControlRecord>> {
        let control = self.common_control.lock().await;
        let Some(session) = control.session.as_ref() else {
            return Ok(None);
        };
        let id = session.id;
        drop(control);
        let owner = self.clone_internal();
        self.connection
            .ledger()
            .tasks()
            .spawn(async move { owner.stop_control_owned(id, None).await })
            .await
            .map_err(|error| state_error(format!("control stop task failed: {error}")))?
    }

    #[cfg(test)]
    async fn stop_control_for(&self, session_id: u64) -> Result<ControlRecord> {
        let owner = self.clone_internal();
        self.connection
            .ledger()
            .tasks()
            .spawn(async move { owner.stop_control_owned(session_id, None).await })
            .await
            .map_err(|error| state_error(format!("control stop task failed: {error}")))??
            .ok_or_else(|| state_error("selected control session is missing or was replaced"))
    }

    async fn stop_control_in_mode(
        &self,
        mode: GameMode,
        session_id: Option<u64>,
    ) -> Result<Option<ControlRecord>> {
        let admission = self.connection.ledger().try_work()?;
        admission
            .run_scoped(async {
                let control = self.common_control.lock().await;
                let Some(session) = control.session.as_ref() else {
                    return Ok(None);
                };
                if session.mode != mode || session_id.is_some_and(|id| session.id != id) {
                    return Err(state_error(
                        "selected control was replaced or belongs to a different mode",
                    ));
                }
                let id = session.id;
                drop(control);
                let owner = self.clone();
                self.spawn_finite_result(
                    async move { owner.stop_control_owned(id, Some(mode)).await },
                )
                .await
                .map_err(|error| state_error(format!("control stop task failed: {error}")))?
            })
            .await
    }

    async fn request_ground_jump(
        &self,
        mode: GameMode,
        session_id: u64,
    ) -> Result<crate::client::control::GroundJumpRequest> {
        self.connection.ledger().check_normal()?;
        self.wait_until_ready().await?;
        let _gate = self.coherent_state_gate.lock().await;
        let admission = self.connection.ledger().try_work()?;
        admission
            .run_scoped(async {
                if self
                    .survival
                    .read()
                    .await
                    .game_mode
                    .and_then(|id| GameMode::decode(id & 7).ok())
                    != Some(mode)
                {
                    return Err(state_error("ground jump requires matching received mode"));
                }
                let generation = self.common_receipts.lock().await.generation;
                let kind = {
                    self.common_control
                        .lock()
                        .await
                        .session
                        .as_ref()
                        .map_or(crate::client::control::ControlKind::Ground, |s| s.kind)
                };
                if let Some(reason) = self.control_stop_reason(generation, mode, kind).await {
                    return Err(state_error(reason));
                }
                let mut control = self.common_control.lock().await;
                if self.stopped.load(Ordering::Acquire)
                    || self.connection_state() != crate::ConnectionState::Ready
                {
                    return Err(state_error("ground jump connection is no longer ready"));
                }
                if control.generation != Some(generation) {
                    return Err(state_error(
                        "ground jump control belongs to a previous world",
                    ));
                }
                let request = control
                    .session
                    .as_mut()
                    .filter(|s| s.mode == mode)
                    .ok_or_else(|| state_error("no running control session in the expected mode"))?
                    .request_ground_jump(session_id)?;
                if control.jump_work.is_none() {
                    control.jump_work = Some(self.connection.ledger().work());
                }
                Ok(request)
            })
            .await
    }

    async fn control_record(&self) -> Result<Option<ControlRecord>> {
        Ok(self
            .common_control
            .lock()
            .await
            .session
            .as_ref()
            .map(ControlSession::record))
    }
}

impl Bot {
    pub(super) async fn quiesce_movement(
        &self,
        target: crate::client::StopTarget,
        id: u64,
    ) -> Result<()> {
        let _gate = self.coherent_state_gate.lock().await;
        self.observe_stop_context().await;
        if !self.connection.ledger().stop_valid(target) {
            return Err(state_error("selected movement context changed"));
        }
        let mut control = self.common_control.lock().await;
        let session = control
            .session
            .as_mut()
            .filter(|s| s.id == id && s.mode == target.mode() && s.running())
            .ok_or_else(|| state_error("selected movement owner changed"))?;
        if session.mode_change_sequence.is_some() || session.flight_change_sequence.is_some() {
            return Err(state_error("selected movement mode changed"));
        }
        // This selected common owner owns wire posture, including posture it
        // inherited from a paused native producer. No predicted position.
        if self.player.lock().await.entity_id.is_some() {
            self.control_entity_action(4).await?;
            self.control_entity_action(1).await?;
        } else {
            self.release_control(session).await?;
        }
        session.status = ControlStatus::Stopped {
            reason: "neutralized during quiesce".into(),
        };
        self.connection.ledger().clear_target(target);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::adapter::ControlOps;
    use crate::client::control::Output;
    use tokio::time::timeout;

    #[tokio::test]
    async fn best_effort_unknown_body_and_route_use_owned_native_ticks() {
        let (bot, _packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        bot.player.lock().await.entity_id = Some(42);
        bot.world.lock().await.clear();
        let client = crate::Client::from_java_1_16_1(bot.clone());
        crate::client::tests::best_effort_unknown_body_and_route_scenario(&client).await;
        let _ = client.revoke_connection();
        drop(release);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn glide_start_without_actual_flag_times_out_without_position_or_retry() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        {
            let mut player = bot.player.lock().await;
            player.entity_id = Some(42);
            player.y = 68.0;
            player.on_ground = false;
        }
        bot.common_receipts
            .lock()
            .await
            .pose
            .as_mut()
            .unwrap()
            .position[1] = 68.0;
        let id = crate::client::registry::Registry::for_version(MinecraftVersion::Java1_16_1)
            .item("minecraft:elytra")
            .unwrap()
            .id
            .value() as u16;
        let mut slot = vec![0, 0, 6, 1];
        crate::protocol::put_varint(&mut slot, i32::from(id));
        slot.extend([1, 0]);
        bot.apply_packet(0x16, slot).await.unwrap();
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let control = client.player_control(GameMode::Survival);
        let initial = control
            .start_glide_control(Default::default())
            .await
            .unwrap();
        assert_eq!(packets.recv().await.unwrap(), (0x1c, vec![42, 8, 0]));
        let ended = timeout(Duration::from_secs(4), async {
            loop {
                let record = control.control_record().await.unwrap().unwrap();
                if matches!(record.status, ControlStatus::Stopped { .. }) {
                    break record;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(ended.session_id, initial.session_id);
        assert_eq!(ended.dispatched_ticks, 0);
        assert!(ended.frame.is_none());
        assert!(ended.glide.unwrap().accepted_sequence.is_none());
        assert!(
            timeout(Duration::from_millis(80), packets.recv())
                .await
                .is_err()
        );
        let _ = client.revoke_connection();
        drop(release);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn selected_control_rejects_replacement_and_old_world_without_writes() {
        for best_effort_contacts in [false, true] {
            let (bot, mut packets, release, server) =
                super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
            super::super::common_motion::tests::seed_motion(&bot).await;
            bot.player.lock().await.entity_id = Some(42);
            let received = bot.control_received().await;
            let mut session =
                ControlSession::new(MinecraftVersion::Java1_16_1, 2, [8.5, 65.0, 8.5], &received);
            session.best_effort_contacts = best_effort_contacts;
            session.dispatched(&Output {
                sneak: Some(true),
                sprint: Some(true),
                input: None,
                position: [8.5, 65.0, 8.5],
                rotation: [0.0; 2],
                on_ground: true,
                horizontal_collision: false,
            });
            let generation = bot.common_receipts.lock().await.generation;
            {
                let mut control = bot.common_control.lock().await;
                control.session = Some(session);
                control.generation = Some(generation);
            }
            let client = crate::Client::from_java_1_16_1(bot.clone());
            let api = client.survival();
            let keys = Controls {
                forward: 1,
                ..Default::default()
            };
            assert!(api.set_controls_for(1, keys).await.is_err());
            assert!(api.stop_control_for(1).await.is_err());
            let replacement = api.control_record().await.unwrap().unwrap();
            assert_eq!(replacement.session_id, 2);
            assert_eq!(replacement.status, ControlStatus::Running);
            assert_eq!(replacement.controls, Controls::default());
            assert_eq!(api.set_controls_for(2, keys).await.unwrap().controls, keys);
            // Reproduce a respawn generation before the worker handles it. Even
            // previously submitted sprint/sneak cannot release in the new world.
            bot.common_receipts.lock().await.generation += 1;
            assert!(api.set_controls_for(2, Controls::default()).await.is_err());
            let stopped = api.stop_control_for(2).await.unwrap();
            assert_eq!(stopped.controls, keys);
            assert!(
                matches!(stopped.status, ControlStatus::Stopped { ref reason } if reason.contains("world changed"))
            );
            assert_eq!(api.stop_control_for(2).await.unwrap(), stopped);
            assert!(
                timeout(Duration::from_millis(30), packets.recv())
                    .await
                    .is_err()
            );
            drop(api);
            drop(client);
            drop(release);
            drop(bot);
            timeout(Duration::from_secs(2), server)
                .await
                .unwrap()
                .unwrap();
        }
    }

    #[tokio::test]
    async fn control_restart_keeps_current_momentum_and_aim_after_decoded_velocity() {
        // The fake TCP peer sends one actual velocity packet after a look.
        // The SDK's passive producer is held at the teleport barrier so the
        // stop/start boundary has no intervening model tick or second receipt.
        let mut velocity = vec![42];
        velocity.extend(640_i16.to_be_bytes());
        velocity.extend(0_i16.to_be_bytes());
        velocity.extend(0_i16.to_be_bytes());
        let (bot, _packets, release, server) =
            super::super::tests::operation_test_bot(0x13, 0x46, velocity).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        bot.player.lock().await.entity_id = Some(42);
        let client = crate::Client::from_java_1_16_1(bot.clone());
        client.look([37.0, -12.0]).await.unwrap();
        release.send(()).unwrap();
        timeout(Duration::from_secs(2), async {
            while bot.own_velocity_receipt.lock().await.is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let received = *bot.own_velocity_receipt.lock().await;
        let survival = client.survival();
        let first = survival.start_control().await.unwrap();
        assert_eq!([first.controls.yaw, first.controls.pitch], [37.0, -12.0]);
        timeout(Duration::from_secs(2), async {
            while survival
                .control_record()
                .await
                .unwrap()
                .unwrap()
                .dispatched_ticks
                < 3
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let stopped = survival.stop_control().await.unwrap().unwrap();
        let old_frame = stopped.frame.unwrap();
        assert!(old_frame.velocity[0] > 0.0 && old_frame.velocity[0] < 0.08);
        let started = survival.start_control().await.unwrap();
        assert_ne!(started.session_id, stopped.session_id);
        assert_eq!(
            [started.controls.yaw, started.controls.pitch],
            [37.0, -12.0]
        );
        assert!(!started.controls.jump && !started.controls.sprint && !started.controls.sneak);
        assert_eq!((started.controls.forward, started.controls.strafe), (0, 0));
        timeout(Duration::from_secs(2), async {
            while survival
                .control_record()
                .await
                .unwrap()
                .unwrap()
                .dispatched_ticks
                == 0
            {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
        let resumed = survival.stop_control().await.unwrap().unwrap();
        let frame = resumed.frame.unwrap();
        assert_eq!((resumed.corrections, resumed.velocity_updates), (0, 0));
        assert!(frame.position[0] > old_frame.position[0]);
        assert!(frame.velocity[0] > 0.0 && frame.velocity[0] < old_frame.velocity[0]);
        assert_eq!(*bot.own_velocity_receipt.lock().await, received);
        assert_eq!(client.player_state().await.unwrap().rotation, [37.0, -12.0]);
        let _ = client.revoke_connection();
        drop(client);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn native_ground_queue_cannot_cross_common_control_ownership() {
        let (bot, _packets, _release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        bot.jump().await.unwrap();
        assert!(bot.jump_requested.load(Ordering::Acquire));
        assert!(bot.start_control(GameMode::Survival).await.is_err());
        assert!(bot.jump_requested.load(Ordering::Acquire));
        // The fixture holds the passive producer at its teleport barrier.
        // Retire that pending native tick to exercise the reverse owner boundary.
        bot.jump_requested.store(false, Ordering::Release);
        bot.start_control(GameMode::Survival).await.unwrap();
        assert!(bot.jump().await.is_err());
        assert!(!bot.jump_requested.load(Ordering::Acquire));
        bot.stop_control().await.unwrap();
        bot.disconnect().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn ground_request_admission_rechecks_tcp_mode_and_revocation_after_waiting() {
        let mut creative = vec![3];
        creative.extend(1f32.to_be_bytes());
        let (bot, _packets, release, server) =
            super::super::tests::operation_test_bot(0x03, 0x1e, creative).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let survival = client.survival();
        assert!(survival.request_ground_jump(1).await.is_err());
        let started = survival.start_control().await.unwrap();
        assert!(
            survival
                .request_ground_jump(started.session_id + 1)
                .await
                .is_err()
        );
        timeout(Duration::from_secs(2), async {
            while survival
                .control_record()
                .await
                .unwrap()
                .unwrap()
                .dispatched_ticks
                < 3
            {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        let queued = survival
            .request_ground_jump(started.session_id)
            .await
            .unwrap();
        timeout(Duration::from_secs(2), async {
            loop {
                let record = survival.control_record().await.unwrap().unwrap();
                if matches!(
                    record.ground_jump.unwrap().status,
                    crate::client::control::GroundJumpStatus::Evaluated {
                        dispatched: true,
                        ..
                    }
                ) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        // Cancellation before admission leaves the previous diagnostic intact.
        let gate = bot.coherent_state_gate.lock().await;
        let mut cancelled = Box::pin(survival.request_ground_jump(started.session_id));
        assert!(
            timeout(Duration::from_millis(10), cancelled.as_mut())
                .await
                .is_err()
        );
        drop(cancelled);
        drop(gate);
        assert_eq!(
            survival
                .control_record()
                .await
                .unwrap()
                .unwrap()
                .ground_jump
                .unwrap()
                .request_id,
            queued.request_id
        );
        release.send(()).unwrap();
        bot.send_chat("ground request mode fixture").await.unwrap();
        timeout(Duration::from_secs(2), async {
            while bot.survival.read().await.game_mode != Some(1) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            survival
                .request_ground_jump(started.session_id)
                .await
                .is_err()
        );
        timeout(Duration::from_secs(2), async {
            while !matches!(
                survival.control_record().await.unwrap().unwrap().status,
                ControlStatus::Stopped { .. }
            ) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let mut survival_mode = vec![3];
        survival_mode.extend(0f32.to_be_bytes());
        bot.apply_packet(0x1e, survival_mode).await.unwrap();
        let replacement = survival.start_control().await.unwrap();
        assert!(
            survival
                .request_ground_jump(started.session_id)
                .await
                .is_err()
        );
        let gate = bot.coherent_state_gate.lock().await;
        let mut waiting = Box::pin(survival.request_ground_jump(replacement.session_id));
        assert!(
            timeout(Duration::from_millis(10), waiting.as_mut())
                .await
                .is_err()
        );
        let _ = client.revoke_connection();
        drop(gate);
        assert!(waiting.await.is_err());
        assert!(
            survival
                .control_record()
                .await
                .unwrap()
                .unwrap()
                .ground_jump
                .is_none()
        );
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn falling_control_exports_fall_distance_to_the_passive_model_before_restart() {
        let (bot, _packets, _release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        {
            let mut player = bot.player.lock().await;
            player.y = 68.;
            player.on_ground = false;
        }
        bot.common_receipts
            .lock()
            .await
            .pose
            .as_mut()
            .unwrap()
            .position[1] = 68.;
        bot.start_control(GameMode::Survival).await.unwrap();
        timeout(Duration::from_secs(2), async {
            while bot
                .control_record()
                .await
                .unwrap()
                .unwrap()
                .dispatched_ticks
                < 3
            {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        bot.stop_control().await.unwrap();
        let fall = bot
            .common_control
            .lock()
            .await
            .session
            .as_ref()
            .unwrap()
            .body
            .fall_distance;
        assert!(fall > 0.);
        assert_eq!(f64::from(bot.motion.lock().await.fall_distance), fall);
        bot.start_control(GameMode::Survival).await.unwrap();
        assert!(
            bot.common_control
                .lock()
                .await
                .session
                .as_ref()
                .unwrap()
                .body
                .fall_distance
                >= fall
        );
        bot.stop_control().await.unwrap();
        bot.disconnect().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn cancelling_stop_wait_still_releases_keys_and_retains_record() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        bot.player.lock().await.entity_id = Some(42);
        let received = bot.control_received().await;
        let mut session =
            ControlSession::new(MinecraftVersion::Java1_16_1, 1, [8.5, 65.0, 8.5], &received);
        session.dispatched(&Output {
            sneak: Some(true),
            sprint: Some(true),
            input: None,
            position: [8.5, 65.0, 8.5],
            rotation: [0.0; 2],
            on_ground: true,
            horizontal_collision: false,
        });
        let generation = bot.common_receipts.lock().await.generation;
        {
            let mut control = bot.common_control.lock().await;
            control.session = Some(session);
            control.generation = Some(generation);
        }
        let writer = bot.writer.lock().await;
        let mut wait = Box::pin(bot.stop_control_for(1));
        assert!(
            timeout(Duration::from_millis(30), wait.as_mut())
                .await
                .is_err()
        );
        drop(wait);
        drop(writer);
        for action in [4, 1] {
            assert_eq!(
                timeout(Duration::from_secs(1), packets.recv())
                    .await
                    .unwrap()
                    .unwrap(),
                (0x1c, vec![42, action, 0])
            );
        }
        let record = bot.control_record().await.unwrap().unwrap();
        assert_eq!(record.session_id, 1);
        assert!(matches!(record.status, ControlStatus::Stopped { .. }));
        assert_eq!(bot.stop_control_for(1).await.unwrap(), record);
        assert!(
            timeout(Duration::from_millis(30), packets.recv())
                .await
                .is_err()
        );
        drop(release);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
}

#[cfg(test)]
mod ground_mode_tests {
    use super::*;
    use tokio::time::timeout;

    #[tokio::test]
    async fn ground_modes_fence_other_handles_and_transient_received_mode_changes() {
        for (mode, raw) in [(GameMode::Adventure, 2f32), (GameMode::Creative, 1f32)] {
            let (bot, _packets, _release, server) =
                super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
            super::super::common_motion::tests::seed_motion(&bot).await;
            bot.player.lock().await.entity_id = Some(42);
            let mut packet = vec![3];
            packet.extend(raw.to_be_bytes());
            bot.apply_packet(0x1e, packet.clone()).await.unwrap();
            let client = crate::Client::from_java_1_16_1(bot.clone());
            let handle = client.player_control(mode);
            assert!(
                client
                    .player_control(GameMode::Spectator)
                    .start_control()
                    .await
                    .is_err()
            );
            let start = handle.start_control().await.unwrap();
            assert_eq!(start.mode, mode);
            let keys = Controls {
                forward: 1,
                yaw: 37.,
                pitch: -12.,
                ..Default::default()
            };
            assert!(
                client
                    .survival()
                    .set_controls_for(start.session_id, keys)
                    .await
                    .is_err()
            );
            assert!(
                client
                    .survival()
                    .stop_control_for(start.session_id)
                    .await
                    .is_err()
            );
            assert!(
                client
                    .survival()
                    .request_ground_jump(start.session_id)
                    .await
                    .is_err()
            );
            assert!(
                handle
                    .set_controls_for(start.session_id + 1, keys)
                    .await
                    .is_err()
            );
            assert_eq!(
                handle
                    .set_controls_for(start.session_id, keys)
                    .await
                    .unwrap()
                    .controls,
                keys
            );
            let gate = bot.coherent_state_gate.lock().await;
            let mut away = vec![3];
            away.extend(0f32.to_be_bytes());
            bot.receive_game_state_change(&away, 100).await.unwrap();
            bot.receive_game_state_change(&packet, 101).await.unwrap();
            let before = handle.control_record().await.unwrap().unwrap();
            assert_eq!(before.mode_change_sequence, Some(100));
            assert_eq!(bot.survival.read().await.game_mode, Some(raw as u8));
            drop(gate);
            assert!(
                handle
                    .set_controls_for(start.session_id, keys)
                    .await
                    .is_err()
            );
            assert!(handle.request_ground_jump(start.session_id).await.is_err());
            let stopped = timeout(Duration::from_secs(2), async {
                loop {
                    let r = handle.control_record().await.unwrap().unwrap();
                    if matches!(r.status, ControlStatus::Stopped { .. }) {
                        break r;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert_eq!(before.dispatched_ticks, stopped.dispatched_ticks);
            assert_eq!(
                handle.stop_control_for(start.session_id).await.unwrap(),
                stopped
            );
            let next = handle.start_control().await.unwrap();
            assert_ne!(next.session_id, start.session_id);
            assert_eq!(next.mode_change_sequence, None);
            assert!(handle.stop_control_for(start.session_id).await.is_err());
            let gate = bot.coherent_state_gate.lock().await;
            for (seq, flags) in [(200, 6u8), (201, 4)] {
                let mut abilities = vec![flags];
                abilities.extend(0.05f32.to_be_bytes());
                abilities.extend(0.1f32.to_be_bytes());
                bot.receive_player_abilities(&abilities, seq).await.unwrap();
            }
            assert!(!bot.survival.read().await.flying);
            assert_eq!(
                handle
                    .control_record()
                    .await
                    .unwrap()
                    .unwrap()
                    .flight_change_sequence,
                Some(200)
            );
            drop(gate);
            assert!(
                handle
                    .set_controls_for(next.session_id, keys)
                    .await
                    .is_err()
            );
            assert!(handle.request_ground_jump(next.session_id).await.is_err());
            let generation = bot.common_receipts.lock().await.generation;
            bot.common_receipts.lock().await.generation += 1;
            assert_eq!(
                bot.control_stop_reason(
                    generation,
                    mode,
                    crate::client::control::ControlKind::Ground
                )
                .await,
                Some("world changed (respawn or dimension)")
            );
            let ended = handle.stop_control_for(next.session_id).await.unwrap();
            assert!(
                matches!(ended.status,ControlStatus::Stopped{ref reason} if reason.contains("world changed") || reason=="flying")
            );
            bot.disconnect().await.unwrap();
            server.await.unwrap();
        }
    }
}

#[cfg(test)]
mod flight_tests {
    use super::*;
    use tokio::time::timeout;
    #[tokio::test]
    async fn held_flight_requires_original_abilities_and_latches_restored_permission() {
        for mode in [GameMode::Creative, GameMode::Spectator] {
            let (bot, _packets, _release, server) =
                super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
            super::super::common_motion::tests::seed_motion(&bot).await;
            bot.player.lock().await.entity_id = Some(42);
            let mut packet = vec![3];
            packet.extend(
                (if mode == GameMode::Creative {
                    1f32
                } else {
                    3f32
                })
                .to_be_bytes(),
            );
            bot.apply_packet(0x1e, packet).await.unwrap();
            let client = crate::Client::from_java_1_16_1(bot.clone());
            let handle = client.player_control(mode);
            let keys = Controls {
                jump: true,
                forward: 1,
                ..Default::default()
            };
            assert!(handle.start_flight_control(keys).await.is_err());
            {
                let mut state = bot.survival.write().await;
                state.flying = true;
                state.flying_allowed = true;
            }
            assert!(handle.start_flight_control(keys).await.is_err());
            let mut abilities = vec![6];
            abilities.extend(0.05f32.to_be_bytes());
            abilities.extend(0.1f32.to_be_bytes());
            bot.receive_player_abilities(&abilities, 90).await.unwrap();
            let start = handle.start_flight_control(keys).await.unwrap();
            assert_eq!(start.kind, crate::client::control::ControlKind::Flight);
            assert_eq!(start.controls, keys);
            assert!(handle.request_ground_jump(start.session_id).await.is_err());
            assert!(
                client
                    .survival()
                    .stop_control_for(start.session_id)
                    .await
                    .is_err()
            );
            let gate = bot.coherent_state_gate.lock().await;
            abilities[0] = 2;
            bot.receive_player_abilities(&abilities, 100).await.unwrap();
            abilities[0] = 6;
            bot.receive_player_abilities(&abilities, 101).await.unwrap();
            let before = handle.control_record().await.unwrap().unwrap();
            assert_eq!(before.flight_change_sequence, Some(100));
            drop(gate);
            assert!(
                handle
                    .set_controls_for(start.session_id, keys)
                    .await
                    .is_err()
            );
            let stopped = timeout(Duration::from_secs(2), async {
                loop {
                    let record = handle.control_record().await.unwrap().unwrap();
                    if matches!(record.status, ControlStatus::Stopped { .. }) {
                        break record;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert_eq!(before.dispatched_ticks, stopped.dispatched_ticks);
            let fresh = handle.start_flight_control(keys).await.unwrap();
            assert_ne!(fresh.session_id, start.session_id);
            assert_eq!(fresh.flight_change_sequence, None);
            assert!(handle.stop_control_for(start.session_id).await.is_err());
            handle.stop_control_for(fresh.session_id).await.unwrap();
            bot.disconnect().await.unwrap();
            server.abort();
        }
    }
}

#[cfg(test)]
mod flight_landing_tests {
    use super::*;
    use tokio::time::timeout;
    #[tokio::test]
    async fn held_creative_flight_landing_disables_before_movement_and_ends_owner() {
        let (bot, mut packets, _release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        bot.player.lock().await.entity_id = Some(42);
        let mut mode = vec![3];
        mode.extend(1f32.to_be_bytes());
        bot.apply_packet(0x1e, mode).await.unwrap();
        let mut abilities = vec![6];
        abilities.extend(0.05f32.to_be_bytes());
        abilities.extend(0.1f32.to_be_bytes());
        bot.apply_packet(0x31, abilities).await.unwrap();
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let handle = client.player_control(GameMode::Creative);
        let start = handle
            .start_flight_control(Controls {
                sneak: true,
                ..Default::default()
            })
            .await
            .unwrap();
        let mut wire = vec![];
        timeout(Duration::from_secs(2), async {
            while wire.len() < 4 {
                let packet = packets.recv().await.unwrap();
                if [0x1a, 0x1c, 0x13].contains(&packet.0) {
                    wire.push(packet);
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(wire[0], (0x1a, vec![0]));
        assert_eq!(wire[1], (0x1c, vec![42, 0, 0]));
        assert_eq!(wire[2].0, 0x13);
        assert_eq!(wire[3], (0x1c, vec![42, 1, 0]));
        let record = handle.control_record().await.unwrap().unwrap();
        assert!(
            matches!(record.status,ControlStatus::Stopped{ref reason} if reason=="creative flight model landed")
        );
        assert_eq!(record.dispatched_ticks, 1);
        assert!(!record.frame.as_ref().unwrap().flying);
        assert!(record.flight_disable.as_ref().unwrap().dispatched);
        assert_eq!(
            bot.common_receipts
                .lock()
                .await
                .context
                .abilities
                .as_ref()
                .unwrap()
                .value
                .flags,
            6
        );
        assert!(
            handle
                .set_controls_for(start.session_id, Controls::default())
                .await
                .is_err()
        );
        assert!(
            handle
                .start_flight_control(Controls {
                    jump: true,
                    ..Default::default()
                })
                .await
                .is_err()
        );
        let ground = handle.start_control().await.unwrap();
        assert_ne!(start.session_id, ground.session_id);
        handle.stop_control_for(ground.session_id).await.unwrap();
        bot.disconnect().await.unwrap();
        server.abort();
    }
}
