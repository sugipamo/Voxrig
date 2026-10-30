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
