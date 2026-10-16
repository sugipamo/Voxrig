# Internal staged quiesce ledger and shutdown (#131 PR 1–5)

This is the internal implementation contract and integration checklist for the
public staged API in `voxrig::client`. PR 1–3 supply finite root/child ownership,
real socket permits and protocol/retained cleanup. PR 4 adds ordinary-producer
parking, explicit selected stops, independent-maintenance sealing and public
stage observers. PR 5 adds passive `Client::wait_shutdown`, private task/resource
ownership and evidence-based transaction classification. The ledger, guards,
producer tokens, task registry and CompletionPoint remain crate-private. Resume
and automatic target selection remain excluded.

## Identity, ownership, cuts, and completion

One transport owns a Ledger for its immutable private GenerationKey
(MinecraftVersion, connection_id). Guard/waiter Arcs retain only ledger state,
never the session. Connection identity is distinct from world generation and
controller identity; world changes must not reset counters. Different ledger
instances cannot exchange cuts: each Drain retains the shared state that issued
its private cut. Later integration must ensure exactly one ledger per session
and validate adapter generation/world/owner provenance before granting authority.

Root admission registers an owner under the same synchronous mutex used to
capture C, the last admitted root ID. Register before exposing retained intent,
enqueueing, spawning, or sampling. A snapshot covers roots <= C plus every child,
including children created after C. Independent later roots are outside its debt.
Moving a WorkGuard transfers the same live authority; child creation increments
ownership while its parent is still live. A child can represent a queued command,
pending transaction, timer, receiver reconciliation, or any retained continuation.
The parent may retire after creating a child. No handoff may retire the last guard
before registering the continuation. Dropping a caller future is not completion
of work transferred to another owner. Guard and permit types are not Clone.

Explicit `finish` relinquishes all future write authority held by that guard.
Only the last owner removes the root. Bare Drop, panic, abort, or unclassified
owner loss records OwnerLost; it is never success. A caller may explicitly report
NoWrite, CancelledBeforeWrite, or SuppressedBeforeWrite only with certainty that
its authority produced no write. Complete and Rejected describe classified local
retirement; neither proves game success. Dependency owners must remain live until
all required local cleanup/reconciliation is complete. Frame completion is local socket flush. Separately, root/drain completion waits
for existing protocol pending state and exact-barrier cleanup, including the
existing ACK/barrier commit or expiry. Dropping ProtocolTransaction::wait does
not retire the actor's pending owner. Public transaction/inventory wait outcomes,
deadlines and cancellation behavior remain unchanged.

Failures include UncertainDispatch, TransactionUnconfirmed, RequiredReleaseFailed,
OwnerLost, Revoked, ID exhaustion, and mutex poisoning. The first failure is
session-sticky. All not-yet-issued receipts fail thereafter, including older cuts
unrelated to that failure. Failed waits can finish before other owners retire;
failure does not claim those owners have stopped. Integration must still fence
and await shutdown on error. A receipt already issued and cached by a waiter
remains a historical fact after a later failure. Uncertainty is never converted
into certainty by idle state, receipt eviction, timeout, or later success.

## Producers and admission closure

Producer lifetime is separate from finite debt. A producer registers before it
can create independent finite roots through the same admission gate. Snapshot
cuts do not await producer lifetime. The internal close-and-cut primitive closes
ALL new roots and producer registration irreversibly and requests producer park
under one mutex. It waits for both finite debt and producer park acknowledgements.
Existing guards can still create descendants and write after closure.

A producer acknowledges park only after it cannot create another independent
sample/queue/root; any previously admitted finite work stays counted separately.
A park acknowledgement consumes the producer token; normal producer termination
uses the same explicit acknowledgement. Unacknowledged Drop is OwnerLost.
Parking emits no network stop and selects no owner. No resume exists.

The single closed bit is only a foundation primitive, NOT the complete staged
quiesce state machine. Later PRs must distinguish normal/selected-stop/independent
maintenance admission and maintain ACK-dependent children through final seal.
This PR grants no bypass or stop authority and settles no multi-party authority
policy. A future first drain cannot use this primitive to indiscriminately block
protocol dependencies that must instead be attributed to existing roots.

## Write ordinal and receipt semantics

Each adapter holds its writer/actor serialization before begin_write,
through write_all + flush and commit. No ledger lock is held across I/O or await,
or while acquiring writer/state locks. The ledger independently allows at most
one live write attempt; a second attempt is WriterBusy and consumes no ordinal.
Each attempt receives a checked monotonic ordinal. A permit owns a child guard,
so even retirement of its parent cannot hide in-flight I/O. Commit publishes W
under the same ledger gate used to inspect root completion. An unsuccessful or
cancelled allocated attempt drops the permit, recording sticky UncertainDispatch.
There is no post-allocation "cancelled before write" path for a permit: this is
intentionally conservative until adapters can establish a narrower certainty.

W means every frame in 1..=W completed locally, including flush. It is not the
maximum observed ordinal or a count that skips failures. Dropping ordinal N
cannot advance W past N-1. In the bounded foundation admission mode, terminal
failure rejects further attempts. PR 2 production tracking preserves the existing
adapter admission/error behavior: later permitted frames may still complete, but
no unissued drain can succeed after any sticky failure, and W never crosses a
failed frame. No packet can bypass a real permit in either mode. Commit of
an already active attempt after an unrelated terminal failure may record its
actual local completion, but no new success receipt may be issued. Frame
completion does not imply server read, protocol ACK, or physical stillness.

Receipt issue linearizes under the gate: all covered root owners have retired,
required producers are parked, no terminal failure exists, and W is read in the
same state. Every active target write holds an owner, so a separate counter read
is unnecessary. A snapshot may observe an unrelated later root's completed W;
it neither waits for that root nor prevents further writes. Even the internal
closed receipt describes only the ledger model; it is not a public
SealedForFence certificate.

Both adapters create their transport identity and ledger before the handshake.
The same ledger follows handshake, login, configuration and play; compression,
respawn and reconfiguration never reset its ordinals. Legacy keeps its ledger in
the writer and actor; modern transfers it from login to Session. Ready remains
the initial supported boundary for the public drain API.

The single policy isolation point is `CompletionPoint::SocketFlush`, the
`COMPLETION_POINT` constant and `publish_completion` in
`src/client/ledger/mod.rs`. SocketWriter calls this only after the unchanged
protocol encoder finishes write_all and flush. This completion policy is
**the approved PR 4 write boundary**. W does not wait for server ACK. Changing this
policy later must not silently change public ProtocolTransaction::wait or
inventory wait semantics. Pending guards are independent of frame permits.

SocketWriter has no AsyncWrite implementation and exposes no mutable stream.
Production framing accepts a WorkGuard, allocates a WritePermit while holding
the adapter's existing writer serialization, and consumes it at that policy
point. Its separate shutdown method preserves existing transport teardown.
Raw protocol encoding remains available to test servers, not to a production
socket outside this wrapper.

Legacy write-capable queue entries register before enqueueing and own a scoped
guard through actor processing, including normal/primitive/batch/protocol/
bounded_motion writes. ACK, expiry and local cleanup commands retire existing
pending authority without independently admitting roots, so they still run
after root closure.
Each acknowledged dispatch transfers a child to its pending map. The public
ProtocolTransaction only keeps a Weak reference to that pending owner so merely
holding the public handle after ACK cannot prolong debt. Exact barriers obtain
children while pending remains live. Barrier removal precedes actor commit;
expiry records TransactionExpired before reporting DeliveryUnknown. ACK
rejection replies inherit children from the matching pending root before its
retirement; unmatched replies use independent maintenance roots. Teleport confirm
and position response, and login settings/brand responses, share maintenance
roots across all their frames.

Modern sends register before waiting on the writer mutex; cancellation of that
queue wait retires only that scope. The existing interrupted-frame mechanism and
public errors are preserved. Receiver response vectors (including position,
configuration and loading responses) share a maintenance root. Inline native and
common pending_swap state owns a guard before exposure/send, and its frame uses
a child. Existing successful wait/record cleanup retires it; wait timeout records
sticky TransactionExpired while preserving the public pending marker for
resumed waits. Context reset which discards pending_swap records TransactionOutcomeUnknown.
No new expiry deadline, autonomous cleanup worker or public wait outcome is added.
An expiry failure is sticky for all unissued drains, including a cut covering the
expired state, and cannot turn clean after later ACK or idle state.

## Waiting, terminal errors, and bounded retention

Each Drain subscribes to its own Tokio watch receiver before inspecting state.
The wait loop is inspect under mutex, await changed, inspect again. Watch values
are hints, never completion authority. send_replace publishes changes even with
no receiver, and preserves a change between inspection and waiter registration.
Multiple waiters have independent wakers. Dropping a waiter does not retire work,
reopen admission, or cancel the session. The waiter caches its first issued
result and can be awaited/inspected repeatedly. `Drain::inspect` remains an
internal synchronous non-blocking path: one short lock, a BTreeMap prefix query
(O(log active roots)), and O(1) producer/failure checks; it never awaits or scans
retired history. A no-runtime unit test covers pending, completion, sticky failure
and historical cached success for the public synchronous poll. No runtime or timing
assumption is needed in the ledger core tests; they poll futures with counting wakers and use explicit
handoffs rather than sleeps.

The foundation admission mode retains provisional bounds: 1,024 active roots,
4,096 live guards (including write permits), and 128 unparked producers. Capacity
rejection happens before authority or IDs are created and is retryable; moving a guard works even at capacity.
PR 2's tracking mode deliberately adds no transport capacity rejection: legacy
already permits 1,024 pending transactions plus queued/unacknowledged traffic,
and modern permits arbitrarily many caller futures waiting on its writer lock.
Imposing the foundation caps would change observable admission/errors. Tracking
retains only the actual live queue/writer/pending owners and removes them at
retirement; it adds no per-packet history or detached background owners. It uses
the identical root/child attribution model and real permits. Tracking does not
ignore root closure, WriterBusy, checked ID exhaustion or sticky drain failure.
It grants no arbitrary maintenance/stop bypass; PR 4 supplies separate restricted
admission classes and the final seal across these same writer boundaries. Root, producer,
and write IDs use checked u64 allocation (0 is the empty prefix); u64::MAX is a
valid final ID and the next request fails closed permanently. No wrapping,
Atomic::try_update, new dependency, or post-1.85 API is used.

Retired roots/producers are removed immediately. There is no retired-ID set,
per-frame history, or ledger-owned receipt/waiter registry. Live roots plus C
implicitly compress the retired prefix; late descendants cannot recreate a
retired root because no guard remains. Outcome totals count guard retirements,
including write children, cumulatively at receipt issue (NOT exact cut/root
statistics). Totals saturate without affecting authority or failure state. A
single terminal summary retains the first cause and optional root forever.
Receipt/waiter storage belongs to its caller and is not retained by the ledger.
Admission-mode bounds, tracking of existing transport admission, cumulative
diagnostics, global failure conservatism, one write attempt, and one closed bit
are internal choices; no public API is introduced here.

## Operation-family completion and dependency inventory

This checklist comes from design §1.3 at baseline 9932ae4. PR 2 wires the
protocol pending/writer boundaries described above; PR 3 implements the finite
operation-family ownership described below. PR 4 supplies producer/stop obligations below; PR 5 supplies shutdown ownership below. "Classify"
means retain certainty/uncertainty and required local dependencies, not a settled
Golemkit ACK/outcome contract. Frame W and pending cleanup follow the approved
interpretation above; the public stop scope is documented below. Golemkit
BackendDrain itself is external.

| Family / implementation anchors | Future owner and completion dependencies | Boundary or unresolved integration point |
| --- | --- | --- |
| Connection actor, protocol transactions, expiry (`java_1_16_1/lifecycle.rs`; modern operation state) | Pending entry owns a guard through reply/expiry classification, required barrier release and history update. | PR 2: dropping the public waiter does not retire pending debt; existing ACK/barrier cleanup retires it and expiry fails the ledger. |
| Diagnostic dig expiry (legacy lifecycle) | No socket write debt; task termination belongs to shutdown registry. | Do not mistake diagnostic task completion for transaction certainty. |
| Compatibility inventory transaction monitor (legacy client) | Inherit transaction owner through inventory/exact barrier cleanup. | Completion of transaction.wait alone is insufficient. |
| Inventory swap / click (`common_inventory`, `common_click`; modern `operations/inventory.rs`) | Send, retained confirmation/reconciliation and inspection use root/children until no write/reservation remains. | Inline modern paths count as well as spawned workers; protocol-specific reply requirements remain open. |
| Inventory transfer (`common_transfer`; modern `inventory/transfer`) | Preserve guard from send through reconciliation/inspection. | Never expose zero debt between worker exit and retained record cleanup. |
| Crafting result take / transfer (`common_crafting`; modern `inventory/crafting`) | Own confirmation, record reconciliation and actor finish/release. | Must progress without callers repeatedly fetching public records. |
| Recipe placement (`common_recipe_placement`; modern `inventory/recipe_placement`) | Retained receipt, reconciliation and actor release inherit ownership. | Receipt getter is not an admission/completion mechanism. |
| Placement (`common_placement`; modern `operations/placement`) | Send plus retained intent/placement_record reconciliation own dependencies. | Pure observation history alone is not debt after all authority retires. |
| Staged mining START / FINISH / ABORT (`common_mining`; modern `operations/mining`) | Every admitted stage and its retained transaction/cleanup owns finite debt. | A not-yet-requested FINISH is future admission; held sessions are separate from finite debt and can be explicitly selected through the PR 4 permit. |
| Operational dig / cancel ABORT (common `operational_dig`) | One root owns START, timed FINISH, error/cancel ABORT and cleanup history; descendants survive caller drop. | Timed/cancel continuation is existing debt, unlike a future staged mining request. |
| Finite path / motion (`common_motion`; modern `movement/control`) | Finite run owns all ticks, landing/release and cleanup even after API returns a record. | A returned record does not retire the run. |
| Continuous ground / flight / glide (`common_control`; modern `operations/continuous`) | Registered producer plus separate finite tick/update roots; park ACK after no new independent emission is possible. | Classify sampled/queued updates and jumps; public suppression/ACK mapping remains undecided. |
| Selected stop / release (`stop_control_in_mode` in both adapters) | Explicitly admitted finite root owns every release frame and local cleanup. | No automatic selection, stop facade, scope, or Golemkit outcome decided here. |
| Legacy passive / native physics (legacy client 50 ms loop) | Producer registry and same admission gate as common control. | Native/passive loop must not restart independent emissions after park. |
| Flight toggle / landing (`common_flight`; modern `operations/flight`) | Inherit root through finite landing motion and release. | No automatic land choice during quiesce. |
| Container open / cursor return / close (`common_container`; modern `operations/container`) | Owner spans every cursor return step, intervening receive wait, subsequent write and release. | Include inline open paths; an idle interval is not completion. |
| Dismount / release / vehicle control / resume ground (`common_vehicle`; modern `operations/vehicle`) | Finite inputs, neutral release and grounding run use roots/children through cleanup. | Selection of neutral/dismount/resume actions remains stop policy. |
| Respawn (`common_api`; modern `operations.rs`) | Connection-owned send/history guard survives caller cancellation. | World reset must not discard old connection debt. |
| Reader / supervisor (both clients) | Reader remains alive for dependency replies; each generated response gets a child or independent maintenance root. | Reader lifetime is shutdown debt; final maintenance seal needs protocol audit. |
| Manager relay / diagnostics reporter (legacy manager/reader_diagnostics; common manager) | Reporter is connection-owned and joined; manager event relay owns no socket/admission. | Manager relay and arbitrary subscriber lifetimes are outside Client shutdown. |
| Writer teardown (legacy lifecycle; modern outbound) | One coordinator joins tasks and retains writer shutdown failure; the serialized writer slot releases the socket. | Abort/spawn alone and drain receipts do not prove shutdown. |
| Recovery / reconnect (common survival/recovery; modern profile_recovery/recovery) | Old ledger covers old transport only; fresh login/write uses a new ledger. | Reattach orchestration serializes claims; no receipt transfer across connections. |

PR 2 audits normal gameplay, primitive/batch sends, legacy bounded_motion writes
and independent protocol maintenance (keepalive, teleport/config/loading) through
SocketWriter. Remaining integration must audit retained worker/continuation state,
inspection and producer parking, including native paths, not merely tokio::spawn
sites. PR 4 integrates the selected stop/producer/seal obligations below. Resume is excluded. Test-only scoped threads, observations tests, and
native trial spawn_blocking paths are not production owners. Public receipts depend on the complete writer/producer/dependency audit below.


## PR 3 finite ownership and caller cancellation

Finite admission scopes register before publishing intent. A connection-owned
worker receives a child before spawning and retains a raw WorkGuard through its
entire future, including timers, receiver waits, subsequent frames, release and
history cleanup. Cancelling the public waiter does not cancel that worker.
Worker cancellation/panic is OwnerLost; a failed finite result records sticky
TransactionUnconfirmed while preserving the original public result. Existing
explicit aborted stages and receipt timeouts cannot become clean drains.

`ledger::finite` propagates the same existing root/child authority through a
private Tokio task-local scope. `Ledger::work` inherits only when Arc identity
matches the originating ledger, including after root closure; a different
transport (even a separate ledger with the same diagnostic GenerationKey) admits
its own root. This is context propagation for the existing attribution model,
not a new attribution policy, a maintenance bypass, or a stop permit. Actual
writes still allocate real permits at the unchanged SocketWriter boundary.

The connection/state-owned Retained map holds guards independently of public
records and send workers. It retains only live families/attempts (plus modern
creative slots); replacing/resetting unresolved authority fails closed. Receiver
and existing inspection reconciliation retire it only after required local
completion or sticky uncertainty. Legacy inspection/actor cleanup inherits a
child from retained authority, so it cannot vanish between send-worker exit and
actor release. Completed/failed history carries no write authority. Neither
retained maps nor their weak expiry observers retain a session through a guard.
No getter or scan was added to Drain::inspect: its original synchronous prefix
query and terminal checks are unchanged.

Legacy crafting, swap, transfer, placement and exact-target mining reconcile at
the receive boundary. Crafting no longer has a `confirm` flag that only a public
getter could set. Modern common swaps use the same received-result predicate as
their existing getter to release pending state; click/transfer/crafting/recipe,
container opening and native/common placement/mining retire their ledger owners
from received state. Pure historical observation after a completed dispatch is
not a new continuation. Native pending swaps retain PR 2's explicit wait/cleanup
contract and sticky timeout behavior; merely dropping the public submission
handle never retires pending ownership.

The existing legacy click/swap receipt deadlines also have weak, write-free
ledger expiry observers: a drain fails on expiry even with no subsequent packet
or getter. Successful retirement disarms them by dropping the pending owner;
these observers cannot prolong a completed root. No public deadline, packet,
retry, inspection result or cancellation action is added.

The approved caller-owned timer decision is intentionally different from a
connection-owned continuation. Native 1.16.1 dig_block/use_item_for and modern
dig_survival_cube (and analogous caller-owned finite helpers) retain their
existing cancellation behavior. Dropping such a future records sticky OwnerLost
without scheduling FINISH, ABORT or release. Their existing rustdoc/contracts
remain unchanged. Operational dig is already connection-owned: its root spans
START, timed FINISH or cancellation ABORT, observation wait and cleanup history;
its existing timeout/abort result also makes drain fail permanently.

Finite flight landing spans disable, neutral and every ground tick. Finite
vehicle inputs span every tick and neutral release; dismount/explicit release
and resume-ground each own their accepted finite action. Respawn spans its owned
send and history update on the original transport; a later received world is not
a new ledger. A staged mining START or dismount request does not pre-admit a
future caller-requested FINISH/ABORT/release. Only existing admitted stages and
connection-owned continuations are debt. Recovery creates a fresh connection
with a fresh ledger; no authority/receipt migrates from the old connection.

PR 4 extends these finite owners with continuous/passive producer parking,
queued-jump ownership, selected neutral stops and final maintenance sealing as
described below. Individual writes retain the same real permits. Shutdown
ownership is described in PR 5 below; resume remains excluded.


## PR 4 staged integration and public receipt audit

Normal root admission, begin/cut capture, special stop/maintenance admission,
final seal, failure and receipt issue use the same short synchronous ledger
mutex. Normal closure is irreversible; the final seal additionally closes stop
and independent maintenance. A private task-local child may only inherit from
the identical ledger Arc. Accepted descendants remain legal after seal; the
last guard's retirement removes every ability to create another descendant.
No authority is granted by diagnostic IDs or current idle state.

The public handle shares one stage inspection/cache across clones; each wait
subscribes independently before polling. `is_drained` is synchronous with no
runtime, await, I/O, reconciliation or history scan. Prefix inspection remains
O(log live roots), with constant-time producer/failure checks. Both stages expose
fixed cuts; the first receipt does not prohibit independent maintenance.

Common ground/flight/glide tasks register a producer before publication and own
one finite sample through every tick's frames/model cleanup. Park acknowledges
only after sampling is permanently impossible. Legacy passive/native physics
uses the same registration and gate. Accepted queued jumps retain a separate raw
finite owner until their once-only next tick; they cannot disappear at begin.
Persistent key updates after begin are rejected. The legacy void-returning
set_control/clear_control/suspend_movement_for are explicitly no-ops after begin.
Raw/native/common clones all share the originating ledger and gates.

Receiver response vectors and legacy teleport-confirm/position and settings/
brand pairs hold one maintenance owner across all frames. Final seal rejects new
unrelated response roots before queue/write exposure. Existing ACK rejection
responses inherit the pending owner's child even after seal. Pending transactions,
exact barriers, pending_swap and retained continuations remain distinct debt;
W alone cannot retire them. Writer stalls and teleport/seal race tests check the
no-write-after-final promise at the real socket boundary.

Stop provenance is captured at original control admission or held-input frame
submission, never invented by candidate inspection. The live stop book retains
only current family bindings and selected result observers, not per-frame history
(at most five slots: distinct common/native movement plus the other three kinds).
Retirement uses the exact control or target identity, so native local clear cannot
retire a common owner or a newly replaced native input lifetime. A native local
clear retains its binding until the next neutral producer sample reconciles wire
posture, or until an explicit selected stop. If common control owns wire posture,
a selected native stop clears only dormant local input; the independently selected
common stop releases inherited wire posture. Mount transitions latch movement
invalidation. Confirmed ordinary item release remains known while received using-
item flags catch up. World/mode/hand/mount interruption invalidates an old binding
permanently; a world reset also retires its old held input. Targets
have no public constructor or Deserialize. Inspection selects nothing. One
borrowed StopPermit admits one connection-owned worker per chosen binding;
retries join/cache it and waiter cancellation cannot cancel cleanup. A rejected
required stop is sticky failure. No raw-packet or normal-control bypass exists.

Intentional permit dig ABORT is the approved exception to PR 3's abort rule:
a confirmed cleanup yields a clean StopReceipt and may yield a clean drain.
Observed cleanup deadline expiry is TransactionExpired and unknown delivery remains
UncertainDispatch. Other aborts/expiry/owner loss, including caller-dropped native
helper futures, stay sticky. Item release is permitted despite gameplay effects.

Revoke records its ledger cause before publishing the separate transport fence;
receipt issue uses that same ledger gate. Issued stage facts remain historical;
unissued stages fail. Disconnect/EOF is failure, never a completion certificate.
These stage facts do not assert shutdown, fresh-reattach readiness or server stillness.
PR 5 supplies the separate local shutdown observer below.
The public surface is the approved minimal draft: no try_receipt, native bridge,
public ledger/watch/guard/CompletionPoint, or additional prelude identities.

Jump admission and queue publication hold the producer's state/coherent lock.
A request still waiting for that lock when begin closes admission has not yet
been accepted and returns State. The producer cannot acknowledge park between
acceptance and publication of a queued jump. This ordering is tested in both
adapters, including the legacy native jump path.


## PR 5 shutdown completion and Golemkit integration

`Client::wait_shutdown()` passively observes the exact receiver's original
`GenerationKey`; it never selects cleanup, begins quiesce, revokes, or reconnects.
Before termination it remains pending. All clones and concurrent waiters observe
one immutable cached terminal result. A call after completion returns the same
`Ok(())` or retained error immediately. Dropping/timing out a wait affects only
observation. Accepted teardown continues under connection ownership.

Each transport's private registry serializes task registration with shutdown.
Tasks are enqueued under that gate before callers receive their output observers.
A child registers before its parent can retire; after closing, new child futures
are dropped without running. The supervisor removes completed tasks as it awaits
actual JoinHandles, keeping only live tasks and the first error. On termination
it aborts remaining original tasks, awaits every actual handle (including
synchronous destructors), then shuts down/releases the writer under its existing
serialization. It does not hold the registry lock across joins or writer I/O.
Output futures are caller-owned observers with no session/transport capture;
no detached output-observer task is spawned. Dropping an output future cannot
cancel accepted connection-owned work.
Once finite authority has retired or the connection is terminal, read-only
history inspection runs in the caller future instead of entering the closed
registry. Interrupted active motion remains RequiresInspection in native/common
history, even when shutdown aborts its worker before ordinary error cleanup.

The registry covers the legacy actor, reader and supervisor, modern receiver,
continuous/passive producers, finite and selected-stop workers, retained expiry
observers, transaction/diagnostic timers and legacy reader diagnostics reporter.
Natural receiver/actor exit, including unwind, starts teardown without a second
revoke. Explicit revoke closes admission and starts the same teardown. Ordinary
disconnect retains its existing API/behavior; use wait_shutdown for the separate
completion fact. No application packet is emitted by teardown after final seal.
The closed writer slot releases its write half even while idle Client/native
clones survive; the joined receiver releases its read half. External finite guards
are not shutdown lifetime debt, and later transport attempts remain fenced.

`Ok(())` certifies local transport release and actual connection task exit, not
peer EOF/ACK, remote game success, physical stillness or reattach readiness.
Manager-owned event relays and application-owned futures/tasks/subscribers are
excluded. The application must cancel/join its own source tasks. A stalled
executor or externally held writer lock can delay completion without bound.
The final publisher retains only result state after joins and teardown.
`WriterShutdownFailed` retains a failed serialized writer shutdown even after
socket release; `TaskFailed` retains panic/unexpected cancellation; requested
shutdown aborts are expected termination after joining. Cleanup continues after
ordinary errors. `Internal` fails closed when bookkeeping is untrustworthy and
may return without proof of cessation. Every error withholds the success certificate.
Errors preserve a shared immutable source for all waiters.

E1 retains `TransactionUnconfirmed` for compatibility and unspecialized failures.
Only an observed required protocol/cleanup deadline emits `TransactionExpired`
(legacy transaction expiry, retained deadlines, modern swap/mining/placement
waits, selected legacy dig ABORT timeout). Actual discarded unresolved swap
context or lost held-input provenance emits `TransactionOutcomeUnknown`.
Generic finite Err/failed booleans and generic Timeout errors without domain
evidence retain `TransactionUnconfirmed`. Interrupted delivery remains
`UncertainDispatch`. Outer observer timeouts latch neither transaction kind.
The first recorded cause wins in stop errors and final drain diagnostics; later
ACKs, idleness or successful shutdown cannot repair a failed drain.

### C1 preflight and source handoff

**Containers must be explicitly closed and ordinary item use ended before
begin_quiesce.** Serialize application source admission first, settle existing
inventory operations, close the original actual `ScreenId` with existing
mode-specific close_container, and end item use. Keep source
serialization through begin so another common/native clone cannot reopen a screen
or restart use. Quiesce supplies no atomic pre-close gate or automatic cleanup;
new ordinary container closes are rejected after begin.

Use audited cursor-return receipts and complete close dispatch. Vanilla need not
send a close echo: Dispatched is not ObservedClosed, and neither shutdown nor a
drain proves server-side return of crafting inputs. Preflight error requires
fencing and shutdown while preserving the failed handoff; it must not manufacture
a successful close. No container StopTarget or permit-close method is added.

Golemkit should retain the exact source Client and pair its key with its own
source generation. After preflight, retain the QuiesceHandle, expose the first
DrainHandle through a synchronous `Fn() -> Result<bool, BackendError> + Send + Sync`,
explicitly neutralize selected original targets, then seal and await the final
DrainHandle. Even after `is_drained()` returns true, call `wait()` for the receipt
and retain scope/key/cut/W. A first-stage true cannot complete the handoff.

On success, error, outer deadline or orchestration cancellation, a surviving
application supervisor must preserve the preflight/stop/drain result, revoke that
same source, cancel/join application-owned source tasks and await
`source.wait_shutdown()`. Dropping an orchestration future does not execute its
remaining cleanup lines. Keep primary and shutdown errors separately; successful
shutdown after failed drain remains a failed handoff. Shutdown timeout/Err blocks
a successful completion claim. Only then may policy attach a fresh generation.
Never substitute a newly current backend for the original source during cleanup.

Map TransactionExpired to Golemkit's timeout category; map
TransactionOutcomeUnknown, TransactionUnconfirmed and UncertainDispatch to its
operation-unconfirmed category; fail closed for other/future kinds. Preserve the
original DrainError as source and never automatically retry uncertain mutations.
No Golemkit dependency or BackendDrain implementation belongs in Voxrig. The
available GitHub credentials returned 404 for the Golemkit repository during PR 5;
its actual constructor/error names remain unverified. The API-contract example
therefore describes the callback contract and requires consumer-side construction
against Golemkit's real definitions.

The only PR 5 public additions are Client::wait_shutdown, opaque ShutdownError
with generation/kind and Clone/Debug/Display/Error, non-exhaustive
ShutdownErrorKind::{WriterShutdownFailed, TaskFailed, Internal}, and the two
DrainErrorKind variants. Shutdown types are exported from voxrig::client only,
with no root alias, prelude expansion, native bridge, synchronous poll, shutdown
handle/receipt, public trait or new dependency.
