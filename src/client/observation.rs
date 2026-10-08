//! Common observations. Missing, received and locally computed values stay distinct.
use super::registry::{Registry, RegistryId};
use crate::{MinecraftVersion, Result};

/// Received game mode. Choosing an operations handle does not change it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameMode {
    /// Ordinary survival mode.
    Survival,
    /// Server-granted creative mode.
    Creative,
    /// Adventure mode; neither normal mode handle grants its permissions.
    Adventure,
    /// Spectator mode.
    Spectator,
}
impl GameMode {
    pub(crate) fn decode(id: u8) -> anyhow::Result<Self> {
        Ok(match id {
            0 => Self::Survival,
            1 => Self::Creative,
            2 => Self::Adventure,
            3 => Self::Spectator,
            _ => anyhow::bail!("invalid game mode"),
        })
    }
}
/// Identity of a transport and its current world. Not a server tick or action authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct SessionStamp {
    /// Exact native registry version.
    pub version: MinecraftVersion,
    /// Process-local connection identity, meaningful only within the owning run.
    pub connection_id: u64,
    /// Native reset boundary; changes on login/respawn/reconfiguration.
    pub world_generation: u64,
}
/// Received login UUID/name, captured with the current connection/world stamp.
/// These fields originate in LOGIN_SUCCESS, outside the play receive ordinal stream.
/// They are profile facts, not a recovery fence or permission to restore a saved job.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ConnectionIdentity {
    /// Owning connection and world at capture.
    pub session: SessionStamp,
    /// Exact received UUID bytes; never calculated from the requested name.
    pub uuid: [u8; 16],
    /// Exact received login name.
    pub name: String,
}
/// Origin of a value. None is represented by absence, never an invented default.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ValueSource {
    /// Native packet applied to this session.
    Received {
        /// Packet ordinal, not a tick.
        sequence: u64,
    },
    /// Submitted locally; transport completion is not server acceptance.
    Submitted,
    /// Computed by the version-specific client model.
    Predicted,
    /// Compatibility cache may contain acknowledged click predictions.
    LocalCache,
}
/// A value with its source kept alongside it.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ObservedValue<T> {
    /// Value at capture.
    pub value: T,
    /// Where the value came from.
    pub source: ValueSource,
}
crate::diagnostic_projection::diagnostic_record! {
/// A complete encoded value of one native item data component.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ItemComponent => RecordedItemComponent {
    /// Exact version-bound type, distinct from item or block-state identity.
    pub definition: super::registry::ItemComponentDefinition,
    /// Native value bytes, excluding the type ID. Empty is valid for unit components.
    pub bytes: Vec<u8>,
}
    diagnostic_serde {}
}

crate::diagnostic_projection::diagnostic_record! {
/// A lossless native item-component patch against the owning item's prototype.
/// Lists retain original wire ordering; removed types are not empty values.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ItemComponentPatch => RecordedItemComponentPatch {
    /// Added or replaced values; an empty list does not mean an empty item.
    pub added: Vec<ItemComponent>,
    /// Components explicitly removed from the native item prototype.
    pub removed: Vec<super::registry::ItemComponentDefinition>,
}
    diagnostic_serde {}
}

impl PartialEq for ItemComponentPatch {
    fn eq(&self, other: &Self) -> bool {
        // Native patches are maps/sets. Keep wire order for inspection without
        // treating a reordered same-value receipt as a changed item.
        let mut added = self.added.iter().collect::<Vec<_>>();
        let mut other_added = other.added.iter().collect::<Vec<_>>();
        added.sort_by_key(|c| c.definition.id.value());
        other_added.sort_by_key(|c| c.definition.id.value());
        let mut removed = self.removed.iter().collect::<Vec<_>>();
        let mut other_removed = other.removed.iter().collect::<Vec<_>>();
        removed.sort_by_key(|c| c.id.value());
        other_removed.sort_by_key(|c| c.id.value());
        added == other_added && removed == other_removed
    }
}
impl Eq for ItemComponentPatch {}
crate::diagnostic_projection::diagnostic_record! {
/// Item data whose interpretation belongs to the owning version.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ItemData => RecordedItemData {
    /// No legacy NBT or an empty modern patch; native prototypes may still contain data.
    Default,
    /// Complete legacy NBT, including the root tag byte. Not modern components.
    LegacyNbt {
        /// Native encoded bytes.
        bytes: Vec<u8>,
    },
    /// Complete supported modern patch. This is not legacy NBT or a resolved prototype.
    ModernComponents {
        /// Added values and explicit removals, including exact encoded native data.
        patch: ItemComponentPatch,
    },
}
diagnostic_serde { #[serde(tag = "kind", rename_all = "snake_case")] }
}
crate::diagnostic_projection::diagnostic_record! {
/// A common stack identity with lossless supported native data.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ItemStack => RecordedItemStack {
    /// ID bound to this stack's adapter.
    pub id: RegistryId,
    /// Namespaced item name.
    pub name: String,
    /// Positive count from the native packet or cache.
    pub count: u32,
    /// Supported native item data; unsupported stacks remain unavailable.
    pub data: ItemData,
}
diagnostic_serde {  }
}
impl ItemStack {
    /// Decode common custom metadata while preserving the original ItemData bytes.
    /// Legacy returns its complete tag, including conventional item fields;
    /// modern returns the custom_data component. All pinned modern default item
    /// prototypes have no custom_data, verified from the original 1,505 items.
    /// This read-only value is not a normalized complete item or action authority.
    ///
    /// ```no_run
    /// use voxrig::client::prelude::*;
    /// fn inspect(item: &ItemStack) -> Result<()> {
    ///     if let Some(data) = item.custom_data()? {
    ///         if let Some(marker) = data.root().get("VoxrigProbe").and_then(NbtValue::as_int) {
    ///             println!("marker = {marker}");
    ///         }
    ///     }
    ///     Ok(())
    /// }
    /// ```
    pub fn custom_data(&self) -> Result<Option<super::nbt::NbtData>> {
        use super::registry::invalid;
        let registry = Registry::for_version(self.id.version());
        if registry.item_definition(self.id)?.name != self.name {
            return Err(invalid("item name and ID disagree"));
        }
        match (self.id.version(), &self.data) {
            (_, ItemData::Default) => Ok(None),
            (MinecraftVersion::Java1_16_1, ItemData::LegacyNbt { bytes }) => {
                super::nbt::decode(bytes, self.id.version()).map(Some)
            }
            (MinecraftVersion::Java1_21_11, ItemData::ModernComponents { patch }) => {
                let mut seen = std::collections::BTreeSet::new();
                let mut custom = None;
                for component in &patch.added {
                    if registry.item_component_definition(component.definition.id)?
                        != component.definition
                        || !seen.insert(component.definition.id.value())
                    {
                        return Err(invalid("invalid or duplicate component identity"));
                    }
                    if component.definition.name == "minecraft:custom_data" {
                        custom = Some(&component.bytes);
                    }
                }
                for removed in &patch.removed {
                    if registry.item_component_definition(removed.id)? != *removed
                        || !seen.insert(removed.id.value())
                    {
                        return Err(invalid("invalid or duplicate removed component identity"));
                    }
                }
                custom
                    .map(|bytes| super::nbt::decode(bytes, self.id.version()))
                    .transpose()
            }
            _ => Err(invalid(
                "item data representation belongs to another adapter",
            )),
        }
    }
}
crate::diagnostic_projection::diagnostic_record! {
/// Knowledge of one inventory slot.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SlotKnowledge => RecordedSlotKnowledge {
    /// No complete supported value is known.
    Unavailable,
    /// Explicitly empty, with source carried by the surrounding value.
    Empty,
    /// Known stack.
    Item {
        /// Exact stack identity and data.
        item: ItemStack,
    },
}
diagnostic_serde { #[serde(tag = "kind", rename_all = "snake_case")] }
}
/// Inventory at one capture boundary. Slot indices use player-screen layout 0..45.
#[derive(Clone, Debug, serde::Serialize)]
pub struct InventoryObservation {
    /// Received slots; unavailable until individually or fully established.
    pub slots: Vec<Option<ObservedValue<SlotKnowledge>>>,
    /// Received carried stack, not inferred from a click prediction.
    pub cursor: Option<ObservedValue<SlotKnowledge>>,
    /// Received active container ID, when established.
    pub window_id: Option<i32>,
    /// Player UI access, independent of the last received window ID.
    pub player_screen: Option<super::container::PlayerScreenAccess>,
    /// Received revision if the selected protocol supplies one.
    pub screen_revision: Option<i32>,
    /// Last actual player-screen-zero revision and packet ordinal, including
    /// before a foreign screen opening. Not a foreign menu revision or ACK.
    pub player_screen_revision: Option<ObservedValue<i32>>,
    /// Legacy compatibility cache, which may include local click predictions.
    /// Kept separate from received values; absent when no such cache exists.
    pub local_cache: Option<Vec<SlotKnowledge>>,
}
impl Default for InventoryObservation {
    fn default() -> Self {
        Self {
            slots: vec![None; 46],
            cursor: None,
            window_id: None,
            player_screen: None,
            screen_revision: None,
            player_screen_revision: None,
            local_cache: None,
        }
    }
}
/// Last supported health/hunger packet.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Health {
    /// Native health points.
    pub health: f32,
    /// Native food points.
    pub food: i32,
    /// Native saturation.
    pub saturation: f32,
}
/// Last position packet, retained separately from current predicted/submitted position.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ReceivedPose {
    /// Resolved feet position.
    pub position: [f64; 3],
    /// Resolved yaw/pitch.
    pub rotation: [f32; 2],
    /// Packet ordinal.
    pub receive_sequence: u64,
}
/// Current dimension metadata. Never silently clamped to another version's height.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Dimension {
    /// Received dimension name.
    pub name: String,
    /// Inclusive minimum Y.
    pub min_y: i32,
    /// Number of block levels; exclusive maximum is min_y + height.
    pub height: i32,
}
/// Player and inventory captured under one adapter lock boundary.
#[derive(Clone, Debug, serde::Serialize)]
pub struct PlayerObservation {
    /// Owning connection/world.
    pub session: SessionStamp,
    /// Last applied receive ordinal, not proof of freshness for every field.
    pub receive_sequence: u64,
    /// A common dispatch was interrupted or is still unresolved; do not replay it.
    pub pending_dispatch: bool,
    /// Current dimension, absent during setup/reset.
    pub dimension: Option<Dimension>,
    /// Current local position and its basis.
    pub position: Option<ObservedValue<[f64; 3]>>,
    /// Last actual position packet in this world.
    pub received_pose: Option<ReceivedPose>,
    /// Current local rotation; may reflect submitted look or physics.
    pub rotation: [f32; 2],
    /// Independent origin of `rotation`; absent before a supported update in this world.
    /// Equal numeric values do not establish a packet receipt.
    pub rotation_source: Option<ValueSource>,
    /// Current native model's ground flag at this same capture boundary.
    /// Absence is not false. Corrections/reset invalidate it until a new supported
    /// model update; this is never an own-player server ground receipt.
    pub on_ground: Option<ObservedValue<bool>>,
    /// Latest received mode.
    pub game_mode: Option<GameMode>,
    /// Last received flight permission; None before any abilities packet.
    pub may_fly: Option<bool>,
    /// Health is absent until received, never inferred from mode.
    pub health: Option<ObservedValue<Health>>,
    /// Main-hand selection, distinguishing packet from local dispatch.
    pub selected_hotbar: Option<ObservedValue<u8>>,
    /// Received inventory and optional compatibility cache.
    pub inventory: InventoryObservation,
    /// Item use from the own player's received living-entity flags: `Some(hand)` while the
    /// server reports an item in use, `None` when it reports none. Absent until the first
    /// flags packet in this world; a sent use request is never shown here.
    pub using_item: Option<ObservedValue<Option<super::Hand>>>,
    /// Own native entity ID in this world, from the login or respawn handshake.
    pub entity_id: Option<i32>,
    /// Received attributes by 1.21.11 name (`minecraft:attack_speed`; 1.16.1 keys are
    /// renamed). Absent until received in this world, even where the official client
    /// keeps a value across a respawn.
    pub attributes: std::collections::BTreeMap<String, ObservedValue<super::PlayerAttribute>>,
    /// Received status effects by name, until a removal is received. Durations are
    /// as received; no local countdown or expiry is applied.
    pub effects: std::collections::BTreeMap<String, ObservedValue<super::PlayerEffect>>,
    /// Received air supply in ticks (300 when full).
    pub air_supply: Option<ObservedValue<i32>>,
    /// Latest received world time sample. Servers send one about every 20 ticks, so
    /// the current game time can be ahead of it by that much.
    pub world_time: Option<ObservedValue<WorldTime>>,
}

/// World clock as received from the server's periodic time packet.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct WorldTime {
    /// Game time in ticks (`Level.getGameTime`); compare with
    /// [`super::EntityDataField::AngerEndTime`].
    pub game_time: i64,
    /// Time of day in ticks, as received (negative when the daylight cycle is stopped
    /// on Java 1.16.1).
    pub day_time: i64,
}

/// Player, inventory and received region at one adapter capture boundary.
/// A capture records knowledge; it does not authorize replay or certify server outcomes.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Capture {
    /// Player/inventory at the same boundary as world cells.
    pub player: PlayerObservation,
    /// Complete bounded region, preserving unloaded cells.
    pub world: crate::Observation,
}

pub(crate) fn legacy_slot(
    stack: Option<&crate::versions::java_1_16_1::ItemStack>,
) -> Result<SlotKnowledge> {
    let Some(stack) = stack else {
        return Ok(SlotKnowledge::Empty);
    };
    let definition =
        Registry::for_version(MinecraftVersion::Java1_16_1).item_by_native_id(stack.item_id)?;
    let count = u32::try_from(stack.count)
        .ok()
        .filter(|count| *count > 0)
        .ok_or_else(|| super::registry::invalid("invalid native item count"))?;
    Ok(SlotKnowledge::Item {
        item: ItemStack {
            id: definition.id,
            name: definition.name,
            count,
            data: stack
                .nbt
                .as_ref()
                .map_or(ItemData::Default, |bytes| ItemData::LegacyNbt {
                    bytes: bytes.clone(),
                }),
        },
    })
}
pub(crate) fn received<T>(value: T, sequence: u64) -> ObservedValue<T> {
    ObservedValue {
        value,
        source: ValueSource::Received { sequence },
    }
}

#[derive(Default)]
pub(crate) struct LegacyReceipts {
    pub entities: super::entity::SpawnLedger,
    pub vehicles: super::vehicle::PassengerLedger,
    pub registries: super::registry::received::ReceivedRegistries,
    pub recipes: super::crafting::recipes::RecipeReceipts,
    pub recipe_ghost: Option<super::crafting::ghost::GhostReceipts>,
    pub container: Option<super::container::ScreenReceipts>,
    pub generation: u64,
    pub pose: Option<ReceivedPose>,
    pub position_source: Option<ValueSource>,
    pub rotation_source: Option<ValueSource>,
    pub ground_source: Option<ValueSource>,
    pub health: Option<ObservedValue<Health>>,
    pub may_fly: Option<bool>,
    pub selected_hotbar: Option<ObservedValue<u8>>,
    pub inventory: InventoryObservation,
    pub player_starts: std::collections::HashMap<i8, usize>,
    pub requested_flying: bool,
    pub abilities: Option<ObservedValue<u8>>,
    pub pending_dispatch: bool,
    pub using_item: Option<ObservedValue<Option<super::Hand>>>,
    pub attributes: std::collections::BTreeMap<String, ObservedValue<super::PlayerAttribute>>,
    pub effects: std::collections::BTreeMap<String, ObservedValue<super::PlayerEffect>>,
    pub air_supply: Option<ObservedValue<i32>>,
    pub world_time: Option<ObservedValue<WorldTime>>,
    pub death_message: Option<ObservedValue<super::ui::UiText>>,
    pub disconnect_reason: Option<super::ui::UiText>,
}
impl LegacyReceipts {
    pub fn window_items(
        &mut self,
        window: i8,
        slots: &[Option<crate::versions::java_1_16_1::ItemStack>],
        sequence: u64,
    ) -> Result<()> {
        let decoded = slots
            .iter()
            .map(|slot| legacy_slot(slot.as_ref()).map(|slot| received(slot, sequence)))
            .collect::<Result<Vec<_>>>()?;
        if window == 0 {
            if self.container.is_none() {
                self.inventory.window_id = Some(0);
            }
            self.inventory.slots = vec![None; 46];
            for (out, slot) in self.inventory.slots.iter_mut().zip(decoded) {
                *out = Some(slot);
            }
        } else if self
            .container
            .as_ref()
            .is_some_and(|screen| screen.window == i32::from(window))
        {
            let screen = self.container.as_mut().expect("matching opening");
            screen.full_items(decoded.into_iter().map(Some).collect(), None, sequence)?;
            if let Some(layout) = &screen.layout {
                if layout.player_slots.len() == 36 {
                    if let Some(first) = layout.player_slots.iter().map(|m| m.screen_slot).min() {
                        self.player_starts.insert(window, first);
                    }
                }
                for mapping in &layout.player_slots {
                    self.inventory.slots[mapping.player_slot] =
                        screen.slots[mapping.screen_slot].clone();
                }
            }
        }
        Ok(())
    }
    pub fn slot(&mut self, update: &crate::versions::java_1_16_1::SlotUpdate) -> Result<()> {
        let value = received(legacy_slot(update.item.as_ref())?, update.packet_sequence);
        if update.window_id == -1 && update.slot == -1 {
            self.inventory.cursor = Some(value);
            return Ok(());
        }
        let Ok(slot) = usize::try_from(update.slot) else {
            return Ok(());
        };
        let index = if update.window_id == -2 {
            crate::versions::java_1_16_1::inventory::player_inventory_slot(update.slot)
        } else if update.window_id == 0 {
            Some(slot)
        } else if self
            .container
            .as_ref()
            .is_some_and(|screen| screen.window == i32::from(update.window_id))
        {
            self.player_starts
                .get(&update.window_id)
                .and_then(|start| slot.checked_sub(*start))
                .filter(|i| *i < 36)
                .map(|i| 9 + i)
        } else {
            None
        };
        if let Some(screen) = &mut self.container {
            if update.window_id > 0 && screen.window == i32::from(update.window_id) {
                screen.slot(slot, Some(value.clone()), None, update.packet_sequence)?;
                if let Some(mapping) = screen
                    .layout
                    .as_ref()
                    .and_then(|layout| layout.player_slots.iter().find(|m| m.screen_slot == slot))
                {
                    self.inventory.slots[mapping.player_slot] = Some(value.clone());
                }
            } else if update.window_id == -2 {
                if let Some(screen_slot) = index.and_then(|i| {
                    screen
                        .layout
                        .as_ref()
                        .and_then(|layout| layout.player_slots.iter().find(|m| m.player_slot == i))
                        .map(|m| m.screen_slot)
                }) {
                    screen.slot(
                        screen_slot,
                        Some(value.clone()),
                        None,
                        update.packet_sequence,
                    )?;
                }
            }
        }
        if let Some(out) = index.and_then(|i| self.inventory.slots.get_mut(i)) {
            *out = Some(value);
        }
        Ok(())
    }
}
