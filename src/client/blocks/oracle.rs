//! Frozen pre-optimization decoded traversal; test/benchmark oracle only.
use super::*;
fn enter(
    origin: [f64; 3],
    direction: [f64; 3],
    b: [f64; 6],
    limit: f64,
) -> Option<(f64, usize, bool)> {
    let (mut near, mut far) = (f64::NEG_INFINITY, limit);
    let mut entry = (0usize, false);
    for axis in 0..3 {
        if direction[axis] == 0.0 {
            if origin[axis] < b[axis] || origin[axis] > b[axis + 3] {
                return None;
            }
            continue;
        }
        let t1 = (b[axis] - origin[axis]) / direction[axis];
        let t2 = (b[axis + 3] - origin[axis]) / direction[axis];
        let (lo, hi) = if t1 < t2 { (t1, t2) } else { (t2, t1) };
        if lo > near {
            near = lo;
            entry = (axis, direction[axis] > 0.0);
        }
        far = far.min(hi);
        if near > far {
            return None;
        }
    }
    // An origin inside the box is not an entry.
    (near >= 0.0).then_some((near, entry.0, entry.1))
}

fn face(axis: usize, positive: bool) -> BlockFace {
    // A ray travelling +axis enters through the box's negative face.
    match (axis, positive) {
        (0, true) => BlockFace::West,
        (0, false) => BlockFace::East,
        (1, true) => BlockFace::Down,
        (1, false) => BlockFace::Up,
        (2, true) => BlockFace::North,
        _ => BlockFace::South,
    }
}

pub(crate) fn raycast(
    version: MinecraftVersion,
    origin: [f64; 3],
    direction: [f64; 3],
    distance: f64,
    cell: impl Fn([i32; 3]) -> Option<Option<NativeBlockState>>,
) -> Result<BlockRaycast> {
    let mut position = origin.map(|v| v.floor() as i32);
    let step = direction.map(|d| {
        if d > 0.0 {
            1
        } else if d < 0.0 {
            -1
        } else {
            0
        }
    });
    let mut next = [0.0f64; 3];
    let mut delta = [f64::INFINITY; 3];
    for axis in 0..3 {
        if direction[axis] != 0.0 {
            delta[axis] = 1.0 / direction[axis].abs();
            let boundary = f64::from(position[axis]) + if step[axis] > 0 { 1.0 } else { 0.0 };
            next[axis] = (boundary - origin[axis]) / direction[axis];
        } else {
            next[axis] = f64::INFINITY;
        }
    }
    loop {
        match cell(position).ok_or_else(|| invalid_input("raycast left the captured region"))? {
            None => return Ok(BlockRaycast::Unloaded { position }),
            Some(state) => {
                let mut best: Option<(f64, usize, bool)> = None;
                for b in collision_boxes(version, &state)? {
                    let world = [
                        b[0] + f64::from(position[0]),
                        b[1] + f64::from(position[1]),
                        b[2] + f64::from(position[2]),
                        b[3] + f64::from(position[0]),
                        b[4] + f64::from(position[1]),
                        b[5] + f64::from(position[2]),
                    ];
                    if let Some(hit) = enter(origin, direction, world, distance) {
                        if best.is_none_or(|(t, ..)| hit.0 < t) {
                            best = Some(hit);
                        }
                    }
                }
                if let Some((t, axis, positive)) = best {
                    return Ok(BlockRaycast::Hit {
                        position,
                        state,
                        face: face(axis, positive),
                        point: [0, 1, 2].map(|a| origin[a] + direction[a] * t),
                        distance: t,
                    });
                }
            }
        }
        let axis = (0..3)
            .min_by(|&a, &b| next[a].total_cmp(&next[b]))
            .expect("three axes");
        if next[axis] > distance {
            return Ok(BlockRaycast::Miss);
        }
        position[axis] += step[axis];
        next[axis] += delta[axis];
    }
}
