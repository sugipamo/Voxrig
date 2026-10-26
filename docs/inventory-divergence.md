# Inventory divergence and received change history

Both Java 1.16.1 and 1.21.11 expose the same API.
[Retained official-server LIVE evidence](evidence/inventory-divergence-20261026.json)
records verified jar hashes and packet/report hashes. The packet reducer owns current
inventory. Comparisons below are immutable evidence and never write predicted
values into current slots, menus or the cursor. `EventKind::InventoryChanged`
remains a `Copy` notification; read the separate change log for its payload.

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
