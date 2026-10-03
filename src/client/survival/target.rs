//! Shared static outline traversal. Shape selection and version math remain explicit.
use crate::client::BlockFace;
use crate::{MinecraftVersion, NativeBlockState};

/// Model-selected block from received geometry; never server targeting authority.
#[derive(Clone, Debug, serde::Serialize)]
pub struct BlockTargetHit {
    /// Cell owning the outline, which may protrude outside that cell.
    pub position: [i32; 3],
    /// Complete version-native received state selected by the adapter.
    pub state: NativeBlockState,
    /// Entry face according to the selected version's outline traversal.
    pub face: BlockFace,
    /// Model hit position, separate from packet-received geometry.
    pub point: [f64; 3],
    /// Model distance from the captured player's native standing eye.
    pub distance: f64,
}
/// Own-player capture and a bounded dry-standing outline query at one boundary.
/// Not a reusable mining/placement plan or a server hit receipt.
#[derive(Clone, Debug, serde::Serialize)]
pub struct BlockTargetObservation {
    /// Player/inventory and received-vs-local pose provenance.
    pub initial: crate::client::PlayerObservation,
    /// Received world-cache revision in the same adapter boundary.
    pub world_revision: u64,
    /// Native default standing eye derived from the captured position.
    pub eye: [f64; 3],
    /// Caller-selected query reach, bounded by default survival range.
    pub maximum_distance: f64,
    /// None only after a complete available query found no static block outline.
    pub hit: Option<BlockTargetHit>,
}

pub(crate) fn validate_reach(distance: f64) -> crate::Result<()> {
    if !distance.is_finite() || distance <= 0.0 || distance > 4.5 {
        return Err(crate::Error::new(
            crate::ErrorKind::InvalidInput,
            anyhow::anyhow!("outline reach must be finite, positive and at most 4.5 blocks"),
        ));
    }
    Ok(())
}
pub(crate) fn validate_rotation([yaw, pitch]: [f32; 2]) -> crate::Result<()> {
    if !yaw.is_finite() || !pitch.is_finite() || !(-90.0..=90.0).contains(&pitch) {
        return Err(crate::Error::new(
            crate::ErrorKind::State,
            anyhow::anyhow!("outline requires an available finite native view rotation"),
        ));
    }
    Ok(())
}
/// Native Entity.calculateViewVector: float operations and version-selected sine table.
pub(crate) fn direction(version: MinecraftVersion, [yaw, pitch]: [f32; 2]) -> [f64; 3] {
    let pitch = pitch * (std::f32::consts::PI / 180.0);
    let yaw = -yaw * (std::f32::consts::PI / 180.0);
    let trig = |angle, cosine| super::model::trig(version, angle, cosine);
    [
        f64::from(trig(yaw, false) * trig(pitch, true)),
        f64::from(-trig(pitch, false)),
        f64::from(trig(yaw, true) * trig(pitch, true)),
    ]
}
/// Native DDA visits cells in order and returns the first cell's hit. Neighboring
/// protrusions are not added to this traversal (unlike the collision query).
pub(crate) fn cast<'a>(
    start: [f64; 3],
    end: [f64; 3],
    mut read: impl FnMut([i32; 3]) -> anyhow::Result<NativeBlockState>,
    mut shapes: impl FnMut(&NativeBlockState) -> anyhow::Result<(&'a [[f64; 6]], &'a [[f64; 6]])>,
) -> anyhow::Result<Option<BlockTargetHit>> {
    if start == end {
        return Ok(None);
    }
    let from: [f64; 3] = std::array::from_fn(|i| start[i] - 1e-7 * (end[i] - start[i]));
    let to: [f64; 3] = std::array::from_fn(|i| end[i] - 1e-7 * (start[i] - end[i]));
    let delta: [f64; 3] = std::array::from_fn(|i| to[i] - from[i]);
    let step = delta.map(|d| {
        if d > 0.0 {
            1
        } else if d < 0.0 {
            -1
        } else {
            0
        }
    });
    let mut cell = from.map(|v| v.floor() as i32);
    let interval: [f64; 3] = std::array::from_fn(|i| {
        if step[i] == 0 {
            f64::MAX
        } else {
            f64::from(step[i]) / delta[i]
        }
    });
    let mut boundary: [f64; 3] = std::array::from_fn(|i| {
        let fraction = from[i] - from[i].floor();
        interval[i]
            * if step[i] > 0 {
                1.0 - fraction
            } else {
                fraction
            }
    });
    for _ in 0..512 {
        let state = read(cell)?;
        let (main, auxiliary) = shapes(&state)?;
        if let Some((fraction, mut face)) = intersect(start, end, cell, main) {
            // The auxiliary shape may override the face, never the hit position.
            if let Some((nearer, side)) = intersect(start, end, cell, auxiliary) {
                if nearer < fraction {
                    face = side;
                }
            }
            let distance = start
                .into_iter()
                .zip(end)
                .map(|(a, b)| (b - a).powi(2))
                .sum::<f64>()
                .sqrt()
                * fraction;
            return Ok(Some(BlockTargetHit {
                position: cell,
                state,
                distance,
                face,
                point: std::array::from_fn(|i| start[i] + (end[i] - start[i]) * fraction),
            }));
        }
        if boundary.iter().all(|&t| t > 1.0) {
            return Ok(None);
        }
        // Equal crossing times choose Z, then Y, then X, as in BlockView.
        let axis = if boundary[0] < boundary[1] {
            if boundary[0] < boundary[2] { 0 } else { 2 }
        } else if boundary[1] < boundary[2] {
            1
        } else {
            2
        };
        cell[axis] += step[axis];
        boundary[axis] += interval[axis];
    }
    anyhow::bail!("outline ray traversal limit exceeded")
}

fn intersect(
    start: [f64; 3],
    end: [f64; 3],
    cell: [i32; 3],
    boxes: &[[f64; 6]],
) -> Option<(f64, BlockFace)> {
    use BlockFace::*;
    let delta: [f64; 3] = std::array::from_fn(|i| end[i] - start[i]);
    if delta.iter().map(|d| d * d).sum::<f64>() < 1e-7 || boxes.is_empty() {
        return None;
    }
    let inside: [f64; 3] =
        std::array::from_fn(|i| start[i] + delta[i] * 0.001 - f64::from(cell[i]));
    if boxes
        .iter()
        .any(|b| (0..3).all(|i| inside[i] >= b[i] && inside[i] < b[i + 3]))
    {
        // getFacing converts to float and uses enum order for equal dot products.
        let d = delta.map(|v| v as f32);
        let mut best = f32::from_bits(1);
        let mut face = South;
        for (dot, opposite) in [
            (-d[1], Up),
            (d[1], Down),
            (-d[2], South),
            (d[2], North),
            (-d[0], East),
            (d[0], West),
        ] {
            if dot > best {
                best = dot;
                face = opposite;
            }
        }
        return Some((0.001, face));
    }
    let mut best = 1.0;
    let mut face = None;
    for bounds in boxes {
        // Native Box.offset precedes ray intersection. Applying epsilon in local
        // coordinates changes edge inclusion after world-coordinate rounding.
        let bounds: [f64; 6] = std::array::from_fn(|i| bounds[i] + f64::from(cell[i % 3]));
        for axis in 0..3 {
            if delta[axis].abs() <= 1e-7 {
                continue;
            }
            let near = if delta[axis] > 0.0 {
                bounds[axis]
            } else {
                bounds[axis + 3]
            };
            let t = (near - start[axis]) / delta[axis];
            if t > 0.0
                && t < best
                && (0..3).filter(|&i| i != axis).all(|i| {
                    let v = start[i] + t * delta[i];
                    bounds[i] - 1e-7 < v && v < bounds[i + 3] + 1e-7
                })
            {
                best = t;
                face = Some(if delta[axis] > 0.0 {
                    [West, Down, North][axis]
                } else {
                    [East, Up, South][axis]
                });
            }
        }
    }
    face.map(|f| (best, f))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    #[derive(Deserialize)]
    struct Rotation {
        rotation: [f32; 2],
        direction: [f64; 3],
    }
    #[derive(Deserialize)]
    struct Outline {
        name: String,
        outline: Vec<[f64; 6]>,
        auxiliary: Vec<[f64; 6]>,
    }
    #[derive(Deserialize, Debug)]
    struct Hit {
        position: [i32; 3],
        face: String,
        point: [f64; 3],
    }
    #[derive(Deserialize, Debug)]
    struct Ray {
        cells: Vec<[i32; 3]>,
        start: [f64; 3],
        end: [f64; 3],
        hit: Option<Hit>,
    }
    #[derive(Deserialize)]
    struct Oracle {
        rotations: Vec<Rotation>,
        outlines: Vec<Outline>,
        rays: Vec<Ray>,
    }
    fn oracle() -> Oracle {
        serde_json::from_str(include_str!(
            "../../../data/client_api/legacy_targeting_oracle.json"
        ))
        .unwrap()
    }
    #[test]
    fn legacy_view_vector_matches_unmodified_native_entity_method() {
        let cases = oracle().rotations;
        assert_eq!(cases.len(), 72);
        for case in cases {
            for (got, want) in direction(MinecraftVersion::Java1_16_1, case.rotation)
                .into_iter()
                .zip(case.direction)
            {
                assert!(
                    (got - want).abs() < 1e-15,
                    "{:?}: {got} != {want}",
                    case.rotation
                );
            }
        }
    }
    #[test]
    fn legacy_audited_cube_outlines_include_every_native_property_variant() {
        let cases = oracle().outlines;
        assert_eq!(cases.len(), 13); // grass_block has two snowy variants.
        let names: std::collections::BTreeSet<_> = cases.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            super::super::model::DRY_CUBES.iter().copied().collect()
        );
        for case in cases {
            assert_eq!(
                case.outline,
                vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]],
                "{}",
                case.name
            );
            assert!(case.auxiliary.is_empty(), "{}", case.name);
        }
    }
    #[test]
    fn shared_traversal_matches_legacy_block_getter_clip_not_self_generated_expectations() {
        let cases = oracle().rays;
        assert_eq!(cases.len(), 585);
        for (index, case) in cases.iter().enumerate() {
            let got = cast(
                case.start,
                case.end,
                |p| {
                    crate::versions::java_1_16_1::native_state(if case.cells.contains(&p) {
                        1
                    } else {
                        0
                    })
                    .map_err(Into::into)
                },
                |state| {
                    if state.name == "minecraft:stone" {
                        Ok((&[[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]], &[]))
                    } else {
                        Ok((&[], &[]))
                    }
                },
            )
            .unwrap();
            assert_eq!(got.is_some(), case.hit.is_some(), "case {index}: {case:?}");
            if let (Some(got), Some(want)) = (got, &case.hit) {
                assert_eq!(got.position, want.position, "case {index}: {case:?}");
                assert_eq!(
                    format!("{:?}", got.face).to_lowercase(),
                    want.face,
                    "case {index}: {case:?}"
                );
                for (actual, expected) in got.point.into_iter().zip(want.point) {
                    assert!(
                        (actual - expected).abs() < 1e-12,
                        "case {index}: {got:?} vs {case:?}"
                    );
                }
                let distance = case
                    .start
                    .into_iter()
                    .zip(want.point)
                    .map(|(a, b)| (b - a).powi(2))
                    .sum::<f64>()
                    .sqrt();
                assert!(
                    (got.distance - distance).abs() < 1e-12,
                    "case {index}: {got:?} vs {case:?}"
                );
            }
        }
    }
}
