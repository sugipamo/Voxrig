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
    /// Complete swap/resync frames, then one fresh full player-contents boundary
    /// (every slot and the cursor at one receive ordinal after the send) still shows
    /// both hands and the cursor equal to their pre-send values. The server processed
    /// the exchange without effect. Terminal; releases the inventory owner.
    ObservedUnchanged,
    /// Still `Pending` at the bounded response deadline. The outcome is unknown and
    /// the server may still apply it; this is not a refusal. Terminal: the connection
    /// ledger fails with `TransactionExpired`, so later work needs a new connection.
    Expired,
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
    /// Full no-op resync request written after the swap frame. Not a received result.
    /// Java 1.21.11 uses player screen 0, PICKUP slot -1, revision -1, as selected drop.
    pub resync_dispatched: bool,
    /// Fresh selected-slot receipt matching the original offhand.
    pub main_receipt: Option<ObservedValue<SlotKnowledge>>,
    /// Fresh offhand receipt matching the original selected stack.
    pub offhand_receipt: Option<ObservedValue<SlotKnowledge>>,
    /// First conflict, never cleared by a later matching packet.
    pub requires_inspection: Option<String>,
    /// Receive ordinal of the full player contents that showed no effect.
    /// Set only together with `ObservedUnchanged`.
    pub unchanged_boundary: Option<u64>,
    /// Outcome separate from current inventory state.
    pub stage: HandSwapStage,
    #[serde(skip)]
    context: ItemContext,
    /// Absolute response deadline, set when the record is retained for I/O.
    #[serde(skip)]
    pub(crate) deadline: Option<tokio::time::Instant>,
}

impl HandSwapRecord {
    /// Blocks other inventory owners. `Expired` keeps blocking; its connection is failed.
    pub(crate) fn unresolved(&self) -> bool {
        !matches!(
            self.stage,
            HandSwapStage::ObservedSwapped | HandSwapStage::ObservedUnchanged
        )
    }
    fn open(&self) -> bool {
        matches!(
            self.stage,
            HandSwapStage::Pending | HandSwapStage::RequiresInspection
        )
    }
    /// `Pending` past its deadline becomes `Expired` before any later evidence is read.
    pub(crate) fn expire_due(&mut self) {
        if self.stage == HandSwapStage::Pending
            && self
                .deadline
                .is_some_and(|deadline| tokio::time::Instant::now() >= deadline)
        {
            self.stage = HandSwapStage::Expired;
        }
    }
    pub(crate) fn inspection(&mut self, reason: impl std::fmt::Display) {
        self.expire_due();
        if self.open() {
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
        resync_dispatched: false,
        initial,
        mode,
        main_slot,
        main_before,
        offhand_before,
        cursor_before,
        main_receipt: None,
        offhand_receipt: None,
        requires_inspection: None,
        unchanged_boundary: None,
        stage: HandSwapStage::Pending,
        context,
        deadline: None,
    })
}

pub(crate) fn receive(
    record: &mut HandSwapRecord,
    current: &PlayerObservation,
    registries: &ServerRegistryObservation,
) {
    record.expire_due();
    if !record.open() {
        return;
    }
    match receive_inner(record, current, registries) {
        Err(error) => record.inspection(error),
        Ok(Some(boundary)) if record.requires_inspection.is_none() => {
            record.unchanged_boundary = Some(boundary);
            record.stage = HandSwapStage::ObservedUnchanged;
            return;
        }
        Ok(_) => {}
    }
    if record.send.dispatched
        && record.resync_dispatched
        && record.requires_inspection.is_none()
        && record.main_receipt.is_some()
        && record.offhand_receipt.is_some()
    {
        record.stage = HandSwapStage::ObservedSwapped;
    }
}
/// One full player-contents packet after `after`: every slot and the cursor share
/// its receive ordinal. A single-slot update or an older capture is never one.
pub(crate) fn full_boundary(current: &PlayerObservation, after: u64) -> Option<u64> {
    let inventory = &current.inventory;
    let Some(ValueSource::Received { sequence }) = inventory
        .slots
        .first()
        .and_then(Option::as_ref)
        .map(|value| value.source)
    else {
        return None;
    };
    let at = |value: Option<&ObservedValue<SlotKnowledge>>| {
        value.is_some_and(|value| value.source == ValueSource::Received { sequence })
    };
    (sequence > after
        && sequence <= current.receive_sequence
        && inventory.slots.len() == 46
        && inventory.slots.iter().all(|slot| at(slot.as_ref()))
        && at(inventory.cursor.as_ref()))
    .then_some(sequence)
}

/// `Some(boundary)` when the resync's full contents show the exchange had no effect.
fn receive_inner(
    record: &mut HandSwapRecord,
    current: &PlayerObservation,
    registries: &ServerRegistryObservation,
) -> Result<Option<u64>> {
    if !record.send.dispatched || !record.resync_dispatched {
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
    let full = full_boundary(current, record.send.after_sequence);
    let mut unchanged = 0;
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
            if receipt.is_none() && was_before && full == Some(sequence) {
                unchanged += 1;
            }
        }
    }
    // Vanilla handles PLAYER_ACTION before the following resync click, so its
    // full contents are the processed boundary: both hands or neither.
    match (full, unchanged) {
        (Some(boundary), 2) => Ok(Some(boundary)),
        (Some(_), 1) => Err(unavailable(
            "hand swap full contents show only one exchanged hand",
        )),
        _ => Ok(None),
    }
}
