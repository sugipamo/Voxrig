//! Complete supported item-stack metadata, alongside the compatibility scalar projection.
use super::{EntityId, SpawnLedger};
use crate::client::{ObservedValue, SessionStamp, SlotKnowledge};

/// One explicit received item-stack metadata accessor, including empty/unsupported stacks.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EntityItemStackObservation {
    /// Connection, world and original spawn lifetime.
    pub entity: EntityId,
    /// Native metadata accessor index (version/type specific).
    pub index: u8,
    /// Complete supported stack with exact NBT/component patch, or Unavailable.
    pub stack: ObservedValue<SlotKnowledge>,
}

/// Received item-stack metadata for all surviving entities at one boundary.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EntityItemStacksObservation {
    /// Owning connection/world.
    pub session: SessionStamp,
    /// Capture receive boundary, independent of each stack's receipt.
    pub receive_sequence: u64,
    /// Only explicitly received item-stack accessors; no spawn defaults are invented.
    pub stacks: Vec<EntityItemStackObservation>,
}
impl SpawnLedger {
    pub(crate) fn collected_stack(
        &self,
        session: SessionStamp,
        native: i32,
    ) -> Option<EntityItemStackObservation> {
        let spawn = self.0.get(&native)?;
        if spawn.name.as_deref() != Some("minecraft:item") {
            return None;
        }
        let index = match session.version {
            crate::MinecraftVersion::Java1_16_1 => 7,
            crate::MinecraftVersion::Java1_21_11 => 8,
        };
        Some(EntityItemStackObservation {
            entity: self.identity(session, native)?,
            index,
            stack: spawn.extra.item_metadata.get(&index)?.clone(),
        })
    }
    pub(crate) fn receive_item_metadata(
        &mut self,
        native_id: i32,
        items: impl IntoIterator<Item = (u8, SlotKnowledge)>,
        sequence: u64,
    ) {
        if let Some(spawn) = self.0.get_mut(&native_id) {
            for (index, value) in items {
                spawn
                    .extra
                    .item_metadata
                    .insert(index, crate::client::received(value, sequence));
            }
        }
    }
    pub(crate) fn capture_items(
        &self,
        session: SessionStamp,
        receive_sequence: u64,
    ) -> EntityItemStacksObservation {
        EntityItemStacksObservation {
            session,
            receive_sequence,
            stacks: self
                .0
                .iter()
                .flat_map(|(&native_id, spawn)| {
                    spawn
                        .extra
                        .item_metadata
                        .iter()
                        .map(move |(&index, stack)| EntityItemStackObservation {
                            entity: EntityId {
                                session,
                                native_id,
                                spawn_sequence: spawn.sequence,
                            },
                            index,
                            stack: stack.clone(),
                        })
                })
                .collect(),
        }
    }
}
impl crate::Client {
    /// Explicit received entity item-stack metadata, including dropped items.
    /// Keeps the historical `EntityDataValue::Other` projection source-compatible.
    /// Missing entries are unknown, never empty; item data uses the same lossless
    /// representation as received inventory. Removed/reused entity IDs retire old stacks.
    pub async fn entity_item_stacks(&self) -> crate::Result<EntityItemStacksObservation> {
        crate::client::dispatch!(&self.adapter, a => crate::client::adapter::CoreOps::entity_item_stacks(a).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        MinecraftVersion,
        client::{entity::NativeSpawn, received, registry::Registry},
    };
    #[test]
    fn item_receipts_follow_spawn_lifetimes_and_replacement_serializers_in_both_versions() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let session = SessionStamp {
                version,
                connection_id: 3,
                world_generation: 7,
            };
            let mut ledger = SpawnLedger::default();
            let native = || NativeSpawn {
                living: None,
                id: 7,
                uuid: None,
                type_id: Some(
                    Registry::for_version(version)
                        .builtin_id("minecraft:entity_type", "minecraft:item")
                        .unwrap()
                        .value(),
                ),
                dedicated_type_name: None,
                position: [0.; 3],
            };
            ledger.insert(version, native(), 1, 16).unwrap();
            assert!(ledger.capture_items(session, 2).stacks.is_empty());
            let index = if version == MinecraftVersion::Java1_16_1 {
                7
            } else {
                8
            };
            ledger.receive_item_metadata(7, [(index, SlotKnowledge::Empty)], 3);
            let observed = ledger.capture_items(session, 10);
            assert_eq!(observed.receive_sequence, 10);
            assert_eq!(observed.stacks[0].stack, received(SlotKnowledge::Empty, 3));
            assert_eq!(observed.stacks[0].entity.spawn_sequence(), 1);
            ledger.receive_metadata(
                7,
                [(index, super::super::EntityDataValue::Byte(0))],
                true,
                11,
            );
            assert!(ledger.capture_items(session, 11).stacks.is_empty());
            ledger.receive_item_metadata(7, [(index, SlotKnowledge::Empty)], 12);
            ledger.remove(7);
            ledger.insert(version, native(), 14, 16).unwrap();
            assert!(ledger.capture_items(session, 15).stacks.is_empty());
        }
    }
}
