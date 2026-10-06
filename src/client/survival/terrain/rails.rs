//! Registered native dry rails: empty collision and independently received outlines.
use super::*;
fn bytes(version: MinecraftVersion) -> &'static [u8] {
    match version {
        MinecraftVersion::Java1_16_1 => {
            include_bytes!("../../../../data/client_api/rail_terrain-1.16.1.json.gz")
        }
        MinecraftVersion::Java1_21_11 => {
            include_bytes!("../../../../data/client_api/rail_terrain-1.21.11.json.gz")
        }
    }
}
fn shapes(version: MinecraftVersion) -> &'static Shapes {
    static LEGACY: OnceLock<Shapes> = OnceLock::new();
    static MODERN: OnceLock<Shapes> = OnceLock::new();
    let cell = match version {
        MinecraftVersion::Java1_16_1 => &LEGACY,
        MinecraftVersion::Java1_21_11 => &MODERN,
    };
    cell.get_or_init(|| {
        let mut decoded = Vec::new();
        flate2::read::GzDecoder::new(bytes(version))
            .read_to_end(&mut decoded)
            .expect("packaged original rail gzip");
        let terrain: Terrain =
            serde_json::from_slice(&decoded).expect("validated native rail states");
        let mut result: Shapes = BTreeMap::new();
        for shape in terrain.states {
            let previous = result
                .entry(shape.state.name.clone())
                .or_default()
                .insert(shape.state.properties.clone(), shape);
            assert!(previous.is_none(), "unique original rail state");
        }
        result
    })
}
pub(super) fn lookup(
    version: MinecraftVersion,
    state: &NativeBlockState,
) -> Option<&'static Shape> {
    shapes(version)
        .get(&state.name)
        .and_then(|states| states.get(&state.properties))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::survival::{model, target};
    fn oracle(version: MinecraftVersion) -> serde_json::Value {
        let mut decoded = Vec::new();
        flate2::read::GzDecoder::new(bytes(version))
            .read_to_end(&mut decoded)
            .unwrap();
        serde_json::from_slice(&decoded).unwrap()
    }
    #[test]
    fn dry_rails_bind_all_registered_native_states_without_wet_or_missing_fallback() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let facts = oracle(version);
            assert_eq!(facts["blocks"].as_array().unwrap().len(), 4);
            assert_eq!(facts["states"].as_array().unwrap().len(), 46);
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
                    assert!(
                        model::collision_shape(version, &shape.state)
                            .unwrap()
                            .is_empty()
                    );
                    assert!(!shape.outline.is_empty());
                    assert_eq!(
                        super::super::lookup(version, &shape.state).unwrap().state,
                        shape.state
                    );
                    let mut bad = shape.state.clone();
                    bad.properties.pop_first();
                    assert!(lookup(version, &bad).is_none());
                    bad = shape.state.clone();
                    bad.properties.insert("waterlogged".into(), "true".into());
                    assert!(model::collision_shape(version, &bad).is_err());
                }
            }
            assert_eq!(ids.len(), 46);
            for block in facts["blocks"].as_array().unwrap() {
                assert_eq!(
                    block["material"],
                    serde_json::json!([f64::from(0.6f32), 1.0, 1.0])
                );
            }
        }
    }
    #[test]
    fn dry_rails_collision_and_outline_match_original_native_game_methods() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let facts = oracle(version);
            for row in facts["collisions"].as_array().unwrap() {
                let state: NativeBlockState = serde_json::from_value(
                    facts["states"][row["state_index"].as_u64().unwrap() as usize]["state"].clone(),
                )
                .unwrap();
                let position: [f64; 3] = serde_json::from_value(row["position"].clone()).unwrap();
                let motion: [f64; 3] = serde_json::from_value(row["motion"].clone()).unwrap();
                let expected: [f64; 3] = serde_json::from_value(row["expected"].clone()).unwrap();
                assert_eq!(row["initial_intersection"], false);
                let actual = model::collide_geometry(
                    model::body(position),
                    motion,
                    &model::CollisionGeometry::joined(
                        model::collision_shape(version, &state).unwrap(),
                    ),
                );
                for (actual, expected) in actual.into_iter().zip(expected) {
                    assert!((actual - expected).abs() < 1e-10, "{version:?} {state:?}");
                }
            }
            for row in facts["rays"].as_array().unwrap() {
                let state: NativeBlockState = serde_json::from_value(
                    facts["states"][row["state_index"].as_u64().unwrap() as usize]["state"].clone(),
                )
                .unwrap();
                let cell: [i32; 3] = serde_json::from_value(row["cell"].clone()).unwrap();
                let start = serde_json::from_value(row["start"].clone()).unwrap();
                let end = serde_json::from_value(row["end"].clone()).unwrap();
                let actual = target::cast(
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
                            super::super::outlines(version, s).unwrap()
                        })
                    },
                )
                .unwrap();
                if row["hit"].is_null() {
                    assert!(actual.is_none());
                } else {
                    let hit = actual.unwrap();
                    assert_eq!(hit.position, cell);
                    assert_eq!(
                        format!("{:?}", hit.face).to_lowercase(),
                        row["hit"]["face"].as_str().unwrap()
                    );
                    let expected: [f64; 3] =
                        serde_json::from_value(row["hit"]["point"].clone()).unwrap();
                    for (actual, expected) in hit.point.into_iter().zip(expected) {
                        assert!((actual - expected).abs() < 1e-10);
                    }
                }
            }
        }
    }
}
