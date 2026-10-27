# Changelog

## 0.3.0

- BREAKING: native Java 1.16.1 inventory exposes received values only; ACK predictions never update inventory.
- Add single-send inventory acceptance APIs with independent response/state progress and explicit timeout resync.
- Canonical shared player/menu slots and received-basis container close. See [migration](docs/migration-0.3.md).


## Unreleased

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
