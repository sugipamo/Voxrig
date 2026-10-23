//! Dry oak stairs: native yaw/half and front/back corner rules, without air defaults.
use super::*;
use crate::client::survival::{StairNeighborUpdate, StairShapeInput};
use std::collections::BTreeMap;

const FAMILY: &str = "minecraft:oak_stairs";
// Native Direction.fromYRot order and double-precision quadrant rounding.
const DIRECTIONS: &[(&str, [i32; 3])] = &[
    ("south", [0, 0, 1]),
    ("west", [-1, 0, 0]),
    ("north", [0, 0, -1]),
    ("east", [1, 0, 0]),
];
const SHAPES: &[&str] = &[
    "straight",
    "inner_left",
    "inner_right",
    "outer_left",
    "outer_right",
];
fn kind(state: &crate::NativeBlockState) -> Option<(usize, bool)> {
    if state.name != FAMILY
        || state.properties.len() != 4
        || state.properties.get("waterlogged").map(String::as_str) != Some("false")
        || !SHAPES.contains(&state.properties.get("shape")?.as_str())
    {
        return None;
    }
    let facing = DIRECTIONS
        .iter()
        .position(|(name, _)| Some(*name) == state.properties.get("facing").map(String::as_str))?;
    let top = match state.properties.get("half")?.as_str() {
        "top" => true,
        "bottom" => false,
        _ => return None,
    };
    Some((facing, top))
}
fn step(p: [i32; 3], direction: usize) -> Result<[i32; 3]> {
    let mut out = p;
    for i in 0..3 {
        out[i] = p[i]
            .checked_add(DIRECTIONS[direction].1[i])
            .ok_or_else(|| unavailable("stair neighbor overflow"))?;
    }
    Ok(out)
}
fn admitted(state: &crate::NativeBlockState) -> bool {
    kind(state).is_some()
        || (state.properties.is_empty()
            && (air(state) || survival::DRY_CUBES.contains(&state.name.as_str())))
}
fn read(
    view: &impl GeometryView,
    inputs: &mut BTreeMap<[i32; 3], crate::NativeBlockState>,
    p: [i32; 3],
) -> Result<crate::NativeBlockState> {
    if let Some(value) = inputs.get(&p) {
        return Ok(value.clone());
    }
    let value = view.block(p)?;
    if !admitted(&value) {
        return Err(unavailable(
            "stair shape input requires complete air, passive cube or dry oak stairs",
        ));
    }
    inputs.insert(p, value.clone());
    Ok(value)
}
fn recalculate(
    state: &crate::NativeBlockState,
    p: [i32; 3],
    mut read: impl FnMut([i32; 3]) -> Result<crate::NativeBlockState>,
) -> Result<crate::NativeBlockState> {
    let (facing, top) =
        kind(state).ok_or_else(|| unavailable("complete dry oak stair state required"))?;
    let mut shape = "straight";
    // Vanilla checks outer before inner. The side blocker is relative to the
    // center cell, including for a recalculated neighbor (diagonal to target).
    for (direction, outer) in [(facing, true), ((facing + 2) % 4, false)] {
        if let Some((other, half)) = kind(&read(step(p, direction)?)?) {
            if half == top && other % 2 != facing % 2 {
                let side = if outer { (other + 2) % 4 } else { other };
                if kind(&read(step(p, side)?)?) != Some((facing, top)) {
                    let left = other == (facing + 3) % 4;
                    shape = match (outer, left) {
                        (true, true) => "outer_left",
                        (true, false) => "outer_right",
                        (false, true) => "inner_left",
                        (false, false) => "inner_right",
                    };
                    break;
                }
            }
        }
    }
    let mut result = state.clone();
    result.properties.insert("shape".into(), shape.into());
    Ok(result)
}
fn base(yaw: f32, face: u8, hit_y: f32) -> crate::NativeBlockState {
    let angle = f64::from(yaw) / 90.0 + 0.5;
    let integer = angle as i32;
    // Mth.floor uses a saturating Java cast followed by a wrapping decrement.
    let quadrant = if angle < f64::from(integer) {
        integer.wrapping_sub(1)
    } else {
        integer
    };
    let facing = (quadrant & 3) as usize;
    let top = face == 0 || (face != 1 && hit_y > 0.5);
    crate::NativeBlockState {
        name: FAMILY.into(),
        properties: [
            ("facing".into(), DIRECTIONS[facing].0.into()),
            ("half".into(), if top { "top" } else { "bottom" }.into()),
            ("shape".into(), "straight".into()),
            ("waterlogged".into(), "false".into()),
        ]
        .into(),
    }
}
pub(super) fn expected(
    view: &impl GeometryView,
    target: [i32; 3],
    yaw: f32,
    face: u8,
    hit_y: f32,
) -> Result<crate::NativeBlockState> {
    let mut inputs = BTreeMap::new();
    recalculate(&base(yaw, face, hit_y), target, |p| {
        read(view, &mut inputs, p)
    })
}
pub(super) fn geometry(
    view: &impl GeometryView,
    standing: &survival::StandingContext,
    clicked: [i32; 3],
    face: u8,
    requested: ([i32; 3], &crate::NativeBlockState),
    rotation: [f32; 2],
) -> Result<PlacementGeometry> {
    if standing.position_basis.geometry_reserve() != [0.; 3] {
        return Err(unavailable(
            "stair placement requires an exact received stationary position",
        ));
    }
    // Keep the existing passive-cube support and first outline restrictions.
    let geometry = placement_geometry_in(
        view,
        standing.position,
        standing.bounds,
        [0.; 3],
        rotation,
        clicked,
        face,
        None,
    )?;
    if geometry.target != requested.0
        || &geometry.before != requested.1
        || !geometry.before.properties.is_empty()
    {
        return Err(unavailable(
            "stair placement requires the exact adjacent air predecessor",
        ));
    }
    Ok(geometry)
}
pub(super) fn unobstructed(
    state: &State,
    connection_id: u64,
    own: [f64; 6],
    target: [i32; 3],
    expected: &crate::NativeBlockState,
) -> Result<()> {
    kind(expected).ok_or_else(|| unavailable("complete dry oak stair shape required"))?;
    slab::unobstructed_shapes(
        state,
        connection_id,
        own,
        target,
        super::super::super::raycast::collision_boxes(expected)?,
        "stair",
    )
}
#[derive(Clone)]
pub(super) struct Capture {
    pub(super) inputs: Vec<StairShapeInput>,
    updates: Vec<StairNeighborUpdate>,
}
pub(super) fn capture(view: &impl GeometryView, intent: &PlacementIntent) -> Result<Capture> {
    let mut inputs = BTreeMap::new();
    read(view, &mut inputs, intent.target)?;
    let expected = recalculate(&intent.expected, intent.target, |p| {
        read(view, &mut inputs, p)
    })?;
    if expected != intent.expected {
        return Err(unavailable("stair target shape changed during capture"));
    }
    let mut updates = Vec::new();
    for direction in 0..4 {
        let position = step(intent.target, direction)?;
        let before = read(view, &mut inputs, position)?;
        if kind(&before).is_none() {
            continue;
        }
        let expected = recalculate(&before, position, |p| {
            if p == intent.target {
                Ok(intent.expected.clone())
            } else {
                read(view, &mut inputs, p)
            }
        })?;
        if before != expected {
            updates.push(StairNeighborUpdate {
                position,
                before,
                expected,
                receipt: None,
            });
        }
    }
    Ok(Capture {
        inputs: inputs
            .into_iter()
            .map(|(position, before)| StairShapeInput { position, before })
            .collect(),
        updates,
    })
}
pub(super) fn confirmed(state: &State) -> bool {
    state
        .common_placement
        .as_ref()
        .and_then(|c| c.stairs.as_ref())
        .is_none_or(|c| c.updates.iter().all(|u| u.receipt.is_some()))
}
fn active(state: &State) -> bool {
    state
        .placement
        .as_ref()
        .is_some_and(|r| r.observation.is_none())
}
pub(super) fn received(state: &mut State, changes: &[([i32; 3], i32)]) -> anyhow::Result<()> {
    if !active(state) {
        return Ok(());
    }
    let Some(capture) = state
        .common_placement
        .as_mut()
        .and_then(|c| c.stairs.as_mut())
    else {
        return Ok(());
    };
    for (p, id) in changes {
        let Some(input) = capture.inputs.iter().find(|i| i.position == *p) else {
            continue;
        };
        if *p == state.placement.as_ref().expect("active").intent.target {
            continue;
        }
        let actual = super::super::super::super::native_state(*id)?;
        let update = capture.updates.iter_mut().find(|u| u.position == *p);
        let permitted = if let Some(update) = update {
            if actual == update.expected && state.world.block(*p) == Some(*id) {
                if update.receipt.is_none()
                    && state.sequence
                        > state
                            .placement
                            .as_ref()
                            .expect("active")
                            .intent
                            .after_sequence
                {
                    update.receipt = Some(crate::client::received(actual.clone(), state.sequence));
                }
                true
            } else {
                actual == update.before && update.receipt.is_none()
            }
        } else {
            actual == input.before
        };
        if !permitted {
            state
                .placement
                .as_mut()
                .expect("active")
                .requires_inspection
                .get_or_insert_with(|| "stair shape input or neighbor conflict".into());
        }
    }
    Ok(())
}
pub(super) fn context_received(state: &mut State) {
    if !active(state) {
        return;
    }
    let Some(capture) = state
        .common_placement
        .as_ref()
        .and_then(|c| c.stairs.as_ref())
    else {
        return;
    };
    let intent = &state.placement.as_ref().expect("active").intent;
    let conflict = capture
        .inputs
        .iter()
        .filter(|i| i.position != intent.target)
        .any(|i| {
            let Ok(actual) = state.block(i.position) else {
                return true;
            };
            match capture.updates.iter().find(|u| u.position == i.position) {
                Some(u) => actual != u.expected && (actual != u.before || u.receipt.is_some()),
                None => actual != i.before,
            }
        });
    if conflict {
        state
            .placement
            .as_mut()
            .expect("active")
            .requires_inspection
            .get_or_insert_with(|| "stair shape input unavailable or changed".into());
    }
}
pub(super) fn chunk_changed(state: &mut State, chunk: [i32; 2]) {
    if !active(state) {
        return;
    }
    if state
        .common_placement
        .as_ref()
        .and_then(|c| c.stairs.as_ref())
        .is_some_and(|c| {
            c.inputs
                .iter()
                .any(|i| [i.position[0].div_euclid(16), i.position[2].div_euclid(16)] == chunk)
        })
    {
        state
            .placement
            .as_mut()
            .expect("active")
            .requires_inspection
            .get_or_insert_with(|| "stair shape input chunk unloaded or replaced".into());
    }
}
pub(super) fn record_in(
    state: &State,
) -> Result<Option<crate::client::survival::StairPlacementRecord>> {
    let Some(capture) = state
        .common_placement
        .as_ref()
        .filter(|c| c.stairs.is_some())
    else {
        return Ok(state.retired_stair_placement.clone());
    };
    let stairs = capture.stairs.as_ref().expect("stairs");
    let r =
        common_record_in(state)?.ok_or_else(|| unavailable("stair placement capture missing"))?;
    Ok(Some(crate::client::survival::StairPlacementRecord {
        first_outline: capture
            .replacement
            .as_ref()
            .expect("outline")
            .first_outline
            .clone(),
        shape_inputs: stairs.inputs.clone(),
        neighbor_updates: stairs.updates.clone(),
        id: r.id,
        initial: r.initial,
        support: r.support,
        support_state: r.support_state,
        face: r.face,
        cursor: r.cursor,
        target: r.target,
        before: r.before,
        expected: r.expected,
        held_before: r.held_before,
        held_receive_sequence: r.held_receive_sequence,
        send: r.send,
        target_receipt: r.target_receipt,
        material_receipt: r.material_receipt,
        processing: r.processing,
        requires_inspection: r.requires_inspection,
        stage: r.stage,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    fn rows(data: &[u8]) -> Vec<serde_json::Value> {
        let mut text = String::new();
        flate2::read::GzDecoder::new(data)
            .read_to_string(&mut text)
            .unwrap();
        serde_json::from_str(&text).unwrap()
    }
    #[test]
    fn original_stair_exhaustive_neighbors_and_corner_blockers() {
        let rows = rows(include_bytes!(
            "../../../../../../data/client_api/stair_placement.json.gz"
        ));
        assert_eq!(rows.len(), 52488);
        for row in rows {
            let state = serde_json::from_value(row["state"].clone()).unwrap();
            let neighbors: Vec<crate::NativeBlockState> =
                serde_json::from_value(row["neighbors"].clone()).unwrap();
            for center in [[0, 2, 0], [15, 2, 15], [-16, 2, -16]] {
                let actual = recalculate(&state, center, |p| {
                    let index = (0..4)
                        .find(|i| step(center, *i).unwrap() == p)
                        .expect("only direct neighbors");
                    Ok(neighbors[index].clone())
                })
                .unwrap();
                assert_eq!(
                    serde_json::to_value(actual).unwrap(),
                    row["expected"],
                    "{row}"
                );
            }
        }
    }
    #[test]
    fn original_stair_yaw_diagonal_and_half_boundaries_all_faces() {
        let rows = rows(include_bytes!(
            "../../../../../../data/client_api/stair_placement_context.json.gz"
        ));
        assert_eq!(rows.len(), 1386);
        for row in rows {
            let actual = base(
                row["yaw"].as_f64().unwrap() as f32,
                row["face"].as_u64().unwrap() as u8,
                row["hit_y"].as_f64().unwrap() as f32,
            );
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                row["expected"],
                "{row}"
            );
        }
    }
    #[test]
    fn original_stair_outline_clips_include_diagonal_half_edges_and_far_coordinates() {
        let rows = rows(include_bytes!(
            "../../../../../../data/client_api/stair_placement_geometry.json.gz"
        ));
        assert_eq!(rows.len(), 3200);
        for row in rows {
            let start = serde_json::from_value(row["start"].clone()).unwrap();
            let end = serde_json::from_value(row["end"].clone()).unwrap();
            let cell: [i32; 3] = serde_json::from_value(row["cell"].clone()).unwrap();
            let id = row["state_index"].as_i64().unwrap() as i32;
            let actual =
                super::super::super::super::super::raycast::replacement_cast_in(start, end, |p| {
                    crate::versions::java_1_21_11::native_state(if p == cell { id } else { 0 })
                        .map_err(Into::into)
                })
                .unwrap();
            assert_eq!(actual.is_some(), !row["hit"].is_null(), "{row}");
            if let Some(actual) = actual {
                assert_eq!(
                    format!("{:?}", actual.face).to_lowercase(),
                    row["hit"]["face"],
                    "{row}"
                );
                let point: [f64; 3] = serde_json::from_value(row["hit"]["point"].clone()).unwrap();
                for (a, b) in actual.point.into_iter().zip(point) {
                    assert!((a - b).abs() < 1e-9, "{row}");
                }
            }
        }
    }
    #[test]
    fn missing_or_unqualified_shape_input_never_becomes_straight() {
        let state = base(0., 1, 0.);
        assert!(recalculate(&state, [0, 0, 0], |_| Err(unavailable("missing"))).is_err());
        assert!(!admitted(&crate::NativeBlockState {
            name: "minecraft:oak_stairs".into(),
            properties: Default::default()
        }));
        let mut wet = state;
        wet.properties.insert("waterlogged".into(), "true".into());
        assert!(!admitted(&wet));
    }
}
