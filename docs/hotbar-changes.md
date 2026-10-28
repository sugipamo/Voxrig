# Selected-hotbar change log (audit I5)

Both Java 1.16.1 and Java 1.21.11 support the same additive common API, with
or without `native`. Import from `voxrig::client::hotbar_changes`,
`voxrig::client`, or the common prelude. Existing events, snapshots and
signatures are unchanged.

`player_state().selected_hotbar` and `received_player_snapshot` only show the
latest value. A server-side held-slot write (1.16.1 `HeldItemSlot` 0x3f,
1.21.11 `HELD_ITEM_SLOT`) replaces it, and `EventKind::InventoryChanged` does
not say what changed. `Client::selected_hotbar_changes_after(after_sequence)`
keeps every applied selection write with its before/after and origin.

## Records

Each `HotbarChange` has its own consecutive `sequence` (starting at 1) and a
frozen `HotbarSelectionChange`:

- `origin`: `Submitted` when this connection completely wrote a serverbound
  held-slot packet (common `select_hotbar`, native `select_hotbar`, Body
  selection, native equip selection, and the automatic reassertion after
  respawn). `Received` when a clientbound held-slot packet was applied.
  Neither origin is a server acknowledgement: vanilla never confirms a
  serverbound held-slot packet.
- `before` / `before_origin`: the previous record in this log, None before the
  first record. A local inventory reset without a write (1.16.1 respawn without
  metadata) is not a record.
- `after`: slot 0..=8.
- `receive_sequence`: latest applied packet ordinal; for `Received` the
  held-slot packet itself. Packet ordinals and log cursors are different axes.
- `session`: version, connection and world generation at application.
- `superseded_submission`: for `Received` only, the cursor of the `Submitted`
  record whose slot this packet replaced with a different slot. This is how a
  caller learns that the server overrode a `select_hotbar`. The submission's own
  record is never rewritten.

Equal writes are kept. A received packet that repeats the current value (join,
cross-dimension teleport) is a server write, and `changed()` is false. It is not
a confirmation of an earlier submission, so `superseded_submission` stays None.

## Cursor, gap and retention

The log follows the same conventions as `control_changes_after`.
`MAX_RETAINED_HOTBAR_CHANGES = 256` per connection, independent of the event
queue. Start with 0 and pass `HotbarChangeLog.sequence` to the next read. A
cursor behind the retained window returns `ErrorKind::State` whose diagnostic
is `HotbarChangeGap { missed, resume_after, sequence }`. A future cursor returns
`InvalidInput`. `before` stays correct after eviction. The retained suffix is
readable after disconnect. Reads send nothing and do not acknowledge anything.

## When does the server write the selection?

From the official server bytecode (`server-named.jar`, both versions), the
clientbound held-slot packet is constructed only in:

| Site | 1.16.1 | 1.21.11 | Initiator |
| --- | --- | --- | --- |
| `PlayerList.placeNewPlayer` | yes | yes | join: the saved selection |
| `PlayerList.sendAllPlayerInfo` | from `ServerPlayer.changeDimension` and cross-level `teleportTo` | from `ServerPlayer.teleport(TeleportTransition)` | server: dimension change / cross-dimension teleport |
| pick-item handler | `handlePickItem` | `tryPickItem` | client request; the server chooses the slot |

Plain respawn does not send it on either version, so Voxrig reasserts the
previous selection itself (a `Submitted` record). Vanilla accepts every in-range
serverbound selection without replying. Voxrig has no pick-item API.

## A received slot is not proof of the server's current value

The server builds the held-slot packet from its own value at the moment it
writes it. If a `select_hotbar` is still in flight, that value is older than
the submission. The server then applies the submission after the packet is
already on its way. The client takes the packet as the latest value (vanilla
does the same), so the local selection and the server's selection now differ.
Vanilla sends nothing that would show this.

LIVE reproduced this on both official servers (1.21.11 in every completed
run, 1.16.1 in two of three runs, depending on timing): `select_hotbar(2)`
followed immediately by an RCON cross-dimension teleport. The log ended with
`Received { before: 2, after: 5, superseded_submission: Some(..) }`,
`player_state().selected_hotbar` reported 5 (Received), and RCON
`data get entity .. SelectedItemSlot` reported 2.

Read `superseded_submission` as "a server write replaced this submission
locally". It does not mean the server rejected the submission. To make the two
sides agree, select the intended slot again. No retry is automatic.

LIVE: `scripts/run_hotbar_changes_live.py` (join, settled nether/overworld
teleports, and the race above), evidence
`docs/evidence/hotbar-changes-live-{1.16.1,1.21.11}-20261028.json.gz` and,
for the 1.16.1 race, `docs/evidence/hotbar-changes-race-live-1.16.1-20261028.json.gz`. Every
original clientbound and serverbound held-slot frame matches one `Received` or
`Submitted` record, in order.
