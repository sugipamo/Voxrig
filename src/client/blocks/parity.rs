//! Independent old capture oracle and repeatable state-id workloads.
use super::*;
use crate::versions::java_1_16_1::{collision::ray_boxes, native_state};
use std::collections::HashMap;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}
fn normalized(d: [f64; 3]) -> [f64; 3] {
    let length = d.iter().map(|v| v * v).sum::<f64>().sqrt();
    d.map(|v| v / length)
}
fn ids_world(rng: &mut Rng) -> HashMap<[i32; 3], Option<i32>> {
    let mut ids = HashMap::new();
    for x in 0..24 {
        for y in 0..24 {
            for z in 0..24 {
                let r = rng.next();
                let id = if r < 0.55 {
                    Some(0)
                } else if r < 0.97 {
                    Some((rng.next() * 17104.0) as i32)
                } else {
                    None
                };
                ids.insert([x, y, z], id);
            }
        }
    }
    ids
}
fn parity(rays: usize) {
    let mut rng = Rng(0x1234_5678_9abc_def0);
    let mut hits = 0;
    for _ in 0..4 {
        let ids = ids_world(&mut rng);
        let decoded: HashMap<_, _> = ids
            .iter()
            .map(|(p, id)| (*p, id.map(|i| native_state(i).unwrap())))
            .collect();
        for _ in 0..rays / 4 {
            let origin = [0, 1, 2].map(|_| 4.0 + rng.next() * 16.0);
            let origin = if rng.next() < 0.1 {
                origin.map(f64::floor)
            } else {
                origin
            };
            let d = if rng.next() < 0.1 {
                let mut d = [0.0; 3];
                d[(rng.next() * 3.0) as usize] = if rng.next() < 0.5 { -1.0 } else { 1.0 };
                d
            } else {
                normalized([0, 1, 2].map(|_| rng.next() * 2.0 - 1.0))
            };
            let distance = 0.1 + rng.next() * 35.0;
            let old = oracle::raycast(MinecraftVersion::Java1_16_1, origin, d, distance, |p| {
                decoded.get(&p).cloned()
            });
            let new = raycast_core(
                origin,
                d,
                distance,
                |p| ids.get(&p).copied(),
                |id| ray_boxes(*id),
                native_state,
            );
            assert_same(old, new, origin, d, distance);
            hits += usize::from(matches!(
                raycast_core(
                    origin,
                    d,
                    distance,
                    |p| ids.get(&p).copied(),
                    |id| ray_boxes(*id),
                    native_state
                ),
                Ok(BlockRaycast::Hit { .. })
            ));
        }
    }
    assert!(hits > rays / 10);
    eprintln!(
        "state_id_raycast_parity: rays={rays}, hits={hits}, mismatches=0, seed=0x123456789abcdef0"
    );
}
fn assert_same(
    old: Result<BlockRaycast>,
    new: Result<BlockRaycast>,
    o: [f64; 3],
    d: [f64; 3],
    dist: f64,
) {
    match (old, new) {
        (Ok(a), Ok(b)) => assert_eq!(a, b, "origin={o:?} direction={d:?} distance={dist}"),
        (Err(a), Err(b)) => assert_eq!(a.to_string(), b.to_string()),
        (a, b) => panic!("origin={o:?} direction={d:?} distance={dist}: {a:?} != {b:?}"),
    }
}
#[test]
fn state_id_raycast_parity() {
    parity(4_000);
}
#[test]
#[ignore = "100000-ray qualification; run explicitly in release mode"]
fn state_id_raycast_parity_100k() {
    parity(100_000);
}

#[test]
fn state_id_boundaries_partial_shapes_air_and_fluids() {
    // Every real state of these blocks, including each stair/slab/fence variant.
    let ranges: serde_json::Value =
        serde_json::from_str(include_str!("../../../data/block_state_ranges.json")).unwrap();
    for block in ranges.as_array().unwrap().iter().filter(|b| {
        [
            "stone",
            "oak_stairs",
            "oak_slab",
            "oak_fence",
            "air",
            "water",
            "lava",
        ]
        .contains(&b["name"].as_str().unwrap())
    }) {
        for id in block["minStateId"].as_i64().unwrap()..=block["maxStateId"].as_i64().unwrap() {
            let id = id as i32;
            for (origin, d, distance) in [
                ([-1.0, 0.25, 0.5], [1.0, 0.0, 0.0], 1.0), // exact range end
                ([-1.0, 0.75, 0.5], [1.0, 0.0, 0.0], 0.999999999),
                ([0.0, 0.5, 0.5], [-1.0, 0.0, 0.0], 2.0), // integer boundary
                ([0.25, 0.25, 0.25], [0.0, 1.0, 0.0], 3.0), // inside
                ([0.5, 2.0, 0.5], [0.0, -1.0, 0.0], 2.0),
                ([-1.0, -1.0, -1.0], normalized([1.0; 3]), 4.0), // corner
                ([-1.0, 0.5, 0.5], normalized([1.0, 1e-13, 0.0]), 2.0),
            ] {
                let cell = |p| Some(Some(if p == [0; 3] { id } else { 0 }));
                let old = oracle::raycast(MinecraftVersion::Java1_16_1, origin, d, distance, |p| {
                    cell(p).map(|id| id.map(|id| native_state(id).unwrap()))
                });
                let new =
                    raycast_core(origin, d, distance, cell, |id| ray_boxes(*id), native_state);
                assert_same(old, new, origin, d, distance);
            }
            if ["air", "water", "lava"].contains(&block["name"].as_str().unwrap()) {
                assert!(ray_boxes(id).unwrap().is_empty());
            }
        }
    }
    for id in [-1, 17104, i32::MAX] {
        assert!(ray_boxes(id).is_err());
    }
}

/// Includes the old per-ray bounded capture/decode/HashMap allocation. Both
/// methods use identical immutable received ids and exactly the same rays.
#[test]
#[ignore = "release-only reproducible microbenchmark"]
#[allow(clippy::assertions_on_constants)] // Intentionally reject explicit debug benchmark runs.
fn state_id_raycast_benchmark() {
    use std::{hint::black_box, time::Instant};
    assert!(!cfg!(debug_assertions), "run with --release");
    let mut rng = Rng(0x97c2_48b0_6eaa_235d);
    let ids = ids_world(&mut rng);
    let rays: Vec<_> = (0..2_000)
        .map(|_| {
            (
                [0, 1, 2].map(|_| 6.0 + rng.next() * 12.0),
                normalized([0, 1, 2].map(|_| rng.next() * 2.0 - 1.0)),
                4.0 + rng.next() * 20.0,
            )
        })
        .collect();
    let old_ray = |&(o, d, distance): &([f64; 3], [f64; 3], f64)| {
        let region = raycast_region(o, d, distance);
        let mut captured = HashMap::with_capacity(region.volume().unwrap());
        for x in region.min[0]..=region.max[0] {
            for y in region.min[1]..=region.max[1] {
                for z in region.min[2]..=region.max[2] {
                    let p = [x, y, z];
                    captured.insert(
                        p,
                        ids.get(&p)
                            .copied()
                            .flatten()
                            .map(|i| native_state(i).unwrap()),
                    );
                }
            }
        }
        oracle::raycast(MinecraftVersion::Java1_16_1, o, d, distance, |p| {
            captured.get(&p).cloned()
        })
    };
    let new_ray = |&(o, d, distance): &([f64; 3], [f64; 3], f64)| {
        raycast_core(
            o,
            d,
            distance,
            |p| Some(ids.get(&p).copied().flatten()),
            |id| ray_boxes(*id),
            native_state,
        )
    };
    for ray in &rays {
        assert_eq!(old_ray(ray).unwrap(), new_ray(ray).unwrap());
    }
    let measure = |old: bool| {
        let start = Instant::now();
        for ray in &rays {
            black_box(if old { old_ray(ray) } else { new_ray(ray) }).unwrap();
        }
        start.elapsed().as_secs_f64() * 1e6 / rays.len() as f64
    };
    for round in 0..3 {
        let (old, new) = if round % 2 == 0 {
            (measure(true), measure(false))
        } else {
            let new = measure(false);
            (measure(true), new)
        };
        eprintln!(
            "state_id_raycast_benchmark: round={round} rays={} old={old:.3} us/ray new={new:.3} us/ray speedup={:.1}x",
            rays.len(),
            old / new
        );
    }
}

#[test]
fn state_id_raycast_only_validates_cells_on_the_ray() {
    // The old eager capture failed even when the bad received id was off-ray.
    // The X-first corner crossing visits [1,0,0], then [1,1,0].
    let origin = [0.5; 3];
    let d = normalized([1.0, 1.0, 0.0]);
    let region = raycast_region(origin, d, 3.0);
    let cell = |p| {
        Some(Some(if p == [0, 1, 0] {
            i32::MAX
        } else if p == [1, 1, 0] {
            1
        } else {
            0
        }))
    };
    let mut old_capture = HashMap::new();
    let old: Result<()> = (|| {
        for x in region.min[0]..=region.max[0] {
            for y in region.min[1]..=region.max[1] {
                for z in region.min[2]..=region.max[2] {
                    let p = [x, y, z];
                    old_capture.insert(p, cell(p).unwrap().map(native_state).transpose()?);
                }
            }
        }
        Ok(())
    })();
    assert_eq!(old.unwrap_err().kind(), crate::ErrorKind::Protocol);
    let new = raycast_core(origin, d, 3.0, cell, |id| ray_boxes(*id), native_state).unwrap();
    assert!(matches!(
        new,
        BlockRaycast::Hit {
            position: [1, 1, 0],
            ..
        }
    ));
    // An unknown id that is actually traversed is still an error.
    assert_eq!(
        raycast_core(
            [0.5; 3],
            [0.0, 1.0, 0.0],
            3.0,
            cell,
            |id| ray_boxes(*id),
            native_state
        )
        .unwrap_err()
        .kind(),
        crate::ErrorKind::Protocol
    );
}
