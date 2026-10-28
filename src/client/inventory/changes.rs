//! Received inventory history. Comparison values never write into the reducer.
use super::{InventorySendId, InventorySource};
use crate::client::{
    InventoryObservation, ItemStack, ObservedValue, PlayerObservation, SessionStamp, SlotKnowledge,
    ValueSource,
    container::{ContainerScreen, ScreenId},
};
use crate::{Error, ErrorKind, Result};
use std::collections::VecDeque;

/// Maximum retained inventory change packets per connection.
pub const MAX_RETAINED_INVENTORY_CHANGES: usize = 1024;

/// A physical slot, without duplicating a menu's appended player inventory.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum InventoryLocation {
    /// Canonical player screen slot 0..45.
    Player {
        /// Native player slot.
        slot: usize,
    },
    /// Non-player slot in one received opening.
    Container {
        /// Original opening.
        screen: ScreenId,
        /// Native screen slot.
        slot: usize,
    },
    /// Carried stack, independent of the selected hotbar slot.
    Cursor,
}
/// Known item quantity change. Identity includes native item data; count is one.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct InventoryItemDelta {
    /// Item identity, with count normalized to one.
    pub item: ItemStack,
    /// Signed increase (negative for removal).
    pub change: i64,
}
/// Before/after receipts of one slot or cursor. None means unknown, never Empty.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct InventorySlotDelta {
    /// Canonical location.
    pub location: InventoryLocation,
    /// Original receipt, absent if not established.
    pub before: Option<ObservedValue<SlotKnowledge>>,
    /// New receipt, absent if invalidated.
    pub after: Option<ObservedValue<SlotKnowledge>>,
    /// Known quantity changes at this location.
    pub items: Vec<InventoryItemDelta>,
}
/// Changes between two captures; repeated identical receipts are not changes.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct InventoryDelta {
    /// Net known physical stock changes, including cursor, excluding result previews.
    pub items: Vec<InventoryItemDelta>,
    /// Changed slots and cursor, including unknown/known transitions and result previews.
    pub slots: Vec<InventorySlotDelta>,
    /// False if any changed location has an unknown predecessor or successor.
    pub complete: bool,
}
/// Immutable received basis for a comparison, not another current inventory owner.
#[derive(Clone, Debug, serde::Serialize)]
pub struct InventorySnapshot {
    /// Connection/version/world at capture.
    pub session: SessionStamp,
    /// Applied receive boundary. Individual values retain their original ordinals.
    pub receive_sequence: u64,
    /// Active source; menu IDs include opening identity.
    pub source: InventorySource,
    /// Canonical player slots (None means unknown).
    pub player_slots: Vec<Option<ObservedValue<SlotKnowledge>>>,
    /// Same-opening menu capture; appended player slots are aliases, not extra stock.
    pub screen: Option<ContainerScreen>,
    /// Actual received carried stack.
    pub cursor: Option<ObservedValue<SlotKnowledge>>,
}
impl InventorySnapshot {
    pub(crate) fn capture(player: &PlayerObservation, screen: Option<ContainerScreen>) -> Self {
        Self::from_inventory(
            player.session,
            player.receive_sequence,
            &player.inventory,
            screen,
        )
    }
    pub(crate) fn from_inventory(
        session: SessionStamp,
        sequence: u64,
        inventory: &InventoryObservation,
        screen: Option<ContainerScreen>,
    ) -> Self {
        let screen = screen.filter(|s| {
            inventory.window_id == Some(s.id.window_id())
                && !matches!(
                    inventory.player_screen,
                    Some(crate::client::container::PlayerScreenAccess::SubmittedClose { .. })
                )
        });
        let source =
            screen
                .as_ref()
                .map_or(InventorySource::Player, |s| InventorySource::Container {
                    screen: s.id,
                });
        Self {
            session,
            receive_sequence: sequence,
            source,
            player_slots: inventory.slots.clone(),
            screen,
            cursor: inventory.cursor.clone().filter(received),
        }
    }
    pub(crate) fn locations(
        &self,
    ) -> Vec<(InventoryLocation, Option<ObservedValue<SlotKnowledge>>)> {
        let mut values: Vec<_> = self
            .player_slots
            .iter()
            .enumerate()
            .map(|(slot, value)| {
                (
                    InventoryLocation::Player { slot },
                    value.clone().filter(received),
                )
            })
            .collect();
        if let Some(screen) = &self.screen {
            for (slot, value) in screen.slots.iter().enumerate() {
                if !screen
                    .layout
                    .as_ref()
                    .is_some_and(|l| l.player_slots.iter().any(|m| m.screen_slot == slot))
                {
                    values.push((
                        InventoryLocation::Container {
                            screen: screen.id,
                            slot,
                        },
                        value.clone().filter(received),
                    ));
                }
            }
        }
        values.push((InventoryLocation::Cursor, self.cursor.clone()));
        values
    }
    pub(crate) fn screen_location(&self, slot: usize) -> InventoryLocation {
        self.screen
            .as_ref()
            .map_or(InventoryLocation::Player { slot }, |screen| {
                screen
                    .layout
                    .as_ref()
                    .and_then(|l| l.player_slots.iter().find(|m| m.screen_slot == slot))
                    .map_or(
                        InventoryLocation::Container {
                            screen: screen.id,
                            slot,
                        },
                        |m| InventoryLocation::Player {
                            slot: m.player_slot,
                        },
                    )
            })
    }
    pub(crate) fn preview(&self, location: InventoryLocation) -> bool {
        match location {
            InventoryLocation::Player { slot: 0 } => true,
            InventoryLocation::Container { screen, slot } => {
                self.screen.as_ref().is_some_and(|s| {
                    s.id == screen
                        && s.menu_name.as_deref().is_some_and(|name| {
                            crate::client::crafting::native_menu(self.session.version, name)
                                .is_some_and(|menu| menu.result_slot == slot)
                        })
                })
            }
            _ => false,
        }
    }
    pub(crate) fn value(
        &self,
        location: InventoryLocation,
    ) -> Option<&ObservedValue<SlotKnowledge>> {
        match location {
            InventoryLocation::Player { slot } => {
                self.player_slots.get(slot).and_then(Option::as_ref)
            }
            InventoryLocation::Container { screen, slot } => self
                .screen
                .as_ref()
                .filter(|s| s.id == screen)
                .and_then(|s| s.slots.get(slot))
                .and_then(Option::as_ref),
            InventoryLocation::Cursor => self.cursor.as_ref(),
        }
    }
    fn contains_location(&self, location: InventoryLocation) -> bool {
        match location {
            InventoryLocation::Player { slot } => slot < self.player_slots.len(),
            InventoryLocation::Container { screen, slot } => self
                .screen
                .as_ref()
                .is_some_and(|s| s.id == screen && slot < s.slots.len()),
            InventoryLocation::Cursor => true,
        }
    }
    pub(crate) fn set(&mut self, location: InventoryLocation, value: SlotKnowledge) {
        let predicted = Some(ObservedValue {
            value,
            source: ValueSource::Predicted,
        });
        match location {
            InventoryLocation::Player { slot } => self.player_slots[slot] = predicted,
            InventoryLocation::Container { slot, .. } => {
                self.screen.as_mut().expect("original screen").slots[slot] = predicted;
            }
            InventoryLocation::Cursor => self.cursor = predicted,
        }
    }
    pub(crate) fn delta(&self, after: &Self) -> InventoryDelta {
        // Comparison copies can contain explicitly tagged Predicted values. They
        // are never exposed by locations() or fed back into received inventory.
        let raw = |s: &Self| {
            let mut values = s.locations();
            for (location, value) in &mut values {
                *value = match *location {
                    InventoryLocation::Player { slot } => s.player_slots[slot].clone(),
                    InventoryLocation::Container { slot, .. } => {
                        s.screen.as_ref().unwrap().slots[slot].clone()
                    }
                    InventoryLocation::Cursor => s.cursor.clone(),
                };
            }
            values
        };
        let mut before = raw(self);
        let next = raw(after);
        for (location, _) in &next {
            if !self.contains_location(*location) {
                before.push((*location, None));
            }
        }
        let mut delta = InventoryDelta {
            complete: true,
            ..Default::default()
        };
        for (location, value) in before {
            let actual = after.value(location).cloned();
            if value.as_ref().map(|v| &v.value) == actual.as_ref().map(|v| &v.value) {
                continue;
            }
            let mut items = Vec::new();
            for (v, sign) in [(&value, -1), (&actual, 1)] {
                match v.as_ref().map(|v| &v.value) {
                    Some(SlotKnowledge::Item { item }) => {
                        add_item(&mut items, item, sign * i64::from(item.count))
                    }
                    Some(SlotKnowledge::Empty) => {}
                    _ => delta.complete = false,
                }
            }
            // Result slots are previews; they are not physical stock.
            if !self.preview(location) && !after.preview(location) {
                for item in &items {
                    add_item(&mut delta.items, &item.item, item.change);
                }
            }
            delta.slots.push(InventorySlotDelta {
                location,
                before: value,
                after: actual,
                items,
            });
        }
        delta
    }
}
fn received(value: &ObservedValue<SlotKnowledge>) -> bool {
    matches!(value.source, ValueSource::Received { .. })
}
fn add_item(items: &mut Vec<InventoryItemDelta>, item: &ItemStack, change: i64) {
    let mut identity = item.clone();
    identity.count = 1;
    if let Some(existing) = items.iter_mut().find(|v| v.item == identity) {
        existing.change += change;
    } else {
        items.push(InventoryItemDelta {
            item: identity,
            change,
        });
    }
    items.retain(|v| v.change != 0);
}
/// Evidence associated with a receive boundary; correlation is not proof of cause.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum InventoryCause {
    /// Own in-flight write; untagged slot packets do not prove causal ownership.
    Send {
        /// Original send identity.
        id: InventorySendId,
    },
    /// A matching legacy negative comparison reply, not necessarily rollback.
    Rejected,
    /// Actual collect_item for this player, received in this operation/batch.
    Collected {
        /// Collected entity IDs, deduplicated in receive order.
        entity_ids: Vec<i32>,
    },
    /// Completed explicit resync after an expired send; the old intent was not replayed.
    TimedOutResync,
    /// Received inside the bounded window of an own non-executor action
    /// (item use, placement, dig, selected drop, hand swap) at a slot that
    /// action can change. Like `Send`, this does not prove causal ownership.
    OwnAction {
        /// Kind of the written frame.
        action: super::InventoryOwnActionKind,
        /// Per-connection order of own actions, starting at 1.
        ordinal: u64,
        /// 1.21.11 interaction sequence of the frame; None on 1.16.1.
        interaction_sequence: Option<i32>,
        /// Changed locations of this delta inside the action's scope.
        locations: Vec<InventoryLocation>,
    },
    /// No more specific received explanation; also used for unexplained residual
    /// changes, including slots outside every `OwnAction` scope of the same delta.
    External,
}
/// One receive-time inventory change; immutable after recording.
#[derive(Clone, Debug, serde::Serialize)]
pub struct InventoryChange {
    /// Exact connection/world owner.
    pub session: SessionStamp,
    /// Original packet receive ordinal; use the log boundary as the next cursor.
    pub receive_sequence: u64,
    /// Actual active opening at receipt.
    pub source: InventorySource,
    /// Physical stock, slot and cursor differences.
    pub delta: InventoryDelta,
    /// Received hints, not an assertion of causality.
    pub causes: Vec<InventoryCause>,
}
/// Inventory changes after a receive cursor, in receive order.
#[derive(Clone, Debug, serde::Serialize)]
pub struct InventoryChangeLog {
    /// Next receive cursor (initially use zero), independent of events_after's cursor.
    pub receive_sequence: u64,
    /// Retained records in the current world; opening identities remain explicit.
    pub changes: Vec<InventoryChange>,
}
impl crate::Client {
    /// Read bounded inventory differences in either supported version. Sends no packets.
    /// Retention/world gaps return State; a future cursor returns InvalidInput.
    /// Readable after disconnect. Resume a gap by capturing current inventory and
    /// using its receive boundary; never silently infer missing differences.
    pub async fn inventory_changes_after(&self, after_sequence: u64) -> Result<InventoryChangeLog> {
        crate::client::dispatch!(&self.adapter, a => crate::client::adapter::EventOps::inventory_changes_after(a, after_sequence).await)
    }
}
#[derive(Default)]
pub(crate) struct InventoryChangeLedger {
    records: VecDeque<InventoryChange>,
    previous: Option<InventorySnapshot>,
    dropped_through: u64,
    collected: Vec<(i32, u64)>,
    own_actions: VecDeque<super::own_action::OwnActionWindow>,
    own_ordinal: u64,
}
impl InventoryChangeLedger {
    /// Open windows for own actions resolved from the writer handoff.
    pub(crate) fn own_actions(
        &mut self,
        windows: impl IntoIterator<Item = super::own_action::OwnActionWindow>,
    ) {
        self.own_actions.extend(windows);
        while self.own_actions.len() > MAX_RETAINED_INVENTORY_CHANGES {
            self.own_actions.pop_front();
        }
    }
    pub(crate) fn own_ordinal(&mut self) -> &mut u64 {
        &mut self.own_ordinal
    }
    // Own-action hints for the changed slots of one delta, plus whether every
    // changed location is inside some window's scope.
    fn own_action_causes(
        &mut self,
        session: SessionStamp,
        delta: &InventoryDelta,
    ) -> (Vec<InventoryCause>, bool) {
        let now = tokio::time::Instant::now();
        self.own_actions
            .retain(|w| w.session == session && w.deadline > now);
        let mut causes = Vec::new();
        let mut covered = vec![false; delta.slots.len()];
        for window in &self.own_actions {
            let locations: Vec<_> = delta
                .slots
                .iter()
                .zip(&mut covered)
                .filter(|(slot, _)| window.scope.contains(&slot.location))
                .map(|(slot, covered)| {
                    *covered = true;
                    slot.location
                })
                .collect();
            if !locations.is_empty() {
                causes.push(InventoryCause::OwnAction {
                    action: window.kind,
                    ordinal: window.ordinal,
                    interaction_sequence: window.interaction_sequence,
                    locations,
                });
            }
        }
        (causes, covered.iter().all(|c| *c))
    }
    pub(crate) fn collect(&mut self, entity: i32, sequence: u64) {
        if !self.collected.iter().any(|(id, _)| *id == entity) {
            self.collected.push((entity, sequence));
        }
        if self.collected.len() > MAX_RETAINED_INVENTORY_CHANGES {
            self.collected.remove(0);
        }
    }
    pub(crate) fn receive(
        &mut self,
        current: InventorySnapshot,
        send: Option<InventorySendId>,
        hints: &[InventoryCause],
    ) {
        // Untagged slot updates cannot establish causality. Associate collect
        // hints only with the next nearby delta, never an arbitrary later /give.
        self.collected
            .retain(|(_, seq)| current.receive_sequence.saturating_sub(*seq) <= 128);
        if let Some(previous) = self.previous.take() {
            if previous.session != current.session || previous.source != current.source {
                self.records.clear();
                self.dropped_through = current.receive_sequence;
                self.collected.clear();
                if previous.session != current.session {
                    self.own_actions.clear();
                }
            } else {
                let delta = previous.delta(&current);
                if !delta.slots.is_empty() {
                    let mut causes = send
                        .filter(|id| {
                            id.session() == current.session && id.source() == current.source
                        })
                        .map(|id| vec![InventoryCause::Send { id }])
                        .unwrap_or_default();
                    for hint in hints {
                        if !causes.contains(hint) {
                            causes.push(hint.clone());
                        }
                    }
                    if !self.collected.is_empty() {
                        let cause = InventoryCause::Collected {
                            entity_ids: std::mem::take(&mut self.collected)
                                .into_iter()
                                .map(|(id, _)| id)
                                .collect(),
                        };
                        if !causes.contains(&cause) {
                            causes.push(cause);
                        }
                    }
                    // Own-action hints never hide a change outside their scope.
                    let (own, covered) = self.own_action_causes(current.session, &delta);
                    let explained = !causes.is_empty() || (!own.is_empty() && covered);
                    causes.extend(own);
                    if !explained {
                        causes.push(InventoryCause::External);
                    }
                    self.records.push_back(InventoryChange {
                        session: current.session,
                        receive_sequence: current.receive_sequence,
                        source: current.source,
                        delta,
                        causes,
                    });
                    if self.records.len() > MAX_RETAINED_INVENTORY_CHANGES {
                        self.dropped_through = self.records.pop_front().unwrap().receive_sequence;
                    }
                }
            }
        }
        self.previous = Some(current);
    }
    pub(crate) fn after(&self, after: u64, sequence: u64) -> Result<InventoryChangeLog> {
        if after > sequence {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                anyhow::anyhow!("inventory cursor is ahead of reception"),
            ));
        }
        if after < self.dropped_through {
            return Err(Error::new(
                ErrorKind::State,
                anyhow::anyhow!(
                    "inventory changes were dropped; capture current inventory before resuming"
                ),
            ));
        }
        Ok(InventoryChangeLog {
            receive_sequence: sequence,
            changes: self
                .records
                .iter()
                .filter(|r| r.receive_sequence > after)
                .cloned()
                .collect(),
        })
    }
}
