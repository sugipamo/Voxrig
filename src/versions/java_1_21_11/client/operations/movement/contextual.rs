//! Separate read-only admission: no relaxation of standing action contracts.
use super::super::super::players::PlayerPose;
use super::super::survival::ValueBasis;
use super::*;
use crate::client::{
    BlockTargetPose, BlockTargetPoseSource, ContextualBlockTargetObservation, ObservedValue,
    ValueSource,
};

// Native Player POSES dimensions, converted/multiplied as floats before widening.
const DIMENSIONS: &[(BlockTargetPose, f32, f32)] = &[
    (BlockTargetPose::Standing, 1.8, 1.62),
    (BlockTargetPose::Crouching, 1.5, 1.27),
];

// Original RangedAttribute limits, independently exported with the clip fixture.
const ATTRIBUTE_LIMITS: &[(&str, f64, f64)] = &[
    ("minecraft:scale", 0.0625, 16.0),
    ("minecraft:block_interaction_range", 0.0, 64.0),
];

pub(super) fn query(
    ops: &Operations,
    state: &mut State,
    mode: GameMode,
) -> Result<ContextualBlockTargetObservation> {
    let initial = ops.common_player_unlocked(state)?;
    if initial.pending_dispatch || initial.game_mode != Some(mode) {
        return Err(invalid(
            "contextual targeting requires matching mode and no unresolved dispatch",
        ));
    }
    // Limit this first contract to an actual received stationary origin. Settled
    // predicted paths require a separate treatment of their positional reserve.
    if !matches!(
        standing_basis(state)?,
        StandingPositionBasis::Received { .. }
    ) {
        return Err(invalid(
            "contextual targeting requires a received stationary origin",
        ));
    }
    let local = &state.operations.local_player;
    if local.motion_interruption.is_some()
        || state.operations.requested_flying
        || state
            .operations
            .abilities
            .is_some_and(|flags| flags & 2 != 0)
        || !initial
            .health
            .as_ref()
            .is_some_and(|h| h.value.health > 0.0)
    {
        return Err(invalid(
            "contextual targeting requires a healthy unmounted ground player without flight or motion interruption",
        ));
    }
    let pose = match local.pose {
        Some(PlayerPose::Standing) => BlockTargetPose::Standing,
        Some(PlayerPose::Crouching) => BlockTargetPose::Crouching,
        _ => {
            return Err(invalid(
                "contextual targeting requires known Standing/Crouching pose",
            ));
        }
    };
    let pose_source = match local.pose_basis {
        Some(ValueBasis::NativeReset) => BlockTargetPoseSource::NativeReset,
        Some(ValueBasis::Received { sequence }) => BlockTargetPoseSource::Received { sequence },
        None => return Err(invalid("contextual targeting pose source unavailable")),
    };
    let attribute = |name: &str| -> Result<ObservedValue<f64>> {
        let value = initial
            .attributes
            .get(name)
            .filter(|a| matches!(a.source, ValueSource::Received { .. }))
            .ok_or_else(|| invalid(&format!("contextual targeting requires received {name}")))?;
        if !value.value.value.is_finite() {
            return Err(invalid("contextual targeting attribute is non-finite"));
        }
        let (_, min, max) = ATTRIBUTE_LIMITS
            .iter()
            .find(|(key, _, _)| *key == name)
            .unwrap();
        Ok(ObservedValue {
            value: value.value.value.clamp(*min, *max),
            source: value.source,
        })
    };
    let scale = attribute("minecraft:scale")?;
    let reach = attribute("minecraft:block_interaction_range")?;
    if !crate::client::physics::supported_player_scale(
        crate::MinecraftVersion::Java1_21_11,
        Some(scale.value),
    ) || !reach.value.is_finite()
        || !(0.0..=64.0).contains(&reach.value)
    {
        return Err(invalid(
            "contextual targeting scale or reach outside qualified range",
        ));
    }
    let position = initial
        .position
        .as_ref()
        .ok_or_else(|| invalid("contextual targeting position unavailable"))?
        .value;
    if position
        .iter()
        .any(|v| !v.is_finite() || v.abs() > 30_000_000.0)
    {
        return Err(invalid(
            "contextual targeting position outside finite world bounds",
        ));
    }
    crate::client::survival::target::validate_rotation(initial.rotation)?;
    if initial.rotation_source.is_none() {
        return Err(invalid("contextual targeting rotation source unavailable"));
    }
    let model_scale = scale.value as f32;
    let (bounds, eye) = dimensions(pose, model_scale, position);
    state.reconstruction.advance(
        &state.world,
        ops.bot.session.started.elapsed().as_millis() as u64 / 50,
    );
    if state.reconstruction.issue.is_some() || !state.reconstruction.recovery_chunks.is_empty() {
        return Err(invalid("client reconstruction incomplete"));
    }
    let support = grounded(state, bounds)?;
    let hit = super::super::super::raycast::contextual_outline_hit(state, eye, reach.value)?;
    Ok(ContextualBlockTargetObservation {
        initial,
        world_revision: state.world.revision,
        pose,
        pose_source,
        scale,
        model_scale,
        eye: ObservedValue {
            value: eye,
            source: ValueSource::Predicted,
        },
        reach,
        bounds,
        support,
        visibility: ObservedValue {
            value: hit,
            source: ValueSource::Predicted,
        },
        replaceable: None,
        operation_permitted: None,
    })
}

fn dimensions(pose: BlockTargetPose, model_scale: f32, position: [f64; 3]) -> ([f64; 6], [f64; 3]) {
    let (_, height, eye_height) = DIMENSIONS.iter().find(|(p, _, _)| *p == pose).unwrap();
    let half = f64::from((0.6f32 * model_scale) / 2.0);
    let bounds = [
        position[0] - half,
        position[1],
        position[2] - half,
        position[0] + half,
        position[1] + f64::from(height * model_scale),
        position[2] + half,
    ];
    let eye = [
        position[0],
        position[1] + f64::from(eye_height * model_scale),
        position[2],
    ];
    (bounds, eye)
}

fn grounded(state: &impl GeometryView, bounds: [f64; 6]) -> Result<Vec<[i32; 3]>> {
    let mut support = Vec::new();
    for x in (bounds[0] - 1e-7).floor() as i32..=(bounds[3] + 1e-7).floor() as i32 {
        for y in (bounds[1] - 1e-7).floor() as i32..=bounds[4].floor() as i32 {
            for z in (bounds[2] - 1e-7).floor() as i32..=(bounds[5] + 1e-7).floor() as i32 {
                let p = [x, y, z];
                // Existing qualified dry geometry rejects fluids and unknown or
                // context-dependent collision shapes around the player's body.
                let block = state.block(p)?;
                for shape in crate::client::survival::model::collision_shape(
                    crate::MinecraftVersion::Java1_21_11,
                    &block,
                )? {
                    let b: [f64; 6] = std::array::from_fn(|i| shape[i] + f64::from(p[i % 3]));
                    let horizontal = bounds[0] < b[3]
                        && bounds[3] > b[0]
                        && bounds[2] < b[5]
                        && bounds[5] > b[2];
                    if horizontal && bounds[1] < b[4] && bounds[4] > b[1] {
                        return Err(invalid(
                            "contextual targeting body intersects solid geometry",
                        ));
                    }
                    if bounds[0] + 1e-7 < b[3]
                        && bounds[3] - 1e-7 > b[0]
                        && bounds[2] + 1e-7 < b[5]
                        && bounds[5] - 1e-7 > b[2]
                        && (0.0..1e-7).contains(&(bounds[1] - b[4]))
                    {
                        support.push(p);
                    }
                }
            }
        }
    }
    if support.is_empty() {
        return Err(invalid(
            "contextual targeting requires known dry ground support",
        ));
    }
    Ok(support)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn contextual_dimensions_match_original_avatar_pose_dimensions() {
        let mut json = String::new();
        flate2::read::GzDecoder::new(
            &include_bytes!("../../../../../../data/client_api/contextual_targeting.json.gz")[..],
        )
        .read_to_string(&mut json)
        .unwrap();
        let cases: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert_eq!(cases.len(), 1344);
        for c in cases {
            assert_eq!(
                c["scale_limits"],
                serde_json::json!([ATTRIBUTE_LIMITS[0].1, ATTRIBUTE_LIMITS[0].2])
            );
            assert_eq!(
                c["reach_limits"],
                serde_json::json!([ATTRIBUTE_LIMITS[1].1, ATTRIBUTE_LIMITS[1].2])
            );
            let pose = if c["pose"] == 0 {
                BlockTargetPose::Standing
            } else {
                BlockTargetPose::Crouching
            };
            let (bounds, eye) = dimensions(
                pose,
                c["scale"].as_f64().unwrap() as f32,
                serde_json::from_value(c["position"].clone()).unwrap(),
            );
            for (values, field) in [
                (eye.as_slice(), "eye_bits"),
                (bounds.as_slice(), "bounds_bits"),
            ] {
                for (value, expected) in values.iter().zip(c[field].as_array().unwrap()) {
                    assert_eq!(
                        value.to_bits(),
                        u64::from_str_radix(expected.as_str().unwrap(), 16).unwrap(),
                        "{c}"
                    );
                }
            }
        }
    }
}
