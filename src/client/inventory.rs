//! Received player-inventory exchanges; container/crafting rules remain separate.
use super::{
    GameMode, ItemData, ObservedValue, PlayerObservation, SessionStamp, SlotKnowledge, ValueSource,
};
use crate::Result;

/// Connection/world-bound attempt identity; serialization supplies diagnostics only.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct InventorySwapId {
    session: SessionStamp,
    attempt: u64,
}
impl InventorySwapId {
    pub(crate) fn new(session: SessionStamp, attempt: u64) -> Self {
        Self { session, attempt }
    }
    /// Original connection/world, not a current permission.
    pub fn session(self) -> SessionStamp {
        self.session
    }
    /// Adapter-local attempt number, not a server tick.
    pub fn attempt(self) -> u64 {
        self.attempt
    }
}
/// One before-I/O native click, with version-specific synchronization fields.
#[derive(Clone, Debug, serde::Serialize)]
pub struct InventorySwapSend {
    /// Applied receive boundary immediately before submission.
    pub after_sequence: u64,
    /// Legacy click transaction number; absent on modern.
    pub legacy_action: Option<i16>,
    /// Legacy comparison stack selected from a received nonempty predecessor
    /// to request native full resync; not a click result or a received outcome.
    pub legacy_comparison: Option<super::ItemStack>,
    /// Received modern screen revision; absent on legacy. Not an execution lock.
    pub screen_revision: Option<i32>,
    /// Complete command frame was written; not server acceptance.
    pub dispatched: bool,
}
/// Actual legacy response; accepted alone does not establish both destinations.
#[derive(Clone, Debug, serde::Serialize)]
pub struct InventoryTransactionReply {
    /// Native player screen identifier.
    pub window_id: i8,
    /// Original click number.
    pub action: i16,
    /// Native returned-stack comparison. False can follow an applied click
    /// and native resync; it is not rollback or gameplay rejection.
    pub accepted: bool,
    /// Actual response packet ordinal.
    pub receive_sequence: u64,
}
/// Historical outcome of one explicit exchange.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InventorySwapStage {
    /// Required fresh receipts have not arrived. Do not repeat the click.
    Pending,
    /// Complete submission and both exact fresh destination receipts agree.
    ObservedSwapped,
    /// First conflict or uncertain delivery retained even after restoration.
    RequiresInspection,
}
/// Retained exchange diagnostics, also readable after closure.
#[derive(Clone, Debug, serde::Serialize)]
pub struct InventorySwapRecord {
    /// Opaque immutable attempt identity.
    pub id: InventorySwapId,
    /// Coherent received baseline captured before possible I/O.
    pub initial: PlayerObservation,
    /// Matching received mode required by the operations handle.
    pub mode: GameMode,
    /// Main-inventory player-screen slot, 9..35.
    pub main_slot: u8,
    /// Hotbar index, 0..8; corresponding screen slot is 36..44.
    pub hotbar: u8,
    /// Complete received main predecessor and its packet ordinal.
    pub main_before: ObservedValue<SlotKnowledge>,
    /// Complete received hotbar predecessor and its packet ordinal.
    pub hotbar_before: ObservedValue<SlotKnowledge>,
    /// Intent recorded before I/O; there is no resend API.
    pub send: InventorySwapSend,
    /// Fresh exact main destination receipt.
    pub main_receipt: Option<ObservedValue<SlotKnowledge>>,
    /// Fresh exact hotbar destination receipt.
    pub hotbar_receipt: Option<ObservedValue<SlotKnowledge>>,
    /// Actual legacy transaction response; modern has no such response.
    pub legacy_reply: Option<InventoryTransactionReply>,
    /// First latched conflict; matching later state cannot erase it.
    pub requires_inspection: Option<String>,
    /// Retained outcome, separate from current inventory values.
    pub stage: InventorySwapStage,
}
pub(crate) fn unavailable(message: impl std::fmt::Display) -> crate::Error {
    crate::Error::new(crate::ErrorKind::State, anyhow::anyhow!("{message}"))
}
pub(crate) fn validate_slots(main: u8, hotbar: u8) -> Result<()> {
    if !(9..=35).contains(&main) || hotbar > 8 {
        return Err(super::registry::invalid(
            "swap requires main screen slot 9..35 and hotbar index 0..8",
        ));
    }
    Ok(())
}
pub(crate) fn prepare(
    initial: PlayerObservation,
    mode: GameMode,
    main_slot: u8,
    hotbar: u8,
    attempt: u64,
) -> Result<InventorySwapRecord> {
    validate_slots(main_slot, hotbar)?;
    if !matches!(mode, GameMode::Survival | GameMode::Creative)
        || initial.game_mode != Some(mode)
        || initial.pending_dispatch
        || initial.inventory.window_id != Some(0)
        || !matches!(
            initial.inventory.cursor.as_ref(),
            Some(ObservedValue {
                value: SlotKnowledge::Empty,
                source: ValueSource::Received { .. }
            })
        )
    {
        return Err(unavailable(
            "swap requires matching received mode, player screen, received empty cursor and no unresolved dispatch",
        ));
    }
    let registry = super::registry::Registry::for_version(initial.session.version);
    let get = |slot: usize| -> Result<ObservedValue<SlotKnowledge>> {
        let value = initial
            .inventory
            .slots
            .get(slot)
            .and_then(Option::as_ref)
            .filter(|v| {
                matches!(v.source, ValueSource::Received { .. })
                    && !matches!(v.value, SlotKnowledge::Unavailable)
            })
            .ok_or_else(|| unavailable("complete received swap predecessor unavailable"))?
            .clone();
        if let SlotKnowledge::Item { item } = &value.value {
            let definition = registry.item(&item.name)?;
            if item.data != ItemData::Default
                || item.id != definition.id
                || item.count == 0
                || item.count > definition.max_stack_size
            {
                return Err(crate::Error::new(
                    crate::ErrorKind::Unsupported,
                    anyhow::anyhow!("common swap currently requires valid default stacks"),
                ));
            }
        }
        Ok(value)
    };
    let main_before = get(usize::from(main_slot))?;
    let hotbar_before = get(36 + usize::from(hotbar))?;
    if main_before.value == hotbar_before.value {
        return Err(super::registry::invalid(
            "identical slot contents do not require a swap",
        ));
    }
    Ok(InventorySwapRecord {
        id: InventorySwapId::new(initial.session, attempt),
        send: InventorySwapSend {
            after_sequence: initial.receive_sequence,
            legacy_action: None,
            legacy_comparison: None,
            screen_revision: initial.inventory.screen_revision,
            dispatched: false,
        },
        initial,
        mode,
        main_slot,
        hotbar,
        main_before,
        hotbar_before,
        main_receipt: None,
        hotbar_receipt: None,
        legacy_reply: None,
        requires_inspection: None,
        stage: InventorySwapStage::Pending,
    })
}
pub(crate) fn inspection(record: &mut InventorySwapRecord, reason: impl std::fmt::Display) {
    if record.stage != InventorySwapStage::ObservedSwapped {
        record
            .requires_inspection
            .get_or_insert_with(|| reason.to_string());
        record.stage = InventorySwapStage::RequiresInspection;
    }
}
pub(crate) fn receive(record: &mut InventorySwapRecord, current: &PlayerObservation) {
    if record.stage == InventorySwapStage::ObservedSwapped {
        return;
    }
    if current.session != record.initial.session
        || current.game_mode != Some(record.mode)
        || current.inventory.window_id != Some(0)
        || !matches!(
            current.inventory.cursor.as_ref(),
            Some(ObservedValue {
                value: SlotKnowledge::Empty,
                source: ValueSource::Received { .. }
            })
        )
    {
        inspection(
            record,
            "swap session/mode/player screen/cursor context changed",
        );
    }
    for (slot, before, after, receipt) in [
        (
            usize::from(record.main_slot),
            &record.main_before.value,
            &record.hotbar_before.value,
            &mut record.main_receipt,
        ),
        (
            36 + usize::from(record.hotbar),
            &record.hotbar_before.value,
            &record.main_before.value,
            &mut record.hotbar_receipt,
        ),
    ] {
        match current.inventory.slots.get(slot).and_then(Option::as_ref) {
            Some(value)
                if matches!(value.source, ValueSource::Received { .. })
                    && (&value.value == before || &value.value == after) =>
            {
                if let ValueSource::Received { sequence } = value.source {
                    if sequence > record.send.after_sequence {
                        if &value.value == after {
                            receipt.get_or_insert_with(|| value.clone());
                        } else if receipt.is_some() {
                            record.requires_inspection.get_or_insert_with(|| {
                                "swap destination changed again before confirmation".into()
                            });
                        }
                    }
                }
            }
            _ => {
                record.requires_inspection.get_or_insert_with(|| {
                    "swap destination unavailable or conflicts with exact stacks".into()
                });
            }
        }
    }
    if record.requires_inspection.is_some() {
        record.stage = InventorySwapStage::RequiresInspection;
    }
}
pub(crate) fn destinations_ready(record: &InventorySwapRecord) -> bool {
    record.send.dispatched
        && record.requires_inspection.is_none()
        && record.main_receipt.is_some()
        && record.hotbar_receipt.is_some()
}
