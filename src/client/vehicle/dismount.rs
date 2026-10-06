//! Retained one-shot dismount, followed by explicit neutral input after receipt.
use super::{MountId, VehicleObservation, VehicleRelation};
use crate::{
    Result,
    client::{self as api, GameMode, ObservedValue, ValueSource, inventory::unavailable},
    connection::Adapter,
};
pub(crate) type History = std::sync::Arc<std::sync::Mutex<Option<DismountRecord>>>;

/// Original owned attempt. Saved diagnostics cannot authorize a release.
/// ```compile_fail
/// let id: voxrig::client::DismountId = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct DismountId {
    mount: MountId,
    attempt: u64,
}
impl DismountId {
    /// Original continuous mounted lifetime.
    pub fn mount(self) -> MountId {
        self.mount
    }
    /// Local attempt counter, not sent as a native action sequence.
    pub fn attempt(self) -> u64 {
        self.attempt
    }
}
/// Dispatch and actual relationship change are distinct stages.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DismountStage {
    /// Intent retained before actor admission or writer I/O.
    Prepared,
    /// Complete requesting input frame sent; no outcome ACK is implied.
    Submitted,
    /// Actual same-vehicle absence received; neutral input is still required.
    ObservedUnmounted,
    /// Actual absence retained and complete neutral input sent.
    Completed,
    /// Original attempt cannot continue automatically. Never resend the request.
    RequiresInspection,
}
/// Owned dismount facts; no vehicle motion or standing admission is predicted.
#[derive(Clone, Debug, serde::Serialize)]
pub struct DismountRecord {
    /// Live attempt identity.
    pub id: DismountId,
    /// Matching received mode required for both writes.
    pub mode: GameMode,
    /// Coherent original passenger observation, including unknown spawn facts.
    pub initial: VehicleObservation,
    /// Current lifecycle stage.
    pub stage: DismountStage,
    /// Receive boundary immediately before the request frame.
    pub after_sequence: u64,
    /// Entire native request frame was sent, separate from the outcome.
    pub request_dispatched: bool,
    /// First applicable actual same-vehicle absence after request dispatch.
    pub observed_unmounted: Option<ObservedValue<VehicleRelation>>,
    /// Neutral input intent retained before its separate I/O.
    pub release_claimed: bool,
    /// Entire neutral input frame was sent; not a standing or server ACK.
    pub release_dispatched: bool,
    /// Latched conflict, closure or uncertain I/O reason.
    pub requires_inspection: Option<String>,
}
impl DismountRecord {
    pub(crate) fn unresolved(&self) -> bool {
        self.stage != DismountStage::Completed
    }
    pub(crate) fn inspection(&mut self, reason: impl std::fmt::Display) {
        if self.unresolved() {
            self.requires_inspection
                .get_or_insert_with(|| reason.to_string());
            self.stage = DismountStage::RequiresInspection;
        }
    }
    pub(crate) fn sent(&mut self, release: bool) {
        if release {
            self.release_dispatched = true;
        } else {
            self.request_dispatched = true;
        }
        if self.requires_inspection.is_none() {
            self.stage = if self.release_dispatched && self.observed_unmounted.is_some() {
                DismountStage::Completed
            } else if self.observed_unmounted.is_some() {
                DismountStage::ObservedUnmounted
            } else {
                DismountStage::Submitted
            };
        }
    }
}
fn context(
    player: &api::PlayerObservation,
    vehicle: &VehicleObservation,
    mode: GameMode,
    mount: MountId,
) -> Result<()> {
    if player.session != mount.session()
        || vehicle.session != player.session
        || vehicle.receive_sequence != player.receive_sequence
        || player.game_mode != Some(mode)
        || !player.health.as_ref().is_some_and(|h| h.value.health > 0.0)
        || vehicle.player_native_id != Some(mount.player_native_id)
    {
        return Err(unavailable(
            "dismount requires live healthy matching received mode/player/world",
        ));
    }
    Ok(())
}
pub(crate) fn prepare(
    player: &api::PlayerObservation,
    vehicle: VehicleObservation,
    mode: GameMode,
    mount: MountId,
    previous: Option<&DismountRecord>,
) -> Result<DismountRecord> {
    context(player, &vehicle, mode, mount)?;
    if player.pending_dispatch || previous.is_some_and(|r| r.unresolved() || r.id.mount == mount) {
        return Err(unavailable(
            "dismount attempt unresolved or already claimed; inspect without replay",
        ));
    }
    if !vehicle.relation.as_ref().is_some_and(|r| matches!(r.value,VehicleRelation::Mounted { mount: m } if m==mount) && matches!(r.source,ValueSource::Received { sequence } if sequence>=mount.receive_sequence())) {
        return Err(unavailable("original mounted relationship retired or changed"));
    }
    let attempt = previous
        .map_or(Some(1), |r| r.id.attempt.checked_add(1))
        .ok_or_else(|| unavailable("dismount attempts exhausted"))?;
    Ok(DismountRecord {
        id: DismountId { mount, attempt },
        mode,
        after_sequence: player.receive_sequence,
        initial: vehicle,
        stage: DismountStage::Prepared,
        request_dispatched: false,
        observed_unmounted: None,
        release_claimed: false,
        release_dispatched: false,
        requires_inspection: None,
    })
}
pub(crate) fn validate_before(
    record: &DismountRecord,
    player: &api::PlayerObservation,
    vehicle: &VehicleObservation,
    release: bool,
) -> Result<()> {
    context(player, vehicle, record.mode, record.id.mount)?;
    if record.requires_inspection.is_some() {
        return Err(unavailable("dismount requires inspection; no replay"));
    }
    let relation = vehicle
        .relation
        .as_ref()
        .ok_or_else(|| unavailable("dismount relationship unavailable"))?;
    let valid = if release {
        record.request_dispatched
            && record.observed_unmounted.is_some()
            && !record.release_dispatched
            && matches!(relation.value,VehicleRelation::Unmounted {previous_mount} if previous_mount==record.id.mount)
    } else {
        !record.request_dispatched
            && !record.release_claimed
            && matches!(relation.value,VehicleRelation::Mounted {mount} if mount==record.id.mount)
    };
    if !valid {
        return Err(unavailable(
            "original dismount phase or live relationship changed",
        ));
    }
    Ok(())
}
pub(crate) fn receive(
    record: &mut DismountRecord,
    player: &api::PlayerObservation,
    vehicle: &VehicleObservation,
) {
    if !record.unresolved() || record.requires_inspection.is_some() {
        return;
    }
    if let Err(e) = context(player, vehicle, record.mode, record.id.mount) {
        record.inspection(e);
        return;
    }
    let Some(relation) = vehicle.relation.as_ref() else {
        record.inspection("dismount vehicle retired or unreceived");
        return;
    };
    match relation.value {
        VehicleRelation::Mounted { mount }
            if mount == record.id.mount && record.observed_unmounted.is_none() => {}
        VehicleRelation::Unmounted { previous_mount }
            if previous_mount == record.id.mount
                && record.request_dispatched
                && matches!(relation.source,ValueSource::Received {sequence} if sequence>record.after_sequence) =>
        {
            record
                .observed_unmounted
                .get_or_insert_with(|| relation.clone());
            record.stage = DismountStage::ObservedUnmounted;
        }
        _ => record.inspection("dismount relationship changed outside retained phase"),
    }
}
/// Pinned original serializers: old neutral axes + shift flag; modern Input shift bit.
pub(crate) fn payload(version: crate::MinecraftVersion, release: bool) -> (i32, Vec<u8>) {
    match version {
        crate::MinecraftVersion::Java1_16_1 => {
            let mut p = vec![0; 8];
            p.push(if release { 0 } else { 2 });
            (0x1d, p)
        }
        crate::MinecraftVersion::Java1_21_11 => (0x2a, vec![if release { 0 } else { 32 }]),
    }
}
impl api::Client {
    async fn request_common_dismount(
        &self,
        mode: GameMode,
        mount: MountId,
    ) -> Result<DismountRecord> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_dismount(mode, mount).await,
            Adapter::Java1_21_11(bot) => bot.operations().common_dismount(mode, mount).await,
        }
    }
    async fn complete_common_dismount(
        &self,
        mode: GameMode,
        id: DismountId,
    ) -> Result<DismountRecord> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_complete_dismount(mode, id).await,
            Adapter::Java1_21_11(bot) => bot.operations().common_complete_dismount(mode, id).await,
        }
    }
    /// Read retained dismount facts without replay, including while a writer is
    /// stalled or after connection closure. Completed history stays completed.
    pub async fn dismount_record(&self) -> Result<Option<DismountRecord>> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_dismount_record().await,
            Adapter::Java1_21_11(bot) => bot.operations().common_dismount_record().await,
        }
    }
}
macro_rules! handle {
    ($handle:ty,$mode:expr) => {
        impl $handle {
            /// Request dismount once for an actual mounted receipt. Actor ownership
            /// survives caller cancellation. Full dispatch is not actual absence.
            pub async fn dismount(&self, mount: MountId) -> Result<DismountRecord> {
                self.client.request_common_dismount($mode, mount).await
            }
            /// After the original actual unmounted receipt, send neutral input once.
            /// This completes the request without releasing ground-motion guards.
            pub async fn complete_dismount(&self, id: DismountId) -> Result<DismountRecord> {
                self.client.complete_common_dismount($mode, id).await
            }
            /// Read the latest attempt without sending another frame.
            pub async fn dismount_record(&self) -> Result<Option<DismountRecord>> {
                self.client.dismount_record().await
            }
        }
    };
}
handle!(api::Survival, GameMode::Survival);
handle!(api::Creative, GameMode::Creative);
