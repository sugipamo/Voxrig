//! Common observations. Missing, received and locally computed values stay distinct.
use super::registry::{Registry, RegistryId};
use crate::{MinecraftVersion, Result};

/// Received game mode. Choosing an operations handle does not change it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
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
/// Origin of a value. None is represented by absence, never an invented default.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
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
/// Item data whose interpretation belongs to the owning version.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ItemData {
    /// No non-default data.
    Default,
    /// Complete legacy NBT, including the root tag byte. Not modern components.
    LegacyNbt {
        /// Native encoded bytes.
        bytes: Vec<u8>,
    },
}
/// A common stack identity with lossless supported native data.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ItemStack {
    /// ID bound to this stack's adapter.
    pub id: RegistryId,
    /// Namespaced item name.
    pub name: String,
    /// Positive count from the native packet or cache.
    pub count: u32,
    /// Supported native item data; unsupported stacks remain unavailable.
    pub data: ItemData,
}
/// Knowledge of one inventory slot.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SlotKnowledge {
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
/// Inventory at one capture boundary. Slot indices use player-screen layout 0..45.
#[derive(Clone, Debug, serde::Serialize)]
pub struct InventoryObservation {
    /// Received slots; unavailable until individually or fully established.
    pub slots: Vec<Option<ObservedValue<SlotKnowledge>>>,
    /// Received carried stack, not inferred from a click prediction.
    pub cursor: Option<ObservedValue<SlotKnowledge>>,
    /// Received active container ID, when established.
    pub window_id: Option<i32>,
    /// Received revision if the selected protocol supplies one.
    pub screen_revision: Option<i32>,
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
            screen_revision: None,
            local_cache: None,
        }
    }
}
/// Last supported health/hunger packet.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct Health {
    /// Native health points.
    pub health: f32,
    /// Native food points.
    pub food: i32,
    /// Native saturation.
    pub saturation: f32,
}
/// Last position packet, retained separately from current predicted/submitted position.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ReceivedPose {
    /// Resolved feet position.
    pub position: [f64; 3],
    /// Resolved yaw/pitch.
    pub rotation: [f32; 2],
    /// Packet ordinal.
    pub receive_sequence: u64,
}
/// Current dimension metadata. Never silently clamped to another version's height.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
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
    pub generation: u64,
    pub pose: Option<ReceivedPose>,
    pub position_source: Option<ValueSource>,
    pub health: Option<ObservedValue<Health>>,
    pub may_fly: Option<bool>,
    pub selected_hotbar: Option<ObservedValue<u8>>,
    pub inventory: InventoryObservation,
    pub player_starts: std::collections::HashMap<i8, usize>,
    pub requested_flying: bool,
    pub pending_dispatch: bool,
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
        self.inventory.window_id = Some(i32::from(window));
        if window == 0 {
            self.inventory.slots = vec![None; 46];
            for (out, slot) in self.inventory.slots.iter_mut().zip(decoded) {
                *out = Some(slot);
            }
        } else if window > 0 && slots.len() >= 36 {
            let start = slots.len() - 36;
            self.player_starts.insert(window, start);
            for (out, slot) in self.inventory.slots[9..45]
                .iter_mut()
                .zip(decoded.into_iter().skip(start))
            {
                *out = Some(slot);
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
        let index = if update.window_id == 0 || update.window_id == -2 {
            Some(slot)
        } else {
            self.player_starts
                .get(&update.window_id)
                .and_then(|start| slot.checked_sub(*start))
                .filter(|i| *i < 36)
                .map(|i| 9 + i)
        };
        if let Some(out) = index.and_then(|i| self.inventory.slots.get_mut(i)) {
            *out = Some(value);
        }
        Ok(())
    }
}
