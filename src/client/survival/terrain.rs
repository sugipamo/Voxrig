//! Original registered dry slab/stair/rail states, shared by both native adapters.
use crate::{MinecraftVersion, NativeBlockState};
use serde::Deserialize;
use std::{collections::BTreeMap, io::Read};

#[derive(Deserialize)]
pub(crate) struct Shape {
    pub(crate) state: NativeBlockState,
    #[cfg(test)]
    pub(crate) native_id: i32,
    pub(crate) collision: Vec<[f64; 6]>,
    outline: Vec<[f64; 6]>,
    auxiliary: Vec<[f64; 6]>,
}
#[derive(Deserialize)]
struct Terrain {
    states: Vec<Shape>,
}
mod rails;
type Shapes = BTreeMap<String, BTreeMap<BTreeMap<String, String>, Shape>>;
fn shapes(version: MinecraftVersion) -> &'static Shapes {
    static SHAPES: crate::versions::table::PerVersion<Shapes> =
        crate::versions::table::PerVersion::new();
    SHAPES.get(version, |table| {
        let mut decoded = Vec::new();
        flate2::read::GzDecoder::new(table.data.dry_terrain)
            .read_to_end(&mut decoded)
            .expect("packaged native terrain gzip");
        let terrain: Terrain = serde_json::from_slice(&decoded).expect("validated native terrain");
        let mut result: Shapes = BTreeMap::new();
        for shape in terrain.states {
            let previous = result
                .entry(shape.state.name.clone())
                .or_default()
                .insert(shape.state.properties.clone(), shape);
            assert!(previous.is_none(), "unique original terrain state");
        }
        result
    })
}
/// Exact selected-version name and all received properties; missing/waterlogged
/// or unknown states never fall back to a default cube or a neighboring state.
pub(crate) fn lookup(
    version: MinecraftVersion,
    state: &NativeBlockState,
) -> Option<&'static Shape> {
    shapes(version)
        .get(&state.name)
        .and_then(|states| states.get(&state.properties))
        .or_else(|| rails::lookup(version, state))
}
type Outlines = (&'static [[f64; 6]], &'static [[f64; 6]]);
pub(crate) fn outlines(version: MinecraftVersion, state: &NativeBlockState) -> Option<Outlines> {
    lookup(version, state).map(|shape| (shape.outline.as_slice(), shape.auxiliary.as_slice()))
}

#[cfg(test)]
mod tests;
