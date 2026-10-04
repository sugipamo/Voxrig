//! Actual container-screen receipts, separate from the player inventory and click predictions.
pub(crate) mod open;
pub(crate) mod outline;
use super::{ObservedValue, SessionStamp, SlotKnowledge, received};
use crate::MinecraftVersion;
pub use open::{
    ContainerOpenId, ContainerOpenProcessing, ContainerOpenRecord, ContainerOpenSend,
    ContainerOpenStage, ContainerOpenTargetState,
};

/// One close intent, bound to its received connection/world/opening.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ContainerCloseId {
    screen: ScreenId,
    attempt: u64,
}
impl ContainerCloseId {
    /// Original received opening.
    pub fn screen(self) -> ScreenId {
        self.screen
    }
    /// Monotonic attempt on this client.
    pub fn attempt(self) -> u64 {
        self.attempt
    }
}
/// Local player-screen admission, separate from the last received active window.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum PlayerScreenAccess {
    /// Native receive established player screen zero.
    Received,
    /// This client completely dispatched close for the original opening.
    /// Not a received close or permission to replay the original screen's clicks.
    SubmittedClose {
        /// Exact retained close; serialization does not recreate authority.
        close: ContainerCloseId,
    },
}
pub(crate) fn player_screen_access(
    session: SessionStamp,
    window: Option<i32>,
    screen: Option<ScreenId>,
    close: Option<&ContainerCloseRecord>,
) -> Option<PlayerScreenAccess> {
    let closed = close.filter(|r| {
        r.dispatched && r.requires_inspection.is_none() && r.initial.session == session
    });
    if window == Some(0)
        && (screen.is_none() || closed.is_some_and(|r| screen == Some(r.id.screen)))
    {
        return Some(PlayerScreenAccess::Received);
    }
    closed
        .filter(|r| {
            (screen == Some(r.id.screen) && window == Some(r.id.screen.window_id()))
                || (screen.is_none() && window.is_none() && r.server_close_sequence.is_some())
        })
        .map(|r| PlayerScreenAccess::SubmittedClose { close: r.id })
}
/// Close transport facts. Vanilla need not send a close response.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerCloseStage {
    /// Intent retained before any I/O; do not replay it.
    Pending,
    /// Complete close frame written; this is not received server closure.
    Dispatched,
    /// A fresh actual close packet for the original opening was also received.
    ObservedClosed,
    /// Admission, context, cancellation or delivery needs inspection.
    RequiresInspection,
}
/// Retained close intent, readable after caller cancellation or disconnection.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ContainerCloseRecord {
    /// Opaque original attempt; there is no resend method.
    pub id: ContainerCloseId,
    /// Coherent received player/cursor baseline captured before I/O.
    pub initial: super::PlayerObservation,
    /// Original received screen at the same boundary.
    pub initial_screen: ContainerScreen,
    /// Matching received mode at admission.
    pub mode: super::GameMode,
    /// True only after a complete frame write.
    pub dispatched: bool,
    /// Actual original-opening CLOSE packet ordinal; never synthesized from send.
    pub server_close_sequence: Option<u64>,
    /// Persistent uncertainty, if any.
    pub requires_inspection: Option<String>,
    /// Transport and received outcome, kept separate.
    pub stage: ContainerCloseStage,
}
impl ContainerCloseRecord {
    pub(crate) fn unresolved(&self) -> bool {
        matches!(
            self.stage,
            ContainerCloseStage::Pending | ContainerCloseStage::RequiresInspection
        )
    }
    pub(crate) fn inspection(&mut self, reason: impl std::fmt::Display) {
        if self.unresolved() {
            self.requires_inspection
                .get_or_insert_with(|| reason.to_string());
            self.stage = ContainerCloseStage::RequiresInspection;
        }
    }
    pub(crate) fn sent(&mut self) {
        self.dispatched = true;
        if self.requires_inspection.is_none() {
            self.stage = if self.server_close_sequence.is_some() {
                ContainerCloseStage::ObservedClosed
            } else {
                ContainerCloseStage::Dispatched
            };
        }
    }
    pub(crate) fn received_close(&mut self, screen: ScreenId, sequence: u64) {
        if screen == self.id.screen && sequence > self.initial.receive_sequence {
            self.server_close_sequence.get_or_insert(sequence);
            if self.dispatched && self.requires_inspection.is_none() {
                self.stage = ContainerCloseStage::ObservedClosed;
            }
        }
    }
    pub(crate) fn context_received(
        &mut self,
        session: SessionStamp,
        mode: Option<super::GameMode>,
        screen: Option<ScreenId>,
        cursor: Option<&ObservedValue<SlotKnowledge>>,
    ) {
        if self.stage == ContainerCloseStage::Pending
            && (session != self.initial.session
                || mode != Some(self.mode)
                || screen != Some(self.id.screen)
                || !matches!(
                    cursor,
                    Some(ObservedValue {
                        value: SlotKnowledge::Empty,
                        source: super::ValueSource::Received { .. }
                    })
                ))
        {
            self.inspection(
                "close session/mode/opening/cursor context changed before complete dispatch",
            );
        }
    }
}
pub(crate) fn prepare_close(
    initial: super::PlayerObservation,
    screen: ContainerScreen,
    requested: ScreenId,
    mode: super::GameMode,
    previous: Option<&ContainerCloseRecord>,
) -> crate::Result<ContainerCloseRecord> {
    if !matches!(mode, super::GameMode::Survival | super::GameMode::Creative)
        || initial.game_mode != Some(mode)
        || initial.pending_dispatch
        || initial.session != requested.session()
        || screen.id != requested
        || initial.inventory.window_id != Some(requested.window_id())
        || !matches!(
            initial.inventory.cursor.as_ref(),
            Some(ObservedValue {
                value: SlotKnowledge::Empty,
                source: super::ValueSource::Received { .. },
            })
        )
    {
        return Err(super::inventory::unavailable(
            "close requires same live received opening, matching mode and received empty cursor",
        ));
    }
    if previous.is_some_and(|p| p.unresolved() || p.id.screen == requested) {
        return Err(super::inventory::unavailable(
            "close already retained; inspect without replay",
        ));
    }
    let attempt = previous
        .map_or(Some(1), |p| p.id.attempt.checked_add(1))
        .ok_or_else(|| super::inventory::unavailable("close attempts exhausted"))?;
    Ok(ContainerCloseRecord {
        id: ContainerCloseId {
            screen: requested,
            attempt,
        },
        initial,
        initial_screen: screen,
        mode,
        dispatched: false,
        server_close_sequence: None,
        requires_inspection: None,
        stage: ContainerCloseStage::Pending,
    })
}

/// Identity of one received screen opening. Serialized history cannot create a live handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ScreenId {
    session: SessionStamp,
    window: i32,
    opened_sequence: u64,
}
impl ScreenId {
    /// Owning transport and world.
    pub fn session(self) -> SessionStamp {
        self.session
    }
    /// Native numeric container ID; it may be reused by the server.
    pub fn window_id(self) -> i32 {
        self.window
    }
    /// Ordinal of the received opening, distinguishing reused numeric IDs.
    pub fn opened_sequence(self) -> u64 {
        self.opened_sequence
    }
}

/// Native title encoding. Item data and chat component formats are version-specific.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScreenTitle {
    /// Complete legacy chat JSON, not rendered text.
    LegacyJson {
        /// Native JSON string.
        json: String,
    },
    /// Complete modern unnamed NBT component, including the root tag byte.
    NativeNbt {
        /// Native encoded bytes.
        bytes: Vec<u8>,
    },
    /// Native opening supplied no title (for example a horse window).
    Unavailable,
}
/// One native screen slot referring to the player's main inventory or hotbar.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlayerSlotMapping {
    /// Slot in this container screen.
    pub screen_slot: usize,
    /// Canonical player-screen index (main 9..35, hotbar 36..44).
    pub player_slot: usize,
}
/// Layout independently obtained from the selected native menu constructor.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ScreenLayout {
    /// Exact number of slots expected in a full content packet.
    pub total_slots: usize,
    /// Native player slot mappings. No offset is inferred from packet length.
    pub player_slots: Vec<PlayerSlotMapping>,
}
/// One currently open, received container. Missing slots do not mean empty slots.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ContainerScreen {
    /// Session/world/opening-bound identity.
    pub id: ScreenId,
    /// Raw native menu registry ID, absent for special entity windows.
    pub native_menu_id: Option<i32>,
    /// Pinned native registry name, absent for unknown/custom menu IDs.
    pub menu_name: Option<String>,
    /// Actual title from the opening.
    pub title: ObservedValue<ScreenTitle>,
    /// Constructor-verified layout, absent when no native layout is established.
    pub layout: Option<ScreenLayout>,
    /// Received screen slots, including any appended player slots.
    pub slots: Vec<Option<ObservedValue<SlotKnowledge>>>,
    /// Last full content packet for this opening; individual updates do not set it.
    pub full_contents_sequence: Option<u64>,
    /// Received native state ID. Absent on protocols without that field.
    pub revision: Option<ObservedValue<i32>>,
}
/// Coherent screen and cursor capture; it grants no click/replay authority.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ScreenObservation {
    /// Owning connection/world at capture.
    pub session: SessionStamp,
    /// Current applied packet ordinal, not freshness for all fields.
    pub receive_sequence: u64,
    /// Active received window, unknown until established. Zero is the player screen.
    pub active_window: Option<i32>,
    /// Local player-screen access with received/submitted basis kept explicit.
    pub player_screen: Option<PlayerScreenAccess>,
    /// Metadata and actual contents of a received container opening.
    pub screen: Option<ContainerScreen>,
    /// Actual received cursor, independent of legacy full content packets.
    pub cursor: Option<ObservedValue<SlotKnowledge>>,
}

#[derive(Clone, serde::Deserialize)]
struct NativeMenu {
    native_id: i32,
    name: String,
    total_slots: Option<usize>,
    #[serde(default)]
    player_slots: Vec<NativePlayerSlot>,
}
#[derive(Clone, serde::Deserialize)]
struct NativePlayerSlot {
    screen_slot: usize,
    raw_player_slot: usize,
}
fn native_menu(version: MinecraftVersion, id: i32) -> Option<NativeMenu> {
    use std::sync::OnceLock;
    static LEGACY: OnceLock<Vec<NativeMenu>> = OnceLock::new();
    static MODERN: OnceLock<Vec<NativeMenu>> = OnceLock::new();
    let definitions = match version {
        MinecraftVersion::Java1_16_1 => LEGACY.get_or_init(|| {
            serde_json::from_str(include_str!("../../data/client_api/menus-1.16.1.json"))
                .expect("pinned native menus")
        }),
        MinecraftVersion::Java1_21_11 => MODERN.get_or_init(|| {
            serde_json::from_str(include_str!("../../data/client_api/menus-1.21.11.json"))
                .expect("pinned native menus")
        }),
    };
    definitions.iter().find(|m| m.native_id == id).cloned()
}

/// Adapter-owned opening receipts; no session identity is forged at packet decode time.
#[derive(Clone, Debug)]
pub(crate) struct ScreenReceipts {
    pub window: i32,
    pub opened_sequence: u64,
    pub native_menu_id: Option<i32>,
    pub menu_name: Option<String>,
    pub title: ObservedValue<ScreenTitle>,
    pub layout: Option<ScreenLayout>,
    pub slots: Vec<Option<ObservedValue<SlotKnowledge>>>,
    pub full_contents_sequence: Option<u64>,
    pub revision: Option<ObservedValue<i32>>,
}
impl ScreenReceipts {
    pub fn open(
        version: MinecraftVersion,
        window: i32,
        native_id: Option<i32>,
        title: ScreenTitle,
        sequence: u64,
    ) -> Self {
        let menu = native_id.and_then(|id| native_menu(version, id));
        let layout = menu.as_ref().and_then(|menu| {
            menu.total_slots.map(|total_slots| ScreenLayout {
                total_slots,
                player_slots: menu
                    .player_slots
                    .iter()
                    .filter_map(|m| match m.raw_player_slot {
                        0..=8 => Some(PlayerSlotMapping {
                            screen_slot: m.screen_slot,
                            player_slot: m.raw_player_slot + 36,
                        }),
                        9..=35 => Some(PlayerSlotMapping {
                            screen_slot: m.screen_slot,
                            player_slot: m.raw_player_slot,
                        }),
                        _ => None,
                    })
                    .collect(),
            })
        });
        Self {
            window,
            opened_sequence: sequence,
            native_menu_id: native_id,
            menu_name: menu.map(|m| m.name),
            title: received(title, sequence),
            slots: vec![None; layout.as_ref().map_or(0, |l| l.total_slots)],
            layout,
            full_contents_sequence: None,
            revision: None,
        }
    }
    pub fn full_items(
        &mut self,
        values: Vec<Option<ObservedValue<SlotKnowledge>>>,
        revision: Option<i32>,
        sequence: u64,
    ) -> anyhow::Result<()> {
        if self
            .layout
            .as_ref()
            .is_some_and(|layout| layout.total_slots != values.len())
        {
            anyhow::bail!("container content count differs from native menu layout");
        }
        self.slots = values;
        self.full_contents_sequence = Some(sequence);
        self.revision = revision.map(|r| received(r, sequence));
        Ok(())
    }
    pub fn slot(
        &mut self,
        index: usize,
        value: Option<ObservedValue<SlotKnowledge>>,
        revision: Option<i32>,
        sequence: u64,
    ) -> anyhow::Result<()> {
        let bound = self.layout.as_ref().map_or(4096, |l| l.total_slots);
        if index >= bound {
            anyhow::bail!("container slot outside native layout");
        }
        if self.slots.len() <= index {
            self.slots.resize(index + 1, None);
        }
        self.slots[index] = value;
        if let Some(r) = revision {
            self.revision = Some(received(r, sequence));
        }
        Ok(())
    }
    pub fn capture(&self, session: SessionStamp) -> ContainerScreen {
        ContainerScreen {
            id: ScreenId {
                session,
                window: self.window,
                opened_sequence: self.opened_sequence,
            },
            native_menu_id: self.native_menu_id,
            menu_name: self.menu_name.clone(),
            title: self.title.clone(),
            layout: self.layout.clone(),
            slots: self.slots.clone(),
            full_contents_sequence: self.full_contents_sequence,
            revision: self.revision.clone(),
        }
    }
}
