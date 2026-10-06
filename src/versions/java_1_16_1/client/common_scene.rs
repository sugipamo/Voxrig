//! Legacy immutable geometry; admission is shared with the live native preview.
use super::*;
use crate::client::survival::{
    self as api, model,
    scene::{CapturedSurvivalScene, SceneAdapter, SceneSource},
};
use std::collections::BTreeMap;
#[derive(Clone)]
pub(crate) struct LegacyCapturedScene {
    model: model::Model,
    cells: Arc<BTreeMap<[i32; 3], crate::NativeBlockState>>,
}
impl LegacyCapturedScene {
    pub(crate) fn preview_path(
        &self,
        controls: &[api::SurvivalControl],
    ) -> Result<(
        api::PredictedMotionFrame,
        Vec<api::PredictedMotionFrame>,
        api::TerminalClearance,
    )> {
        let mut model = self.model.clone();
        let initial_frame = model.initial_frame();
        let block_at = |p| {
            self.cells.get(&p).cloned().ok_or_else(|| {
                crate::client::recording::invalid(
                    "geometry lies outside the complete captured scene",
                )
            })
        };
        let frames = model::predict(&block_at, &mut model, controls)?;
        let last = frames.last().expect("validated bounded nonempty controls");
        let terminal_clearance = if last.resting {
            match common_motion::legacy_clearance(&block_at, last.position, 1.0 / 16.0) {
                Ok(()) => api::TerminalClearance::Admitted {
                    horizontal_margin: 1.0 / 16.0,
                },
                Err(e) => api::TerminalClearance::RequiresReplan {
                    reason: e.to_string(),
                },
            }
        } else {
            api::TerminalClearance::RequiresReplan {
                reason: "terminal motion must be released and resting".into(),
            }
        };
        Ok((initial_frame, frames, terminal_clearance))
    }
}
impl Bot {
    pub(crate) async fn common_capture_scene(
        &self,
        region: crate::Region,
    ) -> Result<CapturedSurvivalScene> {
        api::scene::validate_region(region)?;
        let _gate = self.coherent_state_gate.lock().await;
        self.common_motion_admission().await?;
        let preview = self
            .common_preview_core(
                &[api::SurvivalControl {
                    yaw: 0.0,
                    input: Default::default(),
                }],
                None,
            )
            .await?;
        let world = self.world.lock().await;
        let mut cells = BTreeMap::new();
        for x in region.min[0]..=region.max[0] {
            for y in region.min[1]..=region.max[1] {
                for z in region.min[2]..=region.max[2] {
                    let p = [x, y, z];
                    let state = common_motion::legacy_motion_block(&world, p)?;
                    if !matches!(
                        state.name.as_str(),
                        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
                    ) && !model::DRY_CUBES.contains(&state.name.as_str())
                    {
                        return Err(crate::client::recording::invalid(
                            "captured scenes require admitted dry cubes and air",
                        ));
                    }
                    cells.insert(p, state);
                }
            }
        }
        let mut model = model::Model::new(
            crate::MinecraftVersion::Java1_16_1,
            preview.initial_frame.position,
        );
        model.frame = preview.initial_frame;
        let scene = LegacyCapturedScene {
            model,
            cells: Arc::new(cells),
        };
        let block_at = |p| {
            scene.cells.get(&p).cloned().ok_or_else(|| {
                crate::client::recording::invalid("scene omits initial standing halo")
            })
        };
        common_motion::legacy_clearance(&block_at, scene.model.frame.position, 0.0)?;
        Ok(CapturedSurvivalScene {
            source: SceneSource {
                version: crate::MinecraftVersion::Java1_16_1,
                connection_id: preview.initial.session.connection_id,
                receive_sequence: preview.initial.receive_sequence,
                world_revision: world.revision(),
                region,
            },
            adapter: SceneAdapter::Legacy(scene),
        })
    }
}
