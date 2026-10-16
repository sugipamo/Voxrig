# Internal draft: admission ledger, writers and finite owners (#131 PR 1–3)

This is an internal implementation contract and integration checklist, not a
public API or a guarantee of current Client behavior. `src/client/ledger/` is
crate-private and has no public re-exports. PR 2 wires both adapters' writers,
legacy protocol transactions/exact barriers, modern inline pending swaps, and
protocol responses. PR 3 wires finite workers and retained continuations; producer park,
restricted stop and public drain belong to PR 4. There is no public receipt yet.
The approved direction is staged quiesce (option B); resume and automatic stop
target selection are excluded. This foundation does not implement QuiesceHandle,
StopPermit, shutdown, or Golemkit BackendDrain.

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
the initial supported boundary for the future public drain API.

The single policy isolation point is `CompletionPoint::SocketFlush`, the
`COMPLETION_POINT` constant and `publish_completion` in
`src/client/ledger/mod.rs`. SocketWriter calls this only after the unchanged
protocol encoder finishes write_all and flush. This completion policy is
**provisional pending Golemkit**. W does not wait for server ACK. Changing this
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
expiry records TransactionUnconfirmed before reporting DeliveryUnknown. ACK
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
sticky TransactionUnconfirmed while preserving the public pending marker for
resumed waits. Context reset which discards pending_swap also records uncertainty.
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
and historical cached success for the future PR 4 poll. No runtime or timing
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
It grants no maintenance/stop bypass; PR 4 must integrate its shared admission
and final seal across these same writer boundaries. Root, producer,
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
operation-family ownership described below. Producer and shutdown obligations
remain for PR 4–5. "Classify"
means retain certainty/uncertainty and required local dependencies, not a settled
Golemkit ACK/outcome contract. Frame W and pending cleanup follow the approved interpretation above; public
stop scope, BackendDrain and multi-party authority are not implemented here.

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
| Staged mining START / FINISH / ABORT (`common_mining`; modern `operations/mining`) | Every admitted stage and its retained transaction/cleanup owns finite debt. | A not-yet-requested FINISH is future admission; held session and later stop policy remain separate and unresolved. |
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
| Manager relay / diagnostics reporter (legacy manager/reader_diagnostics; common manager) | No socket write owner; handle termination in shutdown registry if required. | Exact shutdown task scope is undecided. |
| Post-revoke writer shutdown task (legacy lifecycle; modern outbound) | Retain shutdown result in future shutdown registry. | Spawn alone and drain receipts do not prove shutdown. |
| Recovery / reconnect (common survival/recovery; modern profile_recovery/recovery) | Old ledger covers old transport only; fresh login/write uses a new ledger. | Reattach orchestration serializes claims; no receipt transfer across connections. |

PR 2 audits normal gameplay, primitive/batch sends, legacy bounded_motion writes
and independent protocol maintenance (keepalive, teleport/config/loading) through
SocketWriter. Remaining integration must audit retained worker/continuation state,
inspection and producer parking, including native paths, not merely tokio::spawn
sites. No stop behavior, stop-permit holder, neutral movement/item/vehicle/dig
release, resume, or final public seal is implemented in this PR. Test-only scoped threads, observations tests, and
native trial spawn_blocking paths are not production owners. No public success
may be exposed until all write-capable paths and dependency handoffs are audited.


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

Continuous control/passive physics/receiver response producer lifetimes,
producer samples and queued jump parking, quiesce/neutral-stop selection, final
maintenance seal, public APIs and shutdown registries remain PR 4–5. Existing
individual writes from those paths still require real writer permits. No park,
stop policy, resume behavior or public drain success is introduced here.
