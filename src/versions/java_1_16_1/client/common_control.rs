//! Continuous control on 1.16.1. While a session runs, the Bot's own physics
//! loop is paused (`common_motion_pauses_physics`) and one Client-owned task
//! ticks the shared engine every 50 ms instead.
use super::{Bot, MotionState, Vec3};
use crate::client::GameMode;
use crate::client::control::{ControlRecord, ControlSession, ControlStatus, Controls, Received};
use crate::client::physics::{Body, Environment, Modifier, ModifierOperation};
use crate::protocol::put_varint;
use crate::{Error, ErrorKind, MinecraftVersion, Result};
use std::sync::atomic::Ordering;
use std::time::Duration;

/// The connection's continuous control session.
#[derive(Default)]
pub(crate) struct ContinuousControl {
    session: Option<ControlSession>,
    next_id: u64,
    generation: Option<u64>,
    model_tick: Option<u64>,
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

    async fn release_control(&self, session: &ControlSession) -> Result<()> {
        let (sneak, sprint, _) = session.release();
        if sprint.is_some() {
            self.control_entity_action(4).await?;
        }
        if sneak.is_some() {
            self.control_entity_action(1).await?;
        }
        Ok(())
    }

    // The connection owns release writes even if the caller stops awaiting them.
    async fn stop_control_owned(&self, id: u64) -> Result<Option<ControlRecord>> {
        let _gate = self.coherent_state_gate.lock().await;
        let mut control = self.common_control.lock().await;
        let Some(session) = control.session.as_mut().filter(|s| s.id == id) else {
            return Ok(None);
        };
        let mut release = Ok(());
        if session.running() {
            session.status = ControlStatus::Stopped {
                reason: "stopped by request".into(),
            };
            if !self.stopped.load(Ordering::Acquire) {
                release = self.release_control(session).await;
            }
            if let Err(error) = &release {
                session.status = ControlStatus::Stopped {
                    reason: format!("stopped; release write failed: {error}"),
                };
            }
        }
        release.map(|()| Some(session.record()))
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
                // A new world resets input; never send the old world's entity ID.
                if reason != "world changed (respawn or dimension)" {
                    if let Err(error) = self.release_control(session).await {
                        session.status = ControlStatus::Stopped {
                            reason: format!("{reason}; release write failed: {error}"),
                        };
                    }
                }
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
            let fall_distance = session.body.fall_distance as f32;
            let vertical_collision = session.body.vertical_collision;
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
        let player = self.player.lock().await.clone();
        let motion = **self.motion.lock().await;
        let received = self.control_received().await;
        let mut control = self.common_control.lock().await;
        control.next_id += 1;
        let id = control.next_id;
        let mut body = control
            .session
            .as_ref()
            .filter(|_| {
                control.generation == Some(generation) && control.model_tick == Some(motion.ticks)
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
            }
        }
        control.generation = Some(generation);
        control.model_tick = Some(motion.ticks);
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
        let control = self.common_control.lock().await;
        let Some(session) = control.session.as_ref() else {
            return Ok(None);
        };
        let id = session.id;
        drop(control);
        let owner = self.clone_internal();
        tokio::spawn(async move { owner.stop_control_owned(id).await })
            .await
            .map_err(|error| invalid(&format!("control stop task failed: {error}")))?
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::adapter::ControlOps;
    use crate::client::control::Output;
    use tokio::time::timeout;

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
        bot.common_control.lock().await.session = Some(session);
        let writer = bot.writer.lock().await;
        let mut wait = Box::pin(bot.stop_control());
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
        assert_eq!(bot.stop_control().await.unwrap(), Some(record));
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
