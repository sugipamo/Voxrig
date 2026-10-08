# Protocol 736 packet dispatch

`Bot::apply_packet_diagnosed` remains the ordered Java 1.16.1 receive boundary:
timing, coherent-state gate, receive sequence, recording, entity motion facts,
the literal packet-ID match, common operation hooks, then session limits.

Fifty multi-step handlers are separated by state ownership:

| Module | Responsibilities |
| --- | --- |
| `client/play_world.rs` | Dig acknowledgements, blocks, explosions, chunks and light |
| `client/play_inventory.rs` | Window confirmation/barriers, slots, screens and recipes |
| `client/play_entities.rs` | Entity movement, metadata, equipment, passengers and retirement |
| `client/play_player.rs` | Join, respawn, abilities, health, effects and time |
| `client/play_ui.rs` | Player list, boss bars, scoreboard and display state |
| `client/play_session.rs` | Custom payload, terminal reason and registry tags |

These are private handlers called under the dispatcher's existing coherent gate.
They do not acquire a second coherent gate or spawn work. Packet IDs, await and
guard scopes within each handler, explicit guard drops, state mutations, native
writes and event order are retained. A terminal packet returns immediately from
the dispatcher as before, bypassing post-packet hooks. Chunk/light decoding still
emits its original diagnostic event rather than converting it into a new fatal
error. KeepAlive timing and replies remain in the dispatcher.

Small parse/emit arms remain inline. No packet enum, protocol format, public API,
reader scheduling or lifecycle policy changes are part of this refactoring.
The dispatcher is now 393 lines including its existing pre/post hooks, compared
with 1,504 lines before extraction. Each complex handler can be reviewed with
its decoding, state changes and emission together.

Validation uses the existing packet, transaction, cancellation, malformed-kick,
respawn, metadata, motion and recording tests, including the disconnect race
regression from #20. The extraction was also checked against its parent: all
packet-ID arms retain their order and all fifty handler bodies retain their
original token order, allowing only slice borrowing, formatting and result tails.

After extraction, common-only real connections against checksum-verified official
vanilla 1.16.1 and 1.21.11 passed 25 climbing/waterlogged/stability checks each,
plus 16 legacy multipart dispatch/refusal checks and the modern unsupported-model
check. These cover original received world/entity/player updates and successful
disconnects; they do not claim multipart damage or full application endurance.
Original report hashes and check names are recorded in
[packet-family-refactor-20261008.json](evidence/packet-family-refactor-20261008.json).
