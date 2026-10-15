//! Explicit dimension models at received packet targets, never current collision.
use super::{
    EntityDataField, EntityDataReading, EntityDataSource, EntityDataValue, EntityId,
    EntityObservation, EntityPose,
};
use crate::{
    MinecraftVersion,
    client::{Aabb, ObservedValue, ValueSource, control::ReceivedAttribute},
};

/// The original dimension calculation qualified for this entity type.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum EntityGeometryModel {
    /// Original player pose dimensions; modern scale except fixed sleeping/dying dimensions.
    PlayerPose,
    /// Original cow, pig or zombie dimensions, including baby and modern scale.
    AgeablePose,
}

/// Why a sampled dimension model cannot be evaluated at this capture.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum EntityGeometryUnavailable {
    /// No qualified dimension model for this type.
    UnsupportedType,
    /// No coherent position target, e.g. an unresolved relative correction.
    MissingPosition,
    /// Pose is unknown, uses a different serializer or is absent after partial metadata.
    UnknownPose,
    /// Required baby data is unknown or uses another serializer.
    UnknownBaby,
    /// Modern scale has not been explicitly received or cannot be evaluated.
    UnknownScale,
    /// Position, dimension or bound is not finite.
    NonFiniteState,
}

/// Native effective scale and its original input. Java 1.16.1 has no scale
/// attribute: its invariant unit scale has `attribute: None`.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct EntityScaleReading {
    /// Native range-clamped attribute converted to float before dimension scaling.
    pub value: f32,
    /// Original base, modifiers and receive ordinal; absent only on Java 1.16.1.
    pub attribute: Option<ObservedValue<ReceivedAttribute>>,
}

/// Receipt inputs and explicit defaults for a sampled dimension model.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EntityGeometryEvidence {
    /// Capture ordinal, independent of the individual field ordinals.
    pub receive_sequence: u64,
    /// Original target position source; bounds do not interpolate its quantization.
    pub position_source: ValueSource,
    /// Original pose receipt or explicit original standing type default.
    pub pose: EntityDataReading,
    /// Baby receipt/type default for qualified mobs; None for players.
    pub baby: Option<EntityDataReading>,
    /// Explicit modern scale receipt or legacy invariant.
    pub scale: EntityScaleReading,
    /// False if any metadata packet in this lifetime was partially decoded.
    pub metadata_complete: bool,
}

/// Derived dimensions at a received target. This is neither a server-current
/// hitbox nor evidence of collision, reach, line of sight, seat ownership or push.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EntityGeometry {
    /// Connection/world/spawn lifetime whose inputs produced this model.
    pub entity: EntityId,
    /// Qualified native dimension algorithm.
    pub model: EntityGeometryModel,
    /// Width after native float operations, in blocks.
    pub width: f32,
    /// Height after native float operations, in blocks.
    pub height: f32,
    /// Native box at the last decoded packet target, without interpolation.
    pub bounds: Aabb,
    /// Per-axis packet quantization error, separate from the sampled box.
    pub position_error: [f64; 3],
    /// Original receipt sources and explicit defaults.
    pub evidence: EntityGeometryEvidence,
}

impl EntityObservation {
    /// Latest received modern scale folded in native modifier-map order and
    /// clamped to its original range. At most 16 modifiers are modeled; more
    /// remain readable within its 4096-byte retention budget. Never supplies
    /// a missing modern default.
    pub fn scale(&self) -> Option<EntityScaleReading> {
        let version = self.motion.entity.id.session().version;
        if version == MinecraftVersion::Java1_16_1 {
            return Some(EntityScaleReading {
                value: 1.,
                attribute: None,
            });
        }
        let attribute = self.scale_attribute.as_ref()?;
        if !matches!(attribute.source, ValueSource::Received { .. })
            || attribute.value.modifiers.len() > 16
            || !attribute.value.base.is_finite()
            || attribute
                .value
                .modifiers
                .iter()
                .any(|m| !m.amount.is_finite())
        {
            return None;
        }
        let modifiers = attribute.value.modifiers.iter().collect::<Vec<_>>();
        let value =
            crate::client::physics::attribute_value(version, attribute.value.base, &modifiers);
        if !value.is_finite() {
            return None;
        }
        Some(EntityScaleReading {
            value: value.clamp(0.0625, 16.) as f32,
            attribute: Some(attribute.clone()),
        })
    }

    /// Qualified player/cow/pig/zombie dimension model. `bounding_box` remains
    /// the historical default-size API. Missing or unsupported inputs produce
    /// an explicit error; this model never enables movement or entity pushing.
    pub fn derived_geometry(&self) -> Result<EntityGeometry, EntityGeometryUnavailable> {
        use EntityGeometryUnavailable as U;
        let name = self
            .motion
            .entity
            .type_name
            .as_deref()
            .ok_or(U::UnsupportedType)?;
        let version = self.motion.entity.id.session().version;
        let model = super::contact_policy::selectors::public_geometry(version, name)
            .ok_or(U::UnsupportedType)?;
        let position = self.motion.position.as_ref().ok_or(U::MissingPosition)?;
        // The original constructor's standing default is a model input, not a
        // fabricated pose receipt. Partial packets suppress absent defaults.
        let pose_reading = self
            .data(EntityDataField::Pose)
            .or_else(|| {
                (!self.metadata.contains_key(&6) && self.metadata_complete).then_some(
                    EntityDataReading {
                        index: 6,
                        value: EntityDataValue::Pose(0),
                        source: EntityDataSource::TypeDefault,
                    },
                )
            })
            .ok_or(U::UnknownPose)?;
        let EntityDataValue::Pose(native) = pose_reading.value else {
            return Err(U::UnknownPose);
        };
        let pose = EntityPose::from_native(version, native).ok_or(U::UnknownPose)?;
        let baby = if model == EntityGeometryModel::AgeablePose {
            Some(self.data(EntityDataField::Baby).ok_or(U::UnknownBaby)?)
        } else {
            None
        };
        let baby_factor = match baby.as_ref().map(|b| &b.value) {
            Some(EntityDataValue::Bool(true)) => 0.5f32,
            Some(EntityDataValue::Bool(false)) | None => 1.,
            _ => return Err(U::UnknownBaby),
        };
        let scale = self.scale().ok_or(U::UnknownScale)?;
        let (width, height) = shape(version, name, pose, baby_factor, scale.value);
        let p = position.value.position;
        let half = f64::from(width / 2.);
        let bounds = Aabb {
            min_x: p[0] - half,
            min_y: p[1],
            min_z: p[2] - half,
            max_x: p[0] + half,
            max_y: p[1] + f64::from(height),
            max_z: p[2] + half,
        };
        if !width.is_finite()
            || !height.is_finite()
            || [
                bounds.min_x,
                bounds.min_y,
                bounds.min_z,
                bounds.max_x,
                bounds.max_y,
                bounds.max_z,
            ]
            .iter()
            .any(|n| !n.is_finite())
            || position
                .value
                .quantization_error
                .iter()
                .any(|n| !n.is_finite() || *n < 0.)
        {
            return Err(U::NonFiniteState);
        }
        Ok(EntityGeometry {
            entity: self.motion.entity.id,
            model,
            width,
            height,
            bounds,
            position_error: position.value.quantization_error,
            evidence: EntityGeometryEvidence {
                receive_sequence: self.motion.receive_sequence,
                position_source: position.source,
                pose: pose_reading,
                baby,
                scale,
                metadata_complete: self.metadata_complete,
            },
        })
    }
}

pub(super) fn shape(
    version: MinecraftVersion,
    name: &str,
    pose: EntityPose,
    baby: f32,
    scale: f32,
) -> (f32, f32) {
    if pose == EntityPose::Sleeping || (name == "minecraft:player" && pose == EntityPose::Dying) {
        return (0.2, 0.2); // Both original LivingEntity implementations use fixed sleeping dimensions.
    }
    if name == "minecraft:player" {
        let (width, height) = match pose {
            EntityPose::Sleeping | EntityPose::Dying => (0.2, 0.2),
            EntityPose::FallFlying | EntityPose::Swimming | EntityPose::SpinAttack => (0.6, 0.6),
            EntityPose::Crouching => (0.6, 1.5),
            _ => (0.6, 1.8), // Original POSES.getOrDefault for known native enum values.
        };
        return (width * scale, height * scale);
    }
    let (width, height) =
        super::state::dimensions(version, name).expect("qualified type dimensions");
    // Chicken's modern BABY_DIMENSIONS scales its registered dimensions by
    // 0.5f and changes eye height only; legacy uses the inherited age scale.
    // Both therefore use this same width/height arithmetic in the private lane.
    // Preserve the two original float multiplications; combining the factors
    // changes the last bit for non-power-of-two scales.
    (
        (width as f32 * baby) * scale,
        (height as f32 * baby) * scale,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{
        SessionStamp,
        entity::{NativeSpawn, NativeSpawnMotion, SpawnLedger},
    };
    fn session(version: MinecraftVersion) -> SessionStamp {
        SessionStamp {
            version,
            connection_id: 1,
            world_generation: 0,
        }
    }
    fn spawn(ledger: &mut SpawnLedger, version: MinecraftVersion, name: &str, sequence: u64) {
        let id = crate::client::registry::Registry::for_version(version)
            .builtin_id("minecraft:entity_type", name)
            .unwrap()
            .value();
        ledger
            .insert(
                version,
                NativeSpawn {
                    id: 7,
                    uuid: None,
                    type_id: Some(id),
                    dedicated_type_name: None,
                    position: [0.5, 65., 2.5],
                    living: Some(true),
                },
                sequence,
                16,
            )
            .unwrap();
        ledger.initialize_motion(
            7,
            NativeSpawnMotion {
                position: Some([0.5, 65., 2.5]),
                ..Default::default()
            },
            sequence,
        );
    }
    fn capture(ledger: &SpawnLedger, version: MinecraftVersion) -> EntityObservation {
        ledger
            .capture_all(version, session(version), 99)
            .entities
            .pop()
            .unwrap()
    }
    fn baby_index(version: MinecraftVersion, name: &str) -> u8 {
        version
            .table()
            .entity_data(name)
            .iter()
            .find(|r| {
                EntityDataField::Baby
                    .official_names()
                    .contains(&(r.owner, r.field))
            })
            .unwrap()
            .index
    }
    fn scale(ledger: &mut SpawnLedger, value: f64) {
        ledger.receive_scale(
            7,
            ReceivedAttribute {
                base: value,
                modifiers: vec![],
            },
            5,
        );
    }
    #[test]
    fn original_registered_world_dimensions_match_all_qualified_cases_exactly() {
        let oracle: serde_json::Value = serde_json::from_reader(flate2::read::GzDecoder::new(
            &include_bytes!("../../../data/client_api/entity_dimensions_oracle.json.gz")[..],
        ))
        .unwrap();
        let mut count = 0;
        for (version, key) in [
            (MinecraftVersion::Java1_16_1, "1.16.1"),
            (MinecraftVersion::Java1_21_11, "1.21.11"),
        ] {
            for row in oracle["versions"][key]["results"].as_array().unwrap() {
                assert_eq!(row["default_pose"], 0);
                let name = row["type"].as_str().unwrap();
                let mut ledger = SpawnLedger::default();
                spawn(&mut ledger, version, name, 1);
                ledger.receive_metadata(
                    7,
                    [(
                        6,
                        EntityDataValue::Pose(row["pose"].as_i64().unwrap() as i32),
                    )],
                    true,
                    3,
                );
                if name != "minecraft:player" {
                    ledger.receive_metadata(
                        7,
                        [(
                            baby_index(version, name),
                            EntityDataValue::Bool(row["baby"].as_bool().unwrap()),
                        )],
                        true,
                        4,
                    );
                }
                if version == MinecraftVersion::Java1_21_11 {
                    scale(&mut ledger, row["requested_scale"].as_f64().unwrap());
                }
                let entity = capture(&ledger, version);
                let model = entity.derived_geometry().unwrap();
                assert_eq!(
                    u64::from(model.width.to_bits()),
                    row["width_bits"].as_u64().unwrap(),
                    "{row}"
                );
                assert_eq!(
                    u64::from(model.height.to_bits()),
                    row["height_bits"].as_u64().unwrap(),
                    "{row}"
                );
                let bounds = [
                    model.bounds.min_x,
                    model.bounds.min_y,
                    model.bounds.min_z,
                    model.bounds.max_x,
                    model.bounds.max_y,
                    model.bounds.max_z,
                ];
                let original: Vec<_> = row["bounds_bits"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
                    .collect();
                assert_eq!(
                    bounds.map(f64::to_bits).as_slice(),
                    original.as_slice(),
                    "{row}"
                );
                assert_eq!(
                    u64::from(model.evidence.scale.value.to_bits()),
                    row["effective_scale_bits"].as_u64().unwrap()
                );
                assert_eq!(model.evidence.receive_sequence, 99);
                assert_eq!(
                    model.evidence.pose.source,
                    EntityDataSource::Received { sequence: 3 }
                );
                assert_eq!(
                    model.evidence.position_source,
                    ValueSource::Received { sequence: 1 }
                );
                assert_eq!(
                    entity.metadata.len(),
                    if name == "minecraft:player" { 1 } else { 2 }
                );
                count += 1;
            }
        }
        assert_eq!(count, 931);
    }
    #[test]
    fn absence_partial_packets_wrong_serializers_and_unknown_values_fence_geometry() {
        let v = MinecraftVersion::Java1_21_11;
        let mut l = SpawnLedger::default();
        spawn(&mut l, v, "minecraft:player", 1);
        assert_eq!(
            capture(&l, v).derived_geometry().unwrap_err(),
            EntityGeometryUnavailable::UnknownScale
        );
        scale(&mut l, 1.);
        let default = capture(&l, v).derived_geometry().unwrap();
        assert_eq!(default.evidence.pose.source, EntityDataSource::TypeDefault);
        assert!(capture(&l, v).pose().is_none());
        l.receive_metadata(7, [], false, 6);
        assert_eq!(
            capture(&l, v).derived_geometry().unwrap_err(),
            EntityGeometryUnavailable::UnknownPose
        );
        l.receive_metadata(7, [(6, EntityDataValue::Pose(5))], true, 7);
        let partial = capture(&l, v).derived_geometry().unwrap();
        assert!(!partial.evidence.metadata_complete);
        assert_eq!(partial.height, 1.5);
        for bad in [
            EntityDataValue::Int(5),
            EntityDataValue::Pose(99),
            EntityDataValue::Pose(-1),
        ] {
            l.receive_metadata(7, [(6, bad)], true, 8);
            assert_eq!(
                capture(&l, v).derived_geometry().unwrap_err(),
                EntityGeometryUnavailable::UnknownPose
            );
        }
        spawn(&mut l, v, "minecraft:pig", 10);
        scale(&mut l, 1.);
        l.receive_metadata(7, [(6, EntityDataValue::Pose(0))], false, 11);
        assert_eq!(
            capture(&l, v).derived_geometry().unwrap_err(),
            EntityGeometryUnavailable::UnknownBaby
        );
        let index = baby_index(v, "minecraft:pig");
        l.receive_metadata(7, [(index, EntityDataValue::Int(1))], true, 12);
        assert_eq!(
            capture(&l, v).derived_geometry().unwrap_err(),
            EntityGeometryUnavailable::UnknownBaby
        );
        l.receive_metadata(7, [(index, EntityDataValue::Bool(true))], true, 13);
        assert_eq!(capture(&l, v).derived_geometry().unwrap().width, 0.45);
    }
    #[test]
    fn lifetime_position_and_invalid_scale_cannot_reuse_geometry_inputs() {
        let v = MinecraftVersion::Java1_21_11;
        let mut l = SpawnLedger::default();
        spawn(&mut l, v, "minecraft:zombie", 1);
        scale(&mut l, 2.);
        let old = capture(&l, v).derived_geometry().unwrap();
        spawn(&mut l, v, "minecraft:player", 10);
        assert!(capture(&l, v).scale_attribute.is_none());
        assert_ne!(capture(&l, v).motion.entity.id, old.entity);
        assert!(l.capture_one(session(v), old.entity, 99).is_err());
        scale(&mut l, 1.);
        let mut e = capture(&l, v);
        e.motion.position = None;
        assert_eq!(
            e.derived_geometry().unwrap_err(),
            EntityGeometryUnavailable::MissingPosition
        );
        e = capture(&l, v);
        e.motion.position.as_mut().unwrap().value.position[0] = f64::NAN;
        assert_eq!(
            e.derived_geometry().unwrap_err(),
            EntityGeometryUnavailable::NonFiniteState
        );
        for bad in [f64::NAN, f64::INFINITY] {
            scale(&mut l, bad);
            assert_eq!(
                capture(&l, v).derived_geometry().unwrap_err(),
                EntityGeometryUnavailable::UnknownScale
            );
        }
        l.remove(7);
        assert!(l.capture_all(v, session(v), 99).entities.is_empty());
        // Late attributes for a retired lifetime cannot seed the replacement.
        scale(&mut l, 16.);
        spawn(&mut l, v, "minecraft:player", 20);
        assert!(capture(&l, v).scale_attribute.is_none());
        l.clear();
        assert!(l.capture_all(v, session(v), 99).entities.is_empty());
    }
    #[test]
    fn unsupported_modifier_budgets_and_nonreceipt_scale_stay_unknown() {
        let v = MinecraftVersion::Java1_21_11;
        let mut l = SpawnLedger::default();
        spawn(&mut l, v, "minecraft:player", 1);
        let modifiers = (0..17)
            .map(|n| crate::client::control::Modifier {
                id: format!("voxrig:scale_{n}"),
                amount: 0.01,
                operation: crate::client::control::ModifierOperation::MultiplyTotal,
            })
            .collect();
        l.receive_scale(
            7,
            ReceivedAttribute {
                base: 1.,
                modifiers,
            },
            3,
        );
        let mut e = capture(&l, v);
        assert_eq!(
            e.scale_attribute.as_ref().unwrap().value.modifiers.len(),
            17
        );
        assert_eq!(
            e.derived_geometry().unwrap_err(),
            EntityGeometryUnavailable::UnknownScale
        );
        e.scale_attribute.as_mut().unwrap().value.modifiers.clear();
        e.scale_attribute.as_mut().unwrap().source = ValueSource::Predicted;
        assert!(e.scale().is_none());
    }

    #[test]
    fn oversized_scale_receipts_invalidate_old_geometry_and_recovery_is_explicit() {
        let v = MinecraftVersion::Java1_21_11;
        let mut l = SpawnLedger::default();
        spawn(&mut l, v, "minecraft:player", 1);
        scale(&mut l, 1.);
        let previous = capture(&l, v).derived_geometry().unwrap();
        l.receive_scale(
            7,
            ReceivedAttribute {
                base: 2.,
                modifiers: vec![crate::client::control::Modifier {
                    id: format!("voxrig:{}", "x".repeat(4096)),
                    amount: 0.,
                    operation: crate::client::control::ModifierOperation::Addition,
                }],
            },
            8,
        );
        let rejected = capture(&l, v);
        assert!(rejected.scale_attribute.is_none());
        assert_eq!(rejected.scale_attribute_unavailable_sequence, Some(8));
        assert_eq!(
            rejected.derived_geometry().unwrap_err(),
            EntityGeometryUnavailable::UnknownScale
        );
        assert_eq!(previous.evidence.scale.value, 1.);
        scale(&mut l, 2.);
        let recovered = capture(&l, v);
        assert_eq!(recovered.scale_attribute_unavailable_sequence, None);
        assert_eq!(
            recovered.derived_geometry().unwrap().evidence.scale.value,
            2.
        );
        spawn(&mut l, v, "minecraft:player", 20);
        assert!(capture(&l, v).scale_attribute.is_none());
        assert!(
            capture(&l, v)
                .scale_attribute_unavailable_sequence
                .is_none()
        );
    }

    #[test]
    fn unsupported_types_never_get_a_default_dynamic_model() {
        for v in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let mut l = SpawnLedger::default();
            spawn(&mut l, v, "minecraft:slime", 1);
            assert_eq!(
                capture(&l, v).derived_geometry().unwrap_err(),
                EntityGeometryUnavailable::UnsupportedType
            );
        }
    }
}
