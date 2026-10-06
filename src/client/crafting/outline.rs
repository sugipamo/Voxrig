//! Original crafting-table state and native outline/auxiliary boxes.
use crate::{MinecraftVersion, NativeBlockState};
use std::sync::OnceLock;

#[derive(serde::Deserialize)]
struct Shapes {
    states: Vec<StateShape>,
}
#[derive(serde::Deserialize)]
struct StateShape {
    state: NativeBlockState,
    outline: Vec<[f64; 6]>,
    auxiliary: Vec<[f64; 6]>,
}
fn shapes(version: MinecraftVersion) -> &'static Shapes {
    static LEGACY: OnceLock<Shapes> = OnceLock::new();
    static MODERN: OnceLock<Shapes> = OnceLock::new();
    let parse = |bytes: &[u8]| {
        serde_json::from_reader(flate2::read::GzDecoder::new(bytes))
            .expect("pinned original crafting-table outlines")
    };
    match version {
        MinecraftVersion::Java1_16_1 => LEGACY.get_or_init(|| {
            parse(include_bytes!(
                "../../../data/client_api/crafting_outlines-1.16.1.json.gz"
            ))
        }),
        MinecraftVersion::Java1_21_11 => MODERN.get_or_init(|| {
            parse(include_bytes!(
                "../../../data/client_api/crafting_outlines-1.21.11.json.gz"
            ))
        }),
    }
}
type Boxes = (&'static [[f64; 6]], &'static [[f64; 6]]);
pub(crate) fn lookup(version: MinecraftVersion, state: &NativeBlockState) -> Option<Boxes> {
    shapes(version)
        .states
        .iter()
        .find(|s| s.state == *state)
        .map(|s| (s.outline.as_slice(), s.auxiliary.as_slice()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bytes(version: MinecraftVersion) -> &'static [u8] {
        match version {
            MinecraftVersion::Java1_16_1 => {
                include_bytes!("../../../data/client_api/crafting_outlines-1.16.1.json.gz")
            }
            MinecraftVersion::Java1_21_11 => {
                include_bytes!("../../../data/client_api/crafting_outlines-1.21.11.json.gz")
            }
        }
    }
    #[test]
    fn crafting_table_outline_matches_original_owners_states_and_clips() {
        for (version, block_class, owner) in [
            (MinecraftVersion::Java1_16_1, "bwu", "cfi"),
            (MinecraftVersion::Java1_21_11, "ebn", "eog"),
        ] {
            let native: serde_json::Value =
                serde_json::from_reader(flate2::read::GzDecoder::new(bytes(version))).unwrap();
            assert_eq!(native["native_class"], block_class);
            assert_eq!(native["outline_method_owner"], owner);
            assert_eq!(native["auxiliary_method_owner"], owner);
            assert_eq!(native["states"].as_array().unwrap().len(), 1);
            let entry = &native["states"][0];
            let state: NativeBlockState = serde_json::from_value(entry["state"].clone()).unwrap();
            let id = match version {
                MinecraftVersion::Java1_16_1 => {
                    crate::versions::java_1_16_1::state_id(&state).unwrap()
                }
                MinecraftVersion::Java1_21_11 => {
                    crate::versions::java_1_21_11::state_id(&state).unwrap()
                }
            };
            assert_eq!(entry["native_id"], id);
            assert_eq!(
                lookup(version, &state).unwrap(),
                (&[[0., 0., 0., 1., 1., 1.]][..], &[][..])
            );
            assert_eq!(
                crate::client::container::outline::lookup(version, &state),
                lookup(version, &state)
            );
            let mut invalid = state.clone();
            invalid
                .properties
                .insert("uninspected".into(), "true".into());
            assert!(lookup(version, &invalid).is_none());
            let air = NativeBlockState {
                name: "minecraft:air".into(),
                properties: Default::default(),
            };
            let rays = native["rays"].as_array().unwrap();
            assert_eq!(rays.len(), 88);
            for (i, ray) in rays.iter().enumerate() {
                let cell: [i32; 3] = serde_json::from_value(ray["cell"].clone()).unwrap();
                let start = serde_json::from_value(ray["start"].clone()).unwrap();
                let end = serde_json::from_value(ray["end"].clone()).unwrap();
                let got = crate::client::survival::target::cast(
                    start,
                    end,
                    |p| {
                        Ok(if p == cell {
                            state.clone()
                        } else {
                            air.clone()
                        })
                    },
                    |s| {
                        Ok(if s == &air {
                            (&[][..], &[][..])
                        } else {
                            lookup(version, s).unwrap()
                        })
                    },
                )
                .unwrap();
                let want = &ray["hit"];
                if want.is_null() {
                    assert!(got.is_none(), "{version:?} case{i}");
                } else {
                    let hit = got.unwrap();
                    assert_eq!(hit.position, cell);
                    assert_eq!(format!("{:?}", hit.face).to_lowercase(), want["face"]);
                    let point: [f64; 3] = serde_json::from_value(want["point"].clone()).unwrap();
                    for (a, b) in hit.point.into_iter().zip(point) {
                        assert!((a - b).abs() < 1e-9, "{version:?} case{i}");
                    }
                }
            }
        }
    }
    #[test]
    fn crafting_table_original_outline_evidence_binds_sources_and_outputs() {
        use sha2::{Digest, Sha256};
        let evidence: serde_json::Value = serde_json::from_str(include_str!(
            "../../../data/client_api/crafting_outline_source.json"
        ))
        .unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for object in std::iter::once(&evidence["generators_sha256"]).chain(
            evidence["runs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| &r["files_sha256"]),
        ) {
            for (path, sha) in object.as_object().unwrap() {
                assert_eq!(
                    hex::encode(Sha256::digest(std::fs::read(root.join(path)).unwrap())),
                    sha.as_str().unwrap(),
                    "{path}"
                );
            }
        }
    }
}
