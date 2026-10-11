//! Original living sleep predicate at a received metadata boundary.
use super::{EntityDataField, EntityDataSource, EntityDataValue, EntityObservation};
use crate::client::BlockPos;

/// Latest explicit sleeping-position receipt, independently of the pose field.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct EntitySleepingReading {
    /// Original version-specific metadata accessor index.
    pub index: u8,
    /// Original sleeping block, or an explicit empty optional value.
    pub position: Option<BlockPos>,
    /// Original `LivingEntity.isSleeping` predicate: the position is present.
    /// This is a received-state interpretation, not current server state.
    pub sleeping: bool,
    /// Original packet ordinal, not a tick or an execution acknowledgement.
    pub source: EntityDataSource,
    /// Whether all metadata packets in this spawn lifetime were decoded fully.
    pub metadata_complete: bool,
}

impl EntityObservation {
    /// Latest explicit original sleeping-position receipt. `None` means no
    /// supported receipt, an unknown type, a nonliving type or a different
    /// serializer at the accessor. An explicit empty position returns a reading
    /// with `sleeping == false`. Pose alone cannot determine this predicate.
    /// Partial metadata leaves explicit last-known values readable with
    /// `metadata_complete == false`; no missing type default is invented.
    pub fn sleeping(&self) -> Option<EntitySleepingReading> {
        let reading = self.data(EntityDataField::SleepingPosition)?;
        let EntityDataValue::OptionalBlockPosition(position) = reading.value else {
            return None;
        };
        Some(EntitySleepingReading {
            index: reading.index,
            sleeping: position.is_some(),
            position,
            source: reading.source,
            metadata_complete: self.metadata_complete,
        })
    }
}

#[cfg(test)]
pub(crate) fn original_position_packets(
    version: crate::MinecraftVersion,
) -> Vec<(Vec<u8>, EntityDataValue)> {
    use sha2::Digest;
    let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../data/client_api/entity_position_oracle.json"
    ))
    .unwrap();
    for (key, bytes) in [
        (
            "harness_sha256",
            include_bytes!("../../../scripts/EntityPositionOracle.java").as_slice(),
        ),
        (
            "runner_sha256",
            include_bytes!("../../../scripts/run_entity_position_oracle.py").as_slice(),
        ),
    ] {
        assert_eq!(
            fixture[key].as_str().unwrap(),
            format!("{:x}", sha2::Sha256::digest(bytes))
        );
    }
    let name = if version == crate::MinecraftVersion::Java1_16_1 {
        "1.16.1"
    } else {
        "1.21.11"
    };
    let mut result = Vec::new();
    for codec in fixture["versions"][name]["codecs"].as_array().unwrap() {
        let optional = codec["optional"].as_bool().unwrap();
        for row in codec["positions"].as_array().unwrap() {
            let position = row["decoded"].as_array().map(|v| BlockPos {
                x: v[0].as_i64().unwrap() as i32,
                y: v[1].as_i64().unwrap() as i32,
                z: v[2].as_i64().unwrap() as i32,
            });
            let mut payload = Vec::new();
            if optional {
                payload.push(u8::from(position.is_some()));
            }
            if let Some(position) = position {
                payload.extend(position.packed().to_be_bytes());
            }
            assert_eq!(
                row["encoded_size"].as_u64().unwrap() as usize,
                payload.len()
            );
            assert_eq!(
                row["original_codec_sha256"].as_str().unwrap(),
                format!("{:x}", sha2::Sha256::digest(&payload))
            );
            let mut packet = vec![7, 30, codec["serializer_id"].as_u64().unwrap() as u8];
            packet.extend(payload);
            packet.extend([31, 0, 42, 255]);
            result.push((
                packet,
                if optional {
                    EntityDataValue::OptionalBlockPosition(position)
                } else {
                    EntityDataValue::BlockPosition(position.unwrap())
                },
            ));
        }
    }
    assert_eq!(result.len(), 143);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        MinecraftVersion,
        client::{
            SessionStamp,
            entity::{NativeSpawn, SpawnLedger},
        },
    };

    #[test]
    fn explicit_sleep_does_not_follow_pose_or_invent_absent_defaults() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let session = SessionStamp {
                version,
                connection_id: 1,
                world_generation: 0,
            };
            let mut ledger = SpawnLedger::default();
            let spawn = || NativeSpawn {
                id: 7,
                uuid: None,
                type_id: None,
                dedicated_type_name: Some("cow"),
                position: [0.; 3],
                living: Some(true),
            };
            ledger.insert(version, spawn(), 1, 16).unwrap();
            let capture =
                |ledger: &SpawnLedger| ledger.capture_all(version, session, 99).entities.remove(0);
            let index = if version == MinecraftVersion::Java1_16_1 {
                13
            } else {
                14
            };
            ledger.receive_metadata(7, [(6, EntityDataValue::Pose(2))], true, 2);
            assert!(capture(&ledger).sleeping().is_none());
            let pos = BlockPos {
                x: -33554432,
                y: -2048,
                z: 33554431,
            };
            ledger.receive_metadata(
                7,
                [
                    (6, EntityDataValue::Pose(0)),
                    (index, EntityDataValue::OptionalBlockPosition(Some(pos))),
                ],
                true,
                3,
            );
            let frozen = capture(&ledger).sleeping().unwrap();
            assert_eq!(frozen.position, Some(pos));
            assert!(frozen.sleeping);
            assert_eq!(frozen.source, EntityDataSource::Received { sequence: 3 });
            ledger.receive_metadata(
                7,
                [
                    (6, EntityDataValue::Pose(2)),
                    (index, EntityDataValue::OptionalBlockPosition(None)),
                ],
                false,
                4,
            );
            let clear = capture(&ledger).sleeping().unwrap();
            assert_eq!(clear.position, None);
            assert!(!clear.sleeping && !clear.metadata_complete);
            assert_eq!(clear.source, EntityDataSource::Received { sequence: 4 });
            assert!(frozen.sleeping && frozen.metadata_complete);
            ledger.receive_metadata(7, [(index, EntityDataValue::Other(11))], true, 5);
            assert!(capture(&ledger).sleeping().is_none());
            ledger.remove(7);
            ledger.insert(version, spawn(), 7, 16).unwrap();
            assert!(capture(&ledger).sleeping().is_none());
        }
    }
}
