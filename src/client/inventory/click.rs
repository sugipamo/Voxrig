//! One ordinary native PICKUP intent; predictions never become receive evidence.
use super::{InventoryClickSource, InventoryTransactionReply, unavailable};
use crate::client::{
    GameMode, ObservedValue, PlayerObservation, SessionStamp, SlotKnowledge, ValueSource,
    container::{ContainerScreen, PlayerScreenAccess, ScreenId},
};
use crate::{MinecraftVersion, Result};

/// Source UI for ordinary inventory operations, bound to the current basis.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum InventorySource {
    /// Canonical player screen; each operation validates its supported slot range.
    Player,
    /// Same received audited opening, including its appended player slots.
    Container {
        /// Original session/world/opening.
        screen: ScreenId,
    },
}
/// Native PICKUP button; neither variant is shift-click or item creation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryClickButton {
    /// Take/place/merge the whole available stack.
    Left,
    /// Take half (rounded up), or place one; different items exchange whole stacks.
    Right,
}
impl InventoryClickButton {
    pub(crate) fn native(self) -> u8 {
        match self {
            Self::Left => 0,
            Self::Right => 1,
        }
    }
}
/// Original connection/world-local attempt, not serializable replay authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct InventoryClickId {
    session: SessionStamp,
    attempt: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    close: Option<crate::client::container::ContainerCloseId>,
}
impl InventoryClickId {
    /// Original owning transport/world.
    pub fn session(self) -> SessionStamp {
        self.session
    }
    /// Parent close for an internal return step; None for a standalone click.
    pub fn close(self) -> Option<crate::client::container::ContainerCloseId> {
        self.close
    }
    /// Local attempt number, scoped to the parent when present.
    pub fn attempt(self) -> u64 {
        self.attempt
    }
}
/// Separate native prediction used only to check later actual receipts.
#[derive(Clone, Debug, serde::Serialize)]
pub struct InventoryClickPrediction {
    /// Expected clicked slot, always Predicted.
    pub source: ObservedValue<SlotKnowledge>,
    /// Expected cursor, always Predicted.
    pub cursor: ObservedValue<SlotKnowledge>,
}
/// Submission facts retained before possible write.
#[derive(Clone, Debug, serde::Serialize)]
pub struct InventoryClickSend {
    /// Applied receive boundary before send.
    pub after_sequence: u64,
    /// Native legacy action, absent on modern.
    pub legacy_action: Option<i16>,
    /// Exact comparison, including Empty; None before reservation/modern.
    pub legacy_comparison: Option<SlotKnowledge>,
    /// Actual modern UI revision, absent on legacy.
    pub screen_revision: Option<i32>,
    /// Revision actually encoded in the command; distinct from the received revision.
    pub sent_screen_revision: Option<i32>,
    /// Request a native full screen/cursor update using a revision mismatch.
    /// An empty cursor comparison in this mode is a resync marker, never a receipt.
    pub request_full_resync: bool,
    /// A complete frame write, not acceptance.
    pub dispatched: bool,
}
/// Historical facts of one click; the original intent is never resent.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InventoryClickStage {
    /// Submission/required receipts are incomplete; inspect without replay.
    Pending,
    /// Full dispatch and both fresh exact receipts agree; legacy reply also present.
    ObservedClicked,
    /// First conflict or delivery uncertainty stays latched.
    RequiresInspection,
}
/// One retained ordinary click, accessible after cancellation/disconnection.
#[derive(Clone, Debug, serde::Serialize)]
pub struct InventoryClickRecord {
    #[serde(skip)]
    pub(crate) item_context: Option<std::sync::Arc<super::data::ItemContext>>,
    /// Original opaque identity.
    pub id: InventoryClickId,
    /// Coherent received baseline before I/O.
    pub initial: PlayerObservation,
    /// Required actual mode.
    pub mode: GameMode,
    /// Original source UI.
    pub source: InventoryClickSource,
    /// Native clicked screen slot.
    pub source_slot: u16,
    /// Original native button.
    pub button: InventoryClickButton,
    /// Actual received opening, absent for player screen.
    pub initial_screen: Option<ContainerScreen>,
    /// Received predecessor.
    pub source_before: ObservedValue<SlotKnowledge>,
    /// Received cursor predecessor, possibly nonempty.
    pub cursor_before: ObservedValue<SlotKnowledge>,
    /// Native default-item/slot prediction, not received outcome.
    pub prediction: InventoryClickPrediction,
    /// Original intent/submission.
    pub send: InventoryClickSend,
    /// Fresh matching source receipt.
    pub source_receipt: Option<ObservedValue<SlotKnowledge>>,
    /// Fresh matching cursor receipt.
    pub cursor_receipt: Option<ObservedValue<SlotKnowledge>>,
    /// Matching actual legacy comparison reply; false is not rollback.
    pub legacy_reply: Option<InventoryTransactionReply>,
    /// Persistent first conflict.
    pub requires_inspection: Option<String>,
    /// Retained result independent of current inventory.
    pub stage: InventoryClickStage,
}
impl InventoryClickRecord {
    pub(crate) fn bind_close(&mut self, close: crate::client::container::ContainerCloseId) {
        self.id.close = Some(close);
    }
    /// Native source window, not a live permission.
    pub fn window_id(&self) -> i32 {
        match self.source {
            InventoryClickSource::Player => 0,
            InventoryClickSource::Container { screen } => screen.window_id(),
        }
    }
    pub(crate) fn unresolved(&self) -> bool {
        self.stage != InventoryClickStage::ObservedClicked
    }
    pub(crate) fn inspection(&mut self, reason: impl std::fmt::Display) {
        if self.unresolved() {
            self.requires_inspection
                .get_or_insert_with(|| reason.to_string());
            self.stage = InventoryClickStage::RequiresInspection;
        }
    }
    pub(crate) fn ready(&self) -> bool {
        self.send.dispatched
            && self.requires_inspection.is_none()
            && self.source_receipt.is_some()
            && self.cursor_receipt.is_some()
            && (self.initial.session.version == MinecraftVersion::Java1_21_11
                || self.legacy_reply.is_some())
    }
    fn canonical_slot(&self) -> Option<usize> {
        match self.source {
            InventoryClickSource::Player => Some(usize::from(self.source_slot)),
            InventoryClickSource::Container { .. } => self
                .initial_screen
                .as_ref()?
                .layout
                .as_ref()?
                .player_slots
                .iter()
                .find(|s| s.screen_slot == usize::from(self.source_slot))
                .map(|s| s.player_slot),
        }
    }
}
fn received_value(
    value: Option<&ObservedValue<SlotKnowledge>>,
) -> Result<ObservedValue<SlotKnowledge>> {
    value
        .filter(|v| {
            matches!(v.source, ValueSource::Received { .. })
                && !matches!(v.value, SlotKnowledge::Unavailable)
        })
        .cloned()
        .ok_or_else(|| unavailable("complete received click predecessor unavailable"))
}
pub(crate) fn prepare(
    initial: PlayerObservation,
    mode: GameMode,
    source: InventoryClickSource,
    source_slot: u16,
    button: InventoryClickButton,
    attempt: u64,
    screen: Option<ContainerScreen>,
) -> Result<InventoryClickRecord> {
    prepare_inner(
        (initial, None),
        mode,
        source,
        source_slot,
        button,
        attempt,
        screen,
    )
}
pub(crate) fn prepare_received(
    capture: (
        PlayerObservation,
        crate::client::registry::ServerRegistryObservation,
    ),
    mode: GameMode,
    source: InventoryClickSource,
    source_slot: u16,
    button: InventoryClickButton,
    attempt: u64,
    screen: Option<ContainerScreen>,
) -> Result<InventoryClickRecord> {
    prepare_inner(
        (capture.0, Some(capture.1)),
        mode,
        source,
        source_slot,
        button,
        attempt,
        screen,
    )
}
fn prepare_inner(
    capture: (
        PlayerObservation,
        Option<crate::client::registry::ServerRegistryObservation>,
    ),
    mode: GameMode,
    source: InventoryClickSource,
    source_slot: u16,
    button: InventoryClickButton,
    attempt: u64,
    screen: Option<ContainerScreen>,
) -> Result<InventoryClickRecord> {
    let (initial, registries) = capture;
    if attempt == 0
        || initial.pending_dispatch
        || initial.game_mode != Some(mode)
        || !matches!(mode, GameMode::Survival | GameMode::Creative)
    {
        return Err(unavailable(
            "click requires matching received mode and no unresolved dispatch",
        ));
    }
    let (slots, menu, revision) = match source {
        InventoryClickSource::Player => {
            if !(1..=4).contains(&source_slot) && !(9..=44).contains(&source_slot) {
                return Err(crate::client::registry::invalid(
                    "ordinary player click requires input slot 1..4 or inventory slot 9..44",
                ));
            }
            match initial.inventory.player_screen {
                Some(PlayerScreenAccess::Received) if initial.inventory.window_id == Some(0) => {}
                Some(PlayerScreenAccess::SubmittedClose { close })
                    if close.screen().session() == initial.session => {}
                _ => {
                    return Err(unavailable(
                        "same actual player screen or completed local close required",
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
            (&initial.inventory.slots, "minecraft:player", revision)
        }
        InventoryClickSource::Container { screen: id } => {
            let s = screen
                .as_ref()
                .filter(|s| {
                    s.id == id
                        && id.session() == initial.session
                        && initial.inventory.window_id == Some(id.window_id())
                        && s.full_contents_sequence.is_some()
                })
                .ok_or_else(|| unavailable("same live received audited opening/full required"))?;
            let layout = s
                .layout
                .as_ref()
                .ok_or_else(|| unavailable("native click layout unavailable"))?;
            let menu = s
                .menu_name
                .as_deref()
                .filter(|n| super::storage_menu(n) || *n == "minecraft:crafting")
                .ok_or_else(|| unavailable("audited ordinary menu required"))?;
            if usize::from(source_slot) >= layout.total_slots {
                return Err(crate::client::registry::invalid(
                    "click outside native layout",
                ));
            }
            if let Some(mapping) = layout
                .player_slots
                .iter()
                .find(|m| m.screen_slot == usize::from(source_slot))
            {
                if s.slots.get(mapping.screen_slot)
                    != initial.inventory.slots.get(mapping.player_slot)
                {
                    return Err(unavailable("screen/player source receipts disagree"));
                }
            }
            (
                &s.slots,
                menu,
                s.revision
                    .as_ref()
                    .filter(|r| matches!(r.source, ValueSource::Received { .. }))
                    .map(|r| r.value),
            )
        }
    };
    if initial.session.version == MinecraftVersion::Java1_21_11 && revision.is_none() {
        return Err(unavailable("actual click UI revision unavailable"));
    }
    if initial.session.version == MinecraftVersion::Java1_16_1
        && !matches!(source, InventoryClickSource::Player)
        && screen
            .as_ref()
            .is_none_or(|s| !(1..=127).contains(&s.id.window_id()))
    {
        return Err(unavailable(
            "legacy click requires signed positive native window 1..127",
        ));
    }
    let source_before =
        received_value(slots.get(usize::from(source_slot)).and_then(Option::as_ref))?;
    let cursor_before = received_value(initial.inventory.cursor.as_ref())?;
    let has_data = [&source_before, &cursor_before].iter().any(|v| {
        matches!(&v.value, SlotKnowledge::Item {item} if item.data != crate::client::ItemData::Default)
    });
    let item_context = if let (true, Some(registries)) = (has_data, registries) {
        let context = super::data::ItemContext::new(registries, &initial)?;
        context.predecessor(&source_before)?;
        context.predecessor(&cursor_before)?;
        Some(std::sync::Arc::new(context))
    } else {
        None
    };
    let (expected_source, expected_cursor) = if let Some(context) = &item_context {
        super::slot_policy::pickup_with_data(
            initial.session.version,
            menu,
            usize::from(source_slot),
            button,
            (&source_before.value, &cursor_before.value),
            context,
        )?
    } else {
        super::slot_policy::pickup(
            initial.session.version,
            menu,
            usize::from(source_slot),
            button,
            &source_before.value,
            &cursor_before.value,
        )?
    };
    if expected_source == source_before.value && expected_cursor == cursor_before.value {
        return Err(crate::client::registry::invalid(
            "ordinary click would have no effect; no packet submitted",
        ));
    }
    let prediction = InventoryClickPrediction {
        source: ObservedValue {
            value: expected_source,
            source: ValueSource::Predicted,
        },
        cursor: ObservedValue {
            value: expected_cursor,
            source: ValueSource::Predicted,
        },
    };
    // Modern player inventory updates can omit the crafting slots. A known
    // ordinary source still permits this PICKUP; request actual full contents
    // so the following input operation need not invent an empty crafting grid.
    let missing_player_grid = matches!(source, InventoryClickSource::Player)
        && (0..=4).any(|slot| {
            initial
                .inventory
                .slots
                .get(slot)
                .and_then(Option::as_ref)
                .is_none_or(|v| {
                    !matches!(v.source, ValueSource::Received { .. })
                        || matches!(v.value, SlotKnowledge::Unavailable)
                })
        });
    let request_full_resync = (item_context.is_some() || missing_player_grid)
        && initial.session.version == MinecraftVersion::Java1_21_11;
    let sent_screen_revision = revision.map(|v| {
        if request_full_resync {
            if v == 0 { 1 } else { 0 }
        } else {
            v
        }
    });
    Ok(InventoryClickRecord {
        item_context,
        id: InventoryClickId {
            session: initial.session,
            attempt,
            close: None,
        },
        send: InventoryClickSend {
            after_sequence: initial.receive_sequence,
            legacy_action: None,
            legacy_comparison: None,
            screen_revision: revision,
            sent_screen_revision,
            request_full_resync,
            dispatched: false,
        },
        initial,
        mode,
        source,
        source_slot,
        button,
        initial_screen: if matches!(source, InventoryClickSource::Player) {
            None
        } else {
            screen
        },
        source_before,
        cursor_before,
        prediction,
        source_receipt: None,
        cursor_receipt: None,
        legacy_reply: None,
        requires_inspection: None,
        stage: InventoryClickStage::Pending,
    })
}
pub(crate) fn source_slots<'a>(
    record: &InventoryClickRecord,
    current: &'a PlayerObservation,
    screen: Option<&'a ContainerScreen>,
) -> Result<&'a Vec<Option<ObservedValue<SlotKnowledge>>>> {
    if current.session != record.initial.session || current.game_mode != Some(record.mode) {
        return Err(unavailable("click session/world/mode changed"));
    }
    if record.initial.selected_hotbar.as_ref().map(|s| s.value)
        != current.selected_hotbar.as_ref().map(|s| s.value)
    {
        return Err(unavailable("click selected hand context changed"));
    }
    let slots = match record.source {
        InventoryClickSource::Player => {
            let received = current.inventory.player_screen == Some(PlayerScreenAccess::Received)
                && current.inventory.window_id == Some(0);
            let closed = matches!((record.initial.inventory.player_screen,current.inventory.player_screen),(Some(PlayerScreenAccess::SubmittedClose{close:before}),Some(PlayerScreenAccess::SubmittedClose{close:now}))if before==now);
            if !received && !closed {
                return Err(unavailable("click player UI basis changed"));
            }
            &current.inventory.slots
        }
        InventoryClickSource::Container { screen: id } => {
            let s = screen
                .filter(|s| {
                    s.id == id
                        && s.full_contents_sequence.is_some()
                        && current.inventory.window_id == Some(id.window_id())
                })
                .ok_or_else(|| unavailable("click original opening changed"))?;
            let old = record
                .initial_screen
                .as_ref()
                .ok_or_else(|| unavailable("click opening baseline missing"))?;
            if s.layout != old.layout || s.menu_name != old.menu_name {
                return Err(unavailable("click native layout/menu changed"));
            }
            if let Some(index) = record.canonical_slot() {
                if s.slots.get(usize::from(record.source_slot))
                    != current.inventory.slots.get(index)
                {
                    return Err(unavailable("click player/screen source projection changed"));
                }
            }
            &s.slots
        }
    };
    // An ordinary PICKUP can change its mapped source hand, but no other hand.
    for index in [
        record
            .initial
            .selected_hotbar
            .as_ref()
            .map(|s| 36 + usize::from(s.value)),
        Some(45),
    ]
    .into_iter()
    .flatten()
    {
        if Some(index) == record.canonical_slot() {
            continue;
        }
        if let Some(before) = record
            .initial
            .inventory
            .slots
            .get(index)
            .and_then(Option::as_ref)
            .filter(|v| {
                matches!(v.source, ValueSource::Received { .. })
                    && !matches!(v.value, SlotKnowledge::Unavailable)
            })
        {
            if !current
                .inventory
                .slots
                .get(index)
                .and_then(Option::as_ref)
                .is_some_and(|v| {
                    matches!(v.source, ValueSource::Received { .. }) && v.value == before.value
                })
            {
                return Err(unavailable(
                    "click unaffected hand changed or became unavailable",
                ));
            }
        }
    }
    Ok(slots)
}
pub(crate) fn receive(
    record: &mut InventoryClickRecord,
    current: &PlayerObservation,
    screen: Option<&ContainerScreen>,
) {
    receive_inner(record, current, screen, None);
}
pub(crate) fn receive_with_registries(
    record: &mut InventoryClickRecord,
    current: &PlayerObservation,
    screen: Option<&ContainerScreen>,
    registries: &crate::client::registry::ServerRegistryObservation,
) {
    receive_inner(record, current, screen, Some(registries));
}
fn receive_inner(
    record: &mut InventoryClickRecord,
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
    let values = [
        slots
            .get(usize::from(record.source_slot))
            .and_then(Option::as_ref),
        current.inventory.cursor.as_ref(),
    ];
    for (value, before, after, receipt) in [
        (
            values[0],
            &record.source_before,
            &record.prediction.source.value,
            &mut record.source_receipt,
        ),
        (
            values[1],
            &record.cursor_before,
            &record.prediction.cursor.value,
            &mut record.cursor_receipt,
        ),
    ] {
        let Some(value) = value.filter(|v| matches!(v.source, ValueSource::Received { .. })) else {
            record
                .requires_inspection
                .get_or_insert_with(|| "click receipt unavailable".into());
            continue;
        };
        let classification = if let Some(context) = &record.item_context {
            registries
                .filter(|r| {
                    r.session() == current.session
                        && r.receive_sequence() == current.receive_sequence
                })
                .ok_or_else(|| unavailable("coherent click registry capture unavailable"))
                .and_then(|owner| context.classify(value, before, after, owner))
        } else {
            Ok((value.value == before.value, &value.value == after))
        };
        let (matches_before, matches_after) = match classification {
            Ok(v) if v.0 || v.1 => v,
            Ok(_) => {
                record
                    .requires_inspection
                    .get_or_insert_with(|| "click native item fields conflict".into());
                continue;
            }
            Err(e) => {
                record
                    .requires_inspection
                    .get_or_insert_with(|| e.to_string());
                continue;
            }
        };
        let ValueSource::Received { sequence } = value.source else {
            unreachable!()
        };
        if sequence > record.send.after_sequence {
            if !record.send.dispatched && !matches_before {
                record
                    .requires_inspection
                    .get_or_insert_with(|| "click values changed before complete dispatch".into());
            } else if matches_after {
                receipt.get_or_insert_with(|| value.clone());
            } else if receipt.is_some() {
                record.requires_inspection.get_or_insert_with(|| {
                    "click outcome restored/changed before confirmation".into()
                });
            }
        }
    }
    if record.requires_inspection.is_some() {
        record.stage = InventoryClickStage::RequiresInspection;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crafting_missing_player_grid_requests_native_full_without_empty_prediction() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let record = fixture(version);
            assert!(!record.send.request_full_resync);
            let mut initial = record.initial.clone();
            initial.inventory.slots[1] = None;
            let click = prepare(
                initial,
                GameMode::Survival,
                InventorySource::Player,
                9,
                InventoryClickButton::Left,
                2,
                None,
            )
            .unwrap();
            assert!(click.initial.inventory.slots[1].is_none());
            assert_eq!(
                click.send.request_full_resync,
                version == MinecraftVersion::Java1_21_11
            );
            assert_eq!(
                click.send.sent_screen_revision,
                Some(if version == MinecraftVersion::Java1_21_11 {
                    0
                } else {
                    7
                })
            );
            assert_eq!(click.source_slot, 9);
            assert!(click.source_receipt.is_none());
        }
    }
    fn fixture(version: MinecraftVersion) -> InventoryClickRecord {
        use crate::client::{InventoryObservation, ItemData, ItemStack, registry::Registry};
        let mut slots = vec![Some(crate::client::received(SlotKnowledge::Empty, 10)); 46];
        let definition = Registry::for_version(version)
            .item("minecraft:stone")
            .unwrap();
        slots[9] = Some(crate::client::received(
            SlotKnowledge::Item {
                item: ItemStack {
                    id: definition.id,
                    name: definition.name,
                    count: 7,
                    data: ItemData::Default,
                },
            },
            10,
        ));
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
            selected_hotbar: Some(crate::client::received(0, 10)),
            inventory: InventoryObservation {
                slots,
                cursor: Some(crate::client::received(SlotKnowledge::Empty, 10)),
                window_id: Some(0),
                player_screen: Some(PlayerScreenAccess::Received),
                screen_revision: Some(7),
                player_screen_revision: Some(crate::client::received(7, 10)),
                local_cache: None,
            },
        };
        prepare(
            initial,
            GameMode::Survival,
            InventoryClickSource::Player,
            9,
            InventoryClickButton::Right,
            1,
            None,
        )
        .unwrap()
    }
    fn after(record: &InventoryClickRecord, sequence: u64) -> PlayerObservation {
        let mut current = record.initial.clone();
        current.receive_sequence = sequence;
        current.inventory.slots[9] = Some(crate::client::received(
            record.prediction.source.value.clone(),
            sequence,
        ));
        current.inventory.cursor = Some(crate::client::received(
            record.prediction.cursor.value.clone(),
            sequence,
        ));
        current
    }
    #[test]
    fn pickup_requires_full_write_both_fresh_receipts_and_native_reply_never_prediction() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let mut record = fixture(version);
            let stale = after(&record, 10);
            record.send.dispatched = true;
            receive(&mut record, &stale, None);
            assert!(record.source_receipt.is_none() && record.cursor_receipt.is_none());
            assert!(!record.ready());
            let mut partial = after(&record, 11);
            partial.inventory.cursor = record.initial.inventory.cursor.clone();
            receive(&mut record, &partial, None);
            assert!(record.source_receipt.is_some() && record.cursor_receipt.is_none());
            assert!(!record.ready());
            let full = after(&record, 12);
            receive(&mut record, &full, None);
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
            record.send.dispatched = false;
            assert!(!record.ready());
            let mut unsent = fixture(version);
            let changed = after(&unsent, 11);
            receive(&mut unsent, &changed, None);
            assert_eq!(unsent.stage, InventoryClickStage::RequiresInspection);
            unsent.send.dispatched = true;
            receive(&mut unsent, &changed, None);
            assert!(!unsent.ready());
            let mut predicted = fixture(version);
            predicted.send.dispatched = true;
            let mut changed = after(&predicted, 11);
            changed.inventory.cursor = Some(predicted.prediction.cursor.clone());
            receive(&mut predicted, &changed, None);
            assert_eq!(predicted.stage, InventoryClickStage::RequiresInspection);
        }
    }
    #[test]
    fn pickup_latches_restoration_world_mode_selection_other_hand_and_ui_conflicts() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            for conflict in 0..7 {
                let mut record = fixture(version);
                record.send.dispatched = true;
                let mut current = after(&record, 11);
                current.inventory.cursor = record.initial.inventory.cursor.clone();
                receive(&mut record, &current, None);
                let mut changed = after(&record, 12);
                match conflict {
                    0 => {
                        changed.inventory.slots[9] = Some(crate::client::received(
                            record.source_before.value.clone(),
                            12,
                        ))
                    }
                    1 => changed.session.world_generation += 1,
                    2 => changed.game_mode = Some(GameMode::Creative),
                    3 => changed.selected_hotbar = Some(crate::client::received(1, 12)),
                    4 => changed.inventory.slots[36] = None,
                    5 => changed.inventory.window_id = Some(3),
                    _ => changed.inventory.cursor = None,
                }
                receive(&mut record, &changed, None);
                let reason = record.requires_inspection.clone();
                assert!(reason.is_some(), "{conflict}");
                let restored = after(&record, 13);
                receive(&mut record, &restored, None);
                assert_eq!(record.requires_inspection, reason);
                assert!(!record.ready());
            }
            let mut record = fixture(version);
            record.stage = InventoryClickStage::ObservedClicked;
            let mut closed = after(&record, 20);
            closed.session.world_generation += 1;
            closed.game_mode = None;
            closed.inventory.cursor = None;
            receive(&mut record, &closed, None);
            record.inspection("disconnect");
            assert_eq!(record.stage, InventoryClickStage::ObservedClicked);
            assert!(record.requires_inspection.is_none());
        }
    }
}
