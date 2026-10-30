# Atomic inventory views

Both Java 1.16.1 and 1.21.11 expose these passive reads on the common client,
including Survival and Creative handles:

```rust,ignore
pub async fn Client::capture_inventory_view(&self) -> voxrig::Result<InventoryViewCapture>;
pub async fn Survival::capture_inventory_view(&self) -> voxrig::Result<InventoryViewCapture>;
pub async fn Creative::capture_inventory_view(&self) -> voxrig::Result<InventoryViewCapture>;

impl InventoryViewCapture {
    pub fn player(&self) -> &PlayerObservation;
    pub fn screen(&self) -> &ScreenObservation;
    pub fn snapshot(&self) -> InventorySnapshot;
}
impl ScreenObservation {
    pub fn active_screen(&self) -> ActiveScreen<'_>;
}
impl InventorySnapshot {
    pub fn basis_matches_current(&self, capture: &InventoryViewCapture) -> bool;
}
```

The adapter holds its receive gate/state lock while copying the player inventory,
local player-screen access, active window, cursor and container screen. The two
observations have exactly the same session and receive_sequence. The sequence is
an applied receive boundary, not a server tick or proof that every constituent was
updated in the latest packet. Original field provenance and unknown values remain
intact. Later receives cannot modify this owned capture.

The entire read, including lock acquisition, has a five-second budget. Expiry is
`Err` with `ErrorKind::Timeout`, separately from a false basis comparison or an
unknown screen selection. Other state/connection errors remain ordinary `Err`.
There are no packets, retries, reconciliation, write admission or owner release.
Existing `player_state()` and `screen_state()` retain their individual semantics.

`ActiveScreen` and `ActiveScreenUnknown` live in `voxrig::client::container` and
are non-exhaustive. `ActiveScreen::Open(&ContainerScreen)` means the active
numeric window matches the received screen, with no local close contradicting
it. `Closed` means either the submitted close targets exactly that screen's
ScreenId (including connection, world and opening sequence), or there is no
container and received window zero is active. It does not prove player-screen
write readiness. `Unknown(reason)` distinguishes:

- `CloseTargetsDifferentScreen { closed, observed }`: a local close targets a
  different opening, including numeric ID reuse, or there is no retained screen.
- `WindowMismatch`: the numeric window differs from the retained screen.
- `MissingScreen`: there is no retained screen and window zero is not established.

After a successfully dispatched close for the exact retained ScreenId, a fully
decoded window-zero contents packet retires that historical screen on both
versions. The resulting observation has received window zero, Received player
access and Closed selection. A close for another opening, a slot-only update or
contents without a matching close cannot retire the screen. No server close is
synthesized; the original close record remains a local dispatch fact.

Close-target comparison takes precedence over the window check. Selection uses
only this observation. Inventory history's `from_inventory` shares this exact
implementation, selecting only Open. Ordinary click admission uses the same
selector (Closed for player clicks, Open for container clicks), in addition to
its existing access, slot, cursor, mode and debt checks. Closed/Unknown do not contribute container
slots to a snapshot; the independently observed canonical player slots remain
available. A snapshot's Player source alone does not assert screen readiness.

The basis comparator shares the operational continuation's session, source
(including opening identity), and slot/cursor value comparison. Identical-value
receipts and unrelated entity packets do not invalidate it. This is a planning
comparison, not a complete continuation admission predicate: mode, selection,
response debt and other owner constraints still apply. Check screen selection
before planning. The connection can change again immediately after the capture.
The mo12 inspected recovery uses stricter full-content/cursor receipt ordinals,
coverage, inspection-owner identity and discharged debt, so this value comparator
does not replace its existing admission implementation.

```rust,ignore
use voxrig::client::container::ActiveScreen;
let cut = client.survival().capture_inventory_view().await?;
match cut.screen().active_screen() {
    ActiveScreen::Open(screen) => plan_from_table(screen, cut.player()),
    ActiveScreen::Closed => plan_from_player_inventory(cut.player()),
    ActiveScreen::Unknown(reason) => return inspect(reason),
    _ => return inspect_future_selection(),
}
if let Some(basis) = outcome.basis.as_ref() {
    if !basis.basis_matches_current(&cut) { return replan_or_inspect(); }
}
// Retain the result before planning another write. Native admission still applies.
let continued = client.continue_operational_crafting(outcome.send).await?;
```

Compatibility: screen inclusion in history is unchanged (the previous rule also
excluded every SubmittedClose and every mismatched window). The operational
comparison additionally rejects a changed session/source even if all compared
values happen to be identical; this can only add inspection/rejection. No strict
crafting verdict or CraftingTakeRecord meaning is changed.
