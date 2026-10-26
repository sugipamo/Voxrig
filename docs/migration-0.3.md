# 0.3.0 migration: inventory authority and execution

## BREAKING: native Java 1.16.1 received inventory

`Bot::inventory() -> InventoryState` and
`Bot::inventory_snapshot() -> Snapshot<InventoryState>` now project received
slots, carried stack, and opening. Values become visible **later: on server
receipt, not on ACK prediction**. `WindowClick::prediction`, `WindowPrediction`,
and `PendingClick::prediction` are diagnostic data; they never update inventory.
`Bot::window_click_predictions(&self) -> Vec<(InventorySendId, PendingClick)>`
provides bounded native diagnostic history separately from the inventory view.
The compatibility `InventoryState::pending_clicks` map stays empty; read this
separate history and the common send receipt for local intent/protocol progress.
ACK, rejection, and timeout never roll inventory back to an old predecessor.
The package version is 0.3.0; the minimum Rust version remains 1.85.

Native sequences wait for the matching action response **and** full received
contents/cursor before checking the next click's preconditions. A slot packet
has no action number and is never an ACK. `accepted=false` can mean that the
click was applied and its comparison requested resync; it does not establish a
rollback or an exact effect. Native managed sequences request that resync and
report their completed protocol barrier separately from the raw comparison bit
in `last_transaction`. They stop on timeout; the old intent is never replayed.

Ready-session native `close_window`, sequence cleanup, and explicit close cleanup
share the common received-basis close executor, including explicit cursor return.
A fully sent close revokes the old opening's rights as `SubmittedClose`.
It does not manufacture a close echo or clear a received cursor/opening.
Consequently `open_window_state()` can still contain the historical opening
until a real close/new opening arrives. Use the common close record and current
`player_screen` access to distinguish that history from operation permission.

## Additive common send API (Survival and Creative)

The existing strict arguments, record types, `ObservedClicked` matching, and
inspection latch are unchanged. These additional methods use the same executor
and send once; strict evidence remains available through the existing getters.

```rust,ignore
async fn send_inventory_click(&self, source: InventorySource, slot: u16,
    button: InventoryClickButton) -> Result<InventorySendRecord>;
async fn send_inventory_transfer(&self, source: InventorySource, slot: u16)
    -> Result<InventorySendRecord>;
async fn send_crafting_result(&self, grid: &ReceivedCrafting,
    destination: CraftingResultDestination) -> Result<InventorySendRecord>;
async fn inventory_send_record(&self) -> Result<Option<InventorySendRecord>>;
async fn resync_inventory(&self) -> Result<InventorySendRecord>;
```

`InventorySendRecord` contains its connection/world/opening-bound `InventorySendId`,
native legacy action number, send boundary, separate response/contents/cursor
ordinals, `InventorySendStatus` (`Pending`, `Received`, `ResyncRequired`), and a
`resync` marker. `Received` means the protocol and state boundary is available;
it does not certify the intended item transfer or heal a strict diagnostic.

For Golemkit's click/transfer/crafting helpers, replace the strict completion
loop with this send/current-state path, retain the same send's strict record for
diagnostics, and rederive every remaining step from `received_inventory()` and
`screen_state()`. Never call the old strict send a second time for the same intent.

```rust,ignore
use voxrig::client::inventory::{InventorySource, InventoryClickButton, InventorySendStatus};
let ops = client.survival();
let sent = ops.send_inventory_click(InventorySource::Player, 9,
    InventoryClickButton::Left).await?;
loop {
    let progress = ops.inventory_send_record().await?.unwrap();
    assert_eq!(progress.id, sent.id); // retain one caller's send identity
    match progress.status {
        InventorySendStatus::Received => break,
        InventorySendStatus::ResyncRequired => {
            // Stop this helper; explicitly resync and replan, never replay sent.
            let resync = ops.resync_inventory().await?;
            // Poll resync.id until Received before starting a new explicit intent.
            return Ok(());
        }
        InventorySendStatus::Pending => {}
    }
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
}
let diagnostic = ops.inventory_click_record().await?;
let current = client.received_inventory().await?;
// Decide the next explicit operation from current; diagnostic can stay latched.
```

K2 (connection, opening, target slot, received item/cursor and native slot policy)
is rechecked immediately before sending. Delivery uncertainty and unsupported
native data remain errors (K1/K2 are not upgraded into successful effects).

## Information ownership check

1. **Who drifted:** received slots/cursor/opening, native prediction/cache values,
   copied player/menu slots, and strict `released`/legacy actor click ownership
   described the same information independently.
2. **Authority:** the ordered per-version packet reducer owns current inventory.
   Shared menu positions reference canonical player entries through the layout;
   compatibility views, snapshots, and recipe plans are projections. The inventory
   executor owns response/state debt, independently of strict effect history.
3. **Discard:** received packets replace current values; predictions never have
   current-state authority. Timeout/window/world changes revoke continuation.
   Legacy action numbers are never recycled on a connection. A no-effect PICKUP
   at slot -1 requests full resync with a new identity; on legacy, its protocol
   confirmation first drains a potentially lost comparison-rejection response.
   A late old action cannot discharge the new operation. Untagged slot packets
   update the currently received opening only; they cannot identify a click.
   Reopen creates a new `ScreenId` even when the numeric window ID is reused.
4. **Re-derivation cost:** packet reduction is proportional to changed slots;
   owned snapshot and next transfer/crafting plan materialization is O(screen
   slots), with O(1) opening/action identity checks. Each managed click also
   requests O(screen slots) full-state traffic and a separate response boundary. No correction ledger, count
   add-back, unlatch flag, or original-click replay is used.

Modern 1.21.11 has no click transaction ACK. The executor sends an ID-bearing
command-suggestion request after the click to obtain an ordered main-thread
response, and separately requires full received contents/cursor. A network ping
is insufficient because the original server handles it outside the main thread.
The suggestion request executes no command. Original server bytecode also
confirms that normal PICKUP slot -1 is a no-effect operation on both versions
(and differs from the item-dropping slot -999).

The boundary assumes the normal ordered Minecraft TCP stream. A full contents
packet is synchronization state, not causal evidence for a particular click.
A peer that reorders untagged slot packets across a completed full-state boundary
cannot supply the missing action/generation information; do not treat such a
packet as operation evidence.
