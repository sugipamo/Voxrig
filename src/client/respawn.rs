//! Explicit death recovery on the current transport; no automatic retry.
use super::{Health, ObservedValue, PlayerObservation, SessionStamp, ValueSource};
use crate::Result;
use crate::client::VersionAdapter;

pub(crate) type History = std::sync::Arc<std::sync::Mutex<Option<RespawnRecord>>>;

/// Local request lifecycle, separate from new-world and healthy-player receipts.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RespawnStage {
    /// Owned intent retained before writer admission.
    Prepared,
    /// The complete native request was dispatched; no outcome acknowledgement.
    Submitted,
    /// A subsequent actual RESPawn packet established a new world generation.
    RespawnReceived,
    /// Dispatch failed or was uncertain; inspect without replay.
    RequiresInspection,
}

/// Retained death and request evidence. Does not grant movement or restore IDs.
#[derive(Clone, Debug, serde::Serialize)]
pub struct RespawnRecord {
    /// Transport/world in which the request was admitted.
    pub session: SessionStamp,
    /// Coherent receive boundary before the request.
    pub after_sequence: u64,
    /// Actual received nonpositive health, never inferred from mode or animation.
    pub death: ObservedValue<Health>,
    /// Current request lifecycle.
    pub stage: RespawnStage,
    /// Complete frame dispatch, separately retained even if RESPawn arrives first.
    pub dispatched: bool,
    /// Actual new-world receipt; not fresh pose, health, inventory or readiness.
    pub received_spawn: Option<ObservedValue<SessionStamp>>,
    /// Latched I/O error; the original request is never automatically replayed.
    pub requires_inspection: Option<String>,
}

pub(crate) fn prepare(history: &History, player: &PlayerObservation) -> Result<()> {
    let death = player.health.as_ref().ok_or_else(|| {
        super::inventory::unavailable("respawn requires actual received dead-player health")
    })?;
    let valid_source = matches!(death.source, ValueSource::Received { sequence }
        if sequence >= player.session.world_generation && sequence <= player.receive_sequence);
    if !valid_source || !death.value.health.is_finite() || death.value.health > 0.0 {
        return Err(super::inventory::unavailable(
            "respawn requires actual received dead-player health in this world",
        ));
    }
    let mut latest = history.lock().expect("respawn history");
    if latest.as_ref().is_some_and(|r| r.session == player.session) {
        return Err(super::inventory::unavailable(
            "respawn already claimed in this world; inspect without replay",
        ));
    }
    *latest = Some(RespawnRecord {
        session: player.session,
        after_sequence: player.receive_sequence,
        death: death.clone(),
        stage: RespawnStage::Prepared,
        dispatched: false,
        received_spawn: None,
        requires_inspection: None,
    });
    Ok(())
}

pub(crate) fn dispatched(history: &History, result: &Result<()>) {
    let mut latest = history.lock().expect("respawn history");
    let Some(record) = latest.as_mut() else {
        return;
    };
    match result {
        Ok(()) => {
            record.dispatched = true;
            if record.received_spawn.is_none() {
                record.stage = RespawnStage::Submitted;
            }
        }
        Err(error) => {
            record
                .requires_inspection
                .get_or_insert_with(|| error.to_string());
            record.stage = RespawnStage::RequiresInspection;
        }
    }
}

pub(crate) fn received(history: &History, generation: u64, sequence: u64) {
    let mut latest = history.lock().expect("respawn history");
    let Some(record) = latest.as_mut() else {
        return;
    };
    if sequence <= record.after_sequence
        || generation <= record.session.world_generation
        || record.received_spawn.is_some()
    {
        return;
    }
    record.received_spawn = Some(super::observation::received(
        SessionStamp {
            world_generation: generation,
            ..record.session
        },
        sequence,
    ));
    if record.requires_inspection.is_none() {
        record.stage = RespawnStage::RespawnReceived;
    }
}

pub(crate) fn snapshot(history: &History) -> Option<RespawnRecord> {
    history.lock().expect("respawn history").clone()
}

impl super::Client {
    /// Request one native respawn after actual nonpositive health on this world.
    /// The connection owns the send after admission, including caller cancellation.
    /// Completion is dispatch evidence; wait for new RESPawn, pose, health and
    /// inventory before continuing. Same-world repeat requests are refused.
    pub async fn respawn(&self) -> Result<RespawnRecord> {
        crate::client::dispatch!(&self.adapter, a => VersionAdapter::respawn(a).await)
    }
    /// Inspect the latest retained request without a writer/state lock or resend.
    /// Available during a stalled write and after connection closure.
    pub fn respawn_record(&self) -> Option<RespawnRecord> {
        match &self.adapter {
            crate::connection::Adapter::Java1_16_1(bot) => snapshot(&bot.respawn_history),
            crate::connection::Adapter::Java1_21_11(bot) => snapshot(&bot.respawn_history),
        }
    }
}
