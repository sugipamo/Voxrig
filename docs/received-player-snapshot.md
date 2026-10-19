# Received player snapshots

`Client::received_player_snapshot().await?` copies immutable player evidence at
one coherent reader boundary on Java 1.16.1 and 1.21.11. It is available in the
default build and with `native`. The types live in
`voxrig::client::received_snapshot` and are re-exported by `voxrig::client` and
both preludes.

Each `FieldState<T>` distinguishes:

- `Missing`: no qualifying receipt or applicable invalidation in this scope.
- `Received(FieldReceipt<T>)`: a supported supplied value and its original
  connection/world, packet ordinal, and `Instant` at local application.
- `Invalidated(Invalidation)`: eligibility was withdrawn, with its reason,
  displaced session, event session, and original event ordinal/time for packet
  events. Local events have neither packet field.

A capture has a separate `captured_at`. Capturing, polling, local submissions,
predictions, and computed attribute values never create or refresh receipts.
A new packet containing the same value does refresh its field's receipt.
Ordinals order packets within one connection; compare connection identities
before comparing ordinals. One packet can establish several constituents.

```rust,no_run
use voxrig::client::prelude::*;
async fn inspect(client: &Client) -> voxrig::Result<()> {
    let snapshot = client.received_player_snapshot().await?;
    match snapshot.health() {
        FieldState::Received(field) => {
            let age = snapshot.captured_at().duration_since(field.receipt().applied_at());
            println!("health={} age={age:?}", field.value().health);
        }
        FieldState::Invalidated(event) => println!("lost health: {:?}", event.reason()),
        _ => {} // Missing and future states supply no received knowledge.
    }
    Ok(())
}
```

Every respawn, replacement world entry, and modern reconfiguration invalidates
world-scoped knowledge, including explicit absence and full-content markers.
Fields explicitly supplied by that boundary packet can immediately become
Received. Initial entry leaves unsupplied fields Missing. Legacy copied native
metadata cannot carry old receipt eligibility into the new world.

`correction()` retains literal packet operands. `absolute_position()` requires
all three latest position operands to be absolute; `absolute_rotation()` requires
both angle operands to be absolute. Relative operands invalidate their absolute
projection. Model resolution, normalization and submitted look are separate.
There is no received ground accessor. Attributes contain only base and supplied
modifiers, with no computed effective value.

An effect is `Presence::Present(effect)` or explicit `Presence::Absent` from a
supported removal packet. Removal before addition and repeated removal are
received facts. No timer creates absence. Effects iterate the finite supported
version catalog, including Missing, Invalidated and tombstones; unsupported
lookup names return `InvalidInput`. Storage keeps one state per catalog key,
without expiry, eviction, history or cross-session tombstones. Attribute
iteration includes supplied current-scope keys; `attribute(name)` supplies the
shared Missing/Invalidated fallback for other names. Map omission never proves
absence. Initial adapters never emit an absent attribute.

Coverage scopes are precise:

| Accessor | Scope |
| --- | --- |
| `inventory().slots_coverage()` | Exactly canonical player-screen slots 0..45. |
| `inventory().bundle_coverage()` | Those 46 slots plus independent cursor. |
| `container().contents_coverage()` | That opening's constructor-verified slots; unknown layouts cannot be Complete. |
| `dimension_descriptor_coverage()` | Selection and usable received selected geometry with valid configuration owner. |
| `attributes_coverage()`, `effects_coverage()` | Whole server set: Unknown before eligible deltas, Partial afterward, never Complete. |

Only Received constituents count, including Empty slots, false flags, no-use
`None`, and explicit removal facts. Missing/Invalidated contribute no coverage.
A full-packet marker does not certify contents subsequently invalidated.
Revision, cursor and full-content markers have independent receipts. Legacy
revision stays Missing unless invalidated; legacy full contents do not supply a
cursor. Legacy geometry is Missing because adapter constants are not received.
Modern geometry keeps the original registry packet's session/time and owner,
which can predate the world selection within the same connection.

Container identities include session, window number and opening ordinal.
On Java 1.16.1, received Join Game and Respawn packets set `container_state()` to
`Received(NoOpening)` with that packet's receipt and clear opening details.
This means the client reset to its player inventory and no foreign opening has
been observed since that receipt. A fresh connection stays Missing until Join;
capture and reattachment do not create this receipt. Open Window supersedes
NoOpening, while an unmatched server close leaves it and its receipt unchanged.
`Received(Open(id))` has matching details; `Received(Closed(id))` has no opening
details. A close only describes its exact observed opening. Unmatched closes
cannot certify a current opening. Local close invalidates knowledge without
creating a server-close, empty-cursor or empty-slot receipt. A later matching
server close can certify the retained original identity if no replacement
opening intervened. There is no closed-container history.

The Client read sends no packets, waits for no readiness condition, performs no
reconciliation, and can return partial setup/reconfiguration knowledge after
obtaining the adapter lock. Recorded receive failures return errors; observed
closing, closure or revocation fails closed. Existing observation APIs retain
their behavior. Snapshots and their getters are owned, synchronous after capture,
and independent of later connection changes. **No snapshot proves present
liveness, current server state, action acceptance, or permission to act.**
Consumers own freshness requirements and synchronization with later use.
Revocation does not refresh retained evidence or authorize its reuse for new work.
For the distinct meanings of Body retirement and confirmed shutdown, see the
[lifecycle terminology note](api-contracts.md#retire退役の用語).
