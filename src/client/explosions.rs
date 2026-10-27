//! Bounded received explosion context, independent of motion prediction.
use super::{
    SessionStamp,
    effects::{ParticleData, SoundReference},
};
use crate::{Error, ErrorKind, Result};
use std::collections::VecDeque;

/// Maximum retained explosion packets per connection.
pub const MAX_RETAINED_EXPLOSIONS: usize = 1024;
/// Maximum total retained Java 1.16.1 affected-block positions per connection.
pub const MAX_RETAINED_EXPLOSION_BLOCKS: usize = 65_536;
/// Maximum decompressed Java 1.21.11 explosion payload size, in bytes.
pub const MAX_EXPLOSION_PACKET_BYTES: usize = 16_384;

/// Native particle type and options, without rendering.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ExplosionParticle {
    /// Native particle registry ID in Java 1.21.11.
    pub native_id: i32,
    /// Original typed options.
    pub data: ParticleData,
}
/// One original weighted entry for block particle presentation.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ExplosionBlockParticle {
    /// Particle type and options.
    pub particle: ExplosionParticle,
    /// Original rendering scaling parameter.
    pub scaling: f32,
    /// Original rendering speed parameter.
    pub speed: f32,
    /// Original nonnegative selection weight; zero is retained.
    pub weight: i32,
}
/// Version-specific context, separate from the impulse applied to the player.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum ExplosionPresentation {
    /// Java 1.16.1 has no particle/sound fields. These are the received block
    /// positions applied as air, in packet order, including duplicates.
    Java1_16_1 {
        /// Absolute positions, relative offsets decoded against the floored center.
        affected_blocks: Vec<[i32; 3]>,
    },
    /// All presentation fields decoded with the version's particle table.
    Decoded {
        /// Main explosion particle.
        particle: ExplosionParticle,
        /// Sound holder; a registry ID does not establish a name.
        sound: SoundReference,
        /// Original weighted list, without expanding weights or block count.
        block_particles: Vec<ExplosionBlockParticle>,
    },
    /// An unknown embedded particle layout prevents locating later fields.
    /// These are all original bytes from the main particle ID to packet end;
    /// no absent sound or empty block-particle list is inferred.
    Undecoded {
        /// Bounded original presentation bytes, including particle IDs.
        bytes: Vec<u8>,
    },
}
/// Original explosion fields; interpretation depends on the received version.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ExplosionObservation {
    /// Explosion center, in blocks.
    pub center: [f64; 3],
    /// Original radius parameter.
    pub radius: f32,
    /// Original signed block count. In Java 1.16.1 this is the position list length;
    /// in Java 1.21.11 it does not establish destroyed block positions.
    pub block_count: i32,
    /// Original player knockback; always Some in Java 1.16.1.
    /// None and Some([0.0; 3]) are distinct in Java 1.21.11.
    /// This is an impulse, not a replacement velocity or an estimated displacement.
    pub player_knockback: Option<[f64; 3]>,
    /// Version-specific block/presentation context, independent of player impulse.
    pub presentation: ExplosionPresentation,
}
/// Immutable observation with receive order and world ownership.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ReceivedExplosion {
    /// Connection/version/world at receipt, retained across world changes.
    pub session: SessionStamp,
    /// Connection-local receive ordinal, shared with the change notification.
    pub receive_sequence: u64,
    /// Original decoded fields and any undecoded presentation bytes.
    pub explosion: ExplosionObservation,
}
/// Explosions after a receive-sequence cursor, in receive order.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ExplosionLog {
    /// Next cursor; reception boundary, independent of the event log cursor.
    pub receive_sequence: u64,
    /// Original observations, possibly belonging to earlier worlds.
    pub explosions: Vec<ReceivedExplosion>,
}
impl crate::Client {
    /// Read received explosions after a receive sequence (initially 0).
    /// Readable after disconnect. Retention gaps return `State`; future cursors
    /// return `InvalidInput`. Java 1.16.1 includes applied block positions and a
    /// mandatory impulse; Java 1.21.11 includes presentation. Sends no packets.
    pub async fn explosions_after(&self, after_sequence: u64) -> Result<ExplosionLog> {
        super::dispatch!(&self.adapter, a => super::adapter::EventOps::explosions_after(a, after_sequence).await)
    }
}
#[derive(Default)]
pub(crate) struct ExplosionLedger {
    records: VecDeque<ReceivedExplosion>,
    dropped_through: u64,
    retained_blocks: usize,
}
impl ExplosionLedger {
    pub(crate) fn record(&mut self, record: ReceivedExplosion) {
        self.retained_blocks += Self::block_count(&record);
        self.records.push_back(record);
        while self.records.len() > MAX_RETAINED_EXPLOSIONS
            || self.retained_blocks > MAX_RETAINED_EXPLOSION_BLOCKS
        {
            let dropped = self.records.pop_front().expect("nonempty explosions");
            self.retained_blocks -= Self::block_count(&dropped);
            self.dropped_through = dropped.receive_sequence;
        }
    }
    fn block_count(record: &ReceivedExplosion) -> usize {
        match &record.explosion.presentation {
            ExplosionPresentation::Java1_16_1 { affected_blocks } => affected_blocks.len(),
            _ => 0,
        }
    }
    pub(crate) fn after(&self, after_sequence: u64, receive_sequence: u64) -> Result<ExplosionLog> {
        if after_sequence < self.dropped_through {
            return Err(Error::new(
                ErrorKind::State,
                anyhow::anyhow!(
                    "explosions before the receive cursor were dropped; resume at the current receive sequence"
                ),
            ));
        }
        if after_sequence > receive_sequence {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                anyhow::anyhow!("explosion receive cursor is ahead of reception"),
            ));
        }
        Ok(ExplosionLog {
            receive_sequence,
            explosions: self
                .records
                .iter()
                .filter(|r| r.receive_sequence > after_sequence)
                .cloned()
                .collect(),
        })
    }
}
