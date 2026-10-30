# Operational crafting with packet-backed external pickups

The operational owner implements pamo's 2026-10-29 decision: Voxrig determines
whether the craft happened and reports concurrent inventory changes separately.
`CraftingTakeRecord`, its `is_resolved()` method, and the partial inventory
executor comparison retain their strict meanings. In particular, a selected-hand
pickup can leave strict `RequiresInspection` permanently while the independent
operational outcome proves `Crafted` with typed `CraftExternal` evidence.

## Public API and continuation

1. Capture `Client::received_crafting()` and submit exactly once using
   `Survival::send_operational_crafting_result(&grid)` (also on `Creative`).
   This is one result PICKUP to the cursor; the helper claims the independent
   operational owner of the returned `InventorySendId`.
2. Poll `Client::operational_crafting_outcome(send.id)`. `Pending` waits for the
   original response and complete full-content/cursor boundary. `Crafted`
   certifies the native single-result consumption/output independently of the
   inventory send's `Diverged` and the strict take's inspection.
3. Always deliver the entire report to the upper layer. `external` contains
   exact entity lifetimes, received item/data/count, collector, collected amount
   and collect sequence. `external_delta` contains the matching received stock
   and slot changes. Empty external evidence means an ordinary craft. A fallback
   `InventoryCause::External` is never promoted into pickup evidence.
4. Retain the report and any strict diagnostic before calling
   `Client::continue_operational_crafting(send.id)`. This atomically verifies the
   same live send/session/opening, unchanged proven inventory, mode, entity and
   selection, and releases only the operational owner. The returned `basis` is
   the full received inventory for the next plan; its cursor still holds the
   result. A subsequent acceptance click can put that cursor in an available
   slot. It is a new inventory intention, never a result-click replay.

Claiming an outcome through the polling method also supports migration of an
existing `send_crafting_result` call, provided its original send is still retained.
Claiming blocks subsequent executor inventory sends until successful explicit
continuation or the separate inspected recovery below. Existing unclaimed strict/acceptance APIs retain their admission
contracts. Strict admission continues to see strict inspection after operational
continuation; use the acceptance inventory path for subsequent operational work.
New sends supersede latest-send operational evidence normally, so retain the
report first. No API here clears old strict conflicts to obtain permission.

Inspection cannot be continued with this API. Resync, disconnect, world/opening
retirement, uncertain delivery, missing metadata or overflow cannot retroactively
supply the original proof. Stop this inventory operation and surface inspection;
neither a later matching inventory nor a no-effect resync grants replay authority.

## Inspected recovery for a new plan

`Client::recover_inspected_operational_crafting(original_id, &basis)` releases
only a claimed operational owner that is terminal `RequiresInspection`. It sends
no packet. `basis` is the existing immutable `ReceivedPlayerSnapshot` obtained
from `Client::received_player_snapshot()`. The typed result is
`inventory::InspectedCraftingRecoveryResult::{Released { recovery }, Rejected { reason }}`;
connection/reader failures use the ordinary `Result` error channel.

The returned `InspectedCraftingRecovery` preserves the original operational
verdict (including External, first reason and its original `released` value),
strict take, send record and divergence. Its separate `basis`, `basis_sequence`,
`contents_sequence` and `cursor_sequence` record the release authority. Historical
`RequiresInspection` and `Diverged` remain historical failures. Internal admission
is released independently of the old outcome's `released` flag, which continues
to describe successful Crafted continuation. Read the retained recovery with
`Client::inspected_crafting_recovery(original_id)`; one latest successful recovery
is retained per connection, independently of the next inventory send. Keep a copy.

Recovery requires the original connection/world/opening, a full contents packet
newer than the original final send/probe boundary, actual cursor at least as new
as that full packet, complete active-menu coverage, and unchanged receipt identities
since the supplied capture. Received later slot updates may refine that full packet.
Reused numeric menu IDs are different openings. Once a scope is retired, returning
to player screen zero does not revive it. A slot-only patchwork cannot establish
the missing full boundary. Unknown or invalidated constituents remain inspection.

Pending sends, incomplete native response/state groups, timed-out `ResyncRequired`,
native inventory mutation markers and queued own-action writes reject with
`PendingDebt`. No full packet substitutes for an ACK/fence. If needed, request an
explicit no-effect `resync_inventory()`, await its own complete native response,
full contents and cursor under a caller deadline, then capture a new basis. This
is one new probe, never a replay of the result take. Recovery does not wait for
receipts or start background work. Gate/state access has a five-second total
budget (`RecoveryDeadline`); callers must also bound polling/replanning.

```rust,ignore
let original = client.operational_crafting_outcome(original_id).await?;
publish_failure_and_external(&original); // retain original send/take/divergence too
// If response/state debt remains, use resync_inventory and bounded observation.
let basis = client.received_player_snapshot().await?;
match client.recover_inspected_operational_crafting(original_id, &basis).await? {
    InspectedCraftingRecoveryResult::Released { recovery } => {
        retain(recovery.clone());
        discard_old_suffix();
        replan_remaining_goal(&recovery.basis); // includes actual occupied cursor
        // Dispatch a NEW acceptance send_inventory_click/transfer/crafting_result.
    }
    InspectedCraftingRecoveryResult::Rejected { reason } => {
        surface_inspection(reason); // bounded retry only if new evidence can help
    }
}
```

### Existing gate-release paths audited

| Path | Existing authority and limit |
| --- | --- |
| `continue_operational_crafting` | Releases only Crafted with still-matching proven inventory; inspection is rejected. |
| `resync_inventory` | Establishes a new ordered no-effect received boundary; preserves the claimed operational gate. Recovery retains the superseded original send/divergence. |
| Strict click / transfer / take inspection | Strict admission still consults unresolved strict records; resync does not rewrite them. Existing acceptance `send_inventory_*` paths use current receipts and the executor, so these records need no new strict-release API. Retain strict diagnostics before a new operation supersedes their latest-record cache. |
| Scope retirement / disconnect | Invalidates old authority, without proving success or qualifying a different scope for this recovery API. A fresh connection has independent owners. |

Body, OwnedOperation completion/lifetime, foreign-collect attribution and dig
budgets are unchanged. This API supplies new-plan admission, not old-operation
success, cancellation proof or a write-sealing capability for Golemkit #498.

## Proof and conservative limits

The original send must be completely written, with its own native response
(legacy action reply or modern fence) and fresh full contents. The *latest*
complete grid, result and main/hotbar values must share that full-content receipt;
the received cursor must be at least as new. Session, opening, mode, local entity
and selected slot must still belong to the original take. Old matching receipts
and an output cursor alone are insufficient.

This single-result proof requires every occupied input to decrease by exactly
one compatible item, all empty inputs to remain empty, and the entire original
result to be added to the cursor. A result preview may clear or regenerate the
same displayed result; it is not physical stock. Nonmatching consumption,
remainders, regenerated different outputs and repeated QUICK_MOVE crafts remain
inspection. This is a received native recipe-effect proof, not a recipe-book
display used as a prediction of arbitrary recipe/remainder semantics. Callers
still select and validate the recipe they intend before submission.

Every residual physical change must be a compatible positive stock gain in
player storage/offhand. Each gain must exactly match same-session collect packets
for the original local player and complete item metadata from the corresponding
live `minecraft:item` entity lifetime. Metadata must precede collection; counts
must be positive and no greater than the entity stack. Aggregate item/data and
quantity must equal the residual delta, including an equipped-hand change. Other
players' collects, unexplained gains, mismatched quantities, missing/unsupported
metadata, duplicate collection of a lifetime, or an intermediate storage removal
are inspection. A conflicting received cursor is latched even if later corrected.

The packet reducer writes directly to this bounded owner, independent of callers
polling the inventory ring. It retains at most 64 pickup events, original/current
bounded menu/inventory snapshots, and one outcome. Overflow is an explicit history
failure, never silent eviction followed by inference. Current evidence after
inspection never promotes the failed operational proof. After a completed proof,
any inventory change before explicit continuation invalidates continuation.

## Information ownership

1. **Owners:** server packets own cursor/grid/stock, item entity lifetimes and
   complete metadata, collector and quantity. The strict take owns historical
   conflicts. The inventory executor owns native response debt and partial
   comparison. Its separate operational owner owns the craft outcome, External
   report and continuation reservation; it does not rewrite either older owner.
2. **Authority:** the exact take/send/session/opening, original complete write,
   native response, latest coherent full receipts and exact local-player
   collect/item/data/quantity attribution. No cause hint or missing receipt is
   elevated to authority. Selected-hand changes require the same stock proof.
3. **Retention/release:** one owner per retained crafting send, up to 64 events;
   explicit failure on lost scope, overflow or uncertain delivery. Claiming holds
   the executor right until proven continuation or inspected recovery. Continuation preserves cursor
   ownership and the received basis, leaving strict history unchanged. New sends
   retire only the ordinary latest-send cache; callers retain historical reports.
4. **Re-derivation cost:** bounded inventory/grid and retained events; semantic
   item matching costs O(slots × events + events²), capped at 64 events. No world
   or terrain snapshot is retained. Lost original entity/collect evidence cannot
   be reconstructed from full inventory. Completed/released owners do not add
   ongoing player captures to unrelated packets.

## Golemkit migration

Golemkit main `d6df3f0`,
[`backend_inventory/craft_reconciliation.rs`](https://github.com/sugipamo/Golemkit/blob/d6df3f0/crates/golemkit-voxrig/src/backend_inventory/craft_reconciliation.rs),
currently requires received inventory proof AND `take.is_resolved()`, and fails
immediately on strict inspection. Replace those craft-operation gates with the
new operational outcome's `Crafted` stage for the exact original send/take.
Keep caller recipe-goal checks as appropriate; keep strict and inventory send
records as diagnostics. Always publish External/divergence information upwards,
then explicitly continue and plan cursor placement from the returned basis.
A crafted outcome must not be written back as strict `ObservedTaken`, nor as
inventory-send `Received`. See CHANGELOG for the public names and migration order.

## Verification and LIVE reproduction

The shared packet script crosses normal/unselected/selected storage changes with
no collect/another player's collect/own collect on both adapters. Ordinary craft
succeeds without External. Only own collect with an exactly matching stock gain
succeeds with External; all other combinations require operational inspection.
Strict selected-hand inspection and partial inventory `Diverged` remain unchanged.
Successful continuation performs a new cursor put-down while strict history stays
latched. Additional shared tests cover quantity mismatch, missing metadata, wrong
item, unconsumed grid, duplicate collect, transient stock removal, stock changing
before continuation, pickup-history overflow, a transient conflicting cursor and a collect after proof before continuation.

Re-run official 1.16.1 with:

```sh
python3 scripts/run_crafting_pickup_ownership_live.py --accept-eula \
  --binary <climbing_control_probe> --jars <official-jars> \
  --output <artifacts> --case selected # also unselected and none
```

The proxy holds one original result click after complete client write, summons a
real log through RCON, waits for its original collect, and forwards that click
once unchanged. Neither received frames nor result clicks are fabricated/replayed.
The three mo5 reports/traces in `docs/evidence/mo5-crafting-pickup-*` remain
historical strict characterization; mo6 evidence records the new independent
outcome and real continuation. 1.21.11 is compiled/tested, not claimed LIVE.

Final mo6 evidence (all passed, no trace errors):

- `docs/evidence/mo6-crafting-pickup-selected-1.16.1-20261029.json.gz`
- `docs/evidence/mo6-crafting-pickup-unselected-1.16.1-20261029.json.gz`
- `docs/evidence/mo6-crafting-pickup-none-1.16.1-20261029.json.gz`

Each has adjacent original `.packets.jsonl.gz` evidence. The manifest
`docs/evidence/mo6-crafting-pickup-manifest.json` retains hashes, strict versus
operational outcomes, original collect metadata/counts, and `[0, 10]` clicked
slots: one original result take and one later cursor placement. The selected
case retains strict inspection after placement. Quantity mismatch is tested on
both packet adapters; no 1.21.11 LIVE result is claimed.

The inspected recovery LIVE runner is:

```sh
python3 -B scripts/run_inspected_crafting_recovery_live.py --accept-eula \
  --binary <climbing_control_probe> --jars <official-jars> --output <artifacts>
```

It obtains a real craft with one proven own pickup, then changes actual stock
through RCON while continuation remains claimed. It verifies terminal inspection,
explicit no-effect resync, successful inspected recovery preserving the original
verdict/External, and a new successful cursor put-down. Original clicked slots are
`[0, -1, 10]`; result take is sent once. Official server SHA-1, binary/source/evidence
hashes, the successful report and original packet trace are retained in
`docs/evidence/mo12-inspected-recovery-*`. The deterministic suite covers both
adapters; this LIVE result covers 1.16.1.
