//! Current received entity state at one capture: motion, equipment, health and
//! a default-dimension bounding box. Nothing is predicted or interpolated.
use super::{EntityId, EntityMotionObservation, SpawnLedger};
use crate::MinecraftVersion;
use crate::client::{Aabb, ObservedValue, SessionStamp, SlotKnowledge};
use std::collections::BTreeMap;

/// Equipment slot of an entity, in native order.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash, serde::Serialize)]
pub enum EquipmentSlot {
    /// Main hand.
    MainHand,
    /// Off hand.
    OffHand,
    /// Boots.
    Feet,
    /// Leggings.
    Legs,
    /// Chestplate.
    Chest,
    /// Helmet.
    Head,
    /// Horse and wolf body armor (Java 1.21.11).
    Body,
    /// Saddle (Java 1.21.11).
    Saddle,
}
impl EquipmentSlot {
    /// Native equipment slot index of the selected version.
    pub(crate) fn from_native(version: MinecraftVersion, index: u8) -> Option<Self> {
        version.table().equipment_slot(index)
    }
}

/// One received entity at a capture boundary.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EntityObservation {
    /// Identity, type and latest received motion fields.
    pub motion: EntityMotionObservation,
    /// Box from the type's default dimensions at the latest received position
    /// (or the spawn position). Pose, scale and baby variants are not applied.
    /// None when the type is unknown or has no position.
    pub bounding_box: Option<Aabb>,
    /// Latest received health of a living entity. None until a metadata packet
    /// supplies it; this is client-received state, not server-current health.
    pub health: Option<ObservedValue<f32>>,
    /// Latest received item per equipment slot.
    pub equipment: BTreeMap<EquipmentSlot, ObservedValue<SlotKnowledge>>,
}

/// All received entities at one boundary (the own player is excluded).
#[derive(Clone, Debug, serde::Serialize)]
pub struct EntitiesObservation {
    /// Connection and world.
    pub session: SessionStamp,
    /// Receive boundary of the capture.
    pub receive_sequence: u64,
    /// Entities ordered by native id.
    pub entities: Vec<EntityObservation>,
}

#[derive(Default)]
pub(super) struct Extra {
    pub health: Option<ObservedValue<f32>>,
    pub equipment: BTreeMap<EquipmentSlot, ObservedValue<SlotKnowledge>>,
}

/// Default (width, height) of a namespaced entity type.
pub(crate) fn dimensions(version: MinecraftVersion, name: &str) -> Option<(f64, f64)> {
    version
        .table()
        .entity_dimensions(name)
        .map(|row| (row.width, row.height))
}

/// Whether a Java 1.21.11 entity type is living (has default attributes).
pub(crate) fn modern_living(name: &str) -> bool {
    MinecraftVersion::Java1_21_11
        .table()
        .entity_dimensions(name)
        .and_then(|row| row.living)
        .unwrap_or(false)
}

impl SpawnLedger {
    /// Record health from a living entity's metadata.
    pub(crate) fn receive_health(&mut self, native_id: i32, health: f32, sequence: u64) {
        if let Some(spawn) = self.0.get_mut(&native_id) {
            spawn.extra.health = Some(ObservedValue {
                value: health,
                source: crate::client::ValueSource::Received { sequence },
            });
        }
    }
    /// Record one received equipment slot.
    pub(crate) fn receive_equipment(
        &mut self,
        native_id: i32,
        slot: EquipmentSlot,
        item: SlotKnowledge,
        sequence: u64,
    ) {
        if let Some(spawn) = self.0.get_mut(&native_id) {
            spawn.extra.equipment.insert(
                slot,
                ObservedValue {
                    value: item,
                    source: crate::client::ValueSource::Received { sequence },
                },
            );
        }
    }
    /// Namespaced type name of a live spawn, when resolved.
    pub(crate) fn type_name(&self, native_id: i32) -> Option<&str> {
        self.0
            .get(&native_id)
            .and_then(|spawn| spawn.name.as_deref())
    }
    /// Every surviving spawn with its latest received state.
    pub(crate) fn capture_all(
        &self,
        version: MinecraftVersion,
        session: SessionStamp,
        receive_sequence: u64,
    ) -> EntitiesObservation {
        let entities = self
            .0
            .iter()
            .map(|(&native_id, spawn)| {
                let id = EntityId {
                    session,
                    native_id,
                    spawn_sequence: spawn.sequence,
                };
                let motion = self
                    .capture_motion(session, id, receive_sequence)
                    .expect("identity taken from this ledger");
                let feet = motion
                    .position
                    .as_ref()
                    .map(|p| p.value.position)
                    .unwrap_or(spawn.position);
                let bounding_box = spawn
                    .name
                    .as_deref()
                    .and_then(|name| dimensions(version, name))
                    .map(|(width, height)| Aabb {
                        min_x: feet[0] - width / 2.0,
                        min_y: feet[1],
                        min_z: feet[2] - width / 2.0,
                        max_x: feet[0] + width / 2.0,
                        max_y: feet[1] + height,
                        max_z: feet[2] + width / 2.0,
                    });
                EntityObservation {
                    motion,
                    bounding_box,
                    health: spawn.extra.health.clone(),
                    equipment: spawn.extra.equipment.clone(),
                }
            })
            .collect();
        EntitiesObservation {
            session,
            receive_sequence,
            entities,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::NativeSpawn;
    use super::*;

    #[test]
    fn default_dimensions_come_from_each_versions_data() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            assert_eq!(dimensions(version, "minecraft:pig"), Some((0.9, 0.9)));
            assert_eq!(dimensions(version, "minecraft:zombie"), Some((0.6, 1.95)));
        }
        assert!(modern_living("minecraft:armor_stand"));
        assert!(!modern_living("minecraft:interaction"));
        assert_eq!(
            dimensions(MinecraftVersion::Java1_16_1, "minecraft:breeze"),
            None
        );
    }

    #[test]
    fn captures_combine_motion_health_equipment_and_box() {
        let version = MinecraftVersion::Java1_21_11;
        let session = SessionStamp {
            version,
            connection_id: 1,
            world_generation: 0,
        };
        let pig = crate::client::registry::Registry::for_version(version)
            .builtin_id("minecraft:entity_type", "minecraft:pig")
            .unwrap();
        let mut ledger = SpawnLedger::default();
        ledger
            .insert(
                version,
                NativeSpawn {
                    id: 7,
                    uuid: None,
                    type_id: Some(pig.value()),
                    dedicated_type_name: None,
                    position: [10.0, 64.0, -2.0],
                },
                3,
                16,
            )
            .unwrap();
        ledger.receive_health(7, 4.5, 5);
        ledger.receive_equipment(7, EquipmentSlot::Saddle, SlotKnowledge::Empty, 6);
        ledger.receive_health(99, 1.0, 7); // Unknown entities are ignored.
        let all = ledger.capture_all(version, session, 8);
        assert_eq!(all.entities.len(), 1);
        let pig = &all.entities[0];
        assert_eq!(pig.health.as_ref().unwrap().value, 4.5);
        assert_eq!(pig.equipment.len(), 1);
        let b = pig.bounding_box.unwrap();
        assert_eq!(
            [b.min_x, b.min_y, b.max_x, b.max_y],
            [9.55, 64.0, 10.45, 64.9]
        );
    }
}
