//! Bounded packet-application samples, independent of change notifications.
use super::*;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

/// Maximum retained spatial/status/lifetime samples per connection.
pub const MAX_ENTITY_HISTORY_RECORDS: usize = 8192;
/// Maximum records returned by one read.
pub const MAX_ENTITY_HISTORY_READ: usize = 1024;

/// A connection-scoped record ordinal, not a packet sequence or live action target.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct EntityHistoryCursor {
    version: MinecraftVersion,
    connection_id: u64,
    ordinal: u64,
}
impl EntityHistoryCursor {
    /// Read-only record ordinal for bounding a paged drain at a captured tail.
    /// It is not a packet sequence and cannot construct another cursor/target.
    pub fn ordinal(self) -> u64 {
        self.ordinal
    }
}
/// Explicit loss before the retained window. Reading resumes at its oldest record.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct EntityHistoryGap {
    /// Last ordinal requested by the reader (zero for an initial read).
    pub after: u64,
    /// All records through this ordinal have been evicted.
    pub dropped_through: u64,
}
/// Original decoded/applied facts. Missing spatial fields remain absent.
#[derive(Clone, Debug, serde::Serialize)]
pub enum EntityHistoryKind {
    /// Spawn plus fields supplied by this version's actual spawn packet.
    Spawn(Box<EntityMotionObservation>),
    /// Spatial projection immediately after an actual motion packet, with per-field sources.
    Motion(Box<EntityMotionObservation>),
    /// Native status code; optional identity is absent for an unreceived spawn.
    Status {
        /// Numeric identifier from the original packet.
        native_id: i32,
        /// Original lifetime, if its spawn was received.
        entity: Option<EntityId>,
        /// Version-specific native status code.
        status: i8,
    },
    /// Native animation code; optional identity is absent for an unreceived spawn.
    Animation {
        /// Numeric identifier from the original packet.
        native_id: i32,
        /// Original lifetime, if its spawn was received.
        entity: Option<EntityId>,
        /// Version-specific native animation code.
        animation: u8,
    },
    /// Removal as received, including numeric IDs that had no known spawn.
    Removed {
        /// Numeric identifier from the original packet.
        native_id: i32,
        /// Original lifetime, if its spawn was received.
        entity: Option<EntityId>,
    },
    /// Invalidates old lifetimes without pretending each received an explicit removal.
    WorldChanged {
        /// World generation invalidated by this packet.
        previous_generation: u64,
    },
    /// Resolved own-player correction, distinct from remote spawn lifetimes.
    OwnPositionCorrection {
        /// Resolved pose at receipt, without later submitted/model motion.
        pose: super::super::ReceivedPose,
        /// Resolved velocity if supplied and resolvable by this native correction.
        velocity: Option<ObservedValue<[f64; 3]>>,
    },
}
/// One sample recorded during native packet application, never reconstructed at poll time.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EntityHistoryRecord {
    /// Version, connection and world at application; old worlds retain their own stamp.
    pub session: SessionStamp,
    /// Total record order; several records may belong to one packet.
    pub ordinal: u64,
    /// Native adapter's packet receive/apply ordinal.
    pub receive_sequence: u64,
    /// Monotonic time since this connection's history ledger creation to recording.
    /// This is SDK application time, not socket arrival, relay receipt or poll time.
    pub applied_after: Duration,
    /// Frozen decoded/applied facts.
    pub kind: EntityHistoryKind,
}
/// A page of retained records, readable after closure/revocation.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EntityHistory {
    /// Current session at this coherent read boundary; records may be from older worlds.
    pub session: SessionStamp,
    /// Adapter receive boundary at read time, separate from historical samples.
    pub receive_sequence: u64,
    /// Read-time clock sample from the same ledger origin as `applied_after`.
    /// Advances on quiet reads too; subtract record time to measure its age.
    /// Not packet arrival time, a server tick, or a freshness acknowledgement.
    pub captured_after: Duration,
    /// Oldest-first records, limited by the requested read bound.
    pub records: Vec<EntityHistoryRecord>,
    /// Resume after the last returned record (or the requested cursor if empty).
    pub next_cursor: EntityHistoryCursor,
    /// Tail cursor at this read boundary, for explicitly skipping older records.
    pub latest_cursor: EntityHistoryCursor,
    /// More retained records are available after `next_cursor`.
    pub has_more: bool,
    /// Explicit loss; no gap is silently represented as complete history.
    pub gap: Option<EntityHistoryGap>,
}

pub(crate) struct HistoryLedger {
    records: VecDeque<EntityHistoryRecord>,
    next: u64,
    dropped_through: u64,
    started: Instant,
    pub(super) context: Option<(MinecraftVersion, u64, u64)>,
}
impl Default for HistoryLedger {
    fn default() -> Self {
        Self {
            records: VecDeque::new(),
            next: 0,
            dropped_through: 0,
            started: Instant::now(),
            context: None,
        }
    }
}
impl HistoryLedger {
    fn stamp(&self) -> Option<SessionStamp> {
        self.context
            .map(|(version, world_generation, _)| SessionStamp {
                version,
                connection_id: 0,
                world_generation,
            })
    }
    pub(super) fn record(&mut self, kind: EntityHistoryKind) {
        let Some(session) = self.stamp() else {
            return;
        };
        self.next += 1;
        let receive_sequence = self.context.unwrap().2;
        self.records.push_back(EntityHistoryRecord {
            session,
            ordinal: self.next,
            receive_sequence,
            applied_after: self.started.elapsed(),
            kind,
        });
        if self.records.len() > MAX_ENTITY_HISTORY_RECORDS {
            self.dropped_through = self.records.pop_front().unwrap().ordinal;
        }
    }
    fn after(
        &self,
        session: SessionStamp,
        sequence: u64,
        cursor: Option<EntityHistoryCursor>,
        maximum: usize,
    ) -> Result<EntityHistory> {
        if maximum == 0 || maximum > MAX_ENTITY_HISTORY_READ {
            return Err(super::super::registry::invalid(
                "entity history read bound must be in 1..=1024",
            ));
        }
        if cursor.is_some_and(|c| {
            c.version != session.version
                || c.connection_id != session.connection_id
                || c.ordinal > self.next
        }) {
            return Err(super::super::inventory::unavailable(
                "entity history cursor belongs to another connection or is ahead of history",
            ));
        }
        let after = cursor.map_or(0, |c| c.ordinal);
        let records: Vec<_> = self
            .records
            .iter()
            .filter(|r| r.ordinal > after)
            .take(maximum)
            .cloned()
            .map(|mut r| {
                r.session.connection_id = session.connection_id;
                match &mut r.kind {
                    EntityHistoryKind::Spawn(m) | EntityHistoryKind::Motion(m) => {
                        m.entity.id.session.connection_id = session.connection_id
                    }
                    EntityHistoryKind::Status { entity, .. }
                    | EntityHistoryKind::Animation { entity, .. }
                    | EntityHistoryKind::Removed { entity, .. } => {
                        if let Some(id) = entity {
                            id.session.connection_id = session.connection_id;
                        }
                    }
                    _ => {}
                }
                r
            })
            .collect();
        let next = records.last().map_or(after, |r| r.ordinal);
        let mk = |ordinal| EntityHistoryCursor {
            version: session.version,
            connection_id: session.connection_id,
            ordinal,
        };
        Ok(EntityHistory {
            session,
            receive_sequence: sequence,
            captured_after: self.started.elapsed(),
            records,
            next_cursor: mk(next),
            latest_cursor: mk(self.next),
            has_more: next < self.next,
            gap: (after < self.dropped_through).then_some(EntityHistoryGap {
                after,
                dropped_through: self.dropped_through,
            }),
        })
    }
}
impl SpawnLedger {
    pub(crate) fn history_context(
        &mut self,
        version: MinecraftVersion,
        generation: u64,
        sequence: u64,
    ) {
        let previous = self.1.context.map(|c| c.1);
        self.1.context = Some((version, generation, sequence));
        if let Some(previous_generation) = previous.filter(|g| *g != generation) {
            self.1.record(EntityHistoryKind::WorldChanged {
                previous_generation,
            });
        }
    }
    pub(crate) fn history_motion(&mut self, id: i32, spawn: bool) {
        let Some(session) = self.1.stamp() else {
            return;
        };
        let Some(target) = self.identity(session, id) else {
            return;
        };
        let sequence = self.1.context.unwrap().2;
        if let Ok(m) = self.capture_motion(session, target, sequence) {
            self.1.record(if spawn {
                EntityHistoryKind::Spawn(Box::new(m))
            } else {
                EntityHistoryKind::Motion(Box::new(m))
            });
        }
    }
    pub(crate) fn history_signal(&mut self, id: i32, status: Option<i8>, animation: Option<u8>) {
        let entity = self.1.stamp().and_then(|s| self.identity(s, id));
        if let Some(status) = status {
            self.1.record(EntityHistoryKind::Status {
                native_id: id,
                entity,
                status,
            });
        }
        if let Some(animation) = animation {
            self.1.record(EntityHistoryKind::Animation {
                native_id: id,
                entity,
                animation,
            });
        }
    }
    pub(crate) fn history_pose(
        &mut self,
        pose: super::super::ReceivedPose,
        velocity: Option<ObservedValue<[f64; 3]>>,
    ) {
        self.1
            .record(EntityHistoryKind::OwnPositionCorrection { pose, velocity });
    }
    pub(crate) fn history_after(
        &self,
        session: SessionStamp,
        sequence: u64,
        cursor: Option<EntityHistoryCursor>,
        maximum: usize,
    ) -> Result<EntityHistory> {
        self.1.after(session, sequence, cursor, maximum)
    }
}
impl super::super::Client {
    /// Read frozen entity samples after a connection-scoped cursor. `None` starts at
    /// the retained window and reports any prior eviction explicitly. No packet is sent.
    pub async fn entity_history_after(
        &self,
        cursor: Option<EntityHistoryCursor>,
        maximum_records: usize,
    ) -> Result<EntityHistory> {
        use super::super::adapter::{EventOps, dispatch};
        dispatch!(&self.adapter, a => EventOps::entity_history_after(a, cursor, maximum_records).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn entity_history_preserves_missing_fields_and_rejects_foreign_cursors() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let mut ledger = SpawnLedger::default();
            ledger.history_context(version, 7, 10);
            ledger
                .insert(
                    version,
                    NativeSpawn {
                        id: 42,
                        uuid: None,
                        type_id: None,
                        dedicated_type_name: Some("experience_orb"),
                        position: [1., 2., 3.],
                        living: Some(false),
                    },
                    10,
                    4096,
                )
                .unwrap();
            ledger.initialize_motion(
                42,
                NativeSpawnMotion {
                    position: Some([1., 2., 3.]),
                    ..Default::default()
                },
                10,
            );
            let session = SessionStamp {
                version,
                connection_id: 4,
                world_generation: 7,
            };
            let before = ledger.history_after(session, 10, None, 1).unwrap();
            let EntityHistoryKind::Spawn(m) = &before.records[0].kind else {
                panic!()
            };
            assert!(
                m.rotation.is_none()
                    && m.velocity.is_none()
                    && m.head_yaw.is_none()
                    && m.on_ground.is_none()
            );
            ledger.history_context(version, 7, 11);
            ledger.receive_motion(version, 42, NativeMotion::Velocity([0.; 3]), 11);
            assert!(m.velocity.is_none());
            let mut foreign = before.next_cursor;
            foreign.connection_id += 1;
            assert!(ledger.history_after(session, 11, Some(foreign), 1).is_err());
            foreign = before.next_cursor;
            foreign.version = if version == MinecraftVersion::Java1_16_1 {
                MinecraftVersion::Java1_21_11
            } else {
                MinecraftVersion::Java1_16_1
            };
            assert!(ledger.history_after(session, 11, Some(foreign), 1).is_err());
            foreign = before.next_cursor;
            foreign.ordinal = 999;
            assert!(ledger.history_after(session, 11, Some(foreign), 1).is_err());
            let after = ledger
                .history_after(session, 11, Some(before.next_cursor), 1)
                .unwrap();
            let EntityHistoryKind::Motion(m) = &after.records[0].kind else {
                panic!()
            };
            assert_eq!(
                m.velocity.as_ref().unwrap().source,
                super::super::super::ValueSource::Received { sequence: 11 }
            );
            assert_eq!(
                m.position.as_ref().unwrap().source,
                super::super::super::ValueSource::Received { sequence: 10 }
            );
        }
    }
}
