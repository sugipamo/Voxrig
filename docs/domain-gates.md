# Domain admission for acceptance actions

`HeadlessBody` uses domain admission instead of the all-family mutation veto.
This policy is local admission; it does not assert that a packet succeeded.

| Acceptance operation | Retained receipts that can veto it |
| --- | --- |
| Move, Look, best-effort ground control/path | Motion/flight/vehicle/dismount owner uncertainty and position submission |
| BreakBlock | Dig/mining owner, unresolved dig cleanup, or an inventory write affecting the retained hand |
| ActivateBlock | Inventory/cursor and container/screen operations (opening another screen can return the shared cursor) |
| UseItem, PlaceBlock | Uncertain held-slot dispatch; use_item_expect keeps strict admission |
| SelectHotbar | Uncertain held-slot dispatch; index 0..8 |
| Chat, Command | Connection/readiness/loading and writer lifecycle only |

Inventory PICKUP, SWAP, crafting placement/take/transfer, selected drop and hand
swap share inventory/cursor state. Container open/close also touches that cursor,
so activation cannot bypass an unresolved mutation of it. A deadline does not
prove the cursor empty, release a transaction, refresh a state ID, or permit
replaying a click. Existing window ID, state ID, ScreenId, source-slot, cursor,
registry, connection/world and item checks are retained. The strict inventory
APIs remain the way to submit these operations. No automatic cursor return,
rollback, retry, or receipt deletion is introduced.

While a strict inventory/container receipt is unresolved, submit unrelated work
through `HeadlessBody`. Both versions allow move, look, dig, chat and hotbar
selection. `PlayerControl::start_control_best_effort` and
`start_path_best_effort` also use movement admission on 1.16.1; 1.21.11 already
used independent continuous-control admission. Strict `start_control` retains
its original behavior. A body dig reserves the same connection-wide dig owner
as `begin_dig`/`dig`, through START/STOP/ABORT and cleanup. Separate body wrappers
therefore cannot replace an active operational dig. Survival dig also checks the
canonical selected slot: a pending click/SWAP/transfer that can change that hand
still conflicts; a storage-content or other-slot click does not. Unresolved
crafting/cursor return that can choose hand slots remains a conflict. Atomic
`use_item_expect` retains its existing strict admission to protect its explicit
item promise; ordinary body UseItem remains best-effort. On 1.16.1 the actor-owned
writer also receives the private domain tag: it retains the original inventory
transaction owner while admitting independent packets. Strict native dispatch
still uses the original all-family veto, and generation/observation/lifecycle,
writer failure and disconnect fences are applied before either policy.

## Bounded receipts without changing strict records

```rust,no_run
# async fn example(client: voxrig::Client) -> voxrig::Result<()> {
use voxrig::{body::{BodyConfig, HeadlessBody, ReceiptStatus},
    client::inventory::{InventoryClickButton, InventoryClickSource}};
let body = HeadlessBody::new(client.clone(), BodyConfig::default())?;
let click = client.survival().click_inventory(
    InventoryClickSource::Player, 9, InventoryClickButton::Left).await?;
let watch = body.watch_inventory_click(click.id).await?;
let outcome: ReceiptStatus = watch.status().await;
# let _ = outcome;
# Ok(()) }
```

`InventoryClickWatch` is additive and bound to the original retained click ID.
Its absolute deadline starts when the watch is created, including initial
capture, using `BodyConfig::action_timeout`. Clones share that deadline and
outcome. Polling performs one bounded inspection; no background polling task or
caller loop is required to enforce the deadline. Each inspection also has the
`settle_window` read budget. Missing evidence or a blocked read yields Unknown.

Accepted requires a complete write and fresh actual source/cursor values matching
the prediction, plus the original legacy transaction reply. The legacy reply's
`accepted=false` is a comparison/resync result, not evidence of rollback. Actual
contradictory source/cursor receipts report Corrected, even after Unknown. Late
agreement leaves Unknown. Superseded IDs, changed world/mode/screen, or unavailable
registry context produce Unknown. Accepted/Corrected remain historical results.
The original strict record and its latched inspection reasons are unchanged;
Unknown does not authorize a conflicting same-domain send. Opting into unrelated
body work can still invalidate a stricter diagnostic: for example, changing the
hotbar can leave a modern click's original held-context check at
RequiresInspection. The watch independently classifies actual source/cursor
receipts and never erases that strict diagnostic.

Body actions already have an absolute deadline from submit and return Unknown
when evidence is insufficient. New `HeadlessBody::action_outcome(id)` can inspect
retained original evidence after Accepted/Unknown and return a late Corrected rollback
notice, without reopening control or replaying. Gaps or changed worlds retain
the original outcome. This facility is bounded by the existing action/evidence retention.

## Compatibility and exact behavior changes

No existing strict receipt API or receipt stage semantics change, including
`click_inventory`, inventory record readers, container open/close, mining,
`dig`, `begin_dig`, finite strict motion and the all-family mutation admission
used by them. Golemkit can keep using them unchanged. The new receipt watch does
not turn a strict RequiresInspection record into ObservedClicked.

On 1.16.1, `HeadlessBody::use_item_expect` now checks the common strict
admission before its existing strict writer policy. It can therefore refuse a
retained common receipt even when no raw writer owner is present. This explicit
item-promise path is deliberately conservative; `Client`/`Survival`/`Creative`
strict item-use APIs are unchanged. Ordinary body UseItem uses domain admission.

Existing body/best-effort paths now admit unrelated domains as listed above.
Body correction classification is also domain-specific: inventory resync no
longer marks move/look/chat/hotbar selection Corrected; it still affects UseItem.
Hotbar correction affects SelectHotbar, and pose correction affects spatial
body actions rather than chat/command/selection/item-use. Full contents on
container activation remain ordinary observations. `BodyBackend::action_status`
keeps its immutable terminal contract; the new `action_outcome` view is the
explicit opt-in for late rollback notices; it does not change existing terminal
events or `action_status` values. Body's single active action/control
lease contract remains unchanged; a receipt watch owns no movement lease.

Step 7 separates Body dig admission from timing estimates; see [Body dig](design/body-api.md#拒否削減-step-7-採掘の見積りと送信を分離). Strict timing/receipt APIs remain unchanged.
No entity contact proof types are added.
