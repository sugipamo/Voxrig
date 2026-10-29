//! An independent owner of one received single-result craft and its external stock.
//! Strict take history is diagnostic input only and is never rewritten or released here.
use super::{CraftingResultDestination, CraftingTakeId, CraftingTakeRecord};
use crate::Result;
use crate::client::{
    ItemStack, PlayerObservation, SlotKnowledge, ValueSource,
    entity::EntityItemStackObservation,
    inventory::{
        InventoryDelta, InventoryLocation, InventorySendId, InventorySendRecord,
        InventorySendStatus, InventorySnapshot, ItemContext, unavailable,
    },
};

/// Operational outcome, independent of strict take and partial inventory comparison.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum OperationalCraftingStage {
    /// Original response and complete received proof are still outstanding.
    Pending,
    /// One result was received with complete consumption and external attribution.
    Crafted,
    /// Missing, contradictory or retired evidence; never repaired by a later resync.
    RequiresInspection,
}
/// A packet-backed pickup, returned separately from the manufactured result.
#[derive(Clone, Debug, serde::Serialize)]
pub struct CraftExternal {
    /// Original entity lifetime and complete received stack metadata.
    pub metadata: EntityItemStackObservation,
    /// Actual collector in the original packet, equal to this take's player entity.
    pub collector: i32,
    /// Actual collected quantity (may be a partial entity stack).
    pub count: u32,
    /// Original collect packet ordering fact.
    pub receive_sequence: u64,
}
/// Retain this report before continuing or sending another inventory operation.
#[derive(Clone, Debug, serde::Serialize)]
pub struct OperationalCraftingOutcome {
    /// Original strict diagnostic identity; its status is not changed by this owner.
    pub take: CraftingTakeId,
    /// Exact original send, session and opening.
    pub send: InventorySendId,
    /// Operational craft proof only.
    pub stage: OperationalCraftingStage,
    /// All proven pickups, never folded into crafted output.
    pub external: Vec<CraftExternal>,
    /// Exact received residual slot/stock changes explained by `external`.
    pub external_delta: Option<InventoryDelta>,
    /// Original full-content proof boundary.
    pub full_contents_sequence: Option<u64>,
    /// Full received inventory to use as the next plan's basis after continuation.
    pub basis: Option<InventorySnapshot>,
    /// Persistent first operational failure, independent of strict inspection.
    pub requires_inspection: Option<String>,
    /// Explicit continuation has released this operational owner.
    pub released: bool,
}
#[derive(Clone)]
pub(crate) struct Owner {
    pub(crate) outcome: OperationalCraftingOutcome,
    pub(crate) claimed: bool,
    pub(crate) released: bool,
    record: CraftingTakeRecord,
    before: InventorySnapshot,
    last: InventorySnapshot,
    pickups: Vec<CraftExternal>,
}
impl Owner {
    pub(crate) fn new(
        send: InventorySendId,
        record: &CraftingTakeRecord,
        before: InventorySnapshot,
        claimed: bool,
    ) -> Self {
        Self {
            outcome: OperationalCraftingOutcome {
                take: record.id,
                send,
                stage: OperationalCraftingStage::Pending,
                external: Vec::new(),
                external_delta: None,
                full_contents_sequence: None,
                basis: None,
                requires_inspection: None,
                released: false,
            },
            claimed,
            released: false,
            record: record.clone(),
            last: before.clone(),
            before,
            pickups: Vec::new(),
        }
    }
    pub(crate) fn fail(&mut self, reason: &str) {
        if !self.released {
            self.outcome
                .requires_inspection
                .get_or_insert_with(|| reason.into());
            self.outcome.stage = OperationalCraftingStage::RequiresInspection;
        }
    }
    pub(crate) fn tracking(&self) -> bool {
        !self.released
            && (self.outcome.stage == OperationalCraftingStage::Pending
                || (self.claimed && self.outcome.stage == OperationalCraftingStage::Crafted))
    }
    pub(crate) fn player(&mut self, player: &PlayerObservation) {
        if !self.tracking() {
            return;
        }
        if player.session != self.record.initial.session
            || player.game_mode != Some(self.record.mode)
            || player.entity_id != self.record.initial.entity_id
            || player.selected_hotbar.as_ref().map(|v| v.value)
                != self
                    .record
                    .initial
                    .selected_hotbar
                    .as_ref()
                    .map(|v| v.value)
        {
            self.fail("operational crafting player/session/mode/selection changed");
        }
    }
    pub(crate) fn collect(
        &mut self,
        entity: i32,
        collector: i32,
        count: i32,
        sequence: u64,
        metadata: Option<EntityItemStackObservation>,
    ) {
        if self.released || self.outcome.stage == OperationalCraftingStage::RequiresInspection {
            return;
        }
        if self.outcome.stage == OperationalCraftingStage::Crafted {
            self.fail("collect received after proof but before continuation");
            return;
        }
        if Some(collector) != self.record.initial.entity_id {
            self.fail("another player's collect cannot explain this craft's stock");
            return;
        }
        let Some(metadata) = metadata else {
            self.fail("collected entity item metadata missing");
            return;
        };
        let valid = metadata.entity.session() == self.outcome.send.session()
            && metadata.entity.native_id() == entity
            && metadata.entity.spawn_sequence() < sequence
            && matches!(metadata.stack.source, ValueSource::Received { sequence: s } if s < sequence && s >= metadata.entity.spawn_sequence())
            && matches!(&metadata.stack.value, SlotKnowledge::Item { item } if count > 0 && count as u32 <= item.count);
        if !valid
            || self.pickups.len() >= 64
            || self
                .pickups
                .iter()
                .any(|p| p.metadata.entity == metadata.entity)
        {
            self.fail("collect quantity/lifetime invalid or pickup history incomplete");
            return;
        }
        self.pickups.push(CraftExternal {
            metadata,
            collector,
            count: count as u32,
            receive_sequence: sequence,
        });
    }
    fn context(&self) -> Result<ItemContext> {
        ItemContext::new(
            self.record.before.registry_state().clone(),
            &self.record.initial,
        )
    }
    fn input_locations(&self) -> Result<Vec<InventoryLocation>> {
        let [w, h] = self.record.before.dimensions();
        (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .map(|(x, y)| {
                let (_, slot) = self.record.before.input_source(x, y)?;
                Ok(self.before.screen_location(usize::from(slot)))
            })
            .collect()
    }
    /// Every observed storage transition must be a compatible positive addition.
    /// Thus a transient removal/data swap cannot disappear behind the final full state.
    fn stock_progress(&self, current: &InventorySnapshot) -> Result<()> {
        let context = self.context()?;
        let inputs = self.input_locations()?;
        for (location, old) in self.last.locations() {
            if inputs.contains(&location)
                || self.before.preview(location)
                || location == InventoryLocation::Cursor
            {
                continue;
            }
            let old = old.ok_or_else(|| unavailable("stock predecessor missing"))?;
            let now = current
                .value(location)
                .ok_or_else(|| unavailable("stock receipt missing"))?;
            if !matches!(now.source, ValueSource::Received { .. }) {
                return Err(unavailable("stock successor is not a received value"));
            }
            let okay = match (&old.value, &now.value) {
                (SlotKnowledge::Empty, SlotKnowledge::Empty | SlotKnowledge::Item { .. }) => true,
                (SlotKnowledge::Item { item: a }, SlotKnowledge::Item { item: b }) => {
                    b.count >= a.count && context.same_data(a, b)?
                }
                _ => false,
            };
            if !okay {
                return Err(unavailable("unexplained intermediate stock change"));
            }
        }
        Ok(())
    }
    fn cursor_progress(
        &self,
        send: &InventorySendRecord,
        current: &InventorySnapshot,
    ) -> Result<()> {
        let Some(cursor) = current.cursor.as_ref().filter(|v| matches!(v.source, ValueSource::Received {sequence} if sequence > send.after_sequence)) else { return Ok(()); };
        let context = self.context()?;
        let before =
            context.equivalent_values(&cursor.value, &self.record.cursor_before.value, false)?;
        let output = context.equivalent_values(
            &cursor.value,
            &self.record.cursor_prediction.value,
            false,
        )?;
        let already_output = self.last.cursor.as_ref().is_some_and(|v| {
            matches!(v.source, ValueSource::Received {sequence} if sequence > send.after_sequence)
                && context
                    .equivalent_values(&v.value, &self.record.cursor_prediction.value, false)
                    .unwrap_or(false)
        });
        if !output && (!before || already_output) {
            return Err(unavailable("intermediate crafted cursor conflicts"));
        }
        Ok(())
    }
    pub(crate) fn receive(
        &mut self,
        send: &InventorySendRecord,
        current: &InventorySnapshot,
        ready: bool,
    ) {
        if self.released || self.outcome.stage == OperationalCraftingStage::RequiresInspection {
            return;
        }
        if current.session != self.before.session
            || current.source != self.before.source
            || send.id != self.outcome.send
        {
            self.fail("operational crafting scope/send retired");
            return;
        }
        if self.outcome.stage == OperationalCraftingStage::Crafted {
            if self
                .outcome
                .basis
                .as_ref()
                .is_none_or(|b| !b.delta(current).slots.is_empty())
            {
                self.fail("inventory changed before operational continuation");
            }
            return;
        }
        if let Err(error) = self.cursor_progress(send, current) {
            self.fail(&error.to_string());
            return;
        }
        if let Err(error) = self.stock_progress(current) {
            self.fail(&error.to_string());
            return;
        }
        self.last = current.clone();
        if send.status == InventorySendStatus::ResyncRequired || send.resync {
            self.fail("operational crafting delivery/history incomplete");
            return;
        }
        if !send.dispatched || !ready {
            return;
        }
        match self.prove(send, current) {
            Ok((delta, full)) => {
                self.outcome.stage = OperationalCraftingStage::Crafted;
                self.outcome.external = self.pickups.clone();
                self.outcome.external_delta = Some(delta);
                self.outcome.full_contents_sequence = Some(full);
                self.outcome.basis = Some(current.clone());
            }
            Err(error) => self.fail(&error.to_string()),
        }
    }
    fn prove(
        &self,
        send: &InventorySendRecord,
        current: &InventorySnapshot,
    ) -> Result<(InventoryDelta, u64)> {
        if self.record.destination != CraftingResultDestination::Cursor {
            return Err(unavailable(
                "operational proof requires one result PICKUP; shift crafts require inspection",
            ));
        }
        let full = send
            .contents_sequence
            .filter(|s| *s > send.after_sequence)
            .ok_or_else(|| unavailable("fresh full contents missing"))?;
        let context = self.context()?;
        let mut expected = self.before.clone();
        let inputs = self.input_locations()?;
        let mut consumed = false;
        for location in &inputs {
            let old = self
                .before
                .value(*location)
                .ok_or_else(|| unavailable("input predecessor missing"))?;
            let value = match &old.value {
                SlotKnowledge::Empty => SlotKnowledge::Empty,
                SlotKnowledge::Item { item } if item.count > 0 => {
                    consumed = true;
                    if item.count == 1 {
                        SlotKnowledge::Empty
                    } else {
                        let mut item = item.clone();
                        item.count -= 1;
                        SlotKnowledge::Item { item }
                    }
                }
                _ => return Err(unavailable("recipe input unavailable")),
            };
            expected.set(*location, value);
        }
        if !consumed {
            return Err(unavailable("recipe consumption absent"));
        }
        let result = self.before.screen_location(0);
        for location in inputs
            .iter()
            .copied()
            .chain([result])
            .chain((9..45).map(|slot| InventoryLocation::Player { slot }))
        {
            let actual = current
                .value(location)
                .ok_or_else(|| unavailable("complete grid/result/stock receipt missing"))?;
            if actual.source != (ValueSource::Received { sequence: full }) {
                return Err(unavailable(
                    "latest grid/result/stock not one original full boundary",
                ));
            }
            if inputs.contains(&location)
                && !context.equivalent_values(
                    &actual.value,
                    &expected.value(location).expect("input set").value,
                    false,
                )?
            {
                return Err(unavailable("received recipe consumption/remainder differs"));
            }
        }
        let actual_result = &current.value(result).expect("validated result").value;
        if !matches!(actual_result, SlotKnowledge::Empty)
            && !context.equivalent_values(
                actual_result,
                &self
                    .before
                    .value(result)
                    .expect("validated predecessor")
                    .value,
                false,
            )?
        {
            return Err(unavailable("result preview conflicts with original recipe"));
        }
        let cursor = current
            .cursor
            .as_ref()
            .filter(|v| matches!(v.source,ValueSource::Received {sequence} if sequence >= full))
            .ok_or_else(|| unavailable("fresh received cursor missing"))?;
        if !context.equivalent_values(&cursor.value, &self.record.cursor_prediction.value, false)? {
            return Err(unavailable("received crafted output differs"));
        }
        expected.set(InventoryLocation::Cursor, cursor.value.clone());
        expected.set(result, actual_result.clone());
        let delta = expected.delta(current);
        if !delta.complete {
            return Err(unavailable("inventory history/predecessors incomplete"));
        }
        let mut gains: Vec<(ItemStack, u64)> = Vec::new();
        for slot in &delta.slots {
            if !matches!(slot.location, InventoryLocation::Player { slot: 9..=45 }) {
                return Err(unavailable("residual change outside pickup stock"));
            }
            if !slot.after.as_ref().is_some_and(|value| matches!(value.source,
                ValueSource::Received {sequence} if sequence > send.after_sequence && sequence <= current.receive_sequence)) {
                return Err(unavailable("residual stock change has no fresh received receipt"));
            }
            for item in &slot.items {
                if item.change <= 0 {
                    return Err(unavailable("residual stock removal"));
                }
                add(&context, &mut gains, &item.item, item.change as u64)?;
            }
        }
        let mut pickups = Vec::new();
        for event in &self.pickups {
            if event.receive_sequence <= send.after_sequence || event.receive_sequence >= full {
                return Err(unavailable(
                    "collect lies outside original send/full boundary",
                ));
            }
            let SlotKnowledge::Item { item } = &event.metadata.stack.value else {
                return Err(unavailable("collect stack missing"));
            };
            add(&context, &mut pickups, item, u64::from(event.count))?;
        }
        if gains.len() != pickups.len()
            || gains.iter().any(|(item, count)| {
                !pickups
                    .iter()
                    .any(|(other, n)| count == n && context.same_data(item, other).unwrap_or(false))
            })
        {
            return Err(unavailable(
                "collect item/quantity does not exactly explain residual stock",
            ));
        }
        Ok((delta, full))
    }
    pub(crate) fn release(&mut self, current: &InventorySnapshot) -> Result<()> {
        if self.outcome.stage != OperationalCraftingStage::Crafted
            || self
                .outcome
                .basis
                .as_ref()
                .is_none_or(|b| !b.delta(current).slots.is_empty())
        {
            return Err(unavailable(
                "operational continuation requires unchanged proven full inventory",
            ));
        }
        self.outcome.basis = Some(current.clone());
        self.released = true;
        self.outcome.released = true;
        Ok(())
    }
}
fn add(
    context: &ItemContext,
    values: &mut Vec<(ItemStack, u64)>,
    item: &ItemStack,
    count: u64,
) -> Result<()> {
    for (other, total) in values.iter_mut() {
        if context.same_data(item, other)? {
            *total = total
                .checked_add(count)
                .ok_or_else(|| unavailable("quantity overflow"))?;
            return Ok(());
        }
    }
    values.push((item.clone(), count));
    Ok(())
}
impl crate::Client {
    /// Claim/poll one operational craft. Claiming blocks subsequent inventory sends
    /// until explicit successful continuation. Retain the report, including External.
    /// The strict take and inventory send statuses retain their original meanings.
    pub async fn operational_crafting_outcome(
        &self,
        send: InventorySendId,
    ) -> Result<OperationalCraftingOutcome> {
        crate::client::dispatch!(&self.adapter, a => crate::client::adapter::EventOps::operational_crafting(a,send,false).await)
    }
    /// Release only this operational owner from its still-current full inventory.
    /// Plan the next acceptance operation from the returned received basis, respecting
    /// its occupied cursor. No click is sent and strict inspection remains historical.
    pub async fn continue_operational_crafting(
        &self,
        send: InventorySendId,
    ) -> Result<OperationalCraftingOutcome> {
        crate::client::dispatch!(&self.adapter, a => crate::client::adapter::EventOps::operational_crafting(a,send,true).await)
    }
}
