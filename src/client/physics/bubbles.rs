//! Native modern axis traversal, shared by player and boat bubble callbacks.
use super::{Result, floor, intersects, shifted, unsupported};

pub(super) fn visits(
    from: [f64; 3],
    to: [f64; 3],
    movement_order: [usize; 3],
    inside_box: impl Fn([f64; 3]) -> [f64; 6],
) -> Result<Vec<([i32; 3], bool)>> {
    let delta: [f64; 3] = std::array::from_fn(|i| to[i] - from[i]);
    let mut result = Vec::new();
    let mut at = from;
    let mut visited = std::collections::HashSet::new();
    let mut steps = Vec::new();
    for axis in movement_order {
        if delta[axis] != 0.0 {
            let start = at;
            at[axis] += delta[axis];
            steps.push((start, at, axis));
        }
    }
    if steps.is_empty() {
        steps.push((to, to, 1));
    }
    for (start, end, axis) in steps {
        let distance = end[axis] - start[axis];
        let dest = inside_box(end);
        let source = shifted(dest, std::array::from_fn(|i| start[i] - end[i]));
        let stationary = distance * distance < f64::from(1.0e-5f32 * 1.0e-5f32);
        let long = distance.abs() > 1.0 - f64::from(1.0e-5f32);
        let mut boxes = Vec::new();
        if !stationary {
            boxes.push(source);
            // Axis-aligned traversal adds each newly crossed leading face,
            // then the remaining destination cells, just as BlockGetter.
            let face = if distance > 0.0 { axis + 3 } else { axis };
            let a = floor(source[face]);
            let b = floor(dest[face]);
            if (b - a).abs() > 14 {
                return Err(unsupported("bubble intersection step budget".into()));
            }
            let sign = if distance > 0.0 { 1 } else { -1 };
            let mut cell = a;
            while cell != b {
                cell += sign;
                let mut face_box = dest;
                face_box[axis] = f64::from(cell);
                face_box[axis + 3] = f64::from(cell);
                boxes.push(face_box);
            }
        }
        boxes.push(dest);
        let order = if stationary {
            [2, 1, 0] // BlockPos.betweenClosed: X is fastest.
        } else if axis == 2 {
            [1, 2, 0]
        } else {
            [1, 0, 2]
        };
        for b in boxes {
            let axes: [Vec<i32>; 3] = std::array::from_fn(|i| {
                let mut cells: Vec<_> = (floor(b[i])..=floor(b[i + 3])).collect();
                if i == axis && distance < 0.0 && !stationary {
                    cells.reverse();
                }
                cells
            });
            for &a in &axes[order[0]] {
                for &b in &axes[order[1]] {
                    for &c in &axes[order[2]] {
                        let mut cell = [0; 3];
                        cell[order[0]] = a;
                        cell[order[1]] = b;
                        cell[order[2]] = c;
                        if visited.insert(cell) {
                            let cube = std::array::from_fn(|i| {
                                f64::from(cell[i % 3]) + if i < 3 { 0.0 } else { 1.0 }
                            });
                            result.push((cell, long || intersects(dest, cube)));
                        }
                    }
                }
            }
        }
    }
    Ok(result)
}
