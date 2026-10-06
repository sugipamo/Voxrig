//! Version-specific player construction controls. Sending is not server acceptance.
//! Static dry-cube standing and bounded survival controls have explicit admission.
//! General locomotion/pathfinding and complex item components are not inferred.
//! Mining removal alone does not authorize further mutations on that connection.
pub(super) mod container;
mod flight;
mod geometry;
pub(super) mod inventory;
pub(super) mod mining;
mod movement;
pub(super) mod placement;
mod profile_recovery;
mod recovery;
mod retirement;
mod survival;
pub(super) mod vehicle;
pub use movement::{
    AssumedSurvivalScene, AssumedSurvivalStart, CapturedSurvivalScene, HypotheticalAimRequirement,
    HypotheticalBlockEdit, HypotheticalMovementPreview, HypotheticalPlacement,
    HypotheticalReconnectBoundary, HypotheticalSceneSource, MAX_SURVIVAL_CONTROL_TICKS,
    PredictedMotionFrame, StandingPositionBasis, SurvivalControl, SurvivalInput,
    SurvivalMotionContract, SurvivalMotionRecheck, SurvivalMotionRecord, SurvivalMotionStatus,
    SurvivalMovementPreview, SurvivalScenario, TerminalClearance,
};
#[cfg(test)]
mod component_tests;
#[cfg(test)]
mod tests;
pub use super::loading::{InteractionLoading, LoadingAttempt};
pub use super::motion::{OwnMotion, PositionBasis, PositionSubmission, ReceivedPose};
use super::*;
use crate::diagnostic_projection::diagnostic_record;
pub use inventory::{InventorySwap, InventorySwapObservation};
pub use inventory::{RecordedInventorySwap, RecordedInventorySwapObservation};
pub use mining::{
    MiningIntent, MiningInventoryChange, MiningInventoryChangeKind, MiningRecord, MiningRemoval,
    MiningSend, MiningStatus, MiningTargetReceipt,
};
pub use mining::{
    RecordedMiningIntent, RecordedMiningInventoryChange, RecordedMiningRecord,
    RecordedMiningRemoval, RecordedMiningStatus,
};
pub(super) use mining::{mining_chunk_changed, mining_received, mining_world_changed};
pub use movement::RecordedSurvivalMovementPreview;
pub use movement::{
    RecordedHypotheticalAimRequirement, RecordedHypotheticalBlockEdit,
    RecordedHypotheticalMovementPreview, RecordedHypotheticalPlacement,
    RecordedHypotheticalReconnectBoundary, RecordedHypotheticalSceneSource,
    RecordedSurvivalMotionRecheck, RecordedSurvivalMotionRecord,
};
pub use placement::{PlacementIntent, PlacementObservation, PlacementRecord, PlacementStatus};
pub use placement::{
    RecordedPlacementIntent, RecordedPlacementObservation, RecordedPlacementRecord,
    RecordedPlacementStatus,
};
pub(super) use placement::{
    placement_chunk_changed, placement_context_received, placement_received,
};
pub use profile_recovery::MiningProfileRecoveryWatch;
pub use recovery::{
    MiningRecovery, MiningRecoveryAttempt, MiningRecoveryBoundary, MiningRecoveryEvidence,
    MiningRecoveryMethod, MiningRecoveryTarget,
};
pub use recovery::{RecordedMiningRecoveryBoundary, RecordedMiningRecoveryEvidence};
pub(super) use retirement::retirement_received;
pub use retirement::{MiningRetirementRecord, MiningRetirementStatus, MiningRetirementWatch};
pub use retirement::{RecordedMiningRetirementRecord, RecordedMiningRetirementWatch};
use serde::Serialize;
pub use survival::RecordedStandingContext;
pub use survival::{
    AttributeValue, LocalPlayerState, MotionInterruption, PlayerHealth, ReceivedEffect,
    StandingContext, ValueBasis, VelocitySample,
};

pub use crate::client::GameMode;

/// A default item stack, without added or removed data components.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, serde::Deserialize)]
pub struct PlainItem {
    /// Registry identifier, including namespace.
    pub name: String,
    /// Native item ID, never a block-state ID.
    pub item_id: i32,
    /// Native stack count.
    pub count: i32,
}

/// Validate a component-free stack against the pinned native item registry,
/// without connecting or mutating inventory. Useful for preflighting a batch.
pub fn default_item(name: &str, count: u8) -> Result<PlainItem> {
    let native = name.strip_prefix("minecraft:").unwrap_or(name);
    let definition = items()
        .iter()
        .find(|i| i.name == native)
        .ok_or_else(|| invalid("unknown Java 1.21.11 item"))?;
    if count == 0 || i32::from(count) > definition.stack_size {
        return Err(invalid("invalid default item stack count"));
    }
    Ok(PlainItem {
        name: format!("minecraft:{native}"),
        item_id: definition.id,
        count: i32::from(count),
    })
}
crate::diagnostic_projection::diagnostic_record! {
/// Received inventory knowledge; unknown never means an empty slot.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InventorySlot => RecordedInventorySlot {
    /// No complete supported packet has established this slot.
    Unavailable,
    /// A native packet declared this slot empty.
    Empty,
    /// A component-free default stack was received.
    Item {
        /// Exact item identity and count.
        item: PlainItem,
    },
    /// A received stack with a complete supported native component patch.
    ItemWithComponents {
        /// Native item identity/count; prototype data is not flattened into this value.
        item: PlainItem,
        /// Exact supported values/removals, distinct from a default stack.
        components: crate::client::ItemComponentPatch,
    },
}
    diagnostic_serde { #[serde(tag = "kind", rename_all = "snake_case")] }
}

diagnostic_record! {
    /// Received inventory contents; unsupported components invalidate the affected baseline.
    #[derive(Clone, Debug, Serialize)]
    pub struct Inventory => RecordedInventory {
        /// Player screen slots 0..45 (hotbar 36..44).
        pub slots: Vec<InventorySlot>,
        /// Most recent applied native inventory packet sequence.
        pub receive_sequence: Option<u64>,
        /// Some component encodings require additional version-specific decoders.
        pub unsupported_components: bool,
        /// Submitted creative hotbar writes still awaiting a native inventory update.
        pub pending_creative: Vec<u8>,
        /// Active received container, unknown until an inventory/window packet arrives.
        pub window_id: Option<i32>,
        /// Latest received player-screen revision, never incremented from a submitted click.
        pub screen_revision: Option<i32>,
        /// Received carried stack. Unavailable never means an empty cursor.
        pub cursor: InventorySlot,
        /// An unresolved ordinary swap. Cancellation/timeout never clears its uncertainty.
        pub pending_swap: Option<InventorySwap>,
    }
    native_only {
        #[serde(skip)]
        slot_sequences: Vec<Option<u64>>,
        #[serde(skip)]
        cursor_sequence: Option<u64>,
        #[serde(skip)]
        pub(crate) container: Option<crate::client::container::ScreenReceipts>,
        #[serde(skip)]
        player_revision: Option<crate::client::ObservedValue<i32>>,
    }
    diagnostic_serde {}
}
/// Selected main-hand hotbar slot with explicit receive/submission provenance.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, serde::Deserialize)]
pub struct HotbarSelection {
    /// Native hotbar index 0..8.
    pub slot: u8,
    /// Receive boundary before the ordered selection send, or the selection packet.
    pub sequence: u64,
    /// False for an interrupted local selection attempt.
    pub dispatched: bool,
    /// True for a server-supplied selection; a local send is not an acknowledgement.
    pub from_server: bool,
}
impl Default for Inventory {
    fn default() -> Self {
        Self {
            slots: vec![InventorySlot::Unavailable; 46],
            receive_sequence: None,
            unsupported_components: false,
            pending_creative: Vec::new(),
            window_id: None,
            screen_revision: None,
            cursor: InventorySlot::Unavailable,
            pending_swap: None,
            slot_sequences: vec![None; 46],
            cursor_sequence: None,
            container: None,
            player_revision: None,
        }
    }
}
diagnostic_record! {
    /// A session-bound player/inventory observation; position may include submitted movement.
    #[derive(Clone, Debug, Serialize)]
    pub struct PlayerState => RecordedPlayerState {
        /// Native loading stage; complete dispatch is not an operation acknowledgement.
        pub interaction_loading: InteractionLoading,
        /// Session identity, not reusable across connections.
        pub connection_id: u64,
        /// Last applied receive sequence.
        pub receive_sequence: u64,
        /// Current dimension, absent during reconfiguration.
        pub dimension: Option<String>,
        /// Current feet position.
        pub position: Option<[f64; 3]>,
        /// False after local movement until a server position packet replaces it.
        pub position_from_server: bool,
        /// Separate position receipts and local submissions, with retained provenance.
        pub motion: OwnMotion,
        /// Yaw and pitch in native degrees.
        pub rotation: [f32; 2],
        /// Latest received game mode.
        pub game_mode: Option<GameMode>,
        /// Server permission to fly; false until received.
        pub may_fly: bool,
        /// Latest client flight request, not an acknowledgement.
        pub requested_flying: bool,
        /// Highest received block interaction acknowledgement.
        pub acknowledged_interaction: Option<i32>,
        /// Inventory as received, never filled from submitted creative packets.
        pub inventory: Inventory,
        /// Main-hand selection, absent until explicitly sent or received.
        pub selected_hotbar: Option<HotbarSelection>,
        /// Own-player defaults and received updates, with explicit provenance.
        pub local_player: LocalPlayerState,
        /// Configuration flags actually received, absent until observed.
        pub enabled_features: Option<Vec<String>>,
        /// Last periodic server-time sample; never a current-tick fence.
        pub server_time: Option<ServerTime>,
    }
    diagnostic_serde {}
}
diagnostic_record! {
    /// Diagnostic history, available even after closure. These records are not a
    /// current player/world observation or permission to replay an action.
    #[derive(Clone, Debug, Serialize)]
    pub struct OperationHistory => RecordedOperationHistory {
        /// Owning connection; never reusable on a replacement connection.
        pub connection_id: u64,
        /// Last applied receive ordinal, not a fresh observation fence.
        pub last_receive_sequence: u64,
        /// True when the connection can no longer be used for operations.
        pub connection_closed: bool,
        /// First attempted packet whose frame completion became uncertain.
        /// Does not prove how many bytes or which server effects occurred.
        pub interrupted_packet_id: Option<i32>,
        /// Most recent protocol/receive failure, if retained.
        pub receive_failure: Option<String>,
        /// An unresolved ordinary inventory swap; do not replay from this history.
        pub pending_inventory_swap: Option<InventorySwap>,
        /// Creative slots awaiting a received result, including interrupted sends.
        pub pending_creative_slots: Vec<u8>,
        /// Last mining intent/result, pending or observed. Never replay from history.
        pub mining: Option<MiningRecord>,
        /// Last ordinary placement, including unresolved sends and observed consumption.
        pub placement: Option<PlacementRecord>,
        /// Last independent retirement watch; history does not authorize recovery.
        pub mining_retirement: Option<MiningRetirementRecord>,
        /// Native loading attempts survive failure and remain available as history.
        pub interaction_loading: InteractionLoading,
        /// Last main-hand selection evidence, including incomplete send attempts.
        pub selected_hotbar: Option<HotbarSelection>,
        /// Position receipt/submission history; never replayable authority.
        pub motion: OwnMotion,
        /// Bounded survival control intent and observations; never replayable authority.
        pub survival_motion: Option<SurvivalMotionRecord>,
    }
    diagnostic_serde {}
}
/// A periodic native time packet bound to its receive sequence.
#[derive(Clone, Debug, Serialize, serde::Deserialize, PartialEq)]
pub struct ServerTime {
    /// Received game age.
    pub game_age: i64,
    /// Received day time.
    pub day_time: i64,
    /// Packet receive sequence.
    pub receive_sequence: u64,
}
/// A bounded system-message record, preserving native translation keys/arguments.
#[derive(Clone, Debug, Serialize)]
pub struct SystemMessage {
    /// Packet receive sequence.
    pub receive_sequence: u64,
    /// Parsed native component. None means unavailable, never an empty message.
    pub component: Option<serde_json::Value>,
    /// Whether this was an overlay message.
    pub overlay: bool,
}
impl SystemMessage {
    /// Exact literal text, excluding translated or concatenated components.
    pub fn literal_text(&self) -> Option<&str> {
        let c = self.component.as_ref()?;
        if let Some(text) = c.as_str() {
            return Some(text);
        }
        if c.get("translate").is_some()
            || c.get("extra")
                .is_some_and(|v| v.as_array().is_none_or(|a| !a.is_empty()))
        {
            return None;
        }
        c.get("text")?.as_str()
    }
}
#[derive(Clone, Default)]
pub(super) struct OperationState {
    pub features: Option<Vec<String>>,
    server_time: Option<ServerTime>,
    messages: std::collections::VecDeque<Arc<SystemMessage>>,
    messages_dropped_through: u64,
    game_mode: Option<GameMode>,
    abilities: Option<u8>,
    abilities_sequence: Option<u64>,
    requested_flying: bool,
    ack: Option<i32>,
    ack_receive_sequence: Option<u64>,
    inventory: Inventory,
    selected_hotbar: Option<HotbarSelection>,
    pub(super) local_player: LocalPlayerState,
}
impl OperationState {
    pub(super) fn abilities_receipt(&self) -> Option<crate::client::ObservedValue<u8>> {
        self.abilities
            .zip(self.abilities_sequence)
            .map(|(flags, sequence)| crate::client::received(flags, sequence))
    }
    pub fn reset_configuration(&mut self, sequence: u64) {
        *self = Self {
            messages_dropped_through: sequence,
            ..Self::default()
        };
    }
    pub fn reset_world(&mut self, game_mode: u8) -> anyhow::Result<()> {
        let local_player = self.local_player.reset_world();
        let features = self.features.take();
        let messages = std::mem::take(&mut self.messages);
        let messages_dropped_through = self.messages_dropped_through;
        *self = Self {
            features,
            messages,
            messages_dropped_through,
            game_mode: Some(GameMode::decode(game_mode)?),
            local_player,
            ..Self::default()
        };
        Ok(())
    }
}

/// A handle to operations on one explicitly selected Java 1.21.11 connection.
#[derive(Clone)]
pub struct Operations {
    pub(super) bot: Bot,
}
impl Operations {
    pub(crate) async fn common_screen_state(
        &self,
    ) -> Result<crate::client::container::ScreenObservation> {
        let state = self.bot.session.state.lock().await;
        let player = self.common_player_unlocked(&state)?;
        Ok(crate::client::container::ScreenObservation {
            session: player.session,
            receive_sequence: state.sequence,
            active_window: state.operations.inventory.window_id,
            player_screen: player.inventory.player_screen,
            screen: state
                .operations
                .inventory
                .container
                .as_ref()
                .map(|s| s.capture(player.session)),
            cursor: player.inventory.cursor,
        })
    }
    /// Inspect unresolved operation history without sending, reconnecting or
    /// checking for a live connection. Never turns stale data into action authority.
    pub async fn operation_history(&self) -> OperationHistory {
        let state = self.bot.session.state.lock().await;
        let closed = self.bot.session.stopped.load(Ordering::Acquire);
        let interrupted = self.bot.session.interrupted_packet.load(Ordering::Acquire);
        OperationHistory {
            motion: state.motion.clone(),
            survival_motion: state.survival_motion.clone(),
            connection_id: self.bot.session.id,
            last_receive_sequence: state.sequence,
            connection_closed: closed || interrupted >= 0,
            interrupted_packet_id: (interrupted >= 0).then_some(interrupted),
            receive_failure: state.failure.as_ref().map(ToString::to_string),
            pending_inventory_swap: state.operations.inventory.pending_swap.clone(),
            pending_creative_slots: state.operations.inventory.pending_creative.clone(),
            mining: state.mining.clone(),
            placement: state.placement.clone(),
            mining_retirement: state.retirement.clone(),
            interaction_loading: state.loading.clone(),
            selected_hotbar: state.operations.selected_hotbar.clone(),
        }
    }
    /// Observe other spawned players, without entity physics or render interpolation.
    pub async fn visible_players(&self) -> Result<super::players::PlayerObservations> {
        let state = self.bot.session.state.lock().await;
        self.ready(&state)?;
        Ok(super::players::PlayerObservations {
            connection_id: self.bot.session.id,
            receive_sequence: state.sequence,
            dimension: state
                .world
                .dimension
                .as_ref()
                .expect("ready dimension")
                .0
                .clone(),
            players: state.players.observations(),
        })
    }
    /// Observe player state and native inventory under the same session lock.
    pub async fn player_state(&self) -> Result<PlayerState> {
        let state = self.bot.session.state.lock().await;
        self.bot.session.check(&state)?;
        Ok(PlayerState {
            interaction_loading: state.loading.clone(),
            connection_id: self.bot.session.id,
            receive_sequence: state.sequence,
            dimension: state.world.dimension.as_ref().map(|d| d.0.clone()),
            position: state.position,
            position_from_server: state
                .motion
                .received_position(state.loading.generation, state.position),
            motion: state.motion.clone(),
            rotation: state.rotation,
            game_mode: state.operations.game_mode,
            may_fly: state.operations.abilities.is_some_and(|a| a & 4 != 0),
            requested_flying: state.operations.requested_flying,
            acknowledged_interaction: state.operations.ack,
            inventory: state.operations.inventory.clone(),
            selected_hotbar: state.operations.selected_hotbar.clone(),
            local_player: state.operations.local_player.clone(),
            enabled_features: state.operations.features.clone(),
            server_time: state.operations.server_time.clone(),
        })
    }
    /// Read system messages after a sequence. Overflow returns an error instead
    /// of presenting a partial stream as complete command evidence.
    pub async fn system_messages_after(&self, sequence: u64) -> Result<Vec<SystemMessage>> {
        let state = self.bot.session.state.lock().await;
        self.ready(&state)?;
        if sequence < state.operations.messages_dropped_through {
            return Err(invalid("system message history was truncated"));
        }
        if sequence > state.sequence {
            return Err(invalid("message cursor is ahead of receive state"));
        }
        Ok(state
            .operations
            .messages
            .iter()
            .filter(|m| m.receive_sequence > sequence)
            .map(|m| (**m).clone())
            .collect())
    }
    /// Send an unsigned command on an offline connection. Server permissions still apply.
    /// No leading slash; dispatch does not imply command success.
    pub async fn send_command(&self, command: &str) -> Result<()> {
        if command.is_empty()
            || command.starts_with('/')
            || command.chars().any(char::is_control)
            || command.encode_utf16().count() > 32767
        {
            return Err(invalid("invalid command text"));
        }
        let state = self.bot.session.state.lock().await;
        self.mutable(&state)?;
        let mut payload = Vec::new();
        put_string(&mut payload, command);
        self.bot
            .session
            .send(ids::play_serverbound::CHAT_COMMAND, &payload)
            .await?;
        Ok(())
    }
    /// Request flight only when the server advertises the ability.
    pub async fn set_flying(&self, flying: bool) -> Result<()> {
        self.set_flying_in_mode(None, flying).await
    }
    async fn set_flying_in_mode(&self, mode: Option<GameMode>, flying: bool) -> Result<()> {
        let mut state = self.bot.session.state.lock().await;
        self.mutable(&state)?;
        self.require_mode(&state, mode)?;
        if flying && !state.operations.abilities.is_some_and(|a| a & 4 != 0) {
            return Err(invalid("server has not granted flight"));
        }
        self.bot
            .session
            .send(
                ids::play_serverbound::ABILITIES,
                &[if flying { 2 } else { 0 }],
            )
            .await?;
        state.operations.requested_flying = flying;
        Ok(())
    }
    /// Submit a short creative flight step, with rotation in degrees.
    /// This does not implement collision resolution or server-confirmed teleportation.
    pub async fn move_flying(&self, position: [f64; 3], rotation: [f32; 2]) -> Result<()> {
        self.move_flying_in_mode(None, position, rotation).await
    }
    async fn move_flying_in_mode(
        &self,
        mode: Option<GameMode>,
        position: [f64; 3],
        rotation: [f32; 2],
    ) -> Result<()> {
        validate_pose(position, rotation)?;
        let mut state = self.bot.session.state.lock().await;
        self.mutable(&state)?;
        self.require_mode(&state, mode)?;
        if !state.operations.requested_flying
            || !state.operations.abilities.is_some_and(|a| a & 4 != 0)
        {
            return Err(invalid("flight must be permitted and requested"));
        }
        let before = state.position.context("position unavailable")?;
        if before
            .iter()
            .zip(position)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
            > 16.0
        {
            return Err(invalid("flight step exceeds four blocks"));
        }
        let mut payload = Vec::new();
        for v in position {
            payload.extend(v.to_be_bytes());
        }
        for v in rotation {
            payload.extend(v.to_be_bytes());
        }
        payload.push(0);
        let generation = state.loading.generation;
        let sequence = state.sequence;
        state
            .motion
            .begin(generation, sequence, position, rotation)?;
        self.bot
            .session
            .send(ids::play_serverbound::POSITION_LOOK, &payload)
            .await?;
        state.position = Some(position);
        state.rotation = rotation;
        state.motion.dispatched();
        Ok(())
    }
    /// Submit a view direction in native degrees, without changing position.
    /// Survival rechecks stationary standing geometry to derive the ground bit;
    /// unavailable or unsupported context refuses before sending or changing rotation.
    pub async fn look(&self, rotation: [f32; 2]) -> Result<()> {
        self.look_in_mode(None, rotation).await
    }
    async fn look_in_mode(&self, mode: Option<GameMode>, rotation: [f32; 2]) -> Result<()> {
        validate_pose([0.0; 3], rotation)?;
        let mut state = self.bot.session.state.lock().await;
        self.mutable(&state)?;
        self.require_mode(&state, mode)?;
        let mut payload = Vec::new();
        for v in rotation {
            payload.extend(v.to_be_bytes());
        }
        payload.push(survival::look_flags(
            &mut state,
            self.bot.session.started.elapsed().as_millis() as u64 / 50,
        )?);
        self.bot
            .session
            .send(ids::play_serverbound::LOOK, &payload)
            .await?;
        state.rotation = rotation;
        Ok(())
    }
    /// Submit a default item to a creative hotbar slot (0..8). No inventory echo is invented.
    pub async fn set_creative_hotbar(&self, slot: u8, item: Option<(&str, u8)>) -> Result<()> {
        if slot > 8 {
            return Err(invalid("hotbar slot must be 0..8"));
        }
        let mut payload = (36 + i16::from(slot)).to_be_bytes().to_vec();
        if let Some((name, count)) = item {
            let item = default_item(name, count)?;
            put_varint(&mut payload, item.count);
            put_varint(&mut payload, item.item_id);
            payload.extend([0, 0]);
        } else {
            payload.push(0);
        }
        let mut state = self.bot.session.state.lock().await;
        self.creative(&state)?;
        state.operations.inventory.slots[36 + usize::from(slot)] = InventorySlot::Unavailable;
        if !state.operations.inventory.pending_creative.contains(&slot) {
            state.operations.inventory.pending_creative.push(slot);
        }
        self.bot
            .session
            .send(ids::play_serverbound::SET_CREATIVE_SLOT, &payload)
            .await?;
        Ok(())
    }
    /// Submit held hotbar selection. The server still decides which item is present.
    pub async fn select_hotbar(&self, slot: u8) -> Result<()> {
        self.select_hotbar_in_mode(None, slot).await
    }
    async fn select_hotbar_in_mode(&self, mode: Option<GameMode>, slot: u8) -> Result<()> {
        if slot > 8 {
            return Err(invalid("hotbar slot must be 0..8"));
        }
        let mut state = self.bot.session.state.lock().await;
        self.mutable(&state)?;
        self.require_mode(&state, mode)?;
        if state.operations.inventory.pending_swap.is_some() {
            return Err(Error::new(
                ErrorKind::State,
                anyhow::anyhow!("inventory swap needs inspection"),
            ));
        }
        state.operations.selected_hotbar = Some(HotbarSelection {
            slot,
            sequence: state.sequence,
            dispatched: false,
            from_server: false,
        });
        self.bot
            .session
            .send(
                ids::play_serverbound::HELD_ITEM_SLOT,
                &i16::from(slot).to_be_bytes(),
            )
            .await?;
        state
            .operations
            .selected_hotbar
            .as_mut()
            .expect("selection attempt")
            .dispatched = true;
        Ok(())
    }
    /// Start an ordinary creative block break. Returned sequence identifies submission only.
    pub async fn dig_creative(&self, position: [i32; 3], face: crate::BlockFace) -> Result<i32> {
        let state = self.bot.session.state.lock().await;
        self.creative(&state)?;
        check_reach(&state, position)?;
        let seq = self.next_sequence()?;
        let mut payload = vec![0];
        payload.extend(pack_position(position).to_be_bytes());
        payload.push(face as u8);
        put_varint(&mut payload, seq);
        self.bot
            .session
            .send(ids::play_serverbound::BLOCK_DIG, &payload)
            .await?;
        Ok(seq)
    }
    /// Use the held item against an exact face hit. Placement and activation share this packet.
    /// Cursor coordinates are local to the target cell, each within 0..1.
    pub async fn use_on_block(
        &self,
        position: [i32; 3],
        face: crate::BlockFace,
        cursor: [f32; 3],
    ) -> Result<i32> {
        if cursor
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return Err(invalid("invalid block hit"));
        }
        let state = self.bot.session.state.lock().await;
        self.mutable(&state)?;
        if state.operations.inventory.pending_swap.is_some() {
            return Err(Error::new(
                ErrorKind::State,
                anyhow::anyhow!("inventory swap needs inspection"),
            ));
        }
        if state.operations.game_mode == Some(GameMode::Survival) {
            return Err(invalid(
                "survival placement requires place_survival_cube and received material accounting",
            ));
        }
        check_reach(&state, position)?;
        let seq = self.next_sequence()?;
        let mut payload = vec![0];
        payload.extend(pack_position(position).to_be_bytes());
        put_varint(&mut payload, face as i32);
        for v in cursor {
            payload.extend(v.to_be_bytes());
        }
        payload.extend([0, 0]);
        put_varint(&mut payload, seq);
        self.bot
            .session
            .send(ids::play_serverbound::BLOCK_PLACE, &payload)
            .await?;
        Ok(seq)
    }
    pub(super) fn ready(&self, state: &State) -> Result<()> {
        self.bot.session.check(state)?;
        if !state.ready {
            return Err(invalid("play state is not ready"));
        }
        Ok(())
    }
    pub(super) fn mutable(&self, state: &State) -> Result<()> {
        self.mutable_with_ground_requirement(state, true)
    }
    fn mutable_for_dismount(&self, state: &State) -> Result<()> {
        self.mutable_with_ground_requirement(state, false)
    }
    fn mutable_with_ground_requirement(&self, state: &State, ground: bool) -> Result<()> {
        self.mutable_with_flight_owner(state, ground, false)
    }
    fn mutable_with_flight_owner(
        &self,
        state: &State,
        ground: bool,
        flight_owner: bool,
    ) -> Result<()> {
        self.ready(state)?;
        if !flight_owner && crate::client::flight::unresolved(&state.flight_history) {
            return Err(crate::client::inventory::unavailable(
                "flight dispatch unresolved; inspect without replay",
            ));
        }
        if state
            .dismount_history
            .lock()
            .expect("dismount history")
            .as_ref()
            .is_some_and(|r| r.unresolved())
        {
            return Err(crate::client::inventory::unavailable(
                "dismount input unresolved; inspect and explicitly complete without replay",
            ));
        }
        if state
            .common_container_open
            .as_ref()
            .is_some_and(|o| o.unresolved())
        {
            return Err(crate::client::inventory::unavailable(
                "common storage activation unresolved; inspect without replay",
            ));
        }
        if state
            .common_container_close
            .as_ref()
            .is_some_and(|r| r.unresolved())
        {
            return Err(crate::client::inventory::unavailable(
                "common container close unresolved; inspect without replay",
            ));
        }
        if state.common_inventory_swap.as_ref().is_some_and(|s| {
            s.record.stage != crate::client::inventory::InventorySwapStage::ObservedSwapped
        }) {
            return Err(crate::client::inventory::unavailable(
                "common inventory swap unresolved; inspect retained record without replay",
            ));
        }
        if state
            .common_inventory_click
            .as_ref()
            .is_some_and(|s| s.unresolved())
        {
            return Err(crate::client::inventory::unavailable(
                "common inventory click unresolved; inspect without replay",
            ));
        }
        if state
            .common_crafting_take
            .as_ref()
            .is_some_and(|r| r.unresolved())
        {
            return Err(crate::client::inventory::unavailable(
                "common crafting take unresolved; inspect without replay",
            ));
        }
        if state
            .common_inventory_transfer
            .as_ref()
            .is_some_and(|s| s.unresolved())
        {
            return Err(crate::client::inventory::unavailable(
                "common inventory transfer unresolved; inspect without replay",
            ));
        }
        if !state.loading.notification_dispatched() {
            return Err(Error::new(
                ErrorKind::State,
                anyhow::anyhow!(
                    "native interaction loading pending; await readiness or inspect retained loading attempt"
                ),
            ));
        }
        if state
            .survival_motion
            .as_ref()
            .is_some_and(|r| !r.status.is_continuation_candidate())
        {
            return Err(invalid(
                "survival motion unresolved; inspect retained run before another mutation",
            ));
        }
        if ground && state.survival_motion.is_some() {
            movement::standing_basis(state)?;
        }
        if state.motion.position_basis == PositionBasis::PendingSubmission {
            return Err(Error::new(
                ErrorKind::State,
                anyhow::anyhow!("position submission unresolved; inspect motion history"),
            ));
        }
        if state
            .placement
            .as_ref()
            .is_some_and(|p| p.observation.is_none())
        {
            return Err(Error::new(
                ErrorKind::State,
                anyhow::anyhow!(
                    "survival placement needs target and material observations; inspect the retained intent"
                ),
            ));
        }
        if state.mining.is_some() {
            return Err(Error::new(
                ErrorKind::State,
                anyhow::anyhow!(
                    "survival mining continuation needs inspection; observing removal alone does not authorize another mutation"
                ),
            ));
        }
        Ok(())
    }
    fn require_mode(&self, state: &State, mode: Option<GameMode>) -> Result<()> {
        if mode.is_some() && state.operations.game_mode != mode {
            return Err(Error::new(
                ErrorKind::State,
                anyhow::anyhow!("operation requires matching received game mode"),
            ));
        }
        Ok(())
    }
    fn creative(&self, state: &State) -> Result<()> {
        self.mutable(state)?;
        if state.operations.inventory.pending_swap.is_some() {
            return Err(Error::new(
                ErrorKind::State,
                anyhow::anyhow!("inventory swap needs inspection"),
            ));
        }
        if state.operations.game_mode != Some(GameMode::Creative) {
            return Err(invalid("operation requires received creative game mode"));
        }
        Ok(())
    }
    // Atomic::try_update is unavailable on our Rust 1.85 MSRV.
    #[allow(deprecated)]
    fn next_sequence(&self) -> Result<i32> {
        self.bot
            .session
            .interaction_sequence
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
            .map(|v| v + 1)
            .map_err(|_| invalid("interaction sequence exhausted"))
    }
}
fn invalid(message: &str) -> Error {
    Error::new(ErrorKind::InvalidInput, anyhow::anyhow!("{message}"))
}
fn validate_pose(position: [f64; 3], rotation: [f32; 2]) -> Result<()> {
    if position
        .iter()
        .any(|v| !v.is_finite() || v.abs() > 30_000_000.0)
        || rotation.iter().any(|v| !v.is_finite())
        || rotation[1].abs() > 90.0
    {
        return Err(invalid("invalid player pose"));
    }
    Ok(())
}
fn pack_position(p: [i32; 3]) -> i64 {
    ((i64::from(p[0]) & 0x3ffffff) << 38)
        | ((i64::from(p[2]) & 0x3ffffff) << 12)
        | (i64::from(p[1]) & 0xfff)
}
fn check_reach(state: &State, p: [i32; 3]) -> Result<()> {
    if state.world.block(p).is_none() {
        return Err(invalid("interaction target is not loaded"));
    }
    let player = state.position.context("player position unavailable")?;
    let eye = [player[0], player[1] + 1.62, player[2]];
    if (0..3)
        .map(|i| (eye[i] - f64::from(p[i]) - 0.5).powi(2))
        .sum::<f64>()
        > 4.5f64.powi(2)
    {
        return Err(invalid("interaction target out of reach"));
    }
    Ok(())
}
#[derive(serde::Deserialize)]
struct ItemDefinition {
    id: i32,
    name: String,
    #[serde(rename = "stackSize")]
    stack_size: i32,
}
fn items() -> &'static [ItemDefinition] {
    static ITEMS: std::sync::OnceLock<Vec<ItemDefinition>> = std::sync::OnceLock::new();
    ITEMS.get_or_init(|| {
        serde_json::from_str(include_str!("../../../../data/java_1_21_11/items.json"))
            .expect("valid pinned item registry")
    })
}
pub(crate) fn item_definition(id: i32) -> Result<(String, u32)> {
    let item = items()
        .iter()
        .find(|item| item.id == id)
        .ok_or_else(|| invalid("unknown native item ID"))?;
    let size = u32::try_from(item.stack_size)
        .ok()
        .filter(|size| *size > 0)
        .ok_or_else(|| invalid("invalid native item capacity"))?;
    Ok((format!("minecraft:{}", item.name), size))
}
pub(super) fn slot(r: &mut Reader<'_>) -> anyhow::Result<Option<InventorySlot>> {
    let count = r.varint()?;
    if count < 0 {
        bail!("negative item count");
    }
    if count == 0 {
        return Ok(Some(InventorySlot::Empty));
    }
    let id = r.varint()?;
    let definition = items()
        .iter()
        .find(|i| i.id == id)
        .context("unknown item id")?;
    let Some(components) = super::super::item_components::read_patch(r)? else {
        return Ok(None);
    };
    let item = PlainItem {
        name: format!("minecraft:{}", definition.name),
        item_id: id,
        count,
    };
    Ok(Some(
        if components.added.is_empty() && components.removed.is_empty() {
            InventorySlot::Item { item }
        } else {
            InventorySlot::ItemWithComponents { item, components }
        },
    ))
}
pub(super) fn receive(state: &mut State, id: i32, payload: &[u8]) -> anyhow::Result<bool> {
    use ids::play_clientbound as input;
    if survival::receive(state, id, payload)? {
        return Ok(true);
    }
    if !matches!(
        id,
        input::ABILITIES
            | input::HELD_ITEM_SLOT
            | input::GAME_STATE_CHANGE
            | input::ACKNOWLEDGE_PLAYER_DIGGING
            | input::WINDOW_ITEMS
            | input::SET_SLOT
            | input::SET_PLAYER_INVENTORY
            | input::SET_CURSOR_ITEM
            | input::OPEN_WINDOW
            | input::CLOSE_WINDOW
            | input::SYSTEM_CHAT
            | input::UPDATE_TIME
    ) {
        return Ok(false);
    }
    let mut next = state.operations.clone();
    let mut r = Reader::new(payload);
    match id {
        input::HELD_ITEM_SLOT => {
            let slot = r.varint()?;
            r.end()?;
            if !(0..=8).contains(&slot) {
                bail!("invalid received hotbar selection");
            }
            next.selected_hotbar = Some(HotbarSelection {
                slot: slot as u8,
                sequence: state.sequence,
                dispatched: true,
                from_server: true,
            });
        }
        input::SYSTEM_CHAT => {
            // Validate framing even when presentation exceeds our projection budget.
            r.skip_nbt()?;
            let overlay = r.bool()?;
            r.end()?;
            let component = if payload.len() <= 65536 {
                super::super::component_nbt::read(&mut Reader::new(payload)).ok()
            } else {
                None
            };
            if next.messages.len() == 128 {
                next.messages_dropped_through = next
                    .messages
                    .pop_front()
                    .expect("full message queue")
                    .receive_sequence;
            }
            next.messages.push_back(Arc::new(SystemMessage {
                receive_sequence: state.sequence,
                component,
                overlay,
            }));
        }
        input::UPDATE_TIME => {
            let game_age = r.u64()? as i64;
            let day_time = r.u64()? as i64;
            r.bool()?;
            r.end()?;
            next.server_time = Some(ServerTime {
                game_age,
                day_time,
                receive_sequence: state.sequence,
            });
        }
        input::ABILITIES => {
            let flags = r.u8()?;
            if flags & !15 != 0 {
                bail!("invalid ability flags");
            }
            r.f32()?;
            r.f32()?;
            r.end()?;
            next.abilities = Some(flags);
            next.abilities_sequence = Some(state.sequence);
            if flags & 4 == 0 {
                next.requested_flying = false;
            }
        }
        input::GAME_STATE_CHANGE => {
            let reason = r.u8()?;
            let value = r.f32()?;
            r.end()?;
            if reason == 13 {
                state
                    .loading
                    .initial_chunks_sequence
                    .get_or_insert(state.sequence);
            }
            if reason == 3 {
                if !(0.0..=3.0).contains(&value) || value.fract() != 0.0 {
                    bail!("invalid game mode");
                }
                next.game_mode = Some(GameMode::decode(value as u8)?);
            }
        }
        input::ACKNOWLEDGE_PLAYER_DIGGING => {
            let seq = r.varint()?;
            r.end()?;
            if seq < 0 {
                bail!("negative interaction acknowledgement");
            }
            if next.ack.is_none_or(|old| seq >= old) {
                next.ack = Some(seq);
                next.ack_receive_sequence = Some(state.sequence);
            }
        }
        input::WINDOW_ITEMS
        | input::SET_SLOT
        | input::SET_PLAYER_INVENTORY
        | input::SET_CURSOR_ITEM
        | input::OPEN_WINDOW
        | input::CLOSE_WINDOW => {
            let player_access_after_close =
                state.common_container_close.as_ref().is_some_and(|close| {
                    let mut session = close.initial.session;
                    session.world_generation = state.loading.generation;
                    matches!(
                        crate::client::container::player_screen_access(
                            session,
                            state.operations.inventory.window_id,
                            state
                                .operations
                                .inventory
                                .container
                                .as_ref()
                                .map(|s| s.capture(session).id),
                            Some(close),
                        ),
                        Some(crate::client::container::PlayerScreenAccess::SubmittedClose { .. })
                    )
                });
            inventory::receive(
                &mut next.inventory,
                id,
                payload,
                state.sequence,
                player_access_after_close,
            )?;
            if id == input::CLOSE_WINDOW {
                // Decode succeeded atomically; original state still identifies the opening.
                let mut close = Reader::new(payload);
                container::close_received(state, close.varint()?);
            }
        }
        _ => return Ok(false),
    }
    state.operations = next;
    mining::mining_inventory_received(state);
    Ok(true)
}

impl Operations {
    pub(crate) async fn execute_common(
        &self,
        mode: GameMode,
        action: crate::client::operations::Action<'_>,
    ) -> Result<Option<i32>> {
        use crate::client::operations::Action;
        match action {
            Action::Entity(target, interaction) => {
                let state = self.bot.session.state.lock().await;
                self.mutable(&state)?;
                self.require_mode(&state, Some(mode))?;
                state.entities.validate(
                    crate::client::SessionStamp {
                        version: crate::MinecraftVersion::Java1_21_11,
                        connection_id: self.bot.session.id,
                        world_generation: state.loading.generation,
                    },
                    target,
                )?;
                self.bot
                    .session
                    .send(
                        ids::play_serverbound::USE_ENTITY,
                        &interaction.payload(target),
                    )
                    .await?;
            }

            Action::Look(rotation) => {
                crate::client::operations::validate_rotation(rotation)?;
                self.look_in_mode(Some(mode), rotation).await?;
            }
            Action::SelectHotbar(slot) => self.select_hotbar_in_mode(Some(mode), slot).await?,
            Action::SetFlying(flying) => {
                if mode != GameMode::Creative {
                    return Err(invalid("creative operation required"));
                }
                self.common_flight(crate::client::FlightCommand::SetFlying { flying })
                    .await?;
            }
            Action::MoveFlying(position, rotation) => {
                crate::client::operations::validate_rotation(rotation)?;
                if mode != GameMode::Creative {
                    return Err(invalid("creative operation required"));
                }
                self.common_flight(crate::client::FlightCommand::Move { position, rotation })
                    .await?;
            }
            Action::SetHotbar(slot, item) => {
                if let Some((name, _)) = item {
                    crate::client::registry::Registry::for_version(
                        crate::MinecraftVersion::Java1_21_11,
                    )
                    .item(name)?;
                }
                self.set_creative_hotbar(slot, item).await?;
            }
            Action::Dig(position, face) => {
                return self.dig_creative(position, face).await.map(Some);
            }
            Action::UseOnBlock(position, face, cursor) => {
                return self.use_on_block(position, face, cursor).await.map(Some);
            }
        }
        Ok(None)
    }
    pub(crate) async fn common_connection_identity(
        &self,
    ) -> Result<crate::client::ConnectionIdentity> {
        let state = self.bot.session.state.lock().await;
        let player = self.common_player_unlocked(&state)?;
        let identity = state
            .identity
            .as_ref()
            .ok_or_else(|| invalid("received login profile unavailable"))?;
        Ok(crate::client::ConnectionIdentity {
            session: player.session,
            uuid: identity.uuid,
            name: identity.name.clone(),
        })
    }
    pub(crate) async fn common_entity_spawns(&self) -> Result<crate::client::EntitySpawns> {
        let state = self.bot.session.state.lock().await;
        let player = self.common_player_unlocked(&state)?;
        Ok(state.entities.capture(player.session, state.sequence))
    }
    pub(crate) async fn common_vehicle_state(&self) -> Result<crate::client::VehicleObservation> {
        let state = self.bot.session.state.lock().await;
        let player = self.common_player_unlocked(&state)?;
        Ok(state.vehicles.capture(
            player.session,
            state.sequence,
            state.operations.local_player.entity_id,
            &state.entities,
        ))
    }
    pub(crate) async fn common_player_state(&self) -> Result<crate::client::PlayerObservation> {
        let state = self.bot.session.state.lock().await;
        self.common_player_unlocked(&state)
    }
    pub(crate) async fn common_received_inventory(
        &self,
    ) -> Result<crate::client::ReceivedInventory> {
        let state = self.bot.session.state.lock().await;
        let player = self.common_player_unlocked(&state)?;
        let registries = state
            .registries
            .capture(player.session, player.receive_sequence);
        crate::client::ReceivedInventory::capture(
            player.session,
            player.receive_sequence,
            &player.inventory,
            registries,
        )
    }
    pub(crate) async fn common_received_recipes(&self) -> Result<crate::client::ReceivedRecipes> {
        let state = self.bot.session.state.lock().await;
        let player = self.common_player_unlocked(&state)?;
        state.recipes.capture(
            player.session,
            state.sequence,
            state.registries.capture(player.session, state.sequence),
        )
    }
    pub(crate) async fn common_received_crafting_context(
        &self,
    ) -> Result<Option<crate::client::ReceivedCraftingContext>> {
        let state = self.bot.session.state.lock().await;
        let player = self.common_player_unlocked(&state)?;
        let screen = crate::client::container::ScreenObservation {
            session: player.session,
            receive_sequence: player.receive_sequence,
            active_window: state.operations.inventory.window_id,
            player_screen: player.inventory.player_screen,
            screen: state
                .operations
                .inventory
                .container
                .as_ref()
                .map(|s| s.capture(player.session)),
            cursor: player.inventory.cursor.clone(),
        };
        let registries = state
            .registries
            .capture(player.session, player.receive_sequence);
        let recipes =
            state
                .recipes
                .capture(player.session, player.receive_sequence, registries.clone())?;
        crate::client::ReceivedCraftingContext::capture(player, screen, registries, recipes)
    }
    pub(crate) async fn common_recipe_book_materials(
        &self,
        recipe: &crate::client::RecipeId,
        crafts: u32,
        maximum_bound: u32,
    ) -> Result<crate::client::RecipeBookMaterials> {
        let state = self.bot.session.state.lock().await;
        let player = self.common_player_unlocked(&state)?;
        let registries = state
            .registries
            .capture(player.session, player.receive_sequence);
        let catalogue =
            state
                .recipes
                .capture(player.session, player.receive_sequence, registries.clone())?;
        let inventory = crate::client::ReceivedInventory::capture(
            player.session,
            player.receive_sequence,
            &player.inventory,
            registries,
        )?;
        crate::client::RecipeBookMaterials::capture(
            catalogue,
            inventory,
            recipe,
            crafts,
            maximum_bound,
        )
    }
    pub(crate) async fn common_received_crafting(
        &self,
    ) -> Result<Option<crate::client::ReceivedCrafting>> {
        let state = self.bot.session.state.lock().await;
        let player = self.common_player_unlocked(&state)?;
        let screen = crate::client::container::ScreenObservation {
            session: player.session,
            receive_sequence: state.sequence,
            active_window: state.operations.inventory.window_id,
            player_screen: player.inventory.player_screen,
            screen: state
                .operations
                .inventory
                .container
                .as_ref()
                .map(|s| s.capture(player.session)),
            cursor: player.inventory.cursor.clone(),
        };
        let registries = state
            .registries
            .capture(player.session, player.receive_sequence);
        crate::client::ReceivedCrafting::capture(&player, &screen, registries)
    }
    pub(crate) async fn common_capture(
        &self,
        region: crate::Region,
    ) -> Result<crate::client::Capture> {
        let volume = region.volume()?;
        let state = self.bot.session.state.lock().await;
        let player = self.common_player_unlocked(&state)?;
        let dimension = &state
            .world
            .dimension
            .as_ref()
            .context("dimension unavailable")?
            .1;
        if region.min[1] < dimension.min_y || region.max[1] >= dimension.min_y + dimension.height {
            return Err(invalid("region outside dimension height"));
        }
        let mut blocks = Vec::with_capacity(volume);
        for y in region.min[1]..=region.max[1] {
            for z in region.min[2]..=region.max[2] {
                for x in region.min[0]..=region.max[0] {
                    let position = [x, y, z];
                    blocks.push(crate::ObservedBlock {
                        position,
                        state: state
                            .world
                            .block(position)
                            .map(super::super::native_state)
                            .transpose()?,
                    });
                }
            }
        }
        Ok(crate::client::Capture {
            world: crate::Observation {
                version: crate::MinecraftVersion::Java1_21_11,
                connection_id: self.bot.session.id,
                revision: state.world.revision,
                receive_sequence: Some(state.sequence),
                captured_at: self.bot.session.started.elapsed(),
                region,
                blocks,
            },
            player,
        })
    }
    fn common_player_unlocked(&self, state: &State) -> Result<crate::client::PlayerObservation> {
        self.bot.session.check(state)?;
        common_player_in_state(
            state,
            self.bot.session.id,
            self.bot.session.interrupted_packet.load(Ordering::Acquire) >= 0,
        )
    }
}
pub(super) fn common_player_in_state(
    state: &State,
    connection_id: u64,
    interrupted: bool,
) -> Result<crate::client::PlayerObservation> {
    use crate::client as api;
    let native = &state.operations;
    let inventory = &native.inventory;
    let slots = inventory
        .slots
        .iter()
        .zip(&inventory.slot_sequences)
        .map(|(slot, sequence)| {
            if matches!(slot, InventorySlot::Unavailable) {
                return Ok(None);
            }
            sequence
                .map(|sequence| common_slot(slot).map(|value| api::received(value, sequence)))
                .transpose()
        })
        .collect::<Result<Vec<_>>>()?;
    let cursor = match inventory.cursor {
        InventorySlot::Unavailable => None,
        _ => inventory
            .cursor_sequence
            .map(|sequence| {
                common_slot(&inventory.cursor).map(|value| api::received(value, sequence))
            })
            .transpose()?,
    };
    let pose = state
        .motion
        .received_pose
        .as_ref()
        .filter(|pose| pose.generation == state.loading.generation)
        .map(|pose| api::ReceivedPose {
            position: pose.position,
            rotation: pose.rotation,
            receive_sequence: pose.receive_sequence,
        });
    let source = match state.motion.position_basis {
        PositionBasis::Received => api::ValueSource::Received {
            sequence: pose.as_ref().map_or(state.sequence, |p| p.receive_sequence),
        },
        PositionBasis::Submitted => api::ValueSource::Submitted,
        _ => api::ValueSource::Predicted,
    };
    Ok(api::PlayerObservation {
        session: api::SessionStamp {
            version: crate::MinecraftVersion::Java1_21_11,
            connection_id,
            world_generation: state.loading.generation,
        },
        receive_sequence: state.sequence,
        pending_dispatch: crate::client::flight::unresolved(&state.flight_history)
            || state
                .dismount_history
                .lock()
                .expect("dismount history")
                .as_ref()
                .is_some_and(|r| r.unresolved())
            || state
                .common_container_open
                .as_ref()
                .is_some_and(|o| o.unresolved())
            || interrupted
            || state
                .common_container_close
                .as_ref()
                .is_some_and(|r| r.unresolved())
            || state.common_inventory_swap.as_ref().is_some_and(|s| {
                s.record.stage != crate::client::inventory::InventorySwapStage::ObservedSwapped
            })
            || state
                .common_inventory_click
                .as_ref()
                .is_some_and(|s| s.unresolved())
            || state
                .common_crafting_take
                .as_ref()
                .is_some_and(|r| r.unresolved())
            || state
                .common_inventory_transfer
                .as_ref()
                .is_some_and(|s| s.unresolved())
            || !inventory.pending_creative.is_empty()
            || inventory.pending_swap.is_some()
            || state.motion.position_basis == PositionBasis::PendingSubmission
            || state.mining.is_some()
            || state
                .placement
                .as_ref()
                .is_some_and(|p| p.observation.is_none())
            || state
                .survival_motion
                .as_ref()
                .is_some_and(|m| !m.status.is_continuation_candidate()),
        dimension: state
            .world
            .dimension
            .as_ref()
            .map(|(name, dimension)| api::Dimension {
                name: name.clone(),
                min_y: dimension.min_y,
                height: dimension.height,
            }),
        position: match state.motion.position_basis {
            PositionBasis::Unavailable | PositionBasis::PendingSubmission => None,
            _ => state
                .position
                .map(|value| api::ObservedValue { value, source }),
        },
        received_pose: pose,
        rotation: state.rotation,
        game_mode: native.game_mode,
        may_fly: native.abilities.map(|flags| flags & 4 != 0),
        health: native.local_player.health.as_ref().map(|health| {
            api::received(
                api::Health {
                    health: health.health,
                    food: health.food,
                    saturation: health.saturation,
                },
                health.receive_sequence,
            )
        }),
        selected_hotbar: native
            .selected_hotbar
            .as_ref()
            .map(|selection| api::ObservedValue {
                value: selection.slot,
                source: if selection.from_server {
                    api::ValueSource::Received {
                        sequence: selection.sequence,
                    }
                } else {
                    api::ValueSource::Submitted
                },
            }),
        inventory: api::InventoryObservation {
            slots,
            cursor,
            window_id: inventory.window_id,
            player_screen: crate::client::container::player_screen_access(
                api::SessionStamp {
                    version: crate::MinecraftVersion::Java1_21_11,
                    connection_id,
                    world_generation: state.loading.generation,
                },
                inventory.window_id,
                inventory.container.as_ref().map(|s| {
                    s.capture(api::SessionStamp {
                        version: crate::MinecraftVersion::Java1_21_11,
                        connection_id,
                        world_generation: state.loading.generation,
                    })
                    .id
                }),
                state.common_container_close.as_ref(),
            ),
            screen_revision: inventory.screen_revision,
            player_screen_revision: inventory.player_revision.clone(),
            local_cache: None,
        },
    })
}

pub(super) fn common_slot(slot: &InventorySlot) -> Result<crate::client::SlotKnowledge> {
    use crate::client as api;
    Ok(match slot {
        InventorySlot::Unavailable => api::SlotKnowledge::Unavailable,
        InventorySlot::Empty => api::SlotKnowledge::Empty,
        InventorySlot::Item { item } | InventorySlot::ItemWithComponents { item, .. } => {
            let definition =
                api::registry::Registry::for_version(crate::MinecraftVersion::Java1_21_11)
                    .item_by_native_id(item.item_id)?;
            let count = u32::try_from(item.count)
                .ok()
                .filter(|count| *count > 0)
                .ok_or_else(|| invalid("invalid native item count"))?;
            api::SlotKnowledge::Item {
                item: api::ItemStack {
                    id: definition.id,
                    name: definition.name,
                    count,
                    data: match slot {
                        InventorySlot::ItemWithComponents { components, .. } => {
                            api::ItemData::ModernComponents {
                                patch: components.clone(),
                            }
                        }
                        _ => api::ItemData::Default,
                    },
                },
            }
        }
    })
}
