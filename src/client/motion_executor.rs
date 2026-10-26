//! Private finite strategy of the connection motion executor. The preview is
//! only a constraint: this model advances only after a complete packet write.
//! All access is under the adapter's receive/send lock (legacy coherent gate,
//! modern State). Never retain this object in a worker across a lock boundary.
use super::survival::{PredictedMotionFrame, model::Model};
use crate::MinecraftVersion;

pub(crate) struct FiniteMotion {
    pub run_id: u64,
    pub generation: u64,
    pub valid: bool,
    pub acquired: bool,
    pub model: Model,
    checked_world_revision: Option<u64>,
    #[cfg(test)]
    pub between_packets: Option<(
        std::sync::Arc<tokio::sync::Notify>,
        std::sync::Arc<tokio::sync::Notify>,
    )>,
}
impl FiniteMotion {
    pub fn new(
        version: MinecraftVersion,
        run_id: u64,
        generation: u64,
        frame: PredictedMotionFrame,
    ) -> Self {
        let mut model = Model::new(version, frame.position);
        model.frame = frame;
        Self {
            run_id,
            generation,
            valid: true,
            acquired: false,
            model,
            checked_world_revision: None,
            #[cfg(test)]
            between_packets: None,
        }
    }
    /// A changed terrain snapshot invalidates the *whole* remaining preview,
    /// including future collisions, before another frame can be submitted.
    pub fn recheck(
        &mut self,
        block_at: impl FnMut([i32; 3]) -> crate::Result<crate::NativeBlockState>,
        controls: &[super::survival::SurvivalControl],
        frames: &[PredictedMotionFrame],
        revision: u64,
    ) -> crate::Result<()> {
        if self.checked_world_revision == Some(revision) {
            return Ok(());
        }
        let mut candidate = self.model.clone();
        let sent = candidate.frame.tick;
        let mut predicted =
            match super::survival::model::predict(block_at, &mut candidate, controls) {
                Ok(predicted) => predicted,
                Err(error) => {
                    self.valid = false;
                    return Err(error);
                }
            };
        for frame in &mut predicted {
            frame.tick += sent;
        }
        if predicted != frames {
            self.valid = false;
            return Err(crate::error::state_error(
                "received geometry invalidated finite preview; unsent tail discarded",
            ));
        }
        self.checked_world_revision = Some(revision);
        Ok(())
    }
    pub fn owns(&self, run_id: u64, generation: u64) -> bool {
        self.run_id == run_id && self.generation == generation
    }
    pub fn authorizes(&self, run_id: u64, generation: u64) -> bool {
        self.valid && self.owns(run_id, generation)
    }
}
