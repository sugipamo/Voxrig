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
pub enum PlacementStage {
    /// One or more required receipts have not arrived. Never retry.
    Pending,
    /// Complete submission and independent fresh block/material receipts agree.
    /// Modern additionally requires its actual processing ACK. Not actor attribution.
    ObservedPlaced,
    /// A conflict, missing context or uncertain write is retained for inspection.
    RequiresInspection,
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
    /// First latched conflict; restoration cannot erase it.
    pub requires_inspection: Option<String>,
    /// Retained outcome, separate from current world/slot contents.
    pub stage: PlacementStage,
}
pub(crate) fn unavailable(message: impl std::fmt::Display) -> crate::Error {
    crate::Error::new(crate::ErrorKind::State, anyhow::anyhow!("{message}"))
}
pub(crate) fn air(state: &NativeBlockState) -> bool {
    matches!(
        state.name.as_str(),
        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
    )
}
pub(crate) fn selected_material(p: &PlayerObservation) -> Result<(ItemStack, u64)> {
    if p.received_pose.is_none()
        || p.dimension.is_none()
        || p.inventory.window_id != Some(0)
        || !matches!(
            p.inventory.cursor.as_ref(),
            Some(ObservedValue {
                value: SlotKnowledge::Empty,
                source: ValueSource::Received { .. }
            })
        )
    {
        return Err(unavailable(
            "placement requires received own pose, world, player screen and empty cursor",
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
    if !super::model::DRY_CUBES.contains(&item.name.as_str())
        || item.name == "minecraft:grass_block"
        || item.data != ItemData::Default
        || item.id != definition.id
        || item.count == 0
        || item.count > definition.max_stack_size
    {
        return Err(crate::Error::new(
            crate::ErrorKind::Unsupported,
            anyhow::anyhow!("placement admits default passive cube stacks only"),
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
