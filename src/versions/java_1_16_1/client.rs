//! Connection lifecycle, protocol events, observations, and player operations.

mod common_api;
mod common_chunks;
mod common_click;
mod common_container;
mod common_control;
mod common_crafting;
mod common_entity_motion;
mod common_entity_parts;
pub(crate) use common_entity_parts::derive_entity_parts;
mod common_flight;
mod common_inventory;
mod common_mining;
mod common_motion;
mod common_placement;
mod common_recipe_placement;
mod common_recording;
mod common_scene;
pub(crate) use common_recording::replay_packets;
pub(crate) use common_scene::LegacyCapturedScene;
mod common_transfer;
mod common_vehicle;

use crate::versions::java_1_16_1::Result;
use crate::versions::java_1_16_1::{
    chat::{ChatMessage, PlayerList, apply_player_info, parse_chat},
    entity::{
        EntityState, EntityTracker, MetadataValue, apply_relative, parse_metadata,
        parse_spawn_living, parse_spawn_object, parse_spawn_orb, parse_spawn_painting,
        parse_spawn_player,
    },
    interaction::{
        BlockBreakProgress, BlockFace, BlockPos, DiggingAcknowledgement, DiggingStatus, Hand,
        parse_break_progress, parse_digging_ack,
    },
    inventory::{
        ClickMode, EquipmentSlot, InventoryState, ItemCollected, ItemStack, OpenWindow,
        PendingClick, SlotUpdate, WindowProperty, WindowTransaction, apply_slot,
        apply_window_items, parse_merchant_offers, parse_set_slot, parse_window_items,
        predict_normal_click, read_slot, rollback_click, sync_player_inventory_from_window,
        write_slot,
    },
    lifecycle::{
        ConnectionActor, ConnectionGeneration, ConnectionState, GenerationRevocation,
        OperationAdmissionError, OperationClass, OperationContext, ProtocolTransaction,
        TerminalClassification,
    },
    map::{MapData, MapStore, MapUpdate, parse_map_update},
    operation::{
        AcknowledgedOperation, CleanupDispatchOutcome, CleanupOperation, DispatchError,
        DispatchOutcome, EquipOperation, Operation, SlotExpectation, WindowClick,
        WindowClickSequence,
    },
    physics::{
        Aabb, ControlState, MotionState, PhysicsMetrics, PhysicsTracker, PositionCorrection, Vec3,
        VehicleControl, VehiclePose,
    },
    progress::{AdvancementState, RecipeBookState, StatisticsState},
    protocol::*,
    server_registry::{
        CommandTree, ServerRecipes, ServerTags, parse_command_tree, parse_recipes, parse_tags,
    },
    snapshot::{Snapshot, Versioned},
    survival::{
        Attribute, CombatEvent, Difficulty, Experience, GameStateChange, RespawnState,
        SurvivalState, Vitals, parse_attributes, parse_combat_event, parse_effect,
        parse_experience, parse_join, parse_respawn, parse_vitals, unpack_position,
    },
    ui::{UiState, UiUpdateKind},
    world::{BlockObservation, Fluid, World},
};
use anyhow::Context;
mod reader_diagnostics;

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use reader_diagnostics::{Diagnostics as ReaderDiagnostics, Phase as ReaderPhase};
use std::{
    collections::{HashMap, VecDeque},
    io::Cursor,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicU64, AtomicUsize, Ordering},
    },
};

fn slot_matches(item: &ItemStack, expected: &SlotExpectation) -> bool {
    item.name() == Some(expected.item_name.as_str())
        && i32::from(item.count) == expected.count
        && expected.metadata == 0
}

fn storage_range(window: i8, len: usize) -> Result<std::ops::Range<usize>> {
    if window == 0 {
        check_inventory(len >= 45, "player storage has not arrived")?;
        Ok(9..45)
    } else {
        check_inventory(len >= 36, "window storage has not arrived")?;
        Ok(len - 36..len)
    }
}

fn check_inventory(condition: bool, message: &str) -> Result<()> {
    if !condition {
        return Err(anyhow::anyhow!("{message}").into());
    }
    Ok(())
}

fn merge_room(destination: Option<&ItemStack>, item: &ItemStack) -> i16 {
    let max = crate::versions::java_1_16_1::registry::item_max_stack_size(item.item_id)
        .map_or(0, i16::from);
    match destination {
        None => max,
        Some(other) if other.item_id == item.item_id && other.nbt == item.nbt => {
            (max - i16::from(other.count)).max(0)
        }
        Some(_) => 0,
    }
}

fn storage_capacity(inventory: &InventoryState, window: i8, item: &ItemStack) -> Result<i16> {
    let slots = inventory
        .windows
        .get(&window)
        .context("window storage unavailable")?;
    Ok(storage_range(window, slots.len())?
        .map(|i| merge_room(slots[i].as_ref(), item))
        .sum())
}

fn optional_slot_matches(item: Option<&ItemStack>, expected: Option<&SlotExpectation>) -> bool {
    match (item, expected) {
        (None, None) => true,
        (Some(item), Some(expected)) => slot_matches(item, expected),
        _ => false,
    }
}

fn apply_accepted_normal_click(
    inventory: &mut InventoryState,
    pending: &PendingClick,
) -> Result<()> {
    let current_slot = usize::try_from(pending.slot)
        .ok()
        .and_then(|slot| {
            inventory
                .windows
                .get(&pending.window_id)
                .and_then(|slots| slots.get(slot))
        })
        .cloned()
        .flatten();
    let current_cursor = inventory.cursor.clone();
    let mut predicted = inventory.clone();
    let slot =
        usize::try_from(pending.slot).map_err(|_| anyhow::anyhow!("invalid pending slot"))?;
    let slots = predicted
        .windows
        .get_mut(&pending.window_id)
        .context("pending window has no slots")?;
    let target = slots
        .get_mut(slot)
        .context("pending slot is outside window")?;
    *target = pending.slot_before.clone();
    predicted.cursor = pending.cursor_before.clone();
    predict_normal_click(
        &mut predicted,
        pending.window_id,
        pending.slot,
        pending.button,
    )?;
    // Check Caller predictions against the unmodified cache. Ordinary click
    // prediction can clear a replenished recipe output, destroying the exact
    // prestate that authorizes its accepted effect.
    apply_window_prediction(inventory, pending, &predicted)?;
    let effects = &pending.prediction;
    let clicked_slot_declared = effects
        .slots
        .iter()
        .any(|effect| effect.slot == i32::from(pending.slot));
    let prediction_declared = !effects.slots.is_empty()
        || effects.cursor_before.is_some()
        || effects.cursor_after.is_some();
    // A server SetSlot/WindowItems fact always wins. The compatibility click
    // prediction only fills domains with no Caller-supplied effect; it cannot
    // overwrite a declared identity effect or an early server update.
    if !clicked_slot_declared && current_slot == pending.slot_before {
        inventory.windows.get_mut(&pending.window_id).unwrap()[slot] =
            predicted.windows[&pending.window_id][slot].clone();
    }
    if !prediction_declared && current_cursor == pending.cursor_before {
        inventory.cursor = predicted.cursor;
    }
    sync_player_inventory_from_window(inventory, pending.window_id);
    Ok(())
}

fn effect_stack(expectation: &SlotExpectation) -> Result<ItemStack> {
    let item_id = crate::registry::item_id(&expectation.item_name)
        .context("accepted cache effect names unknown item")?;
    if expectation.metadata != 0 || !(1..=64).contains(&expectation.count) {
        return Err(anyhow::anyhow!("accepted cache effect stack invalid").into());
    }
    Ok(ItemStack {
        item_id,
        count: i8::try_from(expectation.count).context("accepted cache effect count")?,
        nbt: None,
    })
}

fn effect_slot_matches(item: Option<&ItemStack>, expected: Option<&SlotExpectation>) -> bool {
    optional_slot_matches(item, expected)
}

fn apply_window_prediction(
    inventory: &mut InventoryState,
    pending: &PendingClick,
    predicted: &InventoryState,
) -> Result<()> {
    let effects = &pending.prediction;
    for effect in &effects.slots {
        let slot = usize::try_from(effect.slot).context("accepted cache effect slot")?;
        let current = inventory
            .windows
            .get(&pending.window_id)
            .and_then(|slots| slots.get(slot))
            .context("accepted cache effect slot outside window")?
            .as_ref();
        if effect_slot_matches(current, effect.before.as_ref()) {
            let predicted_slot = predicted
                .windows
                .get(&pending.window_id)
                .and_then(|slots| slots.get(slot))
                .and_then(Option::as_ref);
            let after = if effect_slot_matches(predicted_slot, effect.after.as_ref()) {
                predicted_slot.cloned()
            } else {
                effect.after.as_ref().map(effect_stack).transpose()?
            };
            inventory.windows.get_mut(&pending.window_id).unwrap()[slot] = after;
        }
    }
    if effect_slot_matches(inventory.cursor.as_ref(), effects.cursor_before.as_ref()) {
        inventory.cursor =
            if effect_slot_matches(predicted.cursor.as_ref(), effects.cursor_after.as_ref()) {
                predicted.cursor.clone()
            } else {
                effects
                    .cursor_after
                    .as_ref()
                    .map(effect_stack)
                    .transpose()?
            };
    }
    Ok(())
}

/// Completes the client cache for an accepted protocol mode-2 hotbar swap.
///
/// Vanilla 1.16 servers may acknowledge the number-key click without
/// echoing either changed player slot. Each domain is therefore repaired only
/// while its exact prestate is still present; an early SetSlot fact remains
/// authoritative. The actual cached stacks are moved so NBT is preserved.
fn apply_accepted_equip_swap(
    inventory: &mut InventoryState,
    source_slot: u16,
    selected_hotbar_slot: u8,
    source_before: &ItemStack,
    destination_before: Option<&ItemStack>,
) {
    let source_index = usize::from(source_slot);
    let destination_index = 36 + usize::from(selected_hotbar_slot);
    let Some(slots) = inventory.windows.get_mut(&0) else {
        return;
    };
    if source_index >= slots.len() || destination_index >= slots.len() {
        return;
    }
    if slots[source_index].as_ref() == Some(source_before) {
        slots[source_index] = destination_before.cloned();
    }
    if slots[destination_index] == destination_before.cloned() {
        slots[destination_index] = Some(source_before.clone());
    }
}

#[derive(Clone)]
struct ExactWindowBarrier {
    successor: Option<WindowClick>,
    confirmation_seen: bool,
}

fn click_precondition_matches(
    inventory: &InventoryState,
    window_id: i8,
    click: &WindowClick,
) -> bool {
    let slot = usize::try_from(click.slot)
        .ok()
        .and_then(|slot| {
            inventory
                .windows
                .get(&window_id)
                .and_then(|slots| slots.get(slot))
        })
        .and_then(Option::as_ref);
    optional_slot_matches(slot, click.expected_item.as_ref())
        && optional_slot_matches(inventory.cursor.as_ref(), click.expected_cursor.as_ref())
}

use tokio::{
    net::{TcpStream, tcp::OwnedWriteHalf},
    sync::{Mutex, Notify, RwLock, broadcast, mpsc, oneshot},
    time::{Duration, timeout},
};

async fn read_session_cache<T>(cache: &RwLock<T>) -> tokio::sync::RwLockReadGuard<'_, T> {
    // Cardinality checks are short and already inside the coherent-state gate.
    // try_read respects queued writers; contention retains the async fair path.
    match cache.try_read() {
        Ok(guard) => guard,
        Err(_) => cache.read().await,
    }
}

async fn write_entity_update<T>(cache: &RwLock<T>) -> tokio::sync::RwLockWriteGuard<'_, T> {
    match cache.try_write() {
        Ok(guard) => guard,
        Err(_) => cache.write().await,
    }
}

async fn lock_packet_state<T>(state: &Mutex<T>) -> tokio::sync::MutexGuard<'_, T> {
    match state.try_lock() {
        Ok(guard) => guard,
        Err(_) => state.lock().await,
    }
}

macro_rules! bail {
    ($($argument:tt)*) => {
        return Err(crate::versions::java_1_16_1::Error::from(anyhow::anyhow!($($argument)*)).into())
    };
}

mod play_entities;
mod play_inventory;
mod play_player;
mod play_session;
mod play_ui;
mod play_world;

fn oxygen_level_from_air_ticks(air_ticks: i32) -> Option<u8> {
    if air_ticks < 0 {
        return None;
    }
    let rounded = (i64::from(air_ticks) + 7) / 15;
    u8::try_from(rounded).ok().filter(|value| *value <= 20)
}

// Protocol 736 Entity metadata index 1 (air supply) is backed by the
// LivingEntity data-tracker default of 300 ticks. Vanilla does not promise to
// send unchanged default metadata for the local player after Join Game, so
// the client cache must be initialized from the protocol entity default and
// then replaced only by packet facts.
const PROTOCOL_736_DEFAULT_AIR_TICKS: i32 = 300;

fn protocol_default_oxygen_level() -> u8 {
    oxygen_level_from_air_ticks(PROTOCOL_736_DEFAULT_AIR_TICKS)
        .expect("protocol 736 default air supply is representable")
}

fn oxygen_level_after_respawn(current: Option<u8>, copy_metadata: bool) -> Option<u8> {
    if copy_metadata {
        current
    } else {
        Some(protocol_default_oxygen_level())
    }
}

const VANILLA_SPRINT_MODIFIER_UUID: [u8; 16] = [
    0x66, 0x2a, 0x6b, 0x8d, 0xda, 0x3e, 0x4c, 0x1c, 0x88, 0x13, 0x96, 0xea, 0x60, 0x97, 0x27, 0x8d,
];

fn movement_speed_for_control(movement_attribute: Option<&Attribute>, sprint: bool) -> f64 {
    let base_movement_speed = movement_attribute.map_or(0.1, Attribute::value);
    let sprint_modifier_observed = movement_attribute.is_some_and(|attribute| {
        attribute.modifiers.iter().any(|modifier| {
            modifier.uuid == VANILLA_SPRINT_MODIFIER_UUID && modifier.operation == 2
        })
    });
    if sprint && !sprint_modifier_observed {
        base_movement_speed * 1.3
    } else {
        base_movement_speed
    }
}

fn encode_entity_action_packet(entity_id: i32, sneaking: bool) -> (i32, Vec<u8>) {
    let mut payload = Vec::new();
    put_varint(&mut payload, entity_id);
    put_varint(&mut payload, if sneaking { 0 } else { 1 });
    put_varint(&mut payload, 0);
    (0x1c, payload)
}

pub use crate::client::Server;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
/// State and protocol data represented by `ConnectionOptions`.
pub struct ConnectionOptions {
    /// Maximum time allowed to establish the TCP connection.
    pub connect_timeout: Duration,
    /// Maximum delay allowed between packets during login.
    pub login_packet_timeout: Duration,
    /// Maximum delay allowed between packets after login.
    pub play_packet_timeout: Duration,
    /// Default deadline used while waiting for the initial playable state.
    pub ready_timeout: Duration,
    /// Maximum chunks retained by one connection.
    pub max_chunks: usize,
    /// Maximum entities retained by one connection.
    pub max_entities: usize,
    /// Maximum map IDs retained by one connection.
    pub max_maps: usize,
    /// Aggregate cardinality limit for smaller state caches.
    pub max_cached_records: usize,
    /// Maximum retained bytes for one custom payload event.
    pub max_custom_payload_bytes: usize,
    /// Number of events retained for each Bot broadcast receiver.
    pub event_channel_capacity: usize,
    /// Maximum time the connection actor waits for a protocol acknowledgement.
    pub protocol_ack_timeout: Duration,
}

impl Default for ConnectionOptions {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            login_packet_timeout: Duration::from_secs(15),
            play_packet_timeout: Duration::from_secs(60),
            ready_timeout: Duration::from_secs(15),
            max_chunks: 256,
            max_entities: 4_096,
            max_maps: 128,
            max_cached_records: 4_096,
            max_custom_payload_bytes: 65_536,
            event_channel_capacity: 256,
            protocol_ack_timeout: Duration::from_secs(5),
        }
    }
}

/// Fixed wire-protocol identity of this crate build.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProtocolInfo {
    /// The `minecraft_version` value.
    pub minecraft_version: &'static str,
    /// The `protocol_version` value.
    pub protocol_version: i32,
}

/// Feature boundary advertised to embedding controllers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ClientCapabilities {
    /// The `offline_mode` value.
    pub offline_mode: bool,
    /// The `online_mode_authentication` value.
    pub online_mode_authentication: bool,
    /// The `multi_bot` value.
    pub multi_bot: bool,
    /// The `movement_physics` value.
    pub movement_physics: bool,
    /// The `world_observation` value.
    pub world_observation: bool,
    /// The `inventory_and_windows` value.
    pub inventory_and_windows: bool,
    /// The `crafting` value.
    pub crafting: bool,
    /// The `entity_interaction` value.
    pub entity_interaction: bool,
    /// The `structured_sound_events` value.
    pub structured_sound_events: bool,
    /// The `audio_playback` value.
    pub audio_playback: bool,
    /// The `raycast_and_reach_queries` value.
    pub raycast_and_reach_queries: bool,
    /// The `low_copy_chunk_snapshots` value.
    pub low_copy_chunk_snapshots: bool,
    /// The `shared_chunk_sections` value.
    pub shared_chunk_sections: bool,
    /// The `structured_map_data` value.
    pub structured_map_data: bool,
    /// The `vehicle_input` value.
    pub vehicle_input: bool,
}

/// Build and protocol information for compatibility checks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClientInfo {
    /// The `crate_version` value.
    pub crate_version: &'static str,
    /// The `protocol` value.
    pub protocol: ProtocolInfo,
    /// The `capabilities` value.
    pub capabilities: ClientCapabilities,
}

/// Public constant `CLIENT_INFO`.
pub const CLIENT_INFO: ClientInfo = ClientInfo {
    crate_version: env!("CARGO_PKG_VERSION"),
    protocol: ProtocolInfo {
        minecraft_version: "1.16.1",
        protocol_version: PROTOCOL_VERSION,
    },
    capabilities: ClientCapabilities {
        offline_mode: true,
        online_mode_authentication: false,
        multi_bot: true,
        movement_physics: true,
        world_observation: true,
        inventory_and_windows: true,
        crafting: true,
        entity_interaction: true,
        structured_sound_events: true,
        audio_playback: false,
        raycast_and_reach_queries: true,
        low_copy_chunk_snapshots: true,
        shared_chunk_sections: true,
        structured_map_data: true,
        vehicle_input: true,
    },
};

/// Offline identity and live state. `username` is used by the login packet.
#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    /// The `username` value.
    pub username: String,
    /// The `entity_id` value.
    pub entity_id: Option<i32>,
    /// The `x` value.
    pub x: f64,
    /// The `y` value.
    pub y: f64,
    /// The `z` value.
    pub z: f64,
    /// The `yaw` value.
    pub yaw: f32,
    /// The `pitch` value.
    pub pitch: f32,
    /// The `on_ground` value.
    pub on_ground: bool,
    /// The `spawned` value.
    pub spawned: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i32)]
/// Possible values represented by `ChatMode`.
pub enum ChatMode {
    /// Documentation for this public variant.
    Enabled = 0,
    /// Documentation for this public variant.
    CommandsOnly = 1,
    /// Documentation for this public variant.
    Hidden = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i32)]
/// Possible values represented by `MainHand`.
pub enum MainHand {
    /// Documentation for this public variant.
    Left = 0,
    /// Documentation for this public variant.
    Right = 1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// State and protocol data represented by `ClientSettings`.
pub struct ClientSettings {
    /// The `locale` value.
    pub locale: String,
    /// The `view_distance` value.
    pub view_distance: i8,
    /// The `chat_mode` value.
    pub chat_mode: ChatMode,
    /// The `chat_colors` value.
    pub chat_colors: bool,
    /// The `skin_parts` value.
    pub skin_parts: u8,
    /// The `main_hand` value.
    pub main_hand: MainHand,
    /// The `brand` value.
    pub brand: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// State and protocol data represented by `ResourcePackRequest`.
pub struct ResourcePackRequest {
    /// The `url` value.
    pub url: String,
    /// The `hash` value.
    pub hash: String,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i32)]
/// Possible values represented by `ResourcePackStatus`.
pub enum ResourcePackStatus {
    /// Documentation for this public variant.
    SuccessfullyLoaded = 0,
    /// Documentation for this public variant.
    Declined = 1,
    /// Documentation for this public variant.
    FailedDownload = 2,
    /// Documentation for this public variant.
    Accepted = 3,
}
#[derive(Clone, Debug, Eq, PartialEq)]
/// State and protocol data represented by `TabCompletionMatch`.
pub struct TabCompletionMatch {
    /// The `value` value.
    pub value: String,
    /// The `tooltip_json` value.
    pub tooltip_json: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
/// State and protocol data represented by `TabCompletion`.
pub struct TabCompletion {
    /// The `transaction_id` value.
    pub transaction_id: i32,
    /// The `start` value.
    pub start: i32,
    /// The `length` value.
    pub length: i32,
    /// The `matches` value.
    pub matches: Vec<TabCompletionMatch>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// State and protocol data represented by `WorldEvent`.
pub struct WorldEvent {
    /// The `effect_id` value.
    pub effect_id: i32,
    /// The `position` value.
    pub position: BlockPos,
    /// The `data` value.
    pub data: i32,
    /// The `global` value.
    pub global: bool,
}
#[derive(Clone, Debug, PartialEq)]
/// State and protocol data represented by `WorldParticleEvent`.
pub struct WorldParticleEvent {
    /// The `particle_id` value.
    pub particle_id: i32,
    /// The `long_distance` value.
    pub long_distance: bool,
    /// The `position` value.
    pub position: Vec3,
    /// The `offset` value.
    pub offset: [f32; 3],
    /// The `speed` value.
    pub speed: f32,
    /// The `count` value.
    pub count: i32,
    /// The `data` value.
    pub data: crate::versions::java_1_16_1::ParticleData,
    /// The `raw_data` value.
    pub raw_data: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq)]
/// State and protocol data represented by `ExplosionEvent`.
pub struct ExplosionEvent {
    /// The `position` value.
    pub position: Vec3,
    /// The `radius` value.
    pub radius: f32,
    /// The `affected_blocks` value.
    pub affected_blocks: Vec<BlockPos>,
    /// The `player_motion` value.
    pub player_motion: Vec3,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
/// State and protocol data represented by `WorldViewState`.
pub struct WorldViewState {
    /// The `center_x` value.
    pub center_x: i32,
    /// The `center_z` value.
    pub center_z: i32,
    /// The `distance` value.
    pub distance: i32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// State and protocol data represented by `BlockActionEvent`.
pub struct BlockActionEvent {
    /// The `position` value.
    pub position: BlockPos,
    /// The `action` value.
    pub action: u8,
    /// The `parameter` value.
    pub parameter: u8,
    /// The `block_id` value.
    pub block_id: i32,
}
#[derive(Clone, Debug, Eq, PartialEq)]
/// State and protocol data represented by `StopSoundEvent`.
pub struct StopSoundEvent {
    /// The `category` value.
    pub category: Option<i32>,
    /// The `sound_name` value.
    pub sound_name: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
/// State and protocol data represented by `NbtQueryResponse`.
pub struct NbtQueryResponse {
    /// The `transaction_id` value.
    pub transaction_id: i32,
    /// The `nbt` value.
    pub nbt: Arc<[u8]>,
}

impl Default for ClientSettings {
    fn default() -> Self {
        Self {
            locale: "en_us".into(),
            view_distance: 10,
            chat_mode: ChatMode::Enabled,
            chat_colors: true,
            skin_parts: 0x7f,
            main_hand: MainHand::Right,
            brand: "Voxrig".into(),
        }
    }
}
impl Player {
    /// Performs the `offline` operation.
    pub fn offline(username: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            entity_id: None,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: false,
            spawned: false,
        }
    }
    pub(crate) fn validate(&self) -> Result<()> {
        let valid = (3..=16).contains(&self.username.len())
            && self
                .username
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_');
        if !valid {
            bail!("username must be 3-16 ASCII letters, digits, or underscores");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
/// Possible values represented by `SoundSource`.
pub enum SoundSource {
    /// The `Position` variant.
    Position {
        /// World X coordinate.
        x: f64,
        /// World Y coordinate.
        y: f64,
        /// World Z coordinate.
        z: f64,
    },
    /// The `Entity` variant.
    Entity {
        /// Protocol entity ID producing the sound.
        entity_id: i32,
    },
}
#[derive(Clone, Debug, PartialEq)]
/// State and protocol data represented by `SoundEvent`.
pub struct SoundEvent {
    /// The `sound_id` value.
    pub sound_id: Option<i32>,
    /// The `sound_name` value.
    pub sound_name: Option<String>,
    /// The `category` value.
    pub category: i32,
    /// The `category_name` value.
    pub category_name: Option<&'static str>,
    /// The `source` value.
    pub source: SoundSource,
    /// The `volume` value.
    pub volume: f32,
    /// The `pitch` value.
    pub pitch: f32,
    /// The `sequence` value.
    pub sequence: u64,
    /// The `received_at` value.
    pub received_at: Duration,
}
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
/// Possible values represented by `Event`.
pub enum Event {
    /// The `Login` variant.
    Login,
    /// The `Spawn` variant.
    Spawn,
    /// Documentation for this public variant.
    Position(Player),
    /// The `ChunkLoaded` variant.
    ChunkLoaded {
        /// The `x` value carried by this variant.
        x: i32,
        /// The `z` value carried by this variant.
        z: i32,
    },
    /// The `ChunkUnloaded` variant.
    ChunkUnloaded {
        /// The `x` value carried by this variant.
        x: i32,
        /// The `z` value carried by this variant.
        z: i32,
    },
    /// The `ChunkLightUpdated` variant.
    ChunkLightUpdated {
        /// The `x` value carried by this variant.
        x: i32,
        /// The `z` value carried by this variant.
        z: i32,
    },
    /// The `BlockChanged` variant.
    BlockChanged {
        /// The `x` value carried by this variant.
        x: i32,
        /// The `y` value carried by this variant.
        y: i32,
        /// The `z` value carried by this variant.
        z: i32,
        /// The `state_id` value carried by this variant.
        state_id: i32,
    },
    /// Documentation for this public variant.
    BlockEntityUpdated(crate::versions::java_1_16_1::BlockEntityData),
    /// The `MultiBlockChanged` variant.
    MultiBlockChanged {
        /// The `count` value carried by this variant.
        count: usize,
    },
    /// Documentation for this public variant.
    Sound(SoundEvent),
    /// Documentation for this public variant.
    Vitals(Vitals),
    /// Documentation for this public variant.
    Experience(Experience),
    /// Documentation for this public variant.
    Difficulty(Difficulty),
    /// Documentation for this public variant.
    GameStateChange(GameStateChange),
    /// Documentation for this public variant.
    SpawnPosition(crate::versions::java_1_16_1::SpawnPosition),
    /// The `EntityStatus` variant.
    EntityStatus {
        /// The `entity_id` value carried by this variant.
        entity_id: i32,
        /// The `status` value carried by this variant.
        status: i8,
    },
    /// Documentation for this public variant.
    Combat(CombatEvent),
    /// Documentation for this public variant.
    Respawn(RespawnState),
    /// The `SurvivalStateUpdated` variant.
    SurvivalStateUpdated,
    /// The `InventoryUpdated` variant.
    InventoryUpdated {
        /// The `window_id` value carried by this variant.
        window_id: i8,
        /// Connection-local inbound packet ordering fact.
        packet_sequence: u64,
    },
    /// Documentation for this public variant.
    SlotUpdated(SlotUpdate),
    /// An immutable server Set Slot packet paired with the inventory revision
    /// at which it was applied. Queued events can be compared to snapshots.
    InventorySlotObserved(Snapshot<SlotUpdate>),
    /// The `HeldItemChanged` variant.
    HeldItemChanged {
        /// The `slot` value carried by this variant.
        slot: u8,
    },
    /// Documentation for this public variant.
    ItemCollected(ItemCollected),
    /// Documentation for this public variant.
    DiggingAcknowledged(DiggingAcknowledgement),
    /// Documentation for this public variant.
    BlockBreakProgress(BlockBreakProgress),
    /// Documentation for this public variant.
    WindowOpened(OpenWindow),
    /// The `BookOpened` variant.
    BookOpened {
        /// The `hand` value carried by this variant.
        hand: Hand,
    },
    /// The `SignEditorOpened` variant.
    SignEditorOpened {
        /// The `position` value carried by this variant.
        position: BlockPos,
    },
    /// The `WindowClosed` variant.
    WindowClosed {
        /// The `window_id` value carried by this variant.
        window_id: i8,
    },
    /// Documentation for this public variant.
    WindowProperty(WindowProperty),
    /// Documentation for this public variant.
    WindowTransaction(WindowTransaction),
    /// Documentation for this public variant.
    MerchantOffers(crate::versions::java_1_16_1::MerchantOffers),
    /// Documentation for this public variant.
    EntitySpawned(EntityState),
    /// Documentation for this public variant.
    EntityUpdated(EntityState),
    /// The `EntitiesDestroyed` variant.
    EntitiesDestroyed {
        /// The `entity_ids` value carried by this variant.
        entity_ids: Vec<i32>,
    },
    /// The `EntityAnimation` variant.
    EntityAnimation {
        /// The `entity_id` value carried by this variant.
        entity_id: i32,
        /// The `animation` value carried by this variant.
        animation: u8,
    },
    /// The `PassengersUpdated` variant.
    PassengersUpdated {
        /// The `vehicle_id` value carried by this variant.
        vehicle_id: i32,
        /// The `passengers` value carried by this variant.
        passengers: Vec<i32>,
    },
    /// Documentation for this public variant.
    VehiclePosition(VehiclePose),
    /// The `ItemCooldown` variant.
    ItemCooldown {
        /// The `item_id` value carried by this variant.
        item_id: i32,
        /// The `ticks` value carried by this variant.
        ticks: i32,
    },
    /// Documentation for this public variant.
    Chat(ChatMessage),
    /// The `CustomPayload` variant.
    CustomPayload {
        /// The `channel` value carried by this variant.
        channel: String,
        /// The `data` value carried by this variant.
        data: Arc<[u8]>,
    },
    /// Documentation for this public variant.
    ServerBrand(String),
    /// Documentation for this public variant.
    MapUpdated(MapUpdate),
    /// Documentation for this public variant.
    UiStateUpdated(UiUpdateKind),
    /// Documentation for this public variant.
    ResourcePackRequested(ResourcePackRequest),
    /// Documentation for this public variant.
    TabCompletion(TabCompletion),
    /// Documentation for this public variant.
    WorldEvent(WorldEvent),
    /// Documentation for this public variant.
    Particle(WorldParticleEvent),
    /// Documentation for this public variant.
    Explosion(ExplosionEvent),
    /// Documentation for this public variant.
    WorldViewUpdated(WorldViewState),
    /// Documentation for this public variant.
    BlockAction(BlockActionEvent),
    /// The `CraftRecipeResponse` variant.
    CraftRecipeResponse {
        /// The `window_id` value carried by this variant.
        window_id: i8,
        /// The `recipe_id` value carried by this variant.
        recipe_id: String,
    },
    /// The `CameraChanged` variant.
    CameraChanged {
        /// The `entity_id` value carried by this variant.
        entity_id: i32,
    },
    /// The `EntityAttached` variant.
    EntityAttached {
        /// The `entity_id` value carried by this variant.
        entity_id: i32,
        /// The `attached_to` value carried by this variant.
        attached_to: Option<i32>,
    },
    /// Documentation for this public variant.
    StopSound(StopSoundEvent),
    /// Documentation for this public variant.
    NbtQueryResponse(NbtQueryResponse),
    /// The `CommandTreeUpdated` variant.
    CommandTreeUpdated,
    /// The `TagsUpdated` variant.
    TagsUpdated,
    /// The `RecipesDeclared` variant.
    RecipesDeclared,
    /// The `RecipeBookUpdated` variant.
    RecipeBookUpdated,
    /// The `StatisticsUpdated` variant.
    StatisticsUpdated,
    /// The `AdvancementsUpdated` variant.
    AdvancementsUpdated,
    /// The `PlayerListUpdated` variant.
    PlayerListUpdated {
        /// The `action` value carried by this variant.
        action: i32,
        /// The `uuids` value carried by this variant.
        uuids: Vec<[u8; 16]>,
    },
    /// The `Disconnected` variant.
    Disconnected {
        /// The `reason` value carried by this variant.
        reason: String,
    },
    /// The `Error` variant.
    Error {
        /// The `kind` value carried by this variant.
        kind: &'static str,
        /// The `message` value carried by this variant.
        message: String,
    },
    /// Documentation for this public variant.
    PositionCorrection(PositionCorrection),
}

pub(crate) struct PacketWriter {
    pub(crate) inner: OwnedWriteHalf,
    pub(crate) compression: Option<i32>,
}

struct ObservationEventQueue {
    events: VecDeque<Event>,
    omitted: u32,
}

/// A received own-velocity packet: packet ordinal and value.
type VelocityReceipt = (u64, [f64; 3]);

/// State and protocol data represented by `Bot`.
pub struct Bot {
    connection: ConnectionActor,
    reader_abort: Arc<std::sync::OnceLock<tokio::task::AbortHandle>>,
    server: Server,
    login_profile: Arc<crate::client::login::LoginProfile>,
    writer: Arc<Mutex<PacketWriter>>,
    player: Arc<Mutex<Versioned<Player>>>,
    world: Arc<Mutex<Versioned<World>>>,
    positioned: Arc<Mutex<bool>>,
    ready: Arc<Notify>,
    world_updated: Arc<Notify>,
    packet_applied: Arc<Notify>,
    events: broadcast::Sender<Event>,
    physics: Arc<Mutex<PhysicsTracker>>,
    stopped: Arc<AtomicBool>,
    terminal_emitted: Arc<AtomicBool>,
    jump_requested: Arc<AtomicBool>,
    teleport_barrier_ticks: Arc<AtomicU8>,
    motion: Arc<Mutex<Versioned<MotionState>>>,
    survival: Arc<RwLock<Versioned<SurvivalState>>>,
    world_time_observed: Arc<AtomicBool>,
    oxygen_level: Arc<Mutex<Option<u8>>>,
    local_pose: Arc<Mutex<Option<i32>>>,
    protocol_packet_sequence: Arc<AtomicU64>,
    block_geometry_revision: Arc<AtomicU64>,
    inventory: Arc<RwLock<Versioned<InventoryState>>>,
    common_receipts: Arc<Mutex<crate::client::LegacyReceipts>>,
    packet_trace: Arc<Mutex<Option<crate::client::recording::TraceCapture>>>,
    common_scoreboard: Arc<Mutex<crate::client::ui::ScoreboardLedger>>,
    common_boss_bars: Arc<Mutex<crate::client::ui::boss_bar::BossBarLedger>>,
    common_chat: Arc<Mutex<crate::client::chat::ChatLedger>>,
    common_events: Arc<std::sync::Mutex<crate::client::events::EventLedger>>,
    common_display: Arc<Mutex<crate::client::ui::display::DisplayLedger>>,
    common_teams: Arc<Mutex<crate::client::ui::teams::TeamLedger>>,
    common_player_list: Arc<Mutex<crate::client::ui::player_list::PlayerListLedger>>,
    common_motion: Arc<Mutex<Option<common_motion::NativeMotionRun>>>,
    common_control: Arc<Mutex<common_control::ContinuousControl>>,
    /// Latest received own velocity (packet ordinal, value).
    own_velocity_receipt: Arc<Mutex<Option<VelocityReceipt>>>,
    common_mining: Arc<Mutex<Option<common_mining::NativeMiningRun>>>,
    common_placement: Arc<Mutex<Option<common_placement::NativePlacementRun>>>,
    common_inventory_swap: Arc<Mutex<Option<common_inventory::NativeInventorySwap>>>,
    common_inventory_click: Arc<Mutex<Option<common_click::NativeInventoryClick>>>,
    common_crafting_take: Arc<Mutex<Option<common_crafting::NativeCraftingTake>>>,
    common_recipe_placement: Arc<Mutex<Option<common_recipe_placement::NativeRecipePlacement>>>,
    common_inventory_transfer: Arc<Mutex<Option<common_transfer::NativeInventoryTransfer>>>,
    pub(crate) flight_history: crate::client::flight::History,
    pub(crate) respawn_history: crate::client::respawn::History,
    retired_common_motion: Arc<Mutex<Option<crate::client::survival::MotionRecord>>>,
    dismount_history: crate::client::vehicle::dismount::History,
    vehicle_control_history: crate::client::vehicle::control::History,
    common_container_close: Arc<Mutex<Option<crate::client::container::ContainerCloseRecord>>>,
    common_container_open: Arc<Mutex<Option<common_container::NativeContainerOpen>>>,
    exact_window_barriers: Arc<Mutex<HashMap<(i8, i16), ExactWindowBarrier>>>,
    furnace_window_position: Arc<Mutex<Option<(i8, BlockPos)>>>,
    click_lock: Arc<Mutex<()>>,
    entities: Arc<RwLock<Versioned<EntityTracker>>>,
    last_attack: Arc<Mutex<Option<std::time::Instant>>>,
    players: Arc<RwLock<Versioned<PlayerList>>>,
    client_settings: Arc<RwLock<ClientSettings>>,
    server_brand: Arc<RwLock<Versioned<Option<String>>>>,
    maps: Arc<RwLock<Versioned<MapStore>>>,
    ui: Arc<RwLock<Versioned<UiState>>>,
    sound_sequence: Arc<std::sync::atomic::AtomicU64>,
    connected_at: std::time::Instant,
    connection_options: ConnectionOptions,
    next_tab_completion: Arc<AtomicI32>,
    resource_pack: Arc<RwLock<Versioned<Option<ResourcePackRequest>>>>,
    recipe_book: Arc<RwLock<Versioned<RecipeBookState>>>,
    statistics: Arc<RwLock<Versioned<StatisticsState>>>,
    advancements: Arc<RwLock<Versioned<AdvancementState>>>,
    world_view: Arc<RwLock<Versioned<WorldViewState>>>,
    camera_entity_id: Arc<RwLock<Versioned<Option<i32>>>>,
    next_nbt_query: Arc<AtomicI32>,
    command_tree: Arc<RwLock<Versioned<Option<CommandTree>>>>,
    tags: Arc<RwLock<Versioned<ServerTags>>>,
    server_recipes: Arc<RwLock<Versioned<ServerRecipes>>>,
    cancel: Arc<Notify>,
    external_handles: Arc<AtomicUsize>,
    counts_as_external_handle: bool,
    capture_requests: mpsc::Sender<crate::observation::CaptureCommand>,
    traversal_movement_facts_requests:
        mpsc::Sender<crate::observation::TraversalMovementFactsCommand>,
    coherent_state_gate: Arc<Mutex<()>>,
    observation_events: Arc<StdMutex<ObservationEventQueue>>,
}

impl Clone for Bot {
    fn clone(&self) -> Self {
        self.clone_with_handle(true)
    }
}

impl Drop for Bot {
    fn drop(&mut self) {
        if self.counts_as_external_handle
            && self.external_handles.fetch_sub(1, Ordering::AcqRel) == 1
        {
            self.stopped.store(true, Ordering::Release);
            self.cancel.notify_waiters();
            self.ready.notify_waiters();
            self.world_updated.notify_waiters();
            self.packet_applied.notify_waiters();
        }
    }
}

impl Bot {
    // Atomic::try_update is unavailable on our Rust 1.85 MSRV.
    #[allow(deprecated)]
    fn advance_block_geometry_revision(&self) {
        self.block_geometry_revision
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |revision| {
                revision.checked_add(1)
            })
            .expect("block geometry revision exhausted");
    }

    fn clone_with_handle(&self, counts_as_external_handle: bool) -> Self {
        if counts_as_external_handle {
            self.external_handles.fetch_add(1, Ordering::Relaxed);
        }
        Self {
            connection: self.connection.clone(),
            reader_abort: self.reader_abort.clone(),
            server: self.server.clone(),
            login_profile: self.login_profile.clone(),
            writer: self.writer.clone(),
            player: self.player.clone(),
            world: self.world.clone(),
            positioned: self.positioned.clone(),
            ready: self.ready.clone(),
            world_updated: self.world_updated.clone(),
            packet_applied: self.packet_applied.clone(),
            events: self.events.clone(),
            physics: self.physics.clone(),
            stopped: self.stopped.clone(),
            terminal_emitted: self.terminal_emitted.clone(),
            jump_requested: self.jump_requested.clone(),
            teleport_barrier_ticks: self.teleport_barrier_ticks.clone(),
            motion: self.motion.clone(),
            survival: self.survival.clone(),
            world_time_observed: self.world_time_observed.clone(),
            oxygen_level: self.oxygen_level.clone(),
            local_pose: self.local_pose.clone(),
            protocol_packet_sequence: self.protocol_packet_sequence.clone(),
            block_geometry_revision: self.block_geometry_revision.clone(),
            inventory: self.inventory.clone(),
            common_receipts: self.common_receipts.clone(),
            packet_trace: self.packet_trace.clone(),
            common_scoreboard: self.common_scoreboard.clone(),
            common_boss_bars: self.common_boss_bars.clone(),
            common_chat: self.common_chat.clone(),
            common_events: self.common_events.clone(),
            common_display: self.common_display.clone(),
            common_teams: self.common_teams.clone(),
            common_player_list: self.common_player_list.clone(),
            common_motion: self.common_motion.clone(),
            common_control: self.common_control.clone(),
            own_velocity_receipt: self.own_velocity_receipt.clone(),
            common_mining: self.common_mining.clone(),
            common_placement: self.common_placement.clone(),
            common_inventory_swap: self.common_inventory_swap.clone(),
            common_inventory_click: self.common_inventory_click.clone(),
            common_crafting_take: self.common_crafting_take.clone(),
            common_recipe_placement: self.common_recipe_placement.clone(),
            common_inventory_transfer: self.common_inventory_transfer.clone(),
            flight_history: self.flight_history.clone(),
            respawn_history: self.respawn_history.clone(),
            retired_common_motion: self.retired_common_motion.clone(),
            dismount_history: self.dismount_history.clone(),
            vehicle_control_history: self.vehicle_control_history.clone(),
            common_container_close: self.common_container_close.clone(),
            common_container_open: self.common_container_open.clone(),
            exact_window_barriers: self.exact_window_barriers.clone(),
            furnace_window_position: self.furnace_window_position.clone(),
            click_lock: self.click_lock.clone(),
            entities: self.entities.clone(),
            last_attack: self.last_attack.clone(),
            players: self.players.clone(),
            client_settings: self.client_settings.clone(),
            server_brand: self.server_brand.clone(),
            maps: self.maps.clone(),
            ui: self.ui.clone(),
            sound_sequence: self.sound_sequence.clone(),
            connected_at: self.connected_at,
            connection_options: self.connection_options,
            next_tab_completion: self.next_tab_completion.clone(),
            resource_pack: self.resource_pack.clone(),
            recipe_book: self.recipe_book.clone(),
            statistics: self.statistics.clone(),
            advancements: self.advancements.clone(),
            world_view: self.world_view.clone(),
            camera_entity_id: self.camera_entity_id.clone(),
            next_nbt_query: self.next_nbt_query.clone(),
            command_tree: self.command_tree.clone(),
            tags: self.tags.clone(),
            server_recipes: self.server_recipes.clone(),
            cancel: self.cancel.clone(),
            external_handles: self.external_handles.clone(),
            counts_as_external_handle,
            capture_requests: self.capture_requests.clone(),
            traversal_movement_facts_requests: self.traversal_movement_facts_requests.clone(),
            coherent_state_gate: self.coherent_state_gate.clone(),
            observation_events: self.observation_events.clone(),
        }
    }

    fn clone_internal(&self) -> Self {
        self.clone_with_handle(false)
    }

    pub(crate) async fn connect(
        server: Server,
        player: Player,
        chunk_storage: Arc<crate::versions::java_1_16_1::SharedChunkStorage>,
        connection_options: ConnectionOptions,
    ) -> Result<Self> {
        Self::connect_with_packet_trace(server, player, chunk_storage, connection_options, None)
            .await
    }
    pub(crate) async fn connect_with_packet_trace(
        server: Server,
        player: Player,
        chunk_storage: Arc<crate::versions::java_1_16_1::SharedChunkStorage>,
        connection_options: ConnectionOptions,
        trace_limit: Option<usize>,
    ) -> Result<Self> {
        let trace = trace_limit
            .map(|limit| crate::client::recording::TraceCapture::new(0, limit))
            .transpose()?;
        player.validate()?;
        let stream = timeout(
            connection_options.connect_timeout,
            TcpStream::connect((&*server.host, server.port)),
        )
        .await
        .context("connect timed out")?
        .context("connect failed")?;
        let local_addr = stream.local_addr().ok();
        let peer_addr = stream.peer_addr().ok();
        let (reader, writer) = stream.into_split();
        // Keep the same buffer through login and play so prefetched frames survive.
        let mut reader = tokio::io::BufReader::new(reader);
        let writer = Arc::new(Mutex::new(PacketWriter {
            inner: writer,
            compression: None,
        }));
        let mut handshake = Vec::new();
        put_varint(&mut handshake, PROTOCOL_VERSION);
        put_string(&mut handshake, &server.host);
        handshake.extend(server.port.to_be_bytes());
        put_varint(&mut handshake, 2);
        write_packet(&mut writer.lock().await.inner, None, 0, &handshake).await?;
        let mut login = Vec::new();
        put_string(&mut login, &player.username);
        write_packet(&mut writer.lock().await.inner, None, 0, &login).await?;
        let login_profile = loop {
            let compression = writer.lock().await.compression;
            let (id, payload) = timeout(
                connection_options.login_packet_timeout,
                read_packet(&mut reader, compression),
            )
            .await
            .context("login packet timed out")??;
            match id {
                0x00 => {
                    let mut p = payload.as_slice();
                    bail!("login rejected: {}", get_string(&mut p).unwrap_or_default());
                }
                0x01 => bail!("server requested encryption; only offline-mode is supported"),
                0x02 => break crate::client::login::legacy_profile(&payload, &player.username)?,
                0x03 => {
                    let mut p = payload.as_slice();
                    let threshold = get_varint(&mut p)?;
                    if threshold < 0 {
                        bail!("server sent negative compression threshold {threshold}");
                    }
                    writer.lock().await.compression = Some(threshold);
                }
                _ => {}
            }
        };
        let (events, _) = broadcast::channel(connection_options.event_channel_capacity.max(1));
        let connected_at = std::time::Instant::now();
        let control = Arc::new(RwLock::new(Versioned::new(
            ControlState::default(),
            connected_at,
        )));
        let connection = ConnectionActor::spawn(
            writer.clone(),
            connection_options.protocol_ack_timeout,
            control,
        );
        crate::lifecycle::emit_protocol_timing(|| {
            serde_json::json!({"stage":"connection_open","generation":connection.generation().get(),
                "username":player.username,"local_addr":local_addr.map(|addr|addr.to_string()),
                "peer_addr":peer_addr.map(|addr|addr.to_string())})
        });
        let (capture_requests, capture_receiver) = mpsc::channel(16);
        let (traversal_movement_facts_requests, traversal_movement_facts_receiver) =
            mpsc::channel(4);
        let bot = Self {
            connection,
            reader_abort: Arc::new(std::sync::OnceLock::new()),
            server,
            login_profile: Arc::new(login_profile),
            writer,
            player: Arc::new(Mutex::new(Versioned::new(player, connected_at))),
            world: Arc::new(Mutex::new(Versioned::new(
                World::with_storage(chunk_storage),
                connected_at,
            ))),
            positioned: Arc::new(Mutex::new(false)),
            ready: Arc::new(Notify::new()),
            world_updated: Arc::new(Notify::new()),
            packet_applied: Arc::new(Notify::new()),
            events,
            physics: Arc::new(Mutex::new(PhysicsTracker::new())),
            stopped: Arc::new(AtomicBool::new(false)),
            terminal_emitted: Arc::new(AtomicBool::new(false)),
            jump_requested: Arc::new(AtomicBool::new(false)),
            teleport_barrier_ticks: Arc::new(AtomicU8::new(0)),
            motion: Arc::new(Mutex::new(Versioned::new(
                MotionState::default(),
                connected_at,
            ))),
            survival: Arc::new(RwLock::new(Versioned::new(
                SurvivalState::default(),
                connected_at,
            ))),
            world_time_observed: Arc::new(AtomicBool::new(false)),
            oxygen_level: Arc::new(Mutex::new(None)),
            local_pose: Arc::new(Mutex::new(None)),
            protocol_packet_sequence: Arc::new(AtomicU64::new(0)),
            block_geometry_revision: Arc::new(AtomicU64::new(0)),
            inventory: Arc::new(RwLock::new(Versioned::new(
                InventoryState::default(),
                connected_at,
            ))),
            common_receipts: Arc::new(Mutex::new(crate::client::LegacyReceipts::default())),
            packet_trace: Arc::new(Mutex::new(trace)),
            common_scoreboard: Arc::new(Mutex::new(Default::default())),
            common_boss_bars: Arc::new(Mutex::new(Default::default())),
            common_chat: Arc::new(Mutex::new(Default::default())),
            common_events: Arc::new(std::sync::Mutex::new(Default::default())),
            common_display: Arc::new(Mutex::new(Default::default())),
            common_teams: Arc::new(Mutex::new(Default::default())),
            common_player_list: Arc::new(Mutex::new(Default::default())),
            common_motion: Arc::new(Mutex::new(None)),
            common_control: Arc::default(),
            own_velocity_receipt: Arc::default(),
            common_mining: Arc::new(Mutex::new(None)),
            common_placement: Arc::new(Mutex::new(None)),
            common_inventory_swap: Arc::new(Mutex::new(None)),
            common_inventory_click: Arc::new(Mutex::new(None)),
            common_crafting_take: Arc::new(Mutex::new(None)),
            common_recipe_placement: Arc::new(Mutex::new(None)),
            common_inventory_transfer: Arc::new(Mutex::new(None)),
            flight_history: Arc::default(),
            respawn_history: Arc::default(),
            retired_common_motion: Arc::default(),
            dismount_history: Arc::default(),
            vehicle_control_history: Arc::default(),
            common_container_close: Arc::new(Mutex::new(None)),
            common_container_open: Arc::new(Mutex::new(None)),
            exact_window_barriers: Arc::new(Mutex::new(HashMap::new())),
            furnace_window_position: Arc::new(Mutex::new(None)),
            click_lock: Arc::new(Mutex::new(())),
            entities: Arc::new(RwLock::new(Versioned::new(
                EntityTracker::default(),
                connected_at,
            ))),
            last_attack: Arc::new(Mutex::new(None)),
            players: Arc::new(RwLock::new(Versioned::new(
                PlayerList::default(),
                connected_at,
            ))),
            client_settings: Arc::new(RwLock::new(ClientSettings::default())),
            server_brand: Arc::new(RwLock::new(Versioned::new(None, connected_at))),
            maps: Arc::new(RwLock::new(Versioned::new(
                MapStore::default(),
                connected_at,
            ))),
            ui: Arc::new(RwLock::new(Versioned::new(
                UiState::default(),
                connected_at,
            ))),
            sound_sequence: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            connected_at,
            connection_options,
            next_tab_completion: Arc::new(AtomicI32::new(1)),
            resource_pack: Arc::new(RwLock::new(Versioned::new(None, connected_at))),
            recipe_book: Arc::new(RwLock::new(Versioned::new(
                RecipeBookState::default(),
                connected_at,
            ))),
            statistics: Arc::new(RwLock::new(Versioned::new(
                StatisticsState::default(),
                connected_at,
            ))),
            advancements: Arc::new(RwLock::new(Versioned::new(
                AdvancementState::default(),
                connected_at,
            ))),
            world_view: Arc::new(RwLock::new(Versioned::new(
                WorldViewState::default(),
                connected_at,
            ))),
            camera_entity_id: Arc::new(RwLock::new(Versioned::new(None, connected_at))),
            next_nbt_query: Arc::new(AtomicI32::new(1)),
            command_tree: Arc::new(RwLock::new(Versioned::new(None, connected_at))),
            tags: Arc::new(RwLock::new(Versioned::new(
                ServerTags::default(),
                connected_at,
            ))),
            server_recipes: Arc::new(RwLock::new(Versioned::new(
                ServerRecipes::default(),
                connected_at,
            ))),
            cancel: Arc::new(Notify::new()),
            external_handles: Arc::new(AtomicUsize::new(1)),
            counts_as_external_handle: true,
            capture_requests,
            traversal_movement_facts_requests,
            coherent_state_gate: Arc::new(Mutex::new(())),
            observation_events: Arc::new(StdMutex::new(ObservationEventQueue {
                events: VecDeque::with_capacity(256),
                omitted: 0,
            })),
        };
        bot.emit(Event::Login);
        let background = bot.clone_internal();
        let reader_task = tokio::spawn(async move {
            background
                .read_loop(reader, capture_receiver, traversal_movement_facts_receiver)
                .await
        });
        let _ = bot.reader_abort.set(reader_task.abort_handle());
        let supervisor = bot.clone_internal();
        tokio::spawn(async move {
            match reader_task.await {
                Ok(Ok(())) => {
                    if supervisor.connection.lifecycle() == ConnectionState::Disconnected {
                        supervisor.emit(Event::Disconnected {
                            reason: "server disconnect confirmed".to_owned(),
                        });
                    } else {
                        supervisor
                            .connection
                            .mark_unknown(
                                "reader_unconfirmed_end",
                                "reader stopped without a confirmed transport end".to_owned(),
                            )
                            .await;
                        supervisor.emit(Event::Error {
                            kind: "connection",
                            message: "reader stopped without a confirmed transport end".to_owned(),
                        });
                    }
                }
                Ok(Err(error)) => {
                    let requested_transport_end = supervisor.connection.lifecycle()
                        == ConnectionState::Disconnecting
                        && error.diagnostic().chain().any(|cause| {
                            cause.downcast_ref::<std::io::Error>().is_some_and(|io| {
                                matches!(
                                    io.kind(),
                                    std::io::ErrorKind::UnexpectedEof
                                        | std::io::ErrorKind::ConnectionReset
                                        | std::io::ErrorKind::ConnectionAborted
                                        | std::io::ErrorKind::BrokenPipe
                                )
                            })
                        });
                    if requested_transport_end {
                        supervisor
                            .connection
                            .mark_terminal(TerminalClassification::Disconnected)
                            .await;
                        supervisor.emit(Event::Disconnected {
                            reason: "transport ended after disconnect request".to_owned(),
                        });
                    } else {
                        supervisor
                            .connection
                            .mark_unknown("reader_error", format!("{:#}", error.diagnostic()))
                            .await;
                        supervisor.emit(Event::Error {
                            kind: "connection",
                            message: error.to_string(),
                        });
                    }
                }
                Err(error) => {
                    supervisor
                        .connection
                        .mark_unknown("reader_task_failed", error.to_string())
                        .await;
                    supervisor.emit(Event::Error {
                        kind: "connection",
                        message: format!("reader task terminated unexpectedly: {error}"),
                    });
                }
            }
            supervisor.physics.lock().await.record_disconnect();
            supervisor.stopped.store(true, Ordering::Release);
            supervisor.packet_applied.notify_waiters();
        });
        let physics = bot.clone_internal();
        tokio::spawn(async move { physics.control_loop().await });
        Ok(bot)
    }
    /// Performs the `subscribe` operation.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }
    pub(crate) const fn connection_id(&self) -> u64 {
        self.connection.generation().get()
    }
    pub(crate) fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    /// Returns the process-local identity of this exact transport connection.
    #[must_use]
    pub const fn connection_generation(&self) -> ConnectionGeneration {
        self.connection.generation()
    }
    /// Returns the latest lifecycle fact published by the connection actor.
    #[must_use]
    pub fn connection_state(&self) -> ConnectionState {
        self.connection.lifecycle()
    }
    /// Irreversibly fence this connection generation without waiting for capture,
    /// writer admission, pending writes or ordinary disconnect cleanup.
    ///
    /// This is emergency quarantine, not successful cancellation or clean logout.
    /// Already admitted writes can have unknown partial effects. Native tasks are
    /// aborted and writer shutdown is scheduled; transport end is not certified.
    /// This Bot cannot be reconnected or reused after this call.
    #[must_use]
    pub fn revoke_connection(&self) -> GenerationRevocation {
        let receipt = self.connection.revoke();
        self.stopped.store(true, Ordering::Release);
        if let Some(reader) = self.reader_abort.get() {
            reader.abort();
        }
        self.cancel.notify_waiters();
        self.packet_applied.notify_waiters();
        self.ready.notify_waiters();
        self.world_updated.notify_waiters();
        receipt
    }

    /// Creates an operation correlation context for this connection.
    #[must_use]
    pub const fn operation_context(&self, source_observation_sequence: u64) -> OperationContext {
        OperationContext {
            generation: self.connection_generation(),
            source_observation_sequence,
        }
    }
    /// Asks the connection actor to admit an operation before any packet write.
    ///
    /// This read/validation API is public because external controllers
    /// must bind every low-level operation to the observation that selected it.
    pub async fn admit_operation(
        &self,
        context: OperationContext,
        class: OperationClass,
    ) -> std::result::Result<(), OperationAdmissionError> {
        self.connection.admit(context, class).await
    }

    /// Dispatches one low-level client operation through the actor-owned writer.
    ///
    /// The context is mandatory so a primitive cannot be accidentally sent
    /// using a stale connection or an unrelated observation. This API reports
    /// only the transport dispatch stage; protocol acknowledgement and fresh
    /// semantic observation remain separate stages.
    pub async fn dispatch_operation(
        &self,
        context: OperationContext,
        operation: Operation,
    ) -> std::result::Result<DispatchOutcome, DispatchError> {
        self.dispatch_operation_with_diagnostic(context, operation, None)
            .await
    }

    /// Dispatches a primitive with an optional read-only diagnostic
    /// correlation supplied by the external controller.
    pub async fn dispatch_operation_with_diagnostic(
        &self,
        context: OperationContext,
        operation: Operation,
        diagnostic_correlation: Option<crate::DiagnosticCorrelationId>,
    ) -> std::result::Result<DispatchOutcome, DispatchError> {
        let diagnostic_correlation =
            diagnostic_correlation.filter(|_| crate::lifecycle::dig_lifecycle_trace_enabled());
        let dig_diagnostic = match (&operation, diagnostic_correlation) {
            (Operation::DigStart { position, face }, Some(correlation)) => {
                Some(crate::lifecycle::DigWriteDiagnostic::new(
                    correlation,
                    "start",
                    crate::DiggingStatus::Started as i32,
                    *position,
                    *face,
                ))
            }
            _ => None,
        };
        let (packet_id, payload) = operation
            .encode()
            .map_err(|_| DispatchError::InvalidInput)?;
        match operation {
            Operation::BlockInteraction {
                position, sneak, ..
            }
            | Operation::PlacementInteraction {
                position, sneak, ..
            } => {
                let entity_id = self
                    .player
                    .lock()
                    .await
                    .entity_id
                    .ok_or(DispatchError::InvalidInput)?;
                let restore_sneak = self.connection.control_snapshot().await.value.sneak;
                let packets = vec![
                    encode_entity_action_packet(entity_id, sneak.is_required()),
                    (packet_id, payload),
                    encode_entity_action_packet(entity_id, restore_sneak),
                ];
                let furnace_position =
                    matches!(operation, Operation::BlockInteraction { .. }).then_some(position);
                self.connection
                    .dispatch_interaction_batch(
                        context,
                        OperationClass::Normal,
                        furnace_position,
                        packets,
                    )
                    .await
                    .map_err(DispatchError::Admission)
            }
            _ => self
                .connection
                .dispatch_operation_with_diagnostic(
                    context,
                    OperationClass::Normal,
                    packet_id,
                    &payload,
                    dig_diagnostic,
                )
                .await
                .map_err(DispatchError::Admission),
        }
    }

    /// Dispatches one Caller-selected main-hand operation without exposing the
    /// connection-local protocol transaction identity.
    pub async fn dispatch_equip(
        &self,
        context: OperationContext,
        operation: EquipOperation,
    ) -> std::result::Result<DispatchOutcome, DispatchError> {
        let coherent_state = self.coherent_state_gate.lock().await;
        match operation {
            EquipOperation::SelectEmptyHotbar { hotbar_slot } => {
                let inventory = self.inventory.read().await;
                if hotbar_slot > 8
                    || !inventory
                        .player_slots()
                        .get(36 + usize::from(hotbar_slot))
                        .is_some_and(Option::is_none)
                {
                    return Err(DispatchError::InvalidInput);
                }
                drop(inventory);
                let outcome = self
                    .dispatch_operation(context, Operation::SelectHotbar { hotbar_slot })
                    .await?;
                if outcome == DispatchOutcome::Dispatched {
                    self.inventory.write().await.selected_hotbar = hotbar_slot;
                }
                drop(coherent_state);
                Ok(outcome)
            }
            EquipOperation::SelectHotbar {
                hotbar_slot,
                expected_item,
            } => {
                let inventory = self.inventory.read().await;
                let matches = inventory
                    .player_slots()
                    .get(36 + usize::from(hotbar_slot))
                    .and_then(Option::as_ref)
                    .is_some_and(|item| slot_matches(item, &expected_item));
                if !matches {
                    return Err(DispatchError::InvalidInput);
                }
                drop(inventory);
                let outcome = self
                    .dispatch_operation(context, Operation::SelectHotbar { hotbar_slot })
                    .await?;
                if outcome == DispatchOutcome::Dispatched {
                    self.inventory.write().await.selected_hotbar = hotbar_slot;
                }
                drop(coherent_state);
                Ok(outcome)
            }
            EquipOperation::SwapIntoSelectedHotbar {
                inventory_slot,
                selected_hotbar_slot,
                expected_source,
                expected_destination,
                cursor_must_be_empty,
            } => {
                let inventory = self.inventory.read().await;
                let selected_match = inventory.selected_hotbar == selected_hotbar_slot;
                let cursor_empty = inventory.cursor.is_none();
                let cursor_ok = !cursor_must_be_empty || cursor_empty;
                let source = inventory
                    .player_slots()
                    .get(usize::from(inventory_slot))
                    .and_then(Option::as_ref)
                    .filter(|item| slot_matches(item, &expected_source))
                    .cloned();
                let destination = inventory
                    .player_slots()
                    .get(36 + usize::from(selected_hotbar_slot))
                    .and_then(Option::as_ref)
                    .cloned();
                let destination_match =
                    optional_slot_matches(destination.as_ref(), expected_destination.as_ref());
                if !selected_match || !cursor_ok {
                    return Err(DispatchError::InvalidInput);
                }
                let Some(source) = source else {
                    return Err(DispatchError::InvalidInput);
                };
                if !destination_match {
                    return Err(DispatchError::InvalidInput);
                }
                drop(inventory);
                let transaction = match self
                    .dispatch_acknowledged(
                        context,
                        AcknowledgedOperation::WindowClick {
                            window_id: 0,
                            slot: i16::try_from(inventory_slot)
                                .map_err(|_| DispatchError::InvalidInput)?,
                            button: i8::try_from(selected_hotbar_slot)
                                .map_err(|_| DispatchError::InvalidInput)?,
                            mode: ClickMode::Hotbar,
                            clicked: Some(source.clone()),
                        },
                    )
                    .await
                {
                    Ok(transaction) => transaction,
                    Err(error) => return Err(error),
                };
                drop(coherent_state);
                let outcome = transaction.wait().await;
                if outcome == DispatchOutcome::Acknowledged {
                    let _coherent_state = self.coherent_state_gate.lock().await;
                    let mut inventory = self.inventory.write().await;
                    apply_accepted_equip_swap(
                        &mut inventory,
                        inventory_slot,
                        selected_hotbar_slot,
                        &source,
                        destination.as_ref(),
                    );
                }
                Ok(outcome)
            }
        }
    }

    /// Dispatches an ordered window click sequence. Every click is
    /// revalidated against coherent packet state before the actor allocates
    /// its connection-local transaction identity.
    pub async fn dispatch_window_clicks(
        &self,
        context: OperationContext,
        execution: WindowClickSequence,
    ) -> std::result::Result<DispatchOutcome, DispatchError> {
        execution.validate()?;
        self.dispatch_exact_window_clicks(
            context,
            execution.window_id,
            execution.close_window_after,
            execution.clicks,
        )
        .await
    }

    async fn dispatch_exact_window_clicks(
        &self,
        context: OperationContext,
        window_id: i8,
        close_window_after: bool,
        clicks: Vec<crate::WindowClick>,
    ) -> std::result::Result<DispatchOutcome, DispatchError> {
        if clicks.is_empty() {
            return Err(DispatchError::InvalidInput);
        }
        let _click_order = self.click_lock.lock().await;
        let mut acknowledged = 0usize;
        for (index, click) in clicks.iter().enumerate() {
            let successor = clicks.get(index + 1).cloned();
            let coherent_state = self.coherent_state_gate.lock().await;
            let inventory = self.inventory.read().await;
            if (window_id != 0
                && inventory.open_window.as_ref().map(|window| window.id) != Some(window_id))
                || (window_id == 0 && inventory.open_window.is_some())
            {
                return Ok(if acknowledged == 0 {
                    DispatchOutcome::Rejected
                } else {
                    DispatchOutcome::DeliveryUnknown
                });
            }
            let observed = inventory
                .windows
                .get(&window_id)
                .and_then(|slots| {
                    usize::try_from(click.slot)
                        .ok()
                        .and_then(|slot| slots.get(slot))
                })
                .and_then(Option::as_ref);
            let slot_match = optional_slot_matches(observed, click.expected_item.as_ref());
            let cursor_match =
                optional_slot_matches(inventory.cursor.as_ref(), click.expected_cursor.as_ref());
            if !slot_match || !cursor_match {
                return Ok(if acknowledged == 0 {
                    DispatchOutcome::Rejected
                } else {
                    DispatchOutcome::DeliveryUnknown
                });
            }
            let clicked = observed.cloned();
            let slot_before = clicked.clone();
            let cursor_before = inventory.cursor.clone();
            drop(inventory);
            let transaction = match self
                .dispatch_acknowledged(
                    context,
                    AcknowledgedOperation::WindowClick {
                        window_id,
                        slot: click.slot,
                        button: click.button,
                        mode: click.mode,
                        clicked,
                    },
                )
                .await
            {
                Ok(transaction) => transaction,
                Err(error) if acknowledged == 0 => return Err(error),
                Err(_) => return Ok(DispatchOutcome::DeliveryUnknown),
            };
            let action = transaction
                .window_action()
                .ok_or(DispatchError::InvalidInput)?;
            self.inventory.write().await.pending_clicks.insert(
                (window_id, action),
                PendingClick {
                    window_id,
                    action,
                    slot: click.slot,
                    button: click.button,
                    mode: click.mode,
                    slot_before,
                    cursor_before,
                    prediction: click.prediction.clone(),
                },
            );
            self.exact_window_barriers.lock().await.insert(
                (window_id, action),
                ExactWindowBarrier {
                    successor,
                    confirmation_seen: false,
                },
            );
            drop(coherent_state);
            let monitor = self.clone_internal();
            let (outcome_tx, outcome_rx) = oneshot::channel();
            tokio::spawn(async move {
                let outcome = transaction.wait().await;
                if outcome != DispatchOutcome::Acknowledged {
                    let _gate = monitor.coherent_state_gate.lock().await;
                    monitor
                        .inventory
                        .write()
                        .await
                        .pending_clicks
                        .remove(&(window_id, action));
                    monitor
                        .exact_window_barriers
                        .lock()
                        .await
                        .remove(&(window_id, action));
                }
                let _ = outcome_tx.send(outcome);
            });
            match outcome_rx.await.unwrap_or(DispatchOutcome::DeliveryUnknown) {
                DispatchOutcome::Acknowledged => acknowledged += 1,
                DispatchOutcome::Rejected if acknowledged == 0 => {
                    return Ok(DispatchOutcome::Rejected);
                }
                DispatchOutcome::Rejected | DispatchOutcome::DeliveryUnknown => {
                    return Ok(DispatchOutcome::DeliveryUnknown);
                }
                DispatchOutcome::Dispatched => {
                    return Ok(DispatchOutcome::DeliveryUnknown);
                }
            }
        }
        if close_window_after {
            let outcome = match self
                .connection
                .dispatch_operation(context, OperationClass::Cleanup, 0x0a, &[window_id as u8])
                .await
            {
                Ok(outcome) => outcome,
                Err(_) => return Ok(DispatchOutcome::DeliveryUnknown),
            };
            if outcome != DispatchOutcome::Dispatched {
                return Ok(DispatchOutcome::DeliveryUnknown);
            }
            self.clear_local_window(window_id).await;
        }
        Ok(DispatchOutcome::Acknowledged)
    }

    async fn commit_satisfied_window_barriers(&self) {
        let inventory = self.inventory.read().await;
        let mut barriers = self.exact_window_barriers.lock().await;
        let ready = barriers
            .iter()
            .filter_map(|(&(window_id, action), barrier)| {
                (barrier.confirmation_seen
                    && barrier.successor.as_ref().is_none_or(|successor| {
                        click_precondition_matches(&inventory, window_id, successor)
                    }))
                .then_some((window_id, action))
            })
            .collect::<Vec<_>>();
        for identity in &ready {
            barriers.remove(identity);
        }
        drop(barriers);
        drop(inventory);
        for (window_id, action) in ready {
            let _ = self
                .connection
                .commit_window_barrier(window_id, action)
                .await;
        }
    }

    /// Dispatches the Caller-selected yaw and pitch as one rotation-only packet.
    ///
    /// The on-ground bit is supplied by the caller observation that selected
    /// this operation; the client does not re-read or derive it.
    ///
    /// # Errors
    ///
    /// Returns a typed admission or transport error before semantic success is
    /// implied.
    pub async fn dispatch_rotation(
        &self,
        context: OperationContext,
        yaw: f32,
        pitch: f32,
        on_ground: bool,
    ) -> std::result::Result<DispatchOutcome, DispatchError> {
        // A server does not echo ordinary client rotation packets. Serialize
        // the admitted write with coherent capture and project the exact sent
        // orientation into the local player state only after delivery is
        // known. This is client input state, not Minecraft semantic success.
        let _coherent_state = self.coherent_state_gate.lock().await;
        let outcome = self
            .dispatch_operation(
                context,
                Operation::LookRotation {
                    yaw,
                    pitch,
                    on_ground,
                },
            )
            .await?;
        if matches!(
            outcome,
            DispatchOutcome::Dispatched | DispatchOutcome::Acknowledged
        ) {
            let mut player = self.player.lock().await;
            player.yaw = yaw;
            player.pitch = pitch;
            self.common_receipts.lock().await.rotation_source =
                Some(crate::client::ValueSource::Submitted);
        }
        Ok(outcome)
    }

    /// Dispatches one protocol-acknowledged primitive with a client-owned
    /// transaction identity and acknowledgement deadline.
    ///
    /// The returned transaction is a protocol fact only. Callers must use a
    /// fresh client observation to establish any semantic game effect.
    pub async fn dispatch_acknowledged(
        &self,
        context: OperationContext,
        operation: AcknowledgedOperation,
    ) -> std::result::Result<ProtocolTransaction, DispatchError> {
        self.dispatch_acknowledged_with_diagnostic(context, operation, None)
            .await
    }

    /// Dispatches an acknowledged primitive with an optional read-only
    /// diagnostic correlation supplied by the external controller.
    pub async fn dispatch_acknowledged_with_diagnostic(
        &self,
        context: OperationContext,
        operation: AcknowledgedOperation,
        diagnostic_correlation: Option<crate::DiagnosticCorrelationId>,
    ) -> std::result::Result<ProtocolTransaction, DispatchError> {
        self.connection
            .dispatch_acknowledged_with_diagnostic(
                context,
                OperationClass::Normal,
                operation,
                diagnostic_correlation,
            )
            .await
    }

    /// Dispatches one finite cleanup primitive through the disconnect barrier.
    ///
    /// Cleanup remains available in `Disconnecting`, while ordinary primitive
    /// operations are rejected there. Cleanup is not evidence of semantic
    /// success and an uncertain write is reported as `DeliveryUnknown`.
    pub async fn dispatch_cleanup(
        &self,
        context: OperationContext,
        operation: CleanupOperation,
    ) -> std::result::Result<CleanupDispatchOutcome, DispatchError> {
        self.dispatch_cleanup_with_diagnostic(context, operation, None)
            .await
    }

    /// Dispatches cleanup with an optional read-only Dig correlation.
    ///
    /// This is public only so the caller can carry its opaque diagnostic
    /// identity across the crate boundary to the cleanup writer. The identity
    /// has no admission or semantic-completion authority.
    pub async fn dispatch_cleanup_with_diagnostic(
        &self,
        context: OperationContext,
        operation: CleanupOperation,
        diagnostic_correlation: Option<crate::DiagnosticCorrelationId>,
    ) -> std::result::Result<CleanupDispatchOutcome, DispatchError> {
        if matches!(operation, CleanupOperation::ControlClear) {
            self.connection
                .replace_control(context, OperationClass::Cleanup, ControlState::default())
                .await
                .map_err(DispatchError::Admission)?;
            return Ok(CleanupDispatchOutcome::AppliedLocally);
        }
        let Some((packet_id, payload)) = operation
            .encode()
            .map_err(|_| DispatchError::InvalidInput)?
        else {
            unreachable!("ControlClear is handled above");
        };
        let dig_diagnostic = match (
            &operation,
            diagnostic_correlation.filter(|_| crate::lifecycle::dig_lifecycle_trace_enabled()),
        ) {
            (CleanupOperation::DigCancel { position, face }, Some(correlation)) => {
                Some(crate::lifecycle::DigWriteDiagnostic::new(
                    correlation,
                    "cancel",
                    crate::DiggingStatus::Cancelled as i32,
                    *position,
                    *face,
                ))
            }
            _ => None,
        };
        let outcome = self
            .connection
            .dispatch_operation_with_diagnostic(
                context,
                OperationClass::Cleanup,
                packet_id,
                &payload,
                dig_diagnostic,
            )
            .await
            .map_err(DispatchError::Admission)?;
        if matches!(operation, CleanupOperation::CloseWindow { .. })
            && matches!(
                outcome,
                DispatchOutcome::Dispatched | DispatchOutcome::Acknowledged
            )
        {
            // The close packet is a finite cleanup operation.  Keep the
            // predicted cache aligned with the already-written packet so the
            // The caller can observe the window as absent and schedule its bounded
            // reopen interval.  This is local cache state, not proof that the
            // server applied the close; an uncertain write remains unknown.
            if let CleanupOperation::CloseWindow { window_id } = operation {
                self.clear_local_window(window_id).await;
            }
        }
        Ok(match outcome {
            DispatchOutcome::Dispatched => CleanupDispatchOutcome::Dispatched,
            DispatchOutcome::Acknowledged => CleanupDispatchOutcome::Acknowledged,
            DispatchOutcome::Rejected => CleanupDispatchOutcome::Rejected,
            DispatchOutcome::DeliveryUnknown => CleanupDispatchOutcome::DeliveryUnknown,
        })
    }

    async fn clear_local_window(&self, window_id: i8) {
        let _coherent_state = self.coherent_state_gate.lock().await;
        self.clear_local_window_unlocked(window_id).await;
    }
    async fn clear_local_window_unlocked(&self, window_id: i8) {
        let mut inventory = self.inventory.write().await;
        if inventory
            .open_window
            .as_ref()
            .is_none_or(|window| window.id != window_id)
        {
            return;
        }
        inventory.open_window = None;
        inventory.last_transaction = None;
        inventory.merchant_offers = None;
        inventory.windows.remove(&window_id);
        inventory.properties.retain(|(id, _), _| *id != window_id);
        inventory
            .pending_clicks
            .retain(|(id, _), _| *id != window_id);
        let mut furnace_window_position = self.furnace_window_position.lock().await;
        if furnace_window_position.is_some_and(|(id, _)| id == window_id) {
            *furnace_window_position = None;
        }
        drop(furnace_window_position);
        drop(inventory);
        self.emit(Event::WindowClosed { window_id });
    }
    /// Waits without an internal timeout for a confirmed or unknown terminal state.
    /// The outer PlaySupervisor owns the logout deadline.
    pub async fn wait_for_transport_end(&self) -> ConnectionState {
        self.connection.wait_for_terminal().await
    }
    /// Captures all requested raw domains in one connection-actor turn.
    ///
    /// Packet application cannot interleave with this capture. The returned
    /// sequence is scoped to this connection generation and must not be reused
    /// as a application `StateRevision`.
    pub async fn capture_coherent_observation(
        &self,
        request: crate::CoherentObservationRequest,
    ) -> Result<crate::CoherentObservation> {
        let request = request.validate()?;
        if self.connection.lifecycle() != ConnectionState::Ready {
            bail!("coherent observation requires a ready connection");
        }
        let (reply, result) = oneshot::channel();
        self.capture_requests
            .send(crate::observation::CaptureCommand { request, reply })
            .await
            .map_err(|_| {
                crate::Error::new(
                    crate::ErrorKind::State,
                    anyhow::anyhow!("connection actor no longer accepts observation capture"),
                )
            })?;
        result.await.map_err(|_| {
            crate::Error::new(
                crate::ErrorKind::State,
                anyhow::anyhow!("connection actor dropped observation capture result"),
            )
        })?
    }

    /// Captures a bounded raw movement-facts snapshot in one connection-actor
    /// turn. This is an internal typed integration boundary for the application
    /// service; it is not a JSON or public application semantic contract.
    pub async fn capture_movement_snapshot(
        &self,
        request: crate::MovementSnapshotRequest,
    ) -> Result<crate::MovementSnapshot> {
        let request = request.validate()?;
        if request.expected_generation != self.connection_generation() {
            bail!("movement facts request belongs to a stale client generation");
        }
        let (reply, result) = oneshot::channel();
        self.traversal_movement_facts_requests
            .send(crate::observation::TraversalMovementFactsCommand { request, reply })
            .await
            .map_err(|_| {
                crate::Error::new(
                    crate::ErrorKind::State,
                    anyhow::anyhow!("connection actor no longer accepts movement facts capture"),
                )
            })?;
        result.await.map_err(|_| {
            crate::Error::new(
                crate::ErrorKind::State,
                anyhow::anyhow!("connection actor dropped movement facts capture result"),
            )
        })?
    }

    /// Queries matching blocks and loaded coverage under one coherent client
    /// state gate. The result is raw packet-cache evidence only; callers own
    /// all candidate selection, route, and semantic decisions.
    pub async fn query_loaded_blocks(
        &self,
        request: crate::BlockQuery,
    ) -> Result<crate::BlockQuerySnapshot> {
        request.validate()?;
        let _coherent_state = self.coherent_state_gate.lock().await;
        if self.connection.lifecycle() != ConnectionState::Ready {
            bail!("loaded resource query requires a ready connection");
        }
        if request.expected_generation != self.connection_generation() {
            bail!("loaded resource query belongs to a stale client generation");
        }
        if self.survival.read().await.dimension.as_deref()
            != Some(request.expected_dimension.as_str())
        {
            bail!("loaded resource query belongs to a stale dimension");
        }
        let generation = self.connection_generation();
        let world = self.world.lock().await;
        let min_chunk_x = request.region.min.x.div_euclid(16);
        let max_chunk_x = request.region.max.x.div_euclid(16);
        let min_chunk_z = request.region.min.z.div_euclid(16);
        let max_chunk_z = request.region.max.z.div_euclid(16);
        let mut missing_chunks = 0_u32;
        for chunk_x in min_chunk_x..=max_chunk_x {
            for chunk_z in min_chunk_z..=max_chunk_z {
                if world
                    .chunk_snapshot(crate::ChunkPos {
                        x: chunk_x,
                        z: chunk_z,
                    })
                    .is_none()
                {
                    missing_chunks = missing_chunks.saturating_add(1);
                }
            }
        }
        let (candidates, omitted_candidates) = match world.query_resource_index(
            request.region,
            request.block_name.as_str(),
            usize::from(request.limit),
        ) {
            crate::world::ResourceIndexQuery::Complete {
                candidates,
                omitted,
            } => (candidates, omitted),
            crate::world::ResourceIndexQuery::Unsupported => {
                let mut candidates = Vec::new();
                let mut omitted = 0_u32;
                for x in request.region.min.x..=request.region.max.x {
                    for y in request.region.min.y..=request.region.max.y {
                        for z in request.region.min.z..=request.region.max.z {
                            let Some(state_id) = world.block(x, y, z) else {
                                continue;
                            };
                            if crate::block_name_from_state(state_id)
                                != Some(request.block_name.as_str())
                            {
                                continue;
                            }
                            if candidates.len() < usize::from(request.limit) {
                                candidates.push(crate::BlockPos { x, y, z });
                            } else {
                                omitted = omitted.saturating_add(1);
                            }
                        }
                    }
                }
                (candidates, omitted)
            }
            crate::world::ResourceIndexQuery::Unknown => {
                bail!("loaded resource query index is incomplete")
            }
        };
        let block_geometry_revision = self.block_geometry_revision.load(Ordering::Acquire);
        let inventory_revision = self.inventory.read().await.revision();
        Ok(crate::BlockQuerySnapshot {
            capture: crate::CaptureIdentity {
                generation,
                block_geometry_revision,
                inventory_revision,
            },
            coverage: if missing_chunks == 0 {
                crate::BlockQueryCoverage::Complete
            } else {
                crate::BlockQueryCoverage::Partial { missing_chunks }
            },
            candidates,
            omitted_candidates,
        })
    }

    /// Reads one Caller-selected traversal region under the coherent state gate.
    /// The returned facts carry no route, stand, target, or action authority.
    pub async fn query_geometry(
        &self,
        request: crate::GeometryQuery,
    ) -> Result<crate::GeometrySnapshot> {
        request.validate()?;
        let _coherent_state = self.coherent_state_gate.lock().await;
        if self.connection.lifecycle() != ConnectionState::Ready {
            bail!("traversal geometry query requires a ready connection");
        }
        if request.expected_capture.generation != self.connection_generation() {
            bail!("traversal geometry query belongs to a stale client generation");
        }
        if self.survival.read().await.dimension.as_deref()
            != Some(request.expected_dimension.as_str())
        {
            bail!("traversal geometry query belongs to a stale dimension");
        }
        let player = self.player.lock().await.snapshot();
        if !player.value.x.is_finite() || !player.value.y.is_finite() || !player.value.z.is_finite()
        {
            bail!("traversal geometry query captured a non-finite player origin");
        }
        let world = self.world.lock().await.map_snapshot(|world| {
            let mut blocks = Vec::new();
            for x in request.region.min.x..=request.region.max.x {
                for y in request.region.min.y..=request.region.max.y {
                    for z in request.region.min.z..=request.region.max.z {
                        let position = crate::BlockPos { x, y, z };
                        let Some(state_id) = world.block(x, y, z) else {
                            blocks.push(crate::GeometryBlock::Unloaded { position });
                            continue;
                        };
                        let Some(name) = crate::block_name_from_state(state_id) else {
                            blocks.push(crate::GeometryBlock::Unknown { position, state_id });
                            continue;
                        };
                        let Some(collision) = crate::block_collision(state_id) else {
                            blocks.push(crate::GeometryBlock::Unknown { position, state_id });
                            continue;
                        };
                        let Some(support_surface) = crate::block_support_surface(state_id) else {
                            blocks.push(crate::GeometryBlock::Unknown { position, state_id });
                            continue;
                        };
                        let Some(properties) = crate::block_state_properties(state_id) else {
                            blocks.push(crate::GeometryBlock::Unknown { position, state_id });
                            continue;
                        };
                        blocks.push(crate::GeometryBlock::Loaded {
                            position,
                            state_id,
                            name: name.to_owned(),
                            collision,
                            support_surface,
                            properties,
                        });
                    }
                }
            }
            blocks
        });
        let block_geometry_revision = self.block_geometry_revision.load(Ordering::Acquire);
        let inventory_revision = self.inventory.read().await.revision();
        let generation = self.connection_generation();
        if generation != request.expected_capture.generation
            || self.connection.lifecycle() != ConnectionState::Ready
        {
            bail!("traversal geometry query crossed a connection generation boundary");
        }
        Ok(crate::GeometrySnapshot {
            capture: crate::CaptureIdentity {
                generation,
                block_geometry_revision,
                inventory_revision,
            },
            evaluated_origin: crate::Vec3 {
                x: player.value.x,
                y: player.value.y,
                z: player.value.z,
            },
            blocks: world.value,
        })
    }

    /// Captures every chunk currently loaded by this client as one immutable,
    /// low-copy Rust integration object.
    ///
    /// Public visibility is required for the in-process service-to-runtime
    /// Production proof boundary. This method exposes packet facts only and
    /// grants no semantic, route, target, or action authority.
    pub async fn capture_loaded_geometry(&self) -> Result<crate::LoadedGeometrySnapshot> {
        const CELLS_PER_CHUNK: usize = 16 * 16 * 256;
        const SECTIONS_PER_CHUNK: usize = 16;

        let _coherent_state = self.coherent_state_gate.lock().await;
        if self.connection.lifecycle() != ConnectionState::Ready {
            bail!("loaded geometry capture requires a ready connection");
        }
        let generation = self.connection_generation();
        let dimension = self
            .survival
            .read()
            .await
            .dimension
            .clone()
            .context("loaded geometry capture requires a known dimension")?;
        let (loaded_chunks, sections) = self
            .world
            .lock()
            .await
            .map_snapshot(|world| world.loaded_geometry_parts())
            .value;
        if loaded_chunks.len() > self.connection_options.max_chunks
            || sections.len()
                > loaded_chunks
                    .len()
                    .checked_mul(SECTIONS_PER_CHUNK)
                    .context("loaded geometry section bound overflow")?
        {
            bail!("loaded geometry capture exceeds configured world bounds");
        }
        let logical_cell_count = loaded_chunks
            .len()
            .checked_mul(CELLS_PER_CHUNK)
            .context("loaded geometry logical cell count overflow")?;
        let block_geometry_revision = self.block_geometry_revision.load(Ordering::Acquire);
        let inventory_revision = self.inventory.read().await.revision();
        if generation != self.connection_generation()
            || self.connection.lifecycle() != ConnectionState::Ready
        {
            bail!("loaded geometry capture crossed a connection generation boundary");
        }
        Ok(crate::LoadedGeometrySnapshot {
            capture: crate::CaptureIdentity {
                generation,
                block_geometry_revision,
                inventory_revision,
            },
            dimension,
            loaded_chunks,
            sections,
            logical_cell_count,
        })
    }
    /// Public constant `fn`.
    pub const fn client_info() -> ClientInfo {
        CLIENT_INFO
    }
    /// Public constant `fn`.
    pub const fn protocol_info() -> ProtocolInfo {
        CLIENT_INFO.protocol
    }
    /// Public constant `fn`.
    pub const fn capabilities() -> ClientCapabilities {
        CLIENT_INFO.capabilities
    }
    /// Performs the `server_info` operation.
    pub fn server_info(&self) -> &Server {
        &self.server
    }
    /// Performs the `client_settings` operation.
    pub async fn client_settings(&self) -> ClientSettings {
        self.client_settings.read().await.clone()
    }
    /// Performs the `server_brand` operation.
    pub async fn server_brand(&self) -> Option<String> {
        self.server_brand.read().await.clone()
    }
    /// Performs the `map` operation.
    pub async fn map(&self, id: i32) -> Option<MapData> {
        self.maps.read().await.maps.get(&id).cloned()
    }
    /// Performs the `maps_snapshot` operation.
    pub async fn maps_snapshot(&self) -> Snapshot<MapStore> {
        self.maps.read().await.snapshot()
    }
    /// Performs the `ui_state` operation.
    pub async fn ui_state(&self) -> UiState {
        self.ui_snapshot().await.value
    }
    /// Performs the `ui_snapshot` operation.
    pub async fn ui_snapshot(&self) -> Snapshot<UiState> {
        self.ui.read().await.snapshot()
    }
    /// Performs the `resource_pack_request` operation.
    pub async fn resource_pack_request(&self) -> Option<ResourcePackRequest> {
        self.resource_pack.read().await.clone()
    }
    /// Performs the `recipe_book_snapshot` operation.
    pub async fn recipe_book_snapshot(&self) -> Snapshot<RecipeBookState> {
        self.recipe_book.read().await.snapshot()
    }
    /// Performs the `statistics_snapshot` operation.
    pub async fn statistics_snapshot(&self) -> Snapshot<StatisticsState> {
        self.statistics.read().await.snapshot()
    }
    /// Performs the `advancements_snapshot` operation.
    pub async fn advancements_snapshot(&self) -> Snapshot<AdvancementState> {
        self.advancements.read().await.snapshot()
    }
    /// Performs the `world_view_snapshot` operation.
    pub async fn world_view_snapshot(&self) -> Snapshot<WorldViewState> {
        self.world_view.read().await.snapshot()
    }
    /// Performs the `camera_entity_id` operation.
    pub async fn camera_entity_id(&self) -> Option<i32> {
        **self.camera_entity_id.read().await
    }
    /// Performs the `command_tree_snapshot` operation.
    pub async fn command_tree_snapshot(&self) -> Snapshot<Option<CommandTree>> {
        self.command_tree.read().await.snapshot()
    }
    /// Performs the `tags_snapshot` operation.
    pub async fn tags_snapshot(&self) -> Snapshot<ServerTags> {
        self.tags.read().await.snapshot()
    }
    /// Performs the `server_recipes_snapshot` operation.
    pub async fn server_recipes_snapshot(&self) -> Snapshot<ServerRecipes> {
        self.server_recipes.read().await.snapshot()
    }
    /// Performs the `query_block_nbt` operation.
    pub async fn query_block_nbt(
        &self,
        position: BlockPos,
        wait: Duration,
    ) -> Result<NbtQueryResponse> {
        let transaction_id = self.next_nbt_query.fetch_add(1, Ordering::Relaxed);
        let mut events = self.subscribe();
        let mut payload = Vec::new();
        put_varint(&mut payload, transaction_id);
        payload.write_u64::<BigEndian>(position.packed())?;
        self.send(0x01, &payload).await?;
        wait_for_nbt(&mut events, transaction_id, wait).await
    }
    /// Performs the `query_entity_nbt` operation.
    pub async fn query_entity_nbt(
        &self,
        entity_id: i32,
        wait: Duration,
    ) -> Result<NbtQueryResponse> {
        let transaction_id = self.next_nbt_query.fetch_add(1, Ordering::Relaxed);
        let mut events = self.subscribe();
        let mut payload = Vec::new();
        put_varint(&mut payload, transaction_id);
        put_varint(&mut payload, entity_id);
        self.send(0x0d, &payload).await?;
        wait_for_nbt(&mut events, transaction_id, wait).await
    }
    /// Performs the `respond_resource_pack` operation.
    pub async fn respond_resource_pack(&self, status: ResourcePackStatus) -> Result<()> {
        let mut payload = Vec::new();
        put_varint(&mut payload, status as i32);
        self.send(0x20, &payload).await
    }

    /// Replaces and dispatches the standard Client Settings and brand payloads.
    pub async fn set_client_settings(&self, settings: ClientSettings) -> Result<()> {
        self.dispatch_client_settings(settings, false).await
    }

    async fn set_client_settings_protocol(&self, settings: ClientSettings) -> Result<()> {
        self.dispatch_client_settings(settings, true).await
    }

    async fn dispatch_client_settings(
        &self,
        settings: ClientSettings,
        protocol_response: bool,
    ) -> Result<()> {
        if settings.locale.is_empty() || settings.locale.len() > 16 {
            bail!("locale must contain 1..=16 bytes");
        }
        if !(2..=32).contains(&settings.view_distance) {
            bail!("view distance must be 2..=32 chunks");
        }
        let mut payload = Vec::new();
        put_string(&mut payload, &settings.locale);
        payload.write_i8(settings.view_distance)?;
        put_varint(&mut payload, settings.chat_mode as i32);
        payload.push(u8::from(settings.chat_colors));
        payload.push(settings.skin_parts);
        put_varint(&mut payload, settings.main_hand as i32);
        if protocol_response {
            self.send_protocol(0x05, &payload).await?;
        } else {
            self.send(0x05, &payload).await?;
        }
        let mut brand = Vec::new();
        put_string(&mut brand, "minecraft:brand");
        put_string(&mut brand, &settings.brand);
        if protocol_response {
            self.send_protocol(0x0b, &brand).await?;
        } else {
            self.send(0x0b, &brand).await?;
        }
        *self.client_settings.write().await = settings;
        Ok(())
    }
    /// Performs the `username` operation.
    pub async fn username(&self) -> String {
        self.player.lock().await.username.clone()
    }
    /// Performs the `player` operation.
    pub async fn player(&self) -> Player {
        self.player_snapshot().await.value
    }
    /// Performs the `player_snapshot` operation.
    pub async fn player_snapshot(&self) -> Snapshot<Player> {
        self.player.lock().await.snapshot()
    }
    /// Performs the `physics_metrics` operation.
    pub async fn physics_metrics(&self) -> PhysicsMetrics {
        self.physics.lock().await.snapshot()
    }

    /// Starts a typed diagnostic measurement window for this connection.
    ///
    /// Only the client-owned physics instrumentation accumulators are reset;
    /// player state, packet caches, movement proposals, and correction
    /// classification remain untouched. The returned token lets an integrating
    /// service bind its private metrics to the exact client generation.
    pub async fn begin_measurement_epoch(&self) -> crate::physics::PhysicsMeasurementEpoch {
        let generation = self.connection_generation();
        self.physics.lock().await.begin_measurement_epoch();
        crate::physics::PhysicsMeasurementEpoch::new(generation)
    }
    /// Performs the `motion` operation.
    pub async fn motion(&self) -> MotionState {
        self.motion_snapshot().await.value
    }
    /// Performs the `motion_snapshot` operation.
    pub async fn motion_snapshot(&self) -> Snapshot<MotionState> {
        self.motion.lock().await.snapshot()
    }
    /// Returns raw contact/environment facts used by client physics.
    pub async fn environment_state(&self) -> crate::versions::java_1_16_1::EnvironmentState {
        let player = self.player().await;
        self.world
            .lock()
            .await
            .environment_state(Aabb::player(player.x, player.y, player.z))
    }
    /// Performs the `survival_state` operation.
    pub async fn survival_state(&self) -> SurvivalState {
        self.survival_snapshot().await.value
    }
    /// Performs the `survival_snapshot` operation.
    pub async fn survival_snapshot(&self) -> Snapshot<SurvivalState> {
        self.survival.read().await.snapshot()
    }
    /// Performs the `inventory` operation.
    pub async fn inventory(&self) -> InventoryState {
        self.inventory_snapshot().await.value
    }
    /// Performs the `inventory_snapshot` operation.
    pub async fn inventory_snapshot(&self) -> Snapshot<InventoryState> {
        self.inventory.read().await.snapshot()
    }
    /// Performs the `open_window_state` operation.
    pub async fn open_window_state(&self) -> Option<OpenWindow> {
        self.open_window_snapshot().await.value
    }
    /// Performs the `open_window_snapshot` operation.
    pub async fn open_window_snapshot(&self) -> Snapshot<Option<OpenWindow>> {
        self.inventory
            .read()
            .await
            .map_snapshot(|inventory| inventory.open_window.clone())
    }
    /// Performs the `merchant_offers` operation.
    pub async fn merchant_offers(&self) -> Option<crate::versions::java_1_16_1::MerchantOffers> {
        self.inventory.read().await.merchant_offers.clone()
    }
    /// Performs the `entity` operation.
    pub async fn entity(&self, entity_id: i32) -> Option<EntityState> {
        self.entity_snapshot(entity_id).await.value
    }
    /// Performs the `entity_snapshot` operation.
    pub async fn entity_snapshot(&self, entity_id: i32) -> Snapshot<Option<EntityState>> {
        self.entities
            .read()
            .await
            .map_snapshot(|entities| entities.entities.get(&entity_id).cloned())
    }
    /// Returns every currently tracked entity.
    pub async fn entities(&self) -> Vec<EntityState> {
        self.entities_snapshot().await.value
    }
    /// Performs the `entities_snapshot` operation.
    pub async fn entities_snapshot(&self) -> Snapshot<Vec<EntityState>> {
        self.entities
            .read()
            .await
            .map_snapshot(|entities| entities.entities.values().cloned().collect())
    }
    /// Returns the tracked entity carrying this player, when passenger state is known.
    pub async fn vehicle(&self) -> Option<EntityState> {
        let player_id = self.player().await.entity_id?;
        self.entities
            .read()
            .await
            .entities
            .values()
            .find(|entity| entity.passengers.contains(&player_id))
            .cloned()
    }
    /// Performs the `vehicle_pose` operation.
    pub async fn vehicle_pose(&self) -> Option<VehiclePose> {
        self.vehicle().await.map(|vehicle| VehiclePose {
            position: vehicle.position,
            yaw: vehicle.yaw,
            pitch: vehicle.pitch,
        })
    }
    /// Performs the `observe_entities` operation.
    pub async fn observe_entities(&self, radius: f64) -> Vec<EntityState> {
        self.observe_entities_snapshot(radius).await.value
    }
    /// Performs the `observe_entities_snapshot` operation.
    pub async fn observe_entities_snapshot(&self, radius: f64) -> Snapshot<Vec<EntityState>> {
        let player = self.player().await;
        self.entities.read().await.map_snapshot(|entities| {
            entities.observe(
                Vec3 {
                    x: player.x,
                    y: player.y,
                    z: player.z,
                },
                radius,
            )
        })
    }
    /// Performs the `player_list` operation.
    pub async fn player_list(&self) -> PlayerList {
        self.player_list_snapshot().await.value
    }
    /// Performs the `player_list_snapshot` operation.
    pub async fn player_list_snapshot(&self) -> Snapshot<PlayerList> {
        self.players.read().await.snapshot()
    }
    /// Performs the `control` operation.
    pub async fn control(&self) -> ControlState {
        self.control_snapshot().await.value
    }
    /// Performs the `control_snapshot` operation.
    pub async fn control_snapshot(&self) -> Snapshot<ControlState> {
        self.connection.control_snapshot().await
    }
    /// Replaces the persistent input state consumed by the 20 Hz physics loop.
    ///
    /// Returning means the local state was updated; it does not wait for a
    /// movement packet or server acknowledgement. The state remains active
    /// until replaced or cleared.
    pub async fn set_control(&self, control: ControlState) {
        let _ = self
            .connection
            .replace_control(self.operation_context(0), OperationClass::Normal, control)
            .await;
    }

    /// Replaces the actor-owned control state for a Caller-selected observation.
    ///
    /// The context is mandatory on the typed path so stale observations and
    /// post-barrier normal control cannot mutate the state or produce packets.
    /// The replacement is admitted and committed by one connection-actor
    /// command; this method does not choose movement policy or route.
    ///
    /// # Errors
    ///
    /// Returns a typed admission error before the replacement is committed.
    pub async fn set_control_with_context(
        &self,
        context: OperationContext,
        control: ControlState,
    ) -> std::result::Result<(), DispatchError> {
        self.connection
            .replace_control(context, OperationClass::Normal, control)
            .await
            .map_err(DispatchError::Admission)
    }
    /// Clears every persistent control input locally.
    pub async fn clear_control(&self) {
        let _ = self
            .connection
            .replace_control(
                self.operation_context(0),
                OperationClass::Cleanup,
                ControlState::default(),
            )
            .await;
    }
    /// Access protocol-adjacent operations with weaker compatibility guarantees.
    pub fn unstable(&self) -> crate::versions::java_1_16_1::UnstableBot<'_> {
        crate::versions::java_1_16_1::UnstableBot { bot: self }
    }
    /// Temporarily stops the 20 Hz movement producer without changing `ControlState`.
    ///
    /// This is useful when an external controller knows that a server teleport is imminent.
    /// The previous control state is applied again automatically after the suspension.
    pub fn suspend_movement_for(&self, duration: Duration) {
        let ticks = duration.as_millis().div_ceil(50).min(u128::from(u8::MAX)) as u8;
        self.teleport_barrier_ticks
            .fetch_max(ticks, Ordering::AcqRel);
    }
    /// Waits until login, spawn, and the initial authoritative position complete.
    pub async fn wait_until_ready(&self) -> Result<()> {
        loop {
            if self.stopped.load(Ordering::Acquire) {
                bail!("connection closed before initial position");
            }
            let notified = self.ready.notified();
            let spawned = self.player.lock().await.spawned;
            let positioned = *self.positioned.lock().await;
            if spawned && positioned {
                self.connection.mark_ready().await;
                return Ok(());
            }
            timeout(self.connection_options.ready_timeout, notified)
                .await
                .context("timed out waiting for initial position")?;
        }
    }
    pub(crate) async fn move_relative_unchecked(&self, forward: f64, strafe: f64) -> Result<()> {
        self.wait_until_ready().await?;
        let _coherent_state = self.coherent_state_gate.lock().await;
        let mut p = self.player.lock().await;
        let yaw = (p.yaw as f64).to_radians();
        p.x += -yaw.sin() * forward + yaw.cos() * strafe;
        p.z += yaw.cos() * forward + yaw.sin() * strafe;
        {
            let mut receipts = self.common_receipts.lock().await;
            receipts.position_source = Some(crate::client::ValueSource::Submitted);
            receipts.ground_source = None;
        }
        drop(p);
        self.send_position().await
    }
    /// Updates yaw/pitch locally and dispatches one movement packet.
    ///
    /// Success guarantees packet dispatch, not server acceptance of the pose.
    pub async fn look(&self, yaw: f32, pitch: f32) -> Result<()> {
        if !yaw.is_finite() || !pitch.is_finite() {
            bail!("look angles must be finite");
        }
        self.wait_until_ready().await?;
        let _coherent_state = self.coherent_state_gate.lock().await;
        let mut p = self.player.lock().await;
        p.yaw = yaw;
        p.pitch = pitch.clamp(-90.0, 90.0);
        self.common_receipts.lock().await.rotation_source =
            Some(crate::client::ValueSource::Submitted);
        drop(p);
        self.send_position().await
    }
    /// Returns an owned cube of block observations around the current position.
    ///
    /// This waits for the player's current chunk, clamps `radius` to `0..=16`,
    /// and allocates a new `Vec`. Missing entries represent unavailable chunk
    /// data; air is represented by its normal block-state ID.
    pub async fn observe(&self, radius: i32) -> Result<Vec<BlockObservation>> {
        Ok(self.observe_snapshot(radius).await?.value)
    }
    /// Performs the `observe_snapshot` operation.
    pub async fn observe_snapshot(&self, radius: i32) -> Result<Snapshot<Vec<BlockObservation>>> {
        self.wait_until_ready().await?;
        let p = self.player().await;
        self.wait_for_player_chunk(&p).await?;
        Ok(self.world.lock().await.map_snapshot(|world| {
            world.observe_cube(
                p.x.floor() as i32,
                p.y.floor() as i32,
                p.z.floor() as i32,
                radius.clamp(0, 16),
            )
        }))
    }

    pub(crate) async fn observe_region_snapshot(
        &self,
        region: crate::Region,
    ) -> Result<Snapshot<Vec<BlockObservation>>> {
        let volume = region.volume()?;
        if self.is_stopped() {
            return Err(crate::Error::new(
                crate::ErrorKind::Disconnected,
                anyhow::anyhow!("connection closed before observation"),
            ));
        }
        Ok(self.world.lock().await.map_snapshot(|world| {
            let mut blocks = Vec::with_capacity(volume);
            for x in region.min[0]..=region.max[0] {
                for y in region.min[1]..=region.max[1] {
                    for z in region.min[2]..=region.max[2] {
                        blocks.push(BlockObservation {
                            x,
                            y,
                            z,
                            state_id: world.block(x, y, z),
                        });
                    }
                }
            }
            blocks
        }))
    }
    /// Returns a loaded block-state ID, or `None` when the chunk is unavailable.
    ///
    /// `None` never means air.
    pub async fn block(&self, x: i32, y: i32, z: i32) -> Option<i32> {
        self.block_snapshot(x, y, z).await.value
    }
    /// Performs the `block_snapshot` operation.
    pub async fn block_snapshot(&self, x: i32, y: i32, z: i32) -> Snapshot<Option<i32>> {
        self.world
            .lock()
            .await
            .map_snapshot(|world| world.block(x, y, z))
    }
    /// Performs the `loaded_chunks` operation.
    pub async fn loaded_chunks(&self) -> Vec<crate::versions::java_1_16_1::ChunkPos> {
        self.world.lock().await.loaded_chunks()
    }
    /// Performs the `query_blocks` operation.
    pub async fn query_blocks(
        &self,
        region: crate::versions::java_1_16_1::BlockRegion,
        state_ids: &[i32],
        limit: usize,
    ) -> Result<Vec<BlockPos>> {
        Ok(self
            .world
            .lock()
            .await
            .query_blocks(region, state_ids, limit)?)
    }

    /// Performs the `is_chunk_loaded` operation.
    pub async fn is_chunk_loaded(&self, position: crate::versions::java_1_16_1::ChunkPos) -> bool {
        self.world.lock().await.chunk_snapshot(position).is_some()
    }

    /// Returns a low-copy snapshot whose section and NBT buffers are Arc-backed.
    pub async fn chunk_snapshot(
        &self,
        position: crate::versions::java_1_16_1::ChunkPos,
    ) -> Option<crate::versions::java_1_16_1::ChunkSnapshot> {
        self.world.lock().await.chunk_snapshot(position)
    }

    /// Performs the `wait_for_chunk` operation.
    pub async fn wait_for_chunk(
        &self,
        position: crate::versions::java_1_16_1::ChunkPos,
        wait: Duration,
    ) -> Result<crate::versions::java_1_16_1::ChunkSnapshot> {
        Ok(timeout(wait, async {
            loop {
                let notified = self.world_updated.notified();
                if let Some(chunk) = self.chunk_snapshot(position).await {
                    return Ok::<_, anyhow::Error>(chunk);
                }
                if self.stopped.load(Ordering::Acquire) {
                    bail!("connection closed while waiting for chunk");
                }
                notified.await;
            }
        })
        .await
        .context("timed out waiting for chunk")??)
    }
    /// Performs the `wait_for_chunks` operation.
    pub async fn wait_for_chunks(
        &self,
        center: crate::versions::java_1_16_1::ChunkPos,
        radius: i32,
        wait: Duration,
    ) -> Result<Vec<crate::versions::java_1_16_1::ChunkSnapshot>> {
        if !(0..=32).contains(&radius) {
            bail!("chunk wait radius must be 0..=32");
        }
        let min_x = center
            .x
            .checked_sub(radius)
            .context("chunk wait x range underflow")?;
        let max_x = center
            .x
            .checked_add(radius)
            .context("chunk wait x range overflow")?;
        let min_z = center
            .z
            .checked_sub(radius)
            .context("chunk wait z range underflow")?;
        let max_z = center
            .z
            .checked_add(radius)
            .context("chunk wait z range overflow")?;
        Ok(timeout(wait, async {
            loop {
                let notified = self.world_updated.notified();
                let world = self.world.lock().await;
                let mut chunks = Vec::with_capacity(((radius * 2 + 1).pow(2)) as usize);
                let mut complete = true;
                for z in min_z..=max_z {
                    for x in min_x..=max_x {
                        if let Some(chunk) =
                            world.chunk_snapshot(crate::versions::java_1_16_1::ChunkPos { x, z })
                        {
                            chunks.push(chunk)
                        } else {
                            complete = false;
                        }
                    }
                }
                drop(world);
                if complete {
                    return Ok::<_, anyhow::Error>(chunks);
                }
                if self.stopped.load(Ordering::Acquire) {
                    bail!("connection closed while waiting for chunks")
                }
                notified.await
            }
        })
        .await
        .context("timed out waiting for chunks")??)
    }
    /// Waits until the world domain advances beyond `after_revision`, then returns this block.
    pub async fn wait_for_block_revision(
        &self,
        position: BlockPos,
        after_revision: u64,
        wait: Duration,
    ) -> Result<Snapshot<Option<i32>>> {
        Ok(timeout(wait, async {
            loop {
                let notified = self.world_updated.notified();
                let snapshot = self
                    .block_snapshot(position.x, position.y, position.z)
                    .await;
                if snapshot.revision > after_revision {
                    return Ok::<_, anyhow::Error>(snapshot);
                }
                if self.stopped.load(Ordering::Acquire) {
                    bail!("connection closed while waiting for block revision");
                }
                notified.await;
            }
        })
        .await
        .context("timed out waiting for block revision")??)
    }

    /// Waits for the exact block state. `None` means the chunk becoming unavailable.
    pub async fn wait_for_block_state(
        &self,
        position: BlockPos,
        expected: Option<i32>,
        wait: Duration,
    ) -> Result<Snapshot<Option<i32>>> {
        Ok(timeout(wait, async {
            loop {
                let notified = self.world_updated.notified();
                let snapshot = self
                    .block_snapshot(position.x, position.y, position.z)
                    .await;
                if snapshot.value == expected {
                    return Ok::<_, anyhow::Error>(snapshot);
                }
                if self.stopped.load(Ordering::Acquire) {
                    bail!("connection closed while waiting for block state");
                }
                notified.await;
            }
        })
        .await
        .context("timed out waiting for block state")??)
    }
    /// Performs the `wait_for_block_change` operation.
    pub async fn wait_for_block_change(
        &self,
        position: BlockPos,
        previous: Option<i32>,
        wait: Duration,
    ) -> Result<Snapshot<Option<i32>>> {
        Ok(timeout(wait, async {
            loop {
                let notified = self.world_updated.notified();
                let snapshot = self
                    .block_snapshot(position.x, position.y, position.z)
                    .await;
                if snapshot.value != previous {
                    return Ok::<_, anyhow::Error>(snapshot);
                }
                if self.stopped.load(Ordering::Acquire) {
                    bail!("connection closed while waiting for block change")
                }
                notified.await
            }
        })
        .await
        .context("timed out waiting for block change")??)
    }

    /// Waits until any inventory/window state advances beyond `after_revision`.
    pub async fn wait_for_inventory_revision(
        &self,
        after_revision: u64,
        wait: Duration,
    ) -> Result<Snapshot<InventoryState>> {
        let mut events = self.subscribe();
        Ok(timeout(wait, async {
            loop {
                let snapshot = self.inventory_snapshot().await;
                if snapshot.revision > after_revision {
                    return Ok::<_, anyhow::Error>(snapshot);
                }
                match events.recv().await {
                    Ok(Event::Disconnected { reason }) => bail!("disconnected: {reason}"),
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => bail!("event stream closed"),
                }
            }
        })
        .await
        .context("timed out waiting for inventory revision")??)
    }

    /// Waits until tracked entities advance beyond `after_revision`.
    pub async fn wait_for_entities_revision(
        &self,
        after_revision: u64,
        wait: Duration,
    ) -> Result<Snapshot<Vec<EntityState>>> {
        let mut events = self.subscribe();
        Ok(timeout(wait, async {
            loop {
                let snapshot = self.entities_snapshot().await;
                if snapshot.revision > after_revision {
                    return Ok::<_, anyhow::Error>(snapshot);
                }
                match events.recv().await {
                    Ok(Event::Disconnected { reason }) => bail!("disconnected: {reason}"),
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => bail!("event stream closed"),
                }
            }
        })
        .await
        .context("timed out waiting for entity revision")??)
    }
    /// Casts from the player's eyes using the supplied direction and real block shapes.
    pub async fn raycast_blocks(
        &self,
        direction: Vec3,
        max_distance: f64,
    ) -> Option<crate::versions::java_1_16_1::BlockRaycastHit> {
        let player = self.player().await;
        self.world.lock().await.raycast_blocks(
            Vec3 {
                x: player.x,
                y: player.y + 1.62,
                z: player.z,
            },
            direction,
            max_distance,
        )
    }

    /// Returns the block intersected by the current yaw and pitch.
    pub async fn targeted_block(
        &self,
        max_distance: f64,
    ) -> Option<crate::versions::java_1_16_1::BlockRaycastHit> {
        let player = self.player().await;
        let yaw = (player.yaw as f64).to_radians();
        let pitch = (player.pitch as f64).to_radians();
        self.raycast_blocks(
            Vec3 {
                x: -yaw.sin() * pitch.cos(),
                y: -pitch.sin(),
                z: yaw.cos() * pitch.cos(),
            },
            max_distance,
        )
        .await
    }

    /// Casts from the player's eyes and returns the closest entity bounding box hit.
    pub async fn raycast_entities(
        &self,
        direction: Vec3,
        max_distance: f64,
    ) -> Option<crate::versions::java_1_16_1::EntityRaycastHit> {
        let player = self.player().await;
        self.entities.read().await.raycast(
            Vec3 {
                x: player.x,
                y: player.y + 1.62,
                z: player.z,
            },
            direction,
            max_distance,
            player.entity_id,
        )
    }

    /// Returns the visible entity under the current crosshair, accounting for block occlusion.
    pub async fn targeted_entity(
        &self,
        max_distance: f64,
    ) -> Option<crate::versions::java_1_16_1::EntityRaycastHit> {
        let player = self.player().await;
        let yaw = (player.yaw as f64).to_radians();
        let pitch = (player.pitch as f64).to_radians();
        let direction = Vec3 {
            x: -yaw.sin() * pitch.cos(),
            y: -pitch.sin(),
            z: yaw.cos() * pitch.cos(),
        };
        let entity = self.raycast_entities(direction, max_distance).await?;
        let block = self.raycast_blocks(direction, max_distance).await;
        if block.is_some_and(|hit| hit.distance < entity.distance) {
            None
        } else {
            Some(entity)
        }
    }

    /// Performs the `can_reach_block` operation.
    pub async fn can_reach_block(&self, position: BlockPos) -> bool {
        self.digging_info(position).await.reachable
    }

    /// Performs the `can_see_block` operation.
    pub async fn can_see_block(&self, position: BlockPos) -> bool {
        self.digging_info(position).await.visible
    }

    /// Performs the `can_reach_entity` operation.
    pub async fn can_reach_entity(&self, entity_id: i32) -> bool {
        let Some(entity) = self.entity(entity_id).await else {
            return false;
        };
        let player = self.player().await;
        let dx = entity.position.x - player.x;
        let dy = entity.position.y - (player.y + 1.62);
        let dz = entity.position.z - player.z;
        (dx * dx + dy * dy + dz * dz).sqrt() <= 3.0
    }

    /// Performs the `can_see_entity` operation.
    pub async fn can_see_entity(&self, entity_id: i32) -> bool {
        let Some(entity) = self.entity(entity_id).await else {
            return false;
        };
        let player = self.player().await;
        let direction = Vec3 {
            x: entity.position.x - player.x,
            y: entity.position.y + 0.5 - (player.y + 1.62),
            z: entity.position.z - player.z,
        };
        let distance =
            (direction.x * direction.x + direction.y * direction.y + direction.z * direction.z)
                .sqrt();
        let Some(hit) = self.raycast_entities(direction, distance + 1.0).await else {
            return false;
        };
        if hit.entity.entity_id != entity_id {
            return false;
        }
        !self
            .raycast_blocks(direction, hit.distance)
            .await
            .is_some_and(|block| block.distance < hit.distance)
    }

    /// Performs the `digging_info` operation.
    pub async fn digging_info(
        &self,
        position: BlockPos,
    ) -> crate::versions::java_1_16_1::DiggingInfo {
        let player = self.player().await;
        let eye = Vec3 {
            x: player.x,
            y: player.y + 1.62,
            z: player.z,
        };
        let center = Vec3 {
            x: f64::from(position.x) + 0.5,
            y: f64::from(position.y) + 0.5,
            z: f64::from(position.z) + 0.5,
        };
        let direction = Vec3 {
            x: center.x - eye.x,
            y: center.y - eye.y,
            z: center.z - eye.z,
        };
        let distance =
            (direction.x * direction.x + direction.y * direction.y + direction.z * direction.z)
                .sqrt();
        let state_id = self.block(position.x, position.y, position.z).await;
        let tool_id = self
            .inventory
            .read()
            .await
            .selected_item()
            .map(|item| item.item_id);
        let mining = state_id.and_then(|id| crate::registry::mining_info(id, tool_id));
        let visible = self.world.lock().await.block_visible(eye, position);
        crate::DiggingInfo {
            state_id,
            loaded: state_id.is_some(),
            reachable: state_id.is_some() && distance <= 4.5,
            visible,
            diggable: mining.is_some_and(|info| info.diggable),
            harvestable: mining.is_some_and(|info| info.harvestable),
            effective_tool: mining.is_some_and(|info| info.effective_tool),
            predicted_ticks: mining.and_then(|info| info.predicted_ticks),
        }
    }

    /// Performs the `placement_info` operation.
    pub async fn placement_info(
        &self,
        position: BlockPos,
    ) -> crate::versions::java_1_16_1::PlacementInfo {
        let player = self.player().await;
        let eye = Vec3 {
            x: player.x,
            y: player.y + 1.62,
            z: player.z,
        };
        let center = Vec3 {
            x: f64::from(position.x) + 0.5,
            y: f64::from(position.y) + 0.5,
            z: f64::from(position.z) + 0.5,
        };
        let direction = Vec3 {
            x: center.x - eye.x,
            y: center.y - eye.y,
            z: center.z - eye.z,
        };
        let distance =
            (direction.x * direction.x + direction.y * direction.y + direction.z * direction.z)
                .sqrt();
        let world = self.world.lock().await;
        let target_loaded = world.block(position.x, position.y, position.z).is_some();
        let visible = world
            .raycast_blocks(eye, direction, distance + 1.0e-7)
            .is_some_and(|hit| hit.position == position);
        let player_box = Aabb::player(player.x, player.y, player.z);
        let target = Aabb::block(position.x, position.y, position.z);
        let player_intersects_target_cell = target.max_x > player_box.min_x
            && target.min_x < player_box.max_x
            && target.max_y > player_box.min_y
            && target.min_y < player_box.max_y
            && target.max_z > player_box.min_z
            && target.min_z < player_box.max_z;
        crate::versions::java_1_16_1::PlacementInfo {
            target_loaded,
            reachable: target_loaded && distance <= 4.5,
            visible,
            player_intersects_target_cell,
        }
    }
    /// Queues one edge-triggered ground jump for the next physics tick.
    ///
    /// Success means the request was queued locally. It may have no effect when
    /// the player cannot currently jump. For held swimming/climbing input, use
    /// `ControlState::jump` through [`Bot::set_control`].
    pub async fn jump(&self) -> Result<()> {
        self.wait_until_ready().await?;
        let _gate = self.coherent_state_gate.lock().await;
        if self.stopped.load(Ordering::Acquire) {
            bail!("connection closed before ground jump admission");
        }
        if self.common_native_physics_paused().await {
            bail!("another operation owns movement; use its ground-jump API");
        }
        self.jump_requested.store(true, Ordering::Release);
        Ok(())
    }
    /// Sends mounted movement input for steerable entities such as horses and pigs.
    pub async fn set_vehicle_control(&self, control: VehicleControl) -> Result<()> {
        if !control.sideways.is_finite()
            || !control.forward.is_finite()
            || !(-1.0..=1.0).contains(&control.sideways)
            || !(-1.0..=1.0).contains(&control.forward)
        {
            bail!("vehicle input must be finite and within -1.0..=1.0");
        }
        let mut payload = Vec::new();
        payload.write_f32::<BigEndian>(control.sideways)?;
        payload.write_f32::<BigEndian>(control.forward)?;
        payload.push(u8::from(control.jump) | (u8::from(control.dismount) << 1));
        self.send(0x1d, &payload).await
    }

    /// Sends the two paddle inputs used while controlling a boat.
    pub async fn steer_boat(&self, left_paddle: bool, right_paddle: bool) -> Result<()> {
        self.send(0x17, &[u8::from(left_paddle), u8::from(right_paddle)])
            .await
    }

    /// Requests dismount through the mounted-input packet.
    pub async fn dismount(&self) -> Result<()> {
        self.set_vehicle_control(VehicleControl {
            dismount: true,
            ..VehicleControl::default()
        })
        .await
    }
    pub(crate) async fn send_vehicle_pose_unchecked(&self, pose: VehiclePose) -> Result<()> {
        if !pose.position.x.is_finite()
            || !pose.position.y.is_finite()
            || !pose.position.z.is_finite()
            || !pose.yaw.is_finite()
            || !pose.pitch.is_finite()
        {
            bail!("vehicle pose values must be finite");
        }
        let mut payload = Vec::new();
        payload.write_f64::<BigEndian>(pose.position.x)?;
        payload.write_f64::<BigEndian>(pose.position.y)?;
        payload.write_f64::<BigEndian>(pose.position.z)?;
        payload.write_f32::<BigEndian>(pose.yaw)?;
        payload.write_f32::<BigEndian>(pose.pitch)?;
        self.send(0x16, &payload).await
    }
    /// Dispatches a respawn request without waiting for the new spawn state.
    pub async fn respawn(&self) -> Result<()> {
        let mut payload = Vec::new();
        put_varint(&mut payload, 0);
        self.send(0x04, &payload).await
    }
    /// Selects a hotbar slot and updates the predicted local selection.
    ///
    /// Success guarantees packet dispatch; the protocol has no direct
    /// acknowledgement for this action.
    pub async fn select_hotbar(&self, slot: u8) -> Result<()> {
        if slot > 8 {
            bail!("hotbar slot must be 0..=8");
        }
        let _coherent_state = self.coherent_state_gate.lock().await;
        let mut payload = Vec::new();
        payload.write_i16::<BigEndian>(i16::from(slot))?;
        self.send(0x24, &payload).await?;
        self.inventory.write().await.selected_hotbar = slot;
        Ok(())
    }
    /// Dispatches an item-drop action without waiting for an inventory update.
    pub async fn drop_selected(&self, entire_stack: bool) -> Result<()> {
        let mut payload = Vec::new();
        put_varint(&mut payload, if entire_stack { 3 } else { 4 });
        payload.write_u64::<BigEndian>(0)?;
        payload.write_i8(0)?;
        self.send(0x1b, &payload).await
    }
    /// Drops exactly `count` items from the selected hotbar stack.
    pub async fn drop_selected_count(&self, count: u32) -> Result<()> {
        if count == 0 {
            bail!("drop count must be positive");
        }
        let available = self
            .inventory
            .read()
            .await
            .selected_item()
            .map_or(0, |item| u32::from(item.count.max(0) as u8));
        if count > available {
            bail!("drop count {count} exceeds selected stack size {available}");
        }
        if count == available {
            return self.drop_selected(true).await;
        }
        for _ in 0..count {
            self.drop_selected(false).await?;
        }
        Ok(())
    }

    /// Moves/swaps one explicitly chosen player-inventory slot using acknowledged clicks.
    pub async fn move_player_item(&self, source: i16, destination: i16) -> Result<()> {
        if !(0..=45).contains(&source) || !(0..=45).contains(&destination) || source == destination
        {
            bail!("player slots must be distinct values in 0..=45");
        }
        let inventory = self.inventory.read().await;
        if inventory.cursor.is_some() {
            bail!("inventory cursor must be empty before moving an item");
        }
        if inventory
            .player_slots()
            .get(source as usize)
            .and_then(Option::as_ref)
            .is_none()
        {
            bail!("source player slot is empty");
        }
        drop(inventory);
        self.click_slot_and_wait(0, source, 0, ClickMode::Normal)
            .await?;
        self.click_slot_and_wait(0, destination, 0, ClickMode::Normal)
            .await?;
        if self.inventory.read().await.cursor.is_some() {
            self.click_slot_and_wait(0, source, 0, ClickMode::Normal)
                .await?;
        }
        Ok(())
    }

    /// Performs the `equip_from_player_slot` operation.
    pub async fn equip_from_player_slot(
        &self,
        source: i16,
        destination: EquipmentSlot,
    ) -> Result<()> {
        self.move_player_item(source, destination as i16).await
    }

    /// Sends an edited written/writable book stack. NBT construction remains caller-controlled.
    pub async fn edit_book(&self, book: &ItemStack, signing: bool, hand: Hand) -> Result<()> {
        let mut payload = Vec::new();
        write_slot(&mut payload, Some(book));
        payload.push(u8::from(signing));
        put_varint(&mut payload, hand as i32);
        self.send(0x0c, &payload).await
    }
    pub(crate) async fn send_digging_packet(
        &self,
        status: DiggingStatus,
        position: BlockPos,
        face: BlockFace,
    ) -> Result<()> {
        let mut payload = Vec::new();
        put_varint(&mut payload, status as i32);
        payload.write_u64::<BigEndian>(position.packed())?;
        payload.write_i8(face as i8)?;
        self.send(0x1b, &payload).await
    }
    /// Performs timed digging and waits for the matching server acknowledgement.
    pub async fn dig_block(
        &self,
        position: BlockPos,
        face: BlockFace,
    ) -> Result<DiggingAcknowledgement> {
        let state_id = self
            .block(position.x, position.y, position.z)
            .await
            .context("target block chunk is not loaded")?;
        let tool_id = self
            .inventory
            .read()
            .await
            .selected_item()
            .map(|item| item.item_id);
        let ticks = crate::versions::java_1_16_1::registry::mining_ticks(state_id, tool_id)
            .context("target block is not diggable")?;
        let mut events = self.subscribe();
        self.send_digging_packet(DiggingStatus::Started, position, face)
            .await?;
        tokio::time::sleep(Duration::from_millis(ticks.saturating_mul(50))).await;
        let transaction = self
            .dispatch_acknowledged(
                self.operation_context(0),
                AcknowledgedOperation::DigFinish { position, face },
            )
            .await
            .map_err(|error| {
                crate::Error::new(
                    if matches!(error, DispatchError::DeliveryUnknown) {
                        crate::ErrorKind::UncertainDispatch
                    } else {
                        crate::ErrorKind::State
                    },
                    anyhow::anyhow!(error),
                )
            })?;
        match transaction.wait().await {
            DispatchOutcome::Acknowledged => {}
            DispatchOutcome::Rejected => {
                bail!("server rejected digging action at {position:?}");
            }
            DispatchOutcome::DeliveryUnknown => {
                bail!("delivery of digging acknowledgement is unknown");
            }
            DispatchOutcome::Dispatched => {
                bail!("digging action was dispatched without acknowledgement");
            }
        }
        timeout(self.connection_options.protocol_ack_timeout, async {
            loop {
                if let Event::DiggingAcknowledged(ack) = next_operation_event(&mut events).await? {
                    if ack.position == position && ack.status == DiggingStatus::Finished as i32 {
                        return Ok(ack);
                    }
                }
            }
        })
        .await
        .context("timed out waiting for digging response event")?
    }
    /// Dispatches a block-placement interaction.
    ///
    /// Success does not guarantee that a block was placed. Observe the target
    /// block or consume `BlockChanged` to confirm the game result.
    pub async fn place_block(
        &self,
        hand: Hand,
        position: BlockPos,
        face: BlockFace,
        cursor: [f32; 3],
        inside_block: bool,
    ) -> Result<()> {
        if cursor.iter().any(|value| !(0.0..=1.0).contains(value)) {
            bail!("block cursor coordinates must be within 0.0..=1.0");
        }
        let mut payload = Vec::new();
        put_varint(&mut payload, hand as i32);
        payload.write_u64::<BigEndian>(position.packed())?;
        put_varint(&mut payload, face as i32);
        for value in cursor {
            payload.write_f32::<BigEndian>(value)?;
        }
        payload.push(u8::from(inside_block));
        self.send(0x2d, &payload).await
    }
    /// Dispatches placement and waits until the caller-selected observed cell changes.
    pub async fn place_block_and_wait_for_change(
        &self,
        request: crate::versions::java_1_16_1::PlacementRequest,
        observed: BlockPos,
        wait: Duration,
    ) -> Result<Snapshot<Option<i32>>> {
        let previous = self.block(observed.x, observed.y, observed.z).await;
        self.place_block(
            request.hand,
            request.clicked,
            request.face,
            request.cursor,
            request.inside_block,
        )
        .await?;
        self.wait_for_block_change(observed, previous, wait).await
    }
    /// Dispatches a use-item action without waiting for its game result.
    pub async fn use_item(&self, hand: Hand) -> Result<()> {
        let mut payload = Vec::new();
        put_varint(&mut payload, hand as i32);
        self.send(0x2e, &payload).await
    }
    /// Dispatches use-item, waits locally for `duration`, then dispatches release.
    ///
    /// Dropping this future after the first dispatch does not automatically send
    /// the release action.
    pub async fn use_item_for(&self, hand: Hand, duration: Duration) -> Result<()> {
        self.use_item(hand).await?;
        tokio::time::sleep(duration).await;
        self.release_item().await
    }
    /// Dispatches release-use-item without waiting for acknowledgement.
    pub async fn release_item(&self) -> Result<()> {
        self.send_digging_packet(
            DiggingStatus::ReleaseUseItem,
            BlockPos { x: 0, y: 0, z: 0 },
            BlockFace::Down,
        )
        .await
    }
    /// Dispatches an arm animation without waiting for acknowledgement.
    pub async fn swing_arm(&self, hand: Hand) -> Result<()> {
        let mut payload = Vec::new();
        put_varint(&mut payload, hand as i32);
        self.send(0x2b, &payload).await
    }
    /// Dispatches an entity interaction without confirming its game result.
    pub async fn interact_entity(&self, entity_id: i32, hand: Hand, sneaking: bool) -> Result<()> {
        let mut payload = Vec::new();
        put_varint(&mut payload, entity_id);
        put_varint(&mut payload, 0);
        put_varint(&mut payload, hand as i32);
        payload.push(u8::from(sneaking));
        self.send(0x0e, &payload).await
    }
    /// Dispatches a position-specific entity interaction without confirmation.
    pub async fn interact_entity_at(
        &self,
        entity_id: i32,
        hit: [f32; 3],
        hand: Hand,
        sneaking: bool,
    ) -> Result<()> {
        if !hit.iter().all(|coordinate| coordinate.is_finite()) {
            bail!("entity interaction coordinates must be finite");
        }
        let mut payload = Vec::new();
        put_varint(&mut payload, entity_id);
        put_varint(&mut payload, 2);
        for coordinate in hit {
            payload.write_f32::<BigEndian>(coordinate)?;
        }
        put_varint(&mut payload, hand as i32);
        payload.push(u8::from(sneaking));
        self.send(0x0e, &payload).await
    }
    /// Waits for the locally calculated attack cooldown, then dispatches attack
    /// and arm-swing packets. Success does not confirm damage.
    pub async fn attack(&self, entity_id: i32) -> Result<()> {
        let survival = self.survival.read().await;
        let attack_speed = survival
            .attributes
            .get("minecraft:generic.attack_speed")
            .or_else(|| survival.attributes.get("generic.attack_speed"))
            .map_or(4.0, |attribute| attribute.value())
            .max(0.1);
        drop(survival);
        let cooldown = Duration::from_secs_f64(1.0 / attack_speed);
        loop {
            let mut last_attack = self.last_attack.lock().await;
            let remaining = last_attack.map_or(Duration::ZERO, |last| {
                cooldown.saturating_sub(last.elapsed())
            });
            if !remaining.is_zero() {
                // Do not hold the dispatch lock while charging. An immediate
                // defensive hit can reset the shared clock during this wait.
                drop(last_attack);
                tokio::time::sleep(remaining).await;
                continue;
            }
            return self.dispatch_attack(entity_id, &mut last_attack).await;
        }
    }
    /// Dispatches an attack without waiting for locally calculated charge.
    ///
    /// Damage may be reduced or rejected by the server. Success confirms only
    /// packet dispatch, not a hit, knockback or a mob AI phase change. The
    /// shared attack clock is reset, so a subsequent [`Self::attack`] waits
    /// for recovery. Transport and dispatch serialization may still wait.
    pub async fn attack_immediate(&self, entity_id: i32) -> Result<()> {
        let mut last_attack = self.last_attack.lock().await;
        self.dispatch_attack(entity_id, &mut last_attack).await
    }
    async fn dispatch_attack(
        &self,
        entity_id: i32,
        last_attack: &mut Option<std::time::Instant>,
    ) -> Result<()> {
        let mut payload = Vec::new();
        put_varint(&mut payload, entity_id);
        put_varint(&mut payload, 1);
        payload.push(u8::from(
            self.connection.control_snapshot().await.value.sneak,
        ));
        // Keep the clock conservative even if attack/swing dispatch is ambiguous.
        *last_attack = Some(std::time::Instant::now());
        self.send(0x0e, &payload).await?;
        self.swing_arm(Hand::Main).await?;
        Ok(())
    }
    /// Dispatches a chat message after local length validation.
    pub async fn send_chat(&self, message: &str) -> Result<()> {
        if message.is_empty() || message.chars().count() > 256 {
            bail!("chat message must contain 1..=256 characters");
        }
        let mut payload = Vec::new();
        put_string(&mut payload, message);
        self.send(0x03, &payload).await
    }
    /// Requests protocol-level command/chat completion and waits for its matching response.
    pub async fn tab_complete(&self, text: &str, wait: Duration) -> Result<TabCompletion> {
        if text.is_empty() || text.chars().count() > 256 {
            bail!("completion text must contain 1..=256 characters");
        }
        let transaction_id = self.next_tab_completion.fetch_add(1, Ordering::Relaxed);
        let mut events = self.subscribe();
        let mut payload = Vec::new();
        put_varint(&mut payload, transaction_id);
        put_string(&mut payload, text);
        self.send(0x06, &payload).await?;
        Ok(timeout(wait, async {
            loop {
                match events.recv().await {
                    Ok(Event::TabCompletion(result)) if result.transaction_id == transaction_id => {
                        return Ok::<_, anyhow::Error>(result);
                    }
                    Ok(Event::Disconnected { reason }) => bail!("disconnected: {reason}"),
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => bail!("event stream closed"),
                }
            }
        })
        .await
        .context("timed out waiting for tab completion")??)
    }
    /// Dispatches a slash command through the chat protocol.
    pub async fn send_command(&self, command: &str) -> Result<()> {
        let command = command.strip_prefix('/').unwrap_or(command);
        self.send_chat(&format!("/{command}")).await
    }
    /// Applies a predicted click locally, dispatches it, and returns its action ID.
    ///
    /// This does not wait for server acceptance. Use
    /// [`Bot::click_slot_and_wait`] for an acknowledged transaction.
    pub async fn click_slot(
        &self,
        window_id: i8,
        slot: i16,
        button: i8,
        mode: ClickMode,
    ) -> Result<i16> {
        let (action, transaction) = self
            .click_slot_transaction(window_id, slot, button, mode)
            .await?;
        // This compatibility API returns before confirmation. Keep the
        // actor-owned transaction alive and clean prediction metadata if its
        // delivery deadline expires; dropping a caller future must not cancel
        // the actor's pending transaction.
        let monitor = self.clone_internal();
        tokio::spawn(async move {
            if transaction.wait().await == DispatchOutcome::DeliveryUnknown {
                monitor.rollback_pending_click(window_id, action).await;
            }
        });
        Ok(action)
    }

    async fn rollback_pending_click(&self, window_id: i8, action: i16) {
        let _coherent_state = self.coherent_state_gate.lock().await;
        let mut inventory = self.inventory.write().await;
        if let Some(pending) = inventory.pending_clicks.remove(&(window_id, action)) {
            rollback_click(&mut inventory, &pending);
            sync_player_inventory_from_window(&mut inventory, window_id);
        }
    }

    async fn click_slot_transaction(
        &self,
        window_id: i8,
        slot: i16,
        button: i8,
        mode: ClickMode,
    ) -> Result<(i16, ProtocolTransaction)> {
        let _click_guard = self.click_lock.lock().await;
        let _coherent_state = self.coherent_state_gate.lock().await;
        let (clicked, slot_before, cursor_before) = {
            let mut inventory = self.inventory.write().await;
            if window_id != 0
                && inventory.open_window.as_ref().map(|window| window.id) != Some(window_id)
            {
                bail!("window {window_id} is not open");
            }
            if inventory
                .pending_clicks
                .keys()
                .any(|(pending_window, _)| *pending_window == window_id)
            {
                bail!("window {window_id} already has an unacknowledged click");
            }
            let clicked = if slot >= 0 {
                inventory
                    .windows
                    .get(&window_id)
                    .and_then(|slots| slots.get(slot as usize))
                    .cloned()
                    .flatten()
            } else {
                None
            };
            let slot_before = clicked.clone();
            let cursor_before = inventory.cursor.clone();
            if mode == ClickMode::Normal {
                predict_normal_click(&mut inventory, window_id, slot, button)?;
                sync_player_inventory_from_window(&mut inventory, window_id);
            }
            (clicked, slot_before, cursor_before)
        };
        let transaction = match self
            .dispatch_acknowledged(
                self.operation_context(0),
                AcknowledgedOperation::WindowClick {
                    window_id,
                    slot,
                    button,
                    mode,
                    clicked,
                },
            )
            .await
        {
            Ok(transaction) => transaction,
            Err(error) => {
                let mut inventory = self.inventory.write().await;
                if mode == ClickMode::Normal {
                    let pending = PendingClick {
                        window_id,
                        action: 0,
                        slot,
                        button,
                        mode,
                        slot_before,
                        cursor_before,
                        prediction: Default::default(),
                    };
                    rollback_click(&mut inventory, &pending);
                    sync_player_inventory_from_window(&mut inventory, window_id);
                }
                let message = anyhow::anyhow!(error.to_string());
                return Err(crate::Error::new(
                    if matches!(error, DispatchError::DeliveryUnknown) {
                        crate::ErrorKind::UncertainDispatch
                    } else {
                        crate::ErrorKind::State
                    },
                    message,
                ));
            }
        };
        if !transaction.was_dispatched() {
            let mut inventory = self.inventory.write().await;
            if mode == ClickMode::Normal {
                let pending = PendingClick {
                    window_id,
                    action: 0,
                    slot,
                    button,
                    mode,
                    slot_before,
                    cursor_before,
                    prediction: Default::default(),
                };
                rollback_click(&mut inventory, &pending);
                sync_player_inventory_from_window(&mut inventory, window_id);
            }
            return Err(crate::Error::new(
                crate::ErrorKind::Connection,
                anyhow::anyhow!("window click delivery is unknown"),
            ));
        }
        let action = transaction
            .window_action()
            .context("window click transaction did not expose a local action")?;
        let mut inventory = self.inventory.write().await;
        inventory.pending_clicks.insert(
            (window_id, action),
            PendingClick {
                window_id,
                action,
                slot,
                button,
                mode,
                slot_before,
                cursor_before,
                prediction: Default::default(),
            },
        );
        drop(inventory);
        Ok((action, transaction))
    }

    /// Dispatches a click and waits for the matching protocol transaction.
    pub async fn click_slot_and_wait(
        &self,
        window_id: i8,
        slot: i16,
        button: i8,
        mode: ClickMode,
    ) -> Result<i16> {
        let (action, transaction) = self
            .click_slot_transaction(window_id, slot, button, mode)
            .await?;
        match transaction.wait().await {
            crate::DispatchOutcome::Acknowledged => Ok(action),
            crate::DispatchOutcome::Rejected => {
                bail!(
                    "server rejected window action {action} (window {window_id}, slot {slot}, button {button}, mode {mode:?})"
                );
            }
            crate::DispatchOutcome::DeliveryUnknown => {
                self.rollback_pending_click(window_id, action).await;
                bail!("delivery of window action {action} is unknown");
            }
            crate::DispatchOutcome::Dispatched => {
                bail!("window action {action} was dispatched without acknowledgement");
            }
        }
    }
    /// Performs the `craft_once` operation.
    pub async fn craft_once(
        &self,
        window_id: i8,
        width: usize,
        height: usize,
        ingredients: &[Option<i32>],
    ) -> Result<()> {
        if !matches!((window_id, width, height), (0, 2, 2) | (_, 3, 3)) {
            bail!("player crafting must be 2x2 and an open container must be 3x3");
        }
        if ingredients.len() != width * height {
            bail!("ingredient grid length does not match dimensions");
        }
        let grid_end = 1 + ingredients.len();
        self.compact_player_inventory(window_id).await?;
        // A previous attempt can leave ingredients in the grid. Return them before
        // placing a fresh recipe; never consume a partially reconciled recipe twice.
        for slot in 1..grid_end {
            self.return_slot_to_storage(window_id, slot).await?;
        }
        let source_start = if window_id == 0 { 9 } else { grid_end };
        for (grid_index, ingredient) in ingredients.iter().enumerate() {
            let Some(item_id) = ingredient else {
                continue;
            };
            let source = self
                .inventory()
                .await
                .windows
                .get(&window_id)
                .and_then(|slots| {
                    slots
                        .iter()
                        .enumerate()
                        .skip(source_start)
                        .take(36)
                        .find(|(_, item)| {
                            item.as_ref().is_some_and(|item| item.item_id == *item_id)
                        })
                        .map(|(slot, _)| slot as i16)
                })
                .with_context(|| format!("ingredient item {item_id} is unavailable"))?;
            self.click_slot_and_wait(window_id, source, 0, ClickMode::Normal)
                .await
                .with_context(|| format!("pick ingredient {item_id} from slot {source}"))?;
            self.click_slot_and_wait(window_id, (1 + grid_index) as i16, 1, ClickMode::Normal)
                .await
                .with_context(|| format!("place ingredient {item_id} into crafting grid"))?;
            if self.inventory().await.cursor.is_some() {
                self.click_slot_and_wait(window_id, source, 0, ClickMode::Normal)
                    .await
                    .with_context(|| format!("return remaining ingredient {item_id}"))?;
            }
        }
        timeout(Duration::from_secs(5), async {
            let mut events = self.subscribe();
            loop {
                if self
                    .inventory()
                    .await
                    .windows
                    .get(&window_id)
                    .and_then(|slots| slots.first())
                    .is_some_and(Option::is_some)
                {
                    return Ok::<(), anyhow::Error>(());
                }
                let _ = next_operation_event(&mut events).await?;
            }
        })
        .await
        .context("timed out waiting for crafting result")??;
        self.take_crafting_result(window_id, ingredients.len())
            .await
    }
    /// Performs the `take_crafting_result` operation.
    pub async fn take_crafting_result(&self, window_id: i8, grid_slots: usize) -> Result<()> {
        let inventory = self.inventory().await;
        check_inventory(inventory.cursor.is_none(), "crafting cursor is occupied")?;
        let result = inventory
            .windows
            .get(&window_id)
            .and_then(|s| s.first())
            .and_then(Option::as_ref)
            .context("crafting result is unavailable")?;
        check_inventory(
            storage_capacity(&inventory, window_id, result)? >= i16::from(result.count),
            "insufficient compatible player storage for crafting result; result not taken",
        )?;
        self.click_slot_and_wait(window_id, 0, 0, ClickMode::Normal)
            .await
            .context("collect crafting result")?;
        {
            let _coherent_state = self.coherent_state_gate.lock().await;
            // Accepted crafting-result clicks consume one item from every occupied
            // recipe slot client-side; vanilla does not necessarily echo these slots.
            let mut inventory = self.inventory.write().await;
            if let Some(slots) = inventory.windows.get_mut(&window_id) {
                for slot in slots.iter_mut().take(1 + grid_slots).skip(1).flatten() {
                    slot.count -= 1;
                }
                for item in slots.iter_mut().take(1 + grid_slots).skip(1) {
                    if item.as_ref().is_some_and(|item| item.count <= 0) {
                        *item = None;
                    }
                }
                slots[0] = None;
            }
            sync_player_inventory_from_window(&mut inventory, window_id);
        }
        self.store_cursor(window_id)
            .await
            .context("store crafting result")?;
        Ok(())
    }

    async fn store_cursor(&self, window: i8) -> Result<()> {
        // Each successful click fills a destination or empties the cursor.
        for _ in 0..=36 {
            let inventory = self.inventory().await;
            let Some(cursor) = inventory.cursor.as_ref() else {
                return Ok(());
            };
            check_inventory(
                storage_capacity(&inventory, window, cursor)? >= i16::from(cursor.count),
                "insufficient compatible storage for cursor; no items discarded",
            )?;
            let slots = &inventory.windows[&window];
            let range = storage_range(window, slots.len())?;
            let destination = range
                .clone()
                .find(|i| slots[*i].is_some() && merge_room(slots[*i].as_ref(), cursor) > 0)
                .or_else(|| range.clone().find(|i| slots[*i].is_none()))
                .context("no cursor destination")?;
            let moved =
                merge_room(slots[destination].as_ref(), cursor).min(i16::from(cursor.count));
            let mut expected_slot = cursor.clone();
            expected_slot.count =
                (i16::from(slots[destination].as_ref().map_or(0, |s| s.count)) + moved) as i8;
            let mut expected_cursor = cursor.clone();
            expected_cursor.count -= moved as i8;
            self.click_slot_and_wait(window, destination as i16, 0, ClickMode::Normal)
                .await?;
            let after = self.inventory().await;
            check_inventory(
                after.windows.get(&window).and_then(|s| s.get(destination))
                    == Some(&Some(expected_slot))
                    && after.cursor == (expected_cursor.count > 0).then_some(expected_cursor),
                "cursor transfer outcome changed; refusing further clicks",
            )?;
        }
        bail!("cursor reconciliation budget exhausted")
    }

    async fn return_slot_to_storage(&self, window: i8, source: usize) -> Result<()> {
        let inventory = self.inventory().await;
        check_inventory(
            inventory.cursor.is_none(),
            "cursor must be reconciled first",
        )?;
        let Some(item) = inventory
            .windows
            .get(&window)
            .and_then(|s| s.get(source))
            .and_then(Option::as_ref)
        else {
            return Ok(());
        };
        check_inventory(
            storage_capacity(&inventory, window, item)? >= i16::from(item.count),
            &format!("insufficient compatible storage to return slot {source}"),
        )?;
        self.click_slot_and_wait(window, source as i16, 0, ClickMode::Normal)
            .await?;
        let after = self.inventory().await;
        check_inventory(
            after.cursor.as_ref() == Some(item)
                && after
                    .windows
                    .get(&window)
                    .and_then(|s| s.get(source))
                    .is_some_and(Option::is_none),
            "source pickup outcome changed; refusing further clicks",
        )?;
        self.store_cursor(window).await
    }

    /// Reconcile the cursor and merge compatible ordinary-storage stacks.
    /// Never drops items, uses equipment as spare storage, or merges different NBT.
    /// Rejected/ambiguous clicks stop the operation; callers must not replay blindly.
    pub async fn compact_player_inventory(&self, window: i8) -> Result<()> {
        self.store_cursor(window).await?;
        let inventory = self.inventory().await;
        let range = storage_range(
            window,
            inventory
                .windows
                .get(&window)
                .context("window unavailable")?
                .len(),
        )?;
        for source in range.clone().rev() {
            let inventory = self.inventory().await;
            let slots = inventory
                .windows
                .get(&window)
                .context("window changed during compaction")?;
            let Some(item) = slots.get(source).and_then(Option::as_ref) else {
                continue;
            };
            if (range.start..source)
                .any(|i| slots[i].is_some() && merge_room(slots[i].as_ref(), item) > 0)
            {
                self.click_slot_and_wait(window, source as i16, 0, ClickMode::Normal)
                    .await?;
                let after = self.inventory().await;
                check_inventory(
                    after.cursor.as_ref() == Some(item)
                        && after
                            .windows
                            .get(&window)
                            .and_then(|s| s.get(source))
                            .is_some_and(Option::is_none),
                    "compaction pickup outcome changed; refusing further clicks",
                )?;
                self.store_cursor(window).await?;
            }
        }
        if window == 0 {
            let inventory = self.inventory().await;
            if let Some(item) = inventory
                .windows
                .get(&0)
                .and_then(|s| s.get(45))
                .and_then(Option::as_ref)
            {
                if storage_capacity(&inventory, 0, item)? >= i16::from(item.count) {
                    self.return_slot_to_storage(0, 45).await?;
                }
            }
        }
        Ok(())
    }
    /// Dispatches recipe placement without waiting for inventory synchronization.
    pub async fn place_recipe(&self, window_id: i8, recipe_id: &str, make_all: bool) -> Result<()> {
        let mut payload = vec![window_id as u8];
        put_string(&mut payload, recipe_id);
        payload.push(u8::from(make_all));
        self.send(0x19, &payload).await
    }
    /// Dispatches close-window and immediately clears the predicted local window.
    pub async fn close_window(&self) -> Result<()> {
        let _coherent_state = self.coherent_state_gate.lock().await;
        let window_id = self
            .inventory
            .read()
            .await
            .open_window
            .as_ref()
            .map(|window| window.id)
            .context("no window is open")?;
        self.send(0x0a, &[window_id as u8]).await?;
        let mut inventory = self.inventory.write().await;
        inventory.open_window = None;
        inventory.last_transaction = None;
        inventory.merchant_offers = None;
        inventory.windows.remove(&window_id);
        inventory.properties.retain(|(id, _), _| *id != window_id);
        inventory
            .pending_clicks
            .retain(|(id, _), _| *id != window_id);
        let mut furnace_window_position = self.furnace_window_position.lock().await;
        if furnace_window_position.is_some_and(|(id, _)| id == window_id) {
            *furnace_window_position = None;
        }
        drop(furnace_window_position);
        drop(inventory);
        self.emit(Event::WindowClosed { window_id });
        Ok(())
    }
    /// Selects one server-advertised merchant offer without choosing it automatically.
    pub async fn select_trade(&self, index: usize) -> Result<()> {
        let offers = self
            .merchant_offers()
            .await
            .context("no merchant offers are open")?;
        if index >= offers.offers.len() {
            bail!("merchant offer index is out of range");
        }
        let mut payload = Vec::new();
        put_varint(&mut payload, i32::try_from(index)?);
        self.send(0x22, &payload).await
    }

    /// Selects one of the three enchantment-table options.
    pub async fn select_enchantment(&self, option: u8) -> Result<()> {
        if option > 2 {
            bail!("enchantment option must be 0..=2");
        }
        let window_id = self
            .open_window_state()
            .await
            .context("no window is open")?
            .id;
        self.send(0x08, &[window_id as u8, option]).await
    }

    /// Sends the raw anvil rename text for the currently open anvil.
    pub async fn rename_item(&self, name: &str) -> Result<()> {
        if name.chars().count() > 50 {
            bail!("item name must not exceed 50 characters");
        }
        let mut payload = Vec::new();
        put_string(&mut payload, name);
        self.send(0x1f, &payload).await
    }

    /// Performs the `set_beacon_effects` operation.
    pub async fn set_beacon_effects(&self, primary: i32, secondary: i32) -> Result<()> {
        let mut payload = Vec::new();
        put_varint(&mut payload, primary);
        put_varint(&mut payload, secondary);
        self.send(0x23, &payload).await
    }

    /// Performs the `update_sign` operation.
    pub async fn update_sign(&self, position: BlockPos, lines: [&str; 4]) -> Result<()> {
        if lines.iter().any(|line| line.chars().count() > 384) {
            bail!("sign lines must not exceed 384 characters");
        }
        let mut payload = Vec::new();
        payload.write_u64::<BigEndian>(position.packed())?;
        for line in lines {
            put_string(&mut payload, line);
        }
        self.send(0x2a, &payload).await
    }

    /// Performs the `swap_hands` operation.
    pub async fn swap_hands(&self) -> Result<()> {
        self.send_digging_packet(
            DiggingStatus::SwapHands,
            BlockPos { x: 0, y: 0, z: 0 },
            BlockFace::Down,
        )
        .await
    }
    /// Starts the operation barrier, stops local movement, closes the TCP
    /// writer, and waits for the reader to publish a terminal transport fact.
    ///
    /// This method owns no timeout. Callers that require a logout deadline must
    /// apply it outside this client.
    pub async fn disconnect(&self) -> Result<()> {
        self.connection.begin_disconnect().await.map_err(|error| {
            crate::Error::new(
                crate::ErrorKind::State,
                anyhow::anyhow!("disconnect rejected: {error:?}"),
            )
        })?;
        let cleanup = self
            .dispatch_cleanup(self.operation_context(0), CleanupOperation::ControlClear)
            .await
            .map_err(|error| {
                crate::Error::new(
                    crate::ErrorKind::State,
                    anyhow::anyhow!("disconnect control cleanup rejected: {error:?}"),
                )
            })?;
        require_disconnect_control_cleanup(cleanup)?;
        if let Err(error) = self.connection.shutdown_writer().await {
            self.connection
                .mark_unknown("shutdown_writer_failed", error.to_string())
                .await;
            return Err(error);
        }
        match self.connection.wait_for_terminal().await {
            ConnectionState::Disconnected => Ok(()),
            ConnectionState::ConnectionStateUnknown => {
                bail!("connection state became unknown while disconnecting")
            }
            _ => unreachable!("terminal wait returned a non-terminal lifecycle"),
        }
    }

    // Atomic::try_update is unavailable on our Rust 1.85 MSRV.
    #[allow(deprecated)]
    async fn control_loop(&self) {
        if self.wait_until_ready().await.is_err() {
            return;
        }
        let mut ticker = tokio::time::interval(Duration::from_millis(50));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut jump_held = false;
        let mut sprinting = false;
        let mut sneaking = false;
        while !self.stopped.load(Ordering::Acquire) {
            let scheduled = ticker.tick().await;
            if self.common_native_physics_paused().await {
                continue;
            }
            let lag = tokio::time::Instant::now().saturating_duration_since(scheduled);
            if self
                .teleport_barrier_ticks
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |ticks| {
                    ticks.checked_sub(1)
                })
                .is_ok()
            {
                self.physics.lock().await.record_tick(Duration::ZERO, lag);
                continue;
            }
            let (control, movement_fraction) = self
                .connection
                .consume_control_for_tick(Duration::from_millis(50))
                .await;
            if control.sprint != sprinting {
                if self
                    .send_entity_action(if control.sprint { 3 } else { 4 })
                    .await
                    .is_err()
                {
                    // A finite owner may acquire actor admission after this
                    // loop sampled released controls. Keep the loop alive;
                    // retry posture reconciliation only after the run settles.
                    if self.common_native_physics_paused().await {
                        continue;
                    }
                    break;
                }
                sprinting = control.sprint;
            }
            if control.sneak != sneaking {
                if self
                    .send_entity_action(if control.sneak { 0 } else { 1 })
                    .await
                    .is_err()
                {
                    // A finite owner may acquire actor admission after this
                    // loop sampled released controls. Keep the loop alive;
                    // retry posture reconciliation only after the run settles.
                    if self.common_native_physics_paused().await {
                        continue;
                    }
                    break;
                }
                sneaking = control.sneak;
            }
            let jump =
                (control.jump && !jump_held) || self.jump_requested.swap(false, Ordering::AcqRel);
            jump_held = control.jump;
            let tick_started = std::time::Instant::now();
            if self
                .physics_tick_with_fraction(control, jump, movement_fraction)
                .await
                .is_err()
            {
                break;
            }
            self.physics
                .lock()
                .await
                .record_tick(tick_started.elapsed(), lag);
        }
    }

    #[cfg(test)]
    async fn physics_tick(&self, control: ControlState, jump: bool) -> Result<()> {
        self.physics_tick_with_fraction(control, jump, 1.0).await
    }

    async fn physics_tick_with_fraction(
        &self,
        control: ControlState,
        jump: bool,
        movement_fraction: f64,
    ) -> Result<()> {
        let _coherent_state = self.coherent_state_gate.lock().await;
        // Respawn keeps compatibility coordinates until the new own-position
        // packet. New chunks alone cannot make that preceding-world pose valid.
        if !*self.positioned.lock().await {
            self.common_receipts.lock().await.ground_source = None;
            return Ok(());
        }
        if self.common_native_physics_paused().await {
            return Ok(());
        }
        let survival = self.survival.read().await;
        if survival.game_mode == Some(1)
            && survival.flying_allowed
            && self.common_receipts.lock().await.requested_flying
        {
            return Ok(());
        }
        let movement_attribute = survival
            .attributes
            .get("minecraft:generic.movement_speed")
            .or_else(|| survival.attributes.get("generic.movement_speed"));
        let movement_speed = movement_speed_for_control(movement_attribute, control.sprint);
        drop(survival);
        let mut player = self.player.lock().await;
        let mut motion = self.motion.lock().await;
        let was_on_ground = player.on_ground;
        // A partial/unknown collision update must not expose the old flag as current.
        self.common_receipts.lock().await.ground_source = None;
        let mut forward = f64::from(i8::from(control.forward) - i8::from(control.back));
        let mut strafe = f64::from(i8::from(control.right) - i8::from(control.left));
        if control.sneak {
            forward *= 0.3;
            strafe *= 0.3;
        }
        let input_length = (forward * forward + strafe * strafe).sqrt();
        if input_length > 1.0 {
            forward /= input_length;
            strafe /= input_length;
        }
        forward *= 0.98 * movement_fraction;
        strafe *= 0.98 * movement_fraction;

        let aabb = Aabb::player(player.x, player.y, player.z);
        let world = self.world.lock().await;
        let fluid = world.fluid_intersecting(aabb);
        let fluid_flow = fluid.map(|kind| world.fluid_flow(aabb, kind));
        let climbing = world.is_climbable(aabb);
        let below = world.block_below_name(aabb);
        let effects = world.contact_effects(aabb);
        let slipperiness: f64 = match below {
            Some("blue_ice") => 0.989,
            Some("ice" | "packed_ice" | "frosted_ice") => 0.98,
            _ => 0.6,
        };
        let ground_friction: f64 = 0.91 * slipperiness;
        // Vanilla applies the +30% sprint modifier locally as soon as sprint
        // input is active. A server attribute update may already carry the
        // stable sprint UUID; only that exact evidence suppresses local
        // application so the modifier is never doubled.
        let acceleration = if fluid.is_some() {
            0.02
        } else if was_on_ground {
            movement_speed * (0.162_771_36 / ground_friction.powi(3))
        } else if control.sprint {
            0.026
        } else {
            0.02
        };
        let yaw = (player.yaw as f64).to_radians();
        motion.velocity.x += (strafe * yaw.cos() - forward * yaw.sin()) * acceleration;
        motion.velocity.z += (forward * yaw.cos() + strafe * yaw.sin()) * acceleration;
        if let Some(flow) = fluid_flow {
            motion.velocity.x += flow.x * 0.014;
            motion.velocity.z += flow.z * 0.014;
        }
        if control.jump && fluid.is_some_and(|kind| world.swimming_jump(aabb, kind, was_on_ground))
        {
            motion.velocity.y += 0.04;
        } else if jump && was_on_ground {
            motion.velocity.y = if below == Some("honey_block") {
                0.42 * 0.4
            } else {
                0.42
            };
            if control.sprint {
                motion.velocity.x -= yaw.sin() * 0.2;
                motion.velocity.z += yaw.cos() * 0.2;
            }
        }
        if climbing {
            motion.velocity.x = motion.velocity.x.clamp(-0.15, 0.15);
            motion.velocity.z = motion.velocity.z.clamp(-0.15, 0.15);
            motion.velocity.y = motion
                .velocity
                .y
                .max(if control.sneak { 0.0 } else { -0.15 });
            motion.fall_distance = 0.0;
            if control.jump {
                motion.velocity.y = 0.2;
            }
        }
        if effects.cobweb {
            motion.velocity.x *= 0.25;
            motion.velocity.y *= 0.05;
            motion.velocity.z *= 0.25;
        } else if effects.berry_bush {
            motion.velocity.x *= 0.8;
            motion.velocity.y *= 0.75;
            motion.velocity.z *= 0.8;
        }
        if effects.bubble_push != 0 {
            motion.velocity.y += if effects.bubble_push > 0 { 0.06 } else { -0.03 };
        }

        let mut requested = motion.velocity;
        if control.sneak && was_on_ground {
            let Some((dx, dz)) = world.limit_sneak_movement(aabb, requested.x, requested.z) else {
                motion.velocity = Vec3::default();
                return Ok(());
            };
            requested.x = dx;
            requested.z = dz;
        }
        let Some((moved, actual)) = world.collide_with_step(aabb, requested, was_on_ground, 0.6)
        else {
            motion.velocity = Vec3::default();
            return Ok(());
        };
        let landed_on = world.block_below_name(moved);
        let collided_x = (requested.x - actual.x).abs() > 1.0e-9;
        let collided_y = (requested.y - actual.y).abs() > 1.0e-9;
        let collided_z = (requested.z - actual.z).abs() > 1.0e-9;
        self.common_receipts.lock().await.position_source =
            Some(crate::client::ValueSource::Predicted);
        player.x = (moved.min_x + moved.max_x) * 0.5;
        player.y = moved.min_y;
        player.z = (moved.min_z + moved.max_z) * 0.5;
        player.on_ground = collided_y && requested.y < 0.0;
        self.common_receipts.lock().await.ground_source =
            Some(crate::client::ValueSource::Predicted);
        motion.collided_horizontal = collided_x || collided_z;
        motion.collided_vertical = collided_y;
        if player.on_ground {
            motion.fall_distance = 0.0;
        } else if actual.y < 0.0 {
            motion.fall_distance += (-actual.y) as f32;
        }
        if collided_x {
            motion.velocity.x = 0.0;
        }
        if collided_y {
            motion.velocity.y =
                if requested.y < 0.0 && landed_on == Some("slime_block") && !control.sneak {
                    -requested.y
                } else {
                    0.0
                };
        }
        if collided_z {
            motion.velocity.z = 0.0;
        }
        if climbing && motion.collided_horizontal {
            motion.velocity.y = 0.2;
        }
        match fluid {
            Some(Fluid::Water) => {
                motion.velocity.x *= 0.8;
                motion.velocity.y = motion.velocity.y * 0.8 - 0.02;
                motion.velocity.z *= 0.8;
            }
            Some(Fluid::Lava) => {
                motion.velocity.x *= 0.5;
                motion.velocity.y = motion.velocity.y * 0.5 - 0.02;
                motion.velocity.z *= 0.5;
            }
            None => {
                motion.velocity.y = (motion.velocity.y - 0.08) * 0.98;
                let horizontal_drag = if was_on_ground { ground_friction } else { 0.91 };
                motion.velocity.x *= horizontal_drag;
                motion.velocity.z *= horizontal_drag;
            }
        }
        if let Some(up) = world.fluid_exit_boost(
            moved,
            motion.velocity,
            actual.y,
            fluid,
            motion.collided_horizontal,
        ) {
            motion.velocity.y = up;
        }
        drop(world);
        if player.on_ground && matches!(landed_on, Some("soul_sand" | "honey_block")) {
            motion.velocity.x *= 0.4;
            motion.velocity.z *= 0.4;
        }
        motion.ticks += 1;
        drop(motion);
        drop(player);
        self.send_position().await
    }

    async fn read_loop<R: tokio::io::AsyncRead + Unpin>(
        &self,
        mut reader: R,
        mut capture_requests: mpsc::Receiver<crate::observation::CaptureCommand>,
        mut traversal_movement_facts_requests: mpsc::Receiver<
            crate::observation::TraversalMovementFactsCommand,
        >,
    ) -> Result<()> {
        let mut next_observation_sequence = 1_u64;
        let mut packets_since_yield = 0_u8;
        // Keep cancellation registered across packet application and batch yields.
        let cancelled = self.cancel.notified();
        tokio::pin!(cancelled);
        cancelled.as_mut().enable();
        // Compression negotiation finishes before entering the play reader.
        let compression = self.writer.lock().await.compression;
        let diagnostics = ReaderDiagnostics::new(self.connection_generation().get());
        // Set when cancellation interrupts reading: frames already received may still
        // carry the server's kick reason (see `drain_disconnect_reason`).
        let mut cancelled_frame: Option<Option<Frame>> = None;
        while !self.stopped.load(Ordering::Acquire) {
            diagnostics.enter(ReaderPhase::Reading, None);
            diagnostics.read_started();
            // A capture may interrupt waiting, but must not discard bytes
            // already consumed from this packet. Keep both read progress and
            // its original deadline until the packet completes or we exit.
            let packet = {
                let packet_read = timeout(
                    self.connection_options.play_packet_timeout,
                    read_packet(&mut reader, compression),
                );
                tokio::pin!(packet_read);
                loop {
                    tokio::select! {
                        biased;
                        _ = &mut cancelled => {
                            cancelled_frame = Some(if self.fail_closed() {
                                finish_frame(packet_read.as_mut()).await
                            } else {
                                None
                            });
                            break None;
                        }
                        request = capture_requests.recv() => {
                            let Some(request) = request else {
                                return Ok(());
                            };
                            let started = std::time::Instant::now();
                            diagnostics.enter(ReaderPhase::Capture, None);
                            let result = self
                                .capture_observation_at_sequence(request.request, next_observation_sequence)
                                .await;
                            self.trace_slow_capture("coherent", next_observation_sequence, started.elapsed(), result.is_ok());
                            diagnostics.capture_completed();
                            diagnostics.enter(ReaderPhase::Reading, None);
                            if result.is_ok() {
                                next_observation_sequence = next_observation_sequence
                                    .checked_add(1)
                                    .context("coherent observation sequence exhausted")?;
                            }
                            let _ = request.reply.send(result);
                        }
                        request = traversal_movement_facts_requests.recv() => {
                            let Some(request) = request else {
                                return Ok(());
                            };
                            let started = std::time::Instant::now();
                            diagnostics.enter(ReaderPhase::MovementCapture, None);
                            let result = self
                                .capture_traversal_movement_facts_at_sequence(
                                    request.request,
                                    next_observation_sequence,
                                )
                                .await;
                            self.trace_slow_capture("movement", next_observation_sequence, started.elapsed(), result.is_ok());
                            diagnostics.capture_completed();
                            diagnostics.enter(ReaderPhase::Reading, None);
                            if result.is_ok() {
                                next_observation_sequence = next_observation_sequence
                                    .checked_add(1)
                                    .context("coherent observation sequence exhausted")?;
                            }
                            let _ = request.reply.send(result);
                        }
                        packet = std::future::poll_fn(|cx| diagnostics.poll(packet_read.as_mut(), cx)) => break Some(packet.context("play packet timed out")?),
                    }
                    // Give the same packet read (and its deadline) one poll after
                    // each capture, without letting a packet backlog starve captures.
                    tokio::select! {
                        biased;
                        _ = &mut cancelled => {
                            cancelled_frame = Some(if self.fail_closed() {
                                finish_frame(packet_read.as_mut()).await
                            } else {
                                None
                            });
                            break None;
                        }
                        packet = std::future::poll_fn(|cx| {
                            std::task::Poll::Ready(diagnostics.poll(packet_read.as_mut(), cx))
                        }) => {
                            if let std::task::Poll::Ready(packet) = packet {
                                break Some(packet.context("play packet timed out")?);
                            }
                        }
                    }
                }
            };
            let Some(packet) = packet else {
                break;
            };
            let (id, p) = packet?;
            diagnostics.enter(ReaderPhase::ApplyGate, Some(id));
            let applied = self.apply_packet_diagnosed(id, p, Some(&diagnostics)).await;
            self.packet_applied.notify_waiters();
            let rejected_during_disconnect = applied.as_ref().err().is_some_and(|error| {
                self.connection.lifecycle() == ConnectionState::Disconnecting
                    && error
                        .diagnostic()
                        .downcast_ref::<super::lifecycle::ProtocolDispatchRejected>()
                        .is_some_and(|rejection| rejection.state == ConnectionState::Disconnecting)
            });
            if !rejected_during_disconnect {
                if applied.is_err() && self.fail_closed() {
                    // A failed reply write may precede the server's kick reason.
                    self.drain_disconnect_reason(None, &mut reader, compression)
                        .await;
                }
                if !applied? {
                    break;
                }
                diagnostics.applied();
            }
            // Intentional pre-write rejection is neither a delivered reply nor
            // a transport end. Consume subsequent frames until actual EOF (or
            // another real error), retaining the ordinary batch yield below.
            packets_since_yield += 1;
            if packets_since_yield == 32 {
                // Yield without cache/gate guards even when all frames and locks are ready.
                packets_since_yield = 0;
                diagnostics.enter(ReaderPhase::Yielding, None);
                tokio::task::yield_now().await;
            }
        }
        // Only a completed in-flight frame keeps the following frames aligned.
        if let Some(Some(first)) = cancelled_frame {
            self.drain_disconnect_reason(Some(first), &mut reader, compression)
                .await;
        }
        Ok(())
    }

    /// The connection already ended unclassified (a failed write): the only case in
    /// which remaining frames are read for a kick reason.
    fn fail_closed(&self) -> bool {
        self.connection.lifecycle() == ConnectionState::ConnectionStateUnknown
    }

    /// After a fail-close (for example a write that failed because the server already
    /// closed the socket), frames the server sent before closing may still be readable.
    /// Only a kick's reason is recorded from them; nothing else is applied and the
    /// lifecycle classification is unchanged.
    async fn drain_disconnect_reason<R: tokio::io::AsyncRead + Unpin>(
        &self,
        first: Option<Frame>,
        reader: &mut R,
        compression: Option<i32>,
    ) {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(200);
        let mut frame = match first {
            Some(frame) => Some(frame),
            None => tokio::time::timeout_at(deadline, read_packet(reader, compression))
                .await
                .ok()
                .map(|r| r.map_err(Into::into)),
        };
        for _ in 0..64 {
            match frame {
                Some(Ok((0x1a, payload))) => {
                    if let Ok(reason) = get_string(&mut payload.as_slice()) {
                        self.common_receipts.lock().await.disconnect_reason =
                            Some(crate::client::ui::UiText::LegacyJson { json: reason });
                    }
                    return;
                }
                Some(Ok(_)) => {}
                _ => return,
            }
            frame = tokio::time::timeout_at(deadline, read_packet(reader, compression))
                .await
                .ok()
                .map(|r| r.map_err(Into::into));
        }
    }

    fn trace_slow_capture(&self, kind: &str, sequence: u64, elapsed: Duration, success: bool) {
        if elapsed >= Duration::from_millis(100) {
            crate::lifecycle::emit_protocol_timing(|| {
                serde_json::json!({"stage":"slow_capture","generation":self.connection_generation().get(),
                    "kind":kind,"sequence":sequence,"elapsed_ms":elapsed.as_millis(),"success":success})
            });
        }
    }

    #[cfg(test)]
    async fn apply_packet(&self, id: i32, p: Vec<u8>) -> Result<bool> {
        self.apply_packet_diagnosed(id, p, None).await
    }

    async fn apply_packet_diagnosed(
        &self,
        id: i32,
        p: Vec<u8>,
        diagnostics: Option<&ReaderDiagnostics>,
    ) -> Result<bool> {
        let keepalive_started = (id == 0x20).then(std::time::Instant::now);
        if keepalive_started.is_some() {
            crate::lifecycle::emit_protocol_timing(|| {
                serde_json::json!({"stage":"keepalive_frame_decoded","generation":self.connection_generation().get(),
                    "keepalive_id":p.as_slice().try_into().ok().map(i64::from_be_bytes)})
            });
        }
        let _coherent_state = self.coherent_state_gate.lock().await;
        if let Some(diagnostics) = diagnostics {
            diagnostics.enter(ReaderPhase::Applying, Some(id));
        }
        if let Some(started) = keepalive_started {
            crate::lifecycle::emit_protocol_timing(|| {
                serde_json::json!({"stage":"keepalive_gate_acquired","generation":self.connection_generation().get(),
                    "keepalive_id":p.as_slice().try_into().ok().map(i64::from_be_bytes),
                    "elapsed_ms":started.elapsed().as_millis()})
            });
        }
        let packet_sequence = self
            .protocol_packet_sequence
            .fetch_add(1, Ordering::AcqRel)
            .checked_add(1)
            .context("protocol packet sequence exhausted")?;
        if let Some(trace) = lock_packet_state(&self.packet_trace).await.as_mut() {
            let local_player_basis = if id == 0x35 {
                let player = self.player.lock().await;
                Some(crate::client::recording::LocalPlayerBasis {
                    position: Some([player.x, player.y, player.z]),
                    rotation: [player.yaw, player.pitch],
                    velocity: None,
                })
            } else {
                None
            };
            trace.record(
                packet_sequence,
                self.connected_at.elapsed().as_millis() as u64 / 50,
                crate::client::recording::PacketPhase::Play,
                id,
                &p,
                local_player_basis,
            );
        }
        {
            let mut receipts = self.common_receipts.lock().await;
            let generation = receipts.generation;
            receipts.entities.history_context(
                crate::MinecraftVersion::Java1_16_1,
                generation,
                packet_sequence,
            );
        }
        if let Some((target, update)) = common_entity_motion::decode(id, &p)? {
            let player = self.player.lock().await.entity_id;
            let mut receipts = self.common_receipts.lock().await;
            let target =
                target.or_else(|| receipts.vehicles.mounted_entity(player, &receipts.entities));
            if let Some(target) = target {
                receipts.entities.receive_motion(
                    crate::MinecraftVersion::Java1_16_1,
                    target,
                    update,
                    packet_sequence,
                );
            }
        }
        match id {
            0x00 => self.insert_entity(parse_spawn_object(&p)?).await?,
            0x01 => self.insert_entity(parse_spawn_orb(&p)?).await?,
            0x02 => self.insert_entity(parse_spawn_living(&p)?).await?,
            0x03 => self.insert_entity(parse_spawn_painting(&p)?).await?,
            0x04 => self.insert_entity(parse_spawn_player(&p)?).await?,
            0x05 => {
                let mut rest = p.as_slice();
                let entity_id = get_varint(&mut rest)?;
                let animation = *rest.first().context("missing animation")?;
                self.common_receipts.lock().await.entities.history_signal(
                    entity_id,
                    None,
                    Some(animation),
                );
                self.emit(Event::EntityAnimation {
                    entity_id,
                    animation,
                });
            }
            0x06 => {
                self.statistics.write().await.apply(&p)?;
                self.emit(Event::StatisticsUpdated);
            }
            0x07 => {
                self.receive_digging_acknowledgement(&p, packet_sequence)
                    .await?
            }
            0x08 => self.emit(Event::BlockBreakProgress(parse_break_progress(&p)?)),
            0x09 => {
                let update = self.world.lock().await.apply_block_entity(&p)?;
                self.world_updated.notify_waiters();
                self.emit(Event::BlockEntityUpdated(update));
            }
            0x0a => {
                let mut cursor = Cursor::new(&p);
                let event = BlockActionEvent {
                    position: BlockPos::unpack(cursor.read_u64::<BigEndian>()?),
                    action: cursor.read_u8()?,
                    parameter: cursor.read_u8()?,
                    block_id: {
                        let mut rest = &p[cursor.position() as usize..];
                        get_varint(&mut rest)?
                    },
                };
                self.emit(Event::BlockAction(event));
            }
            0x0b => self.receive_block_change(&p, packet_sequence).await?,
            0x0c => self.receive_boss_bar(&p, packet_sequence).await?,
            0x0d => {
                let difficulty = Difficulty {
                    id: *p.first().context("missing difficulty")?,
                    locked: *p.get(1).context("missing difficulty lock")? != 0,
                };
                self.survival.write().await.difficulty = Some(difficulty);
                self.emit(Event::Difficulty(difficulty));
            }
            0x0e => {
                let chat = parse_chat(&p)?;
                self.common_chat
                    .lock()
                    .await
                    .receive_legacy(&p, packet_sequence)?;
                self.emit(Event::Chat(chat));
            }
            0x0f => self.receive_multi_block_change(&p, packet_sequence).await?,
            0x10 => self.emit(Event::TabCompletion(parse_tab_completion(&p)?)),
            0x11 => {
                **self.command_tree.write().await = Some(parse_command_tree(&p)?);
                self.emit(Event::CommandTreeUpdated);
            }
            0x12 => {
                self.receive_window_confirmation(&p, packet_sequence)
                    .await?
            }
            0x13 => self.receive_close_window(&p, packet_sequence).await?,
            0x14 => self.receive_window_items(&p, packet_sequence).await?,
            0x15 => {
                let mut c = Cursor::new(&p);
                let property = WindowProperty {
                    window_id: c.read_u8()? as i8,
                    property: c.read_i16::<BigEndian>()?,
                    value: c.read_i16::<BigEndian>()?,
                };
                self.inventory
                    .write()
                    .await
                    .properties
                    .insert((property.window_id, property.property), property.value);
                self.emit(Event::WindowProperty(property));
            }
            0x16 => self.receive_set_slot(&p, packet_sequence).await?,
            0x17 => {
                let mut rest = p.as_slice();
                let item_id = get_varint(&mut rest)?;
                let ticks = get_varint(&mut rest)?;
                self.survival
                    .write()
                    .await
                    .item_cooldowns
                    .insert(item_id, ticks);
                self.emit(Event::ItemCooldown { item_id, ticks });
            }
            0x18 => self.receive_custom_payload(&p).await?,
            0x19 => self.handle_sound(&p, true)?,
            0x1a => return self.receive_disconnect_packet(&p).await,
            0x1b => {
                let mut c = Cursor::new(&p);
                let entity_id = c.read_i32::<BigEndian>()?;
                let status = c.read_i8()?;
                self.common_receipts.lock().await.entities.history_signal(
                    entity_id,
                    Some(status),
                    None,
                );
                self.emit(Event::EntityStatus { entity_id, status });
            }
            0x1c => self.receive_explosion(&p).await?,
            0x1d => self.receive_unload_chunk(&p).await?,
            0x1e => self.receive_game_state_change(&p).await?,
            0x1f => self.receive_horse_window(&p, packet_sequence).await?,
            0x20 => {
                let result = self.send_protocol(0x10, &p).await;
                crate::lifecycle::emit_protocol_timing(|| {
                    serde_json::json!({"stage":if result.is_ok() {"keepalive_reply_write_completed"} else {"keepalive_reply_failed"},
                        "generation":self.connection_generation().get(),
                        "keepalive_id":p.as_slice().try_into().ok().map(i64::from_be_bytes),
                        "elapsed_ms":keepalive_started.expect("keepalive packet starts timing").elapsed().as_millis(),
                        "error":result.as_ref().err().map(|error|error.to_string().chars().take(512).collect::<String>())})
                });
                result?;
            }
            0x21 => self.receive_chunk_data(&p).await?,
            0x22 => self.emit(Event::WorldEvent(parse_world_event(&p)?)),
            0x23 => self.emit(Event::Particle(parse_particle(&p)?)),
            0x24 => self.receive_light_update(&p).await?,
            0x25 => self.receive_join_game(&p, packet_sequence).await?,
            0x26 => {
                let update = parse_map_update(&p)?;
                self.maps
                    .write()
                    .await
                    .apply(&update, self.connection_options.max_maps)?;
                self.emit(Event::MapUpdated(update));
            }
            0x27 => {
                let offers = parse_merchant_offers(&p)?;
                self.inventory.write().await.merchant_offers = Some(offers.clone());
                self.emit(Event::MerchantOffers(offers));
            }
            0x28 | 0x29 => self.receive_entity_relative_move(&p, id).await?,
            0x2a => self.receive_entity_rotation(&p).await?,
            0x2b => {
                // Original base MoveEntity contains only the entity ID, no ground flag.
                let mut rest = p.as_slice();
                let entity_id = get_varint(&mut rest)?;
                if entity_id < 0 || !rest.is_empty() {
                    bail!("invalid base entity packet");
                }
            }
            0x2c => self.receive_vehicle_position(&p).await?,
            0x2d => {
                let mut rest = p.as_slice();
                let hand = match get_varint(&mut rest)? {
                    0 => Hand::Main,
                    1 => Hand::Off,
                    value => bail!("unknown book hand {value}"),
                };
                self.emit(Event::BookOpened { hand });
            }
            0x2e => self.receive_open_window(&p, packet_sequence).await?,
            0x2f => {
                let mut cursor = Cursor::new(&p);
                self.emit(Event::SignEditorOpened {
                    position: BlockPos::unpack(cursor.read_u64::<BigEndian>()?),
                });
            }
            0x30 => self.receive_recipe_ghost(&p, packet_sequence).await?,
            0x31 => self.receive_player_abilities(&p).await?,
            0x32 => self.receive_combat_event(&p, packet_sequence).await?,
            0x33 => self.receive_player_info(&p, id, packet_sequence).await?,
            0x34 => self.receive_face_player(&p).await?,
            0x35 => self.handle_position(&p).await?,
            0x36 => self.receive_recipe_book(&p, packet_sequence).await?,
            0x37 => self.receive_destroy_entities(&p).await?,
            0x38 => self.receive_remove_effect(&p).await?,
            0x39 => {
                let mut rest = p.as_slice();
                let request = ResourcePackRequest {
                    url: get_string(&mut rest)?,
                    hash: get_string(&mut rest)?,
                };
                **self.resource_pack.write().await = Some(request.clone());
                self.emit(Event::ResourcePackRequested(request));
            }
            0x3a => self.receive_respawn(&p, packet_sequence).await?,
            0x3b => self.receive_entity_head_yaw(&p).await?,
            0x3c => {
                let mut rest = p.as_slice();
                let selected = if *rest.first().context("missing advancement tab flag")? != 0 {
                    rest = &rest[1..];
                    Some(get_string(&mut rest)?)
                } else {
                    None
                };
                self.advancements.write().await.selected_tab = selected;
                self.emit(Event::AdvancementsUpdated);
            }
            0x3d => self.receive_world_border(&p, id, packet_sequence).await?,
            0x3e => {
                let mut rest = p.as_slice();
                let entity_id = get_varint(&mut rest)?;
                **self.camera_entity_id.write().await = Some(entity_id);
                self.emit(Event::CameraChanged { entity_id });
            }
            0x3f => {
                let raw = *p.first().context("missing held item slot")?;
                if raw > 8 {
                    bail!("invalid held item slot {raw}");
                }
                self.inventory.write().await.selected_hotbar = raw;
                self.common_receipts.lock().await.selected_hotbar =
                    Some(crate::client::received(raw, packet_sequence));
                self.emit(Event::HeldItemChanged { slot: raw });
            }
            0x40 => {
                let mut rest = p.as_slice();
                let x = get_varint(&mut rest)?;
                let z = get_varint(&mut rest)?;
                let mut view = self.world_view.write().await;
                view.center_x = x;
                view.center_z = z;
                let snapshot = **view;
                drop(view);
                self.emit(Event::WorldViewUpdated(snapshot));
            }
            0x41 => {
                let mut rest = p.as_slice();
                let distance = get_varint(&mut rest)?;
                let mut view = self.world_view.write().await;
                view.distance = distance;
                let snapshot = **view;
                drop(view);
                self.emit(Event::WorldViewUpdated(snapshot));
            }
            0x42 => {
                let mut c = Cursor::new(&p);
                let position = unpack_position(c.read_u64::<BigEndian>()?);
                self.survival.write().await.spawn_position = Some(position);
                self.emit(Event::SpawnPosition(position));
            }
            0x43 => {
                self.receive_display_objective(&p, id, packet_sequence)
                    .await?
            }
            0x44 => self.receive_entity_metadata(&p, packet_sequence).await?,
            0x45 => {
                let mut cursor = Cursor::new(&p);
                let entity_id = cursor.read_i32::<BigEndian>()?;
                let raw = cursor.read_i32::<BigEndian>()?;
                let attached_to = (raw != -1).then_some(raw);
                if let Some(entity) = self.entities.write().await.entities.get_mut(&entity_id) {
                    entity.attached_to = attached_to;
                }
                self.emit(Event::EntityAttached {
                    entity_id,
                    attached_to,
                });
            }
            0x46 => self.receive_entity_velocity(&p).await?,
            0x47 => self.receive_entity_equipment(&p, packet_sequence).await?,
            0x48 => {
                let experience = parse_experience(&p)?;
                self.survival.write().await.experience = experience;
                self.emit(Event::Experience(experience));
            }
            0x49 => self.receive_vitals(&p, packet_sequence).await?,
            0x4a => {
                self.receive_scoreboard_objective(&p, id, packet_sequence)
                    .await?
            }
            0x4b => self.receive_passengers(&p, packet_sequence).await?,
            0x4c => self.receive_team(&p, packet_sequence).await?,
            0x4d => self.receive_score(&p, id, packet_sequence).await?,
            0x4e => self.receive_world_time(&p, packet_sequence).await?,
            0x4f => self.receive_title(&p, id, packet_sequence).await?,
            0x50 => self.handle_entity_sound(&p)?,
            0x51 => self.handle_sound(&p, false)?,
            0x52 => {
                let mut rest = p.as_slice();
                let flags = *rest.first().context("missing stop-sound flags")?;
                rest = &rest[1..];
                let category = if flags & 1 != 0 {
                    Some(get_varint(&mut rest)?)
                } else {
                    None
                };
                let sound_name = if flags & 2 != 0 {
                    Some(get_string(&mut rest)?)
                } else {
                    None
                };
                self.emit(Event::StopSound(StopSoundEvent {
                    category,
                    sound_name,
                }));
            }
            0x53 => self.receive_tab_list(&p, id, packet_sequence).await?,
            0x54 => {
                let mut rest = p.as_slice();
                let transaction_id = get_varint(&mut rest)?;
                self.emit(Event::NbtQueryResponse(NbtQueryResponse {
                    transaction_id,
                    nbt: Arc::from(rest.to_vec()),
                }));
            }
            0x55 => self.receive_collect_item(&p, packet_sequence).await?,
            0x56 => self.receive_entity_teleport(&p).await?,
            0x57 => {
                self.advancements.write().await.apply(&p)?;
                self.emit(Event::AdvancementsUpdated);
            }
            0x58 => self.receive_entity_attributes(&p, packet_sequence).await?,
            0x59 => self.receive_entity_effect(&p, packet_sequence).await?,
            0x5a => self.receive_declare_recipes(&p, packet_sequence).await?,
            0x5b => self.receive_tags(&p, packet_sequence).await?,
            _ => {}
        }
        self.common_dismount_context_received().await;
        self.common_mining_context_received().await?;
        self.common_placement_context_received().await?;
        self.common_inventory_context_received().await?;
        self.common_click_context_received().await?;
        self.common_crafting_context_received(false).await?;
        self.common_recipe_placement_context_received().await?;
        self.common_transfer_context_received().await?;
        self.common_container_close_context_received().await?;
        self.common_container_open_context_received().await?;
        self.enforce_session_limits().await?;
        Ok(true)
    }

    async fn capture_observation_at_sequence(
        &self,
        request: crate::CoherentObservationRequest,
        sequence: u64,
    ) -> Result<crate::CoherentObservation> {
        let _coherent_state = self.coherent_state_gate.lock().await;
        if self.connection.lifecycle() != ConnectionState::Ready {
            bail!("coherent observation capture crossed a non-ready lifecycle");
        }

        let player_guard = self.player.lock().await;
        let player = player_guard.clone();
        drop(player_guard);
        let motion = **self.motion.lock().await;
        let survival = self.survival.read().await.clone();
        let inventory_guard = self.inventory.read().await;
        let inventory_revision = inventory_guard.revision();
        let inventory = inventory_guard.clone();
        drop(inventory_guard);
        let open_window = inventory.open_window.clone();
        let furnace_window_position = *self.furnace_window_position.lock().await;
        let open_furnace = match (open_window.as_ref(), furnace_window_position) {
            (Some(window), Some((window_id, position))) if window.id == window_id => {
                inventory.windows.get(&window.id).cloned().map(|slots| {
                    let mut properties = inventory
                        .properties
                        .iter()
                        .filter(|((id, _), _)| *id == window.id)
                        .map(|((window_id, property), value)| WindowProperty {
                            window_id: *window_id,
                            property: *property,
                            value: *value,
                        })
                        .collect::<Vec<_>>();
                    properties.sort_by_key(|property| property.property);
                    crate::OpenFurnaceObservation {
                        position,
                        window: window.clone(),
                        slots,
                        properties,
                    }
                })
            }
            _ => None,
        };
        let world = self.world.lock().await;
        let block_geometry_revision = self.block_geometry_revision.load(Ordering::Acquire);
        let mining_environment = crate::mining_environment::observe(
            *self.local_pose.lock().await,
            &player,
            |x, y, z| world.block(x, y, z),
        );
        let observation_interest_cells: Vec<_> = request
            .interest
            .iter()
            .map(|position| crate::CoherentInterestCell {
                position: *position,
                state_id: world.block(position.x, position.y, position.z),
                light: world
                    .light_at(position.x, position.y, position.z)
                    .map_or(crate::CoherentLightState::Unknown, |(block, sky)| {
                        crate::CoherentLightState::Observed { block, sky }
                    }),
            })
            .collect();
        let observation_interest_unloaded = observation_interest_cells
            .iter()
            .filter(|cell| cell.state_id.is_none())
            .count()
            .try_into()
            .unwrap_or(u32::MAX);
        let observation_interest_light_unknown = observation_interest_cells
            .iter()
            .filter(|cell| matches!(cell.light, crate::CoherentLightState::Unknown))
            .count()
            .try_into()
            .unwrap_or(u32::MAX);
        drop(world);

        let mut entities = self.entities.read().await.observe(
            Vec3 {
                x: player.x,
                y: player.y,
                z: player.z,
            },
            request.entity_radius,
        );
        entities.sort_by_key(|entity| entity.entity_id);
        let entity_limit = usize::from(request.max_entities);
        let entities_omitted = entities
            .len()
            .saturating_sub(entity_limit)
            .try_into()
            .unwrap_or(u32::MAX);
        entities.truncate(entity_limit);

        let (events, events_queue_omitted, events_request_omitted) = {
            let mut queued = self
                .observation_events
                .lock()
                .expect("observation event queue poisoned");
            let event_limit = usize::from(request.max_events);
            let omitted_by_request = queued.events.len().saturating_sub(event_limit);
            let drain_len = event_limit.min(queued.events.len());
            let events = queued.events.drain(..drain_len).collect();
            queued.events.clear();
            let queue_omitted = queued.omitted;
            let request_omitted = omitted_by_request.try_into().unwrap_or(u32::MAX);
            queued.omitted = 0;
            (events, queue_omitted, request_omitted)
        };
        let events_omitted = events_queue_omitted.saturating_add(events_request_omitted);

        let world_time =
            self.world_time_observed
                .load(Ordering::Acquire)
                .then_some(crate::CoherentWorldTime {
                    world_age: survival.world_age,
                    time_of_day: survival.time_of_day,
                    daylight_cycle: Some(survival.time_of_day >= 0),
                });

        let observation = crate::CoherentObservation {
            generation: self.connection_generation(),
            sequence: crate::ObservationSequence::new(sequence),
            sensor_capture: crate::CaptureIdentity {
                generation: self.connection_generation(),
                block_geometry_revision,
                inventory_revision,
            },
            received_at: std::time::Instant::now(),
            player,
            motion,
            survival,
            oxygen_level: *self.oxygen_level.lock().await,
            mining_environment,
            world_time,
            inventory,
            open_window,
            open_furnace,
            interest: crate::CoherentObservationInterest {
                generation: request.interest_generation,
                cells: observation_interest_cells,
                unloaded_cells: observation_interest_unloaded,
                light_unknown_cells: observation_interest_light_unknown,
            },
            entities,
            entities_omitted,
            events,
            events_omitted,
            events_queue_omitted,
            events_request_omitted,
        };
        self.connection
            .record_observation(sequence)
            .await
            .map_err(|error| {
                crate::Error::new(
                    crate::ErrorKind::State,
                    anyhow::anyhow!("coherent observation publication rejected: {error}"),
                )
            })?;
        Ok(observation)
    }

    async fn capture_traversal_movement_facts_at_sequence(
        &self,
        request: crate::MovementSnapshotRequest,
        sequence: u64,
    ) -> Result<crate::MovementSnapshot> {
        let _coherent_state = self.coherent_state_gate.lock().await;
        if self.connection.lifecycle() != ConnectionState::Ready {
            bail!("movement facts capture requires a ready connection");
        }
        if request.expected_generation != self.connection_generation() {
            bail!("movement facts request belongs to a stale client generation");
        }

        let player = self.player.lock().await.clone();
        let motion = **self.motion.lock().await;
        let survival = self.survival.read().await.clone();
        let inventory = self.inventory.read().await.clone();
        let inventory_slots = inventory
            .windows
            .get(&0)
            .map(|slots| {
                slots
                    .iter()
                    .take(46)
                    .enumerate()
                    .map(|(slot, item)| crate::InventorySlotFact {
                        slot: i16::try_from(slot).expect("bounded inventory slot fits i16"),
                        item: item.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let world = self.world.lock().await;
        let mut blocks = Vec::new();
        for x in request.region.min.x..=request.region.max.x {
            for y in request.region.min.y..=request.region.max.y {
                for z in request.region.min.z..=request.region.max.z {
                    let position = crate::BlockPos { x, y, z };
                    let Some(state_id) = world.block(x, y, z) else {
                        blocks.push(crate::MovementBlock::Unloaded { position });
                        continue;
                    };
                    let Some(name) = crate::block_name_from_state(state_id) else {
                        blocks.push(crate::MovementBlock::Unknown { position, state_id });
                        continue;
                    };
                    let Some(shapes) = crate::block_collision_shapes(state_id) else {
                        blocks.push(crate::MovementBlock::Unknown { position, state_id });
                        continue;
                    };
                    let Some(registry) = crate::block_movement_registry_facts(state_id) else {
                        blocks.push(crate::MovementBlock::Unknown { position, state_id });
                        continue;
                    };
                    let Some(properties) = crate::block_state_properties(state_id) else {
                        blocks.push(crate::MovementBlock::Unknown { position, state_id });
                        continue;
                    };
                    blocks.push(crate::MovementBlock::Loaded {
                        position,
                        state_id,
                        name: name.to_owned(),
                        shapes,
                        registry,
                        properties,
                    });
                }
            }
        }
        drop(world);

        let mut entities = self.entities.read().await.observe(
            Vec3 {
                x: player.x,
                y: player.y,
                z: player.z,
            },
            f64::from(request.entity_radius),
        );
        entities.sort_by_key(|entity| entity.entity_id);
        let entities_omitted = entities
            .len()
            .saturating_sub(usize::from(request.max_entities))
            .try_into()
            .unwrap_or(u32::MAX);
        entities.truncate(usize::from(request.max_entities));
        let entities = entities
            .into_iter()
            .map(|entity| crate::ObservedEntity {
                dimensions: entity
                    .type_id
                    .and_then(crate::entity_dimensions)
                    .map(|(width, height)| crate::EntityDimensions { width, height }),
                entity,
            })
            .collect();

        let generation = self.connection_generation();
        if request.expected_generation != generation {
            bail!("movement facts capture crossed a client generation boundary");
        }
        if self.connection.lifecycle() != ConnectionState::Ready {
            bail!("movement facts capture crossed a terminal lifecycle boundary");
        }
        let sequence = crate::ObservationSequence::new(sequence);
        self.connection
            .record_observation(sequence.get())
            .await
            .map_err(|error| {
                crate::Error::new(
                    crate::ErrorKind::State,
                    anyhow::anyhow!("movement facts publication rejected: {error}"),
                )
            })?;

        Ok(crate::MovementSnapshot {
            generation,
            sequence,
            player,
            motion,
            survival,
            blocks,
            entities,
            entities_omitted,
            inventory: crate::InventoryFact {
                selected_hotbar: inventory.selected_hotbar,
                slots: inventory_slots,
            },
        })
    }

    async fn insert_entity(&self, entity: EntityState) -> Result<()> {
        validate_position(entity.position.x, entity.position.y, entity.position.z)?;
        if !entity.velocity.x.is_finite()
            || !entity.velocity.y.is_finite()
            || !entity.velocity.z.is_finite()
            || !entity.yaw.is_finite()
            || !entity.pitch.is_finite()
            || !entity.head_yaw.is_finite()
        {
            bail!("entity contains a non-finite motion or rotation");
        }
        let mut entities = self.entities.write().await;
        if !entities.entities.contains_key(&entity.entity_id)
            && entities.entities.len() >= self.connection_options.max_entities
        {
            bail!("entity cache limit exceeded");
        }
        {
            let mut receipts = self.common_receipts.lock().await;
            receipts.entities.insert(
                crate::MinecraftVersion::Java1_16_1,
                crate::client::entity::NativeSpawn {
                    id: entity.entity_id,
                    uuid: entity.uuid,
                    type_id: entity.type_id,
                    dedicated_type_name: entity.type_name,
                    position: [entity.position.x, entity.position.y, entity.position.z],
                    living: Some(matches!(
                        entity.kind,
                        crate::versions::java_1_16_1::entity::EntityKind::Living
                            | crate::versions::java_1_16_1::entity::EntityKind::Player
                    )),
                },
                self.protocol_packet_sequence.load(Ordering::Acquire),
                self.connection_options.max_entities,
            )?;
            use crate::client::entity::NativeSpawnMotion;
            use crate::versions::java_1_16_1::entity::EntityKind;
            receipts.entities.initialize_motion(
                entity.entity_id,
                NativeSpawnMotion {
                    position: (entity.kind != EntityKind::Painting).then_some([
                        entity.position.x,
                        entity.position.y,
                        entity.position.z,
                    ]),
                    rotation: matches!(
                        entity.kind,
                        EntityKind::Object | EntityKind::Living | EntityKind::Player
                    )
                    .then_some([entity.yaw, entity.pitch]),
                    head_yaw: (entity.kind == EntityKind::Living).then_some(entity.head_yaw),
                    velocity: matches!(entity.kind, EntityKind::Object | EntityKind::Living)
                        .then_some([entity.velocity.x, entity.velocity.y, entity.velocity.z]),
                },
                self.protocol_packet_sequence.load(Ordering::Acquire),
            );
            receipts.vehicles.retire(entity.entity_id);
        }
        entities.entities.insert(entity.entity_id, entity.clone());
        drop(entities);
        self.emit(Event::EntitySpawned(entity));
        Ok(())
    }

    async fn enforce_session_limits(&self) -> Result<()> {
        let limit = self.connection_options.max_cached_records;
        let players = read_session_cache(&self.players).await.entries.len();
        let maps = read_session_cache(&self.maps).await.maps.len();
        let entities = read_session_cache(&self.entities).await.entities.len();
        let inventory = read_session_cache(&self.inventory).await;
        let inventory_records = inventory.windows.values().map(Vec::len).sum::<usize>()
            + inventory.properties.len()
            + inventory.pending_clicks.len();
        drop(inventory);
        let ui = read_session_cache(&self.ui).await;
        let ui_records = ui.boss_bars.len()
            + ui.objectives.len()
            + ui.display_objectives.len()
            + ui.scores.len()
            + ui.teams.len();
        drop(ui);
        let progress = read_session_cache(&self.advancements).await;
        let progress_records = progress.definitions.len() + progress.progress.len();
        drop(progress);
        let statistics = read_session_cache(&self.statistics).await.values.len();
        let recipe_book = read_session_cache(&self.recipe_book).await;
        let recipe_records = recipe_book.unlocked.len() + recipe_book.displayed.len();
        let records = players
            .saturating_add(maps)
            .saturating_add(entities)
            .saturating_add(inventory_records)
            .saturating_add(ui_records)
            .saturating_add(progress_records)
            .saturating_add(statistics)
            .saturating_add(recipe_records);
        if records > limit {
            bail!("session cache contains {records} records, limit is {limit}");
        }
        Ok(())
    }
    async fn handle_position(&self, p: &[u8]) -> Result<()> {
        let mut s = self.player.lock().await;
        let (position, rotation, teleport) =
            common_recording::decode_position(p, [s.x, s.y, s.z], [s.yaw, s.pitch])?;
        let [next_x, next_y, next_z] = position;
        let [next_yaw, next_pitch] = rotation;
        s.x = next_x;
        s.y = next_y;
        s.z = next_z;
        s.yaw = next_yaw;
        s.pitch = next_pitch;
        {
            let mut receipts = self.common_receipts.lock().await;
            receipts.ground_source = None;
            receipts.rotation_source = Some(crate::client::ValueSource::Received {
                sequence: self.protocol_packet_sequence.load(Ordering::Acquire),
            });
        }
        let snapshot = s.clone();
        drop(s);
        self.common_receipts.lock().await.position_source =
            Some(crate::client::ValueSource::Received {
                sequence: self.protocol_packet_sequence.load(Ordering::Acquire),
            });
        self.interrupt_common_motion_operations(
            "native own-pose correction interrupted finite motion",
        )
        .await;
        self.common_receipts.lock().await.pose = Some(crate::client::ReceivedPose {
            position: [next_x, next_y, next_z],
            rotation: [next_yaw, next_pitch],
            receive_sequence: self.protocol_packet_sequence.load(Ordering::Acquire),
        });
        {
            let mut receipts = self.common_receipts.lock().await;
            if let Some(pose) = receipts.pose.clone() {
                receipts.entities.history_pose(pose, None);
            }
        }
        let mut positioned = self.positioned.lock().await;
        let was_positioned = *positioned;
        *positioned = true;
        drop(positioned);
        if was_positioned {
            // A server teleport can move the player into chunks whose block data is still in
            // flight. Keep confirming server corrections, but briefly stop the 20 Hz movement
            // producer so stale/local-physics positions cannot race the new authoritative one.
            self.teleport_barrier_ticks.fetch_max(20, Ordering::AcqRel);
        }
        if was_positioned {
            if let Some(correction) = self.physics.lock().await.record_server_position(&snapshot) {
                **self.motion.lock().await = MotionState::default();
                self.emit(Event::PositionCorrection(correction));
            }
        } else if !was_positioned {
            **self.motion.lock().await = MotionState::default();
        }
        self.ready.notify_waiters();
        let mut payload = Vec::new();
        put_varint(&mut payload, teleport);
        self.send_protocol(0x00, &payload).await?;
        self.send_position_protocol().await?;
        self.emit(Event::Position(snapshot));
        Ok(())
    }
    fn handle_sound(&self, p: &[u8], named: bool) -> Result<()> {
        let mut rest = p;
        let (sound_id, sound_name) = if named {
            (None, Some(get_string(&mut rest)?))
        } else {
            let id = get_varint(&mut rest)?;
            (
                Some(id),
                crate::versions::java_1_16_1::registry::sound_name(id).map(str::to_owned),
            )
        };
        let category = get_varint(&mut rest)?;
        let mut c = Cursor::new(rest);
        let x = c.read_i32::<BigEndian>()? as f64 / 8.0;
        let y = c.read_i32::<BigEndian>()? as f64 / 8.0;
        let z = c.read_i32::<BigEndian>()? as f64 / 8.0;
        let volume = c.read_f32::<BigEndian>()?;
        let pitch = c.read_f32::<BigEndian>()?;
        self.emit(Event::Sound(SoundEvent {
            sound_id,
            sound_name,
            category,
            category_name: sound_category_name(category),
            source: SoundSource::Position { x, y, z },
            volume,
            pitch,
            sequence: self.sound_sequence.fetch_add(1, Ordering::Relaxed),
            received_at: self.connected_at.elapsed(),
        }));
        Ok(())
    }
    fn handle_entity_sound(&self, p: &[u8]) -> Result<()> {
        let mut rest = p;
        let sound_id = get_varint(&mut rest)?;
        let category = get_varint(&mut rest)?;
        let entity_id = get_varint(&mut rest)?;
        let mut c = Cursor::new(rest);
        let volume = c.read_f32::<BigEndian>()?;
        let pitch = c.read_f32::<BigEndian>()?;
        self.emit(Event::Sound(SoundEvent {
            sound_id: Some(sound_id),
            sound_name: crate::versions::java_1_16_1::registry::sound_name(sound_id)
                .map(str::to_owned),
            category,
            category_name: sound_category_name(category),
            source: SoundSource::Entity { entity_id },
            volume,
            pitch,
            sequence: self.sound_sequence.fetch_add(1, Ordering::Relaxed),
            received_at: self.connected_at.elapsed(),
        }));
        Ok(())
    }
    async fn wait_for_player_chunk(&self, p: &Player) -> Result<()> {
        loop {
            let notified = self.world_updated.notified();
            if self
                .world
                .lock()
                .await
                .has_chunk_at(p.x.floor() as i32, p.z.floor() as i32)
            {
                return Ok(());
            }
            timeout(Duration::from_secs(15), notified)
                .await
                .context("timed out waiting for player chunk")?;
        }
    }
    async fn send_position(&self) -> Result<()> {
        let p = self.player().await;
        let mut b = Vec::new();
        b.write_f64::<BigEndian>(p.x)?;
        b.write_f64::<BigEndian>(p.y)?;
        b.write_f64::<BigEndian>(p.z)?;
        b.write_f32::<BigEndian>(p.yaw)?;
        b.write_f32::<BigEndian>(p.pitch)?;
        b.push(p.on_ground as u8);
        self.send(0x13, &b).await?;
        self.physics.lock().await.record_movement(p);
        Ok(())
    }
    async fn send_position_protocol(&self) -> Result<()> {
        let p = self.player().await;
        let mut payload = Vec::new();
        payload.write_f64::<BigEndian>(p.x)?;
        payload.write_f64::<BigEndian>(p.y)?;
        payload.write_f64::<BigEndian>(p.z)?;
        payload.write_f32::<BigEndian>(p.yaw)?;
        payload.write_f32::<BigEndian>(p.pitch)?;
        payload.push(p.on_ground as u8);
        self.send_protocol(0x13, &payload).await?;
        self.physics.lock().await.record_movement(p);
        Ok(())
    }
    async fn send(&self, id: i32, p: &[u8]) -> Result<()> {
        self.connection
            .dispatch(self.operation_context(0), OperationClass::Normal, id, p)
            .await
    }
    async fn send_protocol(&self, id: i32, p: &[u8]) -> Result<()> {
        self.connection.dispatch_protocol(id, p).await
    }
    async fn send_entity_action(&self, action: i32) -> Result<()> {
        let entity_id = self
            .player
            .lock()
            .await
            .entity_id
            .context("Join Game entity ID is unavailable")?;
        let mut payload = Vec::new();
        put_varint(&mut payload, entity_id);
        put_varint(&mut payload, action);
        put_varint(&mut payload, 0);
        self.send(0x1c, &payload).await
    }
    fn emit(&self, event: Event) {
        let terminal = matches!(
            event,
            Event::Disconnected { .. }
                | Event::Error {
                    kind: "connection",
                    ..
                }
        );
        if terminal {
            if self.terminal_emitted.swap(true, Ordering::AcqRel) {
                return;
            }
            self.stopped.store(true, Ordering::Release);
            self.cancel.notify_waiters();
            self.ready.notify_waiters();
            self.world_updated.notify_waiters();
            self.packet_applied.notify_waiters();
        }
        {
            let sequence = self.protocol_packet_sequence.load(Ordering::Acquire);
            let mut ledger = self.common_events.lock().expect("event ledger poisoned");
            for kind in common_event_kinds(&event) {
                ledger.record(sequence, kind);
            }
        }
        {
            let mut queued = self
                .observation_events
                .lock()
                .expect("observation event queue poisoned");
            if queued.events.len() == 256 {
                queued.events.pop_front();
                queued.omitted = queued.omitted.saturating_add(1);
            }
            queued.events.push_back(event.clone());
        }
        let _ = self.events.send(event);
    }
}
/// Common change notifications for one native event. Multi-block changes are
/// recorded with their bounds where the packet is applied.
/// One decoded play frame: packet ID and payload.
type Frame = Result<(i32, Vec<u8>)>;

/// Complete an in-flight frame read after cancellation, waiting at most 200 ms.
async fn finish_frame<F, E>(read: std::pin::Pin<&mut F>) -> Option<Frame>
where
    F: std::future::Future<Output = std::result::Result<anyhow::Result<(i32, Vec<u8>)>, E>>,
{
    match tokio::time::timeout(Duration::from_millis(200), read).await {
        Ok(Ok(frame)) => Some(frame.map_err(Into::into)),
        _ => None,
    }
}

fn common_event_kinds(event: &Event) -> Vec<crate::client::EventKind> {
    use crate::client::EventKind as K;
    let block = |x, y, z| K::BlocksChanged {
        min: [x, y, z],
        max: [x, y, z],
    };
    match event {
        Event::BlockChanged { x, y, z, .. } => vec![block(*x, *y, *z)],
        Event::BlockEntityUpdated(data) => {
            vec![block(data.position.x, data.position.y, data.position.z)]
        }
        Event::ChunkLoaded { x, z } => vec![K::ChunkLoaded { x: *x, z: *z }],
        Event::ChunkUnloaded { x, z } => vec![K::ChunkUnloaded { x: *x, z: *z }],
        Event::InventoryUpdated { .. } | Event::SlotUpdated(_) | Event::HeldItemChanged { .. } => {
            vec![K::InventoryChanged]
        }
        Event::WindowOpened(_)
        | Event::WindowClosed { .. }
        | Event::WindowProperty(_)
        | Event::MerchantOffers(_) => vec![K::ScreenChanged],
        Event::Position(_)
        | Event::PositionCorrection(_)
        | Event::Vitals(_)
        | Event::Experience(_)
        | Event::GameStateChange(_)
        | Event::SurvivalStateUpdated => vec![K::PlayerChanged],
        Event::Login | Event::Spawn | Event::Respawn(_) => vec![K::WorldChanged],
        Event::EntitySpawned(entity) => vec![K::EntitySpawned {
            native_id: entity.entity_id,
        }],
        Event::EntitiesDestroyed { entity_ids } => entity_ids
            .iter()
            .map(|id| K::EntityRemoved { native_id: *id })
            .collect(),
        Event::Chat(_) => vec![K::ChatReceived],
        Event::UiStateUpdated(_) | Event::PlayerListUpdated { .. } => vec![K::UiChanged],
        Event::Disconnected { .. }
        | Event::Error {
            kind: "connection", ..
        } => vec![K::Disconnected],
        Event::EntityUpdated(entity) => vec![K::EntityUpdated {
            native_id: entity.entity_id,
        }],
        Event::EntityStatus { entity_id, status } => {
            let mut kinds = vec![K::EntityStatus {
                native_id: *entity_id,
                status: *status,
            }];
            if crate::client::events::LEGACY_HURT_STATUSES.contains(status) {
                kinds.push(K::EntityDamaged {
                    native_id: *entity_id,
                });
            }
            kinds
        }
        Event::Combat(crate::versions::java_1_16_1::CombatEvent::Death { player_id, .. }) => {
            vec![K::PlayerKilled {
                native_id: *player_id,
            }]
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod common_event_kind_tests {
    use super::*;
    use crate::client::EventKind as K;

    #[test]
    fn hurt_statuses_also_report_damage_and_own_death_reports_a_kill() {
        assert_eq!(
            common_event_kinds(&Event::EntityStatus {
                entity_id: 4,
                status: 2
            }),
            vec![
                K::EntityStatus {
                    native_id: 4,
                    status: 2
                },
                K::EntityDamaged { native_id: 4 }
            ]
        );
        assert_eq!(
            common_event_kinds(&Event::EntityStatus {
                entity_id: 4,
                status: 9
            }),
            vec![K::EntityStatus {
                native_id: 4,
                status: 9
            }]
        );
        assert_eq!(
            common_event_kinds(&Event::Combat(
                crate::versions::java_1_16_1::CombatEvent::Death {
                    player_id: 7,
                    entity_id: -1,
                    message_json: "{}".into()
                }
            )),
            vec![K::PlayerKilled { native_id: 7 }]
        );
    }
}

fn parse_tab_completion(payload: &[u8]) -> Result<TabCompletion> {
    let mut rest = payload;
    let transaction_id = get_varint(&mut rest)?;
    let start = get_varint(&mut rest)?;
    let length = get_varint(&mut rest)?;
    let count = get_varint(&mut rest)?;
    if !(0..=4096).contains(&count) {
        bail!("invalid completion match count {count}");
    }
    let mut matches = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let value = get_string(&mut rest)?;
        let has_tooltip = *rest.first().context("missing completion tooltip flag")? != 0;
        rest = &rest[1..];
        let tooltip_json = if has_tooltip {
            Some(get_string(&mut rest)?)
        } else {
            None
        };
        matches.push(TabCompletionMatch {
            value,
            tooltip_json,
        });
    }
    Ok(TabCompletion {
        transaction_id,
        start,
        length,
        matches,
    })
}
async fn wait_for_nbt(
    events: &mut broadcast::Receiver<Event>,
    transaction_id: i32,
    wait: Duration,
) -> Result<NbtQueryResponse> {
    Ok(timeout(wait, async {
        loop {
            match events.recv().await {
                Ok(Event::NbtQueryResponse(response))
                    if response.transaction_id == transaction_id =>
                {
                    return Ok::<_, anyhow::Error>(response);
                }
                Ok(Event::Disconnected { reason }) => bail!("disconnected: {reason}"),
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => bail!("event stream closed"),
            }
        }
    })
    .await
    .context("timed out waiting for NBT query response")??)
}
fn parse_world_event(payload: &[u8]) -> Result<WorldEvent> {
    let mut cursor = Cursor::new(payload);
    Ok(WorldEvent {
        effect_id: cursor.read_i32::<BigEndian>()?,
        position: BlockPos::unpack(cursor.read_u64::<BigEndian>()?),
        data: cursor.read_i32::<BigEndian>()?,
        global: cursor.read_u8()? != 0,
    })
}
fn parse_particle(payload: &[u8]) -> Result<WorldParticleEvent> {
    let mut cursor = Cursor::new(payload);
    let particle_id = cursor.read_i32::<BigEndian>()?;
    let long_distance = cursor.read_u8()? != 0;
    let position = Vec3 {
        x: cursor.read_f64::<BigEndian>()?,
        y: cursor.read_f64::<BigEndian>()?,
        z: cursor.read_f64::<BigEndian>()?,
    };
    let offset = [
        cursor.read_f32::<BigEndian>()?,
        cursor.read_f32::<BigEndian>()?,
        cursor.read_f32::<BigEndian>()?,
    ];
    let speed = cursor.read_f32::<BigEndian>()?;
    let count = cursor.read_i32::<BigEndian>()?;
    validate_position(position.x, position.y, position.z)?;
    if offset.iter().any(|value| !value.is_finite()) || !speed.is_finite() {
        bail!("particle contains a non-finite offset or speed");
    }
    let raw_data = payload[cursor.position() as usize..].to_vec();
    let mut rest = raw_data.as_slice();
    let data = match particle_id {
        3 | 23 => crate::versions::java_1_16_1::ParticleData::BlockState(get_varint(&mut rest)?),
        14 => crate::versions::java_1_16_1::ParticleData::Dust {
            red: read_f32_slice(&mut rest)?,
            green: read_f32_slice(&mut rest)?,
            blue: read_f32_slice(&mut rest)?,
            scale: read_f32_slice(&mut rest)?,
        },
        34 => crate::versions::java_1_16_1::ParticleData::Item(read_slot(&mut rest)?),
        _ => crate::versions::java_1_16_1::ParticleData::None,
    };
    Ok(WorldParticleEvent {
        particle_id,
        long_distance,
        position,
        offset,
        speed,
        count,
        data,
        raw_data,
    })
}
fn parse_explosion(payload: &[u8]) -> Result<ExplosionEvent> {
    let mut cursor = Cursor::new(payload);
    let position = Vec3 {
        x: f64::from(cursor.read_f32::<BigEndian>()?),
        y: f64::from(cursor.read_f32::<BigEndian>()?),
        z: f64::from(cursor.read_f32::<BigEndian>()?),
    };
    let radius = cursor.read_f32::<BigEndian>()?;
    validate_position(position.x, position.y, position.z)?;
    if !radius.is_finite() || !(0.0..=1024.0).contains(&radius) {
        bail!("invalid explosion radius {radius}");
    }
    let count = cursor.read_i32::<BigEndian>()?;
    if !(0..=1_000_000).contains(&count) {
        bail!("invalid explosion affected-block count {count}");
    }
    let remaining = payload.len().saturating_sub(cursor.position() as usize);
    let required = (count as usize)
        .checked_mul(3)
        .and_then(|bytes| bytes.checked_add(12))
        .context("explosion affected-block length overflow")?;
    if required > remaining {
        bail!("explosion affected-block count exceeds packet length");
    }
    let origin = BlockPos {
        x: position.x.floor() as i32,
        y: position.y.floor() as i32,
        z: position.z.floor() as i32,
    };
    let mut affected_blocks = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let offset_x = i32::from(cursor.read_i8()?);
        let offset_y = i32::from(cursor.read_i8()?);
        let offset_z = i32::from(cursor.read_i8()?);
        affected_blocks.push(BlockPos {
            x: origin
                .x
                .checked_add(offset_x)
                .context("explosion x overflow")?,
            y: origin
                .y
                .checked_add(offset_y)
                .context("explosion y overflow")?,
            z: origin
                .z
                .checked_add(offset_z)
                .context("explosion z overflow")?,
        });
    }
    let player_motion = Vec3 {
        x: f64::from(cursor.read_f32::<BigEndian>()?),
        y: f64::from(cursor.read_f32::<BigEndian>()?),
        z: f64::from(cursor.read_f32::<BigEndian>()?),
    };
    if !player_motion.x.is_finite() || !player_motion.y.is_finite() || !player_motion.z.is_finite()
    {
        bail!("explosion contains non-finite player motion");
    }
    Ok(ExplosionEvent {
        position,
        radius,
        affected_blocks,
        player_motion,
    })
}

fn require_disconnect_control_cleanup(outcome: CleanupDispatchOutcome) -> Result<()> {
    if outcome == CleanupDispatchOutcome::AppliedLocally {
        Ok(())
    } else {
        bail!("disconnect control cleanup did not commit locally: {outcome:?}")
    }
}

fn validate_position(x: f64, y: f64, z: f64) -> Result<()> {
    if !x.is_finite() || !y.is_finite() || !z.is_finite() {
        bail!("position contains a non-finite coordinate");
    }
    if x.abs() > 30_000_000.0 || z.abs() > 30_000_000.0 || y.abs() > 2048.0 {
        bail!("position is outside the supported Minecraft coordinate range");
    }
    Ok(())
}
fn read_f32_slice(rest: &mut &[u8]) -> Result<f32> {
    let mut cursor = Cursor::new(*rest);
    let value = cursor.read_f32::<BigEndian>()?;
    *rest = &rest[cursor.position() as usize..];
    Ok(value)
}
fn sound_category_name(id: i32) -> Option<&'static str> {
    [
        "master", "music", "record", "weather", "block", "hostile", "neutral", "player", "ambient",
        "voice",
    ]
    .get(id as usize)
    .copied()
}

async fn next_operation_event(events: &mut broadcast::Receiver<Event>) -> Result<Event> {
    loop {
        match events.recv().await {
            Ok(Event::Disconnected { reason }) => bail!("disconnected: {reason}"),
            Ok(Event::Error {
                kind: "connection",
                message,
            }) => bail!("connection error: {message}"),
            Ok(event) => return Ok(event),
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => {
                bail!("connection closed while waiting for operation response")
            }
        }
    }
}

impl crate::client::adapter::EventOps for Bot {
    async fn entity_history_after(
        &self,
        cursor: Option<crate::client::EntityHistoryCursor>,
        maximum: usize,
    ) -> Result<crate::client::EntityHistory> {
        let _gate = self.coherent_state_gate.lock().await;
        let receipts = self.common_receipts.lock().await;
        receipts.entities.history_after(
            crate::client::SessionStamp {
                version: crate::MinecraftVersion::Java1_16_1,
                connection_id: crate::client::adapter::SessionOps::connection_id(self),
                world_generation: receipts.generation,
            },
            self.protocol_packet_sequence.load(Ordering::Acquire),
            cursor,
            maximum,
        )
    }

    async fn death_message(
        &self,
    ) -> Result<Option<crate::client::ObservedValue<crate::client::ui::UiText>>> {
        Ok(self.common_receipts.lock().await.death_message.clone())
    }
    async fn disconnect_reason(&self) -> Result<Option<crate::client::ui::UiText>> {
        Ok(self.common_receipts.lock().await.disconnect_reason.clone())
    }
    async fn events_after(&self, cursor: u64) -> Result<crate::client::EventLog> {
        // Packet application holds this gate, so the log and sequence agree.
        let _gate = self.coherent_state_gate.lock().await;
        let sequence = self.protocol_packet_sequence.load(Ordering::Acquire);
        self.common_events
            .lock()
            .expect("event ledger poisoned")
            .after(cursor, sequence)
    }
}

impl crate::client::adapter::WaitOps for Bot {
    async fn wait_for_receive(&self, after: u64) -> Result<u64> {
        loop {
            let notified = self.packet_applied.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_stopped() {
                return Err(crate::Error::new(
                    crate::ErrorKind::Disconnected,
                    anyhow::anyhow!("connection closed while waiting for a packet"),
                ));
            }
            let current = self.protocol_packet_sequence.load(Ordering::Acquire);
            if current > after {
                return Ok(current);
            }
            notified.await;
        }
    }
}

impl crate::client::adapter::ChatOps for Bot {
    async fn send_chat(&self, message: &str) -> Result<()> {
        Bot::send_chat(self, message).await
    }
    async fn send_command(&self, command: &str) -> Result<()> {
        Bot::send_command(self, command).await
    }
    async fn chat_after(&self, cursor: u64) -> Result<crate::client::ChatLog> {
        let _gate = self.coherent_state_gate.lock().await;
        if self.is_stopped() {
            return Err(crate::client::inventory::unavailable("connection closed"));
        }
        let player = self.common_player_unlocked().await?;
        self.common_chat
            .lock()
            .await
            .after(cursor, player.session, player.receive_sequence)
    }
}

impl crate::client::adapter::UiOps for Bot {
    async fn scoreboard_state(&self) -> Result<crate::client::ui::ScoreboardObservation> {
        let _gate = self.coherent_state_gate.lock().await;
        if self.is_stopped() {
            return Err(crate::client::inventory::unavailable("connection closed"));
        }
        let player = self.common_player_unlocked().await?;
        Ok(self
            .common_scoreboard
            .lock()
            .await
            .capture(player.session, player.receive_sequence))
    }
    async fn boss_bars(&self) -> Result<crate::client::ui::BossBarsObservation> {
        let _gate = self.coherent_state_gate.lock().await;
        if self.is_stopped() {
            return Err(crate::client::inventory::unavailable("connection closed"));
        }
        let player = self.common_player_unlocked().await?;
        Ok(self
            .common_boss_bars
            .lock()
            .await
            .capture(player.session, player.receive_sequence))
    }
    async fn teams(&self) -> Result<crate::client::ui::TeamsObservation> {
        let _gate = self.coherent_state_gate.lock().await;
        if self.is_stopped() {
            return Err(crate::client::inventory::unavailable("connection closed"));
        }
        let player = self.common_player_unlocked().await?;
        Ok(self
            .common_teams
            .lock()
            .await
            .capture(player.session, player.receive_sequence))
    }
    async fn player_list(&self) -> Result<crate::client::ui::PlayerListObservation> {
        let _gate = self.coherent_state_gate.lock().await;
        if self.is_stopped() {
            return Err(crate::client::inventory::unavailable("connection closed"));
        }
        let player = self.common_player_unlocked().await?;
        Ok(self
            .common_player_list
            .lock()
            .await
            .capture(player.session, player.receive_sequence))
    }
    async fn titles(&self) -> Result<crate::client::ui::TitlesObservation> {
        let _gate = self.coherent_state_gate.lock().await;
        if self.is_stopped() {
            return Err(crate::client::inventory::unavailable("connection closed"));
        }
        let player = self.common_player_unlocked().await?;
        Ok(self
            .common_display
            .lock()
            .await
            .titles(player.session, player.receive_sequence))
    }
    async fn tab_list(&self) -> Result<crate::client::ui::TabListObservation> {
        let _gate = self.coherent_state_gate.lock().await;
        if self.is_stopped() {
            return Err(crate::client::inventory::unavailable("connection closed"));
        }
        let player = self.common_player_unlocked().await?;
        Ok(self
            .common_display
            .lock()
            .await
            .tab_list(player.session, player.receive_sequence))
    }
    async fn world_border(&self) -> Result<crate::client::ui::WorldBorderObservation> {
        let _gate = self.coherent_state_gate.lock().await;
        if self.is_stopped() {
            return Err(crate::client::inventory::unavailable("connection closed"));
        }
        let player = self.common_player_unlocked().await?;
        Ok(self
            .common_display
            .lock()
            .await
            .world_border(player.session, player.receive_sequence))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BlockCollision, CoherentWorldTime, GeometryBlock, GeometryQuery, SlotPrediction,
        WindowPrediction,
    };

    fn repeated_pickup_fixture(window_id: i8) -> (InventoryState, PendingClick) {
        let expected = |name: &str, count| SlotExpectation {
            item_name: name.to_owned(),
            count,
            metadata: 0,
        };
        let stack = |name, count| ItemStack {
            item_id: crate::registry::item_id(name).unwrap(),
            count,
            nbt: None,
        };
        let output = expected("stick", 4);
        let mut inventory = InventoryState::default();
        let mut slots = vec![None; 46];
        slots[0] = Some(stack("stick", 4));
        slots[1] = Some(stack("oak_planks", 2));
        slots[3] = Some(stack("oak_planks", 2));
        inventory.windows.insert(window_id, slots);
        let pending = PendingClick {
            window_id,
            action: 1,
            slot: 0,
            button: 0,
            mode: ClickMode::Normal,
            slot_before: Some(stack("stick", 4)),
            cursor_before: None,
            prediction: WindowPrediction {
                slots: vec![
                    SlotPrediction {
                        slot: 0,
                        before: Some(output.clone()),
                        after: Some(output.clone()),
                    },
                    SlotPrediction {
                        slot: 1,
                        before: Some(expected("oak_planks", 2)),
                        after: Some(expected("oak_planks", 1)),
                    },
                    SlotPrediction {
                        slot: 3,
                        before: Some(expected("oak_planks", 2)),
                        after: Some(expected("oak_planks", 1)),
                    },
                ],
                cursor_before: None,
                cursor_after: Some(output),
            },
        };
        (inventory, pending)
    }

    fn assert_repeated_pickup_can_continue(window_id: i8) {
        let (mut inventory, pending) = repeated_pickup_fixture(window_id);
        apply_accepted_normal_click(&mut inventory, &pending).unwrap();
        assert_eq!(inventory.windows[&window_id][0], pending.slot_before);
        assert_eq!(inventory.cursor, pending.slot_before);
        for slot in [1, 3] {
            assert_eq!(
                inventory.windows[&window_id][slot].as_ref().unwrap().count,
                1
            );
        }
        let output = pending.prediction.cursor_after.clone();
        let deposit = PendingClick {
            window_id,
            action: 2,
            slot: 9,
            button: 0,
            mode: ClickMode::Normal,
            slot_before: None,
            cursor_before: inventory.cursor.clone(),
            prediction: WindowPrediction {
                slots: vec![SlotPrediction {
                    slot: 9,
                    before: None,
                    after: output.clone(),
                }],
                cursor_before: output.clone(),
                cursor_after: None,
            },
        };
        apply_accepted_normal_click(&mut inventory, &deposit).unwrap();
        let next = WindowClick {
            slot: 0,
            button: 0,
            mode: ClickMode::Normal,
            expected_item: output,
            expected_cursor: None,
            prediction: Default::default(),
        };
        assert!(
            click_precondition_matches(&inventory, window_id, &next),
            "confirmed deposit must release the next output pickup barrier"
        );
    }

    #[test]
    fn confirmed_inventory_output_replenishes_for_next_pickup() {
        assert_repeated_pickup_can_continue(0);
    }

    #[test]
    fn confirmed_table_output_replenishes_for_next_pickup() {
        assert_repeated_pickup_can_continue(1);
    }

    #[test]
    fn confirmed_repeating_pickup_preserves_early_empty_server_output() {
        let (mut inventory, pending) = repeated_pickup_fixture(0);
        inventory.windows.get_mut(&0).unwrap()[0] = None;
        apply_accepted_normal_click(&mut inventory, &pending).unwrap();
        assert!(inventory.windows[&0][0].is_none());
        assert_eq!(inventory.cursor, pending.slot_before);
    }

    #[test]
    fn confirmed_repeating_pickup_preserves_early_server_cursor() {
        let (mut inventory, pending) = repeated_pickup_fixture(0);
        let server_cursor = ItemStack {
            item_id: crate::registry::item_id("oak_planks").unwrap(),
            count: 1,
            nbt: None,
        };
        inventory.cursor = Some(server_cursor.clone());
        apply_accepted_normal_click(&mut inventory, &pending).unwrap();
        assert_eq!(inventory.cursor, Some(server_cursor));
    }

    #[test]
    fn confirmed_body_effect_retains_matching_predicted_stack_nbt() {
        let mut inventory = InventoryState::default();
        let source = ItemStack {
            item_id: crate::registry::item_id("oak_log").unwrap(),
            count: 3,
            nbt: Some(vec![10, 0, 0, 0]),
        };
        let expected = SlotExpectation {
            item_name: "oak_log".to_owned(),
            count: 3,
            metadata: 0,
        };
        let mut slots = vec![None; 46];
        slots[9] = Some(source.clone());
        inventory.windows.insert(0, slots);
        let pending = PendingClick {
            window_id: 0,
            action: 1,
            slot: 9,
            button: 0,
            mode: ClickMode::Normal,
            slot_before: Some(source.clone()),
            cursor_before: None,
            prediction: WindowPrediction {
                slots: vec![SlotPrediction {
                    slot: 9,
                    before: Some(expected.clone()),
                    after: None,
                }],
                cursor_before: None,
                cursor_after: Some(expected),
            },
        };
        apply_accepted_normal_click(&mut inventory, &pending).unwrap();
        assert!(inventory.windows[&0][9].is_none());
        assert_eq!(inventory.cursor, Some(source));
    }

    #[test]
    fn disconnect_accepts_only_actor_applied_control_cleanup() {
        assert!(require_disconnect_control_cleanup(CleanupDispatchOutcome::AppliedLocally).is_ok());
        for non_local in [
            CleanupDispatchOutcome::Dispatched,
            CleanupDispatchOutcome::Acknowledged,
            CleanupDispatchOutcome::Rejected,
            CleanupDispatchOutcome::DeliveryUnknown,
        ] {
            assert!(require_disconnect_control_cleanup(non_local).is_err());
        }
    }

    #[test]
    fn accepted_normal_click_preserves_early_server_slot_fact_and_applies_cursor_effect() {
        let output = ItemStack {
            item_id: 15,
            count: 4,
            nbt: None,
        };
        let mut inventory = InventoryState::default();
        inventory.windows.insert(0, vec![None]);
        let pending = PendingClick {
            window_id: 0,
            action: 3,
            slot: 0,
            button: 0,
            mode: ClickMode::Normal,
            slot_before: Some(output.clone()),
            cursor_before: None,
            prediction: WindowPrediction {
                slots: vec![SlotPrediction {
                    slot: 0,
                    before: Some(SlotExpectation {
                        item_name: "oak_log".to_owned(),
                        count: 1,
                        metadata: 0,
                    }),
                    after: None,
                }],
                cursor_before: None,
                cursor_after: Some(SlotExpectation {
                    item_name: "oak_planks".to_owned(),
                    count: 4,
                    metadata: 0,
                }),
            },
        };
        apply_accepted_normal_click(&mut inventory, &pending).unwrap();
        assert!(inventory.windows[&0][0].is_none());
        assert_eq!(inventory.cursor, Some(output));
    }

    #[test]
    fn accepted_equip_swap_repairs_omitted_player_slot_facts_and_preserves_nbt() {
        let source = ItemStack {
            item_id: crate::registry::item_id("crafting_table").unwrap(),
            count: 1,
            nbt: Some(vec![10, 1, 2]),
        };
        let destination = ItemStack {
            item_id: crate::registry::item_id("oak_log").unwrap(),
            count: 2,
            nbt: None,
        };
        let mut inventory = InventoryState {
            windows: [(0, {
                let mut slots = vec![None; 46];
                slots[9] = Some(source.clone());
                slots[36] = Some(destination.clone());
                slots
            })]
            .into_iter()
            .collect(),
            ..InventoryState::default()
        };
        apply_accepted_equip_swap(&mut inventory, 9, 0, &source, Some(&destination));
        assert_eq!(inventory.windows[&0][9], Some(destination.clone()));
        assert_eq!(inventory.windows[&0][36], Some(source.clone()));

        // An early server fact wins per domain; the missing source update can
        // still be repaired without overwriting the already-factual target.
        inventory.windows.get_mut(&0).unwrap()[36] = Some(ItemStack {
            item_id: crate::registry::item_id("stick").unwrap(),
            count: 1,
            nbt: None,
        });
        apply_accepted_equip_swap(&mut inventory, 9, 0, &source, Some(&destination));
        assert_eq!(inventory.windows[&0][9], Some(destination));
        assert_eq!(
            inventory.windows[&0][36].as_ref().unwrap().name(),
            Some("stick")
        );
    }

    #[test]
    fn exact_slot_precondition_rejects_wrong_metadata_before_dispatch() {
        let item = ItemStack {
            item_id: 1,
            count: 2,
            nbt: None,
        };
        let exact = SlotExpectation {
            item_name: item.name().unwrap().to_owned(),
            count: 2,
            metadata: 0,
        };
        assert!(slot_matches(&item, &exact));
        assert!(!slot_matches(
            &item,
            &SlotExpectation {
                metadata: 1,
                ..exact
            }
        ));
    }

    #[test]
    fn interaction_entity_actions_encode_required_state_and_exact_restore() {
        assert_eq!(
            encode_entity_action_packet(42, true),
            (0x1c, vec![42, 0, 0])
        );
        assert_eq!(
            encode_entity_action_packet(42, false),
            (0x1c, vec![42, 1, 0])
        );

        for restore in [false, true] {
            let packets = [
                encode_entity_action_packet(42, true),
                (0x2d, vec![7]),
                encode_entity_action_packet(42, restore),
            ];
            assert_eq!(packets[0], (0x1c, vec![42, 0, 0]));
            assert_eq!(packets[1], (0x2d, vec![7]));
            assert_eq!(packets[2], encode_entity_action_packet(42, restore));
        }
    }

    #[test]
    fn protocol_time_and_air_values_preserve_vanilla_semantics() {
        assert_eq!(
            CoherentWorldTime {
                world_age: 1,
                time_of_day: 24_001,
                daylight_cycle: Some(true),
            }
            .contract_time_of_day(),
            Some((1, true))
        );
        assert_eq!(
            CoherentWorldTime {
                world_age: 1,
                time_of_day: -24_001,
                daylight_cycle: Some(false),
            }
            .contract_time_of_day(),
            Some((1, false))
        );
        assert_eq!(oxygen_level_from_air_ticks(300), Some(20));
        assert_eq!(oxygen_level_from_air_ticks(0), Some(0));
        assert_eq!(oxygen_level_from_air_ticks(316), None);
        assert_eq!(oxygen_level_from_air_ticks(-1), None);
        assert_eq!(protocol_default_oxygen_level(), 20);
        assert_eq!(oxygen_level_after_respawn(Some(7), true), Some(7));
        assert_eq!(oxygen_level_after_respawn(None, true), None);
        assert_eq!(oxygen_level_after_respawn(Some(7), false), Some(20));
    }

    #[test]
    fn movement_speed_applies_sprint_once_using_protocol_attributes() {
        let no_attribute = None;
        assert!((movement_speed_for_control(no_attribute, false) - 0.1).abs() < 1.0e-12);
        assert!((movement_speed_for_control(no_attribute, true) - 0.13).abs() < 1.0e-12);

        let base_attribute = Attribute {
            key: "generic.movement_speed".into(),
            base: 0.2,
            modifiers: Vec::new(),
        };
        assert!((movement_speed_for_control(Some(&base_attribute), false) - 0.2).abs() < 1.0e-12);
        assert!((movement_speed_for_control(Some(&base_attribute), true) - 0.26).abs() < 1.0e-12);

        let known_sprint_attribute = Attribute {
            key: "generic.movement_speed".into(),
            base: 0.1,
            modifiers: vec![AttributeModifier {
                uuid: VANILLA_SPRINT_MODIFIER_UUID,
                amount: 0.3,
                operation: 2,
            }],
        };
        let observed_speed = known_sprint_attribute.value();
        assert!((observed_speed - 0.13).abs() < 1.0e-12);
        assert!(
            (movement_speed_for_control(Some(&known_sprint_attribute), true) - 0.13).abs()
                < 1.0e-12
        );

        let unrelated_modifier_attribute = Attribute {
            key: "generic.movement_speed".into(),
            base: 0.1,
            modifiers: vec![AttributeModifier {
                uuid: [0; 16],
                amount: 0.2,
                operation: 2,
            }],
        };
        assert!(
            (movement_speed_for_control(Some(&unrelated_modifier_attribute), true) - 0.156).abs()
                < 1.0e-12
        );
    }
    use crate::{
        AttributeModifier, BlockQueryCoverage, BlockRegion, CoherentObservationRequest, ErrorKind,
    };
    use std::collections::HashMap;
    use std::{
        pin::Pin,
        task::{Context, Poll},
    };
    use tokio::{
        io::{AsyncRead, AsyncReadExt, ReadBuf},
        net::TcpListener,
        sync::{Notify, oneshot},
    };

    include!("client/packet_deadline_tests.rs");
    include!("client/packet_fairness_tests.rs");
    include!("client/reader_throughput_tests.rs");
    include!("client/storage_tests.rs");

    struct CountingRead<R> {
        inner: R,
        consumed: Arc<AtomicUsize>,
        consumed_notify: Arc<Notify>,
    }

    impl<R: AsyncRead + Unpin> AsyncRead for CountingRead<R> {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let before = buf.filled().len();
            let result = Pin::new(&mut self.inner).poll_read(cx, buf);
            if let Poll::Ready(Ok(())) = result {
                let delta = buf.filled().len().saturating_sub(before);
                if delta != 0 {
                    self.consumed.fetch_add(delta, Ordering::AcqRel);
                    self.consumed_notify.notify_one();
                }
            }
            result
        }
    }

    #[tokio::test]
    async fn login_profile_rejects_malformed_or_different_received_success() {
        for kind in 0..3 {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let (mut reader, mut writer) = stream.into_split();
                read_packet(&mut reader, None).await.unwrap();
                let (_, login) = read_packet(&mut reader, None).await.unwrap();
                let mut payload = crate::client::login::test_legacy_success(&login);
                match kind {
                    0 => payload.clear(),
                    1 => {
                        payload.truncate(16);
                        put_string(&mut payload, "Different");
                    }
                    _ => payload.push(0),
                }
                write_packet(&mut writer, None, 2, &payload).await.unwrap();
                let mut byte = [0; 1];
                assert_eq!(
                    timeout(Duration::from_secs(2), reader.read(&mut byte))
                        .await
                        .unwrap()
                        .unwrap(),
                    0
                );
            });
            assert!(
                Bot::connect(
                    Server::new("127.0.0.1", port),
                    Player::offline("ProfileProbe"),
                    Arc::new(crate::SharedChunkStorage::default()),
                    ConnectionOptions::default()
                )
                .await
                .is_err()
            );
            server.await.unwrap();
        }
    }

    async fn connected_test_bot(
        connection_options: ConnectionOptions,
        play_packets: Vec<(i32, Vec<u8>)>,
    ) -> (Bot, tokio::task::JoinHandle<()>, oneshot::Sender<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (release_tx, release_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut reader, mut writer) = stream.into_split();
            read_packet(&mut reader, None).await.unwrap();
            let (_, login) = read_packet(&mut reader, None).await.unwrap();
            write_packet(
                &mut writer,
                None,
                2,
                &crate::client::login::test_legacy_success(&login),
            )
            .await
            .unwrap();
            release_rx.await.unwrap();
            for (id, payload) in play_packets {
                write_packet(&mut writer, None, id, &payload).await.unwrap();
            }
            let mut byte = [0_u8; 1];
            let _ = reader.read(&mut byte).await;
        });
        let bot = Bot::connect(
            Server::new("127.0.0.1", port),
            Player::offline("LimitProbe"),
            Arc::new(crate::SharedChunkStorage::default()),
            connection_options,
        )
        .await
        .unwrap();
        (bot, server, release_tx)
    }

    pub(super) async fn ready_test_bot(
        connection_options: ConnectionOptions,
    ) -> (Bot, tokio::task::JoinHandle<()>, oneshot::Sender<()>) {
        let (bot, server, release) = connected_test_bot(connection_options, Vec::new()).await;
        bot.player.lock().await.spawned = true;
        *bot.positioned.lock().await = true;
        bot.connection.mark_ready().await;
        tokio::time::timeout(Duration::from_secs(1), async {
            while bot.connection_state() != ConnectionState::Ready {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        (bot, server, release)
    }

    #[tokio::test]
    async fn exact_empty_hotbar_selection_uses_existing_packet_and_updates_cache() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        bot.inventory
            .write()
            .await
            .windows
            .insert(0, vec![None; 45]);
        let outcome = bot
            .dispatch_equip(
                bot.operation_context(0),
                EquipOperation::SelectEmptyHotbar { hotbar_slot: 3 },
            )
            .await
            .unwrap();
        assert_eq!(outcome, DispatchOutcome::Dispatched);
        assert_eq!(bot.inventory.read().await.selected_hotbar, 3);
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn exact_empty_hotbar_selection_rejects_occupied_and_out_of_range_slots() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        let mut slots = vec![None; 45];
        slots[39] = Some(ItemStack {
            item_id: 1,
            count: 1,
            nbt: None,
        });
        bot.inventory.write().await.windows.insert(0, slots);
        for hotbar_slot in [3, 9] {
            assert!(matches!(
                bot.dispatch_equip(
                    bot.operation_context(0),
                    EquipOperation::SelectEmptyHotbar { hotbar_slot }
                )
                .await,
                Err(DispatchError::InvalidInput)
            ));
        }
        assert_eq!(bot.inventory.read().await.selected_hotbar, 0);
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn generic_read_loop_preserves_one_partial_packet_future_deterministically() {
        for prefix_len in [1, 2, 3] {
            let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
            bot.writer.lock().await.compression = Some(256);
            let (mut tx, rx) = tokio::io::duplex(1024);
            let consumed = Arc::new(AtomicUsize::new(0));
            let consumed_notify = Arc::new(Notify::new());
            let (capture_tx, capture_rx) = tokio::sync::mpsc::channel(2);
            let (movement_tx, movement_rx) = tokio::sync::mpsc::channel(2);
            let reader = CountingRead {
                inner: rx,
                consumed: Arc::clone(&consumed),
                consumed_notify: Arc::clone(&consumed_notify),
            };
            let loop_bot = bot.clone_internal();
            let loop_task =
                tokio::spawn(
                    async move { loop_bot.read_loop(reader, capture_rx, movement_rx).await },
                );
            // Compression-enabled, below-threshold packet: frame = 00 1e 01 00 00 00 00.
            let wire = [7, 0, 0x1e, 1, 0, 0, 0, 0];
            tokio::io::AsyncWriteExt::write_all(&mut tx, &wire[..prefix_len])
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(1), async {
                while consumed.load(Ordering::Acquire) < prefix_len {
                    consumed_notify.notified().await;
                }
            })
            .await
            .unwrap();
            assert_eq!(consumed.load(Ordering::Acquire), prefix_len);

            for ordinal in 0..2 {
                let (capture_reply, capture_result) = oneshot::channel();
                capture_tx
                    .send(crate::observation::CaptureCommand {
                        request: CoherentObservationRequest::default(),
                        reply: capture_reply,
                    })
                    .await
                    .unwrap();
                let observation = timeout(Duration::from_secs(1), capture_result)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                assert_eq!(observation.sequence.get(), 2 * ordinal + 1);
                let (movement_reply, movement_result) = oneshot::channel();
                movement_tx
                    .send(crate::observation::TraversalMovementFactsCommand {
                        request: crate::MovementSnapshotRequest {
                            expected_generation: bot.connection_generation(),
                            region: BlockRegion::new(
                                BlockPos { x: 0, y: 0, z: 0 },
                                BlockPos { x: 0, y: 0, z: 0 },
                            ),
                            entity_radius: 0,
                            max_entities: 0,
                        },
                        reply: movement_reply,
                    })
                    .await
                    .unwrap();
                let observation = timeout(Duration::from_secs(1), movement_result)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                assert_eq!(observation.sequence.get(), 2 * ordinal + 2);
                assert_eq!(bot.protocol_packet_sequence.load(Ordering::Acquire), 0);
                assert_eq!(consumed.load(Ordering::Acquire), prefix_len);
            }
            // Only finish the frame after both request types completed twice.
            tokio::io::AsyncWriteExt::write_all(&mut tx, &wire[prefix_len..])
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(1), async {
                while bot.protocol_packet_sequence.load(Ordering::Acquire) != 1 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            bot.cancel.notify_waiters();
            loop_task.await.unwrap().unwrap();
            drop(release);
            server.abort();
        }
    }

    #[tokio::test]
    async fn traversal_geometry_query_projects_exact_order_and_unknown_states() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        bot.survival.write().await.dimension = Some("minecraft:overworld".to_owned());
        let empty = bot.capture_loaded_geometry().await.unwrap();
        assert!(empty.loaded_chunks().is_empty());
        assert!(empty.sections().is_empty());
        assert_eq!(empty.logical_cell_count(), 0);
        assert!(
            bot.apply_packet(0x21, empty_chunk_packet(0, 0))
                .await
                .unwrap()
        );
        bot.world
            .lock()
            .await
            .set_block_for_test(BlockPos { x: 0, y: 0, z: 0 }, 1);
        bot.world
            .lock()
            .await
            .set_block_for_test(BlockPos { x: 1, y: 0, z: 0 }, i32::MAX);
        let observation = bot
            .capture_coherent_observation(CoherentObservationRequest {
                max_entities: 0,
                max_events: 0,
                ..CoherentObservationRequest::default()
            })
            .await
            .unwrap();
        let request = GeometryQuery {
            expected_capture: observation.sensor_capture,
            expected_dimension: "minecraft:overworld".to_owned(),
            region: BlockRegion::new(BlockPos { x: 0, y: 0, z: 0 }, BlockPos { x: 1, y: 1, z: 1 }),
        };
        let snapshot = bot.query_geometry(request.clone()).await.unwrap();
        assert_eq!(snapshot.capture, request.expected_capture);
        assert_eq!(snapshot.evaluated_origin.x, observation.player.x);
        assert_eq!(snapshot.evaluated_origin.y, observation.player.y);
        assert_eq!(snapshot.evaluated_origin.z, observation.player.z);
        assert_eq!(snapshot.blocks.len(), 8);
        let positions = snapshot
            .blocks
            .iter()
            .map(|fact| match fact {
                GeometryBlock::Loaded { position, .. }
                | GeometryBlock::Unloaded { position }
                | GeometryBlock::Unknown { position, .. } => (position.x, position.y, position.z),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            positions,
            vec![
                (0, 0, 0),
                (0, 0, 1),
                (0, 1, 0),
                (0, 1, 1),
                (1, 0, 0),
                (1, 0, 1),
                (1, 1, 0),
                (1, 1, 1),
            ]
        );
        assert!(matches!(
            snapshot.blocks[0],
            GeometryBlock::Loaded {
                collision: BlockCollision::NonEmpty,
                support_surface: crate::BlockSupportSurface::FullTop,
                ..
            }
        ));
        assert!(matches!(
            snapshot.blocks[4],
            GeometryBlock::Unknown {
                state_id: i32::MAX,
                ..
            }
        ));

        let edge = GeometryQuery {
            region: BlockRegion::new(
                BlockPos { x: 15, y: 0, z: 0 },
                BlockPos { x: 16, y: 0, z: 0 },
            ),
            ..request.clone()
        };
        let edge_snapshot = bot.query_geometry(edge).await.unwrap();
        assert!(matches!(
            edge_snapshot.blocks[0],
            GeometryBlock::Loaded {
                collision: BlockCollision::Empty,
                support_surface: crate::BlockSupportSurface::NotFullTop,
                ..
            }
        ));
        assert!(matches!(
            edge_snapshot.blocks[1],
            GeometryBlock::Unloaded { .. }
        ));
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn loaded_geometry_snapshot_is_canonical_complete_and_immutable() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        bot.survival.write().await.dimension = Some("minecraft:overworld".to_owned());
        assert!(
            bot.apply_packet(0x21, empty_chunk_packet(1, 0))
                .await
                .unwrap()
        );
        assert!(
            bot.apply_packet(0x21, empty_chunk_packet(-1, 0))
                .await
                .unwrap()
        );
        let position = BlockPos { x: -1, y: 64, z: 0 };
        bot.world.lock().await.set_block_for_test(position, 1);
        bot.world.lock().await.set_block_for_test(
            BlockPos {
                x: -16,
                y: 255,
                z: 15,
            },
            2,
        );

        let snapshot = bot.capture_loaded_geometry().await.unwrap();
        assert_eq!(snapshot.dimension(), "minecraft:overworld");
        assert_eq!(
            snapshot.loaded_chunks(),
            &[
                crate::ChunkPos { x: -1, z: 0 },
                crate::ChunkPos { x: 1, z: 0 }
            ]
        );
        assert_eq!(snapshot.logical_cell_count(), 2 * 16 * 16 * 256);
        assert_eq!(snapshot.state_id(position), Some(1));
        assert_eq!(
            snapshot.state_id(BlockPos {
                x: -16,
                y: 255,
                z: 15
            }),
            Some(2),
            "negative chunk coordinates use Euclidean local coordinates"
        );
        assert_eq!(snapshot.state_id(BlockPos { x: -1, y: -1, z: 0 }), None);
        assert_eq!(
            snapshot.state_id(BlockPos {
                x: -1,
                y: 256,
                z: 0
            }),
            None
        );
        assert!(snapshot.sections().windows(2).all(|pair| {
            (pair[0].chunk(), pair[0].section_y()) < (pair[1].chunk(), pair[1].section_y())
        }));
        assert!(format!("{snapshot:?}").len() < 512);
        let crate::PlacedBlockPhysicalDescriptorLookup::Known(furnace) =
            snapshot.placed_descriptor("furnace")
        else {
            panic!("snapshot-bound registry resolves invariant furnace placement facts")
        };
        assert_eq!(furnace.state_id(), 3374);
        assert_eq!(snapshot.registry_identity().descriptor_revision(), 3);
        assert_eq!(
            snapshot.state_id(BlockPos {
                x: 16,
                y: 200,
                z: 0
            }),
            Some(0),
            "a missing section in a loaded chunk is implicit air"
        );
        assert_eq!(
            snapshot.state_id(BlockPos { x: 0, y: 64, z: 0 }),
            None,
            "a position outside the loaded manifest is unavailable"
        );

        bot.world.lock().await.set_block_for_test(position, 2);
        assert_eq!(snapshot.state_id(position), Some(1));
        let newer = bot.capture_loaded_geometry().await.unwrap();
        assert_eq!(newer.state_id(position), Some(2));

        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn traversal_geometry_query_returns_actual_origin_and_rejects_generation_dimension_and_nonfinite_origin()
     {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        bot.survival.write().await.dimension = Some("minecraft:overworld".to_owned());
        let observation = bot
            .capture_coherent_observation(CoherentObservationRequest {
                max_entities: 0,
                max_events: 0,
                ..CoherentObservationRequest::default()
            })
            .await
            .unwrap();
        let request = GeometryQuery {
            expected_capture: observation.sensor_capture,
            expected_dimension: "minecraft:overworld".to_owned(),
            region: BlockRegion::new(BlockPos { x: 0, y: 0, z: 0 }, BlockPos { x: 0, y: 0, z: 0 }),
        };
        bot.player.lock().await.x = observation.player.x + 1.0;
        let moved = bot.query_geometry(request.clone()).await.unwrap();
        assert_eq!(moved.evaluated_origin.x, observation.player.x + 1.0);
        assert_eq!(moved.evaluated_origin.y, observation.player.y);
        assert_eq!(moved.evaluated_origin.z, observation.player.z);

        bot.player.lock().await.x = f64::NAN;
        assert!(bot.query_geometry(request.clone()).await.is_err());
        bot.player.lock().await.x = observation.player.x;

        let mut generation_mismatch = request.clone();
        generation_mismatch.expected_capture.generation = ConnectionGeneration::allocate();
        assert!(bot.query_geometry(generation_mismatch).await.is_err());

        let mut dimension_mismatch = request.clone();
        dimension_mismatch.expected_dimension = "minecraft:the_nether".to_owned();
        assert!(bot.query_geometry(dimension_mismatch).await.is_err());

        let mut geometry_revision_mismatch = request.clone();
        bot.advance_block_geometry_revision();
        geometry_revision_mismatch
            .expected_capture
            .block_geometry_revision += 1;
        let geometry_snapshot = bot
            .query_geometry(geometry_revision_mismatch)
            .await
            .unwrap();
        assert_eq!(
            geometry_snapshot.capture.block_geometry_revision,
            request.expected_capture.block_geometry_revision + 1
        );

        let mut inventory_revision_mismatch = request.clone();
        bot.inventory.write().await.selected_hotbar = 1;
        inventory_revision_mismatch
            .expected_capture
            .inventory_revision += 1;
        let inventory_snapshot = bot
            .query_geometry(inventory_revision_mismatch)
            .await
            .unwrap();
        assert_eq!(
            inventory_snapshot.capture.inventory_revision,
            request.expected_capture.inventory_revision + 1
        );
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn traversal_geometry_query_rejects_a_non_ready_lifecycle() {
        let (bot, server, release) =
            connected_test_bot(ConnectionOptions::default(), Vec::new()).await;
        bot.survival.write().await.dimension = Some("minecraft:overworld".to_owned());
        let request = GeometryQuery {
            expected_capture: crate::CaptureIdentity {
                generation: bot.connection_generation(),
                block_geometry_revision: 0,
                inventory_revision: 0,
            },
            expected_dimension: "minecraft:overworld".to_owned(),
            region: BlockRegion::new(BlockPos { x: 0, y: 0, z: 0 }, BlockPos { x: 0, y: 0, z: 0 }),
        };
        assert_ne!(bot.connection_state(), ConnectionState::Ready);
        assert!(bot.query_geometry(request).await.is_err());

        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn unrelated_observation_sequence_advances_without_changing_sensor_identity() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        bot.survival.write().await.dimension = Some("minecraft:overworld".to_owned());

        let first = bot
            .capture_coherent_observation(CoherentObservationRequest {
                max_entities: 0,
                max_events: 0,
                ..CoherentObservationRequest::default()
            })
            .await
            .unwrap();
        let previous_player_revision = bot.player_snapshot().await.revision;
        let on_ground = bot.player_snapshot().await.value.on_ground;
        bot.player.lock().await.on_ground = !on_ground;
        assert!(bot.player_snapshot().await.revision > previous_player_revision);
        let second = bot
            .capture_coherent_observation(CoherentObservationRequest {
                max_entities: 0,
                max_events: 0,
                ..CoherentObservationRequest::default()
            })
            .await
            .unwrap();

        assert!(second.sequence > first.sequence);
        assert_eq!(second.generation, first.generation);
        assert_eq!(second.sensor_capture, first.sensor_capture);

        let query = GeometryQuery {
            expected_capture: first.sensor_capture,
            expected_dimension: "minecraft:overworld".to_owned(),
            region: BlockRegion::new(BlockPos { x: 0, y: 0, z: 0 }, BlockPos { x: 0, y: 0, z: 0 }),
        };
        assert!(bot.query_geometry(query).await.is_ok());

        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    fn loaded_resource_request(
        bot: &Bot,
        region: BlockRegion,
        block_name: &str,
        limit: u16,
    ) -> crate::BlockQuery {
        crate::BlockQuery {
            expected_generation: bot.operation_context(0).generation,
            expected_dimension: "minecraft:overworld".to_owned(),
            region,
            block_name: block_name.to_owned(),
            limit,
        }
    }

    #[tokio::test]
    async fn loaded_resource_query_reports_complete_coverage_and_bounded_matches() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        bot.survival.write().await.dimension = Some("minecraft:overworld".to_owned());
        assert!(
            bot.apply_packet(0x21, empty_chunk_packet(0, 0))
                .await
                .unwrap()
        );
        let result = bot
            .query_loaded_blocks(loaded_resource_request(
                &bot,
                BlockRegion::new(BlockPos { x: 0, y: 0, z: 0 }, BlockPos { x: 1, y: 0, z: 0 }),
                "air",
                1,
            ))
            .await
            .unwrap();
        assert_eq!(result.coverage, BlockQueryCoverage::Complete);
        assert_eq!(result.capture.generation, bot.connection_generation());
        assert_eq!(
            result.capture.block_geometry_revision,
            bot.block_geometry_revision.load(Ordering::Acquire)
        );
        assert_eq!(
            result.capture.inventory_revision,
            bot.inventory.read().await.revision()
        );
        assert_eq!(result.candidates, vec![BlockPos { x: 0, y: 0, z: 0 }]);
        assert_eq!(result.omitted_candidates, 1);
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn loaded_resource_query_uses_index_and_fails_closed_when_it_is_incomplete() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        bot.survival.write().await.dimension = Some("minecraft:overworld".to_owned());
        assert!(
            bot.apply_packet(0x21, empty_chunk_packet(0, 0))
                .await
                .unwrap()
        );
        let iron = crate::registry::block_state_ranges_for_names(&["iron_ore"])[0].0;
        bot.world
            .lock()
            .await
            .set_block_for_test(BlockPos { x: 2, y: 3, z: 4 }, iron);
        let result = bot
            .query_loaded_blocks(loaded_resource_request(
                &bot,
                BlockRegion::new(
                    BlockPos { x: 0, y: 0, z: 0 },
                    BlockPos {
                        x: 15,
                        y: 31,
                        z: 15,
                    },
                ),
                "iron_ore",
                4,
            ))
            .await
            .unwrap();
        assert_eq!(result.candidates, vec![BlockPos { x: 2, y: 3, z: 4 }]);

        {
            let mut world = bot.world.lock().await;
            for index in 0..8_193_i32 {
                let y = index / 256;
                let local = index % 256;
                world.set_block_for_test(
                    BlockPos {
                        x: local % 16,
                        y,
                        z: local / 16,
                    },
                    iron,
                );
            }
        }
        assert!(
            bot.query_loaded_blocks(loaded_resource_request(
                &bot,
                BlockRegion::new(
                    BlockPos { x: 0, y: 0, z: 0 },
                    BlockPos {
                        x: 15,
                        y: 32,
                        z: 15
                    }
                ),
                "iron_ore",
                4,
            ))
            .await
            .is_err()
        );
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn loaded_resource_query_reports_partial_coverage_and_stale_generation() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        bot.survival.write().await.dimension = Some("minecraft:overworld".to_owned());
        let partial = bot
            .query_loaded_blocks(loaded_resource_request(
                &bot,
                BlockRegion::new(
                    BlockPos { x: 0, y: 0, z: 0 },
                    BlockPos { x: 16, y: 0, z: 0 },
                ),
                "air",
                4,
            ))
            .await
            .unwrap();
        assert!(matches!(
            partial.coverage,
            BlockQueryCoverage::Partial { missing_chunks: 2 }
        ));
        assert_eq!(partial.capture.generation, bot.connection_generation());
        assert_eq!(
            partial.capture.block_geometry_revision,
            bot.block_geometry_revision.load(Ordering::Acquire)
        );
        assert_eq!(
            partial.capture.inventory_revision,
            bot.inventory.read().await.revision()
        );

        let mut stale = loaded_resource_request(
            &bot,
            BlockRegion::new(BlockPos { x: 0, y: 0, z: 0 }, BlockPos { x: 0, y: 0, z: 0 }),
            "air",
            4,
        );
        stale.expected_generation = ConnectionGeneration::allocate();
        assert!(bot.query_loaded_blocks(stale).await.is_err());
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn disconnect_waits_for_server_read_eof() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (release_tx, release_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut reader, mut _writer) = stream.into_split();
            read_packet(&mut reader, None).await.unwrap();
            let (_, login) = read_packet(&mut reader, None).await.unwrap();
            write_packet(
                &mut _writer,
                None,
                2,
                &crate::client::login::test_legacy_success(&login),
            )
            .await
            .unwrap();
            release_rx.await.unwrap();
        });
        let bot = Bot::connect(
            Server::new("127.0.0.1", port),
            Player::offline("DiscBarrier"),
            Arc::new(crate::SharedChunkStorage::default()),
            ConnectionOptions::default(),
        )
        .await
        .unwrap();
        bot.set_control(ControlState {
            forward: true,
            ..ControlState::default()
        })
        .await;
        let lifecycle = bot.clone();
        let mut disconnect = tokio::spawn(async move { bot.disconnect().await });
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut disconnect)
                .await
                .is_err()
        );
        assert_eq!(lifecycle.control().await, ControlState::default());
        release_tx.send(()).unwrap();
        assert!(disconnect.await.unwrap().is_ok());
        assert_eq!(lifecycle.connection_state(), ConnectionState::Disconnected);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn disconnect_protocol_rejection_waits_for_transport_end_and_preserves_errors() {
        for malformed_tail in [false, true] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let (written_tx, written_rx) = oneshot::channel();
            let (end_tx, end_rx) = oneshot::channel();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let (mut reader, mut writer) = stream.into_split();
                read_packet(&mut reader, None).await.unwrap();
                let (_, login) = read_packet(&mut reader, None).await.unwrap();
                write_packet(
                    &mut writer,
                    None,
                    2,
                    &crate::client::login::test_legacy_success(&login),
                )
                .await
                .unwrap();
                // Read the caller's actual write-half shutdown, then deliver an
                // in-flight server challenge while retaining the server half.
                while read_packet(&mut reader, None).await.is_ok() {}
                write_packet(&mut writer, None, 0x20, &71_i64.to_be_bytes())
                    .await
                    .unwrap();
                written_tx.send(()).unwrap();
                end_rx.await.unwrap();
                if malformed_tail {
                    // An overlong frame-length VarInt is a protocol failure,
                    // distinct from the intentional admission rejection.
                    tokio::io::AsyncWriteExt::write_all(
                        &mut writer,
                        &[0x80, 0x80, 0x80, 0x80, 0x80, 0],
                    )
                    .await
                    .unwrap();
                }
            });
            let bot = Bot::connect(
                Server::new("127.0.0.1", port),
                Player::offline("DiscProtocol"),
                Arc::new(crate::SharedChunkStorage::default()),
                ConnectionOptions::default(),
            )
            .await
            .unwrap();
            let applied = bot.packet_applied.notified();
            tokio::pin!(applied);
            applied.as_mut().enable();
            let client = crate::Client::from_java_1_16_1(bot.clone());
            let mut disconnect = tokio::spawn(async move { client.disconnect().await });
            written_rx.await.unwrap();
            timeout(Duration::from_secs(2), &mut applied).await.unwrap();
            assert!(
                timeout(Duration::from_millis(50), &mut disconnect)
                    .await
                    .is_err()
            );
            assert_eq!(bot.connection_state(), ConnectionState::Disconnecting);
            end_tx.send(()).unwrap();
            let result = timeout(Duration::from_secs(2), disconnect)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(result.is_ok(), !malformed_tail);
            assert_eq!(
                bot.connection_state(),
                if malformed_tail {
                    ConnectionState::ConnectionStateUnknown
                } else {
                    ConnectionState::Disconnected
                }
            );
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn wait_until_ready_returns_only_after_actor_ready_commit() {
        let (bot, server, release) =
            connected_test_bot(ConnectionOptions::default(), Vec::new()).await;
        bot.player.lock().await.spawned = true;
        *bot.positioned.lock().await = true;
        bot.wait_until_ready().await.unwrap();
        assert_eq!(bot.connection_state(), ConnectionState::Ready);
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn packet_acknowledgements_route_to_actor_transactions() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        let stack = ItemStack {
            item_id: 1,
            count: 2,
            nbt: None,
        };
        bot.inventory
            .write()
            .await
            .windows
            .insert(0, vec![Some(stack.clone())]);
        let window_transaction = bot
            .dispatch_acknowledged(
                bot.operation_context(0),
                AcknowledgedOperation::WindowClick {
                    window_id: 0,
                    slot: 0,
                    button: 0,
                    mode: ClickMode::Normal,
                    clicked: Some(stack.clone()),
                },
            )
            .await
            .unwrap();
        let action = window_transaction.window_action().unwrap();
        bot.inventory.write().await.pending_clicks.insert(
            (0, action),
            PendingClick {
                window_id: 0,
                action,
                slot: 0,
                button: 0,
                mode: ClickMode::Normal,
                slot_before: Some(stack),
                cursor_before: None,
                prediction: Default::default(),
            },
        );
        bot.exact_window_barriers.lock().await.insert(
            (0, action),
            ExactWindowBarrier {
                successor: None,
                confirmation_seen: false,
            },
        );
        let waiter_bot = bot.clone_internal();
        let waiter = tokio::spawn(async move {
            let outcome = window_transaction.wait().await;
            let inventory = waiter_bot.inventory.read().await;
            (
                outcome,
                inventory.windows[&0][0].clone(),
                inventory.cursor.clone(),
            )
        });
        let mut window_ack = Vec::new();
        window_ack.write_i8(0).unwrap();
        window_ack.write_i16::<BigEndian>(action).unwrap();
        window_ack.push(1);
        assert!(bot.apply_packet(0x12, window_ack).await.unwrap());
        let (outcome, slot, cursor) = waiter.await.unwrap();
        assert_eq!(outcome, DispatchOutcome::Acknowledged);
        assert!(slot.is_none());
        assert_eq!(cursor.map(|item| item.count), Some(2));

        let corrective = ItemStack {
            item_id: 3,
            count: 1,
            nbt: None,
        };
        let second = bot
            .dispatch_acknowledged(
                bot.operation_context(0),
                AcknowledgedOperation::WindowClick {
                    window_id: 0,
                    slot: 0,
                    button: 0,
                    mode: ClickMode::Normal,
                    clicked: None,
                },
            )
            .await
            .unwrap();
        let second_action = second.window_action().unwrap();
        bot.inventory.write().await.pending_clicks.insert(
            (0, second_action),
            PendingClick {
                window_id: 0,
                action: second_action,
                slot: 0,
                button: 0,
                mode: ClickMode::Normal,
                slot_before: None,
                cursor_before: None,
                prediction: Default::default(),
            },
        );
        bot.exact_window_barriers.lock().await.insert(
            (0, second_action),
            ExactWindowBarrier {
                successor: None,
                confirmation_seen: false,
            },
        );
        {
            let mut inventory = bot.inventory.write().await;
            inventory.windows.get_mut(&0).unwrap()[0] = Some(corrective.clone());
            inventory.cursor = None;
        }
        let mut second_ack = Vec::new();
        second_ack.write_i8(0).unwrap();
        second_ack.write_i16::<BigEndian>(second_action).unwrap();
        second_ack.push(1);
        assert!(bot.apply_packet(0x12, second_ack).await.unwrap());
        assert_eq!(second.wait().await, DispatchOutcome::Acknowledged);
        assert_eq!(bot.inventory.read().await.windows[&0][0], Some(corrective));

        let position = BlockPos { x: -2, y: 4, z: 6 };
        let dig_transaction = bot
            .dispatch_acknowledged(
                bot.operation_context(0),
                AcknowledgedOperation::DigFinish {
                    position,
                    face: BlockFace::North,
                },
            )
            .await
            .unwrap();
        let mut dig_ack = Vec::new();
        dig_ack.write_u64::<BigEndian>(position.packed()).unwrap();
        put_varint(&mut dig_ack, 0);
        put_varint(&mut dig_ack, DiggingStatus::Finished as i32);
        dig_ack.push(1);
        assert!(bot.apply_packet(0x07, dig_ack).await.unwrap());
        assert_eq!(dig_transaction.wait().await, DispatchOutcome::Acknowledged);

        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn exact_confirmation_waits_for_successor_server_cache_fact() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        let stack = ItemStack {
            item_id: 1,
            count: 1,
            nbt: None,
        };
        bot.inventory
            .write()
            .await
            .windows
            .insert(0, vec![Some(stack.clone()), Some(stack.clone())]);
        let transaction = bot
            .dispatch_acknowledged(
                bot.operation_context(0),
                AcknowledgedOperation::WindowClick {
                    window_id: 0,
                    slot: 0,
                    button: 0,
                    mode: ClickMode::Drop,
                    clicked: Some(stack.clone()),
                },
            )
            .await
            .unwrap();
        let action = transaction.window_action().unwrap();
        bot.inventory.write().await.pending_clicks.insert(
            (0, action),
            PendingClick {
                window_id: 0,
                action,
                slot: 0,
                button: 0,
                mode: ClickMode::Drop,
                slot_before: Some(stack),
                cursor_before: None,
                prediction: Default::default(),
            },
        );
        bot.exact_window_barriers.lock().await.insert(
            (0, action),
            ExactWindowBarrier {
                successor: Some(WindowClick {
                    slot: 1,
                    button: 0,
                    mode: ClickMode::Normal,
                    expected_item: None,
                    expected_cursor: None,
                    prediction: Default::default(),
                }),
                confirmation_seen: false,
            },
        );
        let mut waiter = tokio::spawn(transaction.wait());
        let mut confirmation = Vec::new();
        confirmation.write_i8(0).unwrap();
        confirmation.write_i16::<BigEndian>(action).unwrap();
        confirmation.push(1);
        assert!(bot.apply_packet(0x12, confirmation).await.unwrap());
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut waiter)
                .await
                .is_err()
        );

        let mut set_slot = Vec::new();
        set_slot.write_i8(0).unwrap();
        set_slot.write_i16::<BigEndian>(1).unwrap();
        write_slot(&mut set_slot, None);
        assert!(bot.apply_packet(0x16, set_slot).await.unwrap());
        assert_eq!(waiter.await.unwrap(), DispatchOutcome::Acknowledged);
        assert!(bot.exact_window_barriers.lock().await.is_empty());

        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    fn empty_chunk_packet(x: i32, z: i32) -> Vec<u8> {
        let mut packet = Vec::new();
        packet.write_i32::<BigEndian>(x).unwrap();
        packet.write_i32::<BigEndian>(z).unwrap();
        packet.push(1);
        packet.push(0);
        put_varint(&mut packet, 0);
        packet.push(0);
        packet.extend([0_u8; 4096]);
        put_varint(&mut packet, 0);
        put_varint(&mut packet, 0);
        packet
    }

    fn digging_ack_packet(
        position: BlockPos,
        block_state_id: i32,
        status: DiggingStatus,
        successful: bool,
    ) -> Vec<u8> {
        let mut packet = Vec::new();
        packet.write_u64::<BigEndian>(position.packed()).unwrap();
        put_varint(&mut packet, block_state_id);
        put_varint(&mut packet, status as i32);
        packet.push(u8::from(successful));
        packet
    }

    fn light_update_packet() -> Vec<u8> {
        let mut packet = Vec::new();
        for value in [0, 0, 1, 1 << 1, 1 << 1, 0, 0] {
            put_varint(&mut packet, value);
        }
        put_varint(&mut packet, 2048);
        packet.extend([0x21; 2048]);
        put_varint(&mut packet, 2048);
        packet.extend([0xa5; 2048]);
        packet
    }

    fn multi_block_change_packet(position: BlockPos, state_id: i32) -> Vec<u8> {
        let mut packet = Vec::new();
        packet.extend(position.x.div_euclid(16).to_be_bytes());
        packet.extend(position.z.div_euclid(16).to_be_bytes());
        put_varint(&mut packet, 1);
        packet.push((position.x.rem_euclid(16) << 4 | position.z.rem_euclid(16)) as u8);
        packet.push(position.y.rem_euclid(16) as u8);
        put_varint(&mut packet, state_id);
        packet
    }

    fn explosion_packet() -> Vec<u8> {
        let mut packet = Vec::new();
        for value in [0.5f32, 64.0, 0.5, 1.0] {
            packet.extend(value.to_be_bytes());
        }
        packet.extend(1i32.to_be_bytes());
        packet.extend([0u8, 0, 0]);
        for value in [0.0f32, 0.0, 0.0] {
            packet.extend(value.to_be_bytes());
        }
        packet
    }

    fn respawn_packet() -> Vec<u8> {
        let mut packet = Vec::new();
        put_string(&mut packet, "minecraft:overworld");
        put_string(&mut packet, "world");
        packet.extend(0i64.to_be_bytes());
        packet.extend([0, 0, 0, 0, 0]);
        packet
    }

    #[tokio::test]
    async fn block_geometry_revision_tracks_only_geometry_packets() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        let position = BlockPos { x: 0, y: 0, z: 0 };
        assert_eq!(bot.block_geometry_revision.load(Ordering::Acquire), 0);

        // A malformed chunk never reaches the world mutation or revision gate.
        assert!(bot.apply_packet(0x21, Vec::new()).await.unwrap());
        assert_eq!(bot.block_geometry_revision.load(Ordering::Acquire), 0);

        assert!(
            bot.apply_packet(0x21, empty_chunk_packet(0, 0))
                .await
                .unwrap()
        );
        assert_eq!(bot.block_geometry_revision.load(Ordering::Acquire), 1);

        // Light and block-entity packets mutate their own domains only.
        assert!(bot.apply_packet(0x24, light_update_packet()).await.unwrap());
        let mut block_entity = position.packed().to_be_bytes().to_vec();
        block_entity.push(0);
        assert!(bot.apply_packet(0x09, block_entity).await.unwrap());
        assert_eq!(bot.block_geometry_revision.load(Ordering::Acquire), 1);

        let mut block_change = position.packed().to_be_bytes().to_vec();
        put_varint(&mut block_change, 1);
        assert!(bot.apply_packet(0x0b, block_change).await.unwrap());
        assert_eq!(bot.block_geometry_revision.load(Ordering::Acquire), 2);
        assert!(
            bot.apply_packet(0x0f, multi_block_change_packet(position, 2))
                .await
                .unwrap()
        );
        assert_eq!(bot.block_geometry_revision.load(Ordering::Acquire), 3);
        assert!(
            bot.apply_packet(
                0x07,
                digging_ack_packet(position, 3, DiggingStatus::Finished, true),
            )
            .await
            .unwrap()
        );
        assert_eq!(bot.block_geometry_revision.load(Ordering::Acquire), 4);
        assert!(bot.apply_packet(0x1c, explosion_packet()).await.unwrap());
        assert_eq!(bot.block_geometry_revision.load(Ordering::Acquire), 5);
        assert!(
            bot.apply_packet(0x1d, [0i32.to_be_bytes(), 0i32.to_be_bytes()].concat())
                .await
                .unwrap()
        );
        assert_eq!(bot.block_geometry_revision.load(Ordering::Acquire), 6);
        assert!(
            bot.apply_packet(0x21, empty_chunk_packet(0, 0))
                .await
                .unwrap()
        );
        assert_eq!(bot.block_geometry_revision.load(Ordering::Acquire), 7);
        assert!(bot.apply_packet(0x3a, respawn_packet()).await.unwrap());
        assert_eq!(bot.block_geometry_revision.load(Ordering::Acquire), 8);

        let observation = bot
            .capture_coherent_observation(CoherentObservationRequest {
                max_entities: 0,
                max_events: 0,
                ..CoherentObservationRequest::default()
            })
            .await
            .unwrap();
        assert_eq!(observation.sensor_capture.block_geometry_revision, 8);
        assert_eq!(observation.generation, bot.connection_generation());
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn digging_ack_applies_exact_server_state_before_coherent_capture() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        assert!(
            bot.apply_packet(0x21, empty_chunk_packet(0, 0))
                .await
                .unwrap()
        );
        let position = BlockPos { x: 0, y: 0, z: 0 };

        let successful = bot
            .dispatch_acknowledged(
                bot.operation_context(0),
                AcknowledgedOperation::DigFinish {
                    position,
                    face: BlockFace::North,
                },
            )
            .await
            .unwrap();
        assert!(
            bot.apply_packet(
                0x07,
                digging_ack_packet(position, 0, DiggingStatus::Finished, true),
            )
            .await
            .unwrap()
        );
        assert_eq!(successful.wait().await, DispatchOutcome::Acknowledged);
        let observation = bot
            .capture_coherent_observation(CoherentObservationRequest {
                interest_generation: Some(1),
                interest: vec![BlockPos { x: 0, y: 0, z: 0 }],

                max_entities: 0,
                max_events: 0,
                ..CoherentObservationRequest::default()
            })
            .await
            .unwrap();
        assert_eq!(observation.interest.cells[0].state_id, Some(0));

        let corrective_state = 42;
        bot.admit_operation(
            bot.operation_context(observation.sequence.get()),
            OperationClass::Normal,
        )
        .await
        .unwrap();
        let failed = bot
            .dispatch_acknowledged(
                bot.operation_context(observation.sequence.get()),
                AcknowledgedOperation::DigFinish {
                    position,
                    face: BlockFace::North,
                },
            )
            .await
            .unwrap();
        assert!(
            bot.apply_packet(
                0x07,
                digging_ack_packet(position, corrective_state, DiggingStatus::Finished, false,),
            )
            .await
            .unwrap()
        );
        assert_eq!(failed.wait().await, DispatchOutcome::Rejected);
        let observation = bot
            .capture_coherent_observation(CoherentObservationRequest {
                interest_generation: Some(1),
                interest: vec![BlockPos { x: 0, y: 0, z: 0 }],

                max_entities: 0,
                max_events: 0,
                ..CoherentObservationRequest::default()
            })
            .await
            .unwrap();
        assert_eq!(
            observation.interest.cells[0].state_id,
            Some(corrective_state)
        );

        bot.admit_operation(
            bot.operation_context(observation.sequence.get()),
            OperationClass::Normal,
        )
        .await
        .unwrap();
        let dropped = bot
            .dispatch_acknowledged(
                bot.operation_context(observation.sequence.get()),
                AcknowledgedOperation::DigFinish {
                    position,
                    face: BlockFace::North,
                },
            )
            .await
            .unwrap();
        drop(dropped);
        assert!(
            bot.apply_packet(
                0x07,
                digging_ack_packet(position, 77, DiggingStatus::Finished, true),
            )
            .await
            .unwrap()
        );
        assert_eq!(
            bot.block(position.x, position.y, position.z).await,
            Some(77)
        );

        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn cleanup_api_uses_barrier_and_keeps_local_clear_distinct() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        bot.set_control(ControlState {
            forward: true,
            ..ControlState::default()
        })
        .await;
        assert_eq!(
            bot.dispatch_cleanup(bot.operation_context(0), CleanupOperation::ControlClear,)
                .await
                .unwrap(),
            CleanupDispatchOutcome::AppliedLocally
        );
        assert_eq!(bot.control().await, ControlState::default());

        bot.connection.begin_disconnect().await.unwrap();
        assert_eq!(
            bot.dispatch_cleanup(bot.operation_context(0), CleanupOperation::UseStop,)
                .await
                .unwrap(),
            CleanupDispatchOutcome::Dispatched
        );

        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn furnace_close_cleanup_clears_local_window_without_server_echo() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        let window_id = 2;
        bot.inventory.write().await.open_window = Some(OpenWindow {
            id: window_id,
            window_type: 13,
            title_json: "{}".to_owned(),
            entity_id: None,
            declared_slots: Some(39),
        });
        bot.inventory
            .write()
            .await
            .windows
            .insert(window_id, vec![None; 39]);
        bot.inventory
            .write()
            .await
            .properties
            .insert((window_id, 0), 0);

        assert_eq!(
            bot.dispatch_cleanup(
                bot.operation_context(0),
                CleanupOperation::CloseWindow { window_id },
            )
            .await
            .unwrap(),
            CleanupDispatchOutcome::Dispatched
        );
        assert!(bot.open_window_state().await.is_none());
        let inventory = bot.inventory.read().await;
        assert!(!inventory.windows.contains_key(&window_id));
        assert!(!inventory.properties.keys().any(|(id, _)| *id == window_id));
        drop(inventory);

        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn rotation_dispatch_reflects_exact_pose_only_after_delivery() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        let stale_context = OperationContext {
            generation: ConnectionGeneration::allocate(),
            source_observation_sequence: 1,
        };
        assert_eq!(
            bot.dispatch_rotation(stale_context, 37.5, -22.0, true)
                .await,
            Err(DispatchError::Admission(
                OperationAdmissionError::StaleGeneration,
            ))
        );
        {
            let player = bot.player.lock().await;
            assert_eq!(player.yaw, 0.0);
            assert_eq!(player.pitch, 0.0);
        }

        release.send(()).unwrap();
        assert_eq!(
            bot.dispatch_rotation(bot.operation_context(0), 37.5, -22.0, true)
                .await
                .unwrap(),
            DispatchOutcome::Dispatched
        );
        let player = bot.player.lock().await;
        assert_eq!(player.yaw, 37.5);
        assert_eq!(player.pitch, -22.0);
        drop(player);
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn rotation_dispatch_serializes_with_coherent_capture() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        let gate = bot.coherent_state_gate.clone();
        let guard = gate.lock().await;
        let capture_bot = bot.clone();
        let rotation_bot = bot.clone();
        let capture = tokio::spawn(async move {
            capture_bot
                .capture_coherent_observation(CoherentObservationRequest {
                    entity_radius: 0.0,
                    max_entities: 0,
                    max_events: 0,
                    ..CoherentObservationRequest::default()
                })
                .await
        });
        let rotation = tokio::spawn(async move {
            rotation_bot
                .dispatch_rotation(rotation_bot.operation_context(0), 15.0, -10.0, true)
                .await
        });
        tokio::task::yield_now().await;
        assert!(!capture.is_finished());
        assert!(!rotation.is_finished());
        drop(guard);

        assert!(capture.await.unwrap().is_ok());
        release.send(()).unwrap();
        assert_eq!(
            rotation.await.unwrap().unwrap(),
            DispatchOutcome::Dispatched
        );
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn block_interaction_correlation_is_single_use_bounded_and_actor_owned() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        bot.player.lock().await.entity_id = Some(42);
        let position = BlockPos { x: -3, y: 64, z: 5 };
        let outcome = bot
            .dispatch_operation(
                bot.operation_context(0),
                Operation::BlockInteraction {
                    hand: Hand::Main,
                    position,
                    face: BlockFace::Up,
                    cursor: [0.5, 1.0, 0.5],
                    inside_block: false,
                    sneak: crate::InteractionSneakRequirement::not_required(),
                },
            )
            .await
            .unwrap();
        assert_eq!(outcome, DispatchOutcome::Dispatched);
        // A non-furnace window consumes the one-shot context and cannot leave it
        // available for a later furnace window.
        assert_eq!(bot.connection.observe_furnace_window(12).await, None);
        assert_eq!(bot.connection.observe_furnace_window(13).await, None);
        assert_eq!(
            bot.dispatch_operation(
                bot.operation_context(0),
                Operation::BlockInteraction {
                    hand: Hand::Main,
                    position,
                    face: BlockFace::Up,
                    cursor: [0.5, 1.0, 0.5],
                    inside_block: false,
                    sneak: crate::InteractionSneakRequirement::not_required(),
                },
            )
            .await
            .unwrap(),
            DispatchOutcome::Dispatched
        );
        assert_eq!(
            bot.connection.observe_furnace_window(13).await,
            Some(position)
        );
        assert_eq!(bot.connection.observe_furnace_window(13).await, None);

        // A second unexpired interaction makes the correlation ambiguous; it must
        // fail closed rather than attributing the window to either position.
        let second_position = BlockPos { x: 9, y: 65, z: -4 };
        for (_sequence, position) in [(3, position), (4, second_position)] {
            assert_eq!(
                bot.dispatch_operation(
                    bot.operation_context(0),
                    Operation::BlockInteraction {
                        hand: Hand::Main,
                        position,
                        face: BlockFace::Up,
                        cursor: [0.5, 1.0, 0.5],
                        inside_block: false,
                        sneak: crate::InteractionSneakRequirement::not_required(),
                    },
                )
                .await
                .unwrap(),
                DispatchOutcome::Dispatched
            );
        }
        assert_eq!(bot.connection.observe_furnace_window(13).await, None);

        // The ambiguity is consumed and a later single interaction can correlate
        // exactly once again.
        assert_eq!(
            bot.dispatch_operation(
                bot.operation_context(0),
                Operation::BlockInteraction {
                    hand: Hand::Main,
                    position: second_position,
                    face: BlockFace::Up,
                    cursor: [0.5, 1.0, 0.5],
                    inside_block: false,
                    sneak: crate::InteractionSneakRequirement::not_required(),
                },
            )
            .await
            .unwrap(),
            DispatchOutcome::Dispatched
        );
        assert_eq!(
            bot.connection.observe_furnace_window(13).await,
            Some(second_position)
        );

        // Expired markers are not allowed to explain a later window.
        assert_eq!(
            bot.dispatch_operation(
                bot.operation_context(0),
                Operation::BlockInteraction {
                    hand: Hand::Main,
                    position,
                    face: BlockFace::Up,
                    cursor: [0.5, 1.0, 0.5],
                    inside_block: false,
                    sneak: crate::InteractionSneakRequirement::not_required(),
                },
            )
            .await
            .unwrap(),
            DispatchOutcome::Dispatched
        );
        tokio::time::sleep(Duration::from_secs(2) + Duration::from_millis(20)).await;
        assert_eq!(bot.connection.observe_furnace_window(13).await, None);

        let stale_context = OperationContext {
            generation: ConnectionGeneration::allocate(),
            source_observation_sequence: 7,
        };
        assert_eq!(
            bot.dispatch_operation(
                stale_context,
                Operation::BlockInteraction {
                    hand: Hand::Main,
                    position,
                    face: BlockFace::Up,
                    cursor: [0.5, 1.0, 0.5],
                    inside_block: false,
                    sneak: crate::InteractionSneakRequirement::not_required(),
                },
            )
            .await,
            Err(crate::DispatchError::Admission(
                OperationAdmissionError::StaleGeneration,
            ))
        );
        assert_eq!(bot.connection.observe_furnace_window(13).await, None);
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn local_cancel_becomes_unknown_without_disconnected_event() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (release_tx, release_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut reader, mut writer) = stream.into_split();
            read_packet(&mut reader, None).await.unwrap();
            let (_, login) = read_packet(&mut reader, None).await.unwrap();
            write_packet(
                &mut writer,
                None,
                2,
                &crate::client::login::test_legacy_success(&login),
            )
            .await
            .unwrap();
            release_rx.await.unwrap();
        });
        let bot = Bot::connect(
            Server::new("127.0.0.1", port),
            Player::offline("LocalCancel"),
            Arc::new(crate::SharedChunkStorage::default()),
            ConnectionOptions::default(),
        )
        .await
        .unwrap();
        let observer = bot.clone_internal();
        let mut events = bot.subscribe();
        drop(bot);

        let mut saw_error = false;
        let mut saw_disconnected = false;
        for _ in 0..4 {
            match tokio::time::timeout(Duration::from_secs(1), events.recv()).await {
                Ok(Ok(Event::Error {
                    kind: "connection", ..
                })) => {
                    saw_error = true;
                    break;
                }
                Ok(Ok(Event::Disconnected { .. })) => {
                    saw_disconnected = true;
                    break;
                }
                Ok(Ok(_)) => {}
                Ok(Err(_)) | Err(_) => break,
            }
        }
        assert!(saw_error);
        assert!(!saw_disconnected);
        assert_eq!(
            observer.connection_state(),
            ConnectionState::ConnectionStateUnknown
        );
        release_tx.send(()).unwrap();
        drop(observer);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn terminal_connection_rejects_packet_write() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (observed_tx, observed_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut reader, mut writer) = stream.into_split();
            read_packet(&mut reader, None).await.unwrap();
            let (_, login) = read_packet(&mut reader, None).await.unwrap();
            write_packet(
                &mut writer,
                None,
                2,
                &crate::client::login::test_legacy_success(&login),
            )
            .await
            .unwrap();
            write_packet(&mut writer, None, 0x1a, &[0]).await.unwrap();
            let mut byte = [0_u8; 1];
            let result =
                tokio::time::timeout(Duration::from_millis(100), reader.read(&mut byte)).await;
            let _ = observed_tx.send(result.map(|read| read.unwrap_or_default()));
            release_rx.await.unwrap();
        });
        let bot = Bot::connect(
            Server::new("127.0.0.1", port),
            Player::offline("TerminalWrite"),
            Arc::new(crate::SharedChunkStorage::default()),
            ConnectionOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(
            bot.wait_for_transport_end().await,
            ConnectionState::Disconnected
        );
        assert!(
            bot.respond_resource_pack(ResourcePackStatus::Accepted)
                .await
                .is_err()
        );
        assert!(observed_rx.await.unwrap().is_err());
        release_tx.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn malformed_kick_reason_does_not_commit_clean_disconnect() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        let mut events = bot.subscribe();
        assert!(bot.apply_packet(0x1a, Vec::new()).await.is_err());
        assert_eq!(bot.connection_state(), ConnectionState::Ready);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), events.recv())
                .await
                .is_err()
        );

        bot.connection
            .mark_terminal(TerminalClassification::ConnectionStateUnknown)
            .await;
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn coherent_capture_sequence_is_generation_bound_and_not_a_domain_revision() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        for _ in 0..3 {
            bot.player.lock().await.x += 1.0;
        }
        for value in 0..5 {
            bot.survival.write().await.raining = Some(value % 2 == 0);
        }
        let request = CoherentObservationRequest {
            max_entities: 0,
            max_events: 0,
            ..CoherentObservationRequest::default()
        };
        let first = bot
            .capture_coherent_observation(request.clone())
            .await
            .unwrap();
        let second = bot
            .capture_coherent_observation(request.clone())
            .await
            .unwrap();
        assert_eq!(first.generation, bot.connection_generation());
        assert_eq!(second.generation, first.generation);
        assert_eq!(first.sequence.get(), 1);
        assert_eq!(second.sequence.get(), 2);
        assert_eq!(
            bot.admit_operation(
                bot.operation_context(first.sequence.get()),
                OperationClass::Normal,
            )
            .await,
            Ok(())
        );
        assert_eq!(
            bot.admit_operation(
                bot.operation_context(second.sequence.get()),
                OperationClass::Normal,
            )
            .await,
            Ok(())
        );
        assert_eq!(
            bot.admit_operation(
                bot.operation_context(first.sequence.get()),
                OperationClass::Normal,
            )
            .await,
            Err(OperationAdmissionError::InvalidOperation)
        );
        assert_eq!(bot.player_snapshot().await.revision, 4);
        assert_eq!(bot.survival_snapshot().await.revision, 5);
        assert_eq!(first.interest.unloaded_cells, 0);
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn traversal_facts_capture_is_generation_bound_and_coherent() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        bot.insert_entity(test_entity(7)).await.unwrap();
        let mut known_entity = test_entity(8);
        known_entity.type_id = Some(1);
        known_entity.type_name = Some("armor_stand");
        bot.insert_entity(known_entity).await.unwrap();
        let request = crate::MovementSnapshotRequest {
            expected_generation: bot.connection_generation(),
            region: crate::BlockRegion::new(
                crate::BlockPos { x: 0, y: 0, z: 0 },
                crate::BlockPos { x: 0, y: 0, z: 0 },
            ),
            entity_radius: 0,
            max_entities: 2,
        };
        let gate = bot.coherent_state_gate.clone();
        let guard = gate.lock().await;
        let capture_bot = bot.clone();
        let pending =
            tokio::spawn(async move { capture_bot.capture_movement_snapshot(request).await });
        tokio::task::yield_now().await;
        assert!(!pending.is_finished());
        drop(guard);
        let serialized = pending.await.unwrap().unwrap();
        let first = bot.capture_movement_snapshot(request).await.unwrap();
        let second = bot.capture_movement_snapshot(request).await.unwrap();
        assert_eq!(serialized.generation, request.expected_generation);
        assert_eq!(serialized.sequence.get() + 1, first.sequence.get());
        assert_eq!(first.generation, request.expected_generation);
        assert_eq!(first.generation, second.generation);
        assert_eq!(first.sequence.get() + 1, second.sequence.get());
        assert!(matches!(
            first.blocks.as_slice(),
            [crate::MovementBlock::Unloaded { .. }]
        ));
        assert_eq!(first.entities_omitted, 0);
        assert_eq!(first.entities.len(), 2);
        assert!(first.entities[0].dimensions.is_none());
        assert_eq!(
            first.entities[1].dimensions,
            Some(crate::EntityDimensions {
                width: 0.5,
                height: 1.975,
            })
        );
        assert_eq!(first.inventory.slots.len(), 0);

        let stale = crate::MovementSnapshotRequest {
            expected_generation: crate::ConnectionGeneration::allocate(),
            ..request
        };
        assert!(bot.capture_movement_snapshot(stale).await.is_err());
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn local_pose_metadata_is_retained_in_coherent_observation() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        bot.player.lock().await.entity_id = Some(42);
        let mut packet = Vec::new();
        put_varint(&mut packet, 42);
        packet.push(6);
        put_varint(&mut packet, 18);
        put_varint(&mut packet, 5);
        packet.push(255);
        bot.apply_packet(0x44, packet).await.unwrap();
        let observed = bot
            .capture_coherent_observation(CoherentObservationRequest::default())
            .await
            .unwrap();
        assert_eq!(observed.mining_environment.pose, Some(5));
        assert_eq!(
            observed.mining_environment.eyes_in_water, None,
            "unloaded cells are not dry air"
        );
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn coherent_capture_keeps_requested_block_and_light_evidence_aligned() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        let positions = vec![
            BlockPos { x: -17, y: 3, z: 9 },
            BlockPos { x: 20, y: 64, z: 0 },
        ];
        let observation = bot
            .capture_coherent_observation(CoherentObservationRequest {
                interest_generation: Some(1),
                interest: positions.clone(),
                max_entities: 0,
                max_events: 0,
                ..CoherentObservationRequest::default()
            })
            .await
            .unwrap();
        assert_eq!(observation.interest.cells.len(), positions.len());
        for (cell, position) in observation.interest.cells.iter().zip(positions) {
            assert_eq!(cell.position, position);
            assert_eq!(cell.state_id, None);
            assert_eq!(cell.light, crate::CoherentLightState::Unknown);
        }
        assert_eq!(observation.interest.unloaded_cells, 2);
        assert_eq!(observation.interest.light_unknown_cells, 2);
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn coherent_capture_reports_observed_light_nibbles() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        let mut packet = Vec::new();
        for value in [0, 0, 1, 1 << 1, 1 << 1, 0, 0] {
            put_varint(&mut packet, value);
        }
        put_varint(&mut packet, 2048);
        packet.extend([0x21; 2048]);
        put_varint(&mut packet, 2048);
        packet.extend([0xa5; 2048]);
        assert_eq!(
            bot.world.lock().await.apply_light(&packet, 256).unwrap(),
            (0, 0)
        );
        let observation = bot
            .capture_coherent_observation(CoherentObservationRequest {
                interest_generation: Some(1),
                interest: vec![BlockPos { x: 0, y: 0, z: 0 }],

                entity_radius: 0.0,
                max_entities: 0,
                max_events: 0,
            })
            .await
            .unwrap();
        assert_eq!(observation.interest.cells.len(), 1);
        assert_eq!(
            observation.interest.cells[0].light,
            crate::CoherentLightState::Observed { block: 5, sky: 1 }
        );
        assert_eq!(observation.interest.light_unknown_cells, 0);
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn packet_apply_updates_multiple_domains_before_capture() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        bot.player.lock().await.spawned = false;
        let request = CoherentObservationRequest {
            max_entities: 0,
            max_events: 0,
            ..CoherentObservationRequest::default()
        };
        let before = bot
            .capture_coherent_observation(request.clone())
            .await
            .unwrap();
        assert!(!before.player.spawned);

        let mut join = Vec::new();
        join.write_i32::<BigEndian>(42).unwrap();
        join.extend([1, 0]);
        put_varint(&mut join, 1);
        put_string(&mut join, "minecraft:overworld");
        join.extend([10, 0, 0, 0]); // Complete empty named registry compound.
        put_string(&mut join, "minecraft:overworld");
        put_string(&mut join, "world");
        assert!(bot.apply_packet(0x25, join).await.unwrap());

        let after = bot
            .capture_coherent_observation(request.clone())
            .await
            .unwrap();
        assert!(after.player.spawned);
        assert_eq!(after.player.entity_id, Some(42));
        assert_eq!(after.oxygen_level, Some(20));
        assert_eq!(
            after.survival.dimension.as_deref(),
            Some("minecraft:overworld")
        );
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn packet_apply_and_capture_linearize_without_torn_domains() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        {
            let mut player = bot.player.lock().await;
            player.entity_id = None;
            player.spawned = false;
        }
        {
            let mut survival = bot.survival.write().await;
            survival.dimension = None;
            survival.world_name = None;
        }
        let mut join = Vec::new();
        join.write_i32::<BigEndian>(42).unwrap();
        join.extend([1, 0]);
        put_varint(&mut join, 1);
        put_string(&mut join, "minecraft:overworld");
        join.extend([10, 0, 0, 0]); // Complete empty named registry compound.
        put_string(&mut join, "minecraft:overworld");
        put_string(&mut join, "world");
        let request = CoherentObservationRequest {
            entity_radius: 0.0,
            max_entities: 0,
            max_events: 0,
            ..CoherentObservationRequest::default()
        };

        let gate = bot.coherent_state_gate.clone();
        let guard = gate.lock().await;
        let capture_bot = bot.clone();
        let apply_bot = bot.clone();
        let capture =
            tokio::spawn(async move { capture_bot.capture_coherent_observation(request).await });
        let apply = tokio::spawn(async move { apply_bot.apply_packet(0x25, join).await });
        tokio::task::yield_now().await;
        assert!(!capture.is_finished());
        assert!(!apply.is_finished());
        drop(guard);

        let observation = capture.await.unwrap().unwrap();
        assert!(apply.await.unwrap().unwrap());
        let before = (false, None, None);
        let after = (true, Some(42), Some("minecraft:overworld".to_owned()));
        let observed = (
            observation.player.spawned,
            observation.player.entity_id,
            observation.survival.dimension,
        );
        assert!(observed == before || observed == after);
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn coherent_capture_sorts_and_reports_entity_truncation() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        for id in [3, 1, 2] {
            let mut entity = test_entity(id);
            entity.position = Vec3::default();
            bot.insert_entity(entity).await.unwrap();
        }
        let observation = bot
            .capture_coherent_observation(CoherentObservationRequest {
                entity_radius: 10.0,
                max_entities: 2,
                max_events: 0,
                ..CoherentObservationRequest::default()
            })
            .await
            .unwrap();
        assert_eq!(
            observation
                .entities
                .iter()
                .map(|entity| entity.entity_id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(observation.entities_omitted, 1);
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn coherent_capture_reports_event_queue_and_request_omission() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        {
            let mut queued = bot.observation_events.lock().unwrap();
            queued.events.clear();
            queued.omitted = 0;
        }
        for _ in 0..258 {
            bot.emit(Event::Login);
        }
        let observation = bot
            .capture_coherent_observation(CoherentObservationRequest {
                entity_radius: 0.0,
                max_entities: 0,
                max_events: 2,
                ..CoherentObservationRequest::default()
            })
            .await
            .unwrap();
        assert_eq!(observation.events.len(), 2);
        assert!(
            observation
                .events
                .iter()
                .all(|event| *event == Event::Login)
        );
        assert_eq!(observation.events_omitted, 256);
        assert_eq!(observation.events_queue_omitted, 2);
        assert_eq!(observation.events_request_omitted, 254);
        assert_eq!(
            observation.events_omitted,
            observation
                .events_queue_omitted
                .saturating_add(observation.events_request_omitted)
        );
        let next = bot
            .capture_coherent_observation(CoherentObservationRequest {
                entity_radius: 0.0,
                max_entities: 0,
                max_events: 256,
                ..CoherentObservationRequest::default()
            })
            .await
            .unwrap();
        assert!(next.events.is_empty());
        assert_eq!(next.events_omitted, 0);
        assert_eq!(next.events_queue_omitted, 0);
        assert_eq!(next.events_request_omitted, 0);
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[test]
    fn coherent_capture_rejects_invalid_bounds() {
        let invalid_requests = [
            CoherentObservationRequest {
                entity_radius: f64::NAN,
                ..CoherentObservationRequest::default()
            },
            CoherentObservationRequest {
                max_entities: 513,
                ..CoherentObservationRequest::default()
            },
            CoherentObservationRequest {
                max_events: 257,
                ..CoherentObservationRequest::default()
            },
        ];
        for request in invalid_requests {
            assert_eq!(
                request.validate().unwrap_err().kind(),
                ErrorKind::InvalidInput
            );
        }
    }

    #[test]
    fn large_sparse_interest_and_generation_are_validated() {
        let cells = (0..4607)
            .map(|x| BlockPos { x, y: 0, z: 0 })
            .collect::<Vec<_>>();
        assert!(
            CoherentObservationRequest {
                interest_generation: Some(1),
                interest: cells.clone(),
                ..CoherentObservationRequest::default()
            }
            .validate()
            .is_ok()
        );
        assert_eq!(
            CoherentObservationRequest {
                interest_generation: Some(1),
                interest: vec![BlockPos { x: 1, y: 2, z: 3 }; 2],
                ..CoherentObservationRequest::default()
            }
            .validate()
            .unwrap_err()
            .kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            CoherentObservationRequest {
                interest: vec![BlockPos { x: 1, y: 2, z: 3 }],
                ..CoherentObservationRequest::default()
            }
            .validate()
            .unwrap_err()
            .kind(),
            ErrorKind::InvalidInput
        );
        assert!(CoherentObservationRequest::default().validate().is_ok());
    }

    #[tokio::test]
    async fn sparse_interest_preserves_request_order_and_unknown_counts() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        let requested = vec![
            BlockPos {
                x: -16,
                y: 0,
                z: -16,
            },
            BlockPos { x: 3, y: 4, z: 5 },
            BlockPos {
                x: -1,
                y: -2,
                z: -3,
            },
        ];
        let observation = bot
            .capture_coherent_observation(CoherentObservationRequest {
                entity_radius: 0.0,
                max_entities: 0,
                max_events: 0,
                interest_generation: Some(77),
                interest: requested.clone(),
            })
            .await
            .unwrap();
        assert_eq!(observation.interest.generation, Some(77));
        assert_eq!(
            observation
                .interest
                .cells
                .iter()
                .map(|cell| cell.position)
                .collect::<Vec<_>>(),
            requested
        );
        assert!(observation.interest.cells.iter().all(
            |cell| cell.state_id.is_none() && cell.light == crate::CoherentLightState::Unknown
        ));
        assert_eq!(observation.interest.unloaded_cells, 3);
        assert_eq!(observation.interest.light_unknown_cells, 3);
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn sparse_interest_and_cube_share_packet_state_without_client_selection() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        let mut light_packet = Vec::new();
        for value in [0, 0, 1, 1 << 1, 1 << 1, 0, 0] {
            put_varint(&mut light_packet, value);
        }
        put_varint(&mut light_packet, 2048);
        light_packet.extend([0x21; 2048]);
        put_varint(&mut light_packet, 2048);
        light_packet.extend([0xa5; 2048]);
        assert_eq!(
            bot.world
                .lock()
                .await
                .apply_light(&light_packet, 256)
                .unwrap(),
            (0, 0)
        );
        let position = BlockPos { x: 0, y: 0, z: 0 };
        let mut block_packet = Vec::new();
        block_packet
            .write_u64::<BigEndian>(position.packed())
            .unwrap();
        put_varint(&mut block_packet, 42);
        assert!(bot.apply_packet(0x0b, block_packet).await.unwrap());

        let observation = bot
            .capture_coherent_observation(CoherentObservationRequest {
                entity_radius: 0.0,
                max_entities: 0,
                max_events: 0,
                interest_generation: Some(1),
                interest: vec![position],
            })
            .await
            .unwrap();
        assert_eq!(observation.interest.cells[0].state_id, Some(42));
        assert_eq!(
            observation.interest.cells[0].light,
            crate::CoherentLightState::Unknown
        );
        bot.world
            .lock()
            .await
            .apply_light(&light_packet, 256)
            .unwrap();
        let observation = bot
            .capture_coherent_observation(CoherentObservationRequest {
                entity_radius: 0.0,
                max_entities: 0,
                max_events: 0,
                interest_generation: Some(2),
                interest: vec![position],
            })
            .await
            .unwrap();
        assert_eq!(
            observation.interest.cells[0].light,
            crate::CoherentLightState::Observed { block: 5, sky: 1 }
        );
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn capture_after_terminal_is_fail_closed() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        bot.connection
            .mark_terminal(TerminalClassification::ConnectionStateUnknown)
            .await;
        let observation = bot
            .capture_coherent_observation(CoherentObservationRequest::default())
            .await;
        assert!(observation.is_err());
        assert_eq!(
            bot.connection_state(),
            ConnectionState::ConnectionStateUnknown
        );
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn terminal_capture_race_fails_closed_at_capture_boundary() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        let gate = bot.coherent_state_gate.clone();
        let guard = gate.lock().await;
        let capture_bot = bot.clone();
        let capture = tokio::spawn(async move {
            capture_bot
                .capture_coherent_observation(CoherentObservationRequest::default())
                .await
        });
        tokio::task::yield_now().await;
        bot.connection
            .mark_terminal(TerminalClassification::ConnectionStateUnknown)
            .await;
        tokio::time::timeout(Duration::from_secs(1), async {
            while bot.connection_state() != ConnectionState::ConnectionStateUnknown {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        drop(guard);
        assert!(capture.await.unwrap().is_err());
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn capture_and_physics_tick_are_serialized_by_the_coherent_gate() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        let gate = bot.coherent_state_gate.clone();
        let guard = gate.lock().await;
        let capture_bot = bot.clone();
        let physics_bot = bot.clone();
        let capture = tokio::spawn(async move {
            capture_bot
                .capture_coherent_observation(CoherentObservationRequest {
                    entity_radius: 0.0,
                    max_entities: 0,
                    max_events: 0,
                    ..CoherentObservationRequest::default()
                })
                .await
        });
        let physics = tokio::spawn(async move {
            physics_bot
                .physics_tick(ControlState::default(), false)
                .await
        });
        tokio::task::yield_now().await;
        assert!(!capture.is_finished());
        assert!(!physics.is_finished());
        drop(guard);
        assert!(capture.await.unwrap().is_ok());
        assert!(physics.await.unwrap().is_ok());
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn physics_fraction_scales_input_but_zero_input_preserves_inertia() {
        let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        // Keep the background producer out of this manually clocked physics test.
        bot.teleport_barrier_ticks.store(u8::MAX, Ordering::Release);
        tokio::time::sleep(Duration::from_millis(100)).await;
        bot.apply_packet(0x21, empty_chunk_packet(0, 0))
            .await
            .unwrap();
        {
            let mut world = bot.world.lock().await;
            for y in 62..=64 {
                for x in -1..=1 {
                    for z in -1..=1 {
                        world.set_block_for_test(BlockPos { x, y, z }, if y == 63 { 1 } else { 0 });
                    }
                }
            }
        }
        {
            let mut player = bot.player.lock().await;
            player.x = 0.5;
            player.y = 64.0;
            player.z = 0.5;
            player.yaw = 0.0;
            player.on_ground = true;
        }
        **bot.motion.lock().await = MotionState::default();
        let before = bot.player.lock().await.z;
        bot.physics_tick_with_fraction(
            ControlState {
                forward: true,
                ..ControlState::default()
            },
            false,
            0.25,
        )
        .await
        .unwrap();
        let after_fraction = bot.player.lock().await.z;
        assert!(
            after_fraction > before,
            "fractional input must advance the player: before={before} after={after_fraction} motion={:?}",
            bot.motion.lock().await.velocity
        );
        bot.physics_tick_with_fraction(ControlState::default(), false, 0.0)
            .await
            .unwrap();
        let after_zero_input = bot.player.lock().await.z;
        assert!(
            after_zero_input > after_fraction,
            "clearing input must preserve existing horizontal inertia for this tick"
        );

        // Reset the same real fixture and compare against a full-strength tick.
        {
            let mut player = bot.player.lock().await;
            player.x = 0.5;
            player.y = 64.0;
            player.z = 0.5;
            player.on_ground = true;
            player.yaw = 0.0;
        }
        **bot.motion.lock().await = MotionState::default();
        bot.physics_tick_with_fraction(
            ControlState {
                forward: true,
                ..ControlState::default()
            },
            false,
            1.0,
        )
        .await
        .unwrap();
        let full_delta = bot.player.lock().await.z - before;
        let quarter_delta = after_fraction - before;
        assert!(
            full_delta > 0.0 && (quarter_delta / full_delta - 0.25).abs() < 1e-9,
            "fraction must scale the same initial physical input: quarter={quarter_delta} full={full_delta}"
        );
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    fn test_entity(entity_id: i32) -> EntityState {
        EntityState {
            entity_id,
            uuid: None,
            kind: crate::entity::EntityKind::Object,
            type_id: None,
            type_name: None,
            position: Vec3::default(),
            velocity: Vec3::default(),
            yaw: 0.0,
            pitch: 0.0,
            head_yaw: 0.0,
            on_ground: false,
            object_data: None,
            experience_count: None,
            painting_motive: None,
            painting_direction: None,
            metadata: HashMap::new(),
            equipment: HashMap::new(),
            passengers: Vec::new(),
            attached_to: None,
        }
    }
    pub(super) async fn operation_test_bot(
        outbound_id: i32,
        response_id: i32,
        response: Vec<u8>,
    ) -> (
        Bot,
        tokio::sync::mpsc::UnboundedReceiver<(i32, Vec<u8>)>,
        tokio::sync::oneshot::Sender<()>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (packets, received) = tokio::sync::mpsc::unbounded_channel();
        let (release, released) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut reader, mut writer) = stream.into_split();
            read_packet(&mut reader, None).await.unwrap();
            let (_, login) = read_packet(&mut reader, None).await.unwrap();
            write_packet(
                &mut writer,
                None,
                2,
                &crate::client::login::test_legacy_success(&login),
            )
            .await
            .unwrap();
            let mut released = Some(released);
            while let Ok((id, payload)) = read_packet(&mut reader, None).await {
                packets.send((id, payload.clone())).unwrap();
                let matches = id == outbound_id
                    && (id != 0x1b || payload.first() == Some(&(DiggingStatus::Finished as u8)));
                if matches {
                    if let Some(gate) = released.take() {
                        gate.await.unwrap();
                        write_packet(&mut writer, None, response_id, &response)
                            .await
                            .unwrap();
                    }
                }
            }
            let _ = write_packet(&mut writer, None, 0x1a, &[0]).await;
        });
        let options = ConnectionOptions {
            event_channel_capacity: 2,
            ..ConnectionOptions::default()
        };
        let bot = Bot::connect(
            Server::new("127.0.0.1", port),
            Player::offline("LagProbe"),
            Arc::new(crate::SharedChunkStorage::default()),
            options,
        )
        .await
        .unwrap();
        bot.player.lock().await.spawned = true;
        *bot.positioned.lock().await = true;
        bot.connection.mark_ready().await;
        timeout(Duration::from_secs(1), async {
            while bot.connection_state() != ConnectionState::Ready {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        (bot, received, release, server)
    }

    #[tokio::test]
    async fn immediate_attack_bypasses_charge_and_resets_waiting_normal_attack() {
        let (bot, mut packets, _release, server) = operation_test_bot(0x7f, 0x7f, vec![]).await;
        // Default bare-hand recovery is 250ms.
        *bot.last_attack.lock().await = Some(std::time::Instant::now());
        let normal_bot = bot.clone();
        let normal = tokio::spawn(async move { normal_bot.attack(77).await });
        tokio::time::sleep(Duration::from_millis(40)).await;
        // Measure from admission, not from the caller being scheduled again
        // after dispatch. A delayed continuation must not shorten the measured cooldown.
        let immediate_at = std::time::Instant::now();
        tokio::time::timeout(Duration::from_millis(100), bot.attack_immediate(88))
            .await
            .unwrap()
            .unwrap();
        let first = tokio::time::timeout(Duration::from_millis(100), async {
            loop {
                let packet = packets.recv().await.unwrap();
                if packet.0 == 0x0e {
                    break packet;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(first.1, vec![88, 1, 0]);
        assert!(!normal.is_finished());
        let second = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let packet = packets.recv().await.unwrap();
                if packet.0 == 0x0e {
                    break packet;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(second.1, vec![77, 1, 0]);
        assert!(immediate_at.elapsed() >= Duration::from_millis(230));
        normal.await.unwrap().unwrap();
        bot.disconnect().await.unwrap();
        server.abort();
    }

    #[tokio::test]
    async fn common_revocation_fences_clones_with_capture_and_writer_locked() {
        let (bot, mut packets, release, server) = operation_test_bot(0x7fff, 0, vec![]).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let connection_id = client.player_state().await.unwrap().session.connection_id;
        let capture = bot.coherent_state_gate.lock().await;
        let writer = bot.writer.lock().await;
        let mut queued = Vec::new();
        for _ in 0..32 {
            let sender = bot.clone();
            queued.push(tokio::spawn(async move {
                sender.send_protocol(0x10, &[1]).await
            }));
        }
        tokio::task::yield_now().await;
        let outside = client.clone();
        let revoked = std::thread::spawn(move || {
            crate::client::tests::common_revocation_scenario(
                &outside,
                crate::MinecraftVersion::Java1_16_1,
                connection_id,
            )
        })
        .join()
        .unwrap();
        assert_eq!(revoked.connection_id(), bot.connection_generation().get());
        assert!(bot.is_stopped());
        assert_eq!(
            bot.connection_state(),
            ConnectionState::ConnectionStateUnknown
        );
        for sender in queued {
            assert!(
                timeout(Duration::from_secs(1), sender)
                    .await
                    .unwrap()
                    .unwrap()
                    .is_err()
            );
        }
        // Both locks remain owned until after all queued sends have failed.
        drop(capture);
        drop(writer);
        assert!(client.survival().select_hotbar(0).await.is_err());
        drop(release);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
        while let Some((id, _)) = packets.recv().await {
            assert_ne!(id, 0x10, "revoked queued frame reached the peer");
        }
    }

    #[tokio::test]
    async fn server_slot_event_retains_the_revision_at_application() {
        let (bot, _packets, release, server) =
            operation_test_bot(0x2c, 0x16, vec![0, 0, 36, 0]).await;
        let mut events = bot.subscribe();
        let before = bot.inventory_snapshot().await;
        bot.send(0x2c, &[0]).await.unwrap();
        release.send(()).unwrap();
        let packet = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(Event::InventorySlotObserved(sample)) = events.recv().await {
                    break sample;
                }
            }
        })
        .await
        .unwrap();
        assert!(packet.revision > before.revision);
        assert_eq!(packet.revision, bot.inventory_snapshot().await.revision);
        assert_eq!(packet.value.slot, 36);
        assert_eq!(packet.value.item, None);
        bot.inventory.write().await.windows.insert(9, vec![]);
        assert!(bot.inventory_snapshot().await.revision > packet.revision);
        bot.disconnect().await.unwrap();
        server.abort();
    }

    #[tokio::test]
    async fn digging_visibility_accepts_a_crop_without_ignoring_occluders() {
        let (bot, _packets, _release, server) = operation_test_bot(0x7f, 0x7f, vec![]).await;
        let target = BlockPos { x: 0, y: 1, z: 3 };
        {
            let mut p = bot.player.lock().await;
            p.x = 0.5;
            p.y = 0.;
            p.z = 0.5;
        }
        {
            let mut world = bot.world.lock().await;
            world.apply_chunk(&[0; 14], 256).unwrap();
            let wheat = (0..18000)
                .find(|id| crate::block_name_from_state(*id) == Some("wheat"))
                .unwrap();
            let mut change = target.packed().to_be_bytes().to_vec();
            put_varint(&mut change, wheat);
            world.apply_block_change(&change).unwrap();
        }
        assert!(
            bot.digging_info(target).await.visible,
            "collision-free crop is a visible digging target"
        );
        {
            let mut world = bot.world.lock().await;
            let mut change = BlockPos { x: 0, y: 1, z: 2 }
                .packed()
                .to_be_bytes()
                .to_vec();
            put_varint(&mut change, 1);
            world.apply_block_change(&change).unwrap();
        }
        assert!(
            !bot.can_see_block(target).await,
            "solid intervening block still occludes the crop"
        );
        bot.disconnect().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn timed_dig_survives_event_lag_and_still_requires_matching_ack() {
        let target = BlockPos { x: 0, y: 1, z: 0 };
        let mut response = target.packed().to_be_bytes().to_vec();
        put_varint(&mut response, 0);
        put_varint(&mut response, DiggingStatus::Finished as i32);
        response.push(1);
        let (bot, mut packets, release, server) = operation_test_bot(0x1b, 0x07, response).await;
        {
            let mut world = bot.world.lock().await;
            world.apply_chunk(&[0; 14], 256).unwrap();
            let mut change = target.packed().to_be_bytes().to_vec();
            let log = (0..18000)
                .find(|id| crate::block_name_from_state(*id) == Some("oak_log"))
                .unwrap();
            put_varint(&mut change, log);
            world.apply_block_change(&change).unwrap();
        }
        let worker = bot.clone();
        let digging = tokio::spawn(async move { worker.dig_block(target, BlockFace::Up).await });
        loop {
            let (id, payload) = packets.recv().await.unwrap();
            if id == 0x1b && payload.first() == Some(&(DiggingStatus::Started as u8)) {
                break;
            }
        }
        // The receiver is not polled during the dig timer. Overflow it before
        // the server can acknowledge Finished, reproducing natural-world lag.
        for x in 0..100 {
            bot.emit(Event::ChunkLoaded { x, z: 0 });
        }
        release.send(()).unwrap();
        let ack = timeout(Duration::from_secs(8), digging)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(ack.successful);
        assert_eq!(ack.position, target);
        bot.disconnect().await.unwrap();
        server.await.unwrap();
    }

    async fn lagged_click(accepted: bool) -> Result<i16> {
        // Confirm Transaction: i8 window + i16 action + bool accepted.
        let response = vec![0, 0, 1, u8::from(accepted)];
        let (bot, mut packets, release, server) = operation_test_bot(0x09, 0x12, response).await;
        bot.inventory
            .write()
            .await
            .windows
            .insert(0, vec![None; 46]);
        let worker = bot.clone();
        let clicking =
            tokio::spawn(
                async move { worker.click_slot_and_wait(0, 9, 0, ClickMode::Normal).await },
            );
        while packets.recv().await.unwrap().0 != 0x09 {}
        for x in 0..100 {
            bot.emit(Event::ChunkLoaded { x, z: 0 });
        }
        release.send(()).unwrap();
        let result = timeout(Duration::from_secs(2), clicking)
            .await
            .unwrap()
            .unwrap();
        bot.disconnect().await.unwrap();
        server.await.unwrap();
        result
    }

    #[tokio::test]
    async fn accepted_click_survives_unrelated_event_lag() {
        assert_eq!(lagged_click(true).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn rejected_click_after_event_lag_is_still_rejected() {
        assert_eq!(
            lagged_click(false).await.unwrap_err().kind(),
            crate::ErrorKind::Rejected
        );
    }

    #[tokio::test]
    async fn operation_wait_reports_disconnect_and_closed_channel() {
        let (sender, mut receiver) = broadcast::channel(2);
        sender
            .send(Event::Disconnected {
                reason: "test disconnect".into(),
            })
            .unwrap();
        assert_eq!(
            next_operation_event(&mut receiver)
                .await
                .unwrap_err()
                .kind(),
            crate::ErrorKind::Disconnected
        );
        let (sender, mut receiver) = broadcast::channel::<Event>(2);
        drop(sender);
        assert_eq!(
            next_operation_event(&mut receiver)
                .await
                .unwrap_err()
                .kind(),
            crate::ErrorKind::Disconnected
        );
    }
    #[test]
    fn explosion_offsets_are_relative_to_floored_center() {
        let mut packet = Vec::new();
        for value in [10.75f32, 64.5, -2.25, 4.0] {
            packet.extend(value.to_be_bytes());
        }
        packet.extend(2i32.to_be_bytes());
        packet.extend([1u8, 0, 255, 255, 2, 0]);
        for value in [0.1f32, 0.2, 0.3] {
            packet.extend(value.to_be_bytes());
        }
        let event = parse_explosion(&packet).unwrap();
        assert_eq!(
            event.affected_blocks,
            vec![
                BlockPos {
                    x: 11,
                    y: 64,
                    z: -4
                },
                BlockPos { x: 9, y: 66, z: -3 }
            ]
        );
        assert!((event.player_motion.y - 0.2).abs() < 1.0e-6);
    }

    #[test]
    fn explosion_rejects_non_finite_coordinates() {
        let mut packet = Vec::new();
        for value in [f32::NAN, 64.0, 0.0, 4.0] {
            packet.extend(value.to_be_bytes());
        }
        packet.extend(0_i32.to_be_bytes());
        for value in [0.0_f32, 0.0, 0.0] {
            packet.extend(value.to_be_bytes());
        }
        assert!(parse_explosion(&packet).is_err());
    }

    #[tokio::test]
    async fn dropping_last_bot_handle_closes_connection_tasks() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut reader, mut writer) = stream.into_split();
            read_packet(&mut reader, None).await.unwrap();
            let (_, login) = read_packet(&mut reader, None).await.unwrap();
            write_packet(
                &mut writer,
                None,
                2,
                &crate::client::login::test_legacy_success(&login),
            )
            .await
            .unwrap();
            let mut byte = [0_u8; 1];
            tokio::time::timeout(Duration::from_secs(2), reader.read_exact(&mut byte))
                .await
                .expect("client tasks retained the connection")
                .expect_err("client unexpectedly wrote play data");
        });
        let bot = Bot::connect(
            Server::new("127.0.0.1", port),
            Player::offline("DropProbe"),
            Arc::new(crate::versions::java_1_16_1::SharedChunkStorage::default()),
            ConnectionOptions::default(),
        )
        .await
        .unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn entity_limit_rejects_new_entity_without_mutation() {
        let options = ConnectionOptions {
            max_entities: 0,
            ..ConnectionOptions::default()
        };
        let (bot, server, release) = connected_test_bot(options, Vec::new()).await;
        assert!(bot.insert_entity(test_entity(1)).await.is_err());
        assert!(bot.entities.read().await.entities.is_empty());
        release.send(()).unwrap();
        drop(bot);
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn aggregate_cache_limit_is_observable() {
        let options = ConnectionOptions {
            max_cached_records: 1,
            ..ConnectionOptions::default()
        };
        let (bot, server, release) = connected_test_bot(options, Vec::new()).await;
        bot.players.write().await.entries.insert(
            [0; 16],
            crate::chat::PlayerListEntry {
                uuid: [0; 16],
                name: "one".to_owned(),
                properties: Vec::new(),
                game_mode: 0,
                latency: 0,
                display_name_json: None,
            },
        );
        assert!(bot.enforce_session_limits().await.is_ok());
        bot.players.write().await.entries.insert(
            [1; 16],
            crate::chat::PlayerListEntry {
                uuid: [1; 16],
                name: "two".to_owned(),
                properties: Vec::new(),
                game_mode: 0,
                latency: 0,
                display_name_json: None,
            },
        );
        assert!(bot.enforce_session_limits().await.is_err());
        release.send(()).unwrap();
        drop(bot);
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn custom_payload_limit_rejects_before_event_emission() {
        let mut payload = Vec::new();
        put_string(&mut payload, "minecraft:test");
        payload.extend([1, 2, 3]);
        let options = ConnectionOptions {
            max_custom_payload_bytes: 2,
            ..ConnectionOptions::default()
        };
        let (bot, server, release) = connected_test_bot(options, vec![(0x18, payload)]).await;
        let mut events = bot.subscribe();
        release.send(()).unwrap();
        let event = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Event::Error { kind, message } = events.recv().await.unwrap() {
                    break (kind, message);
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(event.0, "connection");
        assert!(event.1.contains("custom payload"));
        assert_eq!(
            bot.wait_for_transport_end().await,
            ConnectionState::ConnectionStateUnknown
        );
        drop(bot);
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn aggregate_limit_error_makes_generation_unknown() {
        let options = ConnectionOptions {
            max_cached_records: 0,
            ..ConnectionOptions::default()
        };
        let (bot, server, release) = connected_test_bot(options, vec![(0x3f, vec![0])]).await;
        bot.players.write().await.entries.insert(
            [0; 16],
            crate::chat::PlayerListEntry {
                uuid: [0; 16],
                name: "one".to_owned(),
                properties: Vec::new(),
                game_mode: 0,
                latency: 0,
                display_name_json: None,
            },
        );
        let mut events = bot.subscribe();
        release.send(()).unwrap();
        let event = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Event::Error { kind, message } = events.recv().await.unwrap() {
                    break (kind, message);
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(event.0, "connection");
        assert!(event.1.contains("session cache"));
        assert_eq!(
            bot.wait_for_transport_end().await,
            ConnectionState::ConnectionStateUnknown
        );
        drop(bot);
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn event_receiver_reports_lag_instead_of_unbounded_growth() {
        let options = ConnectionOptions {
            event_channel_capacity: 2,
            ..ConnectionOptions::default()
        };
        let (bot, server, release) = connected_test_bot(options, Vec::new()).await;
        let mut events = bot.subscribe();
        bot.emit(Event::Login);
        bot.emit(Event::Login);
        bot.emit(Event::Login);
        assert!(matches!(
            events.recv().await,
            Err(broadcast::error::RecvError::Lagged(1))
        ));
        release.send(()).unwrap();
        drop(bot);
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
}
