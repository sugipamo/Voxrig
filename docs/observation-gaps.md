# Item, posture and fluid observations

These APIs work with either selected adapter (Java 1.16.1 or 1.21.11), without
the `native` feature. Receipts are last-known packet values, not a promise of
server-current state.

## Entity item stacks

`Client::entity_item_stacks()` captures explicit item-stack metadata accessors
for surviving spawn lifetimes. Each row has an `EntityId`, native metadata
`index` and `ObservedValue<SlotKnowledge>`. For `minecraft:item`, the accessor is
7 in 1.16.1 and 8 in 1.21.11. Match `SlotKnowledge::Item { item }` to read its
version-bound ID, namespaced name, count and `ItemData`; `item.custom_data()`
decodes supported custom data. Legacy NBT and modern added/removed component
patches retain their original bytes, using the inventory item representation.

Missing rows mean no explicit stack receipt. `Empty` is an explicit empty stack;
`Unavailable` replaces an older value when a modern stack cannot be followed.
Removal, world reset and entity ID reuse retire the previous lifetime's stacks.

The existing exhaustive `EntityDataValue` enum and `EntityObservation` struct
are unchanged: their compatibility metadata projection still uses `Other` for
item-stack serializers. Use this additional observation for decoded stacks;
join observations by the complete `EntityId`, and compare their capture ordinals
when coordinating separate calls.

## Dig inventory prerequisites

`Survival::dig` and `dig_estimate` need the selected main-hand slot, including
an explicit `Empty` receipt for hand digging. They do not require unrelated
inventory slots or the cursor. Only legacy underwater mining also requires
slot 5, because helmet Aqua Affinity changes mining speed. Modern underwater
mining uses the received player attribute model.

Inspect `Error::dig_refusal()` for `DigRefusal::MissingSelection` or
`DigRefusal::MissingSlot { slot }` (player-screen slot numbers). Both have
`ErrorKind::State`. Missing and unsupported stacks remain unknown; an absent
helmet is never interpreted as an empty helmet. Existing world/hand/timing and
dispatch receipt requirements still apply.

## Pose and eye height

`Client::player_posture()` captures the own player's exact native pose and shared
swimming flag with independent `Received` ordinals. It does not substitute the
client model's standing reset pose for a missing receipt. `entity_pose()` resolves
the native pose using the selected version, keeping unknown IDs readable.

`unscaled_eye_height` exposes the pose constant even without a modern scale
receipt. `eye_height` calculates effective height above feet from explicit pose and, for modern
scalable poses, explicit scale. Its height is `Predicted`, with original pose
and scale inputs retained alongside it. Missing/invalid inputs produce `None`.
`EntityPose::player_eye_height(version)` also exposes the unscaled player pose
constants without requiring a connection: standing 1.62f, crouching 1.27f,
swimming/crawling, Elytra and riptide 0.4f, sleeping 0.2f. Dying uses the original
fixed-dimension player behavior; other valid modern poses use the standing
fallback. Values preserve native float arithmetic.

`EntityObservation::eye_height()` exposes the existing qualified
player/cow/pig/zombie dimension models with `EntityGeometryEvidence`, including
baby and scale inputs. It has the same explicit unavailable reasons as
`derived_geometry()`. The original methods are checked in 931 oracle cases
across all native poses, baby states and modern scales.

## Water and block properties

`Client::block_state`, region observation and `capture` already return complete
`NativeBlockState.properties` from the selected palette. `ObservedBlock::properties()`
and `property(name)` provide convenient access, preserving unloaded `None`.
For example, `property("waterlogged") == Some("true")` identifies a waterlogged
slab/stair, and water's `property("level")` retains the original level 0..15.
All other known palette properties, such as slab type and stair facing, remain
available. This is the received world projection; it does not invent local
placement outcomes.

Neither protocol provides an own-player `in_water` boolean. The received
shared swimming flag in `player_posture()` and remote
`EntityObservation::received_swimming()` are available independently of pose.
A swimming pose can also mean crawling; it does not prove water intersection.
`ControlFrame::in_water_observation()`, `swimming_observation()` and
`eye_in_water_observation()` return explicitly `Predicted` model values for
comparison. The existing plain numerical fields remain available.

The legacy `Client::eye_in_water()` convenience method retains its documented
standing-eye fallback outside a running control session. Use the new posture
and native block properties when a received-input comparison must account for
crouching or crawling.

## Local validation

Build `climbing_control_probe` with `--features native`, then run
`scripts/run_observation_gaps_live.py --accept-eula --binary <path> --jars <dir>`.
The existing runner verifies official JAR hashes, creates disposable loopback
servers, records original protocol bytes and uses RCON as an independent oracle.
It covers item identity/custom data/removal, selected-hand digging, waterlogged
slab/stair properties, water levels, received standing/crouching and swimming
metadata, and explicitly predicted fluid values.

Pass `--sparse-inventory --version 1.16.1` for a separate receipt-loss trial.
The test proxy withholds complete original inventory baselines while forwarding
all other frames unchanged. Real `give`/`clear` commands establish only the held
slot through original single-slot updates. The test requires a typed missing-slot
refusal before that receipt, then successful digging with unrelated slots and
cursor still unknown; withheld frames are recorded as undelivered evidence.
