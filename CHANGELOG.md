# Changelog

## Unreleased

- Changed operational dig refusal messages to name the failing prerequisite, START/STOP phase, elapsed milliseconds since START at STOP, and before/after values including player y/vy (diagnostic only, not a contract); error kind and existing message prefix are unchanged.
- Added `ContainerStatus::NoOpening` to received player snapshots: a received Java 1.16.1 Join Game or Respawn sets `container_state` to `Received(NoOpening)` with that packet's receipt; Open Window and server Close Window supersede it as before, and a local close still invalidates. `ContainerStatus` is `#[non_exhaustive]`, so this is an additive public API change (#192).
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
