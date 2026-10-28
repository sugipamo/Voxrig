//! Owned recipe placement. Only actual inputs plus inventory conservation release it.
use super::*;
use crate::client::inventory::{ItemContext, unavailable};
use crate::client::{GameMode, ReceivedInventory, SlotKnowledge, ValueSource};

/// Connection/world-local placement attempt, never replay authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct RecipePlacementId {
    session: SessionStamp,
    attempt: u64,
}
impl RecipePlacementId {
    /// Original connection/world.
    pub fn session(self) -> SessionStamp {
        self.session
    }
    /// Local monotonic attempt number.
    pub fn attempt(self) -> u64 {
        self.attempt
    }
}
/// Delivery and actual input placement; this does not assert a crafted output.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RecipePlacementStage {
    /// Inspect this attempt, including after caller cancellation; never replay.
    Pending,
    /// Complete write, actual target inputs and conserved actual inventory/grid.
    ObservedPlaced,
    /// Fresh actual ghost display and conserved empty inputs; no placement or output.
    ObservedGhost,
    /// Persistent first ownership conflict or uncertain delivery.
    RequiresInspection,
    /// Complete request and no-op resync frames, then one fresh full-content
    /// boundary after the send (every input, the result, main/hotbar 9..44 at one
    /// receive ordinal, and an empty cursor received with or after it; 1.16.1 also
    /// after the resync's own transaction reply) still equals the pre-send receipts,
    /// with no ghost display. The server processed the request without effect.
    /// Terminal; releases the placement owner. Never derived from a protocol bool.
    ObservedUnchanged,
    /// Still `Pending` at the bounded response deadline. The outcome is unknown and
    /// the server may still apply it; this is not a refusal. Terminal: the connection
    /// ledger fails with `TransactionExpired`, so later work needs a new connection.
    Expired,
}
/// Native recipe request submission facts, independent of actual placement.
#[derive(Clone, Debug, serde::Serialize)]
pub struct RecipePlacementSend {
    /// Applied packet boundary immediately before the owned write.
    pub after_sequence: u64,
    /// Complete transport write, never a server acknowledgement.
    pub dispatched: bool,
}
/// Retained owned Next/Maximum request with actual before/after receipts.
/// No ingredient consumption, result or native type tie is predicted.
#[derive(Clone, Debug, serde::Serialize)]
pub struct RecipePlacementRecord {
    /// Original attempt identity.
    pub id: RecipePlacementId,
    /// Sealed historical preflight, including separately marked return predictions.
    pub plan: RecipePlacementPlan,
    /// Complete write evidence and receive boundary.
    pub send: RecipePlacementSend,
    /// Actual target inputs/result at the conserved capture boundary. Unchanged
    /// cells retain their original ordinals; result is not a crafting assertion.
    pub after: Option<ReceivedCrafting>,
    /// Actual inventory at the same boundary, including return destinations.
    pub inventory_after: Option<ReceivedInventory>,
    /// Actual server UI display, separate from inputs and crafted results. A display-only
    /// response supplies no recipe identity or proof of request causation.
    pub ghost: Option<super::ReceivedRecipeGhost>,
    /// Persistent first conflict; retained even when later packets look plausible.
    pub requires_inspection: Option<String>,
    /// Historical operation status, independent of later inventory edits.
    pub stage: RecipePlacementStage,
    /// Complete write of the one no-op full-resync click that follows the request.
    /// Its full contents are the processed boundary for `ObservedUnchanged`.
    pub resync_dispatched: bool,
    /// Receive ordinal of the full contents that showed no effect.
    /// Set only together with `ObservedUnchanged`.
    pub unchanged_boundary: Option<u64>,
    #[serde(skip)]
    semantic: Arc<ItemContext>,
    /// 1.16.1 receive ordinal of the resync click's own transaction reply.
    #[serde(skip)]
    pub(crate) resync_reply: Option<u64>,
    /// First full-content boundary answering the resync click.
    #[serde(skip)]
    pub(crate) resync_boundary: Option<u64>,
    /// Absolute response deadline, set when the record is retained for I/O.
    #[serde(skip)]
    pub(crate) deadline: Option<tokio::time::Instant>,
}
impl RecipePlacementRecord {
    pub(crate) fn window_id(&self) -> i32 {
        match self.plan.layout().source() {
            CraftingSource::Player { .. } => 0,
            CraftingSource::Table { screen } => screen.window_id(),
        }
    }
    /// Blocks other owners. `Expired` keeps blocking; its connection is failed.
    pub(crate) fn unresolved(&self) -> bool {
        !matches!(
            self.stage,
            RecipePlacementStage::ObservedPlaced
                | RecipePlacementStage::ObservedGhost
                | RecipePlacementStage::ObservedUnchanged
        )
    }
    fn open(&self) -> bool {
        matches!(
            self.stage,
            RecipePlacementStage::Pending | RecipePlacementStage::RequiresInspection
        )
    }
    /// `Pending` past its deadline becomes `Expired` before any later evidence is read.
    pub(crate) fn expire_due(&mut self) {
        if self.stage == RecipePlacementStage::Pending
            && self
                .deadline
                .is_some_and(|deadline| tokio::time::Instant::now() >= deadline)
        {
            self.stage = RecipePlacementStage::Expired;
        }
    }
    pub(crate) fn inspection(&mut self, reason: impl std::fmt::Display) {
        self.expire_due();
        if self.open() {
            self.requires_inspection
                .get_or_insert_with(|| reason.to_string());
            self.stage = RecipePlacementStage::RequiresInspection;
        }
    }
    pub(crate) fn ready(&self) -> bool {
        self.send.dispatched
            && self.requires_inspection.is_none()
            && self.after.is_some()
            && self.inventory_after.is_some()
            // Hold the owner until the resync's full contents: a plan captured
            // earlier would be invalidated by their fresh receipt ordinals.
            && (!self.resync_dispatched || self.resync_boundary.is_some())
    }
}
fn slot_equal(a: &ReceivedSlot, b: &ReceivedSlot) -> bool {
    a.receive_sequence() == b.receive_sequence() && a.value() == b.value()
}
fn tags_equal(a: &ServerRegistryObservation, b: &ServerRegistryObservation) -> bool {
    a.stamp() == b.stamp() && a.tags() == b.tags()
}
fn identity(record: &RecipePlacementRecord, current: &ReceivedCraftingContext) -> Result<()> {
    let before = record.plan.source_context();
    let selected = |p: &PlayerObservation| p.selected_hotbar.as_ref().map(|s| (s.value, s.source));
    if current.session() != before.session()
        || current.player().game_mode != Some(record.plan.mode())
        || !tags_equal(
            current.inventory().registry_state(),
            before.inventory().registry_state(),
        )
        || selected(current.player()).map(|s| s.0) != selected(before.player()).map(|s| s.0)
        || selected(current.player())
            .is_none_or(|s| !matches!(s.1, ValueSource::Received { .. } | ValueSource::Submitted))
        || current.grid().dimensions() != before.grid().dimensions()
    {
        return Err(unavailable(
            "recipe placement session/mode/hand/registry tags changed",
        ));
    }
    let same_ui = before
        .grid()
        .source()
        .accepts_received_source(current.grid().source());
    if !same_ui {
        return Err(unavailable("recipe placement original UI changed"));
    }
    if current.recipes().entry(record.plan.recipe())?.unlocked() != Some(true) {
        return Err(unavailable("recipe placement book entry revoked"));
    }
    let cursor = current
        .inventory()
        .cursor()
        .ok_or_else(|| unavailable("recipe placement actual cursor disappeared"))?;
    if *cursor.value() != SlotKnowledge::Empty {
        return Err(unavailable("recipe placement cursor changed"));
    }
    // Placement can move main/hotbar/offhand ingredients, never armor.
    for index in 5..9 {
        let old = before
            .inventory()
            .slot(index)?
            .ok_or_else(|| unavailable("recipe placement armor predecessor unavailable"))?;
        let now = current
            .inventory()
            .slot(index)?
            .ok_or_else(|| unavailable("recipe placement armor disappeared"))?;
        if now.value() != old.value() {
            return Err(unavailable("recipe placement armor changed"));
        }
    }
    Ok(())
}
pub(crate) fn validate_before(
    record: &RecipePlacementRecord,
    current: &ReceivedCraftingContext,
) -> Result<()> {
    identity(record, current)?;
    let before = record.plan.source_context();
    if current.receive_sequence() < before.receive_sequence()
        || current
            .inventory()
            .cursor()
            .zip(before.inventory().cursor())
            .is_none_or(|(a, b)| !slot_equal(a, b))
        || current.player().selected_hotbar != before.player().selected_hotbar
    {
        return Err(unavailable(
            "recipe placement predecessors changed before I/O",
        ));
    }
    for index in 9..46 {
        let (Some(a), Some(b)) = (
            before.inventory().slot(index)?,
            current.inventory().slot(index)?,
        ) else {
            return Err(unavailable("recipe placement actual inventory required"));
        };
        if !slot_equal(a, b) {
            return Err(unavailable(
                "recipe placement inventory receipt changed before I/O",
            ));
        }
    }
    let [w, h] = before.grid().dimensions();
    for y in 0..h {
        for x in 0..w {
            let (Some(a), Some(b)) = (before.grid().input(x, y)?, current.grid().input(x, y)?)
            else {
                return Err(unavailable(
                    "recipe placement complete actual grid required",
                ));
            };
            if !slot_equal(a, b) {
                return Err(unavailable(
                    "recipe placement input receipt changed before I/O",
                ));
            }
        }
    }
    Ok(())
}
pub(crate) fn prepare(
    plan: &RecipePlacementPlan,
    current: &ReceivedCraftingContext,
    mode: GameMode,
    attempt: u64,
) -> Result<RecipePlacementRecord> {
    if attempt == 0
        || mode != plan.mode()
        || !plan.can_request()
        || current.player().pending_dispatch
    {
        return Err(unavailable(
            "recipe placement requires matching mode, safe plan and no pending mutation",
        ));
    }
    let record = RecipePlacementRecord {
        id: RecipePlacementId {
            session: plan.session(),
            attempt,
        },
        plan: plan.clone(),
        send: RecipePlacementSend {
            after_sequence: current.receive_sequence(),
            dispatched: false,
        },
        after: None,
        inventory_after: None,
        ghost: None,
        requires_inspection: None,
        stage: RecipePlacementStage::Pending,
        resync_dispatched: false,
        unchanged_boundary: None,
        resync_reply: None,
        resync_boundary: None,
        deadline: None,
        semantic: Arc::new(ItemContext::new(
            current.inventory().registry_state().clone(),
            current.player(),
        )?),
    };
    validate_before(&record, current)?;
    Ok(record)
}
/// The packet carries only the opening, native identity and Maximum flag.
/// No click comparison/state ID or invented target count is transmitted.
pub(crate) fn payload(record: &RecipePlacementRecord) -> Result<Vec<u8>> {
    encode(
        record.id.session.version,
        record.window_id(),
        record.plan.recipe().native(),
        record.plan.amount(),
    )
}
fn encode(
    version: MinecraftVersion,
    window: i32,
    native: &recipes::NativeRecipeId,
    amount: RecipePlacementAmount,
) -> Result<Vec<u8>> {
    use recipes::NativeRecipeId;
    let mut bytes = Vec::new();
    match (version, native) {
        (MinecraftVersion::Java1_16_1, NativeRecipeId::Legacy(name))
            if (0..=127).contains(&window) =>
        {
            bytes.push(window as u8);
            crate::protocol::put_string(&mut bytes, name);
        }
        (MinecraftVersion::Java1_21_11, NativeRecipeId::Modern(id)) if *id >= 0 && window >= 0 => {
            crate::protocol::put_varint(&mut bytes, window);
            crate::protocol::put_varint(&mut bytes, *id);
        }
        _ => {
            return Err(unavailable(
                "recipe placement native identity/window mismatch",
            ));
        }
    }
    bytes.push(u8::from(amount == RecipePlacementAmount::Maximum));
    Ok(bytes)
}
fn actual_items(context: &ReceivedCraftingContext) -> Result<Vec<&crate::client::ItemStack>> {
    let mut values = Vec::new();
    for index in 9..46 {
        let slot = context
            .inventory()
            .slot(index)?
            .ok_or_else(|| unavailable("recipe placement inventory receipt missing"))?;
        match slot.value() {
            SlotKnowledge::Empty => {}
            SlotKnowledge::Item { item } => values.push(item),
            _ => return Err(unavailable("recipe placement inventory data unavailable")),
        }
    }
    let [w, h] = context.grid().dimensions();
    for y in 0..h {
        for x in 0..w {
            let slot = context
                .grid()
                .input(x, y)?
                .ok_or_else(|| unavailable("recipe placement input receipt missing"))?;
            match slot.value() {
                SlotKnowledge::Empty => {}
                SlotKnowledge::Item { item } => values.push(item),
                _ => return Err(unavailable("recipe placement input data unavailable")),
            }
        }
    }
    Ok(values)
}
fn conserved(record: &RecipePlacementRecord, current: &ReceivedCraftingContext) -> Result<bool> {
    let before = actual_items(record.plan.source_context())?;
    let after = actual_items(current)?;
    let mut groups: Vec<(&crate::client::ItemStack, i64)> = Vec::new();
    for (items, sign) in [(&before, 1i64), (&after, -1i64)] {
        for &item in items {
            item.properties()?;
            let mut found = None;
            for (i, (other, _)) in groups.iter().enumerate() {
                if record.semantic.same_data(item, other)? {
                    found = Some(i);
                    break;
                }
            }
            let count = i64::from(item.count) * sign;
            if let Some(i) = found {
                groups[i].1 += count;
            } else {
                groups.push((item, count));
            }
        }
    }
    Ok(groups.iter().all(|(_, n)| *n == 0))
}
#[cfg(test)]
pub(crate) fn receive(record: &mut RecipePlacementRecord, current: &ReceivedCraftingContext) {
    receive_with_ghost(record, current, None)
}
pub(crate) fn receive_with_ghost(
    record: &mut RecipePlacementRecord,
    current: &ReceivedCraftingContext,
    ghost: Option<&super::ReceivedRecipeGhost>,
) {
    record.expire_due();
    if record.stage != RecipePlacementStage::Pending || record.requires_inspection.is_some() {
        return;
    }
    let result = (|| {
        identity(record, current)?;
        if let Some(ghost) = ghost.filter(|g| {
            g.receive_sequence() > record.send.after_sequence
                && g.session() == record.id.session()
                && record
                    .plan
                    .layout()
                    .source()
                    .accepts_received_source(g.source())
        }) {
            if ghost.recipe_name().is_some_and(|name| {
                record.plan.recipe().native() != &recipes::NativeRecipeId::Legacy(name.into())
            }) {
                return Err(unavailable("ghost response names another recipe"));
            }
            if ghost.registry_state().stamp()
                != record
                    .plan
                    .source_context()
                    .inventory()
                    .registry_state()
                    .stamp()
            {
                return Err(unavailable("ghost display configuration changed"));
            }
            record.ghost = Some(ghost.clone());
        }
        receive_inner(record, current)
    })();
    if let Err(error) = result {
        record.inspection(error);
    }
}
pub(crate) fn validate_plan_history(
    plan: &RecipePlacementPlan,
    previous: Option<&RecipePlacementRecord>,
) -> Result<()> {
    if previous.is_some_and(|r| plan.receive_sequence() <= r.send.after_sequence) {
        return Err(unavailable(
            "capture a fresh crafting context after the previous request",
        ));
    }
    Ok(())
}
fn receive_inner(
    record: &mut RecipePlacementRecord,
    current: &ReceivedCraftingContext,
) -> Result<()> {
    identity(record, current)?;
    if !record.send.dispatched || current.receive_sequence() <= record.send.after_sequence {
        return Ok(());
    }
    if let Some(full) = full_boundary(record, current)? {
        record.resync_boundary.get_or_insert(full);
    }
    let before = record.plan.source_context();
    if record.ghost.is_some() {
        for y in 0..current.grid().dimensions()[1] {
            for x in 0..current.grid().dimensions()[0] {
                let actual = current
                    .grid()
                    .input(x, y)?
                    .ok_or_else(|| unavailable("ghost input missing"))?;
                let old = before.grid().input(x, y)?.expect("complete baseline");
                if *actual.value() != SlotKnowledge::Empty
                    || (actual.value() != old.value()
                        && actual.receive_sequence() <= record.send.after_sequence)
                {
                    return Ok(());
                }
            }
        }
        let Some(result) = current.grid().result() else {
            return Ok(());
        };
        if *result.value() != SlotKnowledge::Empty
            || before.grid().result().is_some_and(|old| {
                old.value() != result.value()
                    && result.receive_sequence() <= record.send.after_sequence
            })
            || !conserved(record, current)?
        {
            return Ok(());
        }
        for index in 9..46 {
            let old = before.inventory().slot(index)?.expect("complete baseline");
            let actual = current
                .inventory()
                .slot(index)?
                .ok_or_else(|| unavailable("ghost inventory missing"))?;
            if actual.value() != old.value()
                && actual.receive_sequence() <= record.send.after_sequence
            {
                return Ok(());
            }
        }
        record.after = Some(current.grid().clone());
        record.inventory_after = Some(current.inventory().clone());
        return Ok(());
    }
    if let Some(boundary) = unchanged_boundary(record, current)? {
        record.unchanged_boundary = Some(boundary);
        record.stage = RecipePlacementStage::ObservedUnchanged;
        return Ok(());
    }
    let pattern = matching::resolve(before, record.plan.recipe())?;
    let [w, h] = before.grid().dimensions();
    let mut counts = std::collections::BTreeSet::new();
    let mut changed = false;
    for y in 0..h {
        for x in 0..w {
            let coordinate = [x, y];
            let actual = current
                .grid()
                .input(x, y)?
                .ok_or_else(|| unavailable("recipe placement input disappeared"))?;
            let old = before.grid().input(x, y)?.expect("validated predecessor");
            if actual.value() != old.value() {
                if actual.receive_sequence() <= record.send.after_sequence {
                    return Ok(());
                }
                changed = true;
            }
            let cell = record
                .plan
                .layout()
                .cells()
                .iter()
                .find(|c| c.coordinate() == coordinate);
            let ingredient = cell.and_then(|c| pattern.cells[c.display_index()].as_deref());
            match (ingredient, actual.value()) {
                (None, SlotKnowledge::Empty) => {}
                (Some(ids), SlotKnowledge::Item { item })
                    if ids.contains(&item.id.value())
                        && record.plan.targets.contains(&item.count) =>
                {
                    counts.insert(item.count);
                }
                // Incremental packets can temporarily expose an incomplete or different grid.
                _ => return Ok(()),
            }
        }
    }
    if !changed || counts.len() != 1 || !conserved(record, current)? {
        return Ok(());
    }
    for index in 9..46 {
        let old = before
            .inventory()
            .slot(index)?
            .expect("complete predecessor");
        let actual = current
            .inventory()
            .slot(index)?
            .ok_or_else(|| unavailable("recipe placement inventory disappeared"))?;
        if actual.value() != old.value() && actual.receive_sequence() <= record.send.after_sequence
        {
            return Ok(());
        }
    }
    record.after = Some(current.grid().clone());
    record.inventory_after = Some(current.inventory().clone());
    Ok(())
}

/// Vanilla handles the request before the following resync click on the same
/// thread, and answers that click with full contents. Neither version sends any
/// packet when it ignores a request (spectator, other/invalid menu, unknown or
/// locked recipe, impossible placement, grid cannot be cleared, no increase), so
/// only this boundary separates "processed without effect" from "still pending".
/// Single slot updates never qualify; a changed value is not `ObservedUnchanged`.
fn full_boundary(
    record: &RecipePlacementRecord,
    current: &ReceivedCraftingContext,
) -> Result<Option<u64>> {
    if !record.resync_dispatched
        || (record.id.session.version == MinecraftVersion::Java1_16_1
            && record.resync_reply.is_none())
    {
        return Ok(None);
    }
    let [w, h] = current.grid().dimensions();
    let mut slots = Vec::new();
    for y in 0..h {
        for x in 0..w {
            slots.push(current.grid().input(x, y)?);
        }
    }
    slots.push(current.grid().result());
    for index in 9..45 {
        slots.push(current.inventory().slot(index)?);
    }
    let Some(Some(first)) = slots.first() else {
        return Ok(None);
    };
    let full = first.receive_sequence();
    if full <= record.send.after_sequence || record.resync_reply.is_some_and(|reply| full <= reply)
    {
        return Ok(None);
    }
    let complete = slots
        .iter()
        .all(|slot| slot.is_some_and(|slot| slot.receive_sequence() == full));
    // 1.21.11 carries the cursor in the same contents; 1.16.1 sends it right after.
    let cursor = current
        .inventory()
        .cursor()
        .is_some_and(|cursor| cursor.receive_sequence() >= full);
    Ok((complete && cursor).then_some(full))
}
fn unchanged_boundary(
    record: &RecipePlacementRecord,
    current: &ReceivedCraftingContext,
) -> Result<Option<u64>> {
    let Some(full) = record.resync_boundary.filter(|_| record.ghost.is_none()) else {
        return Ok(None);
    };
    if current.receive_sequence() < full {
        return Ok(None);
    }
    let before = record.plan.source_context();
    let [w, h] = before.grid().dimensions();
    let mut pairs = Vec::new();
    for y in 0..h {
        for x in 0..w {
            pairs.push((current.grid().input(x, y)?, before.grid().input(x, y)?));
        }
    }
    pairs.push((current.grid().result(), before.grid().result()));
    for index in 9..45 {
        pairs.push((
            current.inventory().slot(index)?,
            before.inventory().slot(index)?,
        ));
    }
    // Every value at that boundary equals `before`.
    let unchanged = pairs.iter().all(|(actual, old)| match (actual, old) {
        (Some(actual), Some(old)) => {
            actual.receive_sequence() == full && actual.value() == old.value()
        }
        _ => false,
    });
    let cursor = current
        .inventory()
        .cursor()
        .is_some_and(|cursor| *cursor.value() == SlotKnowledge::Empty);
    Ok((unchanged && cursor).then_some(full))
}

#[cfg(test)]
mod tests;
