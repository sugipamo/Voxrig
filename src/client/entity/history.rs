//! Bounded packet-application samples, independent of change notifications.
use super::*;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

/// Maximum retained entity records per connection (also subject to a byte budget).
pub const MAX_ENTITY_HISTORY_RECORDS: usize = 8192;
/// Maximum accounted owned bytes of retained records, excluding allocator overhead.
pub const MAX_ENTITY_HISTORY_BYTES: usize = 8 * 1024 * 1024;
/// Maximum owned payload bytes copied into a metadata/equipment/scale record.
pub const MAX_ENTITY_HISTORY_VALUE_BYTES: usize = 64 * 1024;
/// Maximum records returned by one read.
pub const MAX_ENTITY_HISTORY_READ: usize = 1024;

/// A decoded packet value retained independently of later entity updates.
/// Unavailability here concerns history retention, not native decoding.
#[derive(Clone, Debug, serde::Serialize)]
pub enum EntityHistoryValue<T> {
    /// Decoded values supplied by the adapter (possibly incomplete).
    Retained {
        /// Frozen value, with source given by the surrounding record.
        value: T,
    },
    /// The packet boundary is retained without copying an oversized payload.
    RetentionLimit,
}
/// One metadata entry, in original packet order; duplicate indices remain distinct.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EntityMetadataEntry {
    /// Native version-specific metadata index.
    pub index: u8,
    /// Existing common decoder's value; `Other` retains only serializer identity.
    pub value: EntityDataValue,
}
/// One equipment entry, in original packet order; duplicate slots remain distinct.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EntityEquipmentEntry {
    /// Common equipment slot decoded from the native slot index.
    pub slot: EquipmentSlot,
    /// Exact supported stack, empty slot, or existing decoder's unavailable value.
    pub value: super::super::SlotKnowledge,
}

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
    /// Decoded metadata delta, not a snapshot, default or health projection.
    Metadata {
        /// Numeric identifier from the original packet.
        native_id: i32,
        /// Original lifetime if received; absent for own/unreceived entities.
        entity: Option<EntityId>,
        /// Decoded entries preserved in wire order, or an explicit retention refusal.
        entries: EntityHistoryValue<Vec<EntityMetadataEntry>>,
        /// Whether the common decoder reached the clean native packet end.
        complete: bool,
    },
    /// Decoded equipment delta, including explicitly empty and unavailable slots.
    Equipment {
        /// Numeric identifier from the original packet.
        native_id: i32,
        /// Original lifetime if received; absent for own/unreceived entities.
        entity: Option<EntityId>,
        /// Supported entries in wire order, or an explicit retention refusal.
        entries: EntityHistoryValue<Vec<EntityEquipmentEntry>>,
        /// Whether every native slot was recognized and the clean end reached.
        complete: bool,
    },
    /// Completely decoded modern scale attribute, independent of latest-state limits.
    Scale {
        /// Numeric identifier from the original packet.
        native_id: i32,
        /// Original lifetime if received; absent for own/unreceived entities.
        entity: Option<EntityId>,
        /// Original base and ordered modifiers, or an explicit retention refusal.
        attribute: EntityHistoryValue<super::super::control::ReceivedAttribute>,
    },
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
    records: VecDeque<(EntityHistoryRecord, usize)>,
    bytes: usize,
    next: u64,
    dropped_through: u64,
    started: Instant,
    pub(super) context: Option<(MinecraftVersion, u64, u64)>,
}
impl Default for HistoryLedger {
    fn default() -> Self {
        Self {
            records: VecDeque::new(),
            bytes: 0,
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
        let bytes = record_bytes(&kind);
        self.bytes += bytes;
        self.records.push_back((
            EntityHistoryRecord {
                session,
                ordinal: self.next,
                receive_sequence,
                applied_after: self.started.elapsed(),
                kind,
            },
            bytes,
        ));
        while self.records.len() > MAX_ENTITY_HISTORY_RECORDS
            || self.bytes > MAX_ENTITY_HISTORY_BYTES
        {
            let (record, bytes) = self.records.pop_front().unwrap();
            self.bytes -= bytes;
            self.dropped_through = record.ordinal;
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
            .filter(|(r, _)| r.ordinal > after)
            .take(maximum)
            .cloned()
            .map(|(mut r, _)| {
                r.session.connection_id = session.connection_id;
                match &mut r.kind {
                    EntityHistoryKind::Spawn(m) | EntityHistoryKind::Motion(m) => {
                        m.entity.id.session.connection_id = session.connection_id
                    }
                    EntityHistoryKind::Status { entity, .. }
                    | EntityHistoryKind::Animation { entity, .. }
                    | EntityHistoryKind::Removed { entity, .. }
                    | EntityHistoryKind::Metadata { entity, .. }
                    | EntityHistoryKind::Equipment { entity, .. }
                    | EntityHistoryKind::Scale { entity, .. } => {
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

fn retain<T>(bytes: usize, copy: impl FnOnce() -> T) -> EntityHistoryValue<T> {
    if bytes <= MAX_ENTITY_HISTORY_VALUE_BYTES {
        EntityHistoryValue::Retained { value: copy() }
    } else {
        EntityHistoryValue::RetentionLimit
    }
}
fn metadata_heap(value: &EntityDataValue) -> usize {
    match value {
        EntityDataValue::String(value) => value.len(),
        _ => 0,
    }
}
fn equipment_heap(value: &super::super::SlotKnowledge) -> usize {
    use super::super::{ItemData, SlotKnowledge};
    let SlotKnowledge::Item { item } = value else {
        return 0;
    };
    item.name.len().saturating_add(match &item.data {
        ItemData::Default => 0,
        ItemData::LegacyNbt { bytes } => bytes.len(),
        ItemData::ModernComponents { patch } => {
            let added = patch.added.iter().fold(
                patch
                    .added
                    .len()
                    .saturating_mul(std::mem::size_of::<super::super::ItemComponent>()),
                |total, entry| {
                    total
                        .saturating_add(entry.definition.name.len())
                        .saturating_add(entry.bytes.len())
                },
            );
            patch.removed.iter().fold(
                added.saturating_add(patch.removed.len().saturating_mul(std::mem::size_of::<
                    super::super::registry::ItemComponentDefinition,
                >())),
                |total, entry| total.saturating_add(entry.name.len()),
            )
        }
    })
}
fn scale_heap(value: &super::super::control::ReceivedAttribute) -> usize {
    value.modifiers.iter().fold(
        value
            .modifiers
            .len()
            .saturating_mul(std::mem::size_of::<super::super::control::Modifier>()),
        |total, modifier| total.saturating_add(modifier.id.len()),
    )
}
fn record_bytes(kind: &EntityHistoryKind) -> usize {
    use EntityHistoryKind as K;
    use EntityHistoryValue::Retained;
    let heap = match kind {
        K::Spawn(m) | K::Motion(m) => {
            std::mem::size_of::<EntityMotionObservation>()
                + m.entity.type_name.as_ref().map_or(0, String::len)
                + m.entity
                    .entity_type
                    .as_ref()
                    .map_or(0, |id| id.registry().len())
        }
        K::Metadata {
            entries: Retained { value },
            ..
        } => value.iter().fold(
            value.len() * std::mem::size_of::<EntityMetadataEntry>(),
            |total, entry| total + metadata_heap(&entry.value),
        ),
        K::Equipment {
            entries: Retained { value },
            ..
        } => value.iter().fold(
            value.len() * std::mem::size_of::<EntityEquipmentEntry>(),
            |total, entry| total + equipment_heap(&entry.value),
        ),
        K::Scale {
            attribute: Retained { value },
            ..
        } => scale_heap(value),
        _ => 0,
    };
    std::mem::size_of::<EntityHistoryRecord>() + std::mem::size_of::<usize>() + heap
}
impl SpawnLedger {
    pub(super) fn history_metadata(
        &mut self,
        native_id: i32,
        entries: &[(u8, EntityDataValue)],
        complete: bool,
    ) {
        let Some(session) = self.1.stamp() else {
            return;
        };
        let entity = self.identity(session, native_id);
        let bytes = entries.iter().fold(
            entries
                .len()
                .saturating_mul(std::mem::size_of::<EntityMetadataEntry>()),
            |total, (_, value)| total.saturating_add(metadata_heap(value)),
        );
        self.1.record(EntityHistoryKind::Metadata {
            native_id,
            entity,
            complete,
            entries: retain(bytes, || {
                entries
                    .iter()
                    .map(|(index, value)| EntityMetadataEntry {
                        index: *index,
                        value: value.clone(),
                    })
                    .collect()
            }),
        });
    }
    pub(super) fn history_equipment(
        &mut self,
        native_id: i32,
        entries: &[(EquipmentSlot, super::super::SlotKnowledge)],
        complete: bool,
    ) {
        let Some(session) = self.1.stamp() else {
            return;
        };
        let entity = self.identity(session, native_id);
        let bytes = entries.iter().fold(
            entries
                .len()
                .saturating_mul(std::mem::size_of::<EntityEquipmentEntry>()),
            |total, (_, value)| total.saturating_add(equipment_heap(value)),
        );
        self.1.record(EntityHistoryKind::Equipment {
            native_id,
            entity,
            complete,
            entries: retain(bytes, || {
                entries
                    .iter()
                    .map(|(slot, value)| EntityEquipmentEntry {
                        slot: *slot,
                        value: value.clone(),
                    })
                    .collect()
            }),
        });
    }
    pub(super) fn history_scale(
        &mut self,
        native_id: i32,
        attribute: &super::super::control::ReceivedAttribute,
    ) {
        let Some(session) = self.1.stamp() else {
            return;
        };
        let entity = self.identity(session, native_id);
        self.1.record(EntityHistoryKind::Scale {
            native_id,
            entity,
            attribute: retain(scale_heap(attribute), || attribute.clone()),
        });
    }
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
    use crate::client::{ItemComponent, ItemComponentPatch, ItemData, ItemStack, SlotKnowledge};

    fn session(version: MinecraftVersion, world_generation: u64) -> SessionStamp {
        SessionStamp {
            version,
            connection_id: 97,
            world_generation,
        }
    }
    fn spawn(ledger: &mut SpawnLedger, version: MinecraftVersion, sequence: u64) {
        ledger
            .insert(
                version,
                NativeSpawn {
                    id: 7,
                    uuid: None,
                    type_id: None,
                    dedicated_type_name: Some("cow"),
                    position: [0.; 3],
                    living: Some(true),
                },
                sequence,
                16,
            )
            .unwrap();
        ledger.initialize_motion(7, NativeSpawnMotion::default(), sequence);
    }
    #[test]
    fn deltas_freeze_wire_order_partial_receipts_and_original_lifetimes() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let mut ledger = SpawnLedger::default();
            let old = session(version, 3);
            ledger.history_context(version, 3, 1);
            spawn(&mut ledger, version, 1);
            let tail = ledger.history_after(old, 1, None, 1).unwrap().latest_cursor;
            ledger.history_context(version, 3, 2);
            ledger.receive_metadata(
                7,
                [
                    (6, EntityDataValue::Pose(1)),
                    (6, EntityDataValue::Pose(0)),
                    (8, EntityDataValue::Other(5)),
                ],
                false,
                2,
            );
            ledger.history_context(version, 3, 3);
            ledger.receive_equipment(
                7,
                [
                    (EquipmentSlot::Head, SlotKnowledge::Empty),
                    (EquipmentSlot::Head, SlotKnowledge::Unavailable),
                ],
                false,
                3,
            );
            ledger.history_context(version, 3, 4);
            ledger.receive_scale(
                7,
                crate::client::control::ReceivedAttribute {
                    base: 2.,
                    modifiers: vec![],
                },
                4,
            );
            let frozen = ledger.history_after(old, 4, Some(tail), 10).unwrap();
            assert_eq!(frozen.records.len(), 3);
            assert_eq!(
                frozen
                    .records
                    .iter()
                    .map(|r| r.receive_sequence)
                    .collect::<Vec<_>>(),
                [2, 3, 4]
            );
            let EntityHistoryKind::Metadata {
                entity: Some(id),
                entries: EntityHistoryValue::Retained { value },
                complete,
                ..
            } = &frozen.records[0].kind
            else {
                panic!()
            };
            assert!(!complete);
            assert_eq!(id.session(), old);
            assert_eq!(id.spawn_sequence(), 1);
            assert_eq!(value.iter().map(|e| e.index).collect::<Vec<_>>(), [6, 6, 8]);
            assert_eq!(value[0].value, EntityDataValue::Pose(1));
            assert_eq!(value[1].value, EntityDataValue::Pose(0));
            let EntityHistoryKind::Equipment {
                entries: EntityHistoryValue::Retained { value },
                complete,
                ..
            } = &frozen.records[1].kind
            else {
                panic!()
            };
            assert!(!complete);
            assert_eq!(value[0].value, SlotKnowledge::Empty);
            assert_eq!(value[1].value, SlotKnowledge::Unavailable);
            // A latest-state snapshot collapses duplicate keys; the old delta does not.
            let latest = ledger.capture_all(version, old, 4);
            assert_eq!(
                latest.entities[0].metadata[&6].value,
                EntityDataValue::Pose(0)
            );
            assert!(!latest.entities[0].metadata_complete);
            ledger.history_context(version, 3, 5);
            ledger.remove(7);
            ledger.history_context(version, 3, 6);
            spawn(&mut ledger, version, 6);
            ledger.history_context(version, 3, 7);
            ledger.receive_metadata(7, [(6, EntityDataValue::Pose(5))], true, 7);
            ledger.history_context(version, 4, 8);
            ledger.clear();
            ledger.receive_metadata(7, [], true, 8);
            ledger.receive_equipment(77, [], false, 8);
            let page = ledger
                .history_after(session(version, 4), 8, Some(tail), 20)
                .unwrap();
            assert_eq!(
                serde_json::to_value(&page.records[..3]).unwrap(),
                serde_json::to_value(&frozen.records).unwrap()
            );
            assert!(page.records.iter().any(|r| matches!(r.kind, EntityHistoryKind::Metadata { entity: Some(id), .. } if id.spawn_sequence() == 6 && id.session().world_generation == 3)));
            assert!(page.records.iter().any(|r| matches!(
                r.kind,
                EntityHistoryKind::Metadata {
                    native_id: 7,
                    entity: None,
                    ..
                }
            ) && r.session.world_generation == 4));
            assert!(page.records.iter().any(|r| matches!(
                r.kind,
                EntityHistoryKind::Equipment {
                    native_id: 77,
                    entity: None,
                    ..
                }
            )));
        }
    }

    fn item(version: MinecraftVersion, data: ItemData) -> SlotKnowledge {
        let registry = Registry::for_version(version);
        let id = registry.item("minecraft:stone").unwrap().id;
        SlotKnowledge::Item {
            item: ItemStack {
                id,
                name: "minecraft:stone".into(),
                count: 1,
                data,
            },
        }
    }

    #[test]
    fn oversized_values_keep_receipt_boundaries_without_poisoning_latest_state() {
        use EntityHistoryValue::{Retained, RetentionLimit};
        let version = MinecraftVersion::Java1_21_11;
        let mut ledger = SpawnLedger::default();
        let session = session(version, 0);
        ledger.history_context(version, 0, 1);
        spawn(&mut ledger, version, 1);
        let tail = ledger
            .history_after(session, 1, None, 1)
            .unwrap()
            .latest_cursor;
        let limit = MAX_ENTITY_HISTORY_VALUE_BYTES - std::mem::size_of::<EntityMetadataEntry>();
        for (sequence, length) in [(2, limit), (3, limit + 1)] {
            ledger.history_context(version, 0, sequence);
            ledger.receive_metadata(
                7,
                [(22, EntityDataValue::String("x".repeat(length)))],
                true,
                sequence,
            );
        }
        // Exercise both opaque legacy bytes and modern component/removal/name allocations.
        ledger.history_context(version, 0, 4);
        ledger.receive_equipment(
            7,
            [(
                EquipmentSlot::Head,
                item(
                    MinecraftVersion::Java1_16_1,
                    ItemData::LegacyNbt {
                        bytes: vec![1; MAX_ENTITY_HISTORY_VALUE_BYTES],
                    },
                ),
            )],
            true,
            4,
        );
        let definition = Registry::for_version(version)
            .item_component("minecraft:custom_data")
            .unwrap();
        ledger.history_context(version, 0, 5);
        ledger.receive_equipment(
            7,
            [(
                EquipmentSlot::Head,
                item(
                    version,
                    ItemData::ModernComponents {
                        patch: ItemComponentPatch {
                            added: vec![ItemComponent {
                                definition: definition.clone(),
                                bytes: vec![2; MAX_ENTITY_HISTORY_VALUE_BYTES],
                            }],
                            removed: vec![definition],
                        },
                    },
                ),
            )],
            false,
            5,
        );
        ledger.history_context(version, 0, 6);
        ledger.receive_scale(
            7,
            crate::client::control::ReceivedAttribute {
                base: 3.,
                modifiers: vec![crate::client::control::Modifier {
                    id: "x".repeat(MAX_ENTITY_HISTORY_VALUE_BYTES),
                    amount: 1.,
                    operation: crate::client::control::ModifierOperation::Addition,
                }],
            },
            6,
        );
        let page = ledger.history_after(session, 6, Some(tail), 10).unwrap();
        assert_eq!(page.records.len(), 5);
        assert!(matches!(
            page.records[0].kind,
            EntityHistoryKind::Metadata {
                entries: Retained { .. },
                complete: true,
                ..
            }
        ));
        assert!(matches!(
            page.records[1].kind,
            EntityHistoryKind::Metadata {
                entries: RetentionLimit,
                complete: true,
                ..
            }
        ));
        assert!(matches!(
            page.records[2].kind,
            EntityHistoryKind::Equipment {
                entries: RetentionLimit,
                complete: true,
                ..
            }
        ));
        assert!(matches!(
            page.records[3].kind,
            EntityHistoryKind::Equipment {
                entries: RetentionLimit,
                complete: false,
                ..
            }
        ));
        assert!(matches!(
            page.records[4].kind,
            EntityHistoryKind::Scale {
                attribute: RetentionLimit,
                ..
            }
        ));
        assert!(page.gap.is_none());
        let latest = ledger.capture_all(version, session, 6);
        assert!(
            matches!(&latest.entities[0].metadata[&22].value, EntityDataValue::String(s) if s.len() == limit + 1)
        );
        assert!(matches!(
            &latest.entities[0].equipment[&EquipmentSlot::Head].value,
            SlotKnowledge::Item { .. }
        ));
        assert_eq!(
            latest.entities[0].scale_attribute_unavailable_sequence,
            Some(6)
        );
        ledger.history_context(version, 0, 7);
        ledger.receive_equipment(7, [(EquipmentSlot::Head, SlotKnowledge::Empty)], true, 7);
        let later = ledger
            .history_after(session, 7, Some(page.next_cursor), 1)
            .unwrap();
        assert!(matches!(
            later.records[0].kind,
            EntityHistoryKind::Equipment {
                entries: Retained { .. },
                complete: true,
                ..
            }
        ));
    }

    #[test]
    fn count_and_byte_evictions_share_explicit_gaps_and_bounded_pages() {
        let version = MinecraftVersion::Java1_16_1;
        for payload in [0, 60_000] {
            let mut ledger = SpawnLedger::default();
            let session = session(version, 0);
            for sequence in 1..=MAX_ENTITY_HISTORY_RECORDS as u64 + 3 {
                ledger.history_context(version, 0, sequence);
                if payload == 0 {
                    ledger.history_signal(7, Some(8), None);
                } else {
                    ledger.receive_metadata(
                        7,
                        [(1, EntityDataValue::String("x".repeat(payload)))],
                        true,
                        sequence,
                    );
                }
            }
            assert!(ledger.1.bytes <= MAX_ENTITY_HISTORY_BYTES);
            assert!(ledger.1.records.len() <= MAX_ENTITY_HISTORY_RECORDS);
            if payload == 0 {
                assert_eq!(ledger.1.records.len(), MAX_ENTITY_HISTORY_RECORDS);
            } else {
                assert!(ledger.1.records.len() < 200);
            }
            let page = ledger.history_after(session, 9000, None, 1).unwrap();
            let gap = page.gap.unwrap();
            assert_eq!(gap.after, 0);
            assert_eq!(page.records[0].ordinal, gap.dropped_through + 1);
            assert!(page.has_more);
            let continuation = ledger
                .history_after(
                    session,
                    9000,
                    Some(page.next_cursor),
                    MAX_ENTITY_HISTORY_READ,
                )
                .unwrap();
            assert!(continuation.gap.is_none());
            assert_eq!(continuation.records[0].ordinal, page.records[0].ordinal + 1);
        }
    }
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
