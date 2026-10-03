# R3 primitive dispatch checkpoint

> 過去の派生版の実装・検証記録です。現行の名称・統合方針は[client API設計](public-client-api.md)を参照してください。

This checkpoint adds a typed, context-bound low-level dispatch surface for the
Zen adapter. `PrimitiveOperation` is for ordinary Body-selected packets and
`CleanupPrimitive` is a separate finite cleanup surface. Both pass through the
connection actor and actor-owned writer; a normal operation is rejected after
the disconnect barrier while cleanup remains admissible during `Disconnecting`.

`Dispatched`, `Acknowledged`, `Rejected`, and `DeliveryUnknown` are distinct client facts;
fresh observation and semantic completion are Body facts, not client outcome
variants. This checkpoint returns `Dispatched` after a successful transport
write and `DeliveryUnknown` after an ambiguous write. `ControlClear` returns
the cleanup-only `AppliedLocally` result because it has no packet write. It
does not turn a dispatch into protocol acknowledgement or semantic completion.
A stale generation,
connecting lifecycle, disconnecting normal operation, terminal connection, or
invalid operation is rejected before writing.

The existing public compatibility helpers remain available. Their local
movement, look, hotbar, click prediction, crafting prediction, and window
cleanup mutations now take the coherent-state gate, so they cannot interleave
with packet application or coherent capture.

`PrimitiveOperation::BlockInteraction` is the generic exact block-use packet;
the deprecated `PlaceBlock` spelling remains only as a source-compatible
adapter. After a successful `BlockInteraction` write, the connection actor
keeps at most one short-lived position marker. The next protocol furnace
window (type `13`) consumes that marker once and exposes it through
`OpenFurnaceObservation` together with actor-owned slots and deterministically
ordered raw properties.
Normal windows, expired or ambiguous interactions, stale generations, and
uncertain writes do not produce a furnace correlation. This observation is
raw context only and is not an acknowledgement or semantic completion.

## Protocol transaction ownership

`AcknowledgedPrimitive` is the typed surface for the protocol operations that
have a matching acknowledgement (`WindowClick` and `DigFinish`). The
connection actor is the sole owner of the per-connection transaction number,
pending transaction set, confirmation matching/order, and acknowledgement
deadline. `ProtocolTransaction::wait()` distinguishes `Acknowledged`,
`Rejected`, and `DeliveryUnknown`; none is semantic game success. Dropping a
`ProtocolTransaction` does not cancel the actor-owned pending transaction.
Digging acknowledgement matching includes the requested `Finished` status and
position; the returned block-state ID is result data, not request identity.
When an acknowledgement deadline expires, the identity is retired for the
connection (and the connection is fail-closed if the bounded retirement set is
exhausted), so a late unidentifiable acknowledgement cannot complete a newer
operation.

The public compatibility `click_slot*` helpers are adapters to that same actor
queue. `InventoryState::pending_clicks` remains only as local prediction and
rollback metadata needed by the compatibility API; it does not allocate
identities, track acknowledgement deadlines, or decide transaction outcomes.
The packet apply path routes Window Confirmation and digging acknowledgements
to the actor and continues to emit the existing compatibility events.

Other protocol operations without a matching acknowledgement remain ordinary
dispatches. Acknowledgement is never promoted to fresh observation or semantic
completion, and any protocol transaction types not represented by
`AcknowledgedPrimitive` remain an explicit follow-up surface rather than a
second queue.

## Real-server checkpoint

Against the pinned Minecraft 1.16.1 server, one coherent observation supplied
the mandatory operation context for a typed look packet. The actor returned
`Dispatched`; local control cleanup returned `AppliedLocally`; a subsequent
managed disconnect reached the server-side disconnected event. This proves the
checkpoint packet shape and lifecycle path only. It is not protocol
acknowledgement or semantic completion evidence.

A separate Minecraft 1.16.1 probe dispatched an exact `BlockInteraction` to a
furnace at `(1, 4, 0)`. The next coherent capture returned the same correlated
position, window ID `1`, furnace window type `13`, and 39 raw slots. The
connection then reached server-side disconnect. This validates only the
bounded packet/window correlation used by the observation adapter.

The same pinned server also accepted a legacy window-click adapter through the
actor-owned transaction queue (`window_id=0`, action `1`) and emitted an
accepted Window Confirmation that cleared prediction metadata. A timed dig
returned a matching `Finished (2)` acknowledgement with `successful=true`;
the subsequent placement probe also completed. These are protocol
acknowledgement and compatibility-routing checks, not Body semantic success
claims.
