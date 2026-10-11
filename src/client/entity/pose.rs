//! Version-aware original entity poses; no inferred current geometry.
use super::{EntityDataField, EntityDataSource, EntityDataValue, EntityObservation};
use crate::MinecraftVersion;

/// A common name for a pinned version's original Pose enum value.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EntityPose {
    /// Standing.
    Standing,
    /// Elytra fall flying.
    FallFlying,
    /// Sleeping; orientation is separate.
    Sleeping,
    /// Swimming.
    Swimming,
    /// Riptide spin attack.
    SpinAttack,
    /// Crouching.
    Crouching,
    /// Long jumping (Java 1.21.11).
    LongJumping,
    /// Dying.
    Dying,
    /// Frog croaking (Java 1.21.11).
    Croaking,
    /// Frog tongue use (Java 1.21.11).
    UsingTongue,
    /// Sitting (Java 1.21.11).
    Sitting,
    /// Warden roaring (Java 1.21.11).
    Roaring,
    /// Sniffing (Java 1.21.11).
    Sniffing,
    /// Emerging (Java 1.21.11).
    Emerging,
    /// Digging (Java 1.21.11).
    Digging,
    /// Sliding (Java 1.21.11).
    Sliding,
    /// Shooting (Java 1.21.11).
    Shooting,
    /// Inhaling (Java 1.21.11).
    Inhaling,
}
impl EntityPose {
    /// Resolve an original native identifier in the selected version. Unknown
    /// IDs stay unknown; in particular legacy 6 is Dying, modern 6 LongJumping.
    pub fn from_native(version: MinecraftVersion, native_id: i32) -> Option<Self> {
        match (version, native_id) {
            (_, 0) => Some(Self::Standing),
            (_, 1) => Some(Self::FallFlying),
            (_, 2) => Some(Self::Sleeping),
            (_, 3) => Some(Self::Swimming),
            (_, 4) => Some(Self::SpinAttack),
            (_, 5) => Some(Self::Crouching),
            (MinecraftVersion::Java1_21_11, 6) => Some(Self::LongJumping),
            (MinecraftVersion::Java1_21_11, 7) => Some(Self::Dying),
            (MinecraftVersion::Java1_21_11, 8) => Some(Self::Croaking),
            (MinecraftVersion::Java1_21_11, 9) => Some(Self::UsingTongue),
            (MinecraftVersion::Java1_21_11, 10) => Some(Self::Sitting),
            (MinecraftVersion::Java1_21_11, 11) => Some(Self::Roaring),
            (MinecraftVersion::Java1_21_11, 12) => Some(Self::Sniffing),
            (MinecraftVersion::Java1_21_11, 13) => Some(Self::Emerging),
            (MinecraftVersion::Java1_21_11, 14) => Some(Self::Digging),
            (MinecraftVersion::Java1_21_11, 15) => Some(Self::Sliding),
            (MinecraftVersion::Java1_21_11, 16) => Some(Self::Shooting),
            (MinecraftVersion::Java1_21_11, 17) => Some(Self::Inhaling),
            (MinecraftVersion::Java1_16_1, 6) => Some(Self::Dying),
            _ => None,
        }
    }
}

/// One explicit original pose field at an entity observation boundary.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct EntityPoseReading {
    /// Original spawn's protocol version; native numbers are version-specific.
    pub version: MinecraftVersion,
    /// Original metadata accessor index.
    pub index: u8,
    /// Native value retained even when it cannot be resolved to a known pose.
    pub native_id: i32,
    /// Known common name; None for unknown/invalid native identifiers.
    pub pose: Option<EntityPose>,
    /// Original receive ordinal; this is neither current server state nor a
    /// geometry/animation acknowledgement.
    pub source: EntityDataSource,
    /// No metadata packet in this spawn lifetime was incompletely decoded.
    /// False remains latched, keeping explicit last-known fields readable
    /// without asserting a complete state.
    pub metadata_complete: bool,
}
impl EntityObservation {
    /// Latest explicit original pose receipt. None until the pose is received,
    /// for unknown entity types or for another serializer at the pose index.
    /// No default pose, scaled box, interpolation or collision is synthesized.
    pub fn pose(&self) -> Option<EntityPoseReading> {
        let reading = self.data(EntityDataField::Pose)?;
        let EntityDataValue::Pose(native_id) = reading.value else {
            return None;
        };
        let version = self.motion.entity.id.session().version;
        Some(EntityPoseReading {
            version,
            index: reading.index,
            native_id,
            pose: EntityPose::from_native(version, native_id),
            source: reading.source,
            metadata_complete: self.metadata_complete,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{
        SessionStamp,
        entity::{NativeSpawn, SpawnLedger},
    };
    fn ledger(version: MinecraftVersion) -> SpawnLedger {
        let mut ledger = SpawnLedger::default();
        let kind = crate::client::registry::Registry::for_version(version)
            .builtin_id("minecraft:entity_type", "minecraft:cow")
            .unwrap()
            .value();
        ledger
            .insert(
                version,
                NativeSpawn {
                    living: None,
                    id: 7,
                    uuid: None,
                    type_id: Some(kind),
                    dedicated_type_name: None,
                    position: [0.; 3],
                },
                1,
                16,
            )
            .unwrap();
        ledger
    }
    fn snapshot(ledger: &SpawnLedger, version: MinecraftVersion) -> EntityObservation {
        ledger
            .capture_all(
                version,
                SessionStamp {
                    version,
                    connection_id: 1,
                    world_generation: 0,
                },
                100,
            )
            .entities
            .remove(0)
    }
    #[test]
    fn common_pose_names_match_all_original_codec_roundtrips() {
        let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../data/client_api/entity_pose_oracle.json"
        ))
        .unwrap();
        assert_eq!(
            fixture["harness_sha256"].as_str().unwrap(),
            format!(
                "{:x}",
                sha2::Sha256::digest(include_bytes!("../../../scripts/EntityPoseOracle.java"))
            )
        );
        assert_eq!(
            fixture["runner_sha256"].as_str().unwrap(),
            format!(
                "{:x}",
                sha2::Sha256::digest(include_bytes!("../../../scripts/run_entity_pose_oracle.py"))
            )
        );
        let mut count = 0;
        for (version, label) in [
            (MinecraftVersion::Java1_16_1, "1.16.1"),
            (MinecraftVersion::Java1_21_11, "1.21.11"),
        ] {
            let input = &fixture["versions"][label];
            let mut ledger = ledger(version);
            assert!(snapshot(&ledger, version).pose().is_none());
            for row in input["poses"].as_array().unwrap() {
                let id = row["native_id"].as_i64().unwrap() as i32;
                assert_eq!(row["original_roundtrip"], true);
                ledger.receive_metadata(7, [(6, EntityDataValue::Pose(id))], true, 2 + id as u64);
                let reading = snapshot(&ledger, version).pose().unwrap();
                assert_eq!(reading.native_id, id);
                assert_eq!(
                    reading.index,
                    input["metadata_index"].as_u64().unwrap() as u8
                );
                assert_eq!(
                    serde_json::to_value(reading.pose.unwrap()).unwrap(),
                    row["name"].as_str().unwrap().to_ascii_lowercase()
                );
                assert_eq!(
                    reading.source,
                    EntityDataSource::Received {
                        sequence: 2 + id as u64
                    }
                );
                assert!(reading.metadata_complete);
                count += 1;
            }
        }
        assert_eq!(count, 25);
        assert_eq!(
            EntityPose::from_native(MinecraftVersion::Java1_16_1, 6),
            Some(EntityPose::Dying)
        );
        assert_eq!(
            EntityPose::from_native(MinecraftVersion::Java1_21_11, 6),
            Some(EntityPose::LongJumping)
        );
    }
    #[test]
    fn unknown_wrong_serializer_partial_metadata_and_reused_spawn_do_not_invent_poses() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let mut ledger = ledger(version);
            ledger.receive_metadata(7, [(6, EntityDataValue::Int(5))], true, 2);
            assert!(snapshot(&ledger, version).pose().is_none());
            ledger.receive_metadata(7, [(6, EntityDataValue::Pose(5))], true, 3);
            let prior = snapshot(&ledger, version).pose().unwrap();
            assert_eq!(prior.pose, Some(EntityPose::Crouching));
            ledger.receive_metadata(7, [], false, 4);
            let partial = snapshot(&ledger, version).pose().unwrap();
            assert!(!partial.metadata_complete);
            assert_eq!(partial.source, prior.source);
            assert_eq!(partial.pose, prior.pose);
            for (sequence, id) in [(5, -1), (6, i32::MAX)] {
                ledger.receive_metadata(7, [(6, EntityDataValue::Pose(id))], true, sequence);
                let reading = snapshot(&ledger, version).pose().unwrap();
                assert_eq!(reading.native_id, id);
                assert_eq!(reading.pose, None);
                assert!(!reading.metadata_complete);
            }
            let kind = crate::client::registry::Registry::for_version(version)
                .builtin_id("minecraft:entity_type", "minecraft:cow")
                .unwrap()
                .value();
            ledger
                .insert(
                    version,
                    NativeSpawn {
                        living: None,
                        id: 7,
                        uuid: None,
                        type_id: Some(kind),
                        dedicated_type_name: None,
                        position: [0.; 3],
                    },
                    50,
                    16,
                )
                .unwrap();
            assert!(snapshot(&ledger, version).pose().is_none());
        }
    }
    use sha2::Digest;
}
