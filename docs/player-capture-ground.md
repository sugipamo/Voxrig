# Player ground and rotation at capture

`Client::player_state()` and `Client::capture(region)` expose position, rotation,
`rotation_source`, and `on_ground` at the same adapter lock boundary. Keep the
containing `SessionStamp`; `receive_sequence` is a receive ordinal, not a local
physics revision. Two observations with the same ordinal can have different
locally predicted positions, submitted rotations, and model ground flags. Do not
join separate observations by receive ordinal to manufacture a coherent pose.

`on_ground: Option<ObservedValue<bool>>` describes the owning native model's latest
supported flag. `None` means unavailable, including before a supported model
update and after own-position correction or world reset. `Some(false)` is a
known airborne model flag. The flag's source is `Predicted`: own-player position
packets in these adapters do not supply a ground bit. Sending a ground bit does
not convert this model value into a server receipt or certify executable motion.

Java 1.16.1 publishes its normal collision-physics result, finite movement model,
declared landing model, and continuous controller result. While the own position is unavailable (including the respawn boundary), native
physics does not reuse preceding-world coordinates or dispatch movement; capture
keeps ground unavailable. An incomplete collision
update clears the flag instead of presenting the old value as current. Java
1.21.11 publishes finite movement and continuous controller results; without one
of these model updates it keeps ground unavailable. No new physics runs merely
because an observation is requested. Flying submissions clear ground rather
than inventing an airborne collision result. Local model state may remain after
a controller stops; it is a snapshot, not a promise that prediction continues.

The numeric `rotation` field remains compatible. Its independent
`rotation_source` is absent before a supported update in the current world,
`Received { sequence }` after a decoded own-pose correction, `Submitted` after
local look/movement input, or `Predicted` for a computed face-target direction.
Ordinary position physics does not overwrite the rotation source. Equal values
do not imply the same source. Native operations and common operations update the
same provenance; changing handles does not create another pose or connection.

Respawn/reconfiguration invalidates model ground and rotation provenance. Closed
or revoked clients reject current captures; a saved observation remains historical
data. Packet replay can retain received correction rotation but cannot reconstruct
model ground from an outbound ground bit or an incomplete packet trace.

Consumers own navigation, protections and missing-data policy. They must not label
the flag `Received`, default missing ground to true/false, reimplement SDK collision
physics to fill it, or treat a predicted ground flag as server action authority.

Real-connection validation and reproduction: [player-ground-capture.md](player-ground-capture.md).
