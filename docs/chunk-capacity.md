# Received-view chunk capacity

`ClientLimits::chunk_capacity` and native 1.16.1
`ConnectionOptions::chunk_capacity` replace `max_chunks`. The default is
`ChunkCapacity::Auto { hard_memory_cap: 16_384 }`. **Both fields use columns,
not bytes**: the hard cap bounds retained columns, not process RSS or bytes held
by caller-owned immutable snapshots. Column memory varies with dimension,
sections, metadata and deduplication. Use a smaller explicit cap for your memory
budget; a byte-accounted allocator is a different policy.

Auto reserves `2 * (2 * max(received_distance, 2) + 3)^2` columns. The extra
column around the received view covers light-only columns; the factor two
covers disjoint old/new teleport views. The bootstrap before a received distance
uses radius 2, not sent client settings. View 10 needs 1,058 columns; view 16
needs 2,450; view 32 needs 8,978. These are conservative square bounds, not a
claim that vanilla sends a square or simultaneous teleport batches.

`Fixed(n)` retains an explicit capacity. Auto and Fixed refuse a received view
that needs more columns than their hard budget. `ChunkCapacityError` reports
`required`, `capacity`, `view_distance`; `Error::chunk_capacity()` extracts it
when the original error chain is available, with `ErrorKind::ResourceLimit`.
`Client::chunk_cache().refusal` also retains the typed refusal after a transport
has flattened a terminal diagnostic. It is not a recoverable permission to
accept more chunks.

Capacity pressure evicts distant columns instead of disconnecting. The current
received view square including its light halo is protected. The local body also
protects a 5 by 5 column square, covering local collision/motion queries even
while a teleport's received center and body are far apart. Local submitted or
predicted position can add this protection; it cannot remove the protection of
the received view. If all required data cannot fit, the handler errors before
accepting the incoming column or removing protected data. Fixed must meet the
same two-view minimum; setting Fixed(512) intentionally fails for view 10.

The complete column is retired: terrain, light, chunk context/block entities,
legacy resource indices and modern reconstruction references. Existing
mining/placement/Body unload invalidations run in the same receive commit.
Removed columns return Unknown (`chunk`/`chunk_context` return None, block
queries have no state). A subsequent legacy light-only packet cannot turn
Unknown terrain into Air. A new original terrain packet may restore knowledge.

`Client::chunk_cache()` is a passive coherent observation of availability:
`session`, `epoch`, total `evicted_columns`, `last_eviction` (column, received
view, effective cap), current columns/cap/view and a typed capacity refusal.
Compare **session and epoch** before reusing any world evidence. Epoch advances
on eviction, unload and cache clear; session fences connection/world retirement.
To build external evidence, read cache stamp, read chunks/context, then read
cache stamp again and accept only matching stamps. Validate the stamp again
before reuse. Old immutable snapshots remain historical and may keep their
Rust allocations alive. This API never certifies them current.

Normal admission uses O(1) membership and count checks. On pressure, candidate
selection scans retained columns and sorts by distance to the closer of the
received view/body; ties sort by coordinates. It costs O(n log n), temporary
O(n), only under pressure. Restoring a retired column costs receipt of original
server terrain/light; no local terrain or Air is invented and no old mutation
is replayed.

## Migration

```rust,no_run
use voxrig::client::prelude::*;
let mut config = ConnectionConfig::offline(
    Server::default(), "AgentOne", MinecraftVersion::Java1_16_1);
config.limits.chunk_capacity = ChunkCapacity::Auto { hard_memory_cap: 16_384 };
// Or an explicit budget supporting your received view (view 16 needs 2,450):
config.limits.chunk_capacity = ChunkCapacity::Fixed(2_450);
```

Replace `config.limits.max_chunks = n` and native `ConnectionOptions.max_chunks`
with `.chunk_capacity = ChunkCapacity::Fixed(n)` after checking the view demand,
or opt into Auto. `usize.into()` converts to Fixed. Native `ChunkSnapshot` adds
`terrain_received`; struct literals must include it. A light-only column has
received lighting but no terrain; native `block` returns None for absent
terrain sections and the common `chunk` observation returns None.

## Ownership audit

1. Server owns view distance/center, terrain, light and context receipts. World
   owns cached availability. The old options/config separately fixed the budget
   at 512; it diverged from the received view, especially with legacy light-only
   columns. Modern terrain/light/context maps must be retired together.
2. Actual received view controls demand. Sent preferences, movement predictions
   and absence of a packet do not establish terrain. Local body protection is
   an additional retention constraint only.
3. At pressure remove all column owners under the receive lock, advance epoch,
   notify existing unload consumers, and retain loss metrics. At world retirement
   session changes fence all previous availability evidence. Re-receipt is the
   only way to restore evicted terrain.
4. Normal budget checks O(1); pressure O(n log n)/O(n) temporary storage. External
   evidence must be re-observed after epoch changes. Rebuilding terrain requires
   a server chunk/light response and may be expensive; there is no automatic
   request, prediction, retry or fabricated Air.

LIVE uses the official 1.16.1 server with view 16, continuous movement and distant
teleports. The proxy withholds unload packets to make pressure deterministic;
all other original frames are unchanged. This missing-unload fixture verifies
more than 512 retained columns, capacity eviction without disconnect, Unknown
terrain/context, changed epoch, protected body column and view/cap metrics. It
does not claim that vanilla always exceeds the two-view budget without that
fixture. See `scripts/run_chunk_capacity_live.py` and the committed evidence.

Committed LIVE artifacts:
`docs/evidence/mo3-chunk-capacity-live-1.16.1-20261028.json.gz` and
`docs/evidence/mo3-chunk-capacity-live-1.16.1-20261028.packets.jsonl.gz`.
At view 16/cap 2,450, the run retired 1,279 columns across the teleports and
withheld 3,710 original unload frames. The old column became Unknown while the
current body column stayed known; the connection accepted a final Look.

The source binding for `docs/common-entity-owners.md` in the existing contact
spatial audit is refreshed for its capacity-default documentation change only.
Contact policy, original client/method fingerprints and numerical evidence are
unchanged.
