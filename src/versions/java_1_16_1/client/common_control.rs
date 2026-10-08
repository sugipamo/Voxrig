//! Continuous control on 1.16.1. While a session runs, the Bot's own physics
//! loop is paused (`common_motion_pauses_physics`) and one Client-owned task
//! ticks the shared engine every 50 ms instead.
use super::{Bot, MotionState, Vec3};
use crate::client::GameMode;
use crate::client::control::{ControlRecord, ControlSession, ControlStatus, Controls, Received};
use crate::client::physics::{Environment, Modifier, ModifierOperation};
use crate::protocol::put_varint;
use crate::{Error, ErrorKind, MinecraftVersion, Result};
use std::sync::atomic::Ordering;
use std::time::Duration;

/// The connection's continuous control session.
#[derive(Default)]
pub(crate) struct ContinuousControl {
    session: Option<ControlSession>,
    next_id: u64,
}

impl ContinuousControl {
    pub(crate) fn active(&self) -> bool {
        self.session.as_ref().is_some_and(ControlSession::running)
    }
    /// A release was sent: the running session stops applying the item-use slowdown.
    pub(crate) fn item_released(&mut self, received: &Received) {
        if let Some(session) = self.session.as_mut().filter(|s| s.running()) {
            session.item_released(received);
        }
    }
}

fn invalid(message: &str) -> Error {
    Error::new(ErrorKind::State, anyhow::anyhow!("{message}"))
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
        let pose = receipts
            .pose
            .as_ref()
            .map(|p| (p.receive_sequence, p.position, Some([0.0; 3])));
        let using_item = crate::client::item_use::received_use(
            receipts.using_item.as_ref(),
            receipts.selected_hotbar.as_ref().map(|s| s.value),
            &receipts.inventory.slots,
        );
        drop(receipts);
        let velocity = *self.own_velocity_receipt.lock().await;
        Received {
            environment,
            pose,
            velocity,
            using_item,
        }
    }

    async fn control_stop_reason(&self, generation: u64) -> Option<&'static str> {
        let receipts = self.common_receipts.lock().await;
        if receipts.generation != generation {
            return Some("world changed (respawn or dimension)");
        }
        if receipts.requested_flying || receipts.vehicles.motion_interrupted() {
            return Some("flying or riding");
        }
        drop(receipts);
        let survival = self.survival.read().await;
        if survival.game_mode != Some(0) {
            Some("game mode changed")
        } else if survival.flying {
            Some("flying")
        } else if survival.vitals.as_ref().is_some_and(|v| v.health <= 0.0) {
            Some("player died")
        } else {
            None
        }
    }

    async fn control_entity_action(&self, action: i32) -> Result<()> {
        let entity = self
            .player
            .lock()
            .await
            .entity_id
            .ok_or_else(|| invalid("Join Game entity ID is unavailable"))?;
        let mut payload = Vec::new();
        put_varint(&mut payload, entity);
        put_varint(&mut payload, action);
        put_varint(&mut payload, 0);
        self.send(0x1c, &payload).await
    }

    async fn run_control(&self, id: u64, generation: u64) {
        let mut interval = tokio::time::interval(Duration::from_millis(50));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
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
                return;
            }
            let _gate = self.coherent_state_gate.lock().await;
            let stop = self.control_stop_reason(generation).await;
            let received = self.control_received().await;
            let mut control = self.common_control.lock().await;
            let Some(session) = control
                .session
                .as_mut()
                .filter(|s| s.id == id && s.running())
            else {
                return;
            };
            if let Some(reason) = stop {
                session.status = ControlStatus::Stopped {
                    reason: reason.into(),
                };
                return;
            }
            let output = {
                let world = self.world.lock().await;
                session.step(&received, &mut |p| {
                    super::common_motion::legacy_motion_block(&world, p)
                })
            };
            let output = match output {
                Ok(Some(output)) => output,
                Ok(None) => continue,
                Err(error) => {
                    session.status = ControlStatus::Stopped {
                        reason: error.to_string(),
                    };
                    return;
                }
            };
            let written: Result<()> = async {
                // LocalPlayer.sendPosition: sprint, then sneak, then movement.
                if let Some(start) = output.sprint {
                    self.control_entity_action(if start { 3 } else { 4 })
                        .await?;
                }
                if let Some(press) = output.sneak {
                    self.control_entity_action(if press { 0 } else { 1 })
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
                self.send(0x13, &payload).await
            }
            .await;
            if let Err(error) = written {
                session.status = ControlStatus::Stopped {
                    reason: format!("write failed; inspect before reuse: {error}"),
                };
                return;
            }
            session.dispatched(&output);
            let frame = session.frame.clone().unwrap();
            drop(control);
            let snapshot = {
                let mut player = self.player.lock().await;
                player.x = output.position[0];
                player.y = output.position[1];
                player.z = output.position[2];
                player.yaw = output.rotation[0];
                player.pitch = output.rotation[1];
                player.on_ground = output.on_ground;
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
                    collided_vertical: frame.on_ground,
                    ticks: previous.ticks + 1,
                    fall_distance: previous.fall_distance,
                };
            }
            self.physics.lock().await.record_movement(snapshot);
            self.common_receipts.lock().await.position_source =
                Some(crate::client::ValueSource::Predicted);
        }
    }
}

impl crate::client::adapter::ControlOps for Bot {
    async fn start_control(&self, mode: GameMode) -> Result<ControlRecord> {
        self.wait_until_ready().await?;
        let _gate = self.coherent_state_gate.lock().await;
        if mode != GameMode::Survival || self.survival.read().await.game_mode != Some(0) {
            return Err(invalid(
                "continuous control requires received survival mode",
            ));
        }
        if self.common_control.lock().await.active() {
            return Err(invalid("a control session is already running"));
        }
        if self.common_motion_pauses_physics().await || self.common_native_physics_paused().await {
            return Err(invalid("another movement operation is unresolved"));
        }
        if self.control().await != super::ControlState::default() {
            return Err(invalid("native controls are held; clear them first"));
        }
        let generation = self.common_receipts.lock().await.generation;
        if let Some(reason) = self.control_stop_reason(generation).await {
            return Err(invalid(reason));
        }
        if self.common_receipts.lock().await.pose.is_none() {
            return Err(invalid("continuous control requires a received pose"));
        }
        let position = {
            let p = self.player.lock().await;
            [p.x, p.y, p.z]
        };
        let received = self.control_received().await;
        let mut control = self.common_control.lock().await;
        control.next_id += 1;
        let id = control.next_id;
        let session = ControlSession::new(MinecraftVersion::Java1_16_1, id, position, &received);
        let record = session.record();
        control.session = Some(session);
        drop(control);
        let bot = self.clone();
        tokio::spawn(async move { bot.run_control(id, generation).await });
        Ok(record)
    }

    async fn set_controls(&self, mode: GameMode, controls: Controls) -> Result<ControlRecord> {
        crate::client::operations::validate_rotation([controls.yaw, controls.pitch])?;
        if !(-1..=1).contains(&controls.forward) || !(-1..=1).contains(&controls.strafe) {
            return Err(invalid("controls out of range"));
        }
        if mode != GameMode::Survival || self.survival.read().await.game_mode != Some(0) {
            return Err(invalid("controls require matching received mode"));
        }
        let mut control = self.common_control.lock().await;
        match control.session.as_mut() {
            Some(session) if session.running() => {
                session.controls = controls;
                Ok(session.record())
            }
            _ => Err(invalid("no running control session")),
        }
    }

    async fn stop_control(&self) -> Result<Option<ControlRecord>> {
        let _gate = self.coherent_state_gate.lock().await;
        let mut control = self.common_control.lock().await;
        let Some(session) = control.session.as_mut() else {
            return Ok(None);
        };
        let mut release = Ok(());
        if session.running() {
            let (sneak, sprint, _) = session.release();
            if sprint.is_some() {
                release = self.control_entity_action(4).await;
            }
            if release.is_ok() && sneak.is_some() {
                release = self.control_entity_action(1).await;
            }
            session.status = ControlStatus::Stopped {
                reason: match &release {
                    Ok(()) => "stopped by request".into(),
                    Err(e) => format!("stopped; release write failed: {e}"),
                },
            };
        }
        let record = session.record();
        release.map(|()| Some(record))
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
