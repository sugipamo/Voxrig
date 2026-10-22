//! One owned PLAYER_ACTION selected-stack drop; received evidence stays separate from dispatch.
use super::*;
use crate::client::registry::ServerRegistryObservation;

/// Amount requested by one PLAYER_ACTION. No repeated per-item sends.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SelectedDropAmount {
    /// Drop one item, including the last item in a stack.
    One,
    /// Drop the entire stack captured before submission.
    All,
}

/// Identity of one selected-stack drop, tied to its original connection and world.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct SelectedDropId {
    session: SessionStamp,
    attempt: u64,
}
impl SelectedDropId {
    /// Original session; serialized history confers no operation authority.
    pub fn session(self) -> SessionStamp {
        self.session
    }
    /// Connection-local attempt number.
    pub fn attempt(self) -> u64 {
        self.attempt
    }
}

/// Submission facts, independent of the actual selected-slot receipt.
#[derive(Clone, Debug, serde::Serialize)]
pub struct SelectedDropSend {
    /// Last applied packet before I/O.
    pub after_sequence: u64,
    /// PLAYER_ACTION sequence from the connection's shared interaction counter.
    pub interaction_sequence: i32,
    /// True only after the entire drop frame was written.
    pub dispatched: bool,
    /// Full no-op resync request written after the drop frame. Not a received result.
    /// Java 1.21.11 uses player screen 0, PICKUP slot -1, revision -1.
    pub resync_dispatched: bool,
}

/// Retained outcome of one explicit selected-stack drop.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SelectedDropStage {
    /// A fresh selected-slot decrease has not yet been confirmed. Do not resend.
    Pending,
    /// Complete drop/resync frames and a fresh exact decrease agree; actor is unproven.
    ObservedDecreased,
    /// First conflict or uncertain submission, retained even after restoration.
    RequiresInspection,
}

/// Historical selected-stack drop, readable without submitting another action.
#[derive(Clone, Debug, serde::Serialize)]
pub struct SelectedDropRecord {
    /// Original attempt identity.
    pub id: SelectedDropId,
    /// Mode, selection, slots and cursor captured under the same adapter lock.
    pub initial: PlayerObservation,
    /// Handle mode checked against the received game mode.
    pub mode: GameMode,
    /// Canonical selected player slot, 36..44.
    pub main_slot: u16,
    /// Actual selected stack before I/O, including all item data.
    pub main_before: ObservedValue<SlotKnowledge>,
    /// One item or the whole original stack, fixed before I/O.
    pub amount: SelectedDropAmount,
    /// Expected decrease, not a received value.
    pub expected_after: SlotKnowledge,
    /// Actual cursor before I/O; drop leaves it unchanged.
    pub cursor_before: ObservedValue<SlotKnowledge>,
    /// Submission facts, never an acknowledgement.
    pub send: SelectedDropSend,
    /// Fresh selected-slot receipt matching the exact expected decrease.
    pub main_receipt: Option<ObservedValue<SlotKnowledge>>,
    /// First conflict, never cleared by a later matching packet.
    pub requires_inspection: Option<String>,
    /// Outcome separate from current inventory state.
    pub stage: SelectedDropStage,
    #[serde(skip)]
    context: ItemContext,
}

impl SelectedDropRecord {
    pub(crate) fn unresolved(&self) -> bool {
        self.stage != SelectedDropStage::ObservedDecreased
    }
    pub(crate) fn inspection(&mut self, reason: impl std::fmt::Display) {
        if self.unresolved() {
            self.requires_inspection
                .get_or_insert_with(|| reason.to_string());
            self.stage = SelectedDropStage::RequiresInspection;
        }
    }
}

pub(crate) fn prepare(
    initial: PlayerObservation,
    registries: ServerRegistryObservation,
    mode: GameMode,
    attempt: u64,
    sequence: i32,
    amount: SelectedDropAmount,
) -> Result<SelectedDropRecord> {
    use super::super::container::PlayerScreenAccess;
    if !matches!(mode, GameMode::Survival | GameMode::Creative)
        || initial.game_mode != Some(mode)
        || initial.pending_dispatch
        || initial
            .using_item
            .as_ref()
            .is_some_and(|use_state| use_state.value.is_some())
        || !matches!(initial.inventory.player_screen,
            Some(PlayerScreenAccess::Received) if initial.inventory.window_id == Some(0))
            && !matches!(initial.inventory.player_screen,
                Some(PlayerScreenAccess::SubmittedClose { close }) if close.screen().session() == initial.session)
    {
        return Err(unavailable(
            "selected drop requires matching mode, player screen and no unresolved dispatch",
        ));
    }
    let selected = initial
        .selected_hotbar
        .as_ref()
        .filter(|s| {
            s.value <= 8
                && (matches!(s.source, ValueSource::Received { sequence } if sequence <= initial.receive_sequence)
                    || s.source == ValueSource::Submitted)
        })
        .ok_or_else(|| unavailable("selected drop selection unavailable"))?;
    let main_slot = 36 + u16::from(selected.value);
    let context = ItemContext::new(registries, &initial)?;
    let get =
        |value: Option<&ObservedValue<SlotKnowledge>>| -> Result<ObservedValue<SlotKnowledge>> {
            let value =
                value.ok_or_else(|| unavailable("selected drop predecessor unavailable"))?;
            context.predecessor(value)?;
            if let SlotKnowledge::Item { item } = &value.value {
                let definition =
                    super::super::registry::Registry::for_version(initial.session.version)
                        .item(&item.name)?;
                if definition.id != item.id
                    || item.count == 0
                    || item.count
                        > u32::try_from(item.properties()?.max_stack_size)
                            .map_err(|_| unavailable("selected drop capacity unavailable"))?
                {
                    return Err(super::super::registry::invalid(
                        "invalid selected drop stack",
                    ));
                }
            }
            Ok(value.clone())
        };
    let main_before = get(initial
        .inventory
        .slots
        .get(usize::from(main_slot))
        .and_then(Option::as_ref))?;
    let cursor_before = get(initial.inventory.cursor.as_ref())?;
    let SlotKnowledge::Item { item } = &main_before.value else {
        return Err(super::super::registry::invalid(
            "selected drop requires a nonempty stack",
        ));
    };
    let expected_after = match amount {
        SelectedDropAmount::One if item.count > 1 => {
            let mut remaining = item.clone();
            remaining.count -= 1;
            SlotKnowledge::Item { item: remaining }
        }
        _ => SlotKnowledge::Empty,
    };
    Ok(SelectedDropRecord {
        id: SelectedDropId {
            session: initial.session,
            attempt,
        },
        send: SelectedDropSend {
            after_sequence: initial.receive_sequence,
            interaction_sequence: sequence,
            dispatched: false,
            resync_dispatched: false,
        },
        initial,
        mode,
        main_slot,
        main_before,
        amount,
        expected_after,
        cursor_before,
        main_receipt: None,
        requires_inspection: None,
        stage: SelectedDropStage::Pending,
        context,
    })
}

pub(crate) fn receive(
    record: &mut SelectedDropRecord,
    current: &PlayerObservation,
    registries: &ServerRegistryObservation,
) {
    if !record.unresolved() {
        return;
    }
    let result = receive_inner(record, current, registries);
    if let Err(error) = result {
        record.inspection(error);
    }
    if record.send.dispatched
        && record.send.resync_dispatched
        && record.requires_inspection.is_none()
        && record.main_receipt.is_some()
    {
        record.stage = SelectedDropStage::ObservedDecreased;
    }
}
fn receive_inner(
    record: &mut SelectedDropRecord,
    current: &PlayerObservation,
    registries: &ServerRegistryObservation,
) -> Result<()> {
    if !record.send.dispatched || !record.send.resync_dispatched {
        return Err(unavailable(
            "selected drop submission incomplete or cancelled; do not resend",
        ));
    }
    use super::super::container::PlayerScreenAccess;
    let player_screen = current.inventory.player_screen == Some(PlayerScreenAccess::Received)
        && current.inventory.window_id == Some(0)
        || matches!((record.initial.inventory.player_screen, current.inventory.player_screen),
            (Some(PlayerScreenAccess::SubmittedClose { close: before }), Some(PlayerScreenAccess::SubmittedClose { close: now }))
                if before == now && current.inventory.window_id == record.initial.inventory.window_id);
    if current.session != record.initial.session
        || current.game_mode != Some(record.mode)
        || current.selected_hotbar.as_ref().map(|s| s.value)
            != record.initial.selected_hotbar.as_ref().map(|s| s.value)
        || !player_screen
        || registries.session() != current.session
        || registries.receive_sequence() != current.receive_sequence
    {
        return Err(unavailable(
            "selected drop session/mode/selection/screen context changed",
        ));
    }
    if current
        .using_item
        .as_ref()
        .is_some_and(|use_state| use_state.value.is_some())
    {
        return Err(unavailable("selected drop overlaps received item use"));
    }
    let cursor = current
        .inventory
        .cursor
        .as_ref()
        .ok_or_else(|| unavailable("selected drop cursor unavailable"))?;
    let received = |value: &ObservedValue<SlotKnowledge>| matches!(value.source, ValueSource::Received { sequence } if sequence <= current.receive_sequence);
    if !received(cursor)
        || !record
            .context
            .classify(
                cursor,
                &record.cursor_before,
                &record.cursor_before.value,
                registries,
            )?
            .0
    {
        return Err(unavailable("selected drop cursor changed"));
    }
    {
        let slot = record.main_slot;
        let before = &record.main_before;
        let after = &record.expected_after;
        let receipt = &mut record.main_receipt;
        let actual = current
            .inventory
            .slots
            .get(usize::from(slot))
            .and_then(Option::as_ref)
            .ok_or_else(|| unavailable("selected drop destination unavailable"))?;
        if !received(actual) {
            return Err(unavailable("selected drop destination is not received"));
        }
        let (was_before, is_after) = record.context.classify(actual, before, after, registries)?;
        if let ValueSource::Received { sequence } = actual.source {
            if sequence > record.send.after_sequence && is_after {
                receipt.get_or_insert_with(|| actual.clone());
            } else if !was_before
                || receipt.is_some()
                || actual.value != before.value && sequence <= record.send.after_sequence
            {
                return Err(unavailable(
                    "selected drop destination conflicts with retained drop",
                ));
            }
        }
    }
    Ok(())
}
