//! Java 1.21.11 block outline selection, with no fluid or entity targets.
//! Data and independent native oracle: scripts/ExportOutlineShapes.java.
pub(super) mod uncertainty;
use super::{BlockHit, NativeBlockState};
use crate::versions::java_1_21_11::{reconstruction::Direction, state_id};
use serde::Deserialize;
use std::sync::OnceLock;

/// Native version math is selected explicitly by the shared targeting model.
pub(super) fn direction(rotation: [f32; 2]) -> [f64; 3] {
    crate::client::survival::target::direction(crate::MinecraftVersion::Java1_21_11, rotation)
}

#[derive(Deserialize)]
struct Shapes {
    state_shapes: Vec<Option<[usize; 2]>>,
    shapes: Vec<Vec<[f64; 6]>>,
    #[cfg(test)]
    registry_fnv64: String,
}
fn shapes() -> &'static Shapes {
    static SHAPES: OnceLock<Shapes> = OnceLock::new();
    SHAPES.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../../../data/java_1_21_11/outline_shapes.json"
        ))
        .expect("validated native outline data")
    })
}

/// Native shapes remain selected by this adapter; only traversal is shared.
pub(super) fn cast(
    start: [f64; 3],
    end: [f64; 3],
    read: impl FnMut([i32; 3]) -> anyhow::Result<NativeBlockState>,
) -> anyhow::Result<Option<BlockHit>> {
    crate::client::survival::target::cast(start, end, read, |state| {
        if let Some(shapes) =
            crate::client::container::outline::lookup(crate::MinecraftVersion::Java1_21_11, state)
        {
            return Ok(shapes);
        }
        let data = shapes();
        let pair = data.state_shapes[state_id(state)? as usize]
            .ok_or_else(|| anyhow::anyhow!("outline geometry unsupported: {}", state.name))?;
        Ok((&data.shapes[pair[0]], &data.shapes[pair[1]]))
    })
    .map(|hit| {
        hit.map(|hit| BlockHit {
            position: hit.position,
            state: hit.state,
            distance: hit.distance,
            face: Some(Direction::from_id(hit.face as u8).expect("native face mapping")),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::versions::java_1_21_11::native_state;
    use std::io::Read;
    #[test]
    fn matches_native_entity_rotation_math() {
        #[derive(Deserialize)]
        struct Rotation {
            rotation: [f32; 2],
            direction: [f64; 3],
        }
        let mut text = String::new();
        flate2::read::GzDecoder::new(
            &include_bytes!("../../../../../data/java_1_21_11/outline_rotation_cases.json.gz")[..],
        )
        .read_to_string(&mut text)
        .unwrap();
        let cases: Vec<Rotation> = serde_json::from_str(&text).unwrap();
        assert_eq!(cases.len(), 2304);
        for case in cases {
            for (got, want) in direction(case.rotation).into_iter().zip(case.direction) {
                assert!(
                    (got - want).abs() < 1e-15,
                    "{:?}: {got} vs {want}",
                    case.rotation
                );
            }
        }
    }
    #[derive(Deserialize, Debug)]
    struct Cell {
        position: [i32; 3],
        state: i32,
    }
    #[derive(Deserialize, Debug)]
    struct Hit {
        position: [i32; 3],
        point: [f64; 3],
        face: String,
    }
    #[derive(Deserialize, Debug)]
    struct Case {
        cells: Vec<Cell>,
        start: [f64; 3],
        end: [f64; 3],
        hit: Option<Hit>,
    }
    #[test]
    fn matches_independent_native_outline_raycast_cases() {
        let mut text = String::new();
        flate2::read::GzDecoder::new(
            &include_bytes!("../../../../../data/java_1_21_11/outline_raycast_cases.json.gz")[..],
        )
        .read_to_string(&mut text)
        .unwrap();
        let cases: Vec<Case> = serde_json::from_str(&text).unwrap();
        assert_eq!(cases.len(), 25394);
        for (index, case) in cases.iter().enumerate() {
            let got = cast(case.start, case.end, |p| {
                native_state(
                    case.cells
                        .iter()
                        .find(|c| c.position == p)
                        .map_or(0, |c| c.state),
                )
                .map_err(Into::into)
            })
            .unwrap();
            assert_eq!(got.is_some(), case.hit.is_some(), "case {index}: {case:?}");
            if let (Some(got), Some(want)) = (got, &case.hit) {
                assert_eq!(got.position, want.position, "case {index}: {case:?}");
                assert_eq!(
                    format!("{:?}", got.face.unwrap()).to_lowercase(),
                    want.face,
                    "case {index}: {case:?}"
                );
                let distance = case
                    .start
                    .into_iter()
                    .zip(want.point)
                    .map(|(a, b)| (b - a).powi(2))
                    .sum::<f64>()
                    .sqrt();
                assert!(
                    (got.distance - distance).abs() < 1e-10,
                    "case {index}: {got:?} vs {case:?}"
                );
            }
        }
    }
    #[test]
    fn native_shape_registry_matches_every_state_and_property() {
        let mut hash = 0xcbf29ce484222325u64;
        for id in 0..shapes().state_shapes.len() {
            let state = native_state(id as i32).unwrap();
            let properties = state
                .properties
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(",");
            for b in format!("{id}:{}:{properties}\n", state.name).bytes() {
                hash = (hash ^ u64::from(b)).wrapping_mul(0x100000001b3);
            }
        }
        assert_eq!(hash.to_string(), shapes().registry_fnv64);
        assert!(native_state(shapes().state_shapes.len() as i32).is_err());
    }
    #[test]
    fn unavailable_geometry_before_a_hit_fails_without_collision_fallback() {
        let unsupported = NativeBlockState {
            name: "minecraft:light".into(),
            properties: [
                ("level".into(), "15".into()),
                ("waterlogged".into(), "false".into()),
            ]
            .into(),
        };
        assert!(
            cast([0.5, 0.5, -1.0], [0.5, 0.5, 3.0], |p| {
                if p == [0, 0, 0] {
                    Ok(unsupported.clone())
                } else {
                    Ok(native_state(1)?)
                }
            })
            .is_ok()
        ); // A known nearer stone occludes the unsupported cell.
        let error = cast([0.5, 0.5, -1.0], [0.5, 0.5, 3.0], |p| {
            if p == [0, 0, 0] {
                Ok(unsupported.clone())
            } else {
                Ok(native_state(0)?)
            }
        })
        .unwrap_err();
        assert!(error.to_string().contains("outline geometry unsupported"));
        assert!(
            cast([0.5, 0.5, 0.5], [0.5, 0.5, 3.0], |_| anyhow::bail!(
                "unloaded"
            ))
            .is_err()
        );
    }
}
