//! Finite original mounted inputs, with an explicit final neutral frame.
use super::{MountId, VehicleObservation, VehicleRelation};
use crate::client::adapter::VehicleOps;
pub use crate::client::physics::boat::BoatFrame;
use crate::{
    Result,
    client::{self as api, GameMode, ValueSource, inventory::unavailable},
};

/// Boat prediction and submission history; received motion stays separate.
#[derive(Clone, Debug, serde::Serialize)]
pub struct BoatMotion {
    /// Actual vehicle samples captured at admission.
    pub received: api::EntityMotionObservation,
    /// Declared seed, or the previous submitted run's final prediction.
    pub initial_frame: BoatFrame,
    /// Latest predicted frame retained before the multi-packet write.
    pub attempted_frame: Option<BoatFrame>,
    /// Predicted frames whose input, paddles and vehicle position were fully sent.
    pub frames: Vec<BoatFrame>,
}

pub(crate) type History = std::sync::Arc<std::sync::Mutex<Option<VehicleControlRecord>>>;
/// Maximum native input ticks in one owned mounted-control run.
pub const MAX_VEHICLE_CONTROL_TICKS: usize = 120;
/// Digital native mounted input. Ordinary boats additionally submit predicted
/// paddle and vehicle-motion packets; minecarts retain server-driven motion.
/// Sneak is deliberately a separate owned `dismount` operation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct VehicleInput {
    /// -1 backwards, 0 released, 1 forwards.
    pub forward: i8,
    /// -1 right, 0 released, 1 left.
    pub strafe: i8,
    /// Native jump key; the mounted vehicle determines its behavior.
    pub jump: bool,
}
/// Connection/world and original continuous mount-local run identity.
/// Saved diagnostics cannot construct or replay a live run.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct VehicleControlId {
    mount: MountId,
    attempt: u64,
}
impl VehicleControlId {
    /// Continuous actual mount to which every input belongs.
    pub fn mount(self) -> MountId {
        self.mount
    }
    /// Local run counter, not a native action ID or server tick.
    pub fn attempt(self) -> u64 {
        self.attempt
    }
}
/// Complete dispatch is distinct from received vehicle motion or stopping.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum VehicleControlStage {
    /// Retained finite input plan; its writer owner survives cancelled waiting.
    Running,
    /// Every input, including the caller's final neutral, fully sent.
    Submitted,
    /// First changed context, closure or uncertain write; never auto-replayed.
    RequiresInspection,
}
/// One finite mounted-input run. No control ACK or native position is inferred.
#[derive(Clone, Debug, serde::Serialize)]
pub struct VehicleControlRecord {
    /// Original owned identity.
    pub id: VehicleControlId,
    /// Required actual game mode.
    pub mode: GameMode,
    /// Coherent initial received player facts.
    pub initial: api::PlayerObservation,
    /// Original actual passenger relationship.
    pub vehicle: VehicleObservation,
    /// Caller-supplied digital inputs, ending with explicit neutral.
    pub inputs: Vec<VehicleInput>,
    /// One-based latest frame intent, retained before writer I/O.
    pub attempted_tick: u16,
    /// Number of fully sent frames; not elapsed server ticks.
    pub dispatched_ticks: u16,
    /// Submission facts and persistent first failure.
    pub stage: VehicleControlStage,
    /// First conflict or uncertain delivery.
    pub requires_inspection: Option<String>,
    /// Boat-only prediction; other vehicles use their original server-side input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boat_motion: Option<BoatMotion>,
}
impl VehicleControlRecord {
    pub(crate) fn unresolved(&self) -> bool {
        self.stage != VehicleControlStage::Submitted
    }
    pub(crate) fn inspection(&mut self, reason: impl std::fmt::Display) {
        if self.unresolved() {
            self.requires_inspection
                .get_or_insert_with(|| reason.to_string());
            self.stage = VehicleControlStage::RequiresInspection;
        }
    }
}
pub(crate) fn unresolved(history: &History) -> bool {
    history
        .lock()
        .expect("vehicle control history")
        .as_ref()
        .is_some_and(|r| r.unresolved())
}
fn context(
    player: &api::PlayerObservation,
    vehicle: &VehicleObservation,
    mode: GameMode,
    mount: MountId,
) -> Result<()> {
    if vehicle.player_native_id != Some(mount.player_native_id)
        || player.session!=mount.session() || vehicle.session!=player.session
        || vehicle.receive_sequence!=player.receive_sequence || player.game_mode!=Some(mode)
        || !player.health.as_ref().is_some_and(|h| h.value.health>0.)
        || !vehicle.relation.as_ref().is_some_and(|r|matches!(r.value,VehicleRelation::Mounted{mount:m} if m==mount)
            && matches!(r.source,ValueSource::Received{sequence} if sequence>=mount.receive_sequence())) {
        return Err(unavailable("vehicle control requires live healthy original received mount/mode/world"));
    }
    Ok(())
}
pub(crate) fn prepare(
    player: api::PlayerObservation,
    vehicle: VehicleObservation,
    mode: GameMode,
    mount: MountId,
    inputs: &[VehicleInput],
    previous: Option<&VehicleControlRecord>,
) -> Result<VehicleControlRecord> {
    if inputs.is_empty()
        || inputs.len() > MAX_VEHICLE_CONTROL_TICKS
        || inputs.last() != Some(&VehicleInput::default())
        || inputs
            .iter()
            .any(|i| !(-1..=1).contains(&i.forward) || !(-1..=1).contains(&i.strafe))
    {
        return Err(unavailable(
            "finite digital vehicle inputs must end with explicit neutral",
        ));
    }
    if player.pending_dispatch || previous.is_some_and(|r| r.unresolved()) {
        return Err(unavailable(
            "prior dispatch or vehicle control unresolved; no replay",
        ));
    }
    context(&player, &vehicle, mode, mount)?;
    let attempt = previous
        .map_or(Some(1), |r| r.id.attempt.checked_add(1))
        .ok_or_else(|| unavailable("vehicle control IDs exhausted"))?;
    Ok(VehicleControlRecord {
        id: VehicleControlId { mount, attempt },
        mode,
        initial: player,
        vehicle,
        inputs: inputs.to_vec(),
        attempted_tick: 0,
        dispatched_ticks: 0,
        stage: VehicleControlStage::Running,
        requires_inspection: None,
        boat_motion: None,
    })
}

pub(crate) fn configure_boat(
    record: &mut VehicleControlRecord,
    motion: Option<api::EntityMotionObservation>,
    previous: Option<&VehicleControlRecord>,
) -> Result<()> {
    let Some(motion) = motion else {
        return Ok(());
    };
    let name = motion.entity.type_name.as_deref().unwrap_or_default();
    if name != "minecraft:boat" && !name.ends_with("_boat") && !name.ends_with("_raft") {
        return Ok(());
    }
    if record
        .vehicle
        .passengers
        .as_ref()
        .and_then(|p| p.value.first())
        .copied()
        != record.vehicle.player_native_id
    {
        return Err(unavailable(
            "boat control requires the first actual passenger",
        ));
    }
    let seed = previous
        .filter(|p| p.stage == VehicleControlStage::Submitted && p.id.mount() == record.id.mount())
        .filter(|p| {
            p.vehicle.motion_correction_sequence == record.vehicle.motion_correction_sequence
        })
        .and_then(|p| p.boat_motion.as_ref())
        .and_then(|b| b.frames.last())
        .cloned();
    let initial_frame = match seed {
        Some(frame) => frame,
        None => BoatFrame::new(
            motion
                .position
                .as_ref()
                .ok_or_else(|| unavailable("boat requires received position"))?
                .value
                .position,
            motion
                .rotation
                .as_ref()
                .ok_or_else(|| unavailable("boat requires received rotation"))?
                .value,
            motion
                .velocity
                .as_ref()
                .ok_or_else(|| unavailable("boat requires received velocity"))?
                .value,
        ),
    };
    record.boat_motion = Some(BoatMotion {
        received: motion,
        initial_frame,
        attempted_frame: None,
        frames: Vec::new(),
    });
    Ok(())
}

pub(crate) fn boat_step(
    record: &VehicleControlRecord,
    input: VehicleInput,
    block_at: &mut impl FnMut([i32; 3]) -> Result<crate::NativeBlockState>,
) -> Result<Option<BoatFrame>> {
    record
        .boat_motion
        .as_ref()
        .map(|boat| {
            crate::client::physics::boat::tick(
                record.id.mount().session().version,
                boat.frames.last().unwrap_or(&boat.initial_frame),
                input,
                block_at,
            )
        })
        .transpose()
}

pub(crate) fn boat_packets(
    version: crate::MinecraftVersion,
    frame: &BoatFrame,
) -> [(i32, Vec<u8>); 2] {
    let modern = version == crate::MinecraftVersion::Java1_21_11;
    let mut position = Vec::with_capacity(33);
    for v in frame.position {
        position.extend(v.to_be_bytes());
    }
    for v in frame.rotation {
        position.extend(v.to_be_bytes());
    }
    if modern {
        position.push(u8::from(frame.on_ground));
    }
    [
        (
            if modern { 0x22 } else { 0x17 },
            frame.paddles.map(u8::from).to_vec(),
        ),
        (if modern { 0x21 } else { 0x16 }, position),
    ]
}
pub(crate) fn validate(
    record: &VehicleControlRecord,
    player: &api::PlayerObservation,
    vehicle: &VehicleObservation,
) -> Result<()> {
    if record.stage != VehicleControlStage::Running || record.requires_inspection.is_some() {
        return Err(unavailable(
            "vehicle control cannot continue or replay retained failure",
        ));
    }
    if vehicle.motion_correction_sequence != record.vehicle.motion_correction_sequence {
        return Err(unavailable(
            "received vehicle correction or explosion interrupted control",
        ));
    }
    if record.boat_motion.is_some()
        && vehicle
            .passengers
            .as_ref()
            .and_then(|p| p.value.first())
            .copied()
            != vehicle.player_native_id
    {
        return Err(unavailable("boat controlling passenger changed"));
    }
    context(player, vehicle, record.mode, record.id.mount)
}
pub(crate) fn receive(
    record: &mut VehicleControlRecord,
    player: &api::PlayerObservation,
    vehicle: &VehicleObservation,
) {
    if record.stage == VehicleControlStage::Running {
        if let Err(e) = validate(record, player, vehicle) {
            record.inspection(e);
        }
    }
}
pub(crate) fn payload(version: crate::MinecraftVersion, input: VehicleInput) -> (i32, Vec<u8>) {
    match version {
        crate::MinecraftVersion::Java1_16_1 => {
            let mut p = Vec::with_capacity(9);
            p.extend(f32::from(input.strafe).to_be_bytes());
            p.extend(f32::from(input.forward).to_be_bytes());
            p.push(u8::from(input.jump));
            (0x1d, p)
        }
        crate::MinecraftVersion::Java1_21_11 => {
            let flags = u8::from(input.forward == 1)
                | (u8::from(input.forward == -1) << 1)
                | (u8::from(input.strafe == 1) << 2)
                | (u8::from(input.strafe == -1) << 3)
                | (u8::from(input.jump) << 4);
            (0x2a, vec![flags])
        }
    }
}
impl api::Client {
    /// Retained mounted-input history, readable while its writer waits and after closure.
    pub async fn vehicle_control_record(&self) -> Result<Option<VehicleControlRecord>> {
        crate::client::dispatch!(&self.adapter, a => VehicleOps::vehicle_control_record(a).await)
    }
}
macro_rules! handle {
    ($handle:ty,$mode:expr) => {
        impl $handle {
            /// Dispatch finite digital mounted inputs at native tick spacing.
            /// The last input must be neutral. Cancellation stops only waiting.
            /// Full submission does not prove vehicle motion or a stopped vehicle.
            pub async fn start_vehicle_control(
                &self,
                mount: MountId,
                inputs: &[VehicleInput],
            ) -> Result<VehicleControlRecord> {
                crate::client::dispatch!(&self.client.adapter, a => VehicleOps::vehicle_control(a, $mode, mount, inputs).await)
            }
            /// Latest retained input run; never sends or repeats an input.
            pub async fn vehicle_control_record(&self) -> Result<Option<VehicleControlRecord>> {
                self.client.vehicle_control_record().await
            }
        }
    };
}
handle!(api::Survival, GameMode::Survival);
handle!(api::Creative, GameMode::Creative);
