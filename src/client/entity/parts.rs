//! Derived multipart geometry, separate from received entity state.
use super::{EntityId, EntityObservation, SpawnLedger};
use crate::client::{Aabb, SessionStamp, ValueSource};
use crate::{MinecraftVersion, Result};

/// Supported native dragon parts. Other parts are not reconstructed yet.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum EntityPartKind {
    /// Dragon head, approximated by a rest pose only in sitting phases.
    DragonHead,
    /// First native wing, without an inferred left/right label.
    DragonWing1,
    /// Second native wing.
    DragonWing2,
}

/// A part of one original received parent lifetime, never an independent spawn.
/// Saved diagnostics cannot construct live targets.
/// ```compile_fail
/// let target: voxrig::client::EntityPartId = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct EntityPartId {
    parent: EntityId,
    kind: EntityPartKind,
    native_id: i32,
}
impl EntityPartId {
    /// Original received parent, including connection/world/spawn lifetime.
    pub fn parent(self) -> EntityId {
        self.parent
    }
    /// Native part kind.
    pub fn kind(self) -> EntityPartKind {
        self.kind
    }
    /// Native packet ID for diagnostics; integers cannot construct targets.
    pub fn native_id(self) -> i32 {
        self.native_id
    }
    pub(crate) fn new(parent: EntityId, kind: EntityPartKind, native_id: i32) -> Self {
        Self {
            parent,
            kind,
            native_id,
        }
    }
}

/// Why the supported part model cannot provide geometry at this capture.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum EntityPartUnavailable {
    /// No model for this exact Minecraft version.
    UnsupportedVersion,
    /// Parent is not a recognized multipart entity.
    NotMultipart,
    /// Required phase metadata is absent or has an unexpected serializer.
    UnknownPhase,
    /// Native phase is outside the implemented model.
    UnsupportedPhase(i32),
    /// Dragon death phase or received nonpositive health.
    DeadParent,
    /// No received position target is available.
    MissingPosition,
    /// No received body rotation is available.
    MissingRotation,
    /// Position, rotation, health or resulting bounds contain a non-finite value.
    NonFiniteState,
    /// Flying head needs native latency history and is not reconstructed.
    FlyingHead,
    /// Native part ID does not fit its signed packet field.
    IdOverflow,
}

/// Receipt sources of the parent fields used by a derived shape.
/// These are parent receipts, never a part spawn or server-current pose proof.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EntityPartEvidence {
    /// Coherent capture boundary of the parent.
    pub receive_sequence: u64,
    /// Parent position source.
    pub position_source: ValueSource,
    /// Parent rotation source.
    pub rotation_source: ValueSource,
    /// Parent phase source.
    pub phase_source: ValueSource,
    /// Parent health source if health was received. Missing does not prove life.
    pub health_source: Option<ValueSource>,
}

/// Geometry assumptions, separate from the parent receipt sources.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum EntityPartModel {
    /// Sitting head rest pose: flat latency history and zero turn adjustment.
    /// Native history and turn adjustment are not supplied by these receipts;
    /// even a sitting phase does not prove that the native head has this box.
    DragonSittingRestPose,
    /// Wing offsets at received body yaw, without native float/LUT rounding
    /// or render interpolation. This is a sampled model, not current collision.
    DragonWingPose,
}

/// Explicitly derived geometry or an unavailable part. Never a received entity.
#[derive(Clone, Debug, serde::Serialize)]
pub enum EntityPartState {
    /// Version-specific model evaluated from the parent's received fields.
    /// Does not certify current position, collision, reach, visibility or damage.
    Derived {
        /// Lifetime-scoped target, revalidated before dispatch.
        target: EntityPartId,
        /// Model bounds in blocks; consumer margins and tactics are separate.
        bounds: Aabb,
        /// Explicit geometry assumptions; receipt provenance is separate.
        model: EntityPartModel,
        /// Original parent field sources.
        evidence: EntityPartEvidence,
    },
    /// The model cannot provide this part at this capture.
    Unavailable(EntityPartUnavailable),
}

/// One supported part entry of a multipart parent.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EntityPartObservation {
    /// Native part kind.
    pub kind: EntityPartKind,
    /// Derived geometry or its explicit absence reason.
    pub state: EntityPartState,
}

impl EntityObservation {
    /// Derive supported multipart shapes from this captured parent.
    /// Received fields stay unchanged. Java 1.16.1 exposes sitting head and
    /// wing entries; the sitting head uses an explicit rest-pose approximation.
    /// Flying head and other versions are explicitly unsupported.
    /// Saved captures are not live targets: dispatch rechecks the current ledger.
    pub fn derived_parts(
        &self,
    ) -> std::result::Result<Vec<EntityPartObservation>, EntityPartUnavailable> {
        match self.motion.entity.id.session().version {
            MinecraftVersion::Java1_16_1 => {
                crate::versions::java_1_16_1::client::derive_entity_parts(self)
            }
            _ => Err(EntityPartUnavailable::UnsupportedVersion),
        }
    }
}

impl SpawnLedger {
    /// Revalidate parent lifetime and the current part model before native I/O.
    pub(crate) fn validate_part(
        &self,
        session: SessionStamp,
        target: EntityPartId,
        sequence: u64,
    ) -> Result<i32> {
        self.validate(session, target.parent)?;
        let parent = self.capture_one(session, target.parent, sequence)?;
        let parts = parent.derived_parts().map_err(|reason| {
            super::super::inventory::unavailable(format!("part model unavailable: {reason:?}"))
        })?;
        if parts.into_iter().any(|part| matches!(part.state, EntityPartState::Derived { target: current, .. } if current == target)) {
            Ok(target.native_id)
        } else {
            Err(super::super::inventory::unavailable("part retired or its geometry is unavailable"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::entity::{EntityDataValue, NativeSpawn, NativeSpawnMotion};

    fn session() -> SessionStamp {
        SessionStamp {
            version: MinecraftVersion::Java1_16_1,
            connection_id: 7,
            world_generation: 2,
        }
    }
    fn spawn(ledger: &mut SpawnLedger, id: i32, sequence: u64) -> EntityId {
        ledger
            .insert(
                session().version,
                NativeSpawn {
                    id,
                    uuid: Some([7; 16]),
                    type_id: None,
                    dedicated_type_name: Some("ender_dragon"),
                    position: [1.0, 65.0, 2.0],
                    living: Some(true),
                },
                sequence,
                16,
            )
            .unwrap();
        ledger.initialize_motion(
            id,
            NativeSpawnMotion {
                position: Some([1.0, 65.0, 2.0]),
                rotation: Some([90.0, 0.0]),
                ..Default::default()
            },
            sequence,
        );
        ledger.receive_metadata(id, [(15, EntityDataValue::Int(6))], true, sequence + 1);
        ledger.identity(session(), id).unwrap()
    }
    fn fixture() -> (SpawnLedger, EntityId, EntityObservation) {
        let mut ledger = SpawnLedger::default();
        let id = spawn(&mut ledger, 100, 10);
        let observation = ledger.capture_one(session(), id, 12).unwrap();
        (ledger, id, observation)
    }
    fn target(part: &EntityPartObservation) -> EntityPartId {
        match part.state {
            EntityPartState::Derived { target, .. } => target,
            _ => panic!("expected derived part"),
        }
    }
    fn unavailable(parent: &EntityObservation, reason: EntityPartUnavailable) {
        assert_eq!(parent.derived_parts().unwrap_err(), reason);
    }

    #[test]
    fn derived_parts_match_consumer_fixture_without_inventing_receipts() {
        // Golemkit #103 e57f821f: parent (100, 1,65,2), yaw 90, phase 6.
        let (ledger, id, parent) = fixture();
        let received = serde_json::to_value(&parent).unwrap();
        let parts = parent.derived_parts().unwrap();
        let expected = [
            (
                EntityPartKind::DragonHead,
                101,
                [7.0, 64.0, 1.5, 8.0, 65.0, 2.5],
            ),
            (
                EntityPartKind::DragonWing1,
                107,
                [-1.0, 67.0, 4.5, 3.0, 69.0, 8.5],
            ),
            (
                EntityPartKind::DragonWing2,
                108,
                [-1.0, 67.0, -4.5, 3.0, 69.0, -0.5],
            ),
        ];
        for (part, (kind, native_id, expected)) in parts.iter().zip(expected) {
            assert_eq!(part.kind, kind);
            let EntityPartState::Derived {
                target,
                bounds,
                model,
                evidence,
            } = &part.state
            else {
                panic!("missing shape")
            };
            assert_eq!(
                (target.parent(), target.native_id(), target.kind()),
                (id, native_id, kind)
            );
            let actual = [
                bounds.min_x,
                bounds.min_y,
                bounds.min_z,
                bounds.max_x,
                bounds.max_y,
                bounds.max_z,
            ];
            assert!(
                actual
                    .into_iter()
                    .zip(expected)
                    .all(|(a, b)| (a - b).abs() < 1e-12)
            );
            assert_eq!(
                *model,
                if kind == EntityPartKind::DragonHead {
                    EntityPartModel::DragonSittingRestPose
                } else {
                    EntityPartModel::DragonWingPose
                }
            );
            assert_eq!(evidence.receive_sequence, 12);
            assert_eq!(
                evidence.position_source,
                ValueSource::Received { sequence: 10 }
            );
            assert_eq!(
                evidence.rotation_source,
                ValueSource::Received { sequence: 10 }
            );
            assert_eq!(
                evidence.phase_source,
                ValueSource::Received { sequence: 11 }
            );
            assert_eq!(evidence.health_source, None);
        }
        assert_eq!(serde_json::to_value(&parent).unwrap(), received);
        assert_eq!(
            ledger
                .capture_all(session().version, session(), 12)
                .entities
                .len(),
            1
        );
    }

    #[test]
    fn phase_and_health_fail_closed_with_explicit_reasons() {
        let (_, _, mut parent) = fixture();
        for phase in 0..=8 {
            parent.metadata.get_mut(&15).unwrap().value = EntityDataValue::Int(phase);
            let parts = parent.derived_parts().unwrap();
            assert!(
                parts[1..]
                    .iter()
                    .all(|p| matches!(p.state, EntityPartState::Derived { .. }))
            );
            assert_eq!(
                matches!(parts[0].state, EntityPartState::Derived { .. }),
                (5..=7).contains(&phase)
            );
            if !(5..=7).contains(&phase) {
                assert!(matches!(
                    parts[0].state,
                    EntityPartState::Unavailable(EntityPartUnavailable::FlyingHead)
                ));
            }
        }
        parent.metadata.get_mut(&15).unwrap().value = EntityDataValue::Int(9);
        unavailable(&parent, EntityPartUnavailable::DeadParent);
        for phase in [-1, 10, i32::MAX] {
            parent.metadata.get_mut(&15).unwrap().value = EntityDataValue::Int(phase);
            unavailable(&parent, EntityPartUnavailable::UnsupportedPhase(phase));
        }
        parent.metadata.get_mut(&15).unwrap().value = EntityDataValue::Byte(6);
        unavailable(&parent, EntityPartUnavailable::UnknownPhase);
        parent.metadata.remove(&15);
        unavailable(&parent, EntityPartUnavailable::UnknownPhase);
        let (_, _, mut parent) = fixture();
        for health in [0.0, -1.0] {
            parent.health = Some(crate::client::received(health, 13));
            unavailable(&parent, EntityPartUnavailable::DeadParent);
        }
    }

    #[test]
    fn missing_or_non_finite_inputs_never_produce_targets() {
        let (_, _, parent) = fixture();
        let mut missing = parent.clone();
        missing.motion.position = None;
        unavailable(&missing, EntityPartUnavailable::MissingPosition);
        missing = parent.clone();
        missing.motion.rotation = None;
        unavailable(&missing, EntityPartUnavailable::MissingRotation);
        missing = parent.clone();
        missing.motion.entity.type_name = Some("minecraft:sheep".into());
        unavailable(&missing, EntityPartUnavailable::NotMultipart);
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut invalid = parent.clone();
            invalid.motion.position.as_mut().unwrap().value.position[0] = bad;
            unavailable(&invalid, EntityPartUnavailable::NonFiniteState);
        }
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            for axis in 0..2 {
                let mut invalid = parent.clone();
                invalid.motion.rotation.as_mut().unwrap().value[axis] = bad;
                unavailable(&invalid, EntityPartUnavailable::NonFiniteState);
            }
        }
        for bad in [f32::NAN, f32::INFINITY] {
            let mut invalid = parent.clone();
            invalid.health = Some(crate::client::received(bad, 13));
            unavailable(&invalid, EntityPartUnavailable::NonFiniteState);
        }
        let mut modern_session = session();
        modern_session.version = MinecraftVersion::Java1_21_11;
        let mut ledger = SpawnLedger::default();
        spawn(&mut ledger, 100, 10);
        let modern = ledger
            .capture_one(
                modern_session,
                ledger.identity(modern_session, 100).unwrap(),
                12,
            )
            .unwrap();
        unavailable(&modern, EntityPartUnavailable::UnsupportedVersion);
    }

    #[test]
    fn checked_native_ids_allow_valid_parts_without_wrapping_others() {
        let mut ledger = SpawnLedger::default();
        for id in [i32::MAX - 4, i32::MAX] {
            let parent_id = spawn(&mut ledger, id, 10);
            let parts = ledger
                .capture_one(session(), parent_id, 12)
                .unwrap()
                .derived_parts()
                .unwrap();
            assert_eq!(
                matches!(parts[0].state, EntityPartState::Derived { .. }),
                id < i32::MAX
            );
            assert!(parts[1..].iter().all(|p| matches!(
                p.state,
                EntityPartState::Unavailable(EntityPartUnavailable::IdOverflow)
            )));
            if id == i32::MAX {
                assert!(matches!(
                    parts[0].state,
                    EntityPartState::Unavailable(EntityPartUnavailable::IdOverflow)
                ));
            }
        }
    }

    #[test]
    fn dispatch_revalidates_phase_death_session_and_original_parent_lifetime() {
        let (mut ledger, _, parent) = fixture();
        let parts = parent.derived_parts().unwrap();
        let head = target(&parts[0]);
        let wing = target(&parts[1]);
        assert_eq!(ledger.validate_part(session(), head, 12).unwrap(), 101);
        ledger.receive_metadata(100, [(15, EntityDataValue::Int(3))], true, 13);
        assert!(ledger.validate_part(session(), head, 14).is_err());
        assert_eq!(ledger.validate_part(session(), wing, 14).unwrap(), 107);
        for phase in [9, 10] {
            ledger.receive_metadata(100, [(15, EntityDataValue::Int(phase))], true, 15);
            assert!(ledger.validate_part(session(), wing, 16).is_err());
        }
        ledger.receive_metadata(100, [(15, EntityDataValue::Int(6))], true, 17);
        ledger.receive_health(100, 0.0, 18);
        assert!(ledger.validate_part(session(), head, 19).is_err());
        ledger.receive_health(100, 200.0, 20);
        for foreign in [
            SessionStamp {
                connection_id: 8,
                ..session()
            },
            SessionStamp {
                world_generation: 3,
                ..session()
            },
        ] {
            assert!(ledger.validate_part(foreign, head, 21).is_err());
        }
        ledger.remove(100);
        assert!(ledger.validate_part(session(), head, 22).is_err());
        spawn(&mut ledger, 100, 23);
        assert!(ledger.validate_part(session(), head, 25).is_err());
        let fresh = ledger
            .capture_one(session(), ledger.identity(session(), 100).unwrap(), 25)
            .unwrap()
            .derived_parts()
            .unwrap();
        assert_eq!(
            ledger
                .validate_part(session(), target(&fresh[0]), 25)
                .unwrap(),
            101
        );
    }
}
