//! Caller-owned history for a bounded, explicit pair of inventory slots.
use super::*;
use crate::client::{GameMode, SessionStamp, inventory::InventoryClickStage};

/// Progress of an explicit move. A stopped record is never resumable.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InventoryMoveStage {
    /// No execution has been attempted.
    Ready,
    /// One invocation is in progress; individual clicks need actual receipts.
    Running,
    /// Every planned click has been observed, including an empty final cursor.
    ObservedMoved,
    /// Rejected, timed out, cancelled or uncertain; inspect without replay.
    Stopped,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(super) struct Step {
    pub slot: u16,
    pub before: SlotKnowledge,
    pub cursor_before: SlotKnowledge,
    pub after: SlotKnowledge,
    pub cursor_after: SlotKnowledge,
}

/// History of one explicit move/equipment exchange, retained by the caller.
///
/// Create with [`Self::new`], then pass by mutable reference to
/// [`super::super::Survival::move_inventory`]. Dropping that future retains this
/// record. Call `refresh_inventory_move` to read an admitted child's last state;
/// neither execution nor refresh retries or rolls back any click.
#[derive(Clone, Debug, serde::Serialize)]
pub struct InventoryMoveRecord {
    pub(super) source: InventoryClickSource,
    pub(super) source_slot: u16,
    pub(super) destination_slot: u16,
    pub(super) stage: InventoryMoveStage,
    pub(super) clicks: Vec<InventoryClickRecord>,
    pub(super) completed_steps: usize,
    pub(super) stop_reason: Option<String>,
    pub(super) session: Option<SessionStamp>,
    pub(super) mode: Option<GameMode>,
    pub(super) active_step: Option<usize>,
    pub(super) previous: Option<super::super::inventory::InventoryClickId>,
    #[serde(skip)]
    pub(super) plan: Vec<Step>,
}
impl InventoryMoveRecord {
    /// Specify two native slots in one source UI. No item selection is performed.
    pub fn new(source: InventoryClickSource, source_slot: u16, destination_slot: u16) -> Self {
        Self {
            source,
            source_slot,
            destination_slot,
            stage: InventoryMoveStage::Ready,
            clicks: Vec::new(),
            completed_steps: 0,
            stop_reason: None,
            session: None,
            mode: None,
            active_step: None,
            previous: None,
            plan: Vec::new(),
        }
    }
    /// Original UI.
    pub fn source(&self) -> InventoryClickSource {
        self.source
    }
    /// Explicit source slot.
    pub fn source_slot(&self) -> u16 {
        self.source_slot
    }
    /// Explicit destination slot.
    pub fn destination_slot(&self) -> u16 {
        self.destination_slot
    }
    /// Invocation progress, independent of current inventory.
    pub fn stage(&self) -> InventoryMoveStage {
        self.stage
    }
    /// Child identities and historical outcomes, in submission order.
    pub fn clicks(&self) -> &[InventoryClickRecord] {
        &self.clicks
    }
    /// Number of consecutive children confirmed as `ObservedClicked`.
    pub fn completed_steps(&self) -> usize {
        self.completed_steps
    }
    /// First stopping reason, including cancellation of the helper future.
    pub fn stop_reason(&self) -> Option<&str> {
        self.stop_reason.as_deref()
    }
    /// Slot whose submission may have been interrupted before returning its ID.
    /// Refresh the record before admitting another inventory operation.
    pub fn active_slot(&self) -> Option<u16> {
        self.active_step.map(|index| self.plan[index].slot)
    }
    pub(super) fn retain(&mut self, click: InventoryClickRecord) {
        if let Some(last) = self.clicks.last_mut().filter(|last| last.id == click.id) {
            // A completed receive history is immutable even after screen changes.
            if last.stage != InventoryClickStage::ObservedClicked {
                *last = click;
            }
        } else {
            self.clicks.push(click);
        }
        self.completed_steps = self
            .clicks
            .iter()
            .take_while(|click| click.stage == InventoryClickStage::ObservedClicked)
            .count();
    }
}

/// Drop also covers cancellation inside click admission/write, before it returns.
pub(super) struct Running<'a>(pub &'a mut InventoryMoveRecord);
impl Drop for Running<'_> {
    fn drop(&mut self) {
        if self.0.stage == InventoryMoveStage::Running {
            self.0.stage = InventoryMoveStage::Stopped;
            self.0.stop_reason.get_or_insert_with(|| {
                "move future cancelled; inspect child record and cursor".into()
            });
        }
    }
}

pub(super) fn plan(
    initial: &super::super::PlayerObservation,
    screen: Option<&super::super::container::ContainerScreen>,
    registries: super::super::registry::ServerRegistryObservation,
    record: &InventoryMoveRecord,
    mode: GameMode,
) -> Result<Vec<Step>> {
    use super::super::inventory::{ItemContext, slot_policy};
    let slots = match record.source {
        InventoryClickSource::Player => &initial.inventory.slots,
        InventoryClickSource::Container { .. } => {
            &screen
                .ok_or_else(|| error(ErrorKind::State, "move screen unavailable"))?
                .slots
        }
    };
    let get = |slot: u16| -> Result<ObservedValue<SlotKnowledge>> {
        slots
            .get(usize::from(slot))
            .and_then(Option::as_ref)
            .filter(|v| {
                matches!(v.source, super::super::ValueSource::Received { .. })
                    && v.value != SlotKnowledge::Unavailable
            })
            .cloned()
            .ok_or_else(|| {
                error(
                    ErrorKind::State,
                    "move requires received source and destination",
                )
            })
    };
    if record.source_slot == record.destination_slot {
        return Err(error(
            ErrorKind::InvalidInput,
            "move requires different source and destination slots",
        ));
    }
    let from = get(record.source_slot)?;
    let to = get(record.destination_slot)?;
    if !matches!(from.value, SlotKnowledge::Item { .. }) {
        return Err(error(
            ErrorKind::InvalidInput,
            "move source must contain an item",
        ));
    }
    let context = ItemContext::new(registries, initial)?;
    context.predecessor(&from)?;
    context.predecessor(&to)?;
    let menu = screen
        .and_then(|s| s.menu_name.as_deref())
        .unwrap_or("minecraft:player");
    let mut values = [from.value, to.value];
    let mut cursor = SlotKnowledge::Empty;
    let mut result = Vec::new();
    // The only allowed path: take, place/exchange, return the remainder/displaced stack.
    const PATH: [usize; 3] = [0, 1, 0];
    for index in PATH {
        if result.len() == 2 && cursor == SlotKnowledge::Empty {
            break;
        }
        let slot = [record.source_slot, record.destination_slot][index];
        let before = values[index].clone();
        let cursor_before = cursor.clone();
        let equipment =
            matches!(record.source, InventoryClickSource::Player) && matches!(slot, 5..=8 | 45);
        let (after, cursor_after) = if equipment {
            slot_policy::pickup_equipment(
                initial.session.version,
                usize::from(slot),
                InventoryClickButton::Left,
                (&before, &cursor_before),
                mode,
                Some(&context),
            )?
        } else {
            slot_policy::pickup_with_data(
                initial.session.version,
                menu,
                usize::from(slot),
                InventoryClickButton::Left,
                (&before, &cursor_before),
                &context,
            )?
        };
        if context.equivalent_values(&before, &after, false)?
            && context.equivalent_values(&cursor_before, &cursor_after, false)?
        {
            return Err(error(
                ErrorKind::InvalidInput,
                "move PICKUP would have no effect; no packet submitted",
            ));
        }
        values[index] = after.clone();
        cursor = cursor_after.clone();
        result.push(Step {
            slot,
            before,
            cursor_before,
            after,
            cursor_after,
        });
    }
    if cursor != SlotKnowledge::Empty {
        return Err(error(
            ErrorKind::InvalidInput,
            "move cannot finish with an empty cursor in three PICKUPs",
        ));
    }
    Ok(result)
}
