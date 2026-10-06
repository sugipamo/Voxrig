//! At-most-once container activation and separately received screen/content facts.
use super::*;
use crate::client::{
    BlockTargetHit, BlockTargetObservation, GameMode, PlayerObservation, ReceivedPose, ValueSource,
};
use crate::{NativeBlockState, Result};

/// One local activation intent; serialized history cannot recreate a live owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ContainerOpenId {
    session: SessionStamp,
    attempt: u64,
}
impl ContainerOpenId {
    /// Owning connection and world generation.
    pub fn session(self) -> SessionStamp {
        self.session
    }
    /// Monotonically increasing intent on this client.
    pub fn attempt(self) -> u64 {
        self.attempt
    }
}
/// Complete dispatch and actual screen facts are different boundaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerOpenStage {
    /// Before-I/O intent, including cancellation before a complete write.
    Pending,
    /// Complete activation frame written; opening is not yet established.
    Dispatched,
    /// Fresh matching screen OPEN received, but not complete contents/cursor/processing.
    ObservedScreen,
    /// Matching opening and complete contents/cursor received; modern processing also observed.
    /// The protocol does not identify the target block that produced this screen.
    ObservedContents,
    /// Conflict, unsent outcome, cancellation or unavailable context needs inspection.
    RequiresInspection,
}
/// Retained native activation dispatch, not a target-linked acknowledgement.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ContainerOpenSend {
    /// Last applied packet before the owned write.
    pub after_sequence: u64,
    /// Native processing sequence when this protocol supplies one.
    pub interaction_sequence: Option<i32>,
    /// True only after the complete frame was written.
    pub dispatched: bool,
}
/// Actual global native interaction processing; not a target or screen acknowledgement.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ContainerOpenProcessing {
    /// Native highest processed sequence, which may cover other interactions.
    pub acknowledged_sequence: i32,
    /// Packet ordinal of that actual native ACK.
    pub receive_sequence: u64,
}
/// Last inspected target state in the received world cache, not a block-linked ACK.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ContainerOpenTargetState {
    /// Actual cached native properties; outline/activation outcome remain distinct.
    pub state: NativeBlockState,
    /// Received world-cache revision at inspection.
    pub world_revision: u64,
    /// Packet ordinal at the capture boundary, not the last block-update ordinal.
    pub capture_sequence: u64,
}
/// Intent and facts for one empty-hand container activation on either mode handle.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ContainerOpenRecord {
    /// Local opaque attempt, retained before I/O.
    pub id: ContainerOpenId,
    /// Same-boundary player, inventory, mode and pose provenance.
    pub initial: PlayerObservation,
    /// Actual prior opening metadata, if retained after a local close.
    pub before_screen: Option<ScreenId>,
    /// Native first outline selected from the received world, not a server hit.
    pub target: BlockTargetHit,
    /// Latest inspected cache state, including normal native barrel open-flag changes.
    pub target_state: Option<ContainerOpenTargetState>,
    /// Received world cache revision at admission.
    pub world_revision: u64,
    /// Native standing eye at the same boundary.
    pub eye: [f64; 3],
    /// Mode checked against actual receive, not changed by this operation.
    pub mode: GameMode,
    /// Native menu expected from this block state; title does not prove ownership.
    pub expected_menu: String,
    /// Exact main-hand cursor derived from the native outline point.
    pub cursor: [f32; 3],
    /// Before-I/O and complete-write facts.
    pub send: ContainerOpenSend,
    /// Fresh actual matching screen, retained with its opening-bound identity.
    pub observed_screen: Option<ContainerScreen>,
    /// Actual received empty cursor for the new opening; never synthesized from send.
    pub received_cursor: Option<ObservedValue<SlotKnowledge>>,
    /// Honest legacy absence; modern actual global processing is separate from screen facts.
    pub protocol_processing: Option<ContainerOpenProcessing>,
    /// Persistent first uncertainty; restored values do not erase it.
    pub requires_inspection: Option<String>,
    /// Dispatch/receive boundary, not causal attribution of the screen to the target.
    pub stage: ContainerOpenStage,
}
pub(crate) fn empty(value: Option<&ObservedValue<SlotKnowledge>>) -> bool {
    matches!(
        value,
        Some(ObservedValue {
            value: SlotKnowledge::Empty,
            source: ValueSource::Received { .. }
        })
    )
}
fn menu(state: &NativeBlockState) -> Result<&'static str> {
    match state.name.as_str() {
        "minecraft:chest" | "minecraft:trapped_chest" => {
            match state.properties.get("type").map(String::as_str) {
                Some("single") => Ok("minecraft:generic_9x3"),
                Some("left" | "right") => Ok("minecraft:generic_9x6"),
                _ => Err(crate::client::inventory::unavailable(
                    "native chest type unavailable",
                )),
            }
        }
        "minecraft:barrel" | "minecraft:ender_chest" => Ok("minecraft:generic_9x3"),
        "minecraft:hopper" => Ok("minecraft:hopper"),
        "minecraft:dispenser" | "minecraft:dropper" => Ok("minecraft:generic_3x3"),
        "minecraft:crafting_table" => Ok("minecraft:crafting"),
        _ => Err(crate::client::inventory::unavailable(
            "native container opening not implemented",
        )),
    }
}
pub(crate) fn prepare_open(
    query: BlockTargetObservation,
    requested: [i32; 3],
    mode: GameMode,
    before_screen: Option<ScreenId>,
    previous: Option<&ContainerOpenRecord>,
    sequence: Option<i32>,
) -> Result<ContainerOpenRecord> {
    let initial = query.initial;
    let selected = initial
        .selected_hotbar
        .as_ref()
        .filter(|s| s.value <= 8)
        .map(|s| s.value)
        .ok_or_else(|| {
            crate::client::inventory::unavailable("ordered hand selection unavailable")
        })?;
    let hand = initial
        .inventory
        .slots
        .get(36 + usize::from(selected))
        .and_then(Option::as_ref);
    let offhand = initial.inventory.slots.get(45).and_then(Option::as_ref);
    if initial.pending_dispatch
        || initial.game_mode != Some(mode)
        || !matches!(mode, GameMode::Survival | GameMode::Creative)
        || initial.received_pose.is_none()
        || initial.dimension.is_none()
        || initial.inventory.player_screen.is_none()
        || !empty(hand)
        || !empty(offhand)
        || !empty(initial.inventory.cursor.as_ref())
    {
        return Err(crate::client::inventory::unavailable(
            "container open requires current player UI, received pose/world, matching mode, empty hands/cursor and no pending mutation",
        ));
    }
    if previous.is_some_and(ContainerOpenRecord::unresolved) {
        return Err(crate::client::inventory::unavailable(
            "storage activation retained; inspect without replay",
        ));
    }
    let target = query
        .hit
        .filter(|h| h.position == requested)
        .ok_or_else(|| {
            crate::client::inventory::unavailable(
                "requested storage is not the native first outline",
            )
        })?;
    if outline::lookup(initial.session.version, &target.state).is_none() {
        return Err(crate::client::inventory::unavailable(
            "target container shape/properties not audited for selected version",
        ));
    }
    let expected_menu = menu(&target.state)?.to_owned();
    let attempt = previous
        .map_or(Some(1), |p| p.id.attempt.checked_add(1))
        .ok_or_else(|| {
            crate::client::inventory::unavailable("storage activation attempts exhausted")
        })?;
    let cursor =
        std::array::from_fn(|i| (target.point[i] - f64::from(requested[i])).clamp(0.0, 1.0) as f32);
    Ok(ContainerOpenRecord {
        id: ContainerOpenId {
            session: initial.session,
            attempt,
        },
        before_screen,
        target,
        target_state: None,
        world_revision: query.world_revision,
        eye: query.eye,
        mode,
        expected_menu,
        cursor,
        send: ContainerOpenSend {
            after_sequence: initial.receive_sequence,
            interaction_sequence: sequence,
            dispatched: false,
        },
        initial,
        observed_screen: None,
        received_cursor: None,
        protocol_processing: None,
        requires_inspection: None,
        stage: ContainerOpenStage::Pending,
    })
}
pub(crate) struct OpenContext<'a> {
    pub session: SessionStamp,
    pub mode: Option<GameMode>,
    pub position: Option<[f64; 3]>,
    pub rotation: [f32; 2],
    pub pose: Option<&'a ReceivedPose>,
    pub selected: Option<u8>,
    pub hand: Option<&'a ObservedValue<SlotKnowledge>>,
    pub offhand: Option<&'a ObservedValue<SlotKnowledge>>,
    pub cursor: Option<&'a ObservedValue<SlotKnowledge>>,
    pub player_screen: Option<PlayerScreenAccess>,
    pub screen: Option<&'a ContainerScreen>,
}
impl ContainerOpenRecord {
    pub(crate) fn observe_target_state(
        &mut self,
        state: &NativeBlockState,
        world_revision: u64,
        capture_sequence: u64,
    ) -> bool {
        self.target_state = Some(ContainerOpenTargetState {
            state: state.clone(),
            world_revision,
            capture_sequence,
        });
        if state == &self.target.state {
            return true;
        }
        // Native BarrelBlockEntity changes OPEN while opening/closing. This flag
        // does not change its audited outline or menu. Only after complete dispatch;
        // no facing/name/property guesses and no attribution to this particular player.
        self.send.dispatched
            && state.name == "minecraft:barrel"
            && self.target.state.name == state.name
            && outline::lookup(self.id.session.version, state).is_some()
            && state.properties.len() == self.target.state.properties.len()
            && self.target.state.properties.iter().all(|(key, value)| {
                if key == "open" {
                    matches!(value.as_str(), "false" | "true")
                        && state
                            .properties
                            .get(key)
                            .is_some_and(|v| matches!(v.as_str(), "false" | "true"))
                } else {
                    state.properties.get(key) == Some(value)
                }
            })
    }
    pub(crate) fn unresolved(&self) -> bool {
        self.stage != ContainerOpenStage::ObservedContents
    }
    pub(crate) fn inspection(&mut self, reason: impl std::fmt::Display) {
        self.requires_inspection
            .get_or_insert_with(|| reason.to_string());
        self.stage = ContainerOpenStage::RequiresInspection;
    }
    pub(crate) fn sent(&mut self) {
        self.send.dispatched = true;
        if self.requires_inspection.is_none() {
            self.stage = ContainerOpenStage::Dispatched;
        }
    }
    pub(crate) fn context_received(&mut self, now: OpenContext<'_>) {
        if !self.unresolved() || self.requires_inspection.is_some() {
            return;
        }
        let new_screen = now.screen.filter(|s| {
            Some(s.id) != self.before_screen && s.id.opened_sequence() > self.send.after_sequence
        });
        // Modern OPEN invalidates canonical player slots. Missing knowledge at
        // that boundary is not a changed hand, nor an invented empty receipt.
        let hand_ok = |value: Option<&ObservedValue<SlotKnowledge>>| {
            empty(value) || (self.send.dispatched && new_screen.is_some() && value.is_none())
        };
        if now.session != self.initial.session
            || now.mode != Some(self.mode)
            || now.position != self.initial.position.as_ref().map(|p| p.value)
            || now.rotation != self.initial.rotation
            || now
                .pose
                .zip(self.initial.received_pose.as_ref())
                .is_none_or(|(actual, before)| {
                    actual.position != before.position || actual.rotation != before.rotation
                })
            || now.selected != self.initial.selected_hotbar.as_ref().map(|s| s.value)
            || !hand_ok(now.hand)
            || !hand_ok(now.offhand)
        {
            self.inspection("storage activation pose/session/mode/selected hands changed");
            return;
        }
        if let Some(screen) = new_screen {
            if !self.send.dispatched
                || screen.id.session() != self.initial.session
                || screen.menu_name.as_deref() != Some(self.expected_menu.as_str())
                || screen.layout.is_none()
                || self
                    .observed_screen
                    .as_ref()
                    .is_some_and(|s| s.id != screen.id)
            {
                // Retain even an unexpected OPEN as a diagnostic, not accepted outcome.
                if self.observed_screen.is_none() {
                    self.observed_screen = Some(screen.clone());
                }
                self.inspection("unsent, unexpected or replaced native opening");
                return;
            }
            self.observed_screen = Some(screen.clone());
            self.stage = ContainerOpenStage::ObservedScreen;
            if now.cursor.is_some() && !empty(now.cursor) {
                self.inspection("opened screen cursor not received empty");
                return;
            }
            if empty(now.cursor) {
                self.received_cursor = now.cursor.cloned();
            }
            let complete=screen.full_contents_sequence.is_some_and(|s|s>screen.id.opened_sequence())
                && screen.layout.as_ref().is_some_and(|l|l.total_slots==screen.slots.len())
                && screen.slots.iter().all(|s|matches!(s,Some(ObservedValue {source:ValueSource::Received {..},..})))
                && self.received_cursor.as_ref().is_some_and(|c|matches!(c.source,ValueSource::Received {sequence} if sequence>screen.id.opened_sequence()))
                && self.send.interaction_sequence.is_none_or(|sequence|self.protocol_processing.as_ref()
                    .is_some_and(|ack|ack.acknowledged_sequence>=sequence && ack.receive_sequence>self.send.after_sequence));
            if complete {
                self.stage = ContainerOpenStage::ObservedContents;
            }
        } else if self.observed_screen.is_some()
            || now.player_screen != self.initial.inventory.player_screen
            || !empty(now.cursor)
        {
            self.inspection("player UI/cursor or observed opening changed before complete outcome");
        }
    }
}
