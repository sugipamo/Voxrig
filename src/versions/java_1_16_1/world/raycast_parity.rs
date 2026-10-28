//! Compare the consolidated walker with the frozen native World query.
use super::*;

fn rng(seed: &mut u64) -> f64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    (*seed >> 11) as f64 / (1u64 << 53) as f64
}
fn compare(world: &World, o: Vec3, d: Vec3, distance: f64, target: Option<BlockPos>) {
    assert_eq!(
        world.raycast_blocks_oracle(o, d, distance, target),
        world.raycast_blocks_with_target(o, d, distance, target),
        "o={o:?} d={d:?} distance={distance} target={target:?}"
    );
}
#[test]
fn legacy_world_state_id_raycast_parity() {
    let mut seed = 0xcafe_5927_91ab_4359;
    let mut total = 0;
    for _ in 0..4 {
        let mut world = World::default();
        world.chunks.insert((0, 0), Chunk::default());
        world.chunks.insert((2, 0), Chunk::default());
        for x in 0..16 {
            for y in 0..16 {
                for z in 0..16 {
                    let id = if rng(&mut seed) < 0.55 {
                        0
                    } else {
                        (rng(&mut seed) * 17104.0) as i32
                    };
                    world.set_block(x, y, z, id);
                }
            }
        }
        for _ in 0..1000 {
            let a = [0, 1, 2].map(|_| 1.0 + rng(&mut seed) * 14.0);
            let a = if rng(&mut seed) < 0.1 {
                a.map(f64::floor)
            } else {
                a
            };
            let d = if rng(&mut seed) < 0.1 {
                let mut d = [0.0; 3];
                d[(rng(&mut seed) * 3.0) as usize] = if rng(&mut seed) < 0.5 { -1.0 } else { 1.0 };
                d
            } else {
                [0, 1, 2].map(|_| rng(&mut seed) * 2.0 - 1.0)
            };
            let target = if rng(&mut seed) < 0.5 {
                Some(BlockPos {
                    x: (rng(&mut seed) * 40.0) as i32,
                    y: a[1] as i32,
                    z: a[2] as i32,
                })
            } else {
                None
            };
            compare(
                &world,
                Vec3 {
                    x: a[0],
                    y: a[1],
                    z: a[2],
                },
                Vec3 {
                    x: d[0],
                    y: d[1],
                    z: d[2],
                },
                rng(&mut seed) * 40.0,
                target,
            );
            total += 1;
        }
    }
    eprintln!(
        "legacy_world_state_id_raycast_parity: rays={total}, mismatches=0, seed=0xcafe592791ab4359"
    );
}
#[test]
fn legacy_world_raycast_boundaries_and_partial_shapes() {
    let ranges: serde_json::Value =
        serde_json::from_str(include_str!("../../../../data/block_state_ranges.json")).unwrap();
    for block in ranges.as_array().unwrap().iter().filter(|b| {
        [
            "stone",
            "oak_stairs",
            "oak_slab",
            "oak_fence",
            "air",
            "water",
            "lava",
            "wheat",
        ]
        .contains(&b["name"].as_str().unwrap())
    }) {
        for id in block["minStateId"].as_i64().unwrap()..=block["maxStateId"].as_i64().unwrap() {
            let mut world = World::default();
            world.chunks.insert((0, 0), Chunk::default());
            world.set_block(1, 1, 1, id as i32);
            for (o, d, distance) in [
                ([0.0, 1.25, 1.5], [1.0, 0.0, 0.0], 1.0),
                ([0.0, 1.75, 1.5], [1.0, 0.0, 0.0], 0.9999999995),
                ([1.0, 1.5, 1.5], [-1.0, 0.0, 0.0], 2.0),
                ([1.25, 1.25, 1.25], [0.0, 1.0, 0.0], 3.0),
                ([1.5, 3.0, 1.5], [0.0, -1.0, 0.0], 2.0),
                ([0.0, 0.0, 0.0], [1.0; 3], 4.0),
                ([0.0, 1.5, 1.5], [1.0, 1e-13, 0.0], 2.0),
            ] {
                let o = Vec3 {
                    x: o[0],
                    y: o[1],
                    z: o[2],
                };
                let d = Vec3 {
                    x: d[0],
                    y: d[1],
                    z: d[2],
                };
                compare(&world, o, d, distance, None);
                compare(&world, o, d, distance, Some(BlockPos { x: 1, y: 1, z: 1 }));
            }
        }
    }
}

#[test]
fn legacy_world_unknown_state_keeps_the_existing_empty_shape_contract() {
    let mut world = World::default();
    world.chunks.insert((0, 0), Chunk::default());
    world.set_block(3, 1, 1, 1);
    for id in [-1, 17104, i32::MAX] {
        world.set_block(1, 1, 1, id);
        let o = Vec3 {
            x: 0.5,
            y: 1.5,
            z: 1.5,
        };
        let d = Vec3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        };
        compare(&world, o, d, 8.0, None);
        compare(&world, o, d, 8.0, Some(BlockPos { x: 1, y: 1, z: 1 }));
        assert_eq!(world.raycast_blocks(o, d, 8.0).unwrap().position.x, 3);
    }
}
