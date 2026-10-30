# Changelog

## 0.3.0

- BREAKING: native Java 1.16.1 inventory exposes received values only; ACK predictions never update inventory.
- Add single-send inventory acceptance APIs with independent response/state progress and explicit timeout resync.
- Canonical shared player/menu slots and received-basis container close. See [migration](docs/migration-0.3.md).


## Unreleased

- Add `Client::recover_inspected_operational_crafting` and the bounded receipt
  reader `Client::inspected_crafting_recovery`. A fresh immutable received
  full-contents/cursor basis can release a terminal inspection owner after all
  response/write debt is discharged. Original verdict, External, strict take,
  send and divergence remain intact. See `docs/crafting-pickup-ownership.md`.

- Bound plain survival digging to a 30-second total deadline on both adapters.
  Preserve original START/STOP/ABORT evidence and uncertain gates on expiry;
  expose native stages and estimates through the existing DigOperation snapshots,
  plain results/errors and opt-in operation tracing. Explicit begin_dig ownership
  and cancellation contracts are unchanged. See [await audit](docs/plain-dig-bounds.md).

- Add bounded owned primitive handles for selected controller stop, HeadlessBody
  initial dispatch, strict/acceptance click and transfer, strict result takes and
  operational crafting. Typed terminal evidence and original scope survive waiter
  drop; late owner exit is retained separately. Add explicit bounded connection
  retirement and finite observation on the existing DigOperation. Modern unsent
  caller-owned click/transfer drops now close as NotSent and release their gate.
  See [owned operations](docs/client/owned-operations.md).

- Bind inventory completion to the final send boundary on both adapters. Legacy
  negative stack comparisons no longer imply rejection; completion waits for the
  ordered full contents and fresh cursor. Missing ordinary click/transfer receipts
  trigger one bounded no-effect resync and retain typed inspection reasons without
  replaying the original intent. Strict API semantics are unchanged. See
  [completion ownership](docs/inventory-completion-ownership.md).

- Add independent operational crafting proof on both adapters. Submit one cursor
  result with `Survival::send_operational_crafting_result` or its `Creative`
  counterpart, poll `Client::operational_crafting_outcome(send.id)`, and inspect
  `OperationalCraftingOutcome` / `OperationalCraftingStage`. `Crafted` requires
  fresh received grid/result/cursor consumption/output proof AND exact residual
  stock attribution to local-player collect packets plus item entity metadata,
  native item data and quantities. Expose pickups separately as `CraftExternal`
  and `external_delta`; always report inventory divergence to the upper layer.
  Unknown evidence, quantity mismatches, foreign collect, history overflow,
  uncertain delivery and unsupported recipe effects stay `RequiresInspection`.
- Operational continuation is explicit: retain the outcome/External report, then
  call `Client::continue_operational_crafting(send.id)` and plan the next
  acceptance inventory operation from its full received `basis`, including the
  occupied result cursor. Claiming blocks subsequent inventory executor sends
  until successful continuation; inspection cannot be released this way. No
  result click is resent and no strict history is cleared.
- Golemkit migration from main `d6df3f0` `backend_inventory/craft_reconciliation.rs`:
  replace the craft operation's strict-inspection/`take.is_resolved()` gate and
  partial-send `Diverged` classification with the exact send/take's operational
  `Crafted` outcome; propagate External before continuing. Caller recipe-goal
  checks may remain. Preserve `CraftingTakeRecord` and send comparison diagnostics
  unchanged: selected-hand pickups can still leave strict inspection permanently.
  Existing `send_crafting_result` can opt in by claiming its still-retained outcome.
  The proof supports one ordinary result PICKUP with one-item-per-input
  consumption; remainders/different regenerated results/repeated shift crafts
  conservatively require inspection. See [ownership and migration](docs/crafting-pickup-ownership.md).

- Bound crafting take Pending on Java 1.16.1 and 1.21.11 to a configurable
  inventory receipt wait (default 5 s), one no-effect full resync, and a second
  wait (default 5 s). Missing receipts retain a typed RequiresInspection reason;
  expiry never proves success or permits retry/cursor release. Preserve late
  received inventory/grid and legacy replies for inspection. Add
  `CraftingTakeRecord::is_resolved()` and common inventory receipt timeout limits.
  Strict Look admission is unchanged. See [take deadlines](docs/crafting-take-deadlines.md).

- BREAKING: replace `ClientLimits.max_chunks` and native 1.16.1
  `ConnectionOptions.max_chunks` with `chunk_capacity: ChunkCapacity`.
  Auto sizes from received view distance, including light-only columns and
  teleport overlap, up to a hard column-count cap; Fixed keeps an explicit
  budget. Evict distant columns without disconnecting, preserving the received
  view and body neighborhood; evicted terrain/context/light become Unknown.
  Add `Client::chunk_cache` availability epoch/loss metrics and typed
  `ChunkCapacityError`/`Error::chunk_capacity`. Native `ChunkSnapshot` gains
  `terrain_received`; light-only terrain no longer fabricates Air. See
  [policy and migration](docs/chunk-capacity.md).

- Match Java 1.16.1 dig replies to the last sent action before concluding
  `Rejected`; a delayed START rejection cannot stand in for the STOP response.

- Speed up `Client::raycast_blocks` on Java 1.16.1 and 1.21.11 by walking
  received state IDs under one observation lock and borrowing registry-owned
  `StateBoxes`; only the hit state is decoded. Remove per-ray bounding-box
  capture, decoded-state HashMap, and experimental environment switches.
  Java 1.16.1 native world queries share the same walker and shape owner,
  retaining their existing visibility/containment and boundary rules explicitly.
  Queued world changes now follow a complete ray observation rather than
  splitting its metadata and geometry reads. Unknown IDs are validated along
  the ray, so corrupt off-ray cells no longer fail the common query; the native
  world API retains its existing unknown-ID handling. Public signatures are unchanged.
  See [raycast ownership and compatibility](docs/state-id-raycast.md).

- Inventory sends that the server processed without an observed effect are now
  explicit. Add `InventoryDivergence::effect()` and `#[non_exhaustive]`
  `InventoryEffect { NoEffect, Different }`, derived only from the received
  response/full-state boundary (never from the 1.16.1 negative comparison reply).
  An ordinary no-effect divergence without received hints now has empty `causes`
  instead of the `[External]` fallback (Java 1.21.11 labelled the caller's own
  no-op as external); crafting sends keep the fallback. The strict click/transfer
  record bound to that send ends at the new terminal `ObservedUnchanged` stage
  instead of staying `Pending`, and inventory helpers (`compact_inventory`,
  `fill_crafting_grid`, `move_inventory`, ...) return `ErrorKind::Rejected`
  instead of a 3-second `ErrorKind::Timeout`. New stage variants are additive on
  `#[non_exhaustive]` enums. Audit I1/I2/I3. See
  [no-effect sends](docs/inventory-divergence.md#no-effect-sends).
- BREAKING (behavior): a crafting result take whose fresh full resync after the
  send (grid/result, main/hotbar, cursor; 1.16.1 also its transaction reply) still
  equals the pre-send receipts ends as the new terminal
  `CraftingTakeStage::NotApplied` instead of staying `Pending` forever. The
  provoked 1.16.1 `accepted=false` reply is not used as evidence. `craft_once`
  now returns `ErrorKind::Rejected` for `NotApplied` and `ErrorKind::Timeout` for
  a take still `Pending` at its deadline (both were `Ok`); `RequiresInspection`
  stays `Ok`. `craft_once_checked` returns the new `CraftOnceError::NotApplied`
  instead of `OutcomeUnknown { cause: Timeout }`. Both enums are `#[non_exhaustive]`.
  See [result take not applied](docs/common-recipes.md#result-take-not-applied-notapplied).
- Java 1.21.11 now decodes the server-forced rotation `PLAYER_ROTATION` (0x47;
  yaw f32, relative bool, pitch f32, relative bool), which was a silent no-op.
  Relative angles resolve against the local rotation as the native client does,
  and the native look-only echo is sent. Position and its provenance are unchanged.
  The rotation is reported through `player_state().rotation` (`Received`),
  additive `OwnMotion.received_rotation`, additive
  `ReceivedPlayerSnapshot::rotation_correction()` (also updating
  `absolute_rotation()`), `EventKind::PlayerChanged`, and Body
  `Corrected { ServerCorrection::Pose }` with the unchanged position under its own
  source. A running PlayerControl adopts it, so its next movement packet no longer
  overwrites it with the stale aim; scheduled paths and finite motion keep their
  planned yaw. No public signature or enum changes. See [forced rotation](docs/forced-rotation.md).
- Java 1.21.11 `HeadlessBody` now supports `action_observation` and
  `monitor_changes_after` (previously Unsupported), sharing the 1.16.1 monitor
  reducer and semantics. The 1.21.11 `action_outcome`, send-and-trust
  `action_status` and `events_after` refresh now go through the same monitor and
  record evidence gaps, world/target-chunk loss and timeout/read errors in the
  monitor log instead of silently skipping them; their return values are
  unchanged (the historical result or a late Corrected). An ordinary 1.21.11
  PlaceBlock/ActivateBlock monitor now starts at the sent evidence boundary.
  See [Body monitoring](docs/body-monitoring.md).
- Body `UseItem` reports `Corrected { InventoryResync }` only for an original
  full-content packet of the player inventory window (0) on both Java 1.16.1 and
  1.21.11. A container window's WINDOW_ITEMS / ContainerSetContent (for example an
  open chest being resynchronized) no longer marks the use as corrected; such uses
  now finish `Accepted`. Vanilla's own use-item resync of window 0 still corrects,
  also while a container is open. Behaviour change only; no signature changes.
  See [UseItem inventory resync](docs/design/body-api.md#useitem-の-inventory-resync).
- Java 1.21.11 selected drop and hand swap no longer stay `Pending` forever
  (audit I6/I7). A record whose fresh full player contents after the send still
  show the original hands/selected stack and cursor ends as the new terminal
  `ObservedUnchanged` (with `unchanged_boundary`) and releases the inventory
  owner; this is what vanilla does for a dead (not yet respawned) or unloaded
  player. A record still `Pending` 5 s after it was retained ends as `Expired`
  (outcome unknown) and fails the connection ledger with `TransactionExpired`.
  `swap_hands` now sends the same no-op full resync click after its frame as
  `drop_selected` (`HandSwapRecord::resync_dispatched`). Additive: both stage
  enums are `#[non_exhaustive]`; both records only gain fields.
  See [selected drop](docs/common-selected-drop.md) and
  [hand swap](docs/common-inventory-swaps.md).
- BREAKING: a container activation the server processed without opening a screen
  (locked container, chest under an occluding block, plugin refusal) now ends as
  terminal `ContainerOpenStage::NoScreenObserved` with
  `ContainerOpenRecord::no_screen` evidence, instead of staying `Dispatched`
  and holding the owner. Java 1.21.11 needs a fresh BlockChangedAck covering the
  activation sequence; Java 1.16.1 needs the handler's adjacent
  BlockUpdate(clicked), BlockUpdate(clicked+face) pair. Vanilla sends OPEN and
  contents inside the handler, before either boundary. `ContainerOpenStage` is
  now `#[non_exhaustive]` and gains `is_resolved()`; new `ContainerOpenNoScreen`.
  Audit row C4. See [contract](docs/common-container-open.md).
- Add `MotionRecord.interruption: Option<MotionInterruption>` on Java 1.16.1 and
  1.21.11: the typed received boundary (`Correction { position, rotation }`,
  `Velocity`, `Explosion`, `VehicleChanged`, `WorldChanged`, mode/flight/posture/
  attribute/effect changes, death) and its receive ordinal behind the first
  retained `problem` of a finite survival motion. Local failures stay None; the
  free-text `problem`, status and stop policy are unchanged.
- Add `Client::control_state_changes_after` and the PlayerControl equivalent on
  both versions: a bounded (256) connection-owned log of control owner
  transitions (Started, Paused, Resumed, Stopped with a typed `ControlStopCause`
  and the original reason, Expired) with owner id, world, receive ordinal and
  ticks, published under the executor lock that committed the status. Same
  cursor/`ControlChangeGap` semantics as `control_changes_after`, independent
  retention. Stop/pause policies and `ControlRecord` are unchanged.
  See [contract](docs/control-state-changes.md). `MotionRecord` gains a public
  field, so struct-literal construction outside the crate must add it.
- Expose the server velocity override targeting the own player (knockback) on
  both versions: `ReceivedPlayerSnapshot::velocity_override()` with the packet's
  receipt, an `EntityHistoryKind::OwnVelocity` record per receipt (not a contact
  or replay fence), and `EventKind::PlayerChanged`. Java 1.21.11 keeps its
  existing `EntityUpdated { native_id: self }` for compatibility. Control and
  motion behavior are unchanged. BREAKING for exhaustive matches on
  `EntityHistoryKind` (not `#[non_exhaustive]`). See [own velocity](docs/own-velocity.md).
- BREAKING (exhaustive matches, Rust struct literals): `Survival::dig` /
  `DigResult::Finished` records gain `DigRecord::outcome` (`DigOutcome`:
  `Removed` / `Changed` / `Rejected { rejection: DigRejection }` / `Unconfirmed`,
  both new types `#[non_exhaustive]`). A server-processed boundary with the target
  still at its predecessor now returns `Rejected` early instead of the same
  `removed: false` as the one-second timeout. Java 1.21.11 needs the target echo
  followed by a fresh BlockChangedAck covering the last command; Java 1.16.1 needs
  a `successful = false` START/STOP response with the predecessor plus the last
  command's response still carrying it. `removed` and `final_state` are unchanged.
  Common staged mining gains `MiningStage::ObservedRejected` (the enum is now
  `#[non_exhaustive]`) and `MiningRecord::rejection` on Java 1.21.11 (a fresh ACK
  covering FINISH after the unchanged target echo); 1.16.1 still latches a rejected
  response as `RequiresInspection`. Native 1.21.11 adds
  `MiningStatus::ObservedRejected`, `MiningRejection`, `MiningRecord::rejection`
  and `is_resolved()`; `wait_survival_mining` no longer expires the connection
  when the attempt resolved by its deadline. An ACK alone stays pending.
  See [dig outcome](docs/common-dig.md#観測の終わり方digrecordoutcome).
- BREAKING (exhaustive matches, Rust struct literals): strict survival placement
  gains the terminal `PlacementStage::ObservedRejected` (the enum is now
  `#[non_exhaustive]`, with `is_resolved()`) and a `rejection` field on
  `PlacementRecord`, `ReplaceableCubePlacementRecord`, `SlabPlacementRecord` and
  `StairPlacementRecord` (fluid slabs through `placement`). A server-processed
  boundary with target, support and material still at their predecessors now
  ends the attempt and releases the one-shot owner instead of staying `Pending`
  with look/select/move/next placement refused. Java 1.16.1 has no placement ACK
  and uses the original handler's adjacent support-then-target BlockChange pair;
  Java 1.21.11 needs the target echo plus a covering fresh BlockChangedAck.
  Native 1.21.11 adds `PlacementStatus::ObservedRejected`, `PlacementRejection`
  and record fields; `wait_survival_placement` no longer expires the connection
  when the attempt resolved by its deadline. Early handler returns with an ACK
  only stay `Pending`. Historical placed results are unchanged.
  See [common placement](docs/common-survival-placement.md#結果と次の操作).
- Java 1.16.1: a running common PlayerControl no longer drops explosion knockback.
  The receive path adds the impulse once, in receive order, to the running Ground
  model, so the next movement packets carry it (previously each tick overwrote
  native motion with the owner's own frame). Running Flight/Glide owners stop with
  the Java 1.21.11 reason `vehicle or explosion motion outside the engine`.
  BREAKING (Rust struct literals): `ControlRecord` gains `explosion_impulses`, the
  impulses added to this owner after its start boundary, on both versions.
  No new refusal gates or retries. See [explosions](docs/common-explosions.md)
  and [common control](docs/common-control.md).
- Add `Client::control_corrections_after` and the PlayerControl equivalent on
  Java 1.16.1 and 1.21.11: a bounded (256) owner-scoped log of every received
  own-pose packet over a running/paused control owner, recorded at the receive
  boundary so several corrections between two ticks are no longer collapsed.
  Each `ControlCorrection` carries the receive ordinal, model tick, last written
  submission, replaced model position, received position, delta/distance and
  applied velocity. It reports a superseded submission, never a teleport or
  rejection cause. Same cursor/`ControlChangeGap` semantics as
  `control_changes_after`, with independent retention. Additive: existing
  structs, `corrections`, Running/stop policy and the degradation log are
  unchanged; the new `ControlOps` method defaults to Unsupported.
  See [contract](docs/control-corrections.md).

- Inventory send comparison no longer treats first receipts for unrelated,
  previously unreceived slots/cursor as external changes or effect mismatches.
  Actual server receipts and incomplete deltas are preserved. Intended locations
  with no received basis remain conservative; known external mutations still diverge.
  Fixes coherent Java 1.16.1 tool pickups diverging when player slots 0..4 hydrate
  (sugipamo/Golemkit#424), without changing the public API.

- Recipe-book placement no longer stays `Pending` forever when the server ignores
  the request (audit C3). Vanilla sends nothing for an ignored request, so both
  versions now follow each request with one no-op full-resync click
  (`RecipePlacementRecord::resync_dispatched`). Fresh full contents after it that
  still equal the pre-send grid, result, main/hotbar and cursor (1.16.1: after the
  click's own reply, whose `accepted=false` is never read as a refusal) end as the
  new terminal `ObservedUnchanged` (with `unchanged_boundary`) and release the
  placement owner. `ObservedPlaced` / `ObservedGhost` now also wait for that full
  contents, so a plan captured after completion is not invalidated by them. A record still `Pending` 5 s after it was retained ends as
  `Expired` (outcome unknown) and fails the connection ledger with
  `TransactionExpired`. Additive: `RecipePlacementStage` is `#[non_exhaustive]` and
  the record (not constructible outside Voxrig) only gains fields. Callers that
  count serverbound click frames see one more per placement.
  See [ignored requests](docs/common-recipes.md#ignored-requests-observedunchanged-and-expired).

- Add Java 1.16.1 Body `action_observation` and `monitor_changes_after`, separating
  immutable acceptance from late corrections and evidence completeness. Internal
  evidence gaps, world/target-chunk loss and retryable timeout/read errors are
  visible independently of BodyEvent eviction. Permanent loss retains its first
  boundary; successful reads advance the evidence cursor. Existing signatures,
  structs and refusal gates are unchanged. New BodyBackend methods default to
  Unsupported; Java 1.21.11 returns Unsupported for the new monitoring APIs.
  See [contract and recovery](docs/body-monitoring.md).

- Add `Client::selected_hotbar_changes_after` on Java 1.16.1 and 1.21.11: a
  bounded (256) receive-time log of every selected-hotbar write with before/after,
  origin (`Submitted` serverbound held slot, including the respawn reassertion,
  or `Received` clientbound held slot) and `superseded_submission` when a received
  slot replaced a submission. Same cursor/`State` gap/`InvalidInput` conventions as
  `control_changes_after`; readable after disconnect. Snapshots, events and
  `select_hotbar` are unchanged (audit I5). LIVE shows a stale server write
  overtaking an in-flight submission on both versions.
  See [contract](docs/hotbar-changes.md).
- Add `InventoryCause::OwnAction { action, ordinal, interaction_sequence, locations }`
  (additive; the enum is `#[non_exhaustive]`) with `InventoryOwnActionKind` and
  `INVENTORY_OWN_ACTION_WINDOW`. On both versions, `inventory_changes_after` now
  tags a change from the caller's own item use, placement, dig, selected drop or
  hand swap with this correlation hint instead of `External` when it arrives
  inside the action's bounded window (3 s, same world) at the affected hand slot.
  A delta with any slot outside the scope keeps `External` as well; executor
  outcomes and divergence causes are unchanged (audit I9). See [own actions](docs/inventory-divergence.md).

- Java 1.16.1 native BotManager now forwards source broadcast loss as additive
  `Event::ManagerGap { generation, skipped }` before the next retained event,
  including when a connection stopped during manager insertion. Connection
  generations distinguish reused usernames. Aggregate overflow still reports
  the subscriber's own `Lagged`, including loss of a gap notification. Existing
  signatures, struct fields, send admission and Java 1.21.11 are unchanged.
  See [completeness and recovery](docs/manager-event-gaps.md).

- BREAKING (Rust struct literals): `ControlRecord` gains `degradation`, independent
  of Running/Paused/Stopped and `movement_refusal`. Java 1.16.1 ordinary results
  expose the first `RemoteStateDropped` with owner/world, receive boundary,
  model tick and phase. Add `Client::control_changes_after` and the PlayerControl
  equivalent: 256 retained withdrawals across owner replacement/disconnect,
  explicit `State` on eviction and `InvalidInput` on future cursors. Best-effort
  keeps sending, including after server corrections; no new rejection gates.
  Java 1.21.11 uses the default Unsupported log and leaves the new field None.
  See [contract](docs/control-degradation.md) and
  [migration](docs/migration-control-degradation.md).

- Java 1.16.1 chunk/light decode failures now withdraw observations and terminate
  the reader through the existing connection-failure path. Add non-exhaustive
  `EventKind::ObservationInvalidated` at the failed receive boundary; common
  chunk/light/context reads fail immediately, and `received_player_snapshot`
  retains the original packet diagnostic after termination. Bounded event cursor
  gaps remain explicit `State` errors. Public signatures and normal send admission
  are unchanged; Java 1.21.11 is unchanged. See [recovery and ownership](docs/world-decode-failures.md).
- Opening or closing a foreign screen no longer forgets the received player
- Opening a foreign screen no longer forgets the received player
  2x2 crafting result/grid (slots 0..=4) on Java 1.16.1 and 1.21.11. The server
  keeps that grid while another menu is open and sends no player-menu contents on
  close, so `received_crafting()` previously reported every input as unreceived
  after chest or crafting-table use until another player-menu refresh. Armor and
  offhand were already retained the same way; main/hotbar slots still come only
  from the opened screen's contents. Java 1.16.1 crafting result takes also use
  the common received/submitted-close admission, so a historical native screen
  projection no longer rejects player crafting after a foreign screen close.

- Java 1.16.1 foreign close now invalidates a nonempty (or incomplete) player
  crafting grid and automatically requests actual window-0 full contents using
  the inventory resync executor. While waiting, `received_crafting()` exposes
  missing slots; only actual packets restore them. Admission/write and receipt
  waits are bounded, and timeout never restores the preserved values. A fully
  received empty result and four empty inputs need no resync.

- Java 1.16.1 player-grid crafting take and recipe placement accept the native
  cache still naming a locally closed opening. Vanilla need not echo that close,
  so a submitted-close player grid previously failed with
  "received/native cache ownership disagrees" before any send.

- Java 1.16.1 digging ACKs now emit common `BlocksChanged` and additive
  `EventKind::LegacyDiggingAcknowledged` with the received target, state, status
  and success flag, including rejected, late and unmatched replies. Corrections
  also reach pending placement. Bounded `events_after` / `wait_for_events` retain
  outcomes and report eviction as `State`; existing mining/transaction results
  and send admission remain unchanged. Java 1.21.11 processing ACKs are unchanged.
  See [usage and ownership](docs/digging-ack-corrections.md).

- Java 1.16.1 explosions now emit common `BlocksChanged` bounds and
  `ExplosionReceived` at the packet's application boundary. `explosions_after`
  retains original context through the additive, version-specific
  `ExplosionPresentation::Java1_16_1` variant, including affected positions and
  mandatory impulse. Packet/position retention gaps are explicit `State` errors;
  oversized observations do not add receive/send rejections. Existing signatures
  and Java 1.21.11 behavior are unchanged. See [usage](docs/common-explosions.md).

- Add common received-health cursor log and independent push subscriptions for
  Java 1.16.1 and 1.21.11, directly from update_health reducers. Atomic admission
  baseline/tail, bounded health-only retention, explicit lag, ordered world/close
  boundaries, and raw invalid HP are available without the native feature.
  Existing public signatures/enums remain unchanged. See [usage and Golemkit example](docs/health-log.md).

- Fix Java 1.16.1 inventory sends staying Pending after a locally submitted
  container close without a server close echo. Admission, pre-send comparison,
  and receive snapshots share the same scope projection. Comparison retains its
  deadline until terminal settlement; scope changes require explicit resync.
  Existing public APIs are unchanged
  ([Golemkit #415](https://github.com/sugipamo/Golemkit/issues/415)).

- BREAKING: `InventorySendStatus` gains `Diverged` and is now `#[non_exhaustive]`;
  `Received` requires the declared effect to agree with current received values.
  Add `Client::inventory_divergence` and bounded `inventory_changes_after` on both
  versions, with received snapshots, item/slot/cursor differences and cause hints.
  Timeout resync retains the original intent as `Diverged(TimedOutResync)`.
  Crafting-result sends conservatively diverge because the displayed result alone
  cannot establish an exact ingredient/remainder/repeated-craft delta.
  Predictions remain comparison-only. See [usage and migration](docs/inventory-divergence.md).
- Keep strict Clippy checks compatible with Rust 1.85, including const contact-policy proofs.
- Added bounded unknown-terrain walking to `body::Move` and
  `PlayerControl::start_control_best_effort` on both versions, plus
  `start_path_best_effort` for connection-owned finite routes. Terrain warnings
  and route progress are available through the additive `walking_diagnostics_for` getter. Strict prediction
  and receipt APIs retain their existing proof requirements.
- Added `Survival::begin_dig` / `DigOperation` as a public API addition for
  explicit per-dig cancellation, independent write evidence and joined owner
  completion. Dropping handles or waiters keeps digging; cancellation is explicit.
- Fixed Java 1.16.1 dig cleanup recording an ABORT with unknown delivery as
  `Aborted`. It now records `Failed` and returns `OperationUnconfirmed`,
  retaining dispatch uncertainty. No public API change.
- Changed operational dig to accept positive held-stack count changes at START
  and STOP with the same slot, item, and data, including Java 1.16.1 Damage-only
  wear. Error kind and prefix are unchanged; no public API change
  (held-stack count change).
- Changed operational dig to accept faster or equal estimates at STOP with the
  same hardness and harvestability, finite positive speed/progress, no lower
  progress per tick, and no more ticks. START remains exact; the original wait
  and recorded estimate, error kind and prefix are unchanged. No public API
  change (#198).
- Changed operational dig refusal messages to name the failing prerequisite,
  START/STOP phase, elapsed milliseconds since START at STOP, and before/after
  values including player y/vy (diagnostic only, not a contract); error kind and
  existing message prefix are unchanged (#196).
- Added `ContainerStatus::NoOpening` to received player snapshots: a received
  Java 1.16.1 Join Game or Respawn sets `container_state` to
  `Received(NoOpening)` with that packet's receipt; Open Window and server Close
  Window supersede it as before, and a local close still invalidates.
  `ContainerStatus` is `#[non_exhaustive]`, so this is an additive public API
  change (#192).
- Added `Bot::terminal_event()` to retain the first native Java 1.16.1 terminal
  cause, including early exits relayed by `BotManager`; public API addition (#187).
- Changed Java 1.16.1 reader terminal error messages to include the play packet
  id, payload length and handler name. Error kinds and signatures are unchanged
  (#188).
- Changed the behavior contract of `Survival::craft_once_checked`: pre-send
  failures are now `CraftOnceError::Failed` rather than `OutcomeUnknown`;
  uncertain delivery of an admitted take is reported as `OutcomeUnknown`
  (`RequiresInspection`), carrying the fill record and take identity without
  resending. Public types and signatures are unchanged (#178).
