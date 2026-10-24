//! Original dry natural collision shapes; selection outlines are position dependent.
use super::*;
// Collision-only fixtures stay separate from the frozen contact table. This
// exhaustive version match still prevents a default or cross-version fallback.
fn data(version: MinecraftVersion) -> &'static [u8] {
    match version {
        MinecraftVersion::Java1_16_1 => {
            include_bytes!("../../../../data/client_api/natural_terrain-1.16.1.json.gz")
        }
        MinecraftVersion::Java1_21_11 => {
            include_bytes!("../../../../data/client_api/natural_terrain-1.21.11.json.gz")
        }
    }
}
fn shapes(version: MinecraftVersion) -> &'static Shapes {
    static SHAPES: crate::versions::table::PerVersion<Shapes> =
        crate::versions::table::PerVersion::new();
    SHAPES.get(version, |_| {
        let mut decoded = Vec::new();
        flate2::read::GzDecoder::new(data(version))
            .read_to_end(&mut decoded)
            .expect("packaged original natural terrain gzip");
        let terrain: Terrain =
            serde_json::from_slice(&decoded).expect("validated native natural terrain states");
        let mut result: Shapes = BTreeMap::new();
        for shape in terrain.states {
            let previous = result
                .entry(shape.state.name.clone())
                .or_default()
                .insert(shape.state.properties.clone(), shape);
            assert!(previous.is_none(), "unique original natural terrain state");
        }
        result
    })
}
pub(crate) fn lookup(
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
    use crate::client::survival::{SurvivalControl, SurvivalInput, model};
    fn oracle(version: MinecraftVersion) -> serde_json::Value {
        let bytes = data(version);
        let mut decoded = Vec::new();
        flate2::read::GzDecoder::new(bytes)
            .read_to_end(&mut decoded)
            .unwrap();
        serde_json::from_slice(&decoded).unwrap()
    }
    fn state(version: MinecraftVersion, name: &str) -> NativeBlockState {
        shapes(version)[&format!("minecraft:{name}")]
            .values()
            .next()
            .unwrap()
            .state
            .clone()
    }
    #[test]
    fn natural_states_bind_exact_version_registry_and_default_materials() {
        for (version, count) in [
            (MinecraftVersion::Java1_16_1, 525),
            (MinecraftVersion::Java1_21_11, 730),
        ] {
            let facts = oracle(version);
            let mut seen = std::collections::BTreeSet::new();
            for variants in shapes(version).values() {
                for shape in variants.values() {
                    assert_eq!(
                        (version.table().registry.state_id)(&shape.state).unwrap(),
                        shape.native_id
                    );
                    assert!(seen.insert(shape.native_id));
                    assert_eq!(
                        model::collision_shape(version, &shape.state).unwrap(),
                        shape.collision
                    );
                    let mut wrong = shape.state.clone();
                    wrong.properties.insert("unmodeled".into(), "true".into());
                    let error = model::collision_shape(version, &wrong)
                        .unwrap_err()
                        .context("preview");
                    assert_eq!(error.kind(), crate::ErrorKind::Unsupported);
                    assert_eq!(error.terrain_refusal().unwrap().state, wrong);
                    assert_eq!(error.terrain_refusal().unwrap().version, version);
                    if wrong.properties.contains_key("waterlogged") {
                        wrong = shape.state.clone();
                        wrong.properties.insert("waterlogged".into(), "true".into());
                        assert!(model::collision_shape(version, &wrong).is_err());
                    }
                }
            }
            assert_eq!(seen.len(), count);
            for block in facts["blocks"].as_array().unwrap() {
                assert_eq!(
                    block["material"],
                    serde_json::json!([f64::from(0.6f32), 1., 1.])
                );
            }
            for name in ["fern", "large_fern", "tall_grass", "poppy", "oak_sapling"] {
                assert!(
                    model::collision_shape(version, &state(version, name))
                        .unwrap()
                        .is_empty()
                );
            }
            for name in ["oak_leaves", "oak_log", "sand", "clay", "gravel"] {
                assert_eq!(
                    model::collision_shape(version, &state(version, name)).unwrap(),
                    &[[0., 0., 0., 1., 1., 1.]]
                );
            }
            for layers in 1..=8 {
                let snow = NativeBlockState {
                    name: "minecraft:snow".into(),
                    properties: [("layers".into(), layers.to_string())].into(),
                };
                let boxes = model::collision_shape(version, &snow).unwrap();
                if layers == 1 {
                    assert!(boxes.is_empty());
                } else {
                    assert_eq!(boxes, &[[0., 0., 0., 1., f64::from(layers - 1) / 8., 1.]]);
                }
            }
        }
        // No name-based default shapes, cross-version plant aliases or special effects.
        for (version, name) in [
            (MinecraftVersion::Java1_16_1, "short_grass"),
            (MinecraftVersion::Java1_21_11, "grass"),
            (MinecraftVersion::Java1_16_1, "sweet_berry_bush"),
            (MinecraftVersion::Java1_21_11, "powder_snow"),
        ] {
            let block = NativeBlockState {
                name: format!("minecraft:{name}"),
                properties: Default::default(),
            };
            assert_eq!(
                model::collision_shape(version, &block)
                    .unwrap_err()
                    .terrain_refusal()
                    .unwrap()
                    .state,
                block
            );
        }
    }
    #[test]
    fn natural_collision_matches_original_game_combined_shapes() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let facts = oracle(version);
            for row in facts["collisions"].as_array().unwrap() {
                let block: NativeBlockState = serde_json::from_value(
                    facts["states"][row["state_index"].as_u64().unwrap() as usize]["state"].clone(),
                )
                .unwrap();
                if row["initial_intersection"] == true {
                    continue;
                }
                let position = serde_json::from_value(row["position"].clone()).unwrap();
                let motion = serde_json::from_value(row["motion"].clone()).unwrap();
                let expected: [f64; 3] = serde_json::from_value(row["expected"].clone()).unwrap();
                let actual = model::collide_geometry(
                    model::body(position),
                    motion,
                    &model::CollisionGeometry::joined(
                        model::collision_shape(version, &block).unwrap(),
                    ),
                );
                for (a, b) in actual.into_iter().zip(expected) {
                    assert!((a - b).abs() < 1e-10, "{version:?} {block:?} {row}");
                }
            }
        }
    }
    #[test]
    fn finite_path_passes_plants_stops_at_logs_and_steps_on_snow() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let air = NativeBlockState {
                name: "minecraft:air".into(),
                properties: Default::default(),
            };
            let controls = vec![
                SurvivalControl {
                    yaw: 0.,
                    input: SurvivalInput {
                        forward: 1,
                        ..Default::default()
                    }
                };
                40
            ];
            let mut model = model::Model::new(version, [0.5, 1., 0.5]);
            let frames = model::predict(
                |p| {
                    Ok(if p[1] == 0 {
                        state(
                            version,
                            ["sand", "clay", "gravel", "oak_leaves"][(p[2].rem_euclid(4)) as usize],
                        )
                    } else if p[1] == 1 && p[2] < 4 {
                        state(version, "fern")
                    } else if (1..=2).contains(&p[1]) && p[2] == 4 {
                        state(version, "oak_log")
                    } else {
                        air.clone()
                    })
                },
                &mut model,
                &controls,
            )
            .unwrap();
            let last = frames.last().unwrap();
            assert!(
                last.position[2] > 3.0 && last.position[2] < 3.71,
                "{last:?}"
            );
            assert!(last.horizontal_collision);
            let mut model = model::Model::new(version, [0.5, 1., 0.5]);
            let snow = NativeBlockState {
                name: "minecraft:snow".into(),
                properties: [("layers".into(), "4".into())].into(),
            };
            let frames = model::predict(
                |p| {
                    Ok(if p[1] == 0 {
                        state(version, "sand")
                    } else if p[1] == 1 && p[2] >= 2 {
                        snow.clone()
                    } else {
                        air.clone()
                    })
                },
                &mut model,
                &controls,
            )
            .unwrap();
            assert!(frames.last().unwrap().position[2] > 4.);
            assert_eq!(frames.last().unwrap().position[1], 1.375);
        }
    }
}
