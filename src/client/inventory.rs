//! Received ordinary player/container exchanges; no click prediction is receive evidence.
pub(crate) mod click;
pub(crate) mod slot_policy;
pub(crate) mod transfer;
pub(crate) mod transfer_policy;
use super::container::{ContainerScreen, ScreenId};
use super::{
    GameMode, ItemData, ObservedValue, PlayerObservation, SessionStamp, SlotKnowledge, ValueSource,
};
use crate::Result;
/// Compatibility name for the common source UI used by inventory operations.
pub use click::InventorySource as InventoryClickSource;
pub use click::{
    InventoryClickButton, InventoryClickId, InventoryClickPrediction, InventoryClickRecord,
    InventoryClickSend, InventoryClickStage, InventorySource,
};
pub use transfer::{
    InventoryTransferId, InventoryTransferRecord, InventoryTransferSlot, InventoryTransferStage,
};

/// Screen owning the clicked source slot. Container IDs are bound to an actual opening.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum InventorySwapSource {
    /// Canonical player screen, with source slot 9..35.
    PlayerMain,
    /// Constructor-verified storage screen; the clicked slot is not a player slot.
    Container {
        /// Original live opening identity.
        screen: ScreenId,
    },
}

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
    /// Native click screen identifier.
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
    /// Source screen: player main inventory or a received container opening.
    pub source: InventorySwapSource,
    /// Native screen slot clicked by SWAP, 9..35 for PlayerMain.
    pub source_slot: u16,
    /// Native screen slot referring to the selected hotbar destination.
    pub hotbar_screen_slot: u16,
    /// Actual container baseline at the same capture boundary; absent on PlayerMain.
    pub initial_screen: Option<ContainerScreen>,
    /// Hotbar index, 0..8; canonical player-screen index is 36..44.
    pub hotbar: u8,
    /// Complete received source predecessor and its packet ordinal.
    pub source_before: ObservedValue<SlotKnowledge>,
    /// Complete received hotbar predecessor and its packet ordinal.
    pub hotbar_before: ObservedValue<SlotKnowledge>,
    /// Intent recorded before I/O; there is no resend API.
    pub send: InventorySwapSend,
    /// Fresh exact source destination receipt.
    pub source_receipt: Option<ObservedValue<SlotKnowledge>>,
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
impl InventorySwapRecord {
    /// Native click window ID. It does not confer live operation authority.
    pub fn window_id(&self) -> i32 {
        match self.source {
            InventorySwapSource::PlayerMain => 0,
            InventorySwapSource::Container { screen } => screen.window_id(),
        }
    }
}
pub(crate) fn prepare(
    initial: PlayerObservation,
    mode: GameMode,
    main_slot: u8,
    hotbar: u8,
    attempt: u64,
) -> Result<InventorySwapRecord> {
    validate_slots(main_slot, hotbar)?;
    prepare_source(
        initial,
        mode,
        InventorySwapSource::PlayerMain,
        u16::from(main_slot),
        hotbar,
        attempt,
        None,
    )
}
pub(crate) fn prepare_source(
    initial: PlayerObservation,
    mode: GameMode,
    source: InventorySwapSource,
    source_slot: u16,
    hotbar: u8,
    attempt: u64,
    screen: Option<ContainerScreen>,
) -> Result<InventorySwapRecord> {
    if hotbar > 8
        || !matches!(mode, GameMode::Survival | GameMode::Creative)
        || initial.game_mode != Some(mode)
        || initial.pending_dispatch
        || !matches!(
            initial.inventory.cursor.as_ref(),
            Some(ObservedValue {
                value: SlotKnowledge::Empty,
                source: ValueSource::Received { .. }
            })
        )
    {
        return Err(unavailable(
            "swap requires matching received mode/empty cursor and no unresolved dispatch",
        ));
    }
    let (slots, hotbar_screen_slot, revision) = match source {
        InventorySwapSource::PlayerMain => {
            use super::container::PlayerScreenAccess;
            let player_screen = match initial.inventory.player_screen {
                Some(PlayerScreenAccess::Received) => initial.inventory.window_id == Some(0),
                Some(PlayerScreenAccess::SubmittedClose { close }) => {
                    close.screen().session() == initial.session
                }
                None => false,
            };
            if !(9..=35).contains(&source_slot) || !player_screen {
                return Err(unavailable(
                    "player swap requires received player screen or explicit completed local close, and main slot 9..35",
                ));
            }
            let revision = match initial.inventory.player_screen {
                Some(PlayerScreenAccess::SubmittedClose { .. }) => match initial.session.version {
                    crate::MinecraftVersion::Java1_16_1 => None,
                    crate::MinecraftVersion::Java1_21_11 => Some(
                        initial
                            .inventory
                            .player_screen_revision
                            .as_ref()
                            .filter(|r| matches!(r.source, ValueSource::Received { .. }))
                            .ok_or_else(|| {
                                unavailable(
                                    "actual player-screen-zero revision unavailable after close",
                                )
                            })?
                            .value,
                    ),
                },
                _ => initial.inventory.screen_revision,
            };
            (&initial.inventory.slots, 36 + u16::from(hotbar), revision)
        }
        InventorySwapSource::Container { screen: id } => {
            let screen = screen
                .as_ref()
                .filter(|s| {
                    s.id == id
                        && id.session() == initial.session
                        && initial.inventory.window_id == Some(id.window_id())
                        && s.full_contents_sequence.is_some()
                })
                .ok_or_else(|| unavailable("same live container opening/full contents required"))?;
            if !screen.menu_name.as_deref().is_some_and(storage_menu) {
                return Err(unavailable(
                    "constructor-verified ordinary storage menu required",
                ));
            }
            let layout = screen
                .layout
                .as_ref()
                .ok_or_else(|| unavailable("container layout unavailable"))?;
            if usize::from(source_slot) >= layout.total_slots
                || layout
                    .player_slots
                    .iter()
                    .any(|m| m.screen_slot == usize::from(source_slot))
            {
                return Err(super::registry::invalid(
                    "source must be a storage slot, not an appended player slot",
                ));
            }
            let hotbar_screen_slot = layout
                .player_slots
                .iter()
                .find(|m| m.player_slot == 36 + usize::from(hotbar))
                .and_then(|m| u16::try_from(m.screen_slot).ok())
                .ok_or_else(|| unavailable("native hotbar mapping unavailable"))?;
            (
                &screen.slots,
                hotbar_screen_slot,
                screen.revision.as_ref().map(|r| r.value),
            )
        }
    };
    let registry = super::registry::Registry::for_version(initial.session.version);
    let get = |slot: usize| -> Result<ObservedValue<SlotKnowledge>> {
        let value = slots
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
    let source_before = get(usize::from(source_slot))?;
    let hotbar_before = get(usize::from(hotbar_screen_slot))?;
    slot_policy::validate_swap(
        initial.session.version,
        screen
            .as_ref()
            .and_then(|s| s.menu_name.as_deref())
            .unwrap_or("minecraft:player"),
        usize::from(source_slot),
        &source_before.value,
        &hotbar_before.value,
    )?;
    if source_before.value == hotbar_before.value {
        return Err(super::registry::invalid(
            "identical slot contents do not require a swap",
        ));
    }
    if matches!(source, InventorySwapSource::Container { .. })
        && initial.inventory.slots[36 + usize::from(hotbar)].as_ref() != Some(&hotbar_before)
    {
        return Err(unavailable("container/player hotbar receipts disagree"));
    }
    Ok(InventorySwapRecord {
        id: InventorySwapId::new(initial.session, attempt),
        send: InventorySwapSend {
            after_sequence: initial.receive_sequence,
            legacy_action: None,
            legacy_comparison: None,
            screen_revision: revision,
            dispatched: false,
        },
        initial,
        mode,
        source,
        source_slot,
        hotbar_screen_slot,
        initial_screen: screen,
        hotbar,
        source_before,
        hotbar_before,
        source_receipt: None,
        hotbar_receipt: None,
        legacy_reply: None,
        requires_inspection: None,
        stage: InventorySwapStage::Pending,
    })
}
fn storage_menu(name: &str) -> bool {
    matches!(
        name,
        "minecraft:generic_9x1"
            | "minecraft:generic_9x2"
            | "minecraft:generic_9x3"
            | "minecraft:generic_9x4"
            | "minecraft:generic_9x5"
            | "minecraft:generic_9x6"
            | "minecraft:generic_3x3"
            | "minecraft:hopper"
            | "minecraft:shulker_box"
    )
}
/// Validate current source identity, received cursor/mode and complete slot context.
pub(crate) fn source_slots<'a>(
    record: &InventorySwapRecord,
    current: &'a PlayerObservation,
    screen: Option<&'a ContainerScreen>,
) -> Result<&'a Vec<Option<ObservedValue<SlotKnowledge>>>> {
    if current.session != record.initial.session
        || current.game_mode != Some(record.mode)
        || !matches!(
            current.inventory.cursor.as_ref(),
            Some(ObservedValue {
                value: SlotKnowledge::Empty,
                source: ValueSource::Received { .. }
            })
        )
    {
        return Err(unavailable(
            "swap session/mode/screen/cursor context changed",
        ));
    }
    match record.source {
        InventorySwapSource::PlayerMain => {
            use super::container::PlayerScreenAccess;
            let received = current.inventory.player_screen == Some(PlayerScreenAccess::Received)
                && current.inventory.window_id == Some(0);
            let locally_closed = matches!((record.initial.inventory.player_screen,current.inventory.player_screen),
                (Some(PlayerScreenAccess::SubmittedClose { close: before }),Some(PlayerScreenAccess::SubmittedClose { close: now })) if before == now);
            if !received && !locally_closed {
                return Err(unavailable("player screen/local close basis changed"));
            }
            Ok(&current.inventory.slots)
        }
        InventorySwapSource::Container { screen: id } => {
            if current.inventory.window_id != Some(id.window_id()) {
                return Err(unavailable("container active window changed"));
            }
            let screen = screen
                .filter(|s| s.id == id && s.full_contents_sequence.is_some())
                .ok_or_else(|| unavailable("container opening/full-content context changed"))?;
            let original = record
                .initial_screen
                .as_ref()
                .ok_or_else(|| unavailable("container baseline missing"))?;
            if screen.layout != original.layout
                || screen.menu_name != original.menu_name
                || screen.slots.get(usize::from(record.hotbar_screen_slot))
                    != current.inventory.slots.get(36 + usize::from(record.hotbar))
            {
                return Err(unavailable(
                    "container layout/player hotbar context changed",
                ));
            }
            Ok(&screen.slots)
        }
    }
}
pub(crate) fn inspection(record: &mut InventorySwapRecord, reason: impl std::fmt::Display) {
    if record.stage != InventorySwapStage::ObservedSwapped {
        record
            .requires_inspection
            .get_or_insert_with(|| reason.to_string());
        record.stage = InventorySwapStage::RequiresInspection;
    }
}
pub(crate) fn receive(
    record: &mut InventorySwapRecord,
    current: &PlayerObservation,
    screen: Option<&ContainerScreen>,
) {
    if record.stage == InventorySwapStage::ObservedSwapped {
        return;
    }
    let slots = match source_slots(record, current, screen) {
        Ok(slots) => slots,
        Err(error) => {
            inspection(record, error);
            return;
        }
    };
    for (slot, before, after, receipt) in [
        (
            usize::from(record.source_slot),
            &record.source_before.value,
            &record.hotbar_before.value,
            &mut record.source_receipt,
        ),
        (
            usize::from(record.hotbar_screen_slot),
            &record.hotbar_before.value,
            &record.source_before.value,
            &mut record.hotbar_receipt,
        ),
    ] {
        match slots.get(slot).and_then(Option::as_ref) {
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
        && record.source_receipt.is_some()
        && record.hotbar_receipt.is_some()
}
