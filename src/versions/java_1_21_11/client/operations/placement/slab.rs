//! Finite dry oak-slab placement rules, qualified by the original context oracle.
use super::*;

const FAMILY: &str = "minecraft:oak_slab";
const TYPES: &[(&str, [f64; 6])] = &[
    ("top", [0., 0.5, 0., 1., 1., 1.]),
    ("bottom", [0., 0., 0., 1., 0.5, 1.]),
    ("double", [0., 0., 0., 1., 1., 1.]),
];
fn kind(state: &crate::NativeBlockState) -> Option<&str> {
    (state.name == FAMILY
        && state.properties.len() == 2
        && state.properties.get("waterlogged").map(String::as_str) == Some("false"))
    .then(|| state.properties.get("type").map(String::as_str))
    .flatten()
    .filter(|value| TYPES.iter().any(|(name, _)| name == value))
}
fn shape(state: &crate::NativeBlockState) -> Option<[f64; 6]> {
    let kind = kind(state)?;
    TYPES
        .iter()
        .find(|(name, _)| *name == kind)
        .map(|(_, b)| *b)
}
fn replace_clicked(state: &crate::NativeBlockState, face: u8, hit_y: f32) -> bool {
    match kind(state) {
        Some("bottom") => face == 1 || (face >= 2 && hit_y > 0.5),
        Some("top") => face == 0 || (face >= 2 && hit_y <= 0.5),
        _ => false,
    }
}
pub(super) fn expected(
    before: &crate::NativeBlockState,
    face: u8,
    hit_y: f32,
) -> crate::NativeBlockState {
    let kind = if kind(before).is_some() {
        "double"
    } else if face == 0 || (face >= 2 && hit_y > 0.5) {
        "top"
    } else {
        "bottom"
    };
    crate::NativeBlockState {
        name: FAMILY.into(),
        properties: [
            ("type".into(), kind.into()),
            ("waterlogged".into(), "false".into()),
        ]
        .into(),
    }
}
pub(super) fn geometry(
    view: &impl GeometryView,
    standing: &survival::StandingContext,
    clicked: [i32; 3],
    face: u8,
    requested: ([i32; 3], &crate::NativeBlockState),
    rotation: [f32; 2],
) -> Result<PlacementGeometry> {
    if face > 5 {
        return Err(invalid("invalid slab placement face"));
    }
    let eye = standing.eye_position;
    let hit = super::super::super::raycast::outline_hit_in(eye, rotation, 4.5, |p| {
        view.block(p).map_err(anyhow::Error::from)
    })?
    .ok_or_else(|| unavailable("no first native outline hit for slab placement"))?;
    if hit.position != clicked
        || hit.face.map(|f| f as u8) != Some(face)
        || !(survival::DRY_CUBES.contains(&hit.state.name.as_str()) || kind(&hit.state).is_some())
    {
        return Err(unavailable(
            "slab placement requires the first cube or dry oak slab outline face",
        ));
    }
    // The existing continuous uncertainty proof qualifies full cubes only.
    // Do not infer half selection or merging from sampled uncertain rays.
    if standing.position_basis.geometry_reserve() != [0.; 3] {
        return Err(unavailable(
            "slab placement requires an exact received stationary position",
        ));
    }
    let cursor = super::super::super::raycast::hit_cursor_in(rotation, eye, &hit);
    let target = if replace_clicked(&hit.state, face, cursor[1]) {
        clicked
    } else {
        crate::client::survival::placement::adjacent(
            clicked,
            [
                crate::BlockFace::Down,
                crate::BlockFace::Up,
                crate::BlockFace::North,
                crate::BlockFace::South,
                crate::BlockFace::West,
                crate::BlockFace::East,
            ][face as usize],
        )?
    };
    let before = view.block(target)?;
    if target != requested.0
        || &before != requested.1
        || !(air(&before) || matches!(kind(&before), Some("top" | "bottom")))
    {
        return Err(unavailable(
            "slab placement requires the exact air or replaceable dry oak slab destination",
        ));
    }
    Ok(PlacementGeometry {
        target,
        before,
        hit,
        cursor,
    })
}
fn intersects(a: [f64; 6], b: [f64; 6]) -> bool {
    (0..3).all(|i| a[i] < b[i + 3] && a[i + 3] > b[i])
}
fn array(b: crate::client::Aabb) -> [f64; 6] {
    [b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z]
}
pub(super) fn unobstructed(
    state: &State,
    connection_id: u64,
    own: [f64; 6],
    target: [i32; 3],
    expected: &crate::NativeBlockState,
) -> Result<()> {
    let local = shape(expected).expect("qualified dry oak slab");
    let bounds = std::array::from_fn(|i| local[i] + f64::from(target[i % 3]));
    if intersects(own, bounds) {
        return Err(unavailable("slab shape intersects the standing body"));
    }
    let version = crate::MinecraftVersion::Java1_21_11;
    let entities = state.entities.capture_all(
        version,
        crate::client::SessionStamp {
            version,
            connection_id,
            world_generation: state.loading.generation,
        },
        state.sequence,
    );
    for entity in entities.entities {
        if entity.motion.entity.type_name.is_none() {
            return Err(unavailable("slab obstruction entity type unavailable"));
        }
        if !crate::client::placement_check::blocks_building(version, &entity) {
            continue;
        }
        // Use qualified received pose/baby/scale, never the historical default
        // living box. Fixed nonliving shapes are finite; all other types refuse.
        let bounds_entity = if entity.living == Some(true) {
            entity
                .derived_geometry()
                .map(|g| {
                    let mut b = array(g.bounds);
                    for i in 0..3 {
                        b[i] -= g.position_error[i];
                        b[i + 3] += g.position_error[i];
                    }
                    b
                })
                .map_err(|_| unavailable("slab obstruction requires qualified entity geometry"))?
        } else if [
            "minecraft:tnt",
            "minecraft:falling_block",
            "minecraft:end_crystal",
        ]
        .contains(
            &entity
                .motion
                .entity
                .type_name
                .as_deref()
                .expect("known type"),
        ) {
            let position = entity
                .motion
                .position
                .as_ref()
                .ok_or_else(|| unavailable("slab obstruction entity position unavailable"))?;
            let dimensions = version
                .table()
                .entity_dimensions(
                    entity
                        .motion
                        .entity
                        .type_name
                        .as_deref()
                        .expect("known type"),
                )
                .expect("qualified fixed dimensions");
            let half = f64::from(dimensions.width as f32 / 2.0);
            let height = f64::from(dimensions.height as f32);
            let p = position.value.position;
            let error = position.value.quantization_error;
            [
                p[0] - half - error[0],
                p[1] - error[1],
                p[2] - half - error[2],
                p[0] + half + error[0],
                p[1] + height + error[1],
                p[2] + half + error[2],
            ]
        } else {
            return Err(unavailable("slab obstruction entity geometry unsupported"));
        };
        if bounds_entity.iter().any(|value| !value.is_finite()) {
            return Err(unavailable("slab obstruction entity geometry is nonfinite"));
        }
        if intersects(bounds_entity, bounds) {
            return Err(unavailable(
                "slab shape intersects a building-blocking entity",
            ));
        }
    }
    Ok(())
}

pub(super) fn record_in(
    state: &State,
) -> Result<Option<crate::client::survival::SlabPlacementRecord>> {
    let Some(capture) = state
        .common_placement
        .as_ref()
        .filter(|c| c.slab)
        .and_then(|c| c.replacement.as_ref())
    else {
        return Ok(state.retired_slab_placement.clone());
    };
    let r =
        common_record_in(state)?.ok_or_else(|| unavailable("slab placement capture missing"))?;
    Ok(Some(crate::client::survival::SlabPlacementRecord {
        first_outline: capture.first_outline.clone(),
        merges_clicked: capture.replaces_clicked,
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
    fn original_slab_context_matches_all_faces_boundary_heights_and_destination_rules() {
        let rows = rows(include_bytes!(
            "../../../../../../data/client_api/slab_placement.json.gz"
        ));
        assert_eq!(rows.len(), 2520);
        let mut checked = 0;
        for row in rows {
            let before: crate::NativeBlockState =
                serde_json::from_value(row["before"].clone()).unwrap();
            let face = row["face"].as_u64().unwrap() as u8;
            let y = row["hit_y"].as_f64().unwrap() as f32;
            let adjacent = row["adjacent"].as_bool().unwrap();
            let clicked = if adjacent {
                crate::NativeBlockState {
                    name: "minecraft:stone".into(),
                    properties: Default::default(),
                }
            } else {
                before.clone()
            };
            // Clicked air, foreign slabs and wet states are outside this API.
            if !(kind(&clicked).is_some() || clicked.name == "minecraft:stone") {
                continue;
            }
            let merges = replace_clicked(&clicked, face, y);
            assert_eq!(merges, row["replace_clicked"], "{row}");
            let destination = if merges {
                [2, 2, 0]
            } else {
                let d = super::super::offset(face);
                [2 + d[0], 2 + d[1], d[2]]
            };
            assert_eq!(serde_json::json!(destination), row["destination"]);
            let target_before = crate::versions::java_1_21_11::native_state(
                row["destination_state"].as_i64().unwrap() as i32,
            )
            .unwrap();
            let admitted =
                air(&target_before) || matches!(kind(&target_before), Some("top" | "bottom"));
            if admitted {
                assert_eq!(row["can_place"], true);
                let actual = expected(&target_before, face, y);
                assert_eq!(
                    serde_json::to_value(&actual).unwrap(),
                    row["placed_state"],
                    "{row}"
                );
                assert_eq!(
                    serde_json::json!([shape(&actual).unwrap()]),
                    row["collision"]
                );
                checked += 1;
            } else if target_before.name == "minecraft:spruce_slab" {
                assert_eq!(row["can_place"], false);
            }
        }
        assert_eq!(checked, 588);
    }
    #[test]
    fn original_slab_shape_obstruction_checks_the_half_instead_of_the_cell() {
        let rows = rows(include_bytes!(
            "../../../../../../data/client_api/slab_placement_obstruction.json.gz"
        ));
        assert_eq!(rows.len(), 216);
        for row in rows {
            let state = serde_json::from_value(row["state"].clone()).unwrap();
            let body = serde_json::from_value(row["body"].clone()).unwrap();
            assert_eq!(
                intersects(body, shape(&state).unwrap()),
                row["intersects"],
                "{row}"
            );
        }
    }
    #[test]
    fn original_slab_outline_faces_and_points_match_at_half_edges() {
        let rows = rows(include_bytes!(
            "../../../../../../data/client_api/slab_placement_geometry.json.gz"
        ));
        assert_eq!(rows.len(), 360);
        for row in rows {
            let start = serde_json::from_value(row["start"].clone()).unwrap();
            let end = serde_json::from_value(row["end"].clone()).unwrap();
            let cell: [i32; 3] = serde_json::from_value(row["cell"].clone()).unwrap();
            let id = row["state_index"].as_i64().unwrap() as i32;
            let actual =
                super::super::super::super::raycast::replacement_cast_in(start, end, |p| {
                    crate::versions::java_1_21_11::native_state(if p == cell { id } else { 0 })
                        .map_err(Into::into)
                })
                .unwrap();
            assert_eq!(actual.is_some(), !row["hit"].is_null(), "{row}");
            if let Some(actual) = actual {
                assert_eq!(
                    format!("{:?}", actual.face).to_lowercase(),
                    row["hit"]["face"]
                );
                let point: [f64; 3] = serde_json::from_value(row["hit"]["point"].clone()).unwrap();
                for (a, b) in actual.point.into_iter().zip(point) {
                    assert!((a - b).abs() < 1e-9);
                }
            }
        }
    }
}
