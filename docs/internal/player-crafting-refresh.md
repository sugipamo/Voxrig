# Player crafting after foreign close (1.16.1)

The server is authoritative. A preserved nonempty player grid can become empty
while hidden (`/clear`) without a later input delta. Foreign close therefore
invalidates receipts 0..=4 before requesting real window-0 full contents through
the same no-effect mismatching click as `resync_inventory`. Missing receipts use
the existing `None` representation, including in `received_crafting()` and in
send predecessor validation. No public API was added.

`Bot::refresh_player_crafting_after_close` owns this policy. Its callers hold the
coherence gate. Local close first completes all actual cursor return steps,
sends close and publishes submitted-close access. The helper invalidates grid
knowledge before releasing the ledger's close parent, then uses the ordinary
resync executor/ledger path. Failed release leaves knowledge missing too.
Close admission refuses outstanding executor response debt. Server close first
invalidates sends belonging to the closed opening; an echo of an already
submitted close does not start another refresh. The helper never replays a
click or restores a saved stack. Admission/write has a bounded timeout and the
executor's existing receipt deadline bounds response waiting. On timeout missing
slots stay missing. New Window Items / Set Slot receipts, even before the probe
response or after timeout, immediately become canonical; timeout performs no
later blanket clear that could erase those receipts.

A fully received empty result and four empty inputs skip the probe. This was
checked against the official 1.16.1 server jar, SHA-1
`a412fd69db1f81db3f511c1463fd304675244077`, using its official mappings and
`javap -p -c` (the disassembly is retained under `.local/pr279`).

- `InventoryMenu` (`bhf`) creates its own `CraftingContainer` (`bgu`); foreign
  menus do not share its input slots. Its constructor initializes empty inputs.
- `ServerGamePacketListenerImpl.handleContainerClick` checks the requested
  container ID against `ServerPlayer.containerMenu` before executing a click.
  Foreign clicks therefore cannot populate the hidden player inputs.
- `CraftingContainer.setItem` writes the slot and invokes `slotsChanged`.
  `AbstractContainerMenu.broadcastChanges` compares slots against its saved
  values. `ServerPlayer.slotChanged` sends `ClientboundContainerSetSlotPacket`
  (`oi`) for input slots (its ResultSlot exclusion does not cover inputs).
- Normal foreign close does not add to the player grid. `/clear`, removal,
  crafting consumption and death can remove stacks, but cannot add to an empty
  grid. Server additions to empty inputs pass through the above change/broadcast
  path. The nonempty-to-empty case cannot rely on a later delta, because saved
  broadcast values can already be empty; that is why it always refreshes.

These statements concern the official server, not arbitrary server plugins.
Unknown slots are conservatively refreshed too. Armor/offhand receipts are not
invalidated by this policy. Java 1.21.11 retains the prior receipt policy in this
change; its local tests and compilation are required, not LIVE.

[Verification evidence](../evidence/pr279-crafting-refresh-1.16.1-20261027.json.gz)
contains the disassembly, final local check logs, source/binary hashes and all 19
LIVE checks. In clear cycles 3, 5 and 8 no input-1 Set Slot arrived; each recovered
from exactly one automatic window-0 probe and actual full contents response.
