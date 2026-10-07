//! Collision resolution against block shapes, and the iteration order of
//! attribute modifiers (it decides the last bit of a product of multipliers).
use super::{Modifier, ModifierOperation};
use crate::MinecraftVersion;
use crate::versions::table::StepSearch;

/// Block collision boxes, grouped by the native shape they came from: the
/// native epsilon handling happens once per shape, not once per box.
#[derive(Default)]
pub(crate) struct Geometry {
    boxes: Vec<[f64; 6]>,
    shapes: Vec<std::ops::Range<usize>>,
}

impl Geometry {
    pub(crate) fn push(&mut self, shape: Vec<[f64; 6]>) {
        if !shape.is_empty() {
            let start = self.boxes.len();
            self.boxes.extend(shape);
            self.shapes.push(start..self.boxes.len());
        }
    }
    pub(crate) fn iter(&self) -> impl Iterator<Item = &[f64; 6]> {
        self.boxes.iter()
    }
    fn groups(&self) -> impl Iterator<Item = &[[f64; 6]]> + Clone {
        self.shapes.iter().map(|range| &self.boxes[range.clone()])
    }
}

/// Entity.collide: shape collision with the version's step-up search.
pub(crate) fn collide(
    version: MinecraftVersion,
    bounds: [f64; 6],
    motion: [f64; 3],
    geometry: &Geometry,
    on_ground: bool,
    step_height: f32,
) -> [f64; 3] {
    if motion == [0.0; 3] {
        return motion;
    }
    match version.table().physics_rules.step {
        StepSearch::TwoCandidate => legacy_step(bounds, motion, geometry, on_ground, step_height),
        StepSearch::HeightScan => modern_step(bounds, motion, geometry, on_ground, step_height),
    }
}

/// Axis-ordered collision (Shapes.collide for each axis).
pub(crate) fn collide_shapes(bounds: [f64; 6], motion: [f64; 3], geometry: &Geometry) -> [f64; 3] {
    collide_axes(bounds, motion, geometry, false)
}

/// `border_first`: legacy collision lists the world border shape before the
/// blocks, so a motion below the epsilon is cut even with no block nearby.
fn collide_axes(
    bounds: [f64; 6],
    motion: [f64; 3],
    geometry: &Geometry,
    border_first: bool,
) -> [f64; 3] {
    let mut bounds = bounds;
    let mut result = [0.0; 3];
    let order = if motion[0].abs() < motion[2].abs() {
        [1, 2, 0]
    } else {
        [1, 0, 2]
    };
    for axis in order {
        let mut distance = motion[axis];
        if distance == 0.0 {
            continue;
        }
        if border_first && distance.abs() < 1e-7 {
            distance = 0.0;
        }
        for shape in geometry.groups() {
            if distance.abs() < 1e-7 {
                distance = 0.0;
                break;
            }
            let positive = distance > 0.0;
            for cube in shape {
                if (0..3).any(|i| {
                    i != axis
                        && (bounds[i] + 1e-7 >= cube[i + 3] || bounds[i + 3] - 1e-7 <= cube[i])
                }) {
                    continue;
                }
                let ahead = cube[axis] - bounds[axis + 3];
                let behind = cube[axis + 3] - bounds[axis];
                if positive && ahead >= -1e-7 {
                    distance = distance.min(ahead);
                } else if !positive && behind <= 1e-7 {
                    distance = distance.max(behind);
                }
            }
        }
        result[axis] = distance;
        bounds[axis] += distance;
        bounds[axis + 3] += distance;
    }
    result
}

fn legacy_shapes(bounds: [f64; 6], motion: [f64; 3], geometry: &Geometry) -> [f64; 3] {
    collide_axes(bounds, motion, geometry, true)
}

fn shifted(bounds: [f64; 6], delta: [f64; 3]) -> [f64; 6] {
    std::array::from_fn(|i| bounds[i] + delta[i % 3])
}

fn horizontal(delta: [f64; 3]) -> f64 {
    delta[0] * delta[0] + delta[2] * delta[2]
}

// Older two-candidate step search.
fn legacy_step(
    bounds: [f64; 6],
    motion: [f64; 3],
    geometry: &Geometry,
    on_ground: bool,
    step: f32,
) -> [f64; 3] {
    let adjusted = legacy_shapes(bounds, motion, geometry);
    if step <= 0.0
        || !(on_ground || (motion[1] < 0.0 && motion[1] != adjusted[1]))
        || (motion[0] == adjusted[0] && motion[2] == adjusted[2])
    {
        return adjusted;
    }
    let height = f64::from(step);
    let mut candidate = legacy_shapes(bounds, [motion[0], height, motion[2]], geometry);
    let mut expanded = bounds;
    expanded[0] += motion[0].min(0.0);
    expanded[3] += motion[0].max(0.0);
    expanded[2] += motion[2].min(0.0);
    expanded[5] += motion[2].max(0.0);
    let up = legacy_shapes(expanded, [0.0, height, 0.0], geometry);
    if up[1] < height {
        let alternate = legacy_shapes(shifted(bounds, up), [motion[0], 0.0, motion[2]], geometry);
        let alternate = std::array::from_fn(|i| alternate[i] + up[i]);
        if horizontal(alternate) > horizontal(candidate) {
            candidate = alternate;
        }
    }
    if horizontal(candidate) > horizontal(adjusted) {
        let down = legacy_shapes(
            shifted(bounds, candidate),
            [0.0, motion[1] - candidate[1], 0.0],
            geometry,
        );
        std::array::from_fn(|i| candidate[i] + down[i])
    } else {
        adjusted
    }
}

// Newer scan over candidate box heights.
fn modern_step(
    bounds: [f64; 6],
    motion: [f64; 3],
    geometry: &Geometry,
    on_ground: bool,
    step: f32,
) -> [f64; 3] {
    let adjusted = collide_shapes(bounds, motion, geometry);
    let downward = motion[1] < 0.0 && motion[1] != adjusted[1];
    if step <= 0.0
        || !(downward || on_ground)
        || (motion[0] == adjusted[0] && motion[2] == adjusted[2])
    {
        return adjusted;
    }
    let mut base = bounds;
    if downward {
        base[1] += adjusted[1];
        base[4] += adjusted[1];
    }
    let mut scan = base;
    scan[0] += motion[0].min(0.0);
    scan[3] += motion[0].max(0.0);
    scan[2] += motion[2].min(0.0);
    scan[5] += motion[2].max(0.0);
    scan[4] += f64::from(step);
    if !downward {
        scan[1] -= f64::from(1e-5f32);
    }
    let mut heights: Vec<f32> = geometry
        .iter()
        .filter(|b| (0..3).all(|i| scan[i] < b[i + 3] && scan[i + 3] > b[i]))
        .flat_map(|b| [b[1], b[4]])
        .map(|y| (y - base[1]) as f32)
        .filter(|h| *h >= 0.0 && *h <= step && *h != adjusted[1] as f32)
        .collect();
    heights.sort_by(f32::total_cmp);
    heights.dedup();
    for height in heights {
        let mut candidate =
            collide_shapes(base, [motion[0], f64::from(height), motion[2]], geometry);
        if horizontal(candidate) > horizontal(adjusted) {
            candidate[1] -= bounds[1] - base[1];
            return candidate;
        }
    }
    adjusted
}

fn java_string_hash(s: &str) -> i32 {
    s.encode_utf16().fold(0i32, |h, c| {
        h.wrapping_mul(31)
            .wrapping_add(i32::from(c as i16 as u16 as i16) & 0xffff)
    })
}

/// Iteration order of the modifiers of one attribute, as the version's map
/// yields them: legacy `HashSet` of UUID-keyed modifiers, modern fastutil
/// `Object2ObjectOpenHashMap` keyed by identifier. Inputs are in insertion order.
pub(crate) fn attribute_order<'a>(
    version: MinecraftVersion,
    modifiers: &[&'a Modifier],
) -> Vec<&'a Modifier> {
    let mut by_operation: Vec<&'a Modifier> = Vec::new();
    for operation in [
        ModifierOperation::Addition,
        ModifierOperation::MultiplyBase,
        ModifierOperation::MultiplyTotal,
    ] {
        let group: Vec<&'a Modifier> = modifiers
            .iter()
            .copied()
            .filter(|m| m.operation == operation)
            .collect();
        if group.len() < 2 {
            by_operation.extend(group);
            continue;
        }
        if version.table().physics_rules.generation
            == crate::versions::table::PhysicsGeneration::Legacy
        {
            // java.util.HashMap with 16 buckets: ascending bucket, then insertion.
            let mut keyed: Vec<(u32, usize, &'a Modifier)> = group
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let h = uuid_hash(&m.id) as u32;
                    ((h ^ (h >> 16)) & 15, i, *m)
                })
                .collect();
            keyed.sort_by_key(|(bucket, i, _)| (*bucket, *i));
            by_operation.extend(keyed.into_iter().map(|(_, _, m)| m));
        } else {
            // fastutil open addressing (32 slots, linear probing), iterated downwards.
            let mut slots: [Option<&'a Modifier>; 32] = [None; 32];
            for m in &group {
                let (namespace, path) = m.id.split_once(':').unwrap_or(("minecraft", &m.id));
                let hash = java_string_hash(namespace)
                    .wrapping_mul(31)
                    .wrapping_add(java_string_hash(path));
                let h = hash.wrapping_mul(0x9E3779B9u32 as i32);
                let mut pos = ((h ^ ((h as u32) >> 16) as i32) & 31) as usize;
                while slots[pos].is_some() {
                    pos = (pos + 1) & 31;
                }
                slots[pos] = Some(*m);
            }
            by_operation.extend(slots.iter().rev().flatten().copied());
        }
    }
    by_operation
}

fn uuid_hash(text: &str) -> i32 {
    let hex: String = text.chars().filter(|c| *c != '-').collect();
    let value = u128::from_str_radix(&hex, 16).unwrap_or(0);
    let (most, least) = ((value >> 64) as u64, value as u64);
    let hilo = most ^ least;
    ((hilo >> 32) as i32) ^ (hilo as i32)
}
