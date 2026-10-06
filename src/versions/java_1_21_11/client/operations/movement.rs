//! Bounded dry-terrain prediction and separately observed native controls.
mod control;
mod endpoint;
mod scenario;
use super::geometry::GeometryView;
use super::*;
use crate::diagnostic_projection::diagnostic_record;
pub use control::{RecordedSurvivalMotionRecheck, RecordedSurvivalMotionRecord};
pub use control::{
    StandingPositionBasis, SurvivalMotionContract, SurvivalMotionRecheck, SurvivalMotionRecord,
    SurvivalMotionStatus,
};
pub(super) use control::{flight_can_retire, retire_common_for_flight, standing_basis};
pub use scenario::{
    AssumedSurvivalScene, AssumedSurvivalStart, CapturedSurvivalScene, HypotheticalAimRequirement,
    HypotheticalBlockEdit, HypotheticalMovementPreview, HypotheticalPlacement,
    HypotheticalReconnectBoundary, HypotheticalSceneSource, SurvivalScenario,
};
pub use scenario::{
    RecordedHypotheticalAimRequirement, RecordedHypotheticalBlockEdit,
    RecordedHypotheticalMovementPreview, RecordedHypotheticalPlacement,
    RecordedHypotheticalReconnectBoundary, RecordedHypotheticalSceneSource,
};

pub use crate::client::survival::{
    MAX_SURVIVAL_CONTROL_TICKS, PredictedMotionFrame, SurvivalControl, SurvivalInput,
    TerminalClearance,
};
fn fixed_controls(yaw: f32, inputs: &[SurvivalInput]) -> Result<Vec<SurvivalControl>> {
    if inputs.is_empty() || inputs.len() > MAX_SURVIVAL_CONTROL_TICKS {
        return Err(invalid("motion requires 1..120 bounded digital inputs"));
    }
    Ok(inputs
        .iter()
        .map(|input| SurvivalControl { yaw, input: *input })
        .collect())
}
diagnostic_record! {
    /// Read-only simulation against one received world snapshot. Not a reusable plan.
    #[derive(Clone, Debug, Serialize)]
    pub struct SurvivalMovementPreview => RecordedSurvivalMovementPreview {
        /// Received starting posture, attributes and world revision.
        pub initial: StandingContext,
        /// Initial model frame (tick zero), distinguishing received reset from rest.
        pub initial_frame: PredictedMotionFrame,
        /// World generation of the starting context.
        pub generation: u64,
        /// Exact per-tick heading and input; no packets were sent.
        pub controls: Vec<SurvivalControl>,
        /// Predicted frames. The world itself is not advanced into the future.
        pub frames: Vec<PredictedMotionFrame>,
        /// Prospective terminal clearance; does not authorize later sends.
        pub terminal_clearance: TerminalClearance,
    }
    diagnostic_serde {}
}
const TERMINAL_MARGIN: f64 = 1.0 / 16.0;
fn terminal_clearance(state: &impl GeometryView, frame: &PredictedMotionFrame) -> Result<()> {
    if !frame.resting {
        return Err(invalid("terminal motion must be released and resting"));
    }
    let geometry = survival::standing_geometry(
        state,
        frame.position,
        [TERMINAL_MARGIN, 0.0, TERMINAL_MARGIN],
    )?;
    if geometry.support.is_empty() {
        return Err(invalid("terminal motion lacks conservative floor support"));
    }
    Ok(())
}
impl Operations {
    pub(crate) async fn common_target_block(
        &self,
        mode: crate::client::GameMode,
        distance: f64,
    ) -> Result<crate::client::survival::BlockTargetObservation> {
        use crate::client::survival::target;
        target::validate_reach(distance)?;
        let mut state = self.bot.session.state.lock().await;
        self.ready(&state)?;
        if self.common_player_unlocked(&state)?.pending_dispatch {
            return Err(invalid(
                "prior dispatch unresolved; inspect retained record",
            ));
        }
        self.common_target_unlocked(&mut state, mode, distance)
    }
    pub(super) fn common_target_unlocked(
        &self,
        state: &mut State,
        mode: crate::client::GameMode,
        distance: f64,
    ) -> Result<crate::client::survival::BlockTargetObservation> {
        use crate::client::survival::target;
        if state.operations.game_mode != Some(mode) {
            return Err(invalid(
                "stationary geometry requires matching received mode",
            ));
        }
        let native = survival::context(
            state,
            self.bot.session.id,
            self.bot.session.started.elapsed().as_millis() as u64 / 50,
        )?;
        validate_initial(&native)?;
        let initial = self.common_player_unlocked(state)?;
        if !initial
            .health
            .as_ref()
            .is_some_and(|h| h.value.health > 0.0)
        {
            return Err(invalid(
                "stationary geometry requires received healthy player",
            ));
        }
        target::validate_rotation(initial.rotation)?;
        let eye = native.eye_position;
        let hit = super::super::raycast::stationary_outline_hit(state, eye, distance)?;
        let vector = target::direction(crate::MinecraftVersion::Java1_21_11, initial.rotation);
        let length = vector.iter().map(|v| v * v).sum::<f64>().sqrt();
        let hit = hit.map(|hit| crate::client::survival::BlockTargetHit {
            position: hit.position,
            state: hit.state,
            distance: hit.distance,
            face: [
                crate::BlockFace::Down,
                crate::BlockFace::Up,
                crate::BlockFace::North,
                crate::BlockFace::South,
                crate::BlockFace::West,
                crate::BlockFace::East,
            ][hit.face.expect("outline entry face") as usize],
            point: std::array::from_fn(|i| eye[i] + vector[i] * (hit.distance / length)),
        });
        Ok(crate::client::survival::BlockTargetObservation {
            initial,
            world_revision: state.world.revision,
            eye,
            maximum_distance: distance,
            hit,
        })
    }
    pub(crate) async fn common_preview_path(
        &self,
        mode: GameMode,
        controls: &[SurvivalControl],
    ) -> Result<crate::client::survival::MotionPreview> {
        let mut state = self.bot.session.state.lock().await;
        self.ready(&state)?;
        if self.common_player_unlocked(&state)?.pending_dispatch {
            return Err(invalid(
                "prior dispatch unresolved; inspect retained record",
            ));
        }
        let native = preview_in_mode(
            &mut state,
            self.bot.session.id,
            self.bot.session.started.elapsed().as_millis() as u64 / 50,
            controls,
            mode,
        )?;
        let initial = self.common_player_unlocked(&state)?;
        Ok(crate::client::survival::MotionPreview {
            initial,
            world_revision: native.initial.world_revision,
            initial_frame: native.initial_frame,
            controls: native.controls,
            frames: native.frames,
            terminal_clearance: native.terminal_clearance,
        })
    }
    /// Preview at most 120 known dry-terrain walking/jump ticks. Uses native default motion
    /// attributes, normal posture and no received effect updates. This is a
    /// prediction from the projection, not evidence that effects are absent on
    /// the server, that motion occurred, or that future geometry will stay fixed.
    pub async fn preview_survival_motion(
        &self,
        yaw: f32,
        inputs: &[SurvivalInput],
    ) -> Result<SurvivalMovementPreview> {
        self.preview_survival_path(&fixed_controls(yaw, inputs)?)
            .await
    }
    /// Predict a bounded multi-heading path from the current standing context.
    /// Pure observation: no packet, position assignment or reusable authority.
    pub async fn preview_survival_path(
        &self,
        controls: &[SurvivalControl],
    ) -> Result<SurvivalMovementPreview> {
        let mut state = self.bot.session.state.lock().await;
        self.ready(&state)?;
        preview(
            &mut state,
            self.bot.session.id,
            self.bot.session.started.elapsed().as_millis() as u64 / 50,
            controls,
        )
    }
}
fn preview(
    state: &mut State,
    connection_id: u64,
    tick: u64,
    controls: &[SurvivalControl],
) -> Result<SurvivalMovementPreview> {
    preview_in_mode(state, connection_id, tick, controls, GameMode::Survival)
}
fn preview_in_mode(
    state: &mut State,
    connection_id: u64,
    tick: u64,
    controls: &[SurvivalControl],
    mode: GameMode,
) -> Result<SurvivalMovementPreview> {
    if !matches!(mode, GameMode::Survival | GameMode::Creative)
        || state.operations.game_mode != Some(mode)
    {
        return Err(invalid(
            "ground motion requires matching received survival/creative mode",
        ));
    }
    let initial = survival::context(state, connection_id, tick)?;
    validate_initial(&initial)?;
    let mut model = Model::from_context(&initial);
    let initial_frame = model.initial_frame();
    let frames = predict(state, &mut model, controls)?;
    let terminal_clearance = clearance(state, frames.last().unwrap());
    Ok(SurvivalMovementPreview {
        initial_frame,
        terminal_clearance,
        initial,
        generation: state.loading.generation,
        controls: controls.to_vec(),
        frames,
    })
}
pub(super) fn validate_initial(initial: &StandingContext) -> Result<()> {
    let p = &initial.player;
    if !initial.on_ground
        || !p.effect_updates.is_empty()
        || p.movement_speed.map(|v| v.value) != Some(f64::from(0.1f32))
        || p.gravity.map(|v| v.value) != Some(0.08)
        || p.jump_strength.map(|v| v.value) != Some(f64::from(0.42f32))
        || p.step_height.map(|v| v.value) != Some(0.6)
    {
        return Err(invalid(
            "dry motion preview requires grounded native default movement attributes and no received effects",
        ));
    }
    Ok(())
}
fn predict(
    state: &impl GeometryView,
    model: &mut Model,
    controls: &[SurvivalControl],
) -> Result<Vec<PredictedMotionFrame>> {
    crate::client::survival::model::predict(|p| state.block(p), model, controls)
}
fn clearance(state: &impl GeometryView, frame: &PredictedMotionFrame) -> TerminalClearance {
    match terminal_clearance(state, frame) {
        Ok(()) => TerminalClearance::Admitted {
            horizontal_margin: TERMINAL_MARGIN,
        },
        Err(error) => TerminalClearance::RequiresReplan {
            reason: error.to_string(),
        },
    }
}

use crate::client::survival::model::Model;
#[cfg(test)]
use crate::client::survival::model::body;
fn geometry(
    state: &impl GeometryView,
    p: [f64; 3],
    motion: [f64; 3],
) -> Result<crate::client::survival::model::CollisionGeometry> {
    crate::client::survival::model::geometry(
        crate::MinecraftVersion::Java1_21_11,
        |p| state.block(p),
        p,
        motion,
    )
}
#[cfg(test)]
fn acceleration(input: SurvivalInput, yaw: f32, speed: f32) -> [f64; 3] {
    crate::client::survival::model::acceleration(
        crate::MinecraftVersion::Java1_21_11,
        input,
        yaw,
        speed,
    )
}
#[cfg(test)]
use crate::client::survival::model::collide;
impl Model {
    fn from_context(context: &StandingContext) -> Self {
        let mut model = Self::new(crate::MinecraftVersion::Java1_21_11, context.position);
        if let StandingPositionBasis::PredictedAndObserved { predicted, .. }
        | StandingPositionBasis::Predicted { predicted, .. } = &context.position_basis
        {
            model.frame.velocity = predicted.velocity;
        }
        model
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_and_collision_primitives_match_native_game_methods() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../data/java_1_21_11/dry_movement.json"
        ))
        .unwrap();
        assert_eq!(
            fixture["materials"].as_object().unwrap().len(),
            survival::DRY_CUBES.len()
        );
        for material in survival::DRY_CUBES {
            let v = &fixture["materials"][material];
            assert_eq!(v[0].as_f64().unwrap(), f64::from(0.6f32));
            assert_eq!(v[1].as_f64().unwrap(), 1.0);
            assert_eq!(v[2].as_f64().unwrap(), 1.0);
        }
        for c in fixture["inputs"].as_array().unwrap() {
            let input = SurvivalInput {
                strafe: c["strafe"].as_i64().unwrap() as i8,
                forward: c["forward"].as_i64().unwrap() as i8,
                jump: false,
            };
            let actual = acceleration(
                input,
                c["yaw"].as_f64().unwrap() as f32,
                c["speed"].as_f64().unwrap() as f32,
            );
            for (i, v) in actual.iter().enumerate() {
                assert!(
                    (v - c["expected"][i].as_f64().unwrap()).abs() < 1e-10,
                    "input {c}"
                );
            }
        }
        for c in fixture["collisions"].as_array().unwrap() {
            let actual = collide(
                body(serde_json::from_value(c["position"].clone()).unwrap()),
                serde_json::from_value(c["motion"].clone()).unwrap(),
                &serde_json::from_value::<Vec<[f64; 6]>>(c["boxes"].clone()).unwrap(),
            );
            for (i, v) in actual.iter().enumerate() {
                assert!(
                    (v - c["expected"][i].as_f64().unwrap()).abs() < 1e-10,
                    "collision {c}: {actual:?}"
                );
            }
        }
    }
    #[test]
    fn jump_lands_and_released_walking_brakes_with_gravity_retained() {
        let floor = crate::client::survival::model::CollisionGeometry::joined(&[[
            -20.0, 0.0, -20.0, 20.0, 1.0, 20.0,
        ]]);
        let mut model = Model::new(crate::MinecraftVersion::Java1_21_11, [0.5, 1.0, 0.5]);
        let mut peak = 1.0f64;
        for tick in 0..35 {
            let input = SurvivalInput {
                forward: i8::from(tick < 5),
                jump: tick == 0,
                ..Default::default()
            };
            let delta = model.intent(input, 0.0);
            model.advance(input, delta, &floor);
            peak = peak.max(model.frame.position[1]);
        }
        assert!(peak > 2.2 && peak < 2.3);
        assert_eq!(model.frame.position[1], 1.0);
        assert!(model.frame.resting);
        assert!(model.frame.position[2] > 0.5);
        assert_eq!(model.frame.velocity, [0.0, -0.08 * f64::from(0.98f32), 0.0]);
    }
}
