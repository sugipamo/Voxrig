# Changelog

## 0.3.0

- BREAKING: native Java 1.16.1 inventory exposes received values only; ACK predictions never update inventory.
- Add single-send inventory acceptance APIs with independent response/state progress and explicit timeout resync.
- Canonical shared player/menu slots and received-basis container close. See [migration](docs/migration-0.3.md).


## Unreleased

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
