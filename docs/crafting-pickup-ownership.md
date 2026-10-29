# Crafting take and concurrent pickup: held ownership decision

This is a characterization and design proposal. It changes neither strict API
meaning nor inventory executor outcomes. The proposed behavior change is held
for a choice of API owner; passing these tests is not a fix for the minegpu Body
termination.

The minegpu Mg6x5 trace at 2026-10-28 18:28:01Z contains
`Collected { entity_ids: [1182] }`, not merely an unexplained inventory gain.
Its baseline is receive sequence 17500. Full contents at 17506 clear result slot
0 (four oak planks), consume the oak log in grid slot 1, and add one oak log to
hotbar slot 36. Cursor at 17507 contains the four displayed oak planks. The
excerpt proves receipt of a collection event for this player in the operation
window. It does not retain the collected entity's item metadata or quantity,
so it cannot by itself attribute exactly one oak log to that entity.

The two retained outcomes have different owners:

- The inventory executor compares a partial `CraftingResult` intention. Grid
  consumption and result regeneration are native semantics, not an exact
  precomputed material delta. Its comparison intentionally finishes `Diverged`
  even for an ordinary successful take. `External` in that diagnostic alone is
  not packet-backed proof of a pickup.
- The strict take requires the original session, mode, selected hand, equipped
  stacks, cursor and complete grid receipts. A change in selected slot 36
  permanently produces `RequiresInspection`, even with collection evidence.
  A pickup into unselected slot 37 permits `ObservedTaken` while the inventory
  comparison still reports `Diverged`.

The official 1.16.1 LIVE fixture delays exactly one original serverbound result
click after the client completely writes it, summons a real oak-log item via
RCON, waits for the original collect packet, and releases the unmodified click
once. No packet is fabricated and no operation is replayed. This creates a
controlled inflight-pickup ordering; it is not proof of the historical packet
ordering. Independent RCON inventory, original collect/full-content/cursor
frames, the strict take, and the comparison are retained. The no-pickup control
also has an inventory `Diverged` comparison and an `ObservedTaken` strict take.

The shared packet test runs on both 1.16.1 and 1.21.11. It crosses normal,
unselected-slot and selected-slot changes with no collect packet, another
player's collect, and this player's collect. Only the last provides `Collected`
evidence. It also verifies that later full contents do not erase strict
inspection. These tests preserve the current strict contract deliberately.

## Held alternatives

[pamo's decision on #305](https://github.com/sugipamo/Voxrig/pull/305#issuecomment-6486926817)
keeps strict API semantics unchanged and migrates Golemkit craft/Look to Body
and checked helpers. #305 was closed, unmerged, on 2026-10-29. Exempting pickups
from the existing strict equipped-stack condition would contradict that
constraint; it is excluded from this proposal.

1. Add a separate operational crafting outcome, leaving strict take unchanged.
   Its owner would require the take's full write and fresh cursor/grid proof,
   plus a verified recipe-consumption/remainder model or explicitly partial
   outcome. External changes would require this player's original collect
   evidence, entity item/quantity, and a matching received stock difference.
   It must never promote arbitrary `External` diagnostics, full resync alone,
   or coincidental cursor equality to success. Golemkit would consume this
   outcome without requiring an incompatible strict completion. This is the
   preferred direction if concurrent pickup must not terminate the Body.
2. Keep current SDK outcomes and change Golemkit's handling of a confirmed
   external interruption to a nonfatal replan, with no automatic replay or
   permission to bypass unresolved strict operations. This needs a separate
   continuation contract; merely relabeling `Diverged` as success is unsafe.

An isolated change that exempts native grid slots in `Comparison::inspect`
cannot implement any of these: those slots are already exempt, and the
comparison explicitly does not certify a full crafting delta. Likewise,
`Collected` alone does not identify which slot or item changed.

## Information ownership

1. **Owners:** server packets own cursor, grid, inventory, item entity metadata,
   collect count and collector identity. Strict take owns its historical proof
   and conflicts. Inventory executor owns response progress and partial intent
   comparison. A new operational outcome would be a separate owner, not an
   overwrite of either existing record.
2. **Authority:** require original same-session collect for the local entity,
   matching item/quantity and newer received output/grid/stock boundaries.
   Another player's collect, absence of a packet, and `External` fallback are
   not sufficient. Preview clearing is UI state, not lost physical stock.
3. **Retention and release:** evidence must be bounded to the original take ID,
   send sequence, session and opening, with history-gap detection. Supersession,
   disconnect or world/opening retirement ends attribution. Inspection remains
   latched. Deadline resync grants neither replay nor cursor-release authority.
4. **Re-derivation cost:** compare the bounded active menu and inventory slots
   and retained collect events, O(slots + events). Missing original entity or
   collect evidence cannot be reconstructed from a later matching inventory;
   a full resync supplies current values but cannot recreate causality.

Run each LIVE case with `scripts/run_crafting_pickup_ownership_live.py
--accept-eula --binary <climbing_control_probe> --jars <official-jars>
--output <artifacts> --case selected|unselected|none`.

## Committed 1.16.1 LIVE evidence

- `docs/evidence/mo5-crafting-pickup-selected-1.16.1-20261029.json.gz`:
  original collect for entity 37, collector 36, count 1; strict take requires
  inspection with `crafting take equipped hand changed`.
- `docs/evidence/mo5-crafting-pickup-unselected-1.16.1-20261029.json.gz`:
  the same real collection into slot 37; strict take is `ObservedTaken`.
- `docs/evidence/mo5-crafting-pickup-control-1.16.1-20261029.json.gz`:
  no collection; strict take is `ObservedTaken`.

Each report has an adjacent `.packets.jsonl.gz` containing the original trace.
All three reports passed, with no trace errors. Every inventory executor
comparison is `Diverged`; only the two real pickups have a `Collected` hint.
Independent RCON inventory verifies the picked-up log. Original full-content
and cursor packets verify grid consumption and the four output planks.
