# Crafting take receipt deadlines

Both Java 1.16.1 and 1.21.11 retain one identity for a submitted result PICKUP or
QUICK_MOVE. `CraftingTakeRecord::is_resolved()` is true only for `ObservedTaken`,
`ObservedTransferred`, and `NotApplied`. Internal unresolved admission uses its
negation. `RequiresInspection` remains unresolved, even when late packets arrive.
Strict Look admission and the helpers' existing return contracts are unchanged;
PR #305 remains the separate strict API design decision.

After the complete original write, an owned task waits
`ClientLimits.inventory_click_receipt_timeout` (default five seconds). Only fresh
complete received grid/result, cursor, required inventory, and the original
legacy transaction reply can resolve the take. At that deadline the task checks
the same session/opening and sends one no-effect full resync: PICKUP slot -1,
with a mismatching comparison/revision. It never clicks the result again. The
original inventory executor identity and comparison remain intact.

The task then waits `ClientLimits.crafting_take_resync_timeout` (default five
seconds). If still Pending it latches `ReceiptDeadlineAfterResync` and
`RequiresInspection`. Resync dispatch itself is also bounded by the second
budget; incomplete/failed dispatch latches `ResyncDeliveryUncertain`. Thus the
healthy-writer missing-receipt path is 5 s + resync write + 5 s; even a stalled
resync write has a finite budget. Native 1.16.1 `ConnectionOptions` exposes the
same fields. The common inventory receipt timeout now configures the existing
modern executor too (its default stays five seconds).

The independent executor may require resync after its first receipt deadline;
that status cannot prematurely expire the crafting record or establish its
outcome. Crafting's retained ledger owner follows its own evidence/inspection,
rather than the executor's first timer. Timer work inherits the original finite
connection owner and survives caller cancellation; inspecting a record does not
renew either wait. A completed record's old timer cannot regress its stage.

## Information ownership

1. **Owner.** The server owns received grid/result, cursor, main/hotbar and legacy
   replies. The take record owns their conjunction and the two-phase deadline for
   one take ID. The inventory executor separately owns response/state progress;
   helper `Ok`, dispatch and executor termination are not take completion.
2. **Authority.** Only fresh received values prove an outcome. A resync request,
   elapsed time, prediction or negative transaction reply alone proves neither
   success nor refusal. Unchanged complete receipts can prove `NotApplied`.
3. **Retention and release.** The first inspection reason stays latched. No expiry
   releases the cursor, authorizes replay, clears inventory debt or relaxes strict
   admission. `player_after_inspection` and `crafting_after_inspection` retain the
   newest actual snapshot in the original session; original/probe legacy replies
   are retained separately. Snapshot retention is bounded, not an unbounded packet
   history. Retiring the session/opening follows existing reconciliation rules.
4. **Re-derivation cost.** Reading the retained record is bounded local work.
   Capturing/checking receipts costs O(grid + 36 inventory slots). A timeout sends
   at most one no-effect click (plus the legacy protocol acknowledgement), with
   one full-content server round trip; no recipe/consumption prediction or
   automatic result retry is performed. A finite monitor checks completion every
   20 ms and does not hold a state lock while waiting.

## Verification

The shared #305 reproduction retains all 64 strict Look refusals. Both real
packet adapters additionally test PICKUP and QUICK_MOVE with short configured
budgets: no boundary ends in inspection, late receipts remain inspectable, a
second-phase actual effect resolves, and unchanged actual receipts resolve as
`NotApplied`. Inspection still rejects strict Look and result replay.

`scripts/run_crafting_take_deadline_live.py` checks vanilla 1.16.1 healthy PICKUP
and QUICK_MOVE against independent RCON inventory, then queues original
transaction/full-content/slot packets in a proxy. All other traffic continues.
The default waits cause exactly one additional slot -1 resync and inspection;
releasing the original packets retains late evidence without changing that
outcome. This is a withheld-receipt fixture, not a claim to reproduce the reported
intermittent crafting-table fault on an unmodified connection.

LIVE evidence (2026-10-28):
[`mo4-crafting-take-deadline-live-1.16.1-20261028.json.gz`](evidence/mo4-crafting-take-deadline-live-1.16.1-20261028.json.gz)
and [original packet trace](evidence/mo4-crafting-take-deadline-live-1.16.1-20261028.packets.jsonl.gz).
The no-reader interval was 11 seconds; its first subsequent read already showed
`ReceiptDeadlineAfterResync`. Both healthy takes completed before any deadline
resync. Exactly two click frames were sent during the withheld case: one result
PICKUP, then one no-effect probe. Late original frames preserved inspection.
