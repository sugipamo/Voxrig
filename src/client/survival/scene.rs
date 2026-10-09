//! Detached geometry from a live capture, never reusable dispatch authority.
use super::{PredictedMotionFrame, SurvivalControl, TerminalClearance};
use crate::{MinecraftVersion, Region, Result, connection::Adapter};

/// Diagnostic provenance of an immutable, bounded geometry capture.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneSource {
    /// Selected native model and registry version.
    pub version: MinecraftVersion,
    /// Original transport identity; not a restored session.
    pub connection_id: u64,
    /// Receive boundary shared by the original standing context and geometry.
    pub receive_sequence: u64,
    /// Original decoded world-cache revision.
    pub world_revision: u64,
    /// Complete immutable geometry bounds, including the standing halo.
    pub region: Region,
}
/// Detached, known dry-terrain prediction. Saving it never permits dispatch or continuation.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenePreview {
    /// Original capture provenance, not a fresh observation.
    pub source: SceneSource,
    /// Native model seed, separate from received position evidence.
    pub initial_frame: PredictedMotionFrame,
    /// Explicit caller inputs.
    pub controls: Vec<SurvivalControl>,
    /// Predicted frames, never received positions.
    pub frames: Vec<PredictedMotionFrame>,
    /// Model assessment, never actual execution permission.
    pub terminal_clearance: TerminalClearance,
}
/// Immutable live-captured scene. It retains no Client or sender and cannot be
/// deserialized from saved values. Live operations always perform new admission.
/// ```compile_fail
/// use voxrig::client::survival::CapturedSurvivalScene;
/// fn restore(json: &str) -> CapturedSurvivalScene { serde_json::from_str(json).unwrap() }
/// ```
#[derive(Clone)]
pub struct CapturedSurvivalScene {
    pub(crate) source: SceneSource,
    pub(crate) adapter: SceneAdapter,
}
#[derive(Clone)]
pub(crate) enum SceneAdapter {
    Legacy(crate::versions::java_1_16_1::client::LegacyCapturedScene),
    Modern(Box<crate::versions::java_1_21_11::operations::CapturedSurvivalScene>),
}
impl super::Survival {
    /// Capture complete, loaded air/dry passive cubes and registered dry slabs/stairs under healthy stationary
    /// Survival defaults. At most 64 cells per axis and 32768 total; the initial
    /// standing halo must be included. No packet is sent. Broader edits, chained
    /// hypothetical scenes and reconstruction contracts remain version-specific.
    pub async fn capture_scene(&self, region: Region) -> Result<CapturedSurvivalScene> {
        validate_region(region)?;
        match &self.client.adapter {
            Adapter::Java1_16_1(bot) => bot.common_capture_scene(region).await,
            Adapter::Java1_21_11(bot) => {
                let scene = bot.operations().capture_survival_scene(region).await?;
                let source = SceneSource {
                    version: MinecraftVersion::Java1_21_11,
                    connection_id: scene.source().connection_id,
                    receive_sequence: scene.source().receive_sequence,
                    world_revision: scene.source().world_revision,
                    region: scene.region(),
                };
                Ok(CapturedSurvivalScene {
                    source,
                    adapter: SceneAdapter::Modern(Box::new(scene)),
                })
            }
        }
    }
}
impl CapturedSurvivalScene {
    /// Original immutable diagnostic provenance.
    pub fn source(&self) -> &SceneSource {
        &self.source
    }
    /// Predict against only the captured geometry and native initial model.
    /// Missing geometry rejects the query. The scene and live Client are unchanged.
    pub fn preview_path(&self, controls: &[SurvivalControl]) -> Result<ScenePreview> {
        super::model::validate_controls(controls)?;
        let (initial_frame, frames, terminal_clearance) = match &self.adapter {
            SceneAdapter::Legacy(scene) => scene.preview_path(controls)?,
            SceneAdapter::Modern(scene) => {
                let preview = scene.scenario().preview_path(controls)?;
                (
                    preview.initial_frame,
                    preview.frames,
                    preview.terminal_clearance,
                )
            }
        };
        Ok(ScenePreview {
            source: self.source.clone(),
            initial_frame,
            controls: controls.to_vec(),
            frames,
            terminal_clearance,
        })
    }
}
pub(crate) fn validate_region(region: Region) -> Result<()> {
    if region.volume()? > 32768
        || (0..3).any(|i| {
            i64::from(region.max[i]) - i64::from(region.min[i]) + 1 > 64
                || region.min[i] < -29_999_984
                || region.max[i] > 29_999_984
        })
    {
        return Err(crate::client::recording::invalid(
            "scene requires axes <=64, <=32768 cells and bounded world coordinates",
        ));
    }
    Ok(())
}
