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
    /// Original attempt ended before any write began; it cannot write later.
    NotSent,
    /// Submission or required changed-slot receipts are incomplete.
    Pending,
    /// Full write, all exact fresh changed slots and actual legacy reply agree.
    ObservedTransferred,
    /// First conflict/uncertainty remains even if values later recover.
    RequiresInspection,
    /// The send executor's response/full-state boundary still equals the
    /// pre-send received inventory: no observed effect. Terminal.
    ObservedUnchanged,
}
/// One retained ordinary shift transfer, including partial destination capacity.
#[derive(Clone, Debug, serde::Serialize)]
pub struct InventoryTransferRecord {
    #[serde(skip)]
    item_context: Option<std::sync::Arc<super::data::ItemContext>>,
    #[serde(skip)]
    furnace_context: Option<(
        crate::client::registry::ServerRegistryStamp,
        Option<crate::client::ValueSource>,
        Option<u64>,
    )>,
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
    /// Original received cursor; ordinary QUICK_MOVE preserves it.
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
    pub(crate) fn inspect_furnace_context(
        &mut self,
        inputs: &crate::client::furnace::TransferInputs,
        owner: &crate::client::registry::ServerRegistryObservation,
    ) {
        if let Some((stamp, tags, properties)) = &self.furnace_context {
            if owner.stamp() != *stamp
                || owner.tags().map(|t| t.source) != *tags
                || inputs.sequence != *properties
            {
                self.inspection("furnace transfer recipe/tag ownership changed");
            }
        }
    }
    pub(crate) fn unresolved(&self) -> bool {
        !matches!(
            self.stage,
            InventoryTransferStage::ObservedTransferred
                | InventoryTransferStage::ObservedUnchanged
                | InventoryTransferStage::NotSent
        )
    }
    /// Terminal when the executor proved `after` produced no observed effect.
    pub(crate) fn observe_unchanged(
        &mut self,
        unchanged: Option<(crate::client::SessionStamp, u64)>,
    ) {
        if self.stage == InventoryTransferStage::Pending
            && self.send.dispatched
            && self.requires_inspection.is_none()
            && self.changed_slots.iter().all(|s| s.receipt.is_none())
            && unchanged == Some((self.initial.session, self.send.after_sequence))
        {
            self.stage = InventoryTransferStage::ObservedUnchanged;
        }
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
#[cfg(test)]
pub(crate) fn prepare(
    initial: PlayerObservation,
    mode: GameMode,
    source: InventorySource,
    slot: u16,
    attempt: u64,
    screen: Option<ContainerScreen>,
) -> Result<InventoryTransferRecord> {
    prepare_inner((initial, None, None), mode, source, slot, attempt, screen)
}
pub(crate) fn prepare_received(
    state: (
        PlayerObservation,
        crate::client::registry::ServerRegistryObservation,
        Option<&crate::client::furnace::TransferInputs>,
    ),
    mode: GameMode,
    source: InventorySource,
    slot: u16,
    attempt: u64,
    screen: Option<ContainerScreen>,
) -> Result<InventoryTransferRecord> {
    prepare_inner(
        (state.0, Some(state.1), state.2),
        mode,
        source,
        slot,
        attempt,
        screen,
    )
}
fn prepare_inner(
    state: (
        PlayerObservation,
        Option<crate::client::registry::ServerRegistryObservation>,
        Option<&crate::client::furnace::TransferInputs>,
    ),
    mode: GameMode,
    source: InventorySource,
    slot: u16,
    attempt: u64,
    screen: Option<ContainerScreen>,
) -> Result<InventoryTransferRecord> {
    let (initial, registries, furnace_inputs) = state;
    crate::client::owned_inventory::check_expected_session(initial.session)?;
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
            if !(5..=45).contains(&slot)
                && crate::client::crafting::layout::input_transfer(
                    initial.session.version,
                    "minecraft:player",
                    usize::from(slot),
                )
                .is_none()
            {
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
                .filter(|n| {
                    super::storage_menu(n)
                        || (initial.session.version == MinecraftVersion::Java1_21_11
                            && crate::client::furnace::native_menu(initial.session.version, n)
                                .is_some_and(|m| {
                                    Some(m.native_id) == s.native_menu_id
                                        && m.total_slots == layout.total_slots
                                        && m.player_mappings() == layout.player_slots
                                }))
                        || crate::client::crafting::layout::input_transfer(
                            initial.session.version,
                            n,
                            usize::from(slot),
                        )
                        .is_some_and(|route| {
                            layout.total_slots == route.total_slots
                                && crate::client::crafting::native_menu(initial.session.version, n)
                                    .is_some_and(|menu| menu.native_id == s.native_menu_id)
                        })
                })
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
    if let Some(route) = crate::client::crafting::layout::input_transfer(
        initial.session.version,
        menu,
        usize::from(slot),
    ) {
        for index in route.inputs.clone() {
            known(before.get(index).and_then(Option::as_ref))?;
        }
    }
    let furnace_route = if initial.session.version == MinecraftVersion::Java1_21_11
        && crate::client::furnace::native_menu(initial.session.version, menu).is_some()
    {
        let owner = registries
            .as_ref()
            .filter(|r| {
                r.complete()
                    && r.session() == initial.session
                    && r.receive_sequence() == initial.receive_sequence
            })
            .ok_or_else(|| {
                unavailable("coherent received furnace transfer registries unavailable")
            })?;
        // Smelting may mutate any of the three machine slots independently.
        // Require actual predecessors for all slots, then retain every conflict.
        let opened = screen
            .as_ref()
            .expect("furnace opening")
            .id
            .opened_sequence();
        for value in before.iter().chain(std::iter::once(&Some(cursor.clone()))) {
            let value = known(value.as_ref())?;
            if !matches!(value.source, ValueSource::Received { sequence } if sequence >= opened && sequence <= initial.receive_sequence)
            {
                return Err(unavailable(
                    "furnace transfer predecessor outside original capture/opening",
                ));
            }
        }
        let revision = screen.as_ref().and_then(|s| s.revision.as_ref());
        if !revision.is_some_and(|v| matches!(v.source, ValueSource::Received { sequence } if sequence >= opened && sequence <= initial.receive_sequence)) {
            return Err(unavailable("furnace transfer revision outside original capture/opening"));
        }
        Some(crate::client::furnace::transfer_route(
            menu,
            usize::from(slot),
            &values[usize::from(slot)],
            furnace_inputs,
            Some(owner),
        )?)
    } else {
        None
    };
    let has_data = values.iter().chain(std::iter::once(&cursor.value)).any(|v| matches!(v, SlotKnowledge::Item {item} if item.data != crate::client::ItemData::Default));
    let item_context = if let (true, Some(registries)) = (has_data, registries.clone()) {
        let context = super::data::ItemContext::new(registries, &initial)?;
        for value in before
            .iter()
            .flatten()
            .filter(|v| v.value != SlotKnowledge::Unavailable)
        {
            context.predecessor(value)?;
        }
        context.predecessor(&cursor)?;
        Some(std::sync::Arc::new(context))
    } else {
        None
    };
    transfer_policy::validate(
        initial.session.version,
        &cursor.value,
        item_context.as_deref(),
    )?;
    let calculated = if let Some(route) = &furnace_route {
        transfer_policy::calculate_furnace(
            initial.session.version,
            menu,
            usize::from(slot),
            &values,
            mode,
            item_context.as_deref(),
            route,
        )?
    } else if let Some(context) = &item_context {
        transfer_policy::calculate_with_data(
            initial.session.version,
            menu,
            usize::from(slot),
            &values,
            mode,
            context,
        )?
    } else {
        transfer_policy::calculate(initial.session.version, menu, usize::from(slot), &values)?
    };
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
    let furnace_context = furnace_route.map(|_| {
        let owner = registries
            .as_ref()
            .expect("furnace received registry capture");
        (
            owner.stamp(),
            owner.tags().map(|t| t.source),
            furnace_inputs.and_then(|i| i.sequence),
        )
    });
    Ok(InventoryTransferRecord {
        furnace_context,
        item_context,
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
pub(crate) fn validate_predecessors(
    record: &InventoryTransferRecord,
    current: &PlayerObservation,
    screen: Option<&ContainerScreen>,
    registries: &crate::client::registry::ServerRegistryObservation,
) -> Result<()> {
    let slots = source_slots(record, current, screen)?;
    if registries.session() != current.session
        || registries.receive_sequence() != current.receive_sequence
    {
        return Err(unavailable(
            "coherent transfer registries unavailable before I/O",
        ));
    }
    for (index, before) in record.before_slots.iter().enumerate() {
        let Some(before) = before
            .as_ref()
            .filter(|v| v.value != SlotKnowledge::Unavailable)
        else {
            continue;
        };
        let actual = known(slots.get(index).and_then(Option::as_ref))?;
        let unchanged = if let Some(context) = &record.item_context {
            context
                .classify(&actual, before, &before.value, registries)?
                .0
        } else {
            super::same_received_value(Some(&actual), before)
        };
        if !unchanged {
            return Err(unavailable("transfer predecessor changed before I/O"));
        }
    }
    let cursor = known(current.inventory.cursor.as_ref())?;
    let unchanged = if let Some(context) = &record.item_context {
        context
            .classify(
                &cursor,
                &record.cursor_before,
                &record.cursor_before.value,
                registries,
            )?
            .0
    } else {
        super::same_received_value(Some(&cursor), &record.cursor_before)
    };
    if !unchanged {
        return Err(unavailable("transfer cursor changed before I/O"));
    }
    Ok(())
}
#[cfg(test)]
pub(crate) fn receive(
    record: &mut InventoryTransferRecord,
    current: &PlayerObservation,
    screen: Option<&ContainerScreen>,
) {
    receive_inner(record, current, screen, None)
}
pub(crate) fn receive_with_registries(
    record: &mut InventoryTransferRecord,
    current: &PlayerObservation,
    screen: Option<&ContainerScreen>,
    registries: &crate::client::registry::ServerRegistryObservation,
) {
    receive_inner(record, current, screen, Some(registries))
}
fn receive_inner(
    record: &mut InventoryTransferRecord,
    current: &PlayerObservation,
    screen: Option<&ContainerScreen>,
    registries: Option<&crate::client::registry::ServerRegistryObservation>,
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
    let inspected = known(current.inventory.cursor.as_ref()).and_then(|cursor| {
        let unchanged = if let Some(context) = &record.item_context {
            let owner = registries
                .filter(|r| {
                    r.session() == current.session
                        && r.receive_sequence() == current.receive_sequence
                })
                .ok_or_else(|| unavailable("coherent transfer cursor registries unavailable"))?;
            context
                .classify(
                    &cursor,
                    &record.cursor_before,
                    &record.cursor_before.value,
                    owner,
                )?
                .0
        } else {
            cursor.value == record.cursor_before.value
        };
        if !unchanged {
            return Err(unavailable("transfer cursor changed"));
        }
        Ok(cursor)
    });
    match inspected {
        Ok(cursor) => record.cursor_inspected = Some(cursor),
        Err(e) => {
            record.inspection(e);
            return;
        }
    }
    let menu = record
        .initial_screen
        .as_ref()
        .and_then(|s| s.menu_name.as_deref())
        .unwrap_or("minecraft:player");
    let input = crate::client::crafting::layout::input_transfer(
        record.initial.session.version,
        menu,
        usize::from(record.source_slot),
    );
    for (index, before) in record.before_slots.iter().enumerate() {
        // Native input callbacks recompute the result independently. It is neither
        // an input conflict nor evidence that any source/destination was received.
        if input.is_some_and(|route| index == route.result_slot) {
            continue;
        }
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
        let change = record
            .changed_slots
            .iter_mut()
            .find(|s| usize::from(s.slot) == index);
        let after = change
            .as_ref()
            .map_or(&before.value, |s| &s.prediction.value);
        let classification = actual
            .ok_or_else(|| unavailable("transfer actual slot unavailable"))
            .and_then(|actual| {
                if let Some(context) = &record.item_context {
                    registries
                        .filter(|r| {
                            r.session() == current.session
                                && r.receive_sequence() == current.receive_sequence
                        })
                        .ok_or_else(|| {
                            unavailable("coherent transfer registry capture unavailable")
                        })
                        .and_then(|r| context.classify(actual, before, after, r))
                } else {
                    Ok((actual.value == before.value, &actual.value == after))
                }
            });
        let (matches_before, matches_after) = match classification {
            Ok(v) if v.0 || v.1 => v,
            Ok(_) => {
                record
                    .requires_inspection
                    .get_or_insert_with(|| "transfer native item fields conflict".into());
                continue;
            }
            Err(e) => {
                record
                    .requires_inspection
                    .get_or_insert_with(|| e.to_string());
                continue;
            }
        };
        if let Some(change) = change {
            let actual = actual.expect("classified received slot");
            let ValueSource::Received { sequence } = actual.source else {
                unreachable!()
            };
            if sequence > record.send.after_sequence {
                if !record.send.dispatched && !matches_before {
                    record
                        .requires_inspection
                        .get_or_insert_with(|| "transfer changed before full dispatch".into());
                } else if matches_after {
                    change.receipt.get_or_insert_with(|| actual.clone());
                } else if change.receipt.is_some() {
                    record.requires_inspection.get_or_insert_with(|| {
                        "transfer destination restored before confirmation".into()
                    });
                }
            }
        } else if !matches_before {
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
            using_item: None,
            fall_flying: None,
            entity_id: None,
            attributes: Default::default(),
            effects: Default::default(),
            air_supply: None,
            world_time: None,
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
            rotation_source: None,
            on_ground: None,
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
    fn held_cursor_transfer_accepts_native_normalization_and_latches_item_or_registry_change() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            for mode in [GameMode::Survival, GameMode::Creative] {
                let mut initial = fixture(version, false).initial;
                initial.game_mode = Some(mode);
                initial.inventory.cursor = Some(received(
                    super::super::held_cursor_tests::data_cursor(version, false),
                    10,
                ));
                let mut registry = crate::client::registry::received::ReceivedRegistries::default();
                registry.reset(1);
                super::super::armor_tests::install(&mut registry, version, None);
                let owner = registry.capture(initial.session, initial.receive_sequence);
                let mut record = prepare_received(
                    (initial, owner, None),
                    mode,
                    InventorySource::Player,
                    9,
                    1,
                    None,
                )
                .unwrap();
                let mut current = record.initial.clone();
                current.receive_sequence = 11;
                current.inventory.cursor = Some(received(
                    super::super::held_cursor_tests::data_cursor(version, true),
                    11,
                ));
                validate_predecessors(
                    &record,
                    &current,
                    None,
                    &registry.capture(current.session, 11),
                )
                .unwrap();
                record.send.dispatched = true;
                for change in &record.changed_slots {
                    current.inventory.slots[usize::from(change.slot)] =
                        Some(received(change.prediction.value.clone(), 12));
                }
                current.receive_sequence = 12;
                receive_with_registries(
                    &mut record,
                    &current,
                    None,
                    &registry.capture(current.session, 12),
                );
                assert!(record.requires_inspection.is_none());
                assert!(record.cursor_inspected.is_some());
                assert!(record.changed_slots.iter().all(|s| s.receipt.is_some()));
                assert_eq!(record.ready(), version == MinecraftVersion::Java1_21_11);
                let mut config_conflict = record.clone();
                registry.reset(2);
                registry.finish();
                receive_with_registries(
                    &mut config_conflict,
                    &current,
                    None,
                    &registry.capture(current.session, 12),
                );
                assert_eq!(
                    config_conflict.stage,
                    InventoryTransferStage::RequiresInspection
                );
                registry.reset(1);
                super::super::armor_tests::install(&mut registry, version, None);
                let SlotKnowledge::Item { item } =
                    &mut current.inventory.cursor.as_mut().unwrap().value
                else {
                    unreachable!()
                };
                item.count += 1;
                current.receive_sequence = 13;
                current.inventory.cursor.as_mut().unwrap().source =
                    ValueSource::Received { sequence: 13 };
                assert!(
                    validate_predecessors(
                        &record,
                        &current,
                        None,
                        &registry.capture(current.session, 13)
                    )
                    .is_err()
                );
                receive_with_registries(
                    &mut record,
                    &current,
                    None,
                    &registry.capture(current.session, 13),
                );
                assert_eq!(record.stage, InventoryTransferStage::RequiresInspection);
                assert!(!record.ready());
                current.receive_sequence = 14;
                current.inventory.cursor = Some(received(
                    super::super::held_cursor_tests::data_cursor(version, false),
                    14,
                ));
                receive_with_registries(
                    &mut record,
                    &current,
                    None,
                    &registry.capture(current.session, 14),
                );
                assert_eq!(record.stage, InventoryTransferStage::RequiresInspection);
            }
        }
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
    fn input_fixture(
        version: MinecraftVersion,
        table: bool,
    ) -> (PlayerObservation, Option<ContainerScreen>) {
        let mut initial = fixture(version, false).initial;
        initial.inventory.slots[9] = Some(received(SlotKnowledge::Empty, 10));
        initial.inventory.cursor = Some(received(
            super::super::held_cursor_tests::data_cursor(version, false),
            10,
        ));
        let source = super::super::held_cursor_tests::data_cursor(version, false);
        let destination = super::super::held_cursor_tests::data_cursor(version, true);
        let mut source = source;
        let mut destination = destination;
        if let SlotKnowledge::Item { item } = &mut source {
            item.count = 7;
        }
        if let SlotKnowledge::Item { item } = &mut destination {
            item.count = 63;
        }
        let mut slots = vec![Some(received(SlotKnowledge::Empty, 10)); 46];
        slots[1] = Some(received(source, 10));
        let start = if table { 10 } else { 9 };
        slots[start] = Some(received(destination, 10));
        slots[start + 1] = Some(received(stack(version, "minecraft:stone", 64), 10));
        for value in &mut slots[start + 2..start + 36] {
            *value = Some(received(stack(version, "minecraft:dirt", 64), 10));
        }
        if !table {
            initial.inventory.slots = slots;
            return (initial, None);
        }
        let native = crate::client::crafting::native_menu(version, "minecraft:crafting").unwrap();
        let mut receipts = crate::client::container::ScreenReceipts::open(
            version,
            3,
            native.native_id,
            crate::client::container::ScreenTitle::Unavailable,
            8,
        );
        receipts.full_items(slots, Some(7), 10).unwrap();
        let screen = receipts.capture(initial.session);
        for mapping in &screen.layout.as_ref().unwrap().player_slots {
            initial.inventory.slots[mapping.player_slot] =
                screen.slots[mapping.screen_slot].clone();
        }
        initial.inventory.window_id = Some(3);
        initial.inventory.player_screen = None;
        (initial, Some(screen))
    }
    #[test]
    fn crafting_input_transfer_data_partial_requires_source_and_all_destinations_not_result() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            for table in [false, true] {
                for mode in [GameMode::Survival, GameMode::Creative] {
                    let (mut initial, screen) = input_fixture(version, table);
                    initial.game_mode = Some(mode);
                    let mut registries =
                        crate::client::registry::received::ReceivedRegistries::default();
                    registries.reset(1);
                    super::super::armor_tests::install(&mut registries, version, None);
                    let owner = registries.capture(initial.session, 10);
                    let source = screen.as_ref().map_or(InventorySource::Player, |s| {
                        InventorySource::Container { screen: s.id }
                    });
                    let mut record =
                        prepare_received((initial, owner, None), mode, source, 1, 1, screen)
                            .unwrap();
                    // The plain stone stack cannot absorb data-bearing stone.
                    assert_eq!(record.changed_slots.len(), 2);
                    let mut current = record.initial.clone();
                    let mut screen = record.initial_screen.clone();
                    record.send.dispatched = true;
                    let update = |current: &mut PlayerObservation,
                                  screen: &mut Option<ContainerScreen>,
                                  index: usize,
                                  value: SlotKnowledge,
                                  sequence| {
                        current.receive_sequence = sequence;
                        if let Some(s) = screen {
                            s.slots[index] = Some(received(value, sequence));
                            if let Some(mapping) = s
                                .layout
                                .as_ref()
                                .unwrap()
                                .player_slots
                                .iter()
                                .find(|m| m.screen_slot == index)
                            {
                                current.inventory.slots[mapping.player_slot] =
                                    s.slots[index].clone();
                            }
                        } else {
                            current.inventory.slots[index] = Some(received(value, sequence));
                        }
                    };
                    // Recipe output is unrelated to input completion and may change repeatedly.
                    update(
                        &mut current,
                        &mut screen,
                        0,
                        stack(version, "minecraft:oak_button", 1),
                        11,
                    );
                    receive_with_registries(
                        &mut record,
                        &current,
                        screen.as_ref(),
                        &registries.capture(current.session, 11),
                    );
                    assert!(record.requires_inspection.is_none());
                    assert!(!record.ready());
                    assert!(record.changed_slots.iter().all(|s| s.receipt.is_none()));
                    let dest = record.changed_slots[1].clone();
                    update(
                        &mut current,
                        &mut screen,
                        usize::from(dest.slot),
                        dest.prediction.value,
                        12,
                    );
                    receive_with_registries(
                        &mut record,
                        &current,
                        screen.as_ref(),
                        &registries.capture(current.session, 12),
                    );
                    assert!(!record.ready());
                    assert!(record.changed_slots[0].receipt.is_none());
                    let source = record.changed_slots[0].clone();
                    assert!(
                        matches!(&source.prediction.value, SlotKnowledge::Item {item} if item.count == 6)
                    );
                    update(&mut current, &mut screen, 1, source.prediction.value, 13);
                    receive_with_registries(
                        &mut record,
                        &current,
                        screen.as_ref(),
                        &registries.capture(current.session, 13),
                    );
                    if version == MinecraftVersion::Java1_16_1 {
                        record.legacy_reply = Some(InventoryTransactionReply {
                            window_id: record.window_id() as i8,
                            action: 1,
                            accepted: false,
                            receive_sequence: 14,
                        });
                    }
                    assert!(
                        record.ready(),
                        "{version:?} table={table}: {:?}",
                        record.requires_inspection
                    );
                    assert_eq!(
                        record.cursor_inspected.as_ref().unwrap().value,
                        record.cursor_before.value
                    );
                }
            }
        }
    }
    #[test]
    fn crafting_input_transfer_missing_grid_competing_input_and_old_opening_are_rejected_or_latched()
     {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            for table in [false, true] {
                let (mut initial, mut screen) = input_fixture(version, table);
                // Default data keeps this ownership test independent of registry resolution.
                initial.inventory.cursor = Some(received(SlotKnowledge::Empty, 10));
                let values = if let Some(s) = &mut screen {
                    &mut s.slots
                } else {
                    &mut initial.inventory.slots
                };
                values[1] = Some(received(stack(version, "minecraft:stone", 7), 10));
                let start = if table { 10 } else { 9 };
                values[start] = Some(received(stack(version, "minecraft:stone", 62), 10));
                values[start + 1] = Some(received(stack(version, "minecraft:stone", 63), 10));
                if let Some(s) = &screen {
                    for m in &s.layout.as_ref().unwrap().player_slots {
                        initial.inventory.slots[m.player_slot] = s.slots[m.screen_slot].clone();
                    }
                }
                let source = screen.as_ref().map_or(InventorySource::Player, |s| {
                    InventorySource::Container { screen: s.id }
                });
                let mut record = prepare(
                    initial.clone(),
                    GameMode::Survival,
                    source,
                    1,
                    1,
                    screen.clone(),
                )
                .unwrap();
                let mut missing_grid = initial.clone();
                let mut missing_screen = screen.clone();
                if let Some(s) = &mut missing_screen {
                    s.slots[2] = None;
                } else {
                    missing_grid.inventory.slots[2] = None;
                }
                assert!(
                    prepare(
                        missing_grid,
                        GameMode::Survival,
                        source,
                        1,
                        2,
                        missing_screen
                    )
                    .is_err()
                );

                assert_eq!(record.changed_slots.len(), 3);
                record.send.dispatched = true;
                let mut current = initial.clone();
                current.receive_sequence = 11;
                if let Some(s) = &mut screen {
                    s.slots[2] = Some(received(stack(version, "minecraft:dirt", 1), 11));
                } else {
                    current.inventory.slots[2] =
                        Some(received(stack(version, "minecraft:dirt", 1), 11));
                }
                receive(&mut record, &current, screen.as_ref());
                assert_eq!(record.stage, InventoryTransferStage::RequiresInspection);
                let restored_screen = record.initial_screen.clone();
                receive(&mut record, &initial, restored_screen.as_ref());
                assert!(!record.ready());
                // A missing destination cannot be guessed empty or admitted.
                if let Some(s) = &mut screen {
                    s.slots[start] = None;
                } else {
                    initial.inventory.slots[start] = None;
                }
                assert!(
                    prepare(
                        initial.clone(),
                        GameMode::Survival,
                        source,
                        1,
                        2,
                        screen.clone()
                    )
                    .is_err()
                );
                if let Some(mut s) = record.initial_screen.clone() {
                    // Same numeric window with a newer opening cannot use the old source handle.
                    let mut receipts = crate::client::container::ScreenReceipts::open(
                        version,
                        3,
                        s.native_menu_id,
                        crate::client::container::ScreenTitle::Unavailable,
                        20,
                    );
                    receipts.full_items(s.slots.clone(), Some(8), 21).unwrap();
                    s = receipts.capture(initial.session);
                    assert!(
                        prepare(
                            record.initial.clone(),
                            GameMode::Survival,
                            source,
                            1,
                            2,
                            Some(s)
                        )
                        .is_err()
                    );
                    assert!(
                        prepare(
                            record.initial.clone(),
                            GameMode::Survival,
                            source,
                            1,
                            2,
                            None
                        )
                        .is_err()
                    );
                }
            }
        }
    }
}
