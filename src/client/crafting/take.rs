//! One result PICKUP. Ingredient consumption and remainders are only receipts.
use super::*;
use crate::client::inventory::{
    InventoryClickSend, InventoryTransactionReply, ItemContext, unavailable,
};
use crate::client::{GameMode, ObservedValue, SlotKnowledge, ValueSource};

/// Identity of one connection/world-local result take; never replay authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct CraftingTakeId {
    session: SessionStamp,
    attempt: u64,
}
impl CraftingTakeId {
    /// Original transport/world.
    pub fn session(self) -> SessionStamp {
        self.session
    }
    /// Local attempt number.
    pub fn attempt(self) -> u64 {
        self.attempt
    }
}
/// Delivery and observation of one result take.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CraftingTakeStage {
    /// Inspect the retained attempt; do not replay it.
    Pending,
    /// Complete write, matching fresh output cursor and fresh entire actual grid.
    ObservedTaken,
    /// First conflict or uncertain delivery, permanently retained.
    RequiresInspection,
}
/// Actual before/after grids with a cursor prediction kept separate.
/// There is no predicted ingredient consumption, remainder or next result.
#[derive(Clone, Debug, serde::Serialize)]
pub struct CraftingTakeRecord {
    /// Original identity.
    pub id: CraftingTakeId,
    /// Coherent received player baseline.
    pub initial: PlayerObservation,
    /// Required received mode.
    pub mode: GameMode,
    /// Actual complete input/result receipts before I/O.
    pub before: ReceivedCrafting,
    /// Actual empty cursor before I/O.
    pub cursor_before: ObservedValue<SlotKnowledge>,
    /// Expected output cursor, explicitly Predicted.
    pub cursor_prediction: ObservedValue<SlotKnowledge>,
    /// Submission facts. Full resync is always requested.
    pub send: InventoryClickSend,
    /// Entire grid/result from one fresh actual full-content boundary.
    pub after: Option<ReceivedCrafting>,
    /// Fresh matching actual output cursor.
    pub cursor_receipt: Option<ObservedValue<SlotKnowledge>>,
    /// Actual legacy comparison reply; rejection is not rollback.
    pub legacy_reply: Option<InventoryTransactionReply>,
    /// Persistent first conflict.
    pub requires_inspection: Option<String>,
    /// Historical attempt status, independent of current inventory.
    pub stage: CraftingTakeStage,
    #[serde(skip)]
    context: Arc<ItemContext>,
}
impl CraftingTakeRecord {
    pub(crate) fn window_id(&self) -> i32 {
        match self.before.source {
            CraftingSource::Player { .. } => 0,
            CraftingSource::Table { screen } => screen.window_id(),
        }
    }
    pub(crate) fn unresolved(&self) -> bool {
        self.stage != CraftingTakeStage::ObservedTaken
    }
    pub(crate) fn inspection(&mut self, reason: impl std::fmt::Display) {
        if self.unresolved() {
            self.requires_inspection
                .get_or_insert_with(|| reason.to_string());
            self.stage = CraftingTakeStage::RequiresInspection;
        }
    }
    pub(crate) fn ready(&self) -> bool {
        self.send.dispatched
            && self.requires_inspection.is_none()
            && self.after.is_some()
            && self.cursor_receipt.is_some()
            && (self.initial.session.version == MinecraftVersion::Java1_21_11
                || self.legacy_reply.is_some())
    }
}
pub(crate) fn screen_observation(
    player: &PlayerObservation,
    table: Option<super::super::container::ContainerScreen>,
) -> ScreenObservation {
    ScreenObservation {
        session: player.session,
        receive_sequence: player.receive_sequence,
        active_window: player.inventory.window_id,
        player_screen: player.inventory.player_screen,
        cursor: player.inventory.cursor.clone(),
        screen: table,
    }
}
fn complete(grid: &ReceivedCrafting) -> Result<()> {
    if grid
        .inputs
        .iter()
        .chain(std::iter::once(&grid.result))
        .any(|v| {
            v.as_ref()
                .is_none_or(|r| matches!(r.value(), SlotKnowledge::Unavailable))
        })
    {
        return Err(unavailable("complete actual crafting grid/result required"));
    }
    Ok(())
}
fn values(grid: &ReceivedCrafting) -> impl Iterator<Item = &ReceivedSlot> {
    grid.inputs
        .iter()
        .chain(std::iter::once(&grid.result))
        .filter_map(Option::as_ref)
}
pub(crate) fn prepare(
    initial: PlayerObservation,
    screen: ScreenObservation,
    registries: ServerRegistryObservation,
    requested: &ReceivedCrafting,
    mode: GameMode,
    attempt: u64,
) -> Result<CraftingTakeRecord> {
    if attempt == 0
        || initial.pending_dispatch
        || initial.game_mode != Some(mode)
        || !matches!(mode, GameMode::Survival | GameMode::Creative)
    {
        return Err(unavailable(
            "crafting take requires matching received mode and no pending mutation",
        ));
    }
    let before = ReceivedCrafting::capture(&initial, &screen, registries.clone())?
        .ok_or_else(|| unavailable("actual player/table crafting UI required"))?;
    complete(&before)?;
    complete(requested)?;
    if requested.session != initial.session
        || requested.source != before.source
        || requested.registry_state().stamp() != registries.stamp()
        || requested.receive_sequence > initial.receive_sequence
        || values(requested)
            .zip(values(&before))
            .any(|(a, b)| a.receive_sequence() != b.receive_sequence() || a.value() != b.value())
    {
        return Err(unavailable("requested crafting snapshot has changed"));
    }
    let revision = match before.source {
        CraftingSource::Player {
            access: PlayerScreenAccess::SubmittedClose { .. },
        } => initial
            .inventory
            .player_screen_revision
            .as_ref()
            .filter(|v| matches!(v.source, ValueSource::Received { .. }))
            .map(|v| v.value),
        CraftingSource::Player { .. } => initial.inventory.screen_revision,
        CraftingSource::Table { screen: id } => {
            let table = screen
                .screen
                .as_ref()
                .filter(|t| t.id == id && t.full_contents_sequence.is_some())
                .ok_or_else(|| unavailable("same received table opening/full contents required"))?;
            if initial.session.version == MinecraftVersion::Java1_16_1
                && !(1..=127).contains(&id.window_id())
            {
                return Err(unavailable("legacy table window must be 1..127"));
            }
            table
                .revision
                .as_ref()
                .filter(|v| matches!(v.source, ValueSource::Received { .. }))
                .map(|v| v.value)
        }
    };
    if initial.session.version == MinecraftVersion::Java1_21_11 && revision.is_none() {
        return Err(unavailable("actual crafting screen revision unavailable"));
    }
    let cursor_before = initial
        .inventory
        .cursor
        .clone()
        .filter(|v| {
            matches!(v.source, ValueSource::Received { .. }) && v.value == SlotKnowledge::Empty
        })
        .ok_or_else(|| unavailable("crafting result take requires actual empty cursor"))?;
    let output = before.result.as_ref().expect("complete").value().clone();
    if !matches!(output, SlotKnowledge::Item { .. }) {
        return Err(unavailable("received crafting result is empty"));
    }
    let context = Arc::new(ItemContext::new(registries, &initial)?);
    context.predecessor(&cursor_before)?;
    for v in values(&before) {
        context.predecessor(&super::super::received(
            v.value().clone(),
            v.receive_sequence(),
        ))?;
    }
    Ok(CraftingTakeRecord {
        id: CraftingTakeId {
            session: initial.session,
            attempt,
        },
        send: InventoryClickSend {
            after_sequence: initial.receive_sequence,
            legacy_action: None,
            legacy_comparison: None,
            screen_revision: revision,
            sent_screen_revision: revision.map(|v| if v == 0 { 1 } else { 0 }),
            request_full_resync: true,
            dispatched: false,
        },
        initial,
        mode,
        before,
        cursor_before,
        cursor_prediction: ObservedValue {
            value: output,
            source: ValueSource::Predicted,
        },
        after: None,
        cursor_receipt: None,
        legacy_reply: None,
        requires_inspection: None,
        stage: CraftingTakeStage::Pending,
        context,
    })
}
fn current_grid(
    record: &CraftingTakeRecord,
    player: &PlayerObservation,
    screen: &ScreenObservation,
    registries: &ServerRegistryObservation,
) -> Result<ReceivedCrafting> {
    if player.session != record.initial.session
        || player.game_mode != Some(record.mode)
        || player.selected_hotbar.as_ref().map(|v| v.value)
            != record.initial.selected_hotbar.as_ref().map(|v| v.value)
        || registries.stamp() != record.before.registry_state().stamp()
    {
        return Err(unavailable(
            "crafting take session/mode/hand/registry changed",
        ));
    }
    // Crafting can change the grid and cursor, never an unrelated equipped hand.
    for slot in [
        45,
        record
            .initial
            .selected_hotbar
            .as_ref()
            .map_or(usize::MAX, |v| 36 + usize::from(v.value)),
    ] {
        if let Some(before) = record
            .initial
            .inventory
            .slots
            .get(slot)
            .and_then(Option::as_ref)
        {
            let actual = player
                .inventory
                .slots
                .get(slot)
                .and_then(Option::as_ref)
                .ok_or_else(|| unavailable("crafting hand receipt disappeared"))?;
            if !record
                .context
                .classify(actual, before, &before.value, registries)?
                .0
            {
                return Err(unavailable("crafting take equipped hand changed"));
            }
        }
    }
    let grid = ReceivedCrafting::capture(player, screen, registries.clone())?
        .ok_or_else(|| unavailable("crafting take UI disappeared"))?;
    let same = match (record.before.source, grid.source) {
        (
            CraftingSource::Player { .. },
            CraftingSource::Player {
                access: PlayerScreenAccess::Received,
            },
        ) => player.inventory.window_id == Some(0),
        (a, b) => a == b,
    };
    if !same || grid.dimensions() != record.before.dimensions() {
        return Err(unavailable("crafting take original UI changed"));
    }
    Ok(grid)
}
/// Before I/O require unchanged *receipts*, not just recipe-looking values.
pub(crate) fn validate_before(
    record: &CraftingTakeRecord,
    player: &PlayerObservation,
    screen: &ScreenObservation,
    registries: &ServerRegistryObservation,
) -> Result<()> {
    let grid = current_grid(record, player, screen, registries)?;
    complete(&grid)?;
    if values(&grid)
        .zip(values(&record.before))
        .any(|(a, b)| a.receive_sequence() != b.receive_sequence() || a.value() != b.value())
        || player.inventory.cursor.as_ref() != Some(&record.cursor_before)
    {
        return Err(unavailable("crafting predecessor changed before I/O"));
    }
    Ok(())
}
pub(crate) fn receive(
    record: &mut CraftingTakeRecord,
    player: &PlayerObservation,
    screen: &ScreenObservation,
    registries: &ServerRegistryObservation,
) {
    if !record.unresolved() || record.requires_inspection.is_some() {
        return;
    }
    if let Err(e) = receive_inner(record, player, screen, registries) {
        record.inspection(e);
    }
}
fn receive_inner(
    record: &mut CraftingTakeRecord,
    player: &PlayerObservation,
    screen: &ScreenObservation,
    registries: &ServerRegistryObservation,
) -> Result<()> {
    let grid = current_grid(record, player, screen, registries)?;
    if let Some(after) = &record.after {
        complete(&grid)?;
        for (actual, retained) in values(&grid).zip(values(after)) {
            let actual = super::super::received(actual.value().clone(), actual.receive_sequence());
            let retained =
                super::super::received(retained.value().clone(), retained.receive_sequence());
            if !record
                .context
                .classify(&actual, &retained, &retained.value, registries)?
                .0
            {
                return Err(unavailable("crafting grid changed before completion"));
            }
        }
    }
    // A known fresh unrelated cursor conflicts; a pending/old cursor is not evidence.
    if let Some(cursor) = &player.inventory.cursor {
        let (before, output) = record.context.classify(
            cursor,
            &record.cursor_before,
            &record.cursor_prediction.value,
            registries,
        )?;
        if let ValueSource::Received { sequence } = cursor.source {
            if sequence > record.send.after_sequence {
                if output {
                    record.cursor_receipt.get_or_insert_with(|| cursor.clone());
                } else if !before || record.cursor_receipt.is_some() {
                    return Err(unavailable("crafting output cursor conflicts"));
                }
            }
        }
    }
    // Full resync refreshes even unchanged/empty inputs and a regenerated result.
    // Individual slot packets cannot substitute for the actual full-grid boundary.
    if complete(&grid).is_ok() {
        let mut sequences = values(&grid).map(ReceivedSlot::receive_sequence);
        let sequence = sequences.next().expect("nonempty native grid");
        let full = sequences.all(|s| s == sequence);
        drop(sequences);
        if sequence > record.send.after_sequence && full {
            if matches!(grid.source, CraftingSource::Table { .. })
                && screen
                    .screen
                    .as_ref()
                    .and_then(|t| t.full_contents_sequence)
                    != Some(sequence)
            {
                return Ok(());
            }
            for (actual, before) in values(&grid).zip(values(&record.before)) {
                let actual =
                    super::super::received(actual.value().clone(), actual.receive_sequence());
                let before =
                    super::super::received(before.value().clone(), before.receive_sequence());
                // Validate actual constructors/lifetimes; do not predict any grid value.
                record
                    .context
                    .classify(&actual, &before, &actual.value, registries)?;
            }
            record.after.get_or_insert(grid);
        }
    }
    Ok(())
}
