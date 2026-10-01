# R1a connection lifecycle checkpoint

> 過去の派生版の実装・検証記録です。現行の名称・統合方針は[client API設計](public-client-api.md)を参照してください。

Status: checkpoint; R1 is not complete.

## Implemented

- process-local non-reused `ClientConnectionGeneration`
- connection actor as sole lifecycle and admission owner
- normal, protocol-response, cleanup, and writer-shutdown commands serialized
  through the actor-owned dispatch queue
- `Connecting / Ready / Disconnecting / Disconnected / ConnectionStateUnknown`
- generation-bound `OperationContext`
- normal versus finite-cleanup admission classes
- stale, connecting, disconnecting, disconnected, and unknown typed rejection
- disconnect request closes normal admission, owns no timeout, and waits for a
  terminal reader/transport fact
- server Disconnect is committed to the actor before its terminal event is
  published
- manager registry retains a username until terminal confirmation, preventing
  overlap with a replacement connection
- aggregate resource/protocol failures and admitted packet-write failures
  terminate the generation as unknown and reject all later commands
- dropping the last external handle is local cancellation and becomes unknown;
  it is never reported as a confirmed transport end

The public generation, lifecycle, context, admission, and terminal-wait APIs
exist for the future Zen in-process adapter. Generation is a process-local
transport correlation value. It is not a Body `StateRevision`, packet sequence,
protocol transaction number, or persistent identity.

## Verification

- `cargo fmt --all -- --check`: pass
- `cargo test --all-targets`: pass, 88 tests
- `cargo test --doc`: pass, 1 doctest
- `cargo clippy --all-targets -- -D warnings`: pass
- real Minecraft 1.16.1 `api_surface_probe`: pass with two clients; both
  transports reached the server-side disconnected event before the probe
  returned

## Explicit carry-over

R1 is not complete because packet write and packet/cache application are not
yet actor-owned:

- Public `OperationContext` is not yet attached to each typed primitive packet
  write; compatibility APIs use their connection's current generation and a
  placeholder observation sequence.
- `Disconnecting + Cleanup` is admitted by the actor, but no cleanup-specific
  typed operation/result path exists yet.
- Write ambiguity terminates the lifecycle as unknown, but the public typed
  `Dispatched / Acknowledged / DeliveryUnknown` outcome belongs to R3.
- Existing compatibility APIs remain available and are not the future Zen
  primitive port.
- Packet application still updates domain locks directly. Coherent capture is
  not implemented.

These gaps must not be described as delivery safety or coherent observation.
They are closed by the remaining R1 dispatch ownership work and R2 capture
work before any Zen production adapter is connected.
