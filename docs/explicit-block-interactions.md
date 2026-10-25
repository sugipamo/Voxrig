# Explicit placement and block activation

Use `body::Action::PlaceBlock` or `Body::place_cube(pos, face, cursor, hand)` to
place while moving or airborne, on a face different from the first view-ray hit,
or with an item outside the strict default passive-cube material catalogue.
`place_cube` only derives the adjacent result cell and its expected block name
from the selected hand. For replaceable blocks, slabs or other results, submit
`PlaceBlock` with the explicit result `target` and `expected` state instead.

The existing common `Survival::use_on_block` and Creative counterpart also
send explicit hits with server distance checks, returning dispatch receipts.
The native modern `Operations::use_on_block` now permits explicit Survival hits.
Use body when bounded acceptance is wanted.

Use `body::Action::ActivateBlock { pos, face, cursor, hand }` for crafting tables,
furnaces, chests and other block interactions. Both native versions send the
specified hit unchanged, without a ray, grounded pose, empty-hand or default-item
proof. The held item and sneaking still affect the server's behavior. The caller
owns any existing PlayerControl session and must release it separately. A body
wrapper still owns at most one body action at a time.

The existing `Survival::place_cube`, typed slab/stair/replacement operations and
`Survival::open_container` / Creative counterpart retain their strict receipt
contracts. Their records require a proven initial outline and material/menu
basis; changing that basis would weaken what existing callers can infer. Opt
into body for explicit sending. `HeadlessBody::cube_placement_diagnostics` returns
optional warning strings from the default-stack and standing/first-outline
model. It never sends or vetoes anything. `placement_check` remains a separate
read-only default-box/client-pick-range diagnostic. These diagnostic snapshots
are not dispatch authority.

## Acceptance and identity

Explicit pos/face/cursor/hand, protocol coordinate bounds, finite 0..1 cursor,
connection/world-generation fences and at-most-once writes remain. Existing
unresolved-operation gates are unchanged (separating those is step 6).

Placement still monitors the explicit result cell: a contradictory server update
is `Corrected` with the received rollback, not a local geometric refusal.
Modern placement/activation requires a covering interaction ACK and a bounded
quiet window. Legacy placement requires a fresh matching result-cell update;
legacy activation has no protocol ACK and uses complete send plus the bounded
quiet window. A normal menu contents packet is expected after activation and
is not an inventory rollback. Teleports still produce `Corrected`; world loss or
evidence gaps produce `Unknown`. Every action has an absolute deadline.

`Accepted` for activation does not prove a menu opened or identify which block
caused it. Read `Client::screen_state` for the actual opening and its ScreenId.
A silent server denial without a correction cannot be labelled `Corrected`:
acceptance is only the bounded evidence policy, never proof of effect. The
strict container record remains available when content receipts are needed.

## Official server limits (K1)

Audited from the unmodified Mojang server JARs, using the Mojang-mapped classes.
Machine-readable hashes and bytecode excerpts are in
[evidence/explicit-block-interactions-20261025.json](evidence/explicit-block-interactions-20261025.json).

* [1.16.1 server JAR](https://piston-data.mojang.com/v1/objects/a412fd69db1f81db3f511c1463fd304675244077/server.jar):
  `ServerGamePacketListenerImpl.handleUseItemOn`, offsets 76–125, checks feet to
  clicked block center distance squared **<64**. It does not use the client ray
  or a fixed 4.5 eye-to-hit range.
* [1.21.11 server JAR](https://piston-data.mojang.com/v1/objects/64bb6d763bed0a9f1d632ec347938594144943ed/server.jar):
  `ServerGamePacketListenerImpl.handleUseItemOn`, offsets 84–97, calls
  `Player.isWithinBlockInteractionRange(pos, 1.0)`. That method (source mapping
  lines 2179–2180, bytecode offsets 0–37) checks eye-to-block-AABB squared distance
  **<(blockInteractionRange()+1)^2**. It uses the actual range attribute and eye,
  not a standing constant. The range attribute is clamped to 0..64 by
  `Attributes.BLOCK_INTERACTION_RANGE` / `RangedAttribute`. The handler also validates cursor distance from block
  center, build height, loading/teleport state, features and world permission.
  ACK is registered before those checks and does not prove an effect.

The body adapters reject known violations of these distance checks at the write
boundary. If modern posture/scale or range attributes are missing, they delegate
the exact check to the server rather than inventing standing defaults or
refusing for missing model evidence. Height/protection, collision, item-specific
placement rules and all other authoritative checks remain on the server.

This change is refusal-reduction step 3; it adds no entity contact proofs and
makes no changes to steps 4–7.

## Local validation

[The live summary](evidence/place-activation-live-20261025.json) and compressed
original reports record both SHA-1 verified official versions, built with
`--features native`: five-block bridging, two walking/jumping pillar placements,
off-axis crafting table/furnace/chest openings, optional warnings, exact hit
bytes, unchanged moving owner, beyond-client-pick acceptance, block rollback and
the actual server distance limit. RCON independently confirms positions,
blocks and screens. No GitHub Actions results are used.

```sh
cargo build --locked --features native -j2 --example body_probe
python3 scripts/run_place_activation_live.py --accept-eula \
  --binary target/debug/examples/body_probe --jars /path/to/official-jars \
  --version 1.16.1
# Repeat with --version 1.21.11; jars are named VERSION-server.jar.
```
