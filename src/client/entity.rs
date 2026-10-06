//! Received entity lifetimes and one-shot native interactions.
//! Spawn coordinates are historical receipts, never a current position or a reach test.
use super::{
    ObservedValue, SessionStamp,
    registry::{BuiltinRegistryId, Registry},
};
use crate::{MinecraftVersion, Result};
use std::collections::BTreeMap;

/// An original received spawn on one connection/world, independent of reusable numeric IDs.
/// Saved diagnostics cannot construct live targets.
/// ```compile_fail
/// let target: voxrig::client::EntityId = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct EntityId {
    session: SessionStamp,
    native_id: i32,
    spawn_sequence: u64,
}
impl EntityId {
    /// Original connection and world.
    pub fn session(self) -> SessionStamp {
        self.session
    }
    /// Native packet identifier, for diagnostics; integers cannot create targets.
    pub fn native_id(self) -> i32 {
        self.native_id
    }
    /// Receive ordinal of this exact spawn, not its current position.
    pub fn spawn_sequence(self) -> u64 {
        self.spawn_sequence
    }
}

/// A spawn still present in the received lifetime ledger.
/// Metadata, motion, hitboxes, line of sight and server-current health are not inferred.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EntitySpawn {
    /// Original received lifetime, checked again before interaction I/O.
    pub id: EntityId,
    /// UUID if this version's spawn packet supplies it (legacy experience orbs do not).
    pub uuid: Option<[u8; 16]>,
    /// Exact version's known entity-type definition, absent for unknown types.
    pub entity_type: Option<BuiltinRegistryId>,
    /// Namespaced type name only when resolved in this version's registry.
    pub type_name: Option<String>,
    /// Explicit native type field, absent for legacy dedicated spawn packet kinds.
    pub native_type_id: Option<i32>,
    /// Coordinates as supplied by the spawn packet; not advanced by later movement.
    /// Legacy painting packets supply a block anchor rather than entity feet.
    pub spawn_position: ObservedValue<[f64; 3]>,
}
/// Coherent received spawn/despawn ledger; excludes the own player and unreceived entities.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EntitySpawns {
    /// Owning connection/world.
    pub session: SessionStamp,
    /// Capture's receive boundary, separate from each spawn ordinal.
    pub receive_sequence: u64,
    /// Surviving received spawns, ordered by native ID.
    pub entities: Vec<EntitySpawn>,
}

#[derive(Clone, Copy)]
pub(crate) enum EntityAction {
    Interact { hand: super::Hand, sneaking: bool },
    Attack { sneaking: bool },
}
impl EntityAction {
    // Both pinned native serializers: VarInt entity ID, enum ordinal, action
    // fields (hand only for INTERACT), then the secondary-action boolean.
    pub(crate) fn payload(self, target: EntityId) -> Vec<u8> {
        let mut payload = Vec::new();
        crate::protocol::put_varint(&mut payload, target.native_id);
        match self {
            Self::Interact { hand, sneaking } => {
                crate::protocol::put_varint(&mut payload, 0);
                crate::protocol::put_varint(&mut payload, hand as i32);
                payload.push(u8::from(sneaking));
            }
            Self::Attack { sneaking } => {
                crate::protocol::put_varint(&mut payload, 1);
                payload.push(u8::from(sneaking));
            }
        }
        payload
    }
}

#[derive(Default)]
pub(crate) struct SpawnLedger(BTreeMap<i32, Spawn>);
struct Spawn {
    sequence: u64,
    uuid: Option<[u8; 16]>,
    native_type: Option<i32>,
    entity_type: Option<BuiltinRegistryId>,
    name: Option<String>,
    position: [f64; 3],
}
pub(crate) struct NativeSpawn {
    pub id: i32,
    pub uuid: Option<[u8; 16]>,
    pub type_id: Option<i32>,
    pub dedicated_type_name: Option<&'static str>,
    pub position: [f64; 3],
}
impl SpawnLedger {
    pub(crate) fn spawn_sequence(&self, native_id: i32) -> Option<u64> {
        self.0.get(&native_id).map(|spawn| spawn.sequence)
    }
    pub(crate) fn identity(&self, session: SessionStamp, native_id: i32) -> Option<EntityId> {
        self.0.get(&native_id).map(|spawn| EntityId {
            session,
            native_id,
            spawn_sequence: spawn.sequence,
        })
    }
    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }
    pub(crate) fn remove(&mut self, id: i32) {
        self.0.remove(&id);
    }
    pub(crate) fn insert(
        &mut self,
        version: MinecraftVersion,
        native: NativeSpawn,
        sequence: u64,
        limit: usize,
    ) -> Result<()> {
        if native.id < 0 || native.position.iter().any(|v| !v.is_finite()) {
            return Err(super::registry::invalid(
                "invalid entity spawn identity/coordinates",
            ));
        }
        if !self.0.contains_key(&native.id) && self.0.len() >= limit {
            return Err(super::inventory::unavailable(
                "entity spawn ledger limit exceeded",
            ));
        }
        let registry = Registry::for_version(version);
        let entity_type = match native.type_id {
            Some(id) => registry
                .builtin_id_by_native_id("minecraft:entity_type", id)
                .ok(),
            None => native.dedicated_type_name.and_then(|name| {
                registry
                    .builtin_id("minecraft:entity_type", &format!("minecraft:{name}"))
                    .ok()
            }),
        };
        let name = entity_type
            .as_ref()
            .map(|id| registry.builtin_name(id).map(str::to_owned))
            .transpose()?;
        self.0.insert(
            native.id,
            Spawn {
                sequence,
                uuid: native.uuid,
                native_type: native.type_id,
                entity_type,
                name,
                position: native.position,
            },
        );
        Ok(())
    }
    pub(crate) fn capture(&self, session: SessionStamp, receive_sequence: u64) -> EntitySpawns {
        EntitySpawns {
            session,
            receive_sequence,
            entities: self
                .0
                .iter()
                .map(|(&native_id, spawn)| EntitySpawn {
                    id: EntityId {
                        session,
                        native_id,
                        spawn_sequence: spawn.sequence,
                    },
                    uuid: spawn.uuid,
                    entity_type: spawn.entity_type.clone(),
                    type_name: spawn.name.clone(),
                    native_type_id: spawn.native_type,
                    spawn_position: super::received(spawn.position, spawn.sequence),
                })
                .collect(),
        }
    }
    pub(crate) fn validate(&self, session: SessionStamp, target: EntityId) -> Result<()> {
        if target.session != session
            || !self
                .0
                .get(&target.native_id)
                .is_some_and(|spawn| spawn.sequence == target.spawn_sequence)
        {
            return Err(super::inventory::unavailable(
                "entity target belongs to another connection/world or a retired spawn",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn entity_lifetime_reuse_world_and_connection_are_distinct() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let mut ledger = SpawnLedger::default();
            let spawn = || NativeSpawn {
                id: 42,
                uuid: Some([7; 16]),
                type_id: None,
                dedicated_type_name: Some("sheep"),
                position: [1.0, 65.0, 0.5],
            };
            let session = SessionStamp {
                version,
                connection_id: 1,
                world_generation: 4,
            };
            ledger.insert(version, spawn(), 10, 1).unwrap();
            let received = ledger.capture(session, 20);
            let old = received.entities[0].id;
            assert_eq!(
                received.entities[0].type_name.as_deref(),
                Some("minecraft:sheep")
            );
            ledger.validate(session, old).unwrap();
            assert!(
                ledger
                    .validate(
                        SessionStamp {
                            connection_id: 2,
                            ..session
                        },
                        old
                    )
                    .is_err()
            );
            assert!(
                ledger
                    .validate(
                        SessionStamp {
                            world_generation: 5,
                            ..session
                        },
                        old
                    )
                    .is_err()
            );
            ledger.remove(42);
            assert!(ledger.validate(session, old).is_err());
            ledger.insert(version, spawn(), 21, 1).unwrap();
            assert!(ledger.validate(session, old).is_err());
            let current = ledger.capture(session, 22).entities[0].id;
            ledger.validate(session, current).unwrap();
            assert_ne!(old, current); // Even the same UUID is a new received lifetime.
            assert_eq!(
                EntityAction::Attack { sneaking: false }.payload(current),
                [42, 1, 0]
            );
            assert_eq!(
                EntityAction::Interact {
                    hand: super::super::Hand::Off,
                    sneaking: true
                }
                .payload(current),
                [42, 0, 1, 1]
            );
            ledger.clear();
            assert!(ledger.validate(session, current).is_err());
        }
    }
}
