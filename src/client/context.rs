//! Bounded received own-player and world context, separate from model state.
use super::{ObservedValue, SessionStamp, received};

/// Original respawn target in a modern LOGIN/RESPAWN packet.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct DeathLocation {
    /// Original dimension resource key.
    pub dimension: String,
    /// Original block coordinates; not the current player position.
    pub position: [i32; 3],
}
/// Original world-entry fields, without inferring a current game mode.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct WorldEntryContext {
    /// Original world resource key.
    pub world_name: String,
    /// Explicit 1.16.1 dimension type key.
    pub dimension_type_name: Option<String>,
    /// Explicit 1.21.11 dimension type registry ID.
    pub dimension_type_id: Option<i32>,
    /// Original game-mode byte. Legacy LOGIN includes the hardcore bit.
    pub game_mode: u8,
    /// Original signed byte; -1 is the explicit unknown-mode sentinel.
    pub previous_game_mode: i8,
    /// Original hashed seed. Absent in historical legacy prefix fixtures.
    pub hashed_seed: Option<i64>,
    /// Original debug-world flag, if supplied.
    pub debug: Option<bool>,
    /// Original flat-world flag, if supplied.
    pub flat: Option<bool>,
    /// Explicit modern last death location. None also represents a native absent location.
    pub last_death: Option<DeathLocation>,
    /// Explicit modern portal cooldown, without a local countdown.
    pub portal_cooldown: Option<i32>,
    /// Explicit modern sea level, independent of the world's blocks.
    pub sea_level: Option<i32>,
    /// Respawn keep-data byte: legacy boolean, modern flags. None on LOGIN.
    pub respawn_keep_data: Option<u8>,
}
/// Additional original LOGIN conditions. Not repeated or inferred on respawn.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct LoginConditions {
    /// Original maximum players field.
    pub max_players: i32,
    /// Original reduced-debug-info flag.
    pub reduced_debug_info: bool,
    /// Original enable-respawn-screen flag.
    pub enable_respawn_screen: bool,
    /// Explicit modern hardcore boolean; legacy carries a bit in game_mode.
    pub hardcore: Option<bool>,
    /// Explicit modern limited-crafting flag.
    pub limited_crafting: Option<bool>,
    /// Explicit modern secure-chat enforcement flag.
    pub enforces_secure_chat: Option<bool>,
}
/// Version-specific identity in an original cooldown notification.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum CooldownKey {
    /// 1.16.1 native item registry ID. Not an inventory slot.
    LegacyItem(i32),
    /// 1.21.11 cooldown group resource key. Not necessarily an item ID.
    Group(String),
}
/// Latest original cooldown notification, not an estimated remaining duration.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ItemCooldown {
    /// Original native identity.
    pub key: CooldownKey,
    /// Original ticks, including explicit zero clearing notifications.
    pub ticks: ObservedValue<i32>,
}

/// One original abilities packet; these are receipts, not execution authority.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct PlayerAbilities {
    /// Original flags: invulnerable, flying, may fly and instant build.
    pub flags: u8,
    /// Original flying speed, without deriving an effective movement speed.
    pub flying_speed: f32,
    /// Original walking speed, independent of received movement attributes.
    pub walking_speed: f32,
}
impl PlayerAbilities {
    /// Received invulnerability flag.
    pub const fn invulnerable(self) -> bool {
        self.flags & 1 != 0
    }
    /// Received flying flag, independent of locally requested flight.
    pub const fn flying(self) -> bool {
        self.flags & 2 != 0
    }
    /// Received flight permission; a saved receipt does not authorize an action.
    pub const fn may_fly(self) -> bool {
        self.flags & 4 != 0
    }
    /// Received instant-build flag; not a synthesized game mode.
    pub const fn instant_build(self) -> bool {
        self.flags & 8 != 0
    }
}
/// Original world difficulty notification, without inferring server rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct WorldDifficulty {
    /// Original byte identifier: normally 0 peaceful, 1 easy, 2 normal, 3 hard.
    pub id: u8,
    /// Original difficulty-lock boolean; known false is distinct from absence.
    pub locked: bool,
}

/// One original experience packet. No inferred progress or locally earned XP.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct Experience {
    /// Original fraction toward the next level.
    pub progress: f32,
    /// Original level field.
    pub level: i32,
    /// Original total-experience field.
    pub total: i32,
}
/// Last explicit weather fields; absent fields are never defaulted or blended.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct WeatherObservation {
    /// Actual rain start/stop event. Intensity alone supplies no boolean.
    pub raining: Option<ObservedValue<bool>>,
    /// Original rain intensity, without client interpolation.
    pub rain_level: Option<ObservedValue<f32>>,
    /// Original thunder intensity, without a synthesized thunder boolean.
    pub thunder_level: Option<ObservedValue<f32>>,
}
/// Received default world spawn, distinct from the player's bed/respawn target.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct DefaultSpawnPosition {
    /// Original packed block coordinates.
    pub position: [i32; 3],
    /// Explicit dimension resource key in 1.21.11; absent in 1.16.1.
    pub dimension: Option<String>,
    /// Explicit native spawn yaw in 1.21.11; absent in 1.16.1.
    pub yaw: Option<f32>,
    /// Explicit native spawn pitch in 1.21.11; absent in 1.16.1.
    pub pitch: Option<f32>,
}
/// Explicit world-view packets; neither loaded chunks nor configured budgets.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct WorldViewObservation {
    /// Original center chunk coordinates.
    pub center: Option<ObservedValue<[i32; 2]>>,
    /// Original view distance, in chunks.
    pub distance: Option<ObservedValue<i32>>,
    /// Explicit 1.21.11 simulation distance; always absent in 1.16.1.
    pub simulation_distance: Option<ObservedValue<i32>>,
}
/// Coherent last-received fields for this connection/world generation.
/// Readable after closure. Capture progress never refreshes field sources.
#[derive(Clone, Debug, serde::Serialize)]
pub struct PlayerContextObservation {
    /// Original connection and current received world generation.
    pub session: SessionStamp,
    /// Capture boundary, independent from every field's receipt ordinal.
    pub receive_sequence: u64,
    /// Latest complete LOGIN/RESPAWN packet fields in this generation.
    pub world_entry: Option<ObservedValue<WorldEntryContext>>,
    /// LOGIN-only conditions; absent after a respawn until another LOGIN.
    pub login_conditions: Option<ObservedValue<LoginConditions>>,
    /// Latest cooldown receipts sorted by native key; at most 1024 keys.
    pub item_cooldowns: Vec<ItemCooldown>,
    /// Original abilities packet, if received in this world generation.
    pub abilities: Option<ObservedValue<PlayerAbilities>>,
    /// Original difficulty notification, if received in this generation.
    pub difficulty: Option<ObservedValue<WorldDifficulty>>,
    /// Original own-player experience packet, if received in this generation.
    pub experience: Option<ObservedValue<Experience>>,
    /// Independent actual weather fields.
    pub weather: WeatherObservation,
    /// Original default spawn packet, if received in this generation.
    pub default_spawn: Option<ObservedValue<DefaultSpawnPosition>>,
    /// Independent actual world-view fields.
    pub world_view: WorldViewObservation,
}
impl crate::Client {
    /// Capture bounded received abilities, difficulty, experience, weather, spawn and view
    /// packets. Missing values stay absent; no present-time interpolation,
    /// permission or server outcome is inferred. Readable after closure.
    pub async fn player_context(&self) -> crate::Result<PlayerContextObservation> {
        super::dispatch!(&self.adapter, a => super::adapter::CoreOps::player_context(a).await)
    }
}

#[derive(Default)]
pub(crate) struct ContextLedger {
    pub world_entry: Option<ObservedValue<WorldEntryContext>>,
    pub login_conditions: Option<ObservedValue<LoginConditions>>,
    pub item_cooldowns: std::collections::BTreeMap<CooldownKey, ObservedValue<i32>>,
    pub maps: super::maps::MapLedger,
    pub abilities: Option<ObservedValue<PlayerAbilities>>,
    pub difficulty: Option<ObservedValue<WorldDifficulty>>,
    pub experience: Option<ObservedValue<Experience>>,
    pub weather: WeatherObservation,
    pub default_spawn: Option<ObservedValue<DefaultSpawnPosition>>,
    pub world_view: WorldViewObservation,
}
impl ContextLedger {
    pub fn capture(
        &self,
        session: SessionStamp,
        receive_sequence: u64,
    ) -> PlayerContextObservation {
        PlayerContextObservation {
            session,
            receive_sequence,
            world_entry: self.world_entry.clone(),
            login_conditions: self.login_conditions.clone(),
            item_cooldowns: self
                .item_cooldowns
                .iter()
                .map(|(key, ticks)| ItemCooldown {
                    key: key.clone(),
                    ticks: ticks.clone(),
                })
                .collect(),
            abilities: self.abilities.clone(),
            difficulty: self.difficulty.clone(),
            experience: self.experience.clone(),
            weather: self.weather.clone(),
            default_spawn: self.default_spawn.clone(),
            world_view: self.world_view.clone(),
        }
    }
    pub fn cooldown(&mut self, key: CooldownKey, ticks: i32, sequence: u64) -> anyhow::Result<()> {
        anyhow::ensure!(ticks >= 0, "negative item cooldown");
        anyhow::ensure!(
            self.item_cooldowns.contains_key(&key) || self.item_cooldowns.len() < 1024,
            "cooldown receipt budget exceeded"
        );
        self.item_cooldowns.insert(key, received(ticks, sequence));
        Ok(())
    }
    pub fn weather(&mut self, reason: u8, value: f32, sequence: u64) {
        match reason {
            1 => self.weather.raining = Some(received(true, sequence)),
            2 => self.weather.raining = Some(received(false, sequence)),
            7 => self.weather.rain_level = Some(received(value, sequence)),
            8 => self.weather.thunder_level = Some(received(value, sequence)),
            _ => {}
        }
    }
}
