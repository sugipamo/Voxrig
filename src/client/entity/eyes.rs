//! Eye heights at received targets with the qualified geometry inputs retained.
use super::{EntityGeometryEvidence, EntityGeometryUnavailable, EntityObservation, EntityPose};
use crate::{
    MinecraftVersion,
    client::{ObservedValue, ValueSource},
};

/// Calculated eye height for a qualified player/cow/pig/zombie geometry model.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EntityEyeHeight {
    /// Native float height above feet, explicitly computed rather than received.
    pub height: ObservedValue<f32>,
    /// Same pose, baby, scale and position evidence as `derived_geometry`.
    pub evidence: EntityGeometryEvidence,
}
impl EntityObservation {
    /// Version-correct eye height for the types qualified by `derived_geometry`.
    /// Unsupported types or missing inputs remain explicit errors. These are
    /// sampled dimensions at a packet target, not interpolated/server-current eyes.
    pub fn eye_height(&self) -> Result<EntityEyeHeight, EntityGeometryUnavailable> {
        let geometry = self.derived_geometry()?;
        let version = self.motion.entity.id.session().version;
        let super::EntityDataValue::Pose(native) = geometry.evidence.pose.value else {
            return Err(EntityGeometryUnavailable::UnknownPose);
        };
        let pose = EntityPose::from_native(version, native)
            .ok_or(EntityGeometryUnavailable::UnknownPose)?;
        let baby = matches!(
            geometry.evidence.baby.as_ref().map(|v| &v.value),
            Some(super::EntityDataValue::Bool(true))
        );
        let value = eye_height(
            version,
            self.motion.entity.type_name.as_deref().unwrap(),
            pose,
            baby,
            geometry.evidence.scale.value,
        );
        Ok(EntityEyeHeight {
            height: ObservedValue {
                value,
                source: ValueSource::Predicted,
            },
            evidence: geometry.evidence,
        })
    }
    /// Latest explicit shared swimming flag. Missing/wrong serializers stay
    /// unknown; no type-default false is turned into a receipt.
    pub fn received_swimming(&self) -> Option<ObservedValue<bool>> {
        let flags = self.metadata.get(&0)?;
        let super::EntityDataValue::Byte(value) = flags.value else {
            return None;
        };
        Some(ObservedValue {
            value: value as u8 & 0x10 != 0,
            source: flags.source,
        })
    }
}
fn eye_height(
    version: MinecraftVersion,
    name: &str,
    pose: EntityPose,
    baby: bool,
    scale: f32,
) -> f32 {
    if pose == EntityPose::Sleeping {
        return 0.2;
    }
    if name == "minecraft:player" {
        let eye = pose.player_eye_height(version).expect("qualified pose");
        return if pose == EntityPose::Dying {
            eye
        } else {
            eye * scale
        };
    }
    let base = if version == MinecraftVersion::Java1_16_1 {
        match name {
            "minecraft:cow" if baby => (1.4_f32 * 0.5) * 0.95,
            "minecraft:cow" => 1.3,
            "minecraft:zombie" if baby => 0.93,
            "minecraft:zombie" => 1.74,
            "minecraft:pig" => (0.9_f32 * if baby { 0.5 } else { 1. }) * 0.85,
            _ => unreachable!("qualified geometry type"),
        }
    } else {
        match (name, baby) {
            ("minecraft:cow", true) => 0.665,
            ("minecraft:zombie", true) => 0.93,
            _ => {
                version
                    .table()
                    .entity_dimensions(name)
                    .and_then(|d| d.eye_height)
                    .expect("qualified registered eye height") as f32
                    * if baby { 0.5 } else { 1. }
            }
        }
    };
    base * scale
}
#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;
    #[test]
    fn eye_heights_match_931_original_methods_including_babies_all_poses_and_scale() {
        let fixture: serde_json::Value = serde_json::from_reader(flate2::read::GzDecoder::new(
            include_bytes!("../../../data/client_api/eye_height_oracle.json.gz").as_slice(),
        ))
        .unwrap();
        assert_eq!(
            fixture["runner_sha256"],
            format!(
                "{:x}",
                sha2::Sha256::digest(include_bytes!("../../../scripts/run_eye_height_oracle.py"))
            )
        );
        let mut count = 0;
        for (version, label, source) in [
            (
                MinecraftVersion::Java1_16_1,
                "1.16.1",
                include_bytes!(
                    "../../../scripts/eye_height_oracle/1.16.1/EntityDimensionsOracle.java"
                )
                .as_slice(),
            ),
            (
                MinecraftVersion::Java1_21_11,
                "1.21.11",
                include_bytes!(
                    "../../../scripts/eye_height_oracle/1.21.11/EntityDimensionsOracle.java"
                )
                .as_slice(),
            ),
        ] {
            let data = &fixture["versions"][label];
            assert_eq!(
                data["harness_sha256"],
                format!("{:x}", sha2::Sha256::digest(source))
            );
            for row in data["results"].as_array().unwrap() {
                let pose =
                    EntityPose::from_native(version, row["pose"].as_i64().unwrap() as i32).unwrap();
                let actual = eye_height(
                    version,
                    row["type"].as_str().unwrap(),
                    pose,
                    row["baby"].as_bool().unwrap(),
                    row["effective_scale"].as_f64().unwrap() as f32,
                );
                assert_eq!(
                    u64::from(actual.to_bits()),
                    row["eye_height_bits"].as_u64().unwrap(),
                    "{label} {row}"
                );
                count += 1;
            }
        }
        assert_eq!(count, 931);
    }
}
