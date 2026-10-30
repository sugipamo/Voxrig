# Owned primitive operations

`OwnedOperation<T>` separates an invocation's execution owner from its observers.
Dropping the handle, `finished()` future, or `wait_until()` future does not cancel
execution, cleanup, native response debt, or inventory ownership. There is no
operation seal or generic cancellation request. Existing APIs remain available.

`PlayerControl::begin_stop_for` is the controller preamble entry point. It always
selects the given monotonic controller session ID, including after replacement.
`HeadlessBody::begin_send_and_trust` observes the initial dispatch boundary, not
the action's full lifetime. `Survival` and `Creative` provide owned strict
`begin_inventory_click`, `begin_inventory_transfer`, `begin_crafting_take`, and
`begin_operational_crafting_take`, as well as acceptance
`begin_send_inventory_click` and `begin_send_inventory_transfer`. Composite
inventory helpers are not atomic operations and have no new cancellation claim.

Inventory begin methods require an original `SessionStamp`, even for the player
screen. A passive capture checks it before mutation handoff; the coherent native
prepare boundary rechecks it. Container `InventorySource` additionally retains
its original `ScreenId`. Crafting takes retain the exact `ReceivedCrafting`
world/opening/close basis. Body begin requires `ActionRequest.world = Some(...)`
from that backend's observation. Every invocation also carries a connection-bound
`OwnedOperationId`; native record IDs are retained separately in the result.
No observer polls an unqualified latest record and consumes it as its own.

`OperationLimits` fixes one absolute deadline at begin: `run_budget` plus
`cleanup_budget`, with `0 < total <= 1 hour`. The second field is an observation
allowance, not a request to cancel or forcibly abort execution at the first bound.
A small independent watchdog and `finished()`'s own deadline need neither domain
state nor the actor queue. As with all Tokio deadlines, the runtime must be able
to schedule tasks. Dropping/replacing waiters cannot renew this budget.

The immutable terminal completion distinguishes:

| Outcome | Evidence and permitted interpretation |
| --- | --- |
| `NotSent` | The invocation cannot write in the future: begin failed before registration, an owner exited before mutation handoff, an original scope was refused before mutation, a native stop completed without release writes, or the modern original sender was destroyed before starting a write. A returned record/error explains the refusal or no-op. It says nothing about other invocations or past controller ticks. |
| `SentReceived` | Complete original write and this operation's existing terminal receipt predicates. Read the original record: observed unchanged, divergent inventory, strict inspection alongside operational Crafted, and server rejection are not blanket acceptance. |
| `SentUnconfirmed` | The initial local write completed, but server effect is not established here. Stop is local release, Body Move is its first tick, and Body Break is START. Inventory/take inspection retains its original reason and gates. |
| `Unknown(reason)` | Deadline, owner loss, delivery/receipt failure or original-record supersession. Writes can still occur; do not replay or release gates. |

Begin errors guarantee that invocation was not registered and cannot send. Errors
returned after native handoff are conservatively Unknown unless a sender provides
specific proof. `Body::SendStatus::NotSent` does not provide the proof required by
`OperationOutcome::NotSent`: queued writes can outlive Body cancellation/release.

`completion.owner_joined` certifies only that this registered observer/execution
owner was actually joined. It is not a seal of descendants. A deadline completion
has `owner_joined = false`; it remains unchanged forever. `snapshot().joined`
separately retains the actual late owner completion, with the same operation ID
and scope. `snapshot().record` also retains the most recent original record.
The optional record can be absent when the native API has not yet returned its ID.
A maximum of 128 live owned invocations per connection prevents unbounded orphan
owners; expired-but-running owners consume capacity until actual join. They are
never evicted to admit another operation.

Modern ordinary click/transfer futures additionally close an unsent reservation
synchronously on future drop. Their guard runs under the still-held coherent
state lock. The native `write_started` flag is set immediately before attempting
a frame under writer serialization. A false flag after the original future has
been destroyed proves no frame and no queued descendant remain. Both strict and
executor records become `NotSent`, their finite owner settles, and the gate opens.
After starting any write attempt, drop retains inspection, marks delivery unknown,
and closes the uncertain transport. Matching later slot values cannot promote a
`NotSent` record or an inspection to success. Native fence tokens remain monotonic.

`Client::retire_connection(deadline)` revokes the whole connection **before
returning its future**, then observes shutdown up to the supplied absolute
instant. `ConnectionRetirement::Closed` certifies actual local transport teardown
and connection task join. `Unconfirmed` certifies only local revocation before a
shutdown deadline, and `Failed` retains the actual shutdown failure. All three
retire this connection irreversibly; none proves rollback or physical rest.
The existing `revoke_connection` and unbounded `wait_shutdown` contracts remain.

Dig uses the existing `Survival::begin_dig` and `DigOperation` owner. The additive
`finished_until(deadline)` returns `DigWait::Completed` or `DigWait::Unknown`
with the original snapshot, without requesting cancellation. IDs expose their
original `generation()` and `ordinal()`. A joined `DigCompletion` projects to the
same `operation_outcome()` vocabulary, while independent START/STOP/ABORT write
evidence and the original `DigResult` stay authoritative. Request cancellation
explicitly. ABORT Written and Cancelled/Released do not undo delayed destruction.

A Golemkit preamble can retain only the lightweight handle:

```rust,ignore
let stop = client.player_control(mode).begin_stop_for(owned.id, limits)?;
pending.store(golem_id, stop.clone());
let result = stop.finished().await; // bounded; this waiter can always be dropped
match result.outcome {
    OperationOutcome::NotSent | OperationOutcome::SentUnconfirmed
        if result.record.as_ref().is_some_and(|r|
            r.session_id == owned.id && matches!(r.status, ControlStatus::Stopped { .. })) => {
        clear_original_control_owner(owned.id);
    }
    _ => {
        publish_operation_unconfirmed(result);
        quarantine_new_admissions();
        let retired = client.retire_connection(Instant::now() + retire_budget).await;
        retain_retirement_diagnostic(retired); // Unconfirmed is not Closed
        return;
    }
}
let dig = client.survival().begin_dig(target, face)?; // existing API, no duplicate owner
pending.store_dig(golem_id, dig.clone());
match dig.finished_until(run_deadline).await {
    DigWait::Completed(done) => consume_original_dig(done),
    DigWait::Unknown(_) => {
        dig.request_cancel();
        match dig.finished_until(cleanup_deadline).await {
            DigWait::Completed(done) => consume_original_dig(done),
            DigWait::Unknown(snapshot) => {
                publish_operation_unconfirmed(snapshot);
                quarantine_new_admissions();
                retain_retirement_diagnostic(client.retire_connection(retire_deadline).await);
            }
        }
    }
}
```

For inventory, retain the original send/take ID, inspect the typed outcome and
existing strict/executor record, and keep inspection gates. Operational crafting
requires retaining `OwnedCraftingRecord.operational.external` before explicit
`continue_operational_crafting(original_send_id)` with a current matching basis.
For a terminal operational inspection, the explicit
`recover_inspected_operational_crafting(original_send_id, &received_snapshot)`
can release new-plan admission after fresh same-scope full/cursor receipts and
zero pending debt. Its separate receipt preserves the original owned completion
and strict/operational verdicts; see [recovery contract](../crafting-pickup-ownership.md#inspected-recovery-for-a-new-plan).
Neither success nor handle drop performs continuation. Body callers retain
`ActionDispatch.id` and keep reading action status/events for late `Corrected`,
finite tick completion, lease release, and evidence loss.

## Information ownership

| Information | Authoritative owner | Time difference | Discard boundary | Re-derivation cost |
| --- | --- | --- | --- | --- |
| Invocation ID, scope, deadline, cached terminal and late join | Voxrig operation state, connection registry for actual execution | Deadline can precede actual owner/descendant exit | Last observer plus actual owner and watchdog released; live owners are never LRU evicted | Small synchronous state; process-local identity cannot be recreated as authority from serialized diagnostics |
| Frame start, complete write/flush and queued descendants | Native writer/actor and ledger | Admission, local completion and server processing are different instants | Actual native owner/response debt retirement | Delivered TCP prefixes cannot be undone; timeout cannot reconstruct NotSent |
| Strict result and inventory response debt | Original reducer, strict record, executor/native pending owner | Only receipts newer than original write boundary qualify | Original response/deadline/scope retirement; observer drop does not discard debt | Slot equality cannot reproduce missing original receipts; compare bounded menu snapshots/resync with a new round trip |
| Operational Crafted/External and continuation basis | Independent operational crafting owner | Full contents/cursor/collect arrive independently of strict inspection | Explicit valid continue or scope retirement | Bounded snapshots and collect records; current inventory alone cannot reconstruct external provenance |
| Body acceptance and late Corrected | Body action/evidence log, retained ActionId | Initial write precedes acceptance and possibly later rollback | Existing retention/monitor gap and lifetime policy | Fresh capture cannot recreate a lost past rollback |
| Retirement | Connection revocation and shutdown registry | Local fencing precedes certified teardown | Original connection diagnostic lifetime | Unknown shutdown cannot be promoted to actual closure from elapsed time |
| Intent and caller task budget | Golemkit | Caller may stop waiting before SDK execution finishes | Application diagnostic policy | Caller timeout cannot infer writer state or a seal |

The server owns actual effects. None of these local facts elevates a local write
to server acceptance, synthesizes a receipt, or resends an original click/dig.
