# Changelog

## Unreleased

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
