//! One owned PLAYER_ACTION hand exchange; received evidence stays separate from dispatch.
use super::*;
use crate::client::registry::ServerRegistryObservation;

/// Identity of one hand exchange, tied to its original connection and world.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct HandSwapId {
    session: SessionStamp,
    attempt: u64,
}
impl HandSwapId {
    /// Original session; serialized history confers no operation authority.
    pub fn session(self) -> SessionStamp {
        self.session
    }
    /// Connection-local attempt number.
    pub fn attempt(self) -> u64 {
        self.attempt
    }
}

/// Submission facts, independent of the two actual slot receipts.
#[derive(Clone, Debug, serde::Serialize)]
pub struct HandSwapSend {
    /// Last applied packet before I/O.
    pub after_sequence: u64,
    /// PLAYER_ACTION sequence from the connection's shared interaction counter.
    pub interaction_sequence: i32,
    /// True only after the entire frame was written.
    pub dispatched: bool,
}

/// Retained outcome of one explicit hand exchange.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HandSwapStage {
    /// Both fresh destination receipts have not yet been confirmed. Do not resend.
    Pending,
    /// Complete frame and both fresh exact destination receipts agree.
    ObservedSwapped,
    /// First conflict or uncertain submission, retained even after restoration.
    RequiresInspection,
}

/// Historical hand exchange, readable without submitting another action.
#[derive(Clone, Debug, serde::Serialize)]
pub struct HandSwapRecord {
    /// Original attempt identity.
    pub id: HandSwapId,
    /// Mode, selection, slots and cursor captured under the same adapter lock.
    pub initial: PlayerObservation,
    /// Handle mode checked against the received game mode.
    pub mode: GameMode,
    /// Canonical selected player slot, 36..44. Offhand is always slot 45.
    pub main_slot: u16,
    /// Actual selected stack before I/O, including all item data.
    pub main_before: ObservedValue<SlotKnowledge>,
    /// Actual offhand stack before I/O, including all item data.
    pub offhand_before: ObservedValue<SlotKnowledge>,
    /// Actual cursor before I/O; hand exchange leaves it unchanged.
    pub cursor_before: ObservedValue<SlotKnowledge>,
    /// Submission facts, never an acknowledgement.
    pub send: HandSwapSend,
    /// Fresh selected-slot receipt matching the original offhand.
    pub main_receipt: Option<ObservedValue<SlotKnowledge>>,
    /// Fresh offhand receipt matching the original selected stack.
    pub offhand_receipt: Option<ObservedValue<SlotKnowledge>>,
    /// First conflict, never cleared by a later matching packet.
    pub requires_inspection: Option<String>,
    /// Outcome separate from current inventory state.
    pub stage: HandSwapStage,
    #[serde(skip)]
    context: ItemContext,
}

impl HandSwapRecord {
    pub(crate) fn unresolved(&self) -> bool {
        self.stage != HandSwapStage::ObservedSwapped
    }
    pub(crate) fn inspection(&mut self, reason: impl std::fmt::Display) {
        if self.unresolved() {
            self.requires_inspection
                .get_or_insert_with(|| reason.to_string());
            self.stage = HandSwapStage::RequiresInspection;
        }
    }
}

pub(crate) fn prepare(
    initial: PlayerObservation,
    registries: ServerRegistryObservation,
    mode: GameMode,
    attempt: u64,
    sequence: i32,
) -> Result<HandSwapRecord> {
    use super::super::container::PlayerScreenAccess;
    if !matches!(mode, GameMode::Survival | GameMode::Creative)
        || initial.game_mode != Some(mode)
        || initial.pending_dispatch
        || !matches!(initial.inventory.player_screen,
            Some(PlayerScreenAccess::Received) if initial.inventory.window_id == Some(0))
            && !matches!(initial.inventory.player_screen,
                Some(PlayerScreenAccess::SubmittedClose { close }) if close.screen().session() == initial.session)
    {
        return Err(unavailable(
            "hand swap requires matching mode, player screen and no unresolved dispatch",
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
        .ok_or_else(|| unavailable("hand swap selection unavailable"))?;
    let main_slot = 36 + u16::from(selected.value);
    let context = ItemContext::new(registries, &initial)?;
    let get =
        |value: Option<&ObservedValue<SlotKnowledge>>| -> Result<ObservedValue<SlotKnowledge>> {
            let value = value.ok_or_else(|| unavailable("hand swap predecessor unavailable"))?;
            context.predecessor(value)?;
            if let SlotKnowledge::Item { item } = &value.value {
                let definition =
                    super::super::registry::Registry::for_version(initial.session.version)
                        .item(&item.name)?;
                if definition.id != item.id
                    || item.count == 0
                    || item.count
                        > u32::try_from(item.properties()?.max_stack_size)
                            .map_err(|_| unavailable("hand swap capacity unavailable"))?
                {
                    return Err(super::super::registry::invalid("invalid hand swap stack"));
                }
            }
            Ok(value.clone())
        };
    let main_before = get(initial
        .inventory
        .slots
        .get(usize::from(main_slot))
        .and_then(Option::as_ref))?;
    let offhand_before = get(initial.inventory.slots.get(45).and_then(Option::as_ref))?;
    let cursor_before = get(initial.inventory.cursor.as_ref())?;
    if context.equivalent_values(&main_before.value, &offhand_before.value, false)? {
        return Err(super::super::registry::invalid(
            "identical hands do not require a swap",
        ));
    }
    Ok(HandSwapRecord {
        id: HandSwapId {
            session: initial.session,
            attempt,
        },
        send: HandSwapSend {
            after_sequence: initial.receive_sequence,
            interaction_sequence: sequence,
            dispatched: false,
        },
        initial,
        mode,
        main_slot,
        main_before,
        offhand_before,
        cursor_before,
        main_receipt: None,
        offhand_receipt: None,
        requires_inspection: None,
        stage: HandSwapStage::Pending,
        context,
    })
}

pub(crate) fn receive(
    record: &mut HandSwapRecord,
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
        && record.requires_inspection.is_none()
        && record.main_receipt.is_some()
        && record.offhand_receipt.is_some()
    {
        record.stage = HandSwapStage::ObservedSwapped;
    }
}
fn receive_inner(
    record: &mut HandSwapRecord,
    current: &PlayerObservation,
    registries: &ServerRegistryObservation,
) -> Result<()> {
    if !record.send.dispatched {
        return Err(unavailable(
            "hand swap submission incomplete or cancelled; do not resend",
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
            "hand swap session/mode/selection/screen context changed",
        ));
    }
    let cursor = current
        .inventory
        .cursor
        .as_ref()
        .ok_or_else(|| unavailable("hand swap cursor unavailable"))?;
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
        return Err(unavailable("hand swap cursor changed"));
    }
    for (slot, before, after, receipt) in [
        (
            record.main_slot,
            &record.main_before,
            &record.offhand_before.value,
            &mut record.main_receipt,
        ),
        (
            45,
            &record.offhand_before,
            &record.main_before.value,
            &mut record.offhand_receipt,
        ),
    ] {
        let actual = current
            .inventory
            .slots
            .get(usize::from(slot))
            .and_then(Option::as_ref)
            .ok_or_else(|| unavailable("hand swap destination unavailable"))?;
        if !received(actual) {
            return Err(unavailable("hand swap destination is not received"));
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
                    "hand swap destination conflicts with retained exchange",
                ));
            }
        }
    }
    Ok(())
}
