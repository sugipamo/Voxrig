//! Bounded Java 1.21.11 transient observations, separate from change notifications.
use super::SessionStamp;
use crate::{Error, ErrorKind, Result};
use std::collections::VecDeque;

/// Maximum transient observations retained per connection.
pub const MAX_RETAINED_EFFECTS: usize = 1024;
/// Maximum decompressed payload size for one observed effect packet, in bytes.
pub const MAX_EFFECT_PACKET_BYTES: usize = 16_384;

/// Original sound holder, without guessing a name for a registry ID.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum SoundReference {
    /// Zero-based native sound-event registry ID (wire holder ID minus one).
    RegistryId(i32),
    /// Direct sound event sent inline by the server.
    Inline {
        /// Native resource identifier.
        name: String,
        /// Explicit fixed range; None means the native volume-dependent range.
        fixed_range: Option<f32>,
    },
}
/// Original sound source location; entity sounds do not contain coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum SoundOrigin {
    /// Signed native coordinates, in eighths of a block, retained without rounding.
    PositionEighths([i32; 3]),
    /// Native entity ID in the observation's world context.
    Entity(i32),
}
/// Original sound fields. Receipt does not prove playback or audibility.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct SoundObservation {
    /// Original holder.
    pub sound: SoundReference,
    /// Native sound-source enum ordinal (0 master through 9 voice).
    pub source: i32,
    /// Position or entity from the packet.
    pub origin: SoundOrigin,
    /// Native volume.
    pub volume: f32,
    /// Native pitch.
    pub pitch: f32,
    /// Native random seed, including negative values.
    pub seed: i64,
}
/// Received vibration destination; an entity reference is not resolved to a position.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub enum VibrationDestination {
    /// Native block position.
    Block([i32; 3]),
    /// Native entity ID and eye-height offset.
    Entity {
        /// Native entity ID.
        native_id: i32,
        /// Native vertical offset.
        eye_height: f32,
    },
}
/// Native nonempty item particle stack.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ParticleItem {
    /// Original positive item count.
    pub count: i32,
    /// Original item registry ID.
    pub native_id: i32,
    /// Lossless received component patch, without applying default components.
    pub components: super::ItemComponentPatch,
}
/// Version-bound particle options. Packed colors retain all received bits.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum ParticleData {
    /// Known simple particle with no options.
    Simple,
    /// Native block-state registry ID.
    BlockState(i32),
    /// Packed native color.
    Color(u32),
    /// Dust's packed native color and scale.
    Dust {
        /// Packed color.
        color: u32,
        /// Received scale, before any rendering clamp.
        scale: f32,
    },
    /// Dust color transition.
    DustTransition {
        /// Packed initial color.
        from_color: u32,
        /// Packed final color.
        to_color: u32,
        /// Received scale.
        scale: f32,
    },
    /// Dragon-breath power.
    Power(f32),
    /// Effect or instant-effect packed color and power.
    Spell {
        /// Packed color.
        color: u32,
        /// Native power.
        power: f32,
    },
    /// Sculk-charge roll in radians.
    SculkCharge(f32),
    /// Shriek delay in ticks.
    Shriek(i32),
    /// Native nonempty item stack.
    Item(ParticleItem),
    /// Vibration destination and arrival time.
    Vibration {
        /// Native destination.
        destination: VibrationDestination,
        /// Native arrival time in ticks.
        arrival_ticks: i32,
    },
    /// Trail destination, packed color and duration.
    Trail {
        /// Target coordinates in blocks.
        target: [f64; 3],
        /// Packed color.
        color: u32,
        /// Native duration in ticks.
        duration_ticks: i32,
    },
    /// Unrecognized particle or vibration source. These bounded original option
    /// bytes are preserved without inferring a layout from their length.
    Unknown {
        /// All original bytes after the particle type ID.
        bytes: Vec<u8>,
    },
}
/// One particle packet, never expanded into `count` individual particles.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ParticleObservation {
    /// Native distance/limiter override flag.
    pub override_limiter: bool,
    /// Native always-show flag.
    pub always_show: bool,
    /// Native origin coordinates in blocks.
    pub position: [f64; 3],
    /// Native per-axis offsets; their rendering meaning depends on count.
    pub offset: [f32; 3],
    /// Native speed parameter.
    pub max_speed: f32,
    /// Original signed count. Zero has native directional/single-particle semantics.
    pub count: i32,
    /// Original native particle registry ID.
    pub native_id: i32,
    /// Options decoded with the Java 1.21.11 type table.
    pub data: ParticleData,
}
/// Original Java 1.21.11 damage source, resolved in the received configuration.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct DamageObservation {
    /// Native ID of the damaged entity in the observation's world.
    pub entity_id: i32,
    /// Zero-based damage-type ID with connection/configuration ownership.
    pub source_type: super::registry::ServerRegistryId,
    /// Name resolved from the received damage_type registry at receipt.
    pub source_type_name: String,
    /// Causing entity; wire zero means absent, otherwise wire value minus one.
    pub cause_entity_id: Option<i32>,
    /// Direct entity; wire zero means absent, otherwise wire value minus one.
    pub direct_entity_id: Option<i32>,
    /// Explicit source position in blocks, when present in the packet.
    pub source_position: Option<[f64; 3]>,
}
/// Original transient packet fields, without simulated world or inventory changes.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum PlayEffect {
    /// Received damage source. Does not establish damage amount or new health.
    Damage(DamageObservation),
    /// Hurt animation with the original yaw in degrees.
    HurtAnimation {
        /// Native entity ID in the observation's world.
        entity_id: i32,
        /// Original animation yaw, without normalization.
        yaw: f32,
    },
    /// Own player's combat tracker entered combat; no entity ID is sent.
    CombatEntered,
    /// Own player's combat tracker ended combat; no entity ID is sent.
    CombatEnded {
        /// Original signed duration in server ticks.
        duration_ticks: i32,
    },
    /// Death notification with the original native text component.
    CombatKill {
        /// Native player entity ID supplied by the server.
        player_id: i32,
        /// Original unnamed NBT text; no rendering or translation.
        message: super::ui::UiText,
    },
    /// Position or entity sound.
    Sound(SoundObservation),
    /// Stop all sounds matching the optional source and/or name.
    SoundStopped {
        /// None means every source.
        source: Option<i32>,
        /// None means every sound.
        name: Option<String>,
    },
    /// Particle emission parameters.
    Particles(ParticleObservation),
    /// Native level/world event; data interpretation depends on event ID.
    LevelEvent {
        /// Original event code, including unknown codes.
        event_id: i32,
        /// Block position.
        position: [i32; 3],
        /// Original signed event data.
        data: i32,
        /// Native global-event flag.
        global: bool,
    },
    /// Another breaker's crack animation. Does not prove a block changed.
    BlockDestruction {
        /// Native breaker ID, not necessarily a currently tracked player.
        breaker_id: i32,
        /// Block position.
        position: [i32; 3],
        /// Original unsigned byte; 0..=9 are stages, all other values remove cracks.
        progress: u8,
    },
    /// Pickup animation. Does not establish a new inventory slot or entity removal.
    ItemPickup {
        /// Native collected entity ID (may also represent an experience orb).
        collected_id: i32,
        /// Native collector entity ID, including another player.
        collector_id: i32,
        /// Original signed amount.
        amount: i32,
    },
}
/// Immutable transient observation, retaining its original world and packet sequence.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ReceivedEffect {
    /// Connection/version/world at receipt; IDs must be interpreted in this context.
    pub session: SessionStamp,
    /// Original receive sequence, also present on the corresponding change event.
    pub receive_sequence: u64,
    /// Decoded original packet fields.
    pub effect: PlayEffect,
}
/// Transient observations after a receive-sequence cursor.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EffectLog {
    /// Next cursor; a receive sequence, unlike `EventLog::cursor`.
    pub receive_sequence: u64,
    /// Original observations in receive order, possibly from earlier worlds.
    pub effects: Vec<ReceivedEffect>,
}
impl crate::Client {
    /// Java 1.21.11 transient observations after a receive sequence (initially 0).
    /// Readable after closure. A cursor older than retained effects fails with
    /// `State`; a cursor ahead of reception fails with `InvalidInput`.
    /// Java 1.16.1 returns `Unsupported`. This passive read never sends packets.
    pub async fn effects_after(&self, after_sequence: u64) -> Result<EffectLog> {
        super::dispatch!(&self.adapter, a => super::adapter::EventOps::effects_after(a, after_sequence).await)
    }
}
#[derive(Default)]
pub(crate) struct EffectLedger {
    records: VecDeque<ReceivedEffect>,
    dropped_through: u64,
}
impl EffectLedger {
    pub(crate) fn record(&mut self, record: ReceivedEffect) {
        self.records.push_back(record);
        if self.records.len() > MAX_RETAINED_EFFECTS {
            self.dropped_through = self
                .records
                .pop_front()
                .expect("nonempty effects")
                .receive_sequence;
        }
    }
    pub(crate) fn after(&self, after_sequence: u64, receive_sequence: u64) -> Result<EffectLog> {
        if after_sequence < self.dropped_through {
            return Err(Error::new(
                ErrorKind::State,
                anyhow::anyhow!(
                    "effects before the receive cursor were dropped; resume at the current receive sequence"
                ),
            ));
        }
        if after_sequence > receive_sequence {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                anyhow::anyhow!("effect receive cursor is ahead of reception"),
            ));
        }
        Ok(EffectLog {
            receive_sequence,
            effects: self
                .records
                .iter()
                .filter(|e| e.receive_sequence > after_sequence)
                .cloned()
                .collect(),
        })
    }
}
