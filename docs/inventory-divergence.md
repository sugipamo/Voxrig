# Inventory divergence and received change history

Both Java 1.16.1 and 1.21.11 expose the same API.
[Retained official-server LIVE evidence](evidence/inventory-divergence-20261026.json)
records verified jar hashes and packet/report hashes. The packet reducer owns current
inventory. Comparisons below are immutable evidence and never write predicted
values into current slots, menus or the cursor. `EventKind::InventoryChanged`
remains a `Copy` notification; read the separate change log for its payload.

Java 1.16.1 uses the same received inventory projection for admission, pre-send
comparison and every receive snapshot, including `SubmittedClose` when vanilla
does not echo a local container close. The reducer retains the original opening
receipt; the projection determines whether the send uses Player or Container
scope. Only the executor settles a send at the completed response/contents/cursor
boundary. It retains the deadline until comparison also finishes. A changed
scope invalidates a pending send as `ResyncRequired`; missing or late ACKs cannot
leave it Pending indefinitely or acknowledge a replacement resync/send.
Receipt timeouts, including zero, start after the complete frame write;
reservations have a separate finite waiting budget.
[The official 1.16.1 close/reopen LIVE evidence](evidence/inventory-pending-scope-live-1.16.1-20261027.json.gz)
includes the original responses, comparison evidence, independent server
inventory, packet hashes and source/binary hashes. Reproduce it with
`scripts/run_inventory_pending_scope_live.py --accept-eula --output .local/pending-scope/live`
after building `cargo build --locked --example climbing_control_probe`.

## Send once, then replan when the received result diverges

`Survival` and `Creative` keep their existing `send_inventory_click`,
`send_inventory_transfer`, `send_crafting_result`, `inventory_send_record` and
`resync_inventory` signatures. `InventorySendStatus::Diverged` is the one added
terminal outcome. It releases the send executor just like `Received`; a strict
diagnostic can independently remain `RequiresInspection`.

The additional public methods on `Client` are:

```rust,ignore
pub async fn inventory_divergence(
    &self, send: InventorySendId,
) -> Result<Option<InventoryDivergence>>;
pub async fn inventory_changes_after(
    &self, after_sequence: u64,
) -> Result<InventoryChangeLog>;
```

Types live in `voxrig::client::inventory`: `InventorySnapshot`, `InventoryDelta`,
`InventoryItemDelta`, `InventorySlotDelta`, `InventoryLocation`, `InventoryCause`,
`InventoryChange`, `InventoryChangeLog`, `InventoryIntendedChange` and
`InventoryDivergence`. The retained log bound is
`MAX_RETAINED_INVENTORY_CHANGES: usize = 1024`.

Minimal Golemkit-style loop (one caller owns the send):

```rust,no_run
use voxrig::{Client, Error, ErrorKind, Result};
use voxrig::client::inventory::{
    InventoryClickButton, InventoryDivergence, InventorySendStatus, InventorySource,
};

async fn send_and_check(client: &Client) -> Result<Option<InventoryDivergence>> {
    let ops = client.survival();
    let original = ops.send_inventory_click(
        InventorySource::Player, 9, InventoryClickButton::Left,
    ).await?;
    let mut waiting = original.id;
    loop {
        let record = ops.inventory_send_record().await?.ok_or_else(||
            Error::new(ErrorKind::State, anyhow::anyhow!("send no longer retained")))?;
        if record.id != waiting {
            return Err(Error::new(ErrorKind::State, anyhow::anyhow!("send superseded")));
        }
        match record.status {
            InventorySendStatus::Received => return Ok(None),
            InventorySendStatus::Diverged => {
                // Rebuild the remaining plan from divergence.current.
                // before and intended explain the old plan; neither is current.
                return client.inventory_divergence(original.id).await;
            }
            InventorySendStatus::ResyncRequired => {
                // New no-effect probe. Never replay the expired original click.
                waiting = ops.resync_inventory().await?.id;
            }
            InventorySendStatus::Pending => {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            _ => return Err(Error::new(ErrorKind::State, anyhow::anyhow!("new send status"))),
        }
    }
}
```

`Received` requires the response/full-state boundary and agreement with the
declared intention. `Diverged` carries `send`, received `before` and `current`
snapshots with their original receipt sequences, `intended`, `actual` and
`causes`. A mismatch, an in-flight collect or a transient external change that
was later restored requires replanning. Timeout stays `ResyncRequired` until an
explicit successful resync yields `Diverged` with `TimedOutResync`; its evidence
still identifies the expired original send. An unrelated/no-intent resync can
finish `Received`.

For ordinary PICKUP and QUICK_MOVE, `InventoryIntendedChange::Exact` contains the
full comparison delta derived from the actual pre-send capture. Expected
successors are tagged `Predicted`, and live only in this comparison. Crafting
uses the partial `CraftingResult` intention: received displayed output,
destination and deterministic cursor delta. Native ingredient consumption,
remainders and repeated shift crafts cannot be inferred from a displayed output.
Until the full material/slot intention can be established, **every completed
crafting-result send conservatively yields `Diverged`**, including an ordinary
successful craft. This avoids certifying an unrelated change inside the native
crafting destination range as `Received`. The existing fresh-grid/result receipt
contract still describes delivery independently. `actual` includes all received
slot/item/cursor changes; replan from `current` without replaying the craft.

`Rejected`, `Collected { entity_ids }`, `TimedOutResync` and `External` are hints,
and multiple can coexist. A collect refers to an actual `collect_item` naming
the local player, never another player's pickup. `Send { id }` in change history
means an in-flight send was correlated with the packet, not proven causality.
Java 1.16.1 deliberately uses a mismatching comparison to obtain full contents
and cursor: its ordinary successful click also receives `accepted=false`.
That bit alone is not a failed effect. A divergent result retains `Rejected`;
an exactly matching ordinary result remains `Received`. No ACK rolls inventory
back or commits a prediction.

An unrelated location with no pre-send receipt can become known while a send is
pending. This hydration is neither an external stock mutation nor an effect
mismatch. `Comparison::inspect` owns this policy for both adapters; server
receipts and the actual delta (including `complete=false`) are kept unchanged.
An intended delta location without a received predecessor stays conservative:
even a receipt matching a guessed successor cannot establish agreement without
a prediction basis. Changes to known pre-send values still latch `External`, even if
subsequently reverted. Response, contents and cursor boundaries remain required.

The #424 packet fixture reproduces wooden-pickaxe pickup from slot 9, unrelated
player slots 0..4 hydrating to Empty, and continuation to hotbar 36/37 in all six
ACK/contents/cursor orders. The optional LIVE driver deliberately forgets only
the client's grid knowledge, then checks unchanged official-server receipts and
ordinary production sends, with and without a known external mutation:

```sh
cargo test -j2 --lib --no-run
# Use the test executable printed by Cargo, not the .d dependency file.
python3 scripts/run_inventory_hydration_live.py --accept-eula \
  --test-binary target/debug/deps/voxrig-<hash> --output .local/hydration-live
```

This controlled knowledge fixture is not a claim to replay the original RTA run.
See the [retained 1.16.1 qualification](evidence/inventory-hydration-1.16.1-20261027.json.gz).
The test-only invalidation adds no public API. Discard the active comparison at
its terminal response boundary or scope invalidation; retained immutable evidence
follows the existing lifetime. Fresh capture/comparison and remaining swap planning
are bounded local work over 46 player slots plus cursor, without another owner or
server round trip for hydration.

## No-effect sends

Voxrig never writes predicted slots, so a send the server refused (for example a
non-armor item placed into an armor slot, or a click from a spectator) does not
show up as a change being undone. Instead, the server-processed boundary simply
still equals `before`. `InventoryDivergence::effect()` classifies this explicitly:

```rust,ignore
#[non_exhaustive]
pub enum InventoryEffect { NoEffect, Different }
impl InventoryDivergence { pub fn effect(&self) -> InventoryEffect; }
```

`NoEffect` means `actual` is empty at the response/full-state boundary that ended
the send (every captured location still holds its pre-send value), and every
intended location was received there. `Different` covers everything else,
including an intended location that is still unknown. The classification uses
received values only: the Java 1.16.1 `Rejected` hint is on every legacy click
because Voxrig deliberately sends a mismatching comparison stack, so it is never
the deciding signal. A transient external change that was restored before the
boundary keeps its `External` hint and is still `NoEffect`.

When an ordinary (non-crafting) send ends `NoEffect` and no hint was received,
`causes` is now **empty**. It previously fell back to `[External]`, so Java
1.21.11 labelled the caller's own no-op click as an external change. Crafting
sends keep the old fallback; their strict take record owns that outcome.

The strict click/transfer record bound to the same pre-send boundary
(`Survival::click_inventory`, `transfer_inventory`, and the strict record created
by `send_inventory_click`/`send_inventory_transfer`) then ends at the terminal
`InventoryClickStage::ObservedUnchanged` / `InventoryTransferStage::ObservedUnchanged`
instead of staying `Pending`. A record that already holds a fresh changed receipt
or a latched conflict keeps its stage. Inventory helpers report this as
`ErrorKind::Rejected` instead of a 3-second `ErrorKind::Timeout`.

[Official-server LIVE evidence](evidence/inventory-no-effect-live-20261027.json.gz)
holds one original click while RCON switches the player to spectator, so vanilla
processes it without applying it. Java 1.21.11 resends full contents and answers
the fence: `Diverged`, `NoEffect`, empty `causes`. Java 1.16.1 refreshes the
container without a transaction reply. The send becomes `ResyncRequired`, and the
explicit resync ends `Diverged`/`NoEffect` with `TimedOutResync` and `Rejected`.
In both cases the strict record latches `RequiresInspection` because the received
game mode changed. That conservative stage takes precedence over `ObservedUnchanged`.
Reproduce with `cargo build --locked --example climbing_control_probe` and
`flock ~/logs/voxrig-live.lock python3 scripts/run_inventory_no_effect_live.py
--accept-eula --output .local/no-effect-live`.

## Read received differences with a cursor

```rust,no_run
# use voxrig::{Client, Result};
async fn drain(client: &Client, cursor: &mut u64) -> Result<()> {
    let log = client.inventory_changes_after(*cursor).await?;
    for change in &log.changes {
        // change.receive_sequence, change.session, change.source,
        // change.delta.items, change.delta.slots, change.causes
        for item in &change.delta.items {
            println!("{}: {:+}", item.item.name, item.change);
        }
    }
    *cursor = log.receive_sequence;
    Ok(())
}
```

Initially use zero. The cursor is a **packet receive sequence**, independent of
`events_after`'s event ordinal. Each record is immutable and has the original
packet sequence, connection/world and opening identity. Repeated equal receipts
produce no delta. Reads send no packets and remain available after disconnect.

`items` aggregates physical stock including cursor, without counting appended
player/menu aliases twice or counting crafting result previews as stock.
Identity includes item data with count normalized to one; signed changes retain
metadata changes as removal of one identity plus addition of another.
`slots` includes changed player/container locations and `Cursor`, including
result previews. Before/after values retain their receipt ordinals. Unknown is
`None`/`Unavailable`, never an invented empty stack; `complete=false` warns that
known quantities alone cannot establish a complete net change.

Outside a send, `/give` produces `External`. During a send, unexplained changes
can include both `Send` and `External`. The next nearby inventory delta can
carry a received collect hint (at most 128 intervening packets); this is evidence
of correlation, not a claim that an untagged slot packet belonged to that entity.
The send outcome independently retains all its in-flight collect IDs, bounded
to 1024. Reading a log never consumes its records or changes outcomes.

Retention gaps return `ErrorKind::State`; future cursors return `InvalidInput`.
World and active opening changes clear prior records and invalidate cursors
before that boundary. Reused numeric window IDs never combine old/new openings.
On a gap, discard the upper-layer inventory premise, capture received current
inventory/screen, and resume at that capture's receive sequence; propagate a
second scope/retention error if it changes again. Never silently skip a gap.
Keep divergence evidence before the next send: only the current send (plus its
timed-out original during resync) is retained. Superseded sends and retired
worlds/openings return `State` instead of another send's evidence.

## Compatibility and information ownership

This change is **PUBLIC_API=breaking**: `InventorySendStatus` was exhaustive in
0.3.0. Adding `Diverged` and making it `#[non_exhaustive]` requires downstream
matches to handle `Diverged` and a wildcard, as above. `Received` now compares
the declared effect rather than reporting only protocol progress. Function
signatures and `EventKind::InventoryChanged` are unchanged; all evidence types
and the two getters are additions.

1. Voxrig's received slots and comparison records, and Golemkit's copied stock
   and expected effects, described the same inventory but could disagree.
2. The packet reducer is authoritative. Snapshots/logs are immutable receipts;
   the comparison has no permission to update current state.
3. Each receipt replaces its current value. The next send discards old comparison
   evidence; world/opening changes retire it. The history evicts above 1024 records
   and reports gaps. The upper layer discards its premise on divergence/gaps.
4. Capturing/comparing costs O(slots), item aggregation also compares identities;
   retained history is bounded. Golemkit recomputes only its remaining plan from
   current receipts, without reimplementing protocol transactions or diff storage.

Fixture coverage includes the F96 seed-2 action-23 receipt values and village
window-4 cobblestone x3→x4. The retained F96 diagnostics do not include collect
entity IDs: historical replay reports `Rejected`/`External`; a separately marked
synthetic collect demonstrates the additional `Collected` hint without inventing
historical evidence. `scripts/run_inventory_divergence_live.py` checks real item
entity pickup, ordinary clicks and `/give` on both SHA-1-verified official jars.
