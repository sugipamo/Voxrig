use super::*;
use crate::client::survival::{model, target};

fn oracle(version: MinecraftVersion) -> serde_json::Value {
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(bytes(version))
        .read_to_end(&mut raw)
        .unwrap();
    serde_json::from_slice(&raw).unwrap()
}
#[test]
fn dry_terrain_registered_states_bind_exact_native_registry_and_defaults() {
    for (version, expected_blocks, expected_states) in [
        (MinecraftVersion::Java1_16_1, 76, 1560),
        (MinecraftVersion::Java1_21_11, 112, 2334),
    ] {
        let facts = oracle(version);
        assert_eq!(facts["blocks"].as_array().unwrap().len(), expected_blocks);
        assert_eq!(facts["states"].as_array().unwrap().len(), expected_states);
        for block in facts["blocks"].as_array().unwrap() {
            assert_eq!(
                block["material"],
                serde_json::json!([f64::from(0.6f32), 1., 1.])
            );
        }
        let mut ids = std::collections::BTreeSet::new();
        for variants in shapes(version).values() {
            for shape in variants.values() {
                let id = match version {
                    MinecraftVersion::Java1_16_1 => {
                        crate::versions::java_1_16_1::state_id(&shape.state).unwrap()
                    }
                    MinecraftVersion::Java1_21_11 => {
                        crate::versions::java_1_21_11::state_id(&shape.state).unwrap()
                    }
                };
                assert_eq!(id, shape.native_id);
                assert!(ids.insert(id));
                assert_eq!(
                    shape
                        .state
                        .properties
                        .get("waterlogged")
                        .map(String::as_str),
                    Some("false")
                );
                assert!(!shape.collision.is_empty());
                for b in &shape.collision {
                    for axis in 0..3 {
                        assert!(b[axis] >= 0. && b[axis] < b[axis + 3] && b[axis + 3] <= 1.);
                    }
                }
                let mut bad = shape.state.clone();
                bad.properties.pop_first();
                assert!(lookup(version, &bad).is_none());
                bad = shape.state.clone();
                bad.properties.insert("waterlogged".into(), "true".into());
                assert!(model::collision_shape(version, &bad).is_err());
            }
        }
        assert_eq!(ids.len(), expected_states);
    }
}
#[test]
fn dry_terrain_admitted_collision_matches_original_combined_voxel_shapes() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let facts = oracle(version);
        let cases = facts["collisions"].as_array().unwrap();
        assert_eq!(cases.len(), 810);
        let mut admitted = 0;
        let mut initially_intersecting = 0;
        for row in cases {
            let state: NativeBlockState = serde_json::from_value(
                facts["states"][row["state_index"].as_u64().unwrap() as usize]["state"].clone(),
            )
            .unwrap();
            let position = serde_json::from_value(row["position"].clone()).unwrap();
            let motion = serde_json::from_value(row["motion"].clone()).unwrap();
            let expected: [f64; 3] = serde_json::from_value(row["expected"].clone()).unwrap();
            let body = model::body(position);
            let boxes = &lookup(version, &state).unwrap().collision;
            let overlaps = boxes.iter().any(|bounds| {
                (0..3).all(|axis| {
                    body[axis] + 1e-7 < bounds[axis + 3] && body[axis + 3] - 1e-7 > bounds[axis]
                })
            });
            assert_eq!(
                overlaps,
                row["initial_intersection"].as_bool().unwrap(),
                "{version:?} {state:?} {row}"
            );
            if overlaps {
                // Both adapters refuse a standing seed inside terrain. Original
                // combined-shape results are retained for these invalid seeds;
                // box decomposition does not claim parity inside a solid.
                initially_intersecting += 1;
                continue;
            }
            admitted += 1;
            let actual =
                model::collide_geometry(body, motion, &model::CollisionGeometry::joined(boxes));
            for axis in 0..3 {
                assert!(
                    (actual[axis] - expected[axis]).abs() < 1e-10,
                    "{version:?} {state:?} {row}"
                );
            }
        }
        assert_eq!((admitted, initially_intersecting), (654, 156));
    }
}
#[test]
fn dry_terrain_targeting_matches_original_native_clips() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let facts = oracle(version);
        let cases = facts["rays"].as_array().unwrap();
        assert_eq!(cases.len(), 810);
        for row in cases {
            let state: NativeBlockState = serde_json::from_value(
                facts["states"][row["state_index"].as_u64().unwrap() as usize]["state"].clone(),
            )
            .unwrap();
            let cell: [i32; 3] = serde_json::from_value(row["cell"].clone()).unwrap();
            let start: [f64; 3] = serde_json::from_value(row["start"].clone()).unwrap();
            let end: [f64; 3] = serde_json::from_value(row["end"].clone()).unwrap();
            let hit = target::cast(
                start,
                end,
                |p| {
                    Ok(if p == cell {
                        state.clone()
                    } else {
                        NativeBlockState {
                            name: "minecraft:air".into(),
                            properties: Default::default(),
                        }
                    })
                },
                |s| {
                    Ok(if s.name == "minecraft:air" {
                        (&[][..], &[][..])
                    } else {
                        outlines(version, s).unwrap()
                    })
                },
            )
            .unwrap();
            if row["hit"].is_null() {
                assert!(hit.is_none(), "{state:?} {row}");
            } else {
                let actual = hit.unwrap();
                assert_eq!(actual.position, cell);
                let expected: [f64; 3] =
                    serde_json::from_value(row["hit"]["point"].clone()).unwrap();
                for (a, b) in actual.point.iter().zip(expected) {
                    assert!((a - b).abs() < 1e-10, "{state:?} {row}");
                }
                assert_eq!(
                    format!("{:?}", actual.face).to_lowercase(),
                    row["hit"]["face"].as_str().unwrap()
                );
            }
        }
    }
}
