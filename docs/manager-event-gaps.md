# Java 1.16.1 BotManager event completeness

With `native`, `BotManager::subscribe()` now relays source broadcast overflow as
`BotEvent { username, event: Event::ManagerGap { generation, skipped } }`.
This is additive: `BotEvent` still has the same two fields, `subscribe()` keeps its
return type, and `Event` was already non-exhaustive. Java 1.21.11 does not use this
manager and is unchanged.

Subscribe before connecting. A gap is emitted at the relay read that detects
`RecvError::Lagged(skipped)`, before the next retained event from that source.
`skipped` is the exact number of native source events lost at that read; it is
neither a protocol packet count nor an aggregate-stream loss count. `generation`
matches `Bot::connection_generation()`, so a gap from a retired connection must
not invalidate its replacement merely because they share a username. Multiple
source gaps remain separate notifications. An already stopped connection drains
its buffered events and gaps before replaying a missing terminal cause once.
As before, events preceding the manager's source subscription are not replayed;
the manager is a live notification handle, not a complete connection journal.

```rust,no_run
# async fn observe(manager: voxrig::versions::java_1_16_1::BotManager) -> anyhow::Result<()> {
use voxrig::versions::java_1_16_1::Event;
use tokio::sync::broadcast::error::RecvError;
let mut events = manager.subscribe();
loop {
    match events.recv().await {
        Ok(event) => match event.event {
            Event::ManagerGap { generation, skipped } => {
                // Withdraw this generation's history-dependent assumptions.
                // Re-read received state and retained domain logs before replanning.
                eprintln!("{} connection {} lost {skipped} events",
                          event.username, generation.get());
            }
            _ => { /* ordinary native notification */ }
        },
        Err(RecvError::Lagged(skipped)) => {
            // The aggregate channel lost events, possibly including ManagerGap.
            // Withdraw completeness for ALL connections covered by this receiver.
            eprintln!("manager receiver lost {skipped} notifications");
        }
        Err(RecvError::Closed) => break,
    }
}
# Ok(()) }
```

A source gap does not modify the Bot's received state, reject sends, disconnect,
or imply that the server rejected an operation. It withdraws a claim that the
notification history was complete. Increasing channel capacity only changes the
pressure threshold. The aggregate remains bounded at 512 notifications; losing a
gap at this stage causes the subscriber's own `Lagged`. Neither layer silently
repairs lost transient payloads.

Use the retained Bot handle (and compare its generation), rather than looking up
an old connection by username. Where a common Client is used, read its existing
`inventory_changes_after`, `health_changes_after` / `subscribe_health`,
`explosions_after`, and `events_after` directly. These logs are written by receive
reducers, independently of this relay, and expose their own bounded retention
gaps. BotManager creates no second canonical state or duplicate domain journal.
A missing transient event without a retained log or prior packet trace cannot be
reconstructed from the latest state.

Ownership:

- Previously the Bot's native broadcast, manager queue and an upper consumer's
  completeness assumption could disagree: the relay discarded the source's
  `Lagged` while downstream reception looked complete.
- The server owns game state; receive reducers own its local observations. The
  source receiver's `Lagged` owns the fact and count of relay loss. The manager
  forwards that fact without inventing replacement state.
- On `ManagerGap`, discard history-dependent assumptions for that generation
  before processing its next retained event. On aggregate `Lagged`, discard
  completeness for all connections covered by that subscriber.
- Latest state costs the relevant state read; retained history costs O(retained
  records) within each log's bounds. Unretained transient payloads are
  unrecoverable. Replanning cost belongs to the upper consumer.

Validation uses 1.16.1 protocol fixtures with a two-event source, exact loss
counts, ordering, early terminal delivery, reconnect generations, and independent
fast/slow aggregate subscribers. The LIVE runner uses the existing disposable
vanilla harness and original packet proxy, source capacity one, and a continuously
consumed aggregate; it requires real source gaps without downstream lag and
compares a subsequent received game mode with RCON. It does not synthesize loss
or alter server packets.

```sh
cargo build --locked --features native --example manager_gap_probe
python3 scripts/run_manager_gap_live.py --accept-eula \
  --binary target/debug/examples/manager_gap_probe \
  --jars /path/to/official-jars --output .local/manager-gap-live
```

[Recorded 1.16.1 LIVE evidence](evidence/manager-gap-live-1.16.1-20261027.json.gz)
contains the official server hash, received gap samples, independent RCON state,
and source hashes.
