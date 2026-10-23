//! Read-only rays through qualified boxes at received targets, never live hitboxes.
use super::{EntityGeometry, EntityId};
use crate::client::{
    BlockTargetHit, BlockTargetPose, BlockTargetPoseSource, ObservedValue, PlayerObservation,
};

/// Maximum retained bodies examined by a received-target ray. No truncation or eviction.
pub const MAX_ENTITY_TARGET_QUERY_BODIES: usize = 256;

/// Geometric candidate at a received packet target. Pickability, native spatial
/// visitation order, interpolation, attack eligibility and server success are not inferred.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EntityTargetCandidate {
    /// Connection/world/spawn lifetime, independent of the reusable integer ID.
    pub entity: EntityId,
    /// Qualified dimension algorithm, target box and all input provenance.
    pub geometry: EntityGeometry,
    /// Native pick radius for the qualified player/cow/pig/zombie types (zero).
    pub pick_radius: f32,
    /// Model entry point, or the eye when contained in the half-open box.
    pub point: [f64; 3],
    /// Euclidean distance from the captured eye.
    pub distance: f64,
    /// Native squared distance used for strict comparisons, before square root.
    pub distance_squared: f64,
    /// Native half-open contains test. The original picker treats this before
    /// the root-vehicle exclusion and allows distance zero.
    pub eye_inside: bool,
    /// Whether the candidate shares the own player's received root vehicle.
    pub same_vehicle: bool,
    /// True when the first static block outline is at or before this candidate.
    /// False only after a complete available block ray; never an attack receipt.
    pub block_occluded: bool,
}

/// Coherent own eye/range and bounded received-target geometric candidates.
/// This is a model of packet targets, not the current rendered/server hitboxes.
/// Unknown geometry rejects the whole query, including unsupported remote bodies.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EntityTargetObservation {
    /// Own player/inventory, received attributes and position/rotation provenance.
    pub initial: PlayerObservation,
    /// World cache revision shared by support and block traversal.
    pub world_revision: u64,
    /// Capture ordinal shared by every entity geometry.
    pub receive_sequence: u64,
    /// Qualified stationary own pose.
    pub pose: BlockTargetPose,
    /// Actual own pose receipt or native world-entry initialization.
    pub pose_source: BlockTargetPoseSource,
    /// Received effective own scale, clamped before native float conversion.
    pub scale: ObservedValue<f64>,
    /// Derived eye with modeled provenance.
    pub eye: ObservedValue<[f64; 3]>,
    /// Received entity_interaction_range, natively clamped to 0..64.
    pub reach: ObservedValue<f64>,
    /// Own scaled body bounds, min XYZ then max XYZ.
    pub bounds: [f64; 6],
    /// Unique known dry supporting cells.
    pub support: Vec<[i32; 3]>,
    /// First available static OUTLINE/NONE block hit across the entire range.
    pub block_hit: Option<BlockTargetHit>,
    /// All in-range qualified intersections, including those behind the block hit.
    /// Sorted by squared distance, then native ID for diagnostics. Equal-distance
    /// overlaps are retained: this ordering does not claim native pick priority.
    pub candidates: Vec<EntityTargetCandidate>,
}

/// Native AABB.clip: entry planes in X/Y/Z order, strict segment endpoints,
/// 1e-7 parallel/face tolerance. Unlike VoxelShape.clip there is no inside nudge.
fn intersection(bounds: [f64; 6], start: [f64; 3], end: [f64; 3]) -> Option<([f64; 3], bool)> {
    if (0..3).all(|i| start[i] >= bounds[i] && start[i] < bounds[i + 3]) {
        return Some((start, true));
    }
    let delta: [f64; 3] = std::array::from_fn(|i| end[i] - start[i]);
    let mut best = 1.;
    let mut found = false;
    for axis in 0..3 {
        if delta[axis].abs() <= 1e-7 {
            continue;
        }
        let plane = if delta[axis] > 0. {
            bounds[axis]
        } else {
            bounds[axis + 3]
        };
        let t = (plane - start[axis]) / delta[axis];
        if t > 0.
            && t < best
            && (0..3).filter(|&i| i != axis).all(|i| {
                let v = start[i] + t * delta[i];
                bounds[i] - 1e-7 < v && v < bounds[i + 3] + 1e-7
            })
        {
            best = t;
            found = true;
        }
    }
    found.then(|| (std::array::from_fn(|i| start[i] + best * delta[i]), false))
}

pub(crate) fn ray_candidate(
    geometry: EntityGeometry,
    eye: [f64; 3],
    end: [f64; 3],
    reach: f64,
    same_vehicle: bool,
    block: Option<&BlockTargetHit>,
) -> Option<EntityTargetCandidate> {
    let b = geometry.bounds;
    // Entity.getPickRadius is inherited unchanged by all four qualified types.
    const PICK_RADIUS: f32 = 0.;
    let (point, eye_inside) = intersection(
        [b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z],
        eye,
        end,
    )?;
    let distance_squared = squared(eye, point);
    if !eye_inside && (same_vehicle || distance_squared >= reach * reach) {
        return None;
    }
    Some(EntityTargetCandidate {
        entity: geometry.entity,
        geometry,
        pick_radius: PICK_RADIUS,
        point,
        distance: distance_squared.sqrt(),
        distance_squared,
        eye_inside,
        same_vehicle,
        block_occluded: block.is_some_and(|hit| distance_squared >= squared(eye, hit.point)),
    })
}

fn squared(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d: [f64; 3] = std::array::from_fn(|i| a[i] - b[i]);
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn entity_target_rays_match_original_aabb_and_pick_radius() {
        let original: serde_json::Value = serde_json::from_reader(flate2::read::GzDecoder::new(
            &include_bytes!("../../../data/client_api/entity_target_rays.json.gz")[..],
        ))
        .unwrap();
        assert_eq!(
            original["entity_range_limits"],
            serde_json::json!([0., 64.])
        );
        assert_eq!(original["pick_radii"].as_array().unwrap().len(), 4);
        for radius in original["pick_radii"].as_array().unwrap() {
            assert_eq!(radius["radius"].as_f64().unwrap(), 0.);
        }
        let cases = original["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 1521);
        for row in cases {
            fn bits<const N: usize>(row: &serde_json::Value) -> [f64; N] {
                std::array::from_fn(|i| {
                    f64::from_bits(u64::from_str_radix(row[i].as_str().unwrap(), 16).unwrap())
                })
            }
            let bounds = bits(&row["bounds_bits"]);
            let start = bits(&row["start_bits"]);
            let end = bits(&row["end_bits"]);
            let result = intersection(bounds, start, end);
            if row["point"].is_null() {
                assert!(result.is_none(), "{row}");
            } else {
                let (point, inside) = result.unwrap_or_else(|| panic!("{row}"));
                assert_eq!(inside, row["inside"].as_bool().unwrap(), "{row}");
                let want = bits::<3>(&row["point_bits"]);
                assert_eq!(point, want, "{row}");
            }
        }
    }
    #[test]
    fn native_segment_boundaries_inside_and_parallel_tolerance() {
        let b = [0., 0., 0., 1., 1., 1.];
        assert_eq!(intersection(b, [-1., 0.5, 0.5], [0., 0.5, 0.5]), None);
        assert_eq!(
            intersection(b, [-1., 0.5, 0.5], [1., 0.5, 0.5]),
            Some(([0., 0.5, 0.5], false))
        );
        assert_eq!(
            intersection(b, [0., 0.5, 0.5], [2., 0.5, 0.5]),
            Some(([0., 0.5, 0.5], true))
        );
        assert_eq!(intersection(b, [1., 0.5, 0.5], [-1., 0.5, 0.5]), None);
        assert_eq!(
            intersection(b, [-1., 1. + 2e-7, 0.5], [2., 1. + 2e-7, 0.5]),
            None
        );
        assert!(intersection(b, [-1., 1. + 0.5e-7, 0.5], [2., 1. + 0.5e-7, 0.5]).is_some());
    }
}
