//! Common bounded preview with legacy-native rules and coherent legacy admission.
use super::*;
use crate::client::{
    GameMode,
    survival::{
        MotionPreview, MotionRecord, MotionStatus, SurvivalControl, TerminalClearance, model,
    },
};

#[derive(Clone, Copy)]
pub(super) enum CommonOwner {
    ContainerOpen(crate::client::container::ContainerOpenId),
    Mining(crate::client::survival::MiningId),
    Placement(crate::client::survival::PlacementId),
}
impl CommonOwner {
    fn session(self) -> crate::client::SessionStamp {
        match self {
            Self::ContainerOpen(id) => id.session(),
            Self::Mining(id) => id.session(),
            Self::Placement(id) => id.session(),
        }
    }
}
impl Bot {
    pub(super) async fn common_target_unlocked(
        &self,
        distance: f64,
        mining_owner: Option<crate::client::survival::MiningId>,
    ) -> Result<crate::client::survival::BlockTargetObservation> {
        self.common_target_with_owner(distance, mining_owner.map(CommonOwner::Mining))
            .await
    }
    pub(super) async fn common_target_with_owner(
        &self,
        distance: f64,
        owner: Option<CommonOwner>,
    ) -> Result<crate::client::survival::BlockTargetObservation> {
        self.common_target_in_mode(distance, owner, GameMode::Survival)
            .await
    }
    pub(super) async fn common_target_in_mode(
        &self,
        distance: f64,
        owner: Option<CommonOwner>,
        mode: GameMode,
    ) -> Result<crate::client::survival::BlockTargetObservation> {
        use crate::client::survival::target;
        let preview = self
            .common_preview_in_mode(
                &[SurvivalControl {
                    yaw: 0.0,
                    input: Default::default(),
                }],
                owner,
                mode,
            )
            .await?;
        let mut eye = preview.initial_frame.position;
        target::validate_rotation(preview.initial.rotation)?;
        eye[1] += f64::from(1.62f32);
        let direction = target::direction(
            crate::MinecraftVersion::Java1_16_1,
            preview.initial.rotation,
        );
        let end = std::array::from_fn(|i| eye[i] + direction[i] * distance);
        let world = self.world.lock().await;
        let hit = target::cast(
            eye,
            end,
            |p| legacy_motion_block(&world, p).map_err(anyhow::Error::from),
            |state| {
                const EMPTY: &[[f64; 6]] = &[];
                const CUBE: &[[f64; 6]] = &[[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]];
                if let Some(shapes) = crate::client::container::outline::lookup(
                    crate::MinecraftVersion::Java1_16_1,
                    state,
                ) {
                    Ok(shapes)
                } else if matches!(
                    state.name.as_str(),
                    "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
                ) {
                    Ok((EMPTY, EMPTY))
                } else if model::DRY_CUBES.contains(&state.name.as_str()) {
                    Ok((CUBE, EMPTY))
                } else {
                    Err(crate::Error::new(
                        crate::ErrorKind::Unsupported,
                        anyhow::anyhow!("legacy outline shape not audited: {}", state.name),
                    )
                    .into())
                }
            },
        )
        .map_err(|e| {
            let kind = e
                .downcast_ref::<crate::Error>()
                .map_or(crate::ErrorKind::State, crate::Error::kind);
            crate::Error::new(kind, e)
        })?;
        Ok(crate::client::survival::BlockTargetObservation {
            initial: preview.initial,
            world_revision: world.revision(),
            eye,
            maximum_distance: distance,
            hit,
        })
    }
    pub(super) async fn common_preview_core(
        &self,
        controls: &[SurvivalControl],
        mining_owner: Option<crate::client::survival::MiningId>,
    ) -> Result<MotionPreview> {
        self.common_preview_with_owner(controls, mining_owner.map(CommonOwner::Mining))
            .await
    }
    pub(super) async fn common_preview_with_owner(
        &self,
        controls: &[SurvivalControl],
        owner: Option<CommonOwner>,
    ) -> Result<MotionPreview> {
        self.common_preview_in_mode(controls, owner, GameMode::Survival)
            .await
    }
    async fn common_preview_in_mode(
        &self,
        controls: &[SurvivalControl],
        owner: Option<CommonOwner>,
        mode: GameMode,
    ) -> Result<MotionPreview> {
        if self.connection_state() != ConnectionState::Ready {
            return Err(motion_state("connection not ready"));
        }
        if self.common_control.lock().await.active() {
            return Err(motion_state(
                "continuous control owns the player's movement; stop it before standing operations",
            ));
        }
        let initial = self.common_player_unlocked().await?;
        if self
            .common_receipts
            .lock()
            .await
            .vehicles
            .motion_interrupted()
        {
            return Err(motion_state(
                "received mounted context requires a fresh world motion baseline",
            ));
        }
        let owns_operation = owner.is_some_and(|o| o.session() == initial.session)
            && match owner {
                Some(CommonOwner::ContainerOpen(id)) => {
                    super::lock_packet_state(&self.common_container_open)
                        .await
                        .as_ref()
                        .is_some_and(|o| o.record.id == id && !o.released)
                }
                Some(CommonOwner::Mining(id)) => super::lock_packet_state(&self.common_mining)
                    .await
                    .as_ref()
                    .is_some_and(|m| m.record.id == id),
                Some(CommonOwner::Placement(id)) => {
                    super::lock_packet_state(&self.common_placement)
                        .await
                        .as_ref()
                        .is_some_and(|m| m.record.id == id && !m.released)
                }
                None => false,
            }
            && !self.common_receipts.lock().await.pending_dispatch
            && self
                .common_motion
                .lock()
                .await
                .as_ref()
                .is_none_or(|m| m.record.status.is_continuation_candidate());
        if initial.pending_dispatch && !owns_operation {
            return Err(motion_state("prior common dispatch unresolved"));
        }
        if initial.game_mode != Some(mode)
            || !initial
                .health
                .as_ref()
                .is_some_and(|health| health.value.health > 0.0)
            || *self.local_pose.lock().await != Some(0)
        {
            return Err(motion_state(
                "stationary geometry requires healthy matching received mode and native standing pose",
            ));
        }
        let submitted_flight_stop = self.common_submitted_flight_stop().await;
        let survival = self.survival.read().await;
        if (survival.flying && !submitted_flight_stop)
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
    pub(super) record: MotionRecord,
    flight_stop: Option<crate::client::ObservedValue<u8>>,
    pub(super) expected_motion_revision: u64,
    movement_attribute: Option<Attribute>,
}
impl Bot {
    // Native automatic ground physics must not predict a standing pose during
    // or after mounted motion. This is separate from pending dispatch: a received
    // mount can still admit an explicit owned dismount.
    pub(super) async fn common_native_physics_paused(&self) -> bool {
        let mounted_history = self
            .common_receipts
            .lock()
            .await
            .vehicles
            .motion_interrupted();
        mounted_history || self.common_motion_pauses_physics().await
    }
    pub(super) async fn common_motion_pauses_physics(&self) -> bool {
        if self.common_control.lock().await.active() {
            return true;
        }
        if crate::client::vehicle::control::unresolved(&self.vehicle_control_history) {
            return true;
        }
        if crate::client::flight::unresolved(&self.flight_history) {
            return true;
        }
        if self
            .dismount_history
            .lock()
            .expect("dismount history")
            .as_ref()
            .is_some_and(|r| r.unresolved())
        {
            return true;
        }
        if super::lock_packet_state(&self.common_container_open)
            .await
            .as_ref()
            .is_some_and(|o| !o.released)
        {
            return true;
        }
        if super::lock_packet_state(&self.common_container_close)
            .await
            .as_ref()
            .is_some_and(|r| r.unresolved())
        {
            return true;
        }
        if super::lock_packet_state(&self.common_inventory_swap)
            .await
            .as_ref()
            .is_some_and(|s| !s.released)
            || super::lock_packet_state(&self.common_inventory_click)
                .await
                .as_ref()
                .is_some_and(|s| !s.released)
            || super::lock_packet_state(&self.common_recipe_placement)
                .await
                .as_ref()
                .is_some_and(|s| !s.released)
            || super::lock_packet_state(&self.common_crafting_take)
                .await
                .as_ref()
                .is_some_and(|s| !s.released)
            || super::lock_packet_state(&self.common_inventory_transfer)
                .await
                .as_ref()
                .is_some_and(|s| !s.released)
            || super::lock_packet_state(&self.common_mining)
                .await
                .is_some()
            || super::lock_packet_state(&self.common_placement)
                .await
                .as_ref()
                .is_some_and(|p| !p.released)
        {
            return true;
        }
        self.common_motion
            .lock()
            .await
            .as_ref()
            .is_some_and(|run| !run.record.status.is_continuation_candidate())
    }
    pub(super) async fn interrupt_common_motion(&self, problem: &str) {
        if let Some(o) = super::lock_packet_state(&self.common_container_open)
            .await
            .as_mut()
            .filter(|o| !o.released)
        {
            o.record.inspection(problem);
        }
        if let Some(r) = super::lock_packet_state(&self.common_container_close)
            .await
            .as_mut()
        {
            r.inspection(problem);
        }
        self.interrupt_common_motion_operations(problem).await;
    }
    // Position packets revalidate stationary containers against actual values
    // after decoding. Finite movement and other operations still retain every
    // native correction as an interruption, including an identical refresh.
    pub(super) async fn interrupt_common_motion_operations(&self, problem: &str) {
        self.interrupt_common_mining(problem).await;
        self.interrupt_common_placement(problem).await;
        self.interrupt_common_inventory_swap(problem).await;
        self.interrupt_common_inventory_click(problem).await;
        self.interrupt_common_crafting_take(problem).await;
        self.interrupt_common_recipe_placement(problem).await;
        self.interrupt_common_inventory_transfer(problem).await;
        if let Some(run) = self.common_motion.lock().await.as_mut() {
            run.record.status = MotionStatus::RequiresInspection;
            run.record
                .problem
                .get_or_insert_with(|| problem.to_string());
        }
    }
    pub(super) async fn common_motion_admission(&self) -> Result<()> {
        self.common_motion_admission_inner(false).await
    }
    pub(super) async fn common_motion_admission_inner(&self, flight_owner: bool) -> Result<()> {
        self.common_motion_admission_owned(flight_owner, None).await
    }
    pub(super) async fn common_motion_admission_owned(
        &self,
        flight_owner: bool,
        ground_owner: Option<crate::client::DismountId>,
    ) -> Result<()> {
        if crate::client::vehicle::control::unresolved(&self.vehicle_control_history) {
            return Err(motion_state(
                "vehicle control unresolved; inspect without replay",
            ));
        }
        if !flight_owner && crate::client::flight::unresolved(&self.flight_history) {
            return Err(motion_state(
                "flight dispatch unresolved; inspect without replay",
            ));
        }
        if self
            .dismount_history
            .lock()
            .expect("dismount history")
            .as_ref()
            .is_some_and(|r| r.unresolved() && Some(r.id) != ground_owner)
        {
            return Err(motion_state(
                "dismount input unresolved; inspect and explicitly complete without replay",
            ));
        }
        if super::lock_packet_state(&self.common_container_open)
            .await
            .as_ref()
            .is_some_and(|o| !o.released)
        {
            return Err(motion_state(
                "common container activation unresolved; inspect without replay",
            ));
        }
        if super::lock_packet_state(&self.common_container_close)
            .await
            .as_ref()
            .is_some_and(|r| r.unresolved())
        {
            return Err(motion_state(
                "common container close unresolved; inspect without replay",
            ));
        }
        if super::lock_packet_state(&self.common_inventory_swap)
            .await
            .as_ref()
            .is_some_and(|s| !s.released)
        {
            return Err(motion_state(
                "common inventory swap unresolved; inspect without replay",
            ));
        }
        if super::lock_packet_state(&self.common_inventory_click)
            .await
            .as_ref()
            .is_some_and(|s| !s.released)
        {
            return Err(motion_state(
                "common inventory click unresolved; inspect without replay",
            ));
        }
        if super::lock_packet_state(&self.common_recipe_placement)
            .await
            .as_ref()
            .is_some_and(|s| !s.released)
        {
            return Err(motion_state(
                "common recipe placement unresolved; inspect without replay",
            ));
        }
        if super::lock_packet_state(&self.common_crafting_take)
            .await
            .as_ref()
            .is_some_and(|s| !s.released)
        {
            return Err(motion_state(
                "common crafting take unresolved; inspect without replay",
            ));
        }
        if super::lock_packet_state(&self.common_inventory_transfer)
            .await
            .as_ref()
            .is_some_and(|s| !s.released)
        {
            return Err(motion_state(
                "common inventory transfer unresolved; inspect without replay",
            ));
        }
        if super::lock_packet_state(&self.common_mining)
            .await
            .is_some()
        {
            return Err(motion_state(
                "common mining retained; inspect and use explicit fresh recovery before continuation",
            ));
        }
        if super::lock_packet_state(&self.common_placement)
            .await
            .as_ref()
            .is_some_and(|p| !p.released)
        {
            return Err(motion_state(
                "common placement unresolved; inspect target/material receipts without replay",
            ));
        }
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
    async fn legacy_stable_run(
        &self,
        run: &NativeMotionRun,
        exact_motion_revision: bool,
    ) -> Result<()> {
        let problem = self
            .dismount_history
            .lock()
            .expect("dismount history")
            .as_ref()
            .and_then(|r| r.grounding.as_ref())
            .filter(|g| {
                g.motion.session == run.record.session && g.motion.run_id == run.record.run_id
            })
            .and_then(|g| g.motion.problem.clone());
        if let Some(problem) = problem {
            return Err(motion_state(&problem));
        }
        if self.connection_state() != ConnectionState::Ready || self.stopped.load(Ordering::Acquire)
        {
            return Err(motion_state("connection no longer ready"));
        }
        let receipts = self.common_receipts.lock().await;
        if receipts.generation != run.record.session.world_generation
            || receipts.vehicles.motion_interrupted()
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
        let submitted_flight_stop = run
            .flight_stop
            .as_ref()
            .is_some_and(|s| Some(s) == receipts.abilities.as_ref());
        drop(receipts);
        let survival = self.survival.read().await;
        if survival
            .game_mode
            .and_then(|id| GameMode::decode(id & 7).ok())
            != run.record.preview.initial.game_mode
            || (survival.flying && !submitted_flight_stop)
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
    fn sync_owned_motion(&self, record: &MotionRecord) {
        crate::client::flight::sync_motion(&self.flight_history, record);
        crate::client::vehicle::dismount::sync_motion(&self.dismount_history, record);
    }
    pub(super) async fn run_common_path(
        &self,
        mut run: NativeMotionRun,
        revision: u64,
    ) -> Result<()> {
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
                    crate::MinecraftVersion::Java1_16_1,
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
            self.sync_owned_motion(&self.common_motion.lock().await.as_ref().unwrap().record);
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
            self.sync_owned_motion(&run.record);
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
        self.sync_owned_motion(&self.common_motion.lock().await.as_ref().unwrap().record);
        Ok(())
    }
}
pub(super) fn legacy_movement_attribute(survival: &SurvivalState) -> Option<&Attribute> {
    survival
        .attributes
        .get("minecraft:generic.movement_speed")
        .or_else(|| survival.attributes.get("generic.movement_speed"))
}

pub(super) fn legacy_motion_block(
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
pub(super) fn legacy_clearance(
    block_at: &impl Fn([i32; 3]) -> Result<crate::NativeBlockState>,
    position: [f64; 3],
    margin: f64,
) -> Result<()> {
    let mut bounds = model::body(position);
    bounds[0] -= margin;
    bounds[2] -= margin;
    bounds[3] += margin;
    bounds[5] += margin;
    let cubes = model::geometry(
        crate::MinecraftVersion::Java1_16_1,
        block_at,
        position,
        [0.0; 3],
    )?;
    if cubes.iter().any(|cube| {
        (0..3).all(|axis| {
            bounds[axis] + 1e-7 < cube[axis + 3] && bounds[axis + 3] - 1e-7 > cube[axis]
        })
    }) {
        return Err(motion_state(
            "standing body or reserve intersects known dry terrain",
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

impl Bot {
    async fn common_submitted_flight_stop(&self) -> bool {
        let stop = self
            .common_motion
            .lock()
            .await
            .as_ref()
            .and_then(|r| r.flight_stop.clone());
        let receipts = self.common_receipts.lock().await;
        !receipts.requested_flying
            && stop
                .as_ref()
                .is_some_and(|s| Some(s) == receipts.abilities.as_ref())
    }
    pub(super) async fn landing_plan(
        &self,
        initial: crate::client::PlayerObservation,
    ) -> Result<NativeMotionRun> {
        let state = self.survival.read().await;
        if *self.local_pose.lock().await != Some(0)
            || !state.effects.is_empty()
            || legacy_movement_attribute(&state).is_some_and(|a| a.value() as f32 != 0.1f32)
        {
            return Err(motion_state(
                "landing requires standing dry default motion without received effects",
            ));
        }
        let movement_attribute = legacy_movement_attribute(&state).cloned();
        drop(state);
        let position = initial.position.as_ref().unwrap().value;
        let motion = self.motion.lock().await;
        if motion.velocity.x != 0.0
            || motion.velocity.z != 0.0
            || (motion.velocity.y != 0.0
                && (motion.velocity.y + 0.08 * f64::from(0.98f32)).abs() > 1e-8)
        {
            return Err(motion_state(
                "landing refuses unresolved impulse in native local motion",
            ));
        }
        let revision = motion.revision();
        drop(motion);
        let controls = vec![
            SurvivalControl {
                yaw: initial.rotation[0],
                input: Default::default()
            };
            2
        ];
        let mut model = model::Model::new(crate::MinecraftVersion::Java1_16_1, position);
        let initial_frame = model.initial_frame();
        let world = self.world.lock().await;
        legacy_clearance(&|p| legacy_motion_block(&world, p), position, 1.0 / 16.0)?;
        let frames = model::predict(|p| legacy_motion_block(&world, p), &mut model, &controls)?;
        if !frames.last().unwrap().resting || frames.iter().any(|f| f.position != position) {
            return Err(motion_state(
                "landing stop model did not settle at submitted dry support",
            ));
        }
        let preview = MotionPreview {
            initial,
            world_revision: world.revision(),
            initial_frame,
            controls,
            frames,
            terminal_clearance: TerminalClearance::Admitted {
                horizontal_margin: 1.0 / 16.0,
            },
        };
        drop(world);
        let previous = self
            .retired_common_motion
            .lock()
            .await
            .as_ref()
            .map_or(0, |r| r.run_id);
        let run_id = previous
            .checked_add(1)
            .ok_or_else(|| motion_state("motion run IDs exhausted"))?;
        Ok(NativeMotionRun {
            movement_attribute,
            expected_motion_revision: revision,
            flight_stop: self.common_receipts.lock().await.abilities.clone(),
            record: MotionRecord {
                session: preview.initial.session,
                run_id,
                preview,
                attempted_tick: 0,
                dispatched_ticks: 0,
                status: MotionStatus::Running,
                problem: None,
            },
        })
    }
    pub(super) async fn dismount_ground_plan(
        &self,
        initial: crate::client::PlayerObservation,
    ) -> Result<NativeMotionRun> {
        // Reuse dry-support geometry and the finite declared local zero model;
        // retain actual abilities and velocity observations without rewriting them.
        let mut run = self.landing_plan(initial).await?;
        let receipts = self.common_receipts.lock().await;
        let requested_flying = receipts.requested_flying;
        drop(receipts);
        if requested_flying || self.survival.read().await.flying {
            return Err(motion_state("dismount ground stop refuses active flight"));
        }
        run.flight_stop = None;
        Ok(run)
    }
    pub(super) async fn install_landing_model(&self, mut run: NativeMotionRun) -> NativeMotionRun {
        // Explicit controller reset, separately recorded from every received value.
        let mut motion = self.motion.lock().await;
        motion.velocity = Vec3 {
            x: 0.,
            y: 0.,
            z: 0.,
        };
        motion.collided_horizontal = false;
        motion.collided_vertical = true;
        run.expected_motion_revision = motion.revision();
        drop(motion);
        self.player.lock().await.on_ground = true;
        *self.common_motion.lock().await = Some(run.clone());
        run
    }
}

impl Bot {
    pub(super) async fn retire_common_for_mount(&self) {
        let previous = {
            let mut motion = self.common_motion.lock().await;
            if motion.as_ref().is_some_and(|r| {
                let record = &r.record;
                record.status != MotionStatus::Running
                    && !record.preview.frames.is_empty()
                    && usize::from(record.dispatched_ticks) == record.preview.frames.len()
                    && record.attempted_tick == record.dispatched_ticks
                    && record.preview.frames.last().is_some_and(|f| f.resting)
            }) {
                motion.take()
            } else {
                None
            }
        };
        if let Some(mut previous) = previous {
            previous.record.status = MotionStatus::RequiresInspection;
            previous
                .record
                .problem
                .get_or_insert_with(|| "actual mount superseded settled ground motion".into());
            *self.retired_common_motion.lock().await = Some(previous.record);
        }
    }
}

impl crate::client::adapter::StandingQueryOps for Bot {
    async fn target_block(
        &self,
        mode: GameMode,
        distance: f64,
    ) -> Result<crate::client::survival::BlockTargetObservation> {
        use crate::client::survival::target;
        target::validate_reach(distance)?;
        let _gate = self.coherent_state_gate.lock().await;
        self.common_motion_admission().await?;
        self.common_target_in_mode(distance, None, mode).await
    }
    async fn preview_path(
        &self,
        mode: GameMode,
        controls: &[SurvivalControl],
    ) -> Result<MotionPreview> {
        model::validate_controls(controls)?;
        let _gate = self.coherent_state_gate.lock().await;
        self.common_motion_admission().await?;
        self.common_preview_in_mode(controls, None, mode).await
    }
}

impl crate::client::adapter::PathMotionOps for Bot {
    async fn start_predicted_path(
        &self,
        mode: GameMode,
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
        let preview = self.common_preview_in_mode(controls, None, mode).await?;
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
        let previous_id = self
            .common_motion
            .lock()
            .await
            .as_ref()
            .map(|r| r.record.run_id)
            .or(self
                .retired_common_motion
                .lock()
                .await
                .as_ref()
                .map(|r| r.run_id));
        let run_id = previous_id
            .map_or(Some(1), |id| id.checked_add(1))
            .ok_or_else(|| motion_state("motion run IDs exhausted"))?;
        let movement_attribute = legacy_movement_attribute(&**self.survival.read().await).cloned();
        let flight_stop = self
            .common_motion
            .lock()
            .await
            .as_ref()
            .and_then(|r| r.flight_stop.clone());
        let run = NativeMotionRun {
            flight_stop,
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
    async fn motion_record(&self) -> Result<Option<MotionRecord>> {
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
        let record = self
            .common_motion
            .lock()
            .await
            .as_ref()
            .map(|run| run.record.clone());
        Ok(if record.is_some() {
            record
        } else {
            self.retired_common_motion.lock().await.clone()
        })
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::client::adapter::PathMotionOps;
    #[test]
    fn dry_terrain_standing_accepts_slab_support_and_refuses_embedded_body_or_water() {
        let mut terrain = crate::NativeBlockState {
            name: "minecraft:stone_slab".into(),
            properties: [
                ("type".into(), "bottom".into()),
                ("waterlogged".into(), "false".into()),
            ]
            .into(),
        };
        let check = |terrain: &crate::NativeBlockState| {
            legacy_clearance(
                &|cell| {
                    Ok(if cell == [0, 0, 0] {
                        terrain.clone()
                    } else {
                        crate::NativeBlockState {
                            name: "minecraft:air".into(),
                            properties: Default::default(),
                        }
                    })
                },
                [0.5, 0.5, 0.5],
                0.01,
            )
        };
        assert!(check(&terrain).is_ok());
        terrain.properties.insert("type".into(), "top".into());
        assert!(check(&terrain).is_err());
        terrain.properties.insert("type".into(), "bottom".into());
        terrain
            .properties
            .insert("waterlogged".into(), "true".into());
        assert!(check(&terrain).is_err());
    }
    pub(in crate::versions::java_1_16_1::client) async fn seed_motion(bot: &Bot) {
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
    async fn both_common_modes_target_every_audited_native_storage_state_without_dispatch() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed_motion(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        for (mode, value) in [(GameMode::Survival, 0.0f32), (GameMode::Creative, 1.0f32)] {
            let mut packet = vec![3];
            packet.extend(value.to_be_bytes());
            bot.apply_packet(0x1e, packet).await.unwrap();
            for state in crate::client::tests::common_storage_target_states(
                crate::MinecraftVersion::Java1_16_1,
            ) {
                let id = crate::versions::java_1_16_1::state_id(&state).unwrap();
                bot.world
                    .lock()
                    .await
                    .set_block_for_test(BlockPos { x: 8, y: 66, z: 11 }, id);
                crate::client::tests::common_storage_target_scenario(&client, mode, &state).await;
            }
        }
        assert!(
            timeout(Duration::from_millis(30), packets.recv())
                .await
                .is_err()
        );
        bot.survival.write().await.flying = true;
        assert!(client.creative().target_block(4.5).await.is_err());
        drop(release);
        drop(client);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn common_target_never_treats_unsupported_or_unloaded_geometry_as_air() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed_motion(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let unsupported = crate::versions::java_1_16_1::state_id(&crate::NativeBlockState {
            name: "minecraft:diamond_block".into(),
            properties: Default::default(),
        })
        .unwrap();
        bot.world
            .lock()
            .await
            .set_block_for_test(BlockPos { x: 8, y: 66, z: 11 }, unsupported);
        assert_eq!(
            client
                .survival()
                .target_block(4.5)
                .await
                .unwrap_err()
                .kind(),
            crate::ErrorKind::Unsupported
        );
        bot.world
            .lock()
            .await
            .set_block_for_test(BlockPos { x: 8, y: 66, z: 11 }, 0);
        {
            let mut player = bot.player.lock().await;
            player.x = 14.5;
            player.yaw = -90.0;
        }
        assert_eq!(
            client
                .survival()
                .target_block(4.5)
                .await
                .unwrap_err()
                .kind(),
            crate::ErrorKind::State
        );
        {
            let mut player = bot.player.lock().await;
            player.x = 8.5;
            player.yaw = f32::NAN;
        }
        assert_eq!(
            client
                .survival()
                .target_block(4.5)
                .await
                .unwrap_err()
                .kind(),
            crate::ErrorKind::State
        );
        bot.player.lock().await.yaw = 0.0;
        bot.survival.write().await.game_mode = Some(1);
        assert!(client.survival().target_block(4.5).await.is_err());
        assert!(
            timeout(Duration::from_millis(30), packets.recv())
                .await
                .is_err()
        );
        drop(release);
        drop(client);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn common_motion_uses_legacy_rules_and_retained_connection_owned_dispatch() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed_motion(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        crate::client::tests::common_motion_preview_scenario(&client).await;
        crate::client::tests::common_target_scenario(&client).await;
        let (id, bytes) = packets.recv().await.unwrap();
        assert_eq!(id, 0x13);
        assert_eq!(bytes.len(), 33);
        assert!(
            timeout(Duration::from_millis(30), packets.recv())
                .await
                .is_err()
        );
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
    async fn creative_ground_motion_keeps_mode_and_prediction_contract() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed_motion(&bot).await;
        let mut creative = vec![3];
        creative.extend(1f32.to_be_bytes());
        bot.apply_packet(0x1e, creative.clone()).await.unwrap();
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let controls = [SurvivalControl {
            yaw: 0.0,
            input: Default::default(),
        }; 2];
        assert!(client.survival().preview_path(&controls).await.is_err());
        assert!(
            client
                .survival()
                .start_predicted_path(&controls)
                .await
                .is_err()
        );
        let preview = client.creative().preview_path(&controls).await.unwrap();
        assert_eq!(preview.initial.game_mode, Some(GameMode::Creative));
        let sent = client
            .creative()
            .start_predicted_path(&controls)
            .await
            .unwrap();
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
        let completed = timeout(Duration::from_secs(2), async {
            loop {
                let r = client.creative().motion_record().await.unwrap().unwrap();
                if r.status != MotionStatus::Running {
                    break r;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(completed.run_id, sent.run_id);
        assert_eq!(completed.status, MotionStatus::Predicted);
        assert_eq!(
            completed.preview.initial.received_pose,
            preview.initial.received_pose
        );
        let mut survival = vec![3];
        survival.extend(0f32.to_be_bytes());
        bot.apply_packet(0x1e, survival).await.unwrap();
        assert_eq!(
            client
                .creative()
                .motion_record()
                .await
                .unwrap()
                .unwrap()
                .status,
            MotionStatus::RequiresInspection
        );
        bot.apply_packet(0x1e, creative).await.unwrap();
        assert!(
            client
                .creative()
                .start_predicted_path(&controls)
                .await
                .is_err()
        );
        assert!(packets.try_recv().is_err());
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
        let started = bot
            .start_predicted_path(GameMode::Survival, &controls)
            .await
            .unwrap();
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
        let waiter = tokio::spawn(async move { inspected_bot.motion_record().await });
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
                let record = bot.motion_record().await.unwrap().unwrap();
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
        let record = bot.motion_record().await.unwrap().unwrap();
        assert_eq!(record.status, MotionStatus::RequiresInspection);
        assert!(
            bot.start_predicted_path(GameMode::Survival, &controls)
                .await
                .is_err()
        );
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
        bot.start_predicted_path(GameMode::Survival, &controls)
            .await
            .unwrap();
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
                let record = bot.motion_record().await.unwrap().unwrap();
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
        assert!(
            bot.start_predicted_path(GameMode::Survival, &controls)
                .await
                .is_err()
        );
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
