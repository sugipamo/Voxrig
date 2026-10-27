//! Retained one-shot passive-cube placement with separate block/material receipts.
use crate::client::{
    ItemData, ItemStack, ObservedValue, PlayerObservation, SessionStamp, SlotKnowledge, ValueSource,
};
use crate::{BlockFace, NativeBlockState, Result};
/// Opaque identity of a connection/world-bound attempt; JSON is diagnostics only.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct PlacementId {
    session: SessionStamp,
    attempt: u64,
}
impl PlacementId {
    pub(crate) fn new(session: SessionStamp, attempt: u64) -> Self {
        Self { session, attempt }
    }
    /// Original connection/world, not a current permission.
    pub fn session(self) -> SessionStamp {
        self.session
    }
    /// Adapter-local attempt identity, not a server tick.
    pub fn attempt(self) -> u64 {
        self.attempt
    }
}
/// Before-I/O submission, never sufficient proof of placement.
#[derive(Clone, Debug, serde::Serialize)]
pub struct PlacementSend {
    /// Applied receive boundary before submission.
    pub after_sequence: u64,
    /// Native global sequence when present; absent on 1.16.1.
    pub interaction_sequence: Option<i32>,
    /// Complete command frame was written.
    pub dispatched: bool,
}
/// A modern sequence was processed; target/material outcomes remain separate.
#[derive(Clone, Debug, serde::Serialize)]
pub struct PlacementProcessing {
    /// Actual highest acknowledged native sequence.
    pub sequence: i32,
    /// Actual ACK packet ordinal.
    pub receive_sequence: u64,
}
/// Historical state of an attempt; each next action revalidates its own context.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PlacementStage {
    /// One or more required receipts have not arrived. Never retry.
    Pending,
    /// Complete submission and independent fresh block/material receipts agree.
    /// Modern additionally requires its actual processing ACK. Not actor attribution.
    ObservedPlaced,
    /// A server-processed boundary arrived while the target, support and selected
    /// material still equal their predecessors; see [`PlacementRecord::rejection`].
    /// Terminal and releases the one-shot owner. It names no server reason
    /// (protection, entity, range) and is never a retry instruction.
    ObservedRejected,
    /// A conflict, missing context or uncertain write is retained for inspection.
    RequiresInspection,
}
impl PlacementStage {
    /// The attempt no longer owns the one-shot gate (`ObservedPlaced` or `ObservedRejected`).
    pub fn is_resolved(self) -> bool {
        matches!(self, Self::ObservedPlaced | Self::ObservedRejected)
    }
}
/// Received evidence that the server processed an attempt without changing its target.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub struct PlacementRejection {
    /// Receive ordinal of the processed boundary: the covering modern
    /// `BlockChangedAck`, or the legacy target echo that ends the handler's pair.
    pub boundary_receive_sequence: u64,
    /// The server handler's fresh target update, still equal to `before`.
    pub target_receipt: ObservedValue<NativeBlockState>,
    /// Legacy support echo received immediately before the target echo; absent on modern.
    pub support_receipt: Option<ObservedValue<NativeBlockState>>,
}
/// Connection-owned diagnostic snapshot, also readable after closure.
#[derive(Clone, Debug, serde::Serialize)]
pub struct PlacementRecord {
    /// Immutable attempt identity.
    pub id: PlacementId,
    /// Coherent player/inventory capture before possible submission I/O.
    pub initial: PlayerObservation,
    /// Original clicked passive cube.
    pub support: [i32; 3],
    /// Complete support predecessor.
    pub support_state: NativeBlockState,
    /// First-outline face actually admitted.
    pub face: BlockFace,
    /// Native first hit, local to support, sent without an invented center.
    pub cursor: [f32; 3],
    /// Adjacent destination cell.
    pub target: [i32; 3],
    /// Complete received air predecessor, never inferred from an unloaded cell.
    pub before: NativeBlockState,
    /// Expected property-free passive cube.
    pub expected: NativeBlockState,
    /// Exact supported received selected stack before submission.
    pub held_before: ItemStack,
    /// Ordinal establishing that predecessor stack.
    pub held_receive_sequence: u64,
    /// Retained before I/O; no resend API exists.
    pub send: PlacementSend,
    /// Fresh target-specific received expected block, not a cache revision.
    pub target_receipt: Option<ObservedValue<NativeBlockState>>,
    /// Fresh selected-slot receipt showing exactly one material consumed.
    pub material_receipt: Option<ObservedValue<SlotKnowledge>>,
    /// Actual modern processing receipt; always absent on legacy.
    pub processing: Option<PlacementProcessing>,
    /// Processed-without-effect evidence; present exactly when `stage` is `ObservedRejected`.
    pub rejection: Option<PlacementRejection>,
    /// First latched conflict; restoration cannot erase it.
    pub requires_inspection: Option<String>,
    /// Retained outcome, separate from current world/slot contents.
    pub stage: PlacementStage,
}
/// Connection-owned diagnostic snapshot, also readable after closure.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ReplaceableCubePlacementRecord {
    /// First native outline selected in the same boundary as all predecessors.
    pub first_outline: super::BlockTargetHit,
    /// True when the clicked cell itself is replaced; otherwise the face neighbor.
    pub replaces_clicked: bool,
    /// Immutable attempt identity.
    pub id: PlacementId,
    /// Coherent player/inventory capture before possible submission I/O.
    pub initial: PlayerObservation,
    /// Original clicked cube or audited plant.
    pub support: [i32; 3],
    /// Complete support predecessor.
    pub support_state: NativeBlockState,
    /// First-outline face actually admitted.
    pub face: BlockFace,
    /// Native first hit, local to support, sent without an invented center.
    pub cursor: [f32; 3],
    /// Explicit destination selected by native replace-clicked rules.
    pub target: [i32; 3],
    /// Complete explicitly requested plant predecessor, captured before I/O.
    pub before: NativeBlockState,
    /// Expected property-free passive cube.
    pub expected: NativeBlockState,
    /// Exact supported received selected stack before submission.
    pub held_before: ItemStack,
    /// Ordinal establishing that predecessor stack.
    pub held_receive_sequence: u64,
    /// Retained before I/O; no resend API exists.
    pub send: PlacementSend,
    /// Fresh target-specific received expected block, not a cache revision.
    pub target_receipt: Option<ObservedValue<NativeBlockState>>,
    /// Fresh selected-slot receipt showing exactly one material consumed.
    pub material_receipt: Option<ObservedValue<SlotKnowledge>>,
    /// Actual modern processing receipt; always absent on legacy.
    pub processing: Option<PlacementProcessing>,
    /// Processed-without-effect evidence; present exactly when `stage` is `ObservedRejected`.
    pub rejection: Option<PlacementRejection>,
    /// First latched conflict; restoration cannot erase it.
    pub requires_inspection: Option<String>,
    /// Retained outcome, separate from current world/slot contents.
    pub stage: PlacementStage,
}
/// Connection-owned diagnostic snapshot, also readable after closure.
#[derive(Clone, Debug, serde::Serialize)]
pub struct SlabPlacementRecord {
    /// First native outline selected in the same boundary as all predecessors.
    pub first_outline: super::BlockTargetHit,
    /// True when the clicked slab becomes double in the same cell.
    pub merges_clicked: bool,
    /// Immutable attempt identity.
    pub id: PlacementId,
    /// Coherent player/inventory capture before possible submission I/O.
    pub initial: PlayerObservation,
    /// Original clicked cube or oak slab; the dry entry admits dry states only.
    pub support: [i32; 3],
    /// Complete support predecessor.
    pub support_state: NativeBlockState,
    /// First-outline face actually admitted.
    pub face: BlockFace,
    /// Native first hit, local to support, sent without an invented center.
    pub cursor: [f32; 3],
    /// Explicit destination selected by native replace-clicked rules.
    pub target: [i32; 3],
    /// Complete explicitly requested predecessor; the dry entry admits air/dry halves only.
    pub before: NativeBlockState,
    /// Expected complete oak slab type and waterlogged properties.
    pub expected: NativeBlockState,
    /// Exact supported received selected stack before submission.
    pub held_before: ItemStack,
    /// Ordinal establishing that predecessor stack.
    pub held_receive_sequence: u64,
    /// Retained before I/O; no resend API exists.
    pub send: PlacementSend,
    /// Fresh target-specific received expected block, not a cache revision.
    pub target_receipt: Option<ObservedValue<NativeBlockState>>,
    /// Fresh selected-slot receipt showing exactly one material consumed.
    pub material_receipt: Option<ObservedValue<SlotKnowledge>>,
    /// Actual modern processing receipt; always absent on legacy.
    pub processing: Option<PlacementProcessing>,
    /// Processed-without-effect evidence; present exactly when `stage` is `ObservedRejected`.
    pub rejection: Option<PlacementRejection>,
    /// First latched conflict; restoration cannot erase it.
    pub requires_inspection: Option<String>,
    /// Retained outcome, separate from current world/slot contents.
    pub stage: PlacementStage,
}
/// Audited native fluid state derived from a complete block state (1.21.11).
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SlabFluidState {
    /// Native empty fluid, not an unavailable cell.
    Empty,
    /// Native water; flowing water is distinct from the source fluid type.
    Water {
        /// Native source fluid type, required for a newly waterlogged half.
        source: bool,
        /// Native falling property; falling amount-eight water is still flowing.
        falling: bool,
        /// Native fluid amount, 1..8.
        amount: u8,
    },
}
/// One liquid-aware slab attempt, separate from the dry slab history.
#[derive(Clone, Debug, serde::Serialize)]
pub struct FluidSlabPlacementRecord {
    /// Original intent, complete states and independent block/material/ACK receipts.
    pub placement: SlabPlacementRecord,
    /// Raw received world-cache provenance shared by both original cells.
    /// This is a capture boundary, not the ordinal of an individual block packet.
    pub source: super::SceneSource,
    /// Native fluid of the original clicked block.
    pub support_fluid: SlabFluidState,
    /// Native fluid of the explicitly requested predecessor.
    pub before_fluid: SlabFluidState,
    /// Native fluid of the expected slab, not a receipt of neighboring flow.
    pub expected_fluid: SlabFluidState,
}
/// Connection-owned diagnostic snapshot, also readable after closure.
#[derive(Clone, Debug, serde::Serialize)]
pub struct StairPlacementRecord {
    /// First native outline selected in the same boundary as all predecessors.
    pub first_outline: super::BlockTargetHit,
    /// Exact cells read for target and adjacent shape calculations, before I/O.
    pub shape_inputs: Vec<StairShapeInput>,
    /// Adjacent stairs whose complete state must change and be freshly received.
    pub neighbor_updates: Vec<StairNeighborUpdate>,
    /// Immutable attempt identity.
    pub id: PlacementId,
    /// Coherent player/inventory capture before possible submission I/O.
    pub initial: PlayerObservation,
    /// Original clicked passive cube.
    pub support: [i32; 3],
    /// Complete support predecessor.
    pub support_state: NativeBlockState,
    /// First-outline face actually admitted.
    pub face: BlockFace,
    /// Native first hit, local to support, sent without an invented center.
    pub cursor: [f32; 3],
    /// Explicit adjacent destination.
    pub target: [i32; 3],
    /// Complete explicitly requested air predecessor.
    pub before: NativeBlockState,
    /// Expected complete oak stair facing, half, shape and waterlogged properties.
    pub expected: NativeBlockState,
    /// Exact supported received selected stack before submission.
    pub held_before: ItemStack,
    /// Ordinal establishing that predecessor stack.
    pub held_receive_sequence: u64,
    /// Retained before I/O; no resend API exists.
    pub send: PlacementSend,
    /// Fresh target-specific received expected block, not a cache revision.
    pub target_receipt: Option<ObservedValue<NativeBlockState>>,
    /// Fresh selected-slot receipt showing exactly one material consumed.
    pub material_receipt: Option<ObservedValue<SlotKnowledge>>,
    /// Actual modern processing receipt; always absent on legacy.
    pub processing: Option<PlacementProcessing>,
    /// Processed-without-effect evidence; present exactly when `stage` is `ObservedRejected`.
    pub rejection: Option<PlacementRejection>,
    /// First latched conflict; restoration cannot erase it.
    pub requires_inspection: Option<String>,
    /// Retained outcome, separate from current world/slot contents.
    pub stage: PlacementStage,
}
/// A complete static state used by the bounded stair shape calculation.
#[derive(Clone, Debug, serde::Serialize)]
pub struct StairShapeInput {
    /// Cell read under the original player/world/inventory lock.
    pub position: [i32; 3],
    /// Complete received predecessor; missing cells never become air.
    pub before: NativeBlockState,
}
/// A required adjacent stair change, distinct from the target receipt.
#[derive(Clone, Debug, serde::Serialize)]
pub struct StairNeighborUpdate {
    /// Adjacent cell whose shape changes.
    pub position: [i32; 3],
    /// Complete received predecessor.
    pub before: NativeBlockState,
    /// Complete expected successor.
    pub expected: NativeBlockState,
    /// Fresh matching block/section receipt, absent until independently observed.
    pub receipt: Option<ObservedValue<NativeBlockState>>,
}
pub(crate) fn unavailable(message: impl std::fmt::Display) -> crate::Error {
    crate::error::state_error(message)
}
pub(crate) fn air(state: &NativeBlockState) -> bool {
    matches!(
        state.name.as_str(),
        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
    )
}
pub(crate) fn selected_material(p: &PlayerObservation) -> Result<(ItemStack, u64)> {
    selected_material_in(p, None)
}
pub(crate) fn selected_slab(p: &PlayerObservation) -> Result<(ItemStack, u64)> {
    selected_material_in(p, Some("slab"))
}
pub(crate) fn selected_stair(p: &PlayerObservation) -> Result<(ItemStack, u64)> {
    selected_material_in(p, Some("stair"))
}
fn selected_material_in(p: &PlayerObservation, family: Option<&str>) -> Result<(ItemStack, u64)> {
    if p.received_pose.is_none()
        || p.dimension.is_none()
        || p.inventory.player_screen.is_none()
        || !matches!(
            p.inventory.cursor.as_ref(),
            Some(ObservedValue {
                value: SlotKnowledge::Empty,
                source: ValueSource::Received { .. }
            })
        )
    {
        return Err(unavailable(
            "placement requires received own pose/world, received player screen or complete local close, and empty cursor",
        ));
    }
    let selection = p
        .selected_hotbar
        .as_ref()
        .filter(|s| s.value <= 8)
        .ok_or_else(|| unavailable("ordered selected hand unavailable"))?;
    let Some(ObservedValue {
        value: SlotKnowledge::Item { item },
        source: ValueSource::Received { sequence },
    }) = p.inventory.slots[36 + usize::from(selection.value)].as_ref()
    else {
        return Err(unavailable(
            "placement requires a complete received selected material",
        ));
    };
    let registry = crate::client::registry::Registry::for_version(p.session.version);
    let definition = registry.item(&item.name)?;
    let admitted = if family == Some("stair") {
        item.name == "minecraft:oak_stairs"
    } else if family == Some("slab") {
        item.name == "minecraft:oak_slab"
    } else {
        super::model::DRY_CUBES.contains(&item.name.as_str())
            && item.name != "minecraft:grass_block"
    };
    if !admitted
        || item.data != ItemData::Default
        || item.id != definition.id
        || item.count == 0
        || item.count > definition.max_stack_size
    {
        return Err(crate::Error::new(
            crate::ErrorKind::Unsupported,
            anyhow::anyhow!(if family == Some("stair") {
                "stair placement admits default oak stair stacks only"
            } else if family == Some("slab") {
                "slab placement admits default oak slab stacks only"
            } else {
                "placement admits default passive cube stacks only"
            }),
        ));
    }
    Ok((item.clone(), *sequence))
}
pub(crate) fn remaining(item: &ItemStack) -> SlotKnowledge {
    if item.count == 1 {
        SlotKnowledge::Empty
    } else {
        let mut item = item.clone();
        item.count -= 1;
        SlotKnowledge::Item { item }
    }
}
pub(crate) fn adjacent(support: [i32; 3], face: BlockFace) -> Result<[i32; 3]> {
    let d = [
        [0, -1, 0],
        [0, 1, 0],
        [0, 0, -1],
        [0, 0, 1],
        [-1, 0, 0],
        [1, 0, 0],
    ][face as usize];
    let mut out = support;
    for i in 0..3 {
        out[i] = support[i]
            .checked_add(d[i])
            .ok_or_else(|| unavailable("placement target overflow"))?;
    }
    Ok(out)
}
