# State-id collision raycasts

`Client::raycast_blocks` reads received state IDs along the ray under one
observation lock on Java 1.16.1 and 1.21.11. The Amanatides–Woo engine in
`src/client/blocks.rs` borrows collision boxes and decodes only the selected hit.
There is no runtime fast/slow switch or production parity writer.

## Information ownership

1. Previously the common query captured and decoded every cell in the ray's
   bounding box into `NativeBlockState` Strings/BTreeMaps, built a per-ray
   HashMap, then encoded traversed states again for shape lookup. The native
   1.16.1 `World` query independently walked received IDs through
   `collision::shapes_for`. The supplied prototype added another lazy
   decoded-state-to-box table alongside the existing collision registry; these
   representations and separate walkers could disagree at boundaries.
2. Received world state IDs are authoritative. Each version's collision
   registry owns one immutable `StateBoxes`. In 1.16.1, `shapes_for`, movement,
   support facts and both ray query contracts borrow that same owner. In
   1.21.11, the generated shape-index mapping is retained without duplicating
   its shape lists; common collision rays and reconstructed collision targeting
   borrow it. No world result or decoded cell is cached in `StateBoxes`.
3. Block/chunk updates replace received IDs; unload/respawn discards world
   cells through the existing world lifecycle. Every query reads current IDs
   under the observation lock. The immutable version registry lives for the
   process lifetime and has no world-dependent entries to invalidate. Its
   geometry is initialized once, with no decoded-state cache to go stale.
4. Registry initialization costs O(states + boxes). A ray costs O(traversed
   cells + their boxes), plus one complete-state decode on a hit. The removed
   path cost O(bounding-box volume) decodes and allocations before traversal.
   No SipHash map of decoded cells is built per ray. The underlying world's
   chunk lookup remains unchanged.

## Compatibility and native arithmetic

Public signatures and valid received-world hit/miss/unloaded results remain
unchanged. A queued respawn now runs entirely before or after the observation;
there is no metadata/capture gap that can produce `world changed during
raycast`. The eager capture previously decoded even off-ray cells: malformed
1.16.1 IDs off the traversed path no longer fail the query. IDs actually
traversed still produce a protocol error. A dedicated old-capture/new-walk test
records that intentional error-scope change. The native `World` API retains
its historical empty-shape treatment of unknown IDs. The concurrency tests verify the original generation completes before
an already queued reset.

One traversal implementation serves the common `Entry` and 1.16.1
`LegacyVisibility` contracts. Compatibility rules are explicit:

| Rule | Common collision query | Native 1.16.1 world query |
|---|---|---|
| Origin inside shape | No entry into that shape | Hit at distance zero |
| Parallel threshold | Exact zero | Absolute normalized component <= 1e-12 |
| Equal cell crossings | X, then Y, then Z | Advance simultaneous axes within 1e-12 |
| Shape distance limit | Requested maximum, inclusive | Cell exit/requested maximum + 1e-9 |
| Unloaded cell | Return `Unloaded` | Ordinary query skips; visibility query rejects |
| Empty collision target | Skip, including air/fluids | Explicit digging target may supply a unit-cell box |

The old numeric differences are deliberately retained as policies inside the
shared engine, not silently declared equivalent to vanilla. Neither former
collision query is an exact vanilla selection ray: the official 1.16.1
`VoxelShape.clip` (`dfg.a(dem,dem,fu)`) uses a squared-length cutoff of 1e-7 and
an inside probe at 0.001 of the *whole unnormalized segment*; `AABB.clip`
(`deg`) also uses 1e-7. Native cell traversal pads segment endpoints and selects
Z/Y/X on ties. These are implemented and independently fixture-tested in
`src/client/survival/target.rs`, `scripts/VerifyLegacyTargeting.java`, and the
1.21.11 outline fixtures. That native model is the appropriate source for
actual outline selection, not either of the historical collision tolerances.
Replacing only a normalized-ray epsilon would not reproduce vanilla and would
change containment, endpoint and corner results. Such a selection/API migration
is outside this performance change; boundary and old-world parity tests pin
both existing contracts here.

The official JAR examined for this decision is SHA-1
`a412fd69db1f81db3f511c1463fd304675244077` (1.16.1), using its published Mojang
server mappings. `javap -c -p dfg` and `javap -c -p deg` confirm these constants.

The 1.21.11 reconstructed collision-target traversal remains separate: it
visits neighboring owner cells for protruding shapes in [-0.25, 1.5], orders
candidates by earliest possible hit, accepts containment with no face, and
rejects moving/contextually unavailable geometry. Replacing it with the
received-cell walk would lose those guarantees. It now shares `StateBoxes`.
The outline walker also remains separate because outline/auxiliary geometry,
native segment padding, inside probes, and moving-state rejection have distinct
semantics. A movement collision table cannot own outline selection facts.

## Reproduce qualification

Use `-j2` for each cargo invocation. Ordinary tests include 4,000 fixed-seed
common-query rays over randomized worlds of real 1.16.1 IDs and 4,000 old-world
comparisons. Shape variants of stone, stairs, slabs, fences, air and both fluids
cover axis-parallel, exact block/range boundaries, corners and inside origins;
the native-world suite also covers empty-collision digging targets.

```sh
cargo test --locked --release --lib -j2 state_id_raycast_parity_100k -- --ignored --nocapture
cargo test --locked --release --lib -j2 state_id_raycast_benchmark -- --ignored --nocapture
```

The benchmark reuses exactly the same immutable world and 2,000 rays for both
methods. Its old path includes per-ray bounded capture, decoding and HashMap
allocation; its new path borrows state-id boxes. Both methods warm up first and
assert result equality before three timed rounds with alternating order. This
isolates geometry work, excluding async locks, player metadata and network I/O.

The independent oracle in `src/client/blocks/oracle.rs` preserves the previous
common traversal; `World::raycast_blocks_oracle` preserves the previous native
traversal. Both compile only under `cfg(test)`.

The upstream minegpu measurements are **not** this microbenchmark: see the
provided `~/logs/contact/fastray/table-final.md` and `findings.md` (2026-10-28
JST), [preserved with source hashes](evidence/state-id-raycast-minegpu-20261028.md), patch base `502ac0f`. Their 32-bot baseline recorded 732–804 us/ray and
72–74% executor CPU in attack rays; the prototype recorded about 6 us/ray and
3.3–3.4%. They reported zero mismatches for 100,000 unit rays and about 1.9M
live calls plus randomized rays. These are externally supplied results, not a
new Golemkit live performance run by this PR.

The contact-envelope evidence's SDK source fingerprint for `src/versions/table.rs`
is refreshed after reviewing this diff: only test-only collision adapters and
raw-slice copying change there; entity dimensions, metadata defaults and contact
policy do not. Original client/mapping hashes and method fingerprints are retained.

Local 1.16.1 LIVE qualification used the existing `scripts/run_migration_api.py`
and a freshly built `migration_api_probe` against the SHA-1-verified official
server. All 21 checks passed with no trace errors, including beyond-32-block
occlusion, partial slab geometry, the 64-block maximum, unloaded boundaries and
respawn world identity. The [LIVE report with source/binary hashes](evidence/state-id-raycast-live-1.16.1-20261028.json.gz)
preserves the original results and fixture commands. Java 1.21.11 is covered by
local compile/tests; this change does not claim a new 1.21.11 LIVE run.

## Local measurements (2026-10-28 UTC)

Intel Core i9-12900H, rustc 1.99.0, optimized release profile. After the other
cargo jobs finished, the same release test binary and 2,000 rays were timed
again to avoid concurrent build/test load:

| Round | Old capture + decoded walk (us/ray) | State-id walk (us/ray) | Ratio |
|---|---:|---:|---:|
| 0 | 186.516 | 0.763 | 244.4x |
| 1 | 187.345 | 0.693 | 270.4x |
| 2 | 186.913 | 0.716 | 261.2x |

Median: **186.913 -> 0.716 us/ray**, approximately **261x** for this synthetic
geometry workload. This excludes async locks/player observation/network I/O.
The explicit release 100,000-ray parity run reported 81,473 hits and zero
mismatches, seed `0x123456789abcdef0`. The ordinary old-world 4,000-ray parity
also reported zero mismatches, seed `0xcafe592791ab4359`.
