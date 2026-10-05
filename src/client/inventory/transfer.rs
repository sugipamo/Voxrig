//! One QUICK_MOVE intent with separate predictions and actual changed-slot receipts.
use super::{
    InventoryClickSend, InventorySource, InventoryTransactionReply, transfer_policy, unavailable,
};
use crate::client::{
    GameMode, ObservedValue, PlayerObservation, SessionStamp, SlotKnowledge, ValueSource,
    container::{ContainerScreen, PlayerScreenAccess},
};
use crate::{MinecraftVersion, Result};

/// Original connection/world-local transfer, never replay authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct InventoryTransferId {
    session: SessionStamp,
    attempt: u64,
}
impl InventoryTransferId {
    /// Original connection/world.
    pub fn session(self) -> SessionStamp {
        self.session
    }
    /// Monotonic local attempt number.
    pub fn attempt(self) -> u64 {
        self.attempt
    }
}
/// One destination/source that the native model expects to change.
#[derive(Clone, Debug, serde::Serialize)]
pub struct InventoryTransferSlot {
    /// Original native screen slot.
    pub slot: u16,
    /// Constructor-mapped canonical player slot, absent for storage contents.
    pub player_slot: Option<usize>,
    /// Complete received predecessor.
    pub before: ObservedValue<SlotKnowledge>,
    /// Separate native prediction, never applied to received slots.
    pub prediction: ObservedValue<SlotKnowledge>,
    /// Exact fresh actual receipt, required even for partial transfer.
    pub receipt: Option<ObservedValue<SlotKnowledge>>,
}
/// Retained state of one source transfer; no submission is repeated.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InventoryTransferStage {
    /// Submission or required changed-slot receipts are incomplete.
    Pending,
    /// Full write, all exact fresh changed slots and actual legacy reply agree.
    ObservedTransferred,
    /// First conflict/uncertainty remains even if values later recover.
    RequiresInspection,
}
/// One retained ordinary shift transfer, including partial destination capacity.
#[derive(Clone, Debug, serde::Serialize)]
pub struct InventoryTransferRecord {
    /// Original opaque identity.
    pub id: InventoryTransferId,
    /// Coherent received player baseline before I/O.
    pub initial: PlayerObservation,
    /// Matching actual mode required by the handle.
    pub mode: GameMode,
    /// Original source UI, including same-opening storage player slots.
    pub source: InventorySource,
    /// Original clicked screen slot; no destination override.
    pub source_slot: u16,
    /// Actual opening baseline, absent for player screen.
    pub initial_screen: Option<ContainerScreen>,
    /// Source UI's actual predecessors; missing slots never imply empty.
    pub before_slots: Vec<Option<ObservedValue<SlotKnowledge>>>,
    /// Original received empty cursor; QUICK_MOVE never moves it.
    pub cursor_before: ObservedValue<SlotKnowledge>,
    /// Latest actual unchanged-cursor inspection; may retain an older ordinal.
    /// A fresh cursor update is not claimed or required for an unchanged cursor.
    pub cursor_inspected: Option<ObservedValue<SlotKnowledge>>,
    /// Every predicted change and its separately received result.
    pub changed_slots: Vec<InventoryTransferSlot>,
    /// Native legacy returned-stack prediction, not received outcome; absent on modern.
    pub legacy_return_prediction: Option<ObservedValue<SlotKnowledge>>,
    /// Original submission, using one native QUICK_MOVE frame.
    pub send: InventoryClickSend,
    /// Matching actual legacy response; false comparison is not rollback.
    pub legacy_reply: Option<InventoryTransactionReply>,
    /// First retained conflict.
    pub requires_inspection: Option<String>,
    /// Historical outcome independent of later inventory/mode/connection.
    pub stage: InventoryTransferStage,
}
impl InventoryTransferRecord {
    /// Original native source window, not live authority.
    pub fn window_id(&self) -> i32 {
        match self.source {
            InventorySource::Player => 0,
            InventorySource::Container { screen } => screen.window_id(),
        }
    }
    pub(crate) fn unresolved(&self) -> bool {
        self.stage != InventoryTransferStage::ObservedTransferred
    }
    pub(crate) fn inspection(&mut self, reason: impl std::fmt::Display) {
        if self.unresolved() {
            self.requires_inspection
                .get_or_insert_with(|| reason.to_string());
            self.stage = InventoryTransferStage::RequiresInspection;
        }
    }
    pub(crate) fn ready(&self) -> bool {
        self.send.dispatched
            && self.requires_inspection.is_none()
            && self.cursor_inspected.is_some()
            && self.changed_slots.iter().all(|s| s.receipt.is_some())
            && (self.initial.session.version == MinecraftVersion::Java1_21_11
                || self.legacy_reply.is_some())
    }
}
fn known(value: Option<&ObservedValue<SlotKnowledge>>) -> Result<ObservedValue<SlotKnowledge>> {
    value
        .filter(|v| {
            matches!(v.source, ValueSource::Received { .. })
                && !matches!(v.value, SlotKnowledge::Unavailable)
        })
        .cloned()
        .ok_or_else(|| unavailable("complete received transfer predecessor unavailable"))
}
pub(crate) fn prepare(
    initial: PlayerObservation,
    mode: GameMode,
    source: InventorySource,
    slot: u16,
    attempt: u64,
    screen: Option<ContainerScreen>,
) -> Result<InventoryTransferRecord> {
    if attempt == 0
        || initial.pending_dispatch
        || initial.game_mode != Some(mode)
        || !matches!(mode, GameMode::Survival | GameMode::Creative)
    {
        return Err(unavailable(
            "transfer requires matching received mode and no unresolved dispatch",
        ));
    }
    let (cursor, before, menu, revision) = match source {
        InventorySource::Player => {
            if !(5..=45).contains(&slot) {
                return Err(crate::client::registry::invalid(
                    "player transfer requires slot 5..45; crafting/result remain separate",
                ));
            }
            match initial.inventory.player_screen {
                Some(PlayerScreenAccess::Received) if initial.inventory.window_id == Some(0) => {}
                Some(PlayerScreenAccess::SubmittedClose { close })
                    if close.screen().session() == initial.session => {}
                _ => {
                    return Err(unavailable(
                        "same player UI or completed local close required",
                    ));
                }
            }
            let revision = match initial.inventory.player_screen {
                Some(PlayerScreenAccess::SubmittedClose { .. }) => initial
                    .inventory
                    .player_screen_revision
                    .as_ref()
                    .filter(|r| matches!(r.source, ValueSource::Received { .. }))
                    .map(|r| r.value),
                _ => initial.inventory.screen_revision,
            };
            (
                known(initial.inventory.cursor.as_ref())?,
                initial.inventory.slots.clone(),
                "minecraft:player",
                revision,
            )
        }
        InventorySource::Container { screen: id } => {
            let s = screen
                .as_ref()
                .filter(|s| {
                    s.id == id
                        && id.session() == initial.session
                        && s.full_contents_sequence.is_some()
                        && initial.inventory.window_id == Some(id.window_id())
                })
                .ok_or_else(|| unavailable("same live received storage opening/full required"))?;
            let layout = s
                .layout
                .as_ref()
                .ok_or_else(|| unavailable("constructor-verified transfer layout required"))?;
            if usize::from(slot) >= layout.total_slots {
                return Err(crate::client::registry::invalid(
                    "transfer outside native storage layout",
                ));
            }
            let menu = s
                .menu_name
                .as_deref()
                .filter(|n| super::storage_menu(n))
                .ok_or_else(|| unavailable("audited storage transfer menu required"))?;
            for mapping in &layout.player_slots {
                if s.slots.get(mapping.screen_slot)
                    != initial.inventory.slots.get(mapping.player_slot)
                {
                    return Err(unavailable("transfer screen/player receipts disagree"));
                }
            }
            (
                known(initial.inventory.cursor.as_ref())?,
                s.slots.clone(),
                menu,
                s.revision
                    .as_ref()
                    .filter(|r| matches!(r.source, ValueSource::Received { .. }))
                    .map(|r| r.value),
            )
        }
    };
    if cursor.value != SlotKnowledge::Empty {
        return Err(unavailable(
            "ordinary shift transfer requires received empty cursor",
        ));
    }
    if initial.session.version == MinecraftVersion::Java1_21_11 && revision.is_none() {
        return Err(unavailable("actual transfer UI revision unavailable"));
    }
    if initial.session.version == MinecraftVersion::Java1_16_1
        && matches!(source,InventorySource::Container{screen} if !(1..=127).contains(&screen.window_id()))
    {
        return Err(unavailable("legacy source window must be 1..127"));
    }
    let values: Vec<_> = before
        .iter()
        .map(|v| {
            v.as_ref()
                .filter(|v| matches!(v.source, ValueSource::Received { .. }))
                .map_or(SlotKnowledge::Unavailable, |v| v.value.clone())
        })
        .collect();
    let calculated =
        transfer_policy::calculate(initial.session.version, menu, usize::from(slot), &values)?;
    let mut changed = Vec::new();
    for (index, after) in calculated.slots.into_iter().enumerate() {
        if values[index] == after {
            continue;
        }
        let player_slot = match source {
            InventorySource::Player => Some(index),
            InventorySource::Container { .. } => screen
                .as_ref()
                .and_then(|s| s.layout.as_ref())
                .and_then(|l| l.player_slots.iter().find(|m| m.screen_slot == index))
                .map(|m| m.player_slot),
        };
        changed.push(InventoryTransferSlot {
            slot: index as u16,
            player_slot,
            before: known(before[index].as_ref())?,
            prediction: ObservedValue {
                value: after,
                source: ValueSource::Predicted,
            },
            receipt: None,
        });
    }
    if changed.is_empty() {
        return Err(crate::client::registry::invalid(
            "shift transfer has no effect; no packet submitted",
        ));
    }
    Ok(InventoryTransferRecord {
        id: InventoryTransferId {
            session: initial.session,
            attempt,
        },
        send: InventoryClickSend {
            after_sequence: initial.receive_sequence,
            legacy_action: None,
            legacy_comparison: None,
            screen_revision: revision,
            sent_screen_revision: revision,
            request_full_resync: false,
            dispatched: false,
        },
        legacy_return_prediction: if initial.session.version == MinecraftVersion::Java1_16_1 {
            Some(ObservedValue {
                value: calculated.legacy_return,
                source: ValueSource::Predicted,
            })
        } else {
            None
        },
        initial,
        mode,
        source,
        source_slot: slot,
        initial_screen: if matches!(source, InventorySource::Player) {
            None
        } else {
            screen
        },
        before_slots: before,
        cursor_before: cursor,
        cursor_inspected: None,
        changed_slots: changed,
        legacy_reply: None,
        requires_inspection: None,
        stage: InventoryTransferStage::Pending,
    })
}
pub(crate) fn source_slots<'a>(
    record: &InventoryTransferRecord,
    current: &'a PlayerObservation,
    screen: Option<&'a ContainerScreen>,
) -> Result<&'a Vec<Option<ObservedValue<SlotKnowledge>>>> {
    if current.session != record.initial.session
        || current.game_mode != Some(record.mode)
        || current.selected_hotbar.as_ref().map(|s| s.value)
            != record.initial.selected_hotbar.as_ref().map(|s| s.value)
    {
        return Err(unavailable("transfer session/world/mode/selection changed"));
    }
    let slots = match record.source {
        InventorySource::Player => {
            let received = current.inventory.player_screen == Some(PlayerScreenAccess::Received)
                && current.inventory.window_id == Some(0);
            let closed = matches!((record.initial.inventory.player_screen,current.inventory.player_screen),(Some(PlayerScreenAccess::SubmittedClose{close:a}),Some(PlayerScreenAccess::SubmittedClose{close:b}))if a==b);
            if !received && !closed {
                return Err(unavailable("transfer player UI basis changed"));
            }
            &current.inventory.slots
        }
        InventorySource::Container { screen: id } => {
            let s = screen
                .filter(|s| {
                    s.id == id
                        && s.full_contents_sequence.is_some()
                        && current.inventory.window_id == Some(id.window_id())
                })
                .ok_or_else(|| unavailable("transfer original opening changed"))?;
            let old = record
                .initial_screen
                .as_ref()
                .ok_or_else(|| unavailable("transfer opening baseline missing"))?;
            if old.layout != s.layout || old.menu_name != s.menu_name {
                return Err(unavailable("transfer native layout/menu changed"));
            }
            for mapping in &s
                .layout
                .as_ref()
                .ok_or_else(|| unavailable("transfer layout missing"))?
                .player_slots
            {
                if s.slots.get(mapping.screen_slot)
                    != current.inventory.slots.get(mapping.player_slot)
                {
                    return Err(unavailable("transfer screen/player mapping changed"));
                }
            }
            &s.slots
        }
    };
    if slots.len() != record.before_slots.len() {
        return Err(unavailable("transfer native slot count changed"));
    }
    if matches!(record.source, InventorySource::Container { .. }) {
        if let Some(before) = record
            .initial
            .inventory
            .slots
            .get(45)
            .and_then(Option::as_ref)
            .filter(|v| {
                matches!(v.source, ValueSource::Received { .. })
                    && v.value != SlotKnowledge::Unavailable
            })
        {
            if current
                .inventory
                .slots
                .get(45)
                .and_then(Option::as_ref)
                .is_none_or(|v| {
                    !matches!(v.source, ValueSource::Received { .. }) || v.value != before.value
                })
            {
                return Err(unavailable("transfer unaffected offhand changed"));
            }
        }
    }
    Ok(slots)
}
pub(crate) fn receive(
    record: &mut InventoryTransferRecord,
    current: &PlayerObservation,
    screen: Option<&ContainerScreen>,
) {
    if !record.unresolved() {
        return;
    }
    let slots = match source_slots(record, current, screen) {
        Ok(s) => s,
        Err(e) => {
            record.inspection(e);
            return;
        }
    };
    match known(current.inventory.cursor.as_ref()) {
        Ok(cursor) if cursor.value == record.cursor_before.value => {
            record.cursor_inspected = Some(cursor)
        }
        _ => {
            record.inspection("transfer cursor changed/became unavailable");
            return;
        }
    }
    for (index, before) in record.before_slots.iter().enumerate() {
        let Some(before) = before.as_ref().filter(|v| {
            matches!(v.source, ValueSource::Received { .. })
                && v.value != SlotKnowledge::Unavailable
        }) else {
            continue;
        };
        let actual = slots
            .get(index)
            .and_then(Option::as_ref)
            .filter(|v| matches!(v.source, ValueSource::Received { .. }));
        if let Some(change) = record
            .changed_slots
            .iter_mut()
            .find(|s| usize::from(s.slot) == index)
        {
            let Some(actual) =
                actual.filter(|v| v.value == before.value || v.value == change.prediction.value)
            else {
                record.requires_inspection.get_or_insert_with(|| {
                    "transfer source/destination conflicts or unavailable".into()
                });
                continue;
            };
            let ValueSource::Received { sequence } = actual.source else {
                unreachable!()
            };
            if sequence > record.send.after_sequence {
                if !record.send.dispatched && actual.value != before.value {
                    record
                        .requires_inspection
                        .get_or_insert_with(|| "transfer changed before full dispatch".into());
                } else if actual.value == change.prediction.value {
                    change.receipt.get_or_insert_with(|| actual.clone());
                } else if change.receipt.is_some() {
                    record.requires_inspection.get_or_insert_with(|| {
                        "transfer destination restored before confirmation".into()
                    });
                }
            }
        } else if actual.is_none_or(|v| v.value != before.value) {
            record
                .requires_inspection
                .get_or_insert_with(|| "unplanned transfer slot changed/became unavailable".into());
        }
    }
    if record.requires_inspection.is_some() {
        record.stage = InventoryTransferStage::RequiresInspection;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{InventoryObservation, ItemData, ItemStack, received, registry::Registry};
    fn stack(version: MinecraftVersion, name: &str, count: u32) -> SlotKnowledge {
        let d = Registry::for_version(version).item(name).unwrap();
        SlotKnowledge::Item {
            item: ItemStack {
                id: d.id,
                name: d.name,
                count,
                data: ItemData::Default,
            },
        }
    }
    fn fixture(version: MinecraftVersion, partial: bool) -> InventoryTransferRecord {
        let mut slots = vec![Some(received(SlotKnowledge::Empty, 10)); 46];
        slots[9] = Some(received(stack(version, "minecraft:stone", 7), 10));
        if partial {
            slots[36] = Some(received(stack(version, "minecraft:stone", 63), 10));
            for s in &mut slots[37..45] {
                *s = Some(received(stack(version, "minecraft:dirt", 64), 10));
            }
        }
        let initial = PlayerObservation {
            session: SessionStamp {
                version,
                connection_id: 42,
                world_generation: 1,
            },
            receive_sequence: 10,
            pending_dispatch: false,
            dimension: None,
            position: None,
            received_pose: None,
            rotation: [0., 0.],
            game_mode: Some(GameMode::Survival),
            may_fly: None,
            health: None,
            selected_hotbar: Some(received(0, 10)),
            inventory: InventoryObservation {
                slots,
                cursor: Some(received(SlotKnowledge::Empty, 10)),
                window_id: Some(0),
                player_screen: Some(PlayerScreenAccess::Received),
                screen_revision: Some(7),
                player_screen_revision: Some(received(7, 10)),
                local_cache: None,
            },
        };
        prepare(
            initial,
            GameMode::Survival,
            InventorySource::Player,
            9,
            1,
            None,
        )
        .unwrap()
    }
    fn after(record: &InventoryTransferRecord, sequence: u64) -> PlayerObservation {
        let mut current = record.initial.clone();
        current.receive_sequence = sequence;
        for s in &record.changed_slots {
            current.inventory.slots[usize::from(s.slot)] =
                Some(received(s.prediction.value.clone(), sequence));
        }
        current
    }
    fn receive_after(record: &mut InventoryTransferRecord, sequence: u64) {
        let current = after(record, sequence);
        receive(record, &current, None);
    }
    #[test]
    fn transfer_partial_capacity_requires_all_fresh_changed_slots_full_write_and_reply() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let mut record = fixture(version, true);
            assert_eq!(record.changed_slots.len(), 2);
            assert_eq!(
                record.changed_slots[0].prediction.value,
                stack(version, "minecraft:stone", 6)
            );
            assert_eq!(
                record.changed_slots[1].prediction.value,
                stack(version, "minecraft:stone", 64)
            );
            record.send.dispatched = true;
            receive_after(&mut record, 10);
            assert!(record.changed_slots.iter().all(|s| s.receipt.is_none()) && !record.ready());
            let mut partial = after(&record, 11);
            partial.inventory.slots[36] = record.initial.inventory.slots[36].clone();
            receive(&mut record, &partial, None);
            assert!(
                record.changed_slots[0].receipt.is_some()
                    && record.changed_slots[1].receipt.is_none()
                    && !record.ready()
            );
            receive_after(&mut record, 12);
            assert_eq!(
                record.cursor_inspected.as_ref().unwrap().source,
                record.cursor_before.source
            );
            assert_eq!(record.ready(), version == MinecraftVersion::Java1_21_11);
            if version == MinecraftVersion::Java1_16_1 {
                record.legacy_reply = Some(InventoryTransactionReply {
                    window_id: 0,
                    action: 1,
                    accepted: false,
                    receive_sequence: 13,
                });
                assert!(record.ready());
            }
        }
    }
    #[test]
    fn transfer_unsent_changes_conflicts_and_later_restoration_never_heal() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            for case in 0..9 {
                let mut record = fixture(version, false);
                record.send.dispatched = case != 0;
                let mut current = record.initial.clone();
                current.receive_sequence = 11;
                match case {
                    0 => current = after(&record, 11),
                    1 => current.session.world_generation += 1,
                    2 => current.game_mode = Some(GameMode::Creative),
                    3 => current.selected_hotbar = Some(received(1, 11)),
                    4 => {
                        current.inventory.cursor =
                            Some(received(stack(version, "minecraft:dirt", 1), 11))
                    }
                    5 => {
                        current.inventory.slots[10] =
                            Some(received(stack(version, "minecraft:dirt", 1), 11))
                    }
                    6 => current.inventory.slots[36] = None,
                    7 => current.inventory.window_id = Some(3),
                    _ => {
                        let complete = after(&record, 11);
                        receive(&mut record, &complete, None);
                        current = record.initial.clone();
                        current.inventory.slots[9].as_mut().unwrap().source =
                            ValueSource::Received { sequence: 12 };
                    }
                }
                receive(&mut record, &current, None);
                assert_eq!(
                    record.stage,
                    InventoryTransferStage::RequiresInspection,
                    "case {case}"
                );
                let reason = record.requires_inspection.clone();
                record.send.dispatched = true;
                receive_after(&mut record, 20);
                assert_eq!(record.requires_inspection, reason);
                assert!(!record.ready());
            }
        }
    }
    #[test]
    fn transfer_completed_history_remains_unchanged_after_world_mode_and_inventory_changes() {
        let mut record = fixture(MinecraftVersion::Java1_21_11, false);
        record.send.dispatched = true;
        receive_after(&mut record, 11);
        assert!(record.ready());
        record.stage = InventoryTransferStage::ObservedTransferred;
        let previous = serde_json::to_value(&record).unwrap();
        let mut current = record.initial.clone();
        current.session.world_generation += 1;
        current.game_mode = None;
        current.inventory.slots.clear();
        receive(&mut record, &current, None);
        record.inspection("disconnected");
        assert_eq!(serde_json::to_value(&record).unwrap(), previous);
    }
}
