//! Bounded received own-player and world context, separate from model state.
use super::{ObservedValue, SessionStamp, received};

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
    /// Capture bounded received experience, weather, default spawn and view
    /// packets. Missing values stay absent; no present-time interpolation,
    /// permission or server outcome is inferred. Readable after closure.
    pub async fn player_context(&self) -> crate::Result<PlayerContextObservation> {
        super::dispatch!(&self.adapter, a => super::adapter::CoreOps::player_context(a).await)
    }
}

#[derive(Default)]
pub(crate) struct ContextLedger {
    pub maps: super::maps::MapLedger,
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
            experience: self.experience.clone(),
            weather: self.weather.clone(),
            default_spawn: self.default_spawn.clone(),
            world_view: self.world_view.clone(),
        }
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
