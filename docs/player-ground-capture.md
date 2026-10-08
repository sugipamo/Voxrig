# Player grounding capture validation (#25)

Contract: [player-capture-ground.md](player-capture-ground.md). `on_ground` and
`rotation_source` are captured with the local position and inventory. Missing
knowledge remains distinct from false; no own-player ground value is presented
as received. Ground facts belong to the existing native model under the adapter capture gate
and are invalidated even by an identical-coordinate correction.

The 2026-10-08 run used unmodified SHA-1 verified official 1.16.1 and 1.21.11
servers, disposable flat worlds, offline players, loopback-only Minecraft/RCON,
and the common-only `climbing_control_probe`. Each version passed 10 checks:

- Missing ground after a received correction; received rotation provenance.
- Standing, jumping, landing and stopped-control captures, retaining true/false
  as `Predicted`. For each, an actual original complete serverbound movement
  frame matched the captured position and ground bit. Later independent RCON
  reads also observed the server's OnGround flag (1b/0b/1b/1b) and position.
- Submitted look with independent rotation provenance; the historical received
  rotation stayed unchanged. Both adapters preserved missing model ground after the new correction;
  the ground bit submitted by stationary look did not invent a model tick.
- Same-coordinate teleport cleared old ground knowledge.
- Real death and native respawn changed the world generation on the same
  connection; the subsequent pose did not inherit the old ground value.
- Revoking the client refused subsequent player capture.

The RCON reads happen later; they are independent checks of the fixture stage,
not part of the SDK capture or evidence of an own-player ground receipt. Modern
stationary look requires a received zero-velocity pose or a settled finite-motion
contract; the fixture restores a received baseline after continuous control.
Neither stopping input nor matching RCON establishes indefinite server contact.
These short functional runs do not resolve application endurance issue #5.

An additional ordered TCP regression applies the native respawn packet and seeds new-world geometry
before the new own pose. The unguarded SDK reproduced model ground publication
for an unavailable position. The final guard suppresses native physics updates,
old-position movement packets and ground capture until the own pose arrives.

[Evidence](evidence/player-ground-capture-20261008.json) contains captured fields,
matched original frame records, official server identities and hashes of the
complete local reports, traces and executable. Both traces have zero errors. The SDK source tree and commit identify the final tested code, with #26 preserved
as its baseline. The follow-up pre-pose guard passed the ordered regression and
this repeated real-connection run. Probe source is hashed separately.
An earlier modern fixture tried stationary look directly after continuous
control and was refused by the existing admission contract; it is excluded from
the passing runs. All owned fixture processes were stopped/reaped.

The SDK regression tests exercise missing vs false/true, legacy native physics,
relative movement, native look, identical corrections, world reset, controller
publication and capture after disconnect. This fixture extends verification to
actual jump/landing, server ground reads, native respawn and revoke on both releases.

Reproduce with retained jars and Java 21:

```sh
cargo build --locked --example climbing_control_probe
python3 scripts/run_player_ground_capture.py --accept-eula \
  --binary target/debug/examples/climbing_control_probe --jars /path/to/official-jars
cargo test --locked --lib player_capture_ground_and_rotation --features native
```
