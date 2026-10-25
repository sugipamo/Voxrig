# Player contact scope (issue #246)

`PlayerControl::start_control`, held flight, glide, and each subsequent physics
frame use this boundary on **Java 1.21.11 and Java 1.16.1 native**. The implementation
is shared. Finite numerical contact APIs and boat control retain their previous
budgets and receipt policy.

## Declared guarantee: model origin and explicit ticks (decision A)

This is a guarantee inside the existing [owner contract](common-entity-owners.md)
and [envelope audit](contact-envelope-audit.md). The origin is the complete
received constructor/packet history; progress is the owner's explicit numerical
ticks. Neither packet ordinals, SDK application timestamps nor wall time establish
the server's current entity position. **This is not a proof about the real-time
server position, server AI, packets still in flight, or unobserved teleports.**
No elapsed server ticks or freshness bound is inferred from a 50 ms timer.
Constructor age/pose/scale defaults are explicit native model inputs, not guesses
about unreceived server metadata.

A separate receipt reducer retains a monotone possibility hull, independently of
numerical travel and terrain loading. It covers every admitted spawn/codec anchor,
raw teleport endpoint, remaining interpolation endpoint, clamped placement,
metadata bed placement, native dimensions, and rounding/contact-query margins.
The [all-registry review](contact-scope-all-types.md) extends finite exclusion to
every official EntityType: 107 in 1.16.1 and 157 in 1.21.11, generated from the
official JARs. Pose/age/variant/scale bounds, native no-contact predicates and
received interpolation/attachment endpoints determine the envelope. Nearby
unsupported numerical contact still refuses typed. An unknown registry ID uses
the documented conservative global bound, rather than an invented ordinary size.
Malformed numeric/provenance inputs and incomplete relationship history still
fail closed. Policy pass-through is never reused as exclusion evidence.

Positive class proofs (for example Bat and ArmorStand) remain valid: a connected
group made entirely of proven no-contact classes needs no geometric or velocity
sample. A mixed group with a potentially relevant rider requires every member's
finite group envelope; an unresolved rider is refused. No zero-size box is
fabricated. The original default Spider/CaveSpider provider and its revocations
remain unchanged for independent/boat APIs. Player scope now has an independent
generated proof that includes received spider metadata and maximum dimensions.
The all-registry review also documents the private weighted minecart-path hull
and the fishing-hook/firework target exceptions; neither produces synthetic
public receipt records.

The initial player broad phase encloses its box, momentum, ordinary input and
step alternatives. Every actual attempted physics cell is checked again before
terrain caching, as is the before/after swept hull. These guards cover larger
attribute values, held inputs, flight and collision alternatives; a staged frame
which escapes the initial selection is refused before any movement write.
Incoming pair impulses have a conservative 0.1 block per selected body allowance.
Selection closes to a fixed point across touching/overlapping entity envelopes,
so an indirectly relevant entity cannot be dropped by an own-distance test.
The native map/list slots of excluded entities remain in the original order;
only numerical bodies and their terrain queries are omitted.

An excluded body which becomes relevant after an omitted tick **stops the owner**.
Skipped modern collision/interpolation decisions are not reconstructed from a
latest target. New received lifetimes can be admitted at their receipt boundary.
The entire selection and numerical frame are staged before movement I/O.

## Carrier/passenger proof

The relationship graph uses connection/world/spawn lifetimes and ordered original
passenger receipts. Every connected group has one possibility hull; a carrier
and rider never run as independent ordinary bodies. Missing lifetimes, duplicate
parents, cycles and numeric ID reuse cannot establish a finite group.

For the existing qualified nonauthoritative living species, the audited originals
have no local-player movement authority. `Entity.rideTick` clears delta movement,
executes the remote client lane, then attaches the rider to the carrier.
`LivingEntity.rideTick` adds animation/fall bookkeeping. This is exclusion evidence,
not qualification of passenger travel in the numerical contact engine.
Legacy Player's shift-triggered early dismount leaves its feet in the existing
hull (`removeVehicle` changes membership/cooldown, not placement); its remaining
ride branch adds bob/statistics bookkeeping. Modern Player gates that dismount
branch to the server, and AbstractClientPlayer only resets bob. Both alternatives
remain in the coupled possibility group rather than fabricating independent travel.

In 1.16.1, inherited positioning uses carrier height * 0.75 plus the rider offset;
Chicken adds yaw-rotated 0.1 horizontal displacement and uses half its height.
Qualified overrides (including Zombie/Player offsets) fit the deliberately loose
per-node allowance `4 * (adult width + adult height + 1)` on every axis.
In 1.21.11, the original PASSENGER and VEHICLE attachment points, baby scaling and
yaw rotation fit the same allowance, evaluated from that version's dimensions.
The factor four covers incoming/outgoing attachment contributions at a nested
node and their before/after differences in shifted targets. The group sums these
allowances across the complete nested graph and encloses
all member receipt hulls. An additional coordinate diameter on each axis encloses
translated interpolation targets `target + attached_position - anchor`.
Nearby or touching groups are always refused with a typed cause; no passenger
travel is simulated or certified safe.

Mount/dismount first retains possible attached placements, then changes the whole
graph. Metadata, motion, death/removal and ID reuse update the same receipt state
before selection; removed lifetimes are dropped, never resurrected by numeric ID.
World changes reset the proof. Eviction checkpoints carry the complete proof and
relationships into a fresh origin. A running owner with a cursor gap stops rather
than rebootstrap across lost model ticks. Unknown relationships cannot be repaired
by a distance threshold. Original method owners/fingerprints are recorded in
[evidence](evidence/player-contact-scope-20261024.json).

## Budgets, diagnostics and restarting

`ClientLimits::max_contact_bodies` (and native `ConnectionOptions` of the same
name) defaults to **256 remote bodies**. Only the current propagated scope is
counted; zero permits movement with no scoped remote bodies. Increasing the cap
admits a larger numerical scope, without increasing any independent public
numerical model's fixed budget. Received identity/history storage still has its
own bounds. `max_chunks` now defaults to **512**, covering the 289/361/441 chunk
squares at view distances 8/9/10; larger distances can use the existing setting.

`MovementRefusal::Scoped` wraps the original recovery class without changing the
constructible `MovementRefusalContext`. `context()` retains entity ID/type,
receipt sequence, owner and tick. `geometry()` adds `entity_position`,
`block_position` and `chunk_position`, including negative chunk coordinates.
For a terrain refusal these are the actual requested missing cell and the remote
model feet. For a scope/relationship refusal the cell/chunk locate the last
received entity feet; they do not claim missing terrain. Unknown identity/type or
coordinates stay absent rather than being fabricated. Display includes the cause.

A stopped owner is terminal; `set_controls_for(old_id, ...)` cannot revive it.
A **new `start_control` on the same connection** builds a fresh origin after a
blocking lifetime is removed or otherwise provably excluded, provided the existing
own-pose/momentum/world receipts still qualify. The live regression exercises this
without reconnecting. Monotone hulls intentionally do not forget a previous nearby
anchor merely because a later target is far away; an unqualified dismount,
unsupported callback or lost own momentum may therefore still prevent restart.
A per-lifetime refusal can disappear with that received lifetime. Lost relationship placement or incomplete passenger history is retained for the
affected carrier until fresh complete passenger state (including empty) or that
carrier's removal. A new owner cannot guess the missing placement. A running cursor gap always
stops the old owner; a new origin can use a complete eviction checkpoint and
continuous retained suffix, otherwise it also needs fresh observation. No automatic retry or weakened receipt
acknowledgement is introduced.

## Reproduce

```
cargo test -j2 --lib client::entity::scope::tests -- --test-threads=2
python3 -B scripts/audit_player_contact_scope.py --downloads /path/to/originals --named /path/to/named
cargo build -j2 --features native --example climbing_control_probe
python3 -B scripts/run_contact_scope_live.py --binary target/debug/examples/climbing_control_probe --jars /path/to/official-server-jars --version 1.16.1
```

The live runner uses the existing isolated loopback official-server harness with
RCON, packet forwarding, exact official server hashes and a source/binary manifest.
It receives an actual far Chicken/Zombie group and 270 far cows, verifies walking
and independent server displacement, refuses an adjacent actual group before
new controlled frames, then starts a fresh owner after tracking removal on the
same connection. The fixture establishes a fresh received neutral pose before
the adjacent admission: native maintenance can settle/repeat state after a stop,
while ControlRecord freezes its last controlled frame. The rejection must keep
that owner's dispatched count and every idle echo at the exact received neutral
coordinates. These echoes do not count as new controlled frames.

The `--features native` live [summary](evidence/player-contact-scope-live-20261024.json)
links the compressed complete report and original packet trace, source manifest,
server hash and pinned binary hash. Java 1.21.11 has unit/receipt and original-jar
coverage here; no 1.21.11 live-server result is claimed.

The 2026-10-25 [regression review](contact-scope-regression-audit.md) documents
carrier-scoped history recovery, terrain-footprint closure, retention of nearby
active states and exact stationary re-entry within the same owner. It supersedes
the blanket new-owner requirement only for those proved identity steps; omitted
interpolation/motion and final sweep failures remain typed refusals before I/O.

The [registry-wide extension](contact-scope-all-types.md) and its live evidence
cover both official server versions with varied far and nearby non-touching
entity sets, and confirm touching unsupported contacts still refuse typed.
