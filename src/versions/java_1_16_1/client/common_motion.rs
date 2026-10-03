//! Common bounded preview with legacy-native rules and coherent legacy admission.
use super::*;
use crate::client::{
    GameMode,
    survival::{
        MotionPreview, MotionRecord, MotionStatus, SurvivalControl, TerminalClearance, model,
    },
};

impl Bot {
    pub(crate) async fn common_preview_path(
        &self,
        controls: &[SurvivalControl],
    ) -> Result<MotionPreview> {
        model::validate_controls(controls)?;
        let _gate = self.coherent_state_gate.lock().await;
        self.common_motion_admission().await?;
        self.common_preview_unlocked(controls).await
    }
    async fn common_preview_unlocked(&self, controls: &[SurvivalControl]) -> Result<MotionPreview> {
        if self.connection_state() != ConnectionState::Ready {
            return Err(motion_state("connection not ready"));
        }
        let initial = self.common_player_unlocked().await?;
        if initial.pending_dispatch {
            return Err(motion_state("prior common dispatch unresolved"));
        }
        if initial.game_mode != Some(GameMode::Survival)
            || !initial
                .health
                .as_ref()
                .is_some_and(|health| health.value.health > 0.0)
            || *self.local_pose.lock().await != Some(0)
        {
            return Err(motion_state(
                "dry preview requires healthy received survival mode and native standing pose",
            ));
        }
        let survival = self.survival.read().await;
        if survival.flying
            || !survival.effects.is_empty()
            || survival.attributes.values().any(|attribute| {
                matches!(
                    attribute.key.as_str(),
                    "minecraft:generic.movement_speed" | "generic.movement_speed"
                ) && attribute.value() as f32 != 0.1f32
            })
        {
            return Err(motion_state(
                "dry preview requires native default motion and no received flight/effects",
            ));
        }
        drop(survival);
        if self.control().await != ControlState::default() {
            return Err(motion_state(
                "persistent legacy controls must be released before a bounded preview",
            ));
        }
        let player = self.player.lock().await.clone();
        let motion = **self.motion.lock().await;
        if !player.on_ground
            || motion.velocity.x != 0.0
            || motion.velocity.z != 0.0
            || (motion.velocity.y != 0.0
                && (motion.velocity.y + 0.08 * f64::from(0.98f32)).abs() > 1e-8)
        {
            return Err(motion_state(
                "dry preview requires locally stationary grounded context",
            ));
        }
        let position = initial
            .position
            .as_ref()
            .ok_or_else(|| motion_state("position unavailable"))?
            .value;
        let mut model = model::Model::new(crate::MinecraftVersion::Java1_16_1, position);
        model.frame.velocity = [motion.velocity.x, motion.velocity.y, motion.velocity.z];
        let initial_frame = model.initial_frame();
        let world = self.world.lock().await;
        let block_at = |position| legacy_motion_block(&world, position);
        // Check the whole standing body and support even for released inputs.
        legacy_clearance(&block_at, position, 0.0)?;
        let frames = model::predict(&block_at, &mut model, controls)?;
        let terminal = frames.last().expect("bounded nonempty controls");
        let terminal_clearance = if terminal.resting {
            match legacy_clearance(&block_at, terminal.position, 1.0 / 16.0) {
                Ok(()) => TerminalClearance::Admitted {
                    horizontal_margin: 1.0 / 16.0,
                },
                Err(error) => TerminalClearance::RequiresReplan {
                    reason: error.to_string(),
                },
            }
        } else {
            TerminalClearance::RequiresReplan {
                reason: "terminal motion must be released and resting".into(),
            }
        };
        Ok(MotionPreview {
            initial,
            world_revision: world.revision(),
            initial_frame,
            controls: controls.to_vec(),
            frames,
            terminal_clearance,
        })
    }
}
#[derive(Clone)]
pub(super) struct NativeMotionRun {
    record: MotionRecord,
    expected_motion_revision: u64,
    movement_attribute: Option<Attribute>,
}
impl Bot {
    pub(super) async fn common_motion_pauses_physics(&self) -> bool {
        self.common_motion
            .lock()
            .await
            .as_ref()
            .is_some_and(|run| !run.record.status.is_continuation_candidate())
    }
    pub(super) async fn interrupt_common_motion(&self, problem: &str) {
        if let Some(run) = self.common_motion.lock().await.as_mut() {
            run.record.status = MotionStatus::RequiresInspection;
            run.record
                .problem
                .get_or_insert_with(|| problem.to_string());
        }
    }
    pub(super) async fn common_motion_admission(&self) -> Result<()> {
        let run = self.common_motion.lock().await.clone();
        if let Some(run) = run {
            if run.record.status != MotionStatus::Predicted {
                return Err(motion_state(
                    "common motion unresolved; inspect retained run without replay",
                ));
            }
            let check = async {
                self.legacy_stable_run(&run, false).await?;
                let end = run.record.preview.frames.last().unwrap();
                let world = self.world.lock().await;
                legacy_clearance(
                    &|p| legacy_motion_block(&world, p),
                    end.position,
                    1.0 / 16.0,
                )
            }
            .await;
            if let Err(error) = check {
                self.interrupt_common_motion(&error.to_string()).await;
                return Err(error);
            }
        }
        Ok(())
    }
    pub(crate) async fn common_start_predicted_path(
        &self,
        controls: &[SurvivalControl],
    ) -> Result<MotionRecord> {
        model::validate_controls(controls)?;
        let _gate = self.coherent_state_gate.lock().await;
        self.common_motion_admission().await?;
        // Read the actor revision before capture; a competing native operation
        // between this boundary and exclusive acquisition refuses before sends.
        let revision = self
            .connection
            .motion_admission_revision()
            .await
            .map_err(|e| motion_state(&format!("bounded admission rejected: {e:?}")))?;
        let preview = self.common_preview_unlocked(controls).await?;
        if preview.initial.received_pose.is_none() {
            return Err(motion_state(
                "finite motion requires native own-pose receipt",
            ));
        }
        if !matches!(
            preview.terminal_clearance,
            TerminalClearance::Admitted { .. }
        ) {
            return Err(motion_state(
                "finite controls must end in released rest with full dry support",
            ));
        }
        let run_id = self
            .common_motion
            .lock()
            .await
            .as_ref()
            .map_or(Some(1), |run| run.record.run_id.checked_add(1))
            .ok_or_else(|| motion_state("motion run IDs exhausted"))?;
        let movement_attribute = legacy_movement_attribute(&**self.survival.read().await).cloned();
        let run = NativeMotionRun {
            expected_motion_revision: self.motion.lock().await.revision(),
            movement_attribute,
            record: MotionRecord {
                session: preview.initial.session,
                run_id,
                preview,
                attempted_tick: 0,
                dispatched_ticks: 0,
                status: MotionStatus::Running,
                problem: None,
            },
        };
        *self.common_motion.lock().await = Some(run.clone());
        let owner = self.clone_internal();
        let record = run.record.clone();
        // No await between retaining the intent and spawning its finite owner.
        tokio::spawn(async move {
            if let Err(error) = owner.run_common_path(run.clone(), revision).await {
                let _gate = owner.coherent_state_gate.lock().await;
                if let Some(current) = owner
                    .common_motion
                    .lock()
                    .await
                    .as_mut()
                    .filter(|current| current.record.run_id == run.record.run_id)
                {
                    current.record.status = MotionStatus::RequiresInspection;
                    current
                        .record
                        .problem
                        .get_or_insert_with(|| error.to_string());
                }
            }
        });
        Ok(record)
    }
    pub(crate) async fn common_motion_record(&self) -> Result<Option<MotionRecord>> {
        let _gate = self.coherent_state_gate.lock().await;
        let run = self.common_motion.lock().await.clone();
        if let Some(run) = &run {
            if run.record.status == MotionStatus::Predicted {
                let problem = if self.connection_state() != ConnectionState::Ready {
                    Some(motion_state("connection closed after finite motion"))
                } else {
                    self.legacy_stable_run(run, false).await.err()
                };
                if let Some(problem) = problem {
                    let mut current = self.common_motion.lock().await;
                    let current = current.as_mut().unwrap();
                    current.record.status = MotionStatus::RequiresInspection;
                    current
                        .record
                        .problem
                        .get_or_insert_with(|| problem.to_string());
                }
            }
        }
        Ok(self
            .common_motion
            .lock()
            .await
            .as_ref()
            .map(|run| run.record.clone()))
    }
    async fn legacy_stable_run(
        &self,
        run: &NativeMotionRun,
        exact_motion_revision: bool,
    ) -> Result<()> {
        if self.connection_state() != ConnectionState::Ready || self.stopped.load(Ordering::Acquire)
        {
            return Err(motion_state("connection no longer ready"));
        }
        let receipts = self.common_receipts.lock().await;
        if receipts.generation != run.record.session.world_generation
            || receipts.pose.as_ref().map(|p| p.receive_sequence)
                != run
                    .record
                    .preview
                    .initial
                    .received_pose
                    .as_ref()
                    .map(|p| p.receive_sequence)
            || receipts.pending_dispatch
            || receipts.requested_flying
        {
            return Err(motion_state(
                "world/pose correction or other unresolved dispatch interrupted motion",
            ));
        }
        drop(receipts);
        let survival = self.survival.read().await;
        if survival.game_mode.map(|id| id & 7) != Some(0)
            || survival.flying
            || !survival.effects.is_empty()
            || !survival.vitals.as_ref().is_some_and(|v| v.health > 0.0)
            || legacy_movement_attribute(&survival) != run.movement_attribute.as_ref()
            || *self.local_pose.lock().await != Some(0)
        {
            return Err(motion_state(
                "mode, health, pose, flight, effects or motion attributes changed",
            ));
        }
        drop(survival);
        if self.control().await != ControlState::default() {
            return Err(motion_state(
                "persistent controls replaced during finite motion",
            ));
        }
        let dispatched = run.record.dispatched_ticks;
        let expected = if dispatched == 0 {
            &run.record.preview.initial_frame
        } else {
            &run.record.preview.frames[usize::from(dispatched) - 1]
        };
        let player = self.player.lock().await;
        let motion = self.motion.lock().await;
        if [player.x, player.y, player.z] != expected.position
            || player.on_ground != expected.on_ground
            || (exact_motion_revision && motion.revision() != run.expected_motion_revision)
            || motion.velocity.x != expected.velocity[0]
            || motion.velocity.z != expected.velocity[2]
            || (motion.velocity.y - expected.velocity[1]).abs()
                > if exact_motion_revision { 0.0 } else { 1e-8 }
        {
            return Err(motion_state(
                "position, impulse or local model state changed during finite motion",
            ));
        }
        Ok(())
    }
    async fn run_common_path(&self, mut run: NativeMotionRun, revision: u64) -> Result<()> {
        self.connection
            .begin_bounded_motion(run.record.run_id, revision)
            .await
            .map_err(|e| motion_state(&format!("exclusive motion acquisition rejected: {e:?}")))?;
        let mut model = model::Model::new(
            crate::MinecraftVersion::Java1_16_1,
            run.record.preview.initial_frame.position,
        );
        model.frame = run.record.preview.initial_frame.clone();
        let mut interval = tokio::time::interval(Duration::from_millis(50));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        for (control, expected) in run
            .record
            .preview
            .controls
            .clone()
            .into_iter()
            .zip(run.record.preview.frames.clone())
        {
            interval.tick().await;
            let _gate = self.coherent_state_gate.lock().await;
            self.legacy_stable_run(&run, true).await?;
            if self
                .common_motion
                .lock()
                .await
                .as_ref()
                .is_none_or(|current| {
                    current.record.run_id != run.record.run_id
                        || current.record.status != MotionStatus::Running
                })
            {
                return Err(motion_state("finite run superseded"));
            }
            let proposed = model.intent(control.input, control.yaw);
            {
                let world = self.world.lock().await;
                let boxes = model::geometry(
                    &|p| legacy_motion_block(&world, p),
                    model.frame.position,
                    proposed,
                )?;
                model.advance(control.input, proposed, &boxes);
            }
            if model.frame != expected {
                return Err(motion_state(
                    "changed received geometry changed predicted path; inspect before further input",
                ));
            }
            self.common_motion
                .lock()
                .await
                .as_mut()
                .unwrap()
                .record
                .attempted_tick = expected.tick;
            let mut payload = Vec::with_capacity(33);
            for value in expected.position {
                payload.extend(value.to_be_bytes());
            }
            payload.extend(control.yaw.to_be_bytes());
            payload.extend(0.0f32.to_be_bytes());
            payload.push(u8::from(expected.on_ground));
            self.connection
                .bounded_position(run.record.run_id, payload)
                .await?;
            let mut player = self.player.lock().await;
            player.x = expected.position[0];
            player.y = expected.position[1];
            player.z = expected.position[2];
            player.yaw = control.yaw;
            player.pitch = 0.0;
            player.on_ground = expected.on_ground;
            let mut motion = self.motion.lock().await;
            let previous = **motion;
            let previous_y = if run.record.dispatched_ticks == 0 {
                run.record.preview.initial_frame.position[1]
            } else {
                run.record.preview.frames[usize::from(run.record.dispatched_ticks) - 1].position[1]
            };
            **motion = MotionState {
                velocity: Vec3 {
                    x: expected.velocity[0],
                    y: expected.velocity[1],
                    z: expected.velocity[2],
                },
                collided_horizontal: expected.horizontal_collision,
                collided_vertical: expected.on_ground,
                ticks: previous.ticks + 1,
                fall_distance: if expected.on_ground {
                    0.0
                } else {
                    previous.fall_distance + (previous_y - expected.position[1]).max(0.0) as f32
                },
            };
            run.expected_motion_revision = motion.revision();
            run.record.attempted_tick = expected.tick;
            run.record.dispatched_ticks = expected.tick;
            *self.common_motion.lock().await = Some(run.clone());
            self.common_receipts.lock().await.position_source =
                Some(crate::client::ValueSource::Predicted);
        }
        let _gate = self.coherent_state_gate.lock().await;
        self.legacy_stable_run(&run, true).await?;
        {
            let world = self.world.lock().await;
            legacy_clearance(
                &|p| legacy_motion_block(&world, p),
                run.record.preview.frames.last().unwrap().position,
                1.0 / 16.0,
            )?;
        }
        self.connection
            .finish_bounded_motion(run.record.run_id)
            .await
            .map_err(|e| motion_state(&format!("motion completion rejected: {e:?}")))?;
        self.common_motion
            .lock()
            .await
            .as_mut()
            .unwrap()
            .record
            .status = MotionStatus::Predicted;
        Ok(())
    }
}
fn legacy_movement_attribute(survival: &SurvivalState) -> Option<&Attribute> {
    survival
        .attributes
        .get("minecraft:generic.movement_speed")
        .or_else(|| survival.attributes.get("generic.movement_speed"))
}

fn legacy_motion_block(
    world: &super::World,
    position: [i32; 3],
) -> Result<crate::NativeBlockState> {
    if !(0..=255).contains(&position[1])
        || position[0].abs_diff(0) > 30_000_000
        || position[2].abs_diff(0) > 30_000_000
    {
        return Err(motion_state("motion geometry crosses native world bounds"));
    }
    let state = world
        .block(position[0], position[1], position[2])
        .ok_or_else(|| motion_state("motion geometry is not loaded"))?;
    crate::versions::java_1_16_1::native_state(state)
}
fn legacy_clearance(
    block_at: &impl Fn([i32; 3]) -> Result<crate::NativeBlockState>,
    position: [f64; 3],
    margin: f64,
) -> Result<()> {
    let mut bounds = model::body(position);
    bounds[0] -= margin;
    bounds[2] -= margin;
    bounds[3] += margin;
    bounds[5] += margin;
    let cubes = model::geometry(block_at, position, [0.0; 3])?;
    if cubes.iter().any(|cube| {
        (0..3).all(|axis| {
            bounds[axis] + 1e-7 < cube[axis + 3] && bounds[axis + 3] - 1e-7 > cube[axis]
        })
    }) {
        return Err(motion_state(
            "standing body or reserve intersects a dry cube",
        ));
    }
    let supported_area: f64 = cubes
        .iter()
        .filter(|cube| (cube[4] - position[1]).abs() < 1e-7)
        .map(|cube| {
            (bounds[3].min(cube[3]) - bounds[0].max(cube[0])).max(0.0)
                * (bounds[5].min(cube[5]) - bounds[2].max(cube[2])).max(0.0)
        })
        .sum();
    if supported_area + 1e-7 < (bounds[3] - bounds[0]) * (bounds[5] - bounds[2]) {
        return Err(motion_state(
            "standing body and reserve need full known dry support",
        ));
    }
    Ok(())
}
fn motion_state(message: &str) -> crate::Error {
    crate::Error::new(crate::ErrorKind::State, anyhow::anyhow!("{message}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn seed_motion(bot: &Bot) {
        bot.teleport_barrier_ticks.store(u8::MAX, Ordering::Release);
        bot.survival.write().await.game_mode = Some(0);
        *bot.local_pose.lock().await = Some(0);
        let mut health = 20.0f32.to_be_bytes().to_vec();
        put_varint(&mut health, 20);
        health.extend(5.0f32.to_be_bytes());
        bot.apply_packet(0x49, health).await.unwrap();
        {
            let mut player = bot.player.lock().await;
            player.x = 8.5;
            player.y = 65.0;
            player.z = 8.5;
            player.on_ground = true;
        }
        bot.common_receipts.lock().await.pose = Some(crate::client::ReceivedPose {
            position: [8.5, 65.0, 8.5],
            rotation: [0.0; 2],
            receive_sequence: 1,
        });
        bot.world.lock().await.apply_chunk(&[0; 14], 256).unwrap();
        for x in 0..16 {
            for z in 0..16 {
                bot.world
                    .lock()
                    .await
                    .set_block_for_test(BlockPos { x, y: 64, z }, 1);
            }
        }
    }
    #[tokio::test]
    async fn common_motion_uses_legacy_rules_and_retained_connection_owned_dispatch() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed_motion(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        crate::client::tests::common_motion_preview_scenario(&client).await;
        crate::client::tests::common_motion_dispatch_scenario(&client).await;
        for _ in 0..37 {
            let (id, bytes) = timeout(Duration::from_secs(1), packets.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(id, 0x13);
            assert_eq!(bytes.len(), 33);
            assert!(bytes[32] <= 1); // legacy onGround only; no modern collision bit.
        }
        *bot.local_pose.lock().await = Some(5);
        assert!(
            client
                .survival()
                .preview_path(&[SurvivalControl {
                    yaw: 0.0,
                    input: Default::default()
                }])
                .await
                .is_err()
        );
        *bot.local_pose.lock().await = Some(0);
        bot.survival.write().await.game_mode = Some(1);
        assert!(
            client
                .survival()
                .preview_path(&[SurvivalControl {
                    yaw: 0.0,
                    input: Default::default()
                }])
                .await
                .is_err()
        );
        let mut mode = vec![3];
        mode.extend(1.0f32.to_be_bytes());
        bot.apply_packet(0x1e, mode).await.unwrap();
        let retained = client.survival().motion_record().await.unwrap().unwrap();
        assert_eq!(retained.status, MotionStatus::RequiresInspection);
        assert!(
            client
                .survival()
                .start_predicted_path(&[SurvivalControl {
                    yaw: 0.0,
                    input: Default::default()
                }])
                .await
                .is_err()
        );
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn finite_motion_retains_attempt_before_io_and_caller_cancellation() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed_motion(&bot).await;
        let writer = bot.writer.lock().await;
        let controls = [SurvivalControl {
            yaw: 0.0,
            input: Default::default(),
        }; 2];
        let started = bot.common_start_predicted_path(&controls).await.unwrap();
        timeout(Duration::from_secs(1), async {
            loop {
                if bot
                    .common_motion
                    .lock()
                    .await
                    .as_ref()
                    .unwrap()
                    .record
                    .attempted_tick
                    == 1
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let intent = bot
            .common_motion
            .lock()
            .await
            .as_ref()
            .unwrap()
            .record
            .clone();
        assert_eq!(intent.dispatched_ticks, 0);
        assert_eq!(intent.run_id, started.run_id);
        assert!(
            timeout(Duration::from_millis(30), packets.recv())
                .await
                .is_err()
        );
        // A cancelled read wait cannot release the actor's retained send.
        let inspected_bot = bot.clone();
        let waiter = tokio::spawn(async move { inspected_bot.common_motion_record().await });
        tokio::task::yield_now().await;
        waiter.abort();
        drop(writer);
        for _ in 0..2 {
            assert_eq!(
                timeout(Duration::from_secs(1), packets.recv())
                    .await
                    .unwrap()
                    .unwrap()
                    .0,
                0x13
            );
        }
        timeout(Duration::from_secs(1), async {
            loop {
                let record = bot.common_motion_record().await.unwrap().unwrap();
                if record.status != MotionStatus::Running {
                    assert_eq!(
                        record.status,
                        MotionStatus::Predicted,
                        "{:?}",
                        record.problem
                    );
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // Even a zero impulse must invalidate the declared motion basis.
        bot.player.lock().await.entity_id = Some(42);
        let mut velocity = vec![42];
        velocity.extend([0; 6]);
        bot.apply_packet(0x46, velocity).await.unwrap();
        let record = bot.common_motion_record().await.unwrap().unwrap();
        assert_eq!(record.status, MotionStatus::RequiresInspection);
        assert!(bot.common_start_predicted_path(&controls).await.is_err());
        drop(release);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn finite_motion_geometry_change_stops_without_replaying_dispatched_tick() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed_motion(&bot).await;
        let controls: Vec<_> = (0..35)
            .map(|tick| SurvivalControl {
                yaw: 0.0,
                input: crate::client::survival::SurvivalInput {
                    forward: i8::from(tick < 5),
                    ..Default::default()
                },
            })
            .collect();
        bot.common_start_predicted_path(&controls).await.unwrap();
        timeout(Duration::from_secs(1), packets.recv())
            .await
            .unwrap()
            .unwrap();
        {
            let _gate = bot.coherent_state_gate.lock().await;
            bot.world
                .lock()
                .await
                .set_block_for_test(BlockPos { x: 8, y: 65, z: 9 }, 1);
        }
        let record = timeout(Duration::from_secs(2), async {
            loop {
                let record = bot.common_motion_record().await.unwrap().unwrap();
                if record.status != MotionStatus::Running {
                    break record;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(record.status, MotionStatus::RequiresInspection);
        assert!(record.dispatched_ticks < 35);
        assert!(record.problem.unwrap().contains("geometry"));
        assert!(bot.common_start_predicted_path(&controls).await.is_err());
        while packets.try_recv().is_ok() {}
        assert!(
            timeout(Duration::from_millis(80), packets.recv())
                .await
                .is_err()
        );
        drop(release);
        drop(bot);
        server.await.unwrap();
    }
}
