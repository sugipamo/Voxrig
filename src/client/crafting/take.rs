//! One result PICKUP or QUICK_MOVE. Consumption and remainders are only receipts.
mod shift;
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
/// Original native destination for one result operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CraftingResultDestination {
    /// One entire result into the empty or compatible actual cursor.
    Cursor,
    /// Native QUICK_MOVE into main/hotbar, including its internal crafts.
    Inventory,
}
impl CraftingResultDestination {
    pub(crate) fn click_mode(self) -> u8 {
        match self {
            Self::Cursor => 0,
            Self::Inventory => 1,
        }
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
    /// Full write, unchanged fresh cursor and full grid/main inventory with actual output increase.
    ObservedTransferred,
    /// First conflict or uncertain delivery, permanently retained.
    RequiresInspection,
}
/// Actual before/after grids and optional shift inventory; cursor prediction is separate.
/// There is no predicted ingredient consumption, remainder or next result.
#[derive(Clone, Debug, serde::Serialize)]
pub struct CraftingTakeRecord {
    /// Original identity.
    pub id: CraftingTakeId,
    /// Coherent received player baseline.
    pub initial: PlayerObservation,
    /// Required received mode.
    pub mode: GameMode,
    /// Original one-shot result destination.
    pub destination: CraftingResultDestination,
    /// Actual complete input/result receipts before I/O.
    pub before: ReceivedCrafting,
    /// Actual known cursor: empty/compatible for PICKUP, preserved by QUICK_MOVE.
    pub cursor_before: ObservedValue<SlotKnowledge>,
    /// Entire result added to the cursor, or unchanged cursor for QUICK_MOVE; explicitly Predicted.
    pub cursor_prediction: ObservedValue<SlotKnowledge>,
    /// Submission facts. Full resync is always requested.
    pub send: InventoryClickSend,
    /// Entire grid/result from one fresh actual full-content boundary.
    pub after: Option<ReceivedCrafting>,
    /// Full main/hotbar boundary after native QUICK_MOVE; no inventory prediction.
    pub inventory_after: Option<crate::client::ReceivedInventory>,
    /// Compatible output stock counted from actual main/hotbar before I/O.
    /// Derived from receipts, not a server counter or predicted craft count.
    pub inventory_output_before: Option<u64>,
    /// Actual compatible main/hotbar count increase, not total manufactured output.
    /// Native remainder/drop behavior is not reconstructed from this count.
    pub inventory_output_increase: Option<u64>,
    /// Fresh matching actual output/unchanged cursor.
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
    pub(crate) fn completed_stage(&self) -> CraftingTakeStage {
        match self.destination {
            CraftingResultDestination::Cursor => CraftingTakeStage::ObservedTaken,
            CraftingResultDestination::Inventory => CraftingTakeStage::ObservedTransferred,
        }
    }
    pub(crate) fn legacy_full_comparison(&self) -> Result<SlotKnowledge> {
        match self.destination {
            CraftingResultDestination::Cursor => Ok(SlotKnowledge::Empty),
            CraftingResultDestination::Inventory => shift::legacy_comparison(self),
        }
    }
    pub(crate) fn window_id(&self) -> i32 {
        match self.before.source {
            CraftingSource::Player { .. } => 0,
            CraftingSource::Table { screen } => screen.window_id(),
        }
    }
    pub(crate) fn unresolved(&self) -> bool {
        !matches!(
            self.stage,
            CraftingTakeStage::ObservedTaken | CraftingTakeStage::ObservedTransferred
        )
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
            && (self.destination == CraftingResultDestination::Cursor
                || (self.inventory_after.is_some() && self.inventory_output_increase.is_some()))
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
    prepare_inner(
        initial,
        screen,
        registries,
        requested,
        mode,
        attempt,
        CraftingResultDestination::Cursor,
    )
}
pub(crate) fn prepare_transfer(
    initial: PlayerObservation,
    screen: ScreenObservation,
    registries: ServerRegistryObservation,
    requested: &ReceivedCrafting,
    mode: GameMode,
    attempt: u64,
) -> Result<CraftingTakeRecord> {
    prepare_inner(
        initial,
        screen,
        registries,
        requested,
        mode,
        attempt,
        CraftingResultDestination::Inventory,
    )
}
fn prepare_inner(
    initial: PlayerObservation,
    screen: ScreenObservation,
    registries: ServerRegistryObservation,
    requested: &ReceivedCrafting,
    mode: GameMode,
    attempt: u64,
    destination: CraftingResultDestination,
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
            matches!(v.source, ValueSource::Received { .. })
                && v.value != SlotKnowledge::Unavailable
        })
        .ok_or_else(|| unavailable("crafting result take requires actual known cursor"))?;
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
    let inventory_output_before = match (&output, destination) {
        (SlotKnowledge::Item { item }, CraftingResultDestination::Inventory) => {
            Some(shift::prepare_inventory(&initial, &before, &context, item)?)
        }
        _ => None,
    };
    let cursor_prediction = if destination == CraftingResultDestination::Inventory {
        cursor_before.value.clone()
    } else {
        match (&cursor_before.value, &output) {
            (SlotKnowledge::Empty, _) => output.clone(),
            (SlotKnowledge::Item { item: held }, SlotKnowledge::Item { item: result }) => {
                if !context.same_data(held, result)? {
                    return Err(unavailable(
                        "crafting result differs from held cursor item/data",
                    ));
                }
                let capacity = u32::try_from(held.properties()?.max_stack_size)
                    .map_err(|_| unavailable("crafting cursor capacity is invalid"))?;
                let count = held
                    .count
                    .checked_add(result.count)
                    .filter(|count| *count <= capacity)
                    .ok_or_else(|| {
                        unavailable("entire crafting result does not fit held cursor")
                    })?;
                // Native result slots refuse a partial result. Preserve the actual
                // held representation rather than replacing it with display data.
                let mut combined = held.clone();
                combined.count = count;
                SlotKnowledge::Item { item: combined }
            }
            _ => return Err(unavailable("crafting cursor/result unavailable")),
        }
    };
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
        destination,
        before,
        cursor_before,
        cursor_prediction: ObservedValue {
            value: cursor_prediction,
            source: ValueSource::Predicted,
        },
        after: None,
        inventory_after: None,
        inventory_output_before,
        inventory_output_increase: None,
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
    if record.destination == CraftingResultDestination::Cursor {
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
    } else {
        shift::validate_retained(record, player, registries)?;
    }
    let grid = ReceivedCrafting::capture(player, screen, registries.clone())?
        .ok_or_else(|| unavailable("crafting take UI disappeared"))?;
    let same = record.before.source.accepts_received_source(grid.source);
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
    if record.destination == CraftingResultDestination::Inventory {
        shift::validate_before(record, player)?;
    }
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
            if record.destination == CraftingResultDestination::Inventory {
                let Some((inventory, increase)) =
                    shift::capture_after(record, player, registries, sequence)?
                else {
                    return Ok(());
                };
                record.inventory_after.get_or_insert(inventory);
                record.inventory_output_increase.get_or_insert(increase);
            }
            record.after.get_or_insert(grid);
        }
    }
    Ok(())
}
