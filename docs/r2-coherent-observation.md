# R2 coherent observation checkpoint

> 過去の派生版の実装・検証記録です。現行の名称・統合方針は[client API設計](public-client-api.md)を参照してください。

Status: checkpoint; the R2 data contract and capture boundary are implemented,
but the final R2 gate remains coupled to the R3 actor migration described
below.

## Implemented guarantees

- One connection-owned packet/capture actor serializes packet application and
  `CaptureObservation` commands.
- A shared coherent-state gate serializes packet application, physics ticks,
  and capture across player, motion, survival, inventory, window, world,
  light, entity, and event state.
- Every accepted capture receives a non-reused connection generation and a
  generation-local monotonically increasing observation sequence. Neither
  value is a Zen Body `StateRevision`.
- Cube, sparse Body-selected interest, entities, and events have explicit
  request bounds. Invalid or duplicate sparse interest is rejected instead of
  silently normalized or truncated.
- Unloaded blocks, incomplete light, entity omission, event request omission,
  and connection event-queue overflow remain explicit in the returned value.
- Sparse interest preserves the exact Body-provided order and carries the
  uninterpreted Body-owned interest generation.
- Capture rechecks the lifecycle generation and readiness fail-closed. A
  terminal or unknown connection cannot publish a later coherent observation.

The lifecycle/admission actor and packet/capture actor are separate owners with
non-overlapping authority. The lifecycle actor decides whether a generation is
admissible; the packet/capture actor owns packet-state ordering and coherent
capture. The writer actor owns admitted packet write order. A capture does not
infer connection health from cached world state.

`events_queue_omitted` reports overflow of the connection-owned capture queue.
It is not Tokio broadcast receiver `Lagged` and must not be interpreted as that
transport mechanism. `events_request_omitted` separately reports events left
out because a capture requested a smaller bound.

## Verification evidence

- Unit and concurrency coverage validates request bounds, duplicate rejection,
  exact sparse ordering, unloaded and light-unknown accounting, deterministic
  entity ordering, event omission accounting, generation-local sequences, and
  lifecycle rejection.
- A concurrency test races packet application with capture at the coherent
  gate and accepts only a complete before-state or complete after-state, never
  a cross-domain mixture.
- A real Minecraft 1.16.1 probe captured two observations from one connection:
  generation `1`, sequences `1` then `2`, 3,375 requested cube cells, 675
  light-unknown cells, two sparse-interest cells, 137 queued events, and zero
  omitted events. The probe then observed server-side disconnect.

## Explicit carry-over

R2 is not claimed complete while legacy public helpers can mutate cached state
outside the packet/capture actor. In particular, movement, look, hotbar
selection, and window-click compatibility paths are migrated to typed
actor-owned primitives in R3. The Zen adapter must not assemble formal input by
calling the older per-domain getters; it may consume only one coherent capture.

This checkpoint does not move task meaning, Goal state, Action Safety, Vote,
success evaluation, Body revision ownership, timeout policy, or protocol
transaction identity into the Body. Under TD-413, the new client's connection
actor owns the protocol transaction identity and acknowledgement queue for the
R3 typed click/dig surface; this R2 contract does not expose or reinterpret
those protocol facts.
