//! State-only storage outlines from both original native JARs; animated shapes absent.
use crate::{MinecraftVersion, NativeBlockState};
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Deserialize)]
struct StateShape {
    state: NativeBlockState,
    #[cfg(test)]
    native_id: i32,
    outline: Vec<[f64; 6]>,
    auxiliary: Vec<[f64; 6]>,
}
#[derive(Deserialize)]
struct Shapes {
    states: Vec<StateShape>,
}
fn shapes(version: MinecraftVersion) -> &'static Shapes {
    static LEGACY: OnceLock<Shapes> = OnceLock::new();
    static MODERN: OnceLock<Shapes> = OnceLock::new();
    let (cell, text) = match version {
        MinecraftVersion::Java1_16_1 => (
            &LEGACY,
            include_str!("../../../data/client_api/storage_outlines-1.16.1.json"),
        ),
        MinecraftVersion::Java1_21_11 => (
            &MODERN,
            include_str!("../../../data/client_api/storage_outlines-1.21.11.json"),
        ),
    };
    cell.get_or_init(|| serde_json::from_str(text).expect("validated native storage shapes"))
}
type Boxes = (&'static [[f64; 6]], &'static [[f64; 6]]);
/// Exact complete properties and selected version bind each shape. No numeric
/// identity from a different registry or missing properties can select it.
pub(crate) fn lookup(version: MinecraftVersion, state: &NativeBlockState) -> Option<Boxes> {
    shapes(version)
        .states
        .iter()
        .find(|s| s.state == *state)
        .map(|s| (s.outline.as_slice(), s.auxiliary.as_slice()))
        .or_else(|| crate::client::crafting::outline::lookup(version, state))
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeSet, io::Read};
    #[derive(Deserialize)]
    struct Hit {
        point: [f64; 3],
        face: String,
    }
    #[derive(Deserialize)]
    struct Ray {
        state_index: usize,
        cell: [i32; 3],
        start: [f64; 3],
        end: [f64; 3],
        hit: Option<Hit>,
    }
    #[test]
    fn storage_shapes_cover_exact_native_states_on_both_versions() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let states = &shapes(version).states;
            assert_eq!(states.len(), 102);
            let names: BTreeSet<_> = states.iter().map(|s| s.state.name.as_str()).collect();
            assert_eq!(
                names,
                BTreeSet::from([
                    "minecraft:chest",
                    "minecraft:trapped_chest",
                    "minecraft:barrel",
                    "minecraft:hopper",
                    "minecraft:dispenser",
                    "minecraft:dropper",
                    "minecraft:ender_chest"
                ])
            );
            let mut ids = BTreeSet::new();
            for s in states {
                let id = match version {
                    MinecraftVersion::Java1_16_1 => {
                        crate::versions::java_1_16_1::state_id(&s.state).unwrap()
                    }
                    MinecraftVersion::Java1_21_11 => {
                        crate::versions::java_1_21_11::state_id(&s.state).unwrap()
                    }
                };
                assert_eq!(id, s.native_id);
                assert!(ids.insert(id));
                assert!(!s.outline.is_empty());
                let mut incomplete = s.state.clone();
                incomplete.properties.pop_first();
                assert!(lookup(version, &incomplete).is_none());
            }
            assert!(
                lookup(
                    version,
                    &NativeBlockState {
                        name: "minecraft:shulker_box".into(),
                        properties: Default::default()
                    }
                )
                .is_none()
            );
        }
    }
    #[test]
    fn shared_storage_targeting_matches_original_native_clips() {
        for (version, bytes) in [
            (
                MinecraftVersion::Java1_16_1,
                &include_bytes!("../../../data/client_api/storage_outline_rays-1.16.1.json.gz")[..],
            ),
            (
                MinecraftVersion::Java1_21_11,
                &include_bytes!("../../../data/client_api/storage_outline_rays-1.21.11.json.gz")[..],
            ),
        ] {
            let mut text = String::new();
            flate2::read::GzDecoder::new(bytes)
                .read_to_string(&mut text)
                .unwrap();
            let rays: Vec<Ray> = serde_json::from_str(&text).unwrap();
            assert_eq!(rays.len(), 8976);
            for (index, ray) in rays.iter().enumerate() {
                let state = &shapes(version).states[ray.state_index].state;
                let air = NativeBlockState {
                    name: "minecraft:air".into(),
                    properties: Default::default(),
                };
                let got = crate::client::survival::target::cast(
                    ray.start,
                    ray.end,
                    |p| {
                        Ok(if p == ray.cell {
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
                match (got, &ray.hit) {
                    (None, None) => {}
                    (Some(got), Some(want)) => {
                        assert_eq!(got.position, ray.cell, "{version:?} case {index}");
                        assert_eq!(
                            format!("{:?}", got.face).to_lowercase(),
                            want.face,
                            "{version:?} case {index}"
                        );
                        for (a, b) in got.point.into_iter().zip(want.point) {
                            assert!((a - b).abs() < 1e-9, "{version:?} case {index}: {a} != {b}");
                        }
                    }
                    (a, b) => panic!(
                        "{version:?} native storage clip {index} differs: {a:?} vs hit {}",
                        b.is_some()
                    ),
                }
            }
        }
    }
}
