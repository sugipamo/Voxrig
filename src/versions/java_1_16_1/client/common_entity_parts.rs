//! Java 1.16.1 dragon part model. No tactical decisions or received part spawns.
use crate::client::{
    Aabb, EntityDataValue, EntityObservation, EntityPartEvidence, EntityPartId, EntityPartKind,
    EntityPartModel, EntityPartObservation, EntityPartState, EntityPartUnavailable as U,
};

pub(crate) fn derive_entity_parts(
    parent: &EntityObservation,
) -> std::result::Result<Vec<EntityPartObservation>, U> {
    if parent.motion.entity.type_name.as_deref() != Some("minecraft:ender_dragon") {
        return Err(U::NotMultipart);
    }
    let phase = parent.metadata.get(&15).ok_or(U::UnknownPhase)?;
    let EntityDataValue::Int(phase_id) = phase.value else {
        return Err(U::UnknownPhase);
    };
    if phase_id == 9 || parent.health.as_ref().is_some_and(|h| h.value <= 0.0) {
        return Err(U::DeadParent);
    }
    if !(0..=8).contains(&phase_id) {
        return Err(U::UnsupportedPhase(phase_id));
    }
    let position = parent.motion.position.as_ref().ok_or(U::MissingPosition)?;
    let rotation = parent.motion.rotation.as_ref().ok_or(U::MissingRotation)?;
    let p = position.value.position;
    if !p.into_iter().all(f64::is_finite)
        || !rotation.value.into_iter().all(f32::is_finite)
        || parent.health.as_ref().is_some_and(|h| !h.value.is_finite())
    {
        return Err(U::NonFiniteState);
    }
    let yaw = f64::from(rotation.value[0]).to_radians();
    let evidence = EntityPartEvidence {
        receive_sequence: parent.motion.receive_sequence,
        position_source: position.source,
        rotation_source: rotation.source,
        phase_source: phase.source,
        health_source: parent.health.as_ref().map(|h| h.source),
    };
    let mut parts = Vec::with_capacity(3);
    for (kind, offset, side) in [
        (EntityPartKind::DragonHead, 1, 0.0),
        (EntityPartKind::DragonWing1, 7, 1.0),
        (EntityPartKind::DragonWing2, 8, -1.0),
    ] {
        let state = if kind == EntityPartKind::DragonHead && !(5..=7).contains(&phase_id) {
            EntityPartState::Unavailable(U::FlyingHead)
        } else if let Some(native_id) = parent.motion.entity.id.native_id().checked_add(offset) {
            let (center, width, height) = if kind == EntityPartKind::DragonHead {
                (
                    [p[0] + yaw.sin() * 6.5, p[1] - 1.0, p[2] - yaw.cos() * 6.5],
                    1.0,
                    1.0,
                )
            } else {
                (
                    [
                        p[0] + side * yaw.cos() * 4.5,
                        p[1] + 2.0,
                        p[2] + side * yaw.sin() * 4.5,
                    ],
                    4.0,
                    2.0,
                )
            };
            let bounds = Aabb {
                min_x: center[0] - width / 2.0,
                min_y: center[1],
                min_z: center[2] - width / 2.0,
                max_x: center[0] + width / 2.0,
                max_y: center[1] + height,
                max_z: center[2] + width / 2.0,
            };
            if [
                bounds.min_x,
                bounds.min_y,
                bounds.min_z,
                bounds.max_x,
                bounds.max_y,
                bounds.max_z,
            ]
            .into_iter()
            .all(f64::is_finite)
            {
                EntityPartState::Derived {
                    target: EntityPartId::new(parent.motion.entity.id, kind, native_id),
                    bounds,
                    model: if kind == EntityPartKind::DragonHead {
                        EntityPartModel::DragonSittingRestPose
                    } else {
                        EntityPartModel::DragonWingPose
                    },
                    evidence: evidence.clone(),
                }
            } else {
                EntityPartState::Unavailable(U::NonFiniteState)
            }
        } else {
            EntityPartState::Unavailable(U::IdOverflow)
        };
        parts.push(EntityPartObservation { kind, state });
    }
    Ok(parts)
}
