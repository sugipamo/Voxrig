# Plain dig: deadline and original stage evidence

`Survival::dig` has a **30-second total absolute deadline**, starting when its
future is first polled. Both adapters (Java 1.16.1 and 1.21.11) use it. It includes
native readiness, snapshot locks, queue/writer admission, START, estimated time,
STOP or cancellation ABORT, target captures, rejection inspection, and the
connection registry's joined-result handoff. Tokio must remain schedulable;
blocking a runtime thread inside synchronous code cannot be timed out by Tokio.

The deadline returns `ErrorKind::OperationUnconfirmed`. Read
`error.dig_snapshot()` for the original ID, target, face, optional original
session/estimate, phase, current boundary, first occurrence of each phase/boundary
pair and independent START/STOP/ABORT evidence. `deadline_exceeded()` distinguishes
the internal deadline from other failures. A successful `DigRecord` includes
`snapshot`; this also accompanies `Rejected`, `Changed` and `Unconfirmed` outcomes.
The original error category and structured refusal sources survive added progress
context. Snapshot serialization omits the joined completion to avoid recursive
receipts; `DigCompletion::snapshot()` exposes its frozen pre-publication progress.

`begin_dig`, `DigOperation::finished`, cancellation commitment, joined completion,
Body, strict mining and mo9 `OwnedOperation` retain their existing contracts.
Long digs can use `begin_dig`, `finished_until` and explicit cancellation/retirement.
This change adds evidence to that existing owner; it adds no parallel begin API
or claim that a passive observer deadline joins its owner.

## Timeout and write ownership

The plain owner and its caller observe the same absolute deadline. A replacement
waiter or a new internal stage cannot renew it. The caller's timeout only needs
the small synchronous operation state, so it can return even if the registry or a
native gate is stalled. The owner checks the timer before polling execution,
including a late first poll, and drops its execution future on expiry. It never
replays START/STOP or adds an automatic deadline ABORT. Existing waiter-drop
cancellation still requests the single eligible ABORT while the budget remains.

A committed legacy command may outlive the dropped adapter future in the native
actor. `Pending` in a live/error snapshot is uncertainty, not proof of no write.
Joining the bounded plain owner seals unresolved evidence as `Unknown`; it does
not join that actor descendant. A late original write cannot replace the frozen
completion with success. Modern interrupted-frame teardown and the legacy
`pending_dispatch` fence remain in force. If START may have been sent, the plain
owner retains a failed original-session cleanup record on expiry. Even fully
written STOP at a stalled capture is insufficient to free this uncertainty gate.
No-state/no-commit expiry does not invent a sent packet.

`DigCompletion::operation_outcome()` projects an internal plain deadline as
`Unknown(Deadline)` before considering write dispositions. `AlreadyStopped` or
`Aborted` remains local write evidence and does not override deadline uncertainty.

## Await audit at 97015bc and coverage after the change

Every await in the plain execution subtree below is covered by the same total
budget. “No intrinsic bound” also includes waiting for a mutex whose current
holder awaits something else. Synchronous registry/estimate/validation work is
finite but still requires a schedulable runtime.

| Boundary | Java 1.16.1 before this change | Java 1.21.11 before this change | Current evidence |
| --- | --- | --- | --- |
| Exclusive operation slot | Immediate `try_lock_owned`, no queue | Same | Admission error; no invocation writes |
| Owner result | Connection registry task await, no intrinsic bound | Same | Original snapshot; caller uses the same absolute deadline |
| Initial readiness | `wait_until_ready`: player/positioned locks, actor `mark_ready`, readiness notification (only the notification had a configured timeout) | `ready` under state lock | `Preparing/Snapshot` |
| Snapshot gate | `coherent_state_gate.lock()` | `session.state.lock()` | `Preparing/Gate`, `GateAcquired` |
| Snapshot inputs | `common_player_unlocked`: player, positioned, survival, receipts, inventory projection, motion-pause domain states; then player, motion, world | Player/context/world reads under state | `Preparing/Validate`, then `Estimate` and its exact ticks/speed/progress |
| Old plain dispatch | Held coherence gate, re-read player/target/inputs; `execute_common_unlocked`, motion admission, reach, action prerequisites and actor dispatch/reply, no total bound | Held state; revalidation and `session.send`/writer/write/flush, no total bound | Plain now uses the already-established tracked dispatch below |
| Tracked queue capacity | `reserve_dig` waits for native actor queue capacity | Not a separate queue | `Starting`, `Stopping` or `CleaningUp` / `Capacity` |
| Dispatch state gate | Coherence mutex; player, receipts, motion admission, world, snapshot inputs, reach, stop context, receipts again | State mutex; writer mutex; ready/mutable, target, snapshot, reach | `Gate`, `GateAcquired`, `Validate`; modern `Writer`, `WriterAcquired` |
| Commitment/write/reply | Reserve then atomic commit/enqueue, actor writer lock, packet header/body/flush, oneshot reply, post-reply receipts lock | Atomic commit under state/writer; header/body/flush | `Write`, native `Writer`/`WriterAcquired`, `Written`; dispatch evidence is authoritative |
| Timing | `(ticks + 1) * 50ms`, interruptible by drop, finite for a given estimate but no total ceiling | Same | `Waiting/Wait`, `Waited`; estimate retained |
| Error/cancel cleanup | `cleanup_dig` retains Pending then awaits ABORT dispatch; context/transport errors may retain Failed | Same using modern state/writer | `CleaningUp` plus dispatch boundaries and independent ABORT evidence |
| Target capture | `CoreOps::capture`: coherence gate, `common_player_unlocked` lock subtree, `observe_region_snapshot` world mutex | State mutex and capture reads | `Observing/Capture`, `Captured` |
| Rejection inspection | Small synchronous watch lock | Async state mutex | `Observing/Rejection`; K1 server refusal semantics unchanged |
| Observation backoff | 20ms sleep; nominal 1s window checked **after capture and rejection await**, so it could not bound either | Same | Same semantics, plus total deadline |
| Join publication | Registry must join the actual owner before publishing DigCompletion | Same | Frozen progress on completion; no invented join on caller timeout |

The synchronous coordinator/watch mutexes contain short local operations and do
not hold across awaits. Their liveness assumes normal thread scheduling; the
outer deadline does not make arbitrary blocking code interruptible.

## Concurrent entity/block updates and the reported incident

The 1.16.1 ordered packet dispatcher acquires the same coherence gate as snapshot,
dispatch and capture. Entity motion updates take native entity/common receipt
locks inside that gate. Block changes and digging ACK corrections also serialize
there and may replace the target before STOP or while observing. Modern packet
application similarly shares the session state mutex. A holder delayed by writer
I/O or another state lock can therefore delay a dig; the old plain path had no
whole-operation ceiling. A block competitor can legitimately result in `Changed`
or `Removed` without this invocation writing STOP. It is not evidence of a server
refusal, and it does not justify sending the original mutation again.

The remote issue's 60s interval does not identify a particular native await.
The original remote CT logs were unavailable. Deterministic held-gate, held-state,
writer-prefix/flush, long-estimate and stalled-capture fixtures demonstrate the
old unbounded mechanisms and the new bound, but do not establish which occurred
in that run. An eight-entity/1024-native-motion-update fixture exercises the real
packet application path and a competing target change; it completes without
reproducing a deadlock. No entity-flood-specific execution fix is claimed.

Enable existing tracing with `VOXRIG_TRACE_OPERATIONS=1`. Both versions emit
`voxrig_dig_lifecycle` records: `operational_stage`, `operational_estimate`,
`operational_deadline`, correlated by immutable operation ID. Repeated capture
polls coalesce in history and tracing. The existing process-wide 4096-record trace
cap still applies; returned snapshots remain available independently of tracing.
History is bounded by the finite phase/stage product, not entity packet count.

## Information ownership

1. **Authority:** original `OperationState` owns ID, phase, estimate, deadline and
   independent local-write evidence. The native actor/writer supplies write facts;
   original received target/rejection records supply server facts. No second
   completion oracle is introduced.
2. **Time difference:** snapshots are point-in-time copies. `Pending` may settle
   later; deadline errors and joined receipts stay immutable. A legacy actor
   descendant may write after the plain owner joins; that is still uncertainty.
3. **Discard boundary:** expiry drops the original execution future and prevents
   further owner dispatch. Retain its failed cleanup/pending/transport fences and
   original evidence until inspection or connection retirement. Do not derive
   rollback or replay permission from expiry, new observations or a late write.
4. **Rederivation cost:** bounded in-memory phase/stage pairs and one estimate per
   invocation. ID and past commitment/write facts cannot be reconstructed from a
   fresh world snapshot; retain them. Domain state/writer locks are unnecessary
   to read progress or produce the deadline error.
