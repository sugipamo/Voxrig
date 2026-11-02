# Guarded native item admission

`Survival` and `Creative` add these common APIs, on Java 1.16.1 and 1.21.11:

```rust,ignore
use voxrig::client::guarded_use::{
    BlockUseTarget, GuardedUseResult, HeldItemExpectation, PhysicalCut,
};

async fn use_item_guarded<F>(
    &self,
    hand: Hand,
    expected: Option<HeldItemExpectation>,
    check: F,
) -> voxrig::Result<GuardedUseResult>
where F: FnOnce(&PhysicalCut) -> bool + Send + 'static;

async fn use_on_block_guarded<F>(
    &self,
    target: BlockUseTarget,
    hand: Hand,
    expected: Option<HeldItemExpectation>,
    check: F,
) -> voxrig::Result<GuardedUseResult>
where F: FnOnce(&PhysicalCut) -> bool + Send + 'static;

async fn use_on_block_expect_slot(
    &self,
    target: BlockUseTarget,
    hand: Hand,
    expected: HeldItemExpectation,
) -> voxrig::Result<GuardedUseResult>;
```

Native `java_1_16_1::Bot` and `java_1_21_11::Operations` also expose the two
`*_guarded` methods with these arguments (the legacy Bot takes its native Hand).
Common handles require matching received mode. Native methods retain native
mode freedom; they do not change the server mode. Every guarded path retains
normal connection, strict domain, motion/inventory debt and block-range admission.
For item use it retains the existing common grounded-pose requirement.

`HeldItemExpectation { selected_hotbar, item: Some(id) }` compares selection and
received item identity, ignoring count/components. `item: None` requires explicit
received Empty. Missing, invalidated, unavailable or predicted slots never satisfy
Empty. Offhand uses player slot 45 and still checks selection. Main hand uses
36+selection. `expected: None` skips identity expectations; the guard can inspect
full stack data in the cut. Input slot, registry version/kind and cursor are
validated before admission. Expectation mismatch skips the callback.

`PhysicalCut` carries `player: PlayerObservation`, `hand`, `slot` and `held`.
The player includes current pose and provenance, selected hotbar, received
inventory, session and receive high-water mark. Legacy native-only selection
inconsistent with the common selection receipt is exposed as unknown. The receive
sequence is applied packet order, not a server tick or proof that every field is
fresh. Buffered server packets can still be waiting outside the local state gate.

The guard runs once, after readiness, queue capacity, native state and writer
serialization waits. The local physical state is frozen from snapshot through
frame flush. The guard sees no Client handle in its arguments. It **must** be short,
synchronous and non-reentrant: use the cut plus independent caller data/atomics;
do not call Client/native APIs, acquire locks shared with Voxrig callers, block,
or enter a runtime. `Send + 'static` does not prevent a closure from capturing a
Client; such re-entry violates this callback contract and can deadlock. Callers
own synchronization of their authorization/lease/stop data. The native boundary
does not freeze a remote lease or the server itself.

A false guard returns `NotSent { reason: GuardRejected, cut }`. An unwinding panic
is caught as `GuardPanicked`, with no packet and no poisoned native lock; the
normal Rust panic hook may still log it. `panic=abort` terminates the process.
Expectation mismatch returns `ExpectationMismatch`. All NotSent results prove no
use frame and no surviving send authority from this invocation. No retry, resend,
background cleanup packet or automatic release-use is generated.

True initiates the original native frame I/O directly, with **no intervening
readiness, state, capacity or writer await**. Actual socket I/O/flush is asynchronous.
`Sent { receipt, cut }` proves complete flush and maps to mo9
`OperationOutcome::SentUnconfirmed`. `NotSent` maps to mo9 `NotSent` via `outcome()`.
It does not manufacture an OwnedOperation ID, Body ActionId or server acceptance.
Existing receipt predicates, server corrections and Body statuses retain their
meaning; callers must observe actual received state to decide the game result.
The existing expect, strict APIs and CraftingTakeRecord are unchanged.

The total budget is five seconds, including readiness/admission/writer/flush. A
precommit expiry returns `AdmissionDeadline` and atomically cancels queued authority.
Dropping the future also cancels precommit authority. Cancellation after commit
cannot retract a write. A postcommit timeout, partial frame, failed flush or lost
actor result is an error/uncertainty, never a proof of NotSent. Stop mutations and
inspect without replay. As with other async Rust APIs, a callback that blocks the
runtime cannot be preempted by a Tokio deadline; the short sync callback contract
is required.

## Read cells

`use_item_guarded_with_reads(hand, expected, reads, check)` and
`use_on_block_guarded_with_reads(target, hand, expected, reads, check)` take the
same arguments plus `reads: &[[i32; 3]]` and fill `PhysicalCut.cells:
Vec<([i32; 3], Option<u64>)>` in request order (duplicates kept). Existing guarded
APIs pass no reads and their `cells` is empty. At most `MAX_GUARDED_READS` (64)
cells are accepted; a longer list returns `InvalidInput` before any wait, guard or
frame and is never truncated.

Each ordinal is the receive sequence (the same counter as
`player.receive_sequence`) of the last applied packet that touched the cell, read
under the same gate as the player observation, so every ordinal is at most
`player.receive_sequence`. Touching means:

| Counts | Does not count |
| --- | --- |
| block change (also with an unchanged state) | light-only update |
| multi-block / section change | block-entity data (NBT, not block state) |
| 1.16.1 digging acknowledgement block state | entity spawn/move/metadata |
| 1.16.1 explosion affected blocks | time, health, inventory and all other packets |
| chunk data, full or partial: the whole column | |

Light-only updates are excluded: received light is a separate fact that Voxrig
already invalidates on block changes, and server relighting would otherwise make
any read near activity look changed. `None` means unknown: the column is not
loaded (never received, unloaded, evicted, world reset or dimension change), the
Y is outside the dimension, or a chunk replacement is not yet bound to its packet.
Callers must fail closed on `None`.

Ordinals can be later than the actual last change but never earlier: a chunk
reload sets every cell of the column, and a section with more than 64 touched
cells since its column loaded collapses them into one section floor. Storage is a
per-column load ordinal plus those bounded per-section maps, owned by the world
store and dropped with the column. Entity overlap with a cell is not a cell touch;
callers that depend on entities must compare entity observations separately.

```rust,ignore
let reads = [target, footing, halo_a, halo_b];
let result = client.survival().use_item_guarded_with_reads(
    Hand::Main, Some(expected), &reads, move |cut| {
        cut.cells.iter().all(|(_, o)| o.is_some_and(|o| o <= prevalidated_sequence))
            && policy_accepts(cut)
    },
).await?;
```

## Box reads

`use_item_guarded_with_box_reads(hand, expected, boxes, check)` and
`use_on_block_guarded_with_box_reads(target, hand, expected, boxes, check)` take
`boxes: &[GuardedReadBox]`, each an inclusive axis-aligned box
`GuardedReadBox { min, max }` (every cell with `min[a] <= p[a] <= max[a]`), and
fill `PhysicalCut.boxes: Vec<(GuardedReadBox, Option<u64>)>` in request order.
Each value is the **latest** receive ordinal over every cell of the box, frozen
under the same gate as `player` and `cells`; it is exactly the maximum of the
per-cell reads above. `None` if any part of the box is not loaded, outside the
dimension's Y range, or lost to a reset or dimension change. Box calls leave
`cells` empty; cell calls and the existing APIs leave `boxes` empty. A single
cell is the box `p..=p`.

| Limit | Value | On violation |
| --- | --- | --- |
| `MAX_GUARDED_BOXES` | 16 boxes per call | `InvalidInput` |
| `MAX_GUARDED_BOX_VOLUME` | 4096 cells per box (16³, one section), any shape | `InvalidInput` |
| `min <= max` | on every axis | `InvalidInput` |

Violations are rejected before any wait, cut, guard or frame and are never
truncated or clamped. The query costs one visit per overlapping column plus its
kept touched cells (at most 64 per section) in overlapping sections, so a call is
bounded independent of world size.

The section floor and the column load ordinal count for every cell they cover,
so a box that only overlaps a busy section, or any part of a reloaded column,
reports that ordinal: box results can be later than the real last change inside
the box but never earlier.

```rust,ignore
use voxrig::client::guarded_use::GuardedReadBox;
let boxes = [
    GuardedReadBox { min: sub(target, 2), max: add(target, 2) }, // 5×5×5 halo
    ray_box,
    footing_box,
];
let result = client.survival().use_item_guarded_with_box_reads(
    Hand::Main, Some(expected), &boxes, move |cut| {
        cut.boxes.iter().all(|(_, o)| o.is_some_and(|o| o <= prevalidated_sequence))
            && policy_accepts(cut)
    },
).await?;
```

## Native locking proof

* Java 1.16.1 reserves actor queue capacity before taking the owned coherent gate.
  Its snapshot, range and domain checks run under that gate. Ownership is held in
  the invocation's cancellation cell and transferred to the actor after the actor
  has obtained writer serialization. The actor performs its original motion/lifecycle
  admission, checks cancellation/deadline/revocation, then calls the guard. It holds
  the gate, receipt lock and writer across check, synchronous commit and complete
  frame flush. It uses the same SocketWriter/ledger and own-action observation as
  the existing actor dispatch. The frozen gate can be released by precommit
  cancellation even while the command is behind unrelated actor work. Once taken
  by the actor it is released on actor exit. A single pending -> committed/cancelled
  atomic transition prevents a cancelled command from sending later. Uncertain
  writes retain pending dispatch and terminate the actor's normal write admission.
* Java 1.21.11 takes state then writer, in the established order, and constructs its
  snapshot after both are acquired. It keeps both through direct `write_frame`,
  including the existing interrupted-write guard and complete flush. The mutable,
  mode, range and pending swap checks are the existing rules. No `send()` helper
  reacquires writer serialization after the guard.
* Read cells and boxes come from the version world store under that same lock: 1.16.1 applies
  every packet, including all terrain mutation, while holding the coherent gate, and
  1.21.11 applies packets while holding the state lock. No additional lock is held
  across the guard.

These locks serialize local receive application, physics and submitted common
pose/selection changes. Other unrelated protocol traffic may exist; NotSent means
no frame for this invocation, not a globally silent connection.

## Golemkit / rta integration sketch

```rust,ignore
let lease = Arc::clone(&current_lease);
let stop = Arc::clone(&stop_flag);
let expected_session = planned_session;
let expected = HeldItemExpectation {
    selected_hotbar: planned_selection,
    item: Some(bucket_id), // None for an explicitly empty-hand UseOn
};
let result = client.survival().use_item_guarded(
    Hand::Main, Some(expected), move |cut| {
        lease.is_current() && !stop.load(Ordering::Acquire)
            && cut.player.session == expected_session
            && physical_policy_accepts(&cut.player) // pure and short
    },
).await?;
match result {
    GuardedUseResult::NotSent { reason, cut } => publish_not_sent(reason, cut),
    GuardedUseResult::Sent { receipt, cut } => {
        retain_original_dispatch(receipt, cut);
        observe_existing_received_result_until(goal_deadline).await?;
    }
    _ => require_inspection(), // enum is non_exhaustive
}
// For UseOn call use_on_block_guarded(BlockUseTarget { position, face, cursor },
// Hand::Main, Some(expected), the_same_guard). No external precheck can replace
// this guard at the native commit boundary. Never automatically retry on error.
```

## Information ownership

1. Native received reducers own pose/inventory provenance; native sender/ledger
   own dispatch admission and write evidence. The caller owns lease/authorization/
   stop policy. A returned cut is immutable evidence, not continued authority.
2. The callback closes the external-precheck/readiness time gap. Field provenance
   remains explicit; a high-water mark alone does not refresh older fields. Local
   frozen state and synchronous external checks do not assert a server tick cut.
3. Precommit rejection/drop/deadline retires invocation authority. Commit transfers
   write responsibility to the native sender. Cuts and results are returned to the
   caller and are not stored as a connection-global latest-use cache. The caller
   retains them and discards old plan assumptions when state/policy changes.
4. Snapshot and selected-slot extraction are bounded copies of existing player
   observation data. Historical check decisions and write outcomes cannot be
   reconstructed from later inventory; retain original results. Waiting and any
   later replanning consume the upper layer's existing goal budget.
