# Registry-wide contact exclusion (2026-10-25)

Decision A remains the contract: a complete received model origin and explicit
numerical ticks. There is no claim about real-time server positions, AI, packet
freshness, unreceived movement or teleports. Numerical contact qualification,
strict receipt semantics and public APIs are unchanged.

This supersedes the *type eligibility* limits of the #250 regression review for
player movement scope. The independent capability/packet audits, their original
method fingerprints and the original default-spider provider remain intact. The
new proof is independent of those providers and never turns policy pass-through
into geometric evidence. Boat/independent numerical APIs retain their policies.

## Official inputs and generated classification

`run_entity_scope_oracle.py` verifies both original server JARs and mappings,
compiles our harness against a remapped copy, reverse-remaps only the harness,
and executes it with the unchanged original server classes. Every official
EntityType registry entry is enumerated: **107 in 1.16.1, 157 in 1.21.11**.
Player and the legacy fishing bobber use their actual specialized constructors.
The checked-in oracle contains results and hashes, with no game bytecode.

The harness evaluates every native pose and adult/baby dimensions; native slime
and phantom size domains; salmon enum variants and all puffer states; polar bear
standing animation; sniffer states; painting variant dimensions; scale bounds;
and native pushable/solid predicates. `generate_entity_scope.py` independently
verifies the official client JAR/mapping hashes and resolves all registry classes
and their ancestors. It records every original method fingerprint, then emits a
private sorted Rust table. No list of registry names selects exclusion eligibility.
The generated table is checked against both the oracle and bundled registries.

Dimensions preserve native float results. A full width/height on both sides of each anchor and one block
margin enclose native rounding, the 0.2 pair selector and dimension refresh
anchoring. Adult/pose maxima cover smaller baby/sitting/swimming forms. Polar bear
standing doubles the height; the original animation is driven to its bound by the
oracle. Shulker's original axis-directed opening extends its box one block, so
its unit envelope uses two blocks on every axis; maximum scale includes that
extension. Only block-position fields whose native callback actually places feet enlarge
the hull: generated legacy shulker attachment indices and the existing bed /
shifted-target placements. Turtle home/travel targets belong to server AI; they
do not relocate client feet or bridge a far envelope to the origin.

Integer-size variants use generated cases for the **received** size, retaining
all earlier boxes in the monotone hull. Modern Slime clamps to 1..127 and Phantom
to 0..64. Legacy Slime's setter has no upper clamp: its positive int domain reaches
2,147,483,647, producing approximately 1.117 billion block dimensions. Cases
outside the small generated domain use the recorded maximum, rather than an
invented size cap. An ordinary small slime therefore does not acquire the maximum
box before any such size is received. Valid non-unit scale expands to the native
maximum (16 in modern); it still does not qualify numerical contact nearby.
Cloud radius and Interaction dimensions can contain arbitrary native floats;
those bodies' positive no-contact proof is independent of those dimensions.
A relevant rider cannot inherit an isolated carrier's no-contact proof.

## Position paths, side effects and near refusals

For contact-relevant remote living entities the nonauthoritative client lane
interpolates inside the retained received endpoints. Species AI/input/velocity
changes do not establish authoritative local travel in that lane. The reviewed
species tick wrappers, geometry refreshes, rider hooks and callbacks are included
in the all-ancestor fingerprints. Shulker block snapping/peek stays within the
expanded shape and received anchors. Remote boat/raft and ordinary minecart
interpolation is also convex. The hull retains constructor, raw absolute,
codec, interpolation and collision-query endpoints and never shrinks.

`MOVE_MINECART` remains numerically unsupported. The adapter now privately
retains the hull of every actually decoded weighted path endpoint, including
previous packets, after checking the complete original packet framing and finite
nonnegative weights. Positive weighted interpolation stays in that hull. A far
path is excluded; a path crossing the player refuses with typed entity/packet
context. There are no fabricated motion or metadata history entries and no
synthetic record ordinal. Removal/ID reuse ends the private path lifetime.

Positive no-contact classification requires native constant-false pushability
and null/false solid collision methods, together with the side-effect review.
This includes items, arrows/projectiles, falling blocks, TNT, paintings/frames,
clouds, displays and other generated entries. Their own movement need not be
simulated to exclude them from this contact lane. Projectile hits, block
placement, explosions, pickups and damage are server work or separate actual
incoming packets; original client callbacks here do not invent local contact
impulses. Living bat/armor stand proofs remain unchanged. The evidence records
28/38 positive table classifications, alongside the existing independent proofs.

Two native exceptions prevent a blanket non-pushable rule. Fishing hook status
31 can pull a hooked local player in modern; an attached firework can change a
local player's gliding velocity. The adapters retain their actual target fields
privately, while public metadata values and provenance remain unchanged. An
unattached instance has positive irrelevance evidence. An attached instance adds
the target's receipt envelope, with exact target lifetime, and refuses when that
effect reaches scope. All observed target lifetimes stay in the monotone effect hull, even after a
clear. Missing targets, ID reuse or more than 64 distinct targets per carrier
lifetime use the conservative global bound, never a made-up target position. An attachment to the local player is
necessarily relevant even if the carrier's last received coordinate is far away.

Whole passenger groups still select atomically and include conservative native
attachment offsets and shifted interpolation targets. An autonomous noncontact
carrier with a relevant rider additionally uses a deliberately loose recurrence
`distance(t) <= (|received velocity|_1 + 64) * (2^t - 1)` for omitted explicit
ticks: native projectile/item/gravity acceleration and drag fit that per-step
upper bound. This is numerical time, not elapsed server time. It is capped by the
global finite fallback below; unresolved geometry uses that fallback immediately.
Before dismount/removal splits a group, the omitted-tick travel envelope is
retained on surviving members; those members cannot re-enter as stationary
identity steps. It does not certify near passenger travel. Complete geometry/relationship
history is still mandatory and genuinely lost history remains refused.

Near unsupported contacts retain `MovementRefusal::Scoped` with the entity type,
packet/sequence, last received feet, block and chunk. Their mere type at a far
coordinate no longer rejects selection. Contact propagation closure, actual
terrain-query and before/after-sweep guards, owner lifetime, numerical contact
budgets and strict no-replay/acknowledgement semantics remain in force.

## Unknown registry IDs and conservative global bound

The fallback extent is **32 × f32::MAX**, approximately **1.0889 × 10^40 blocks**
on each axis. This intentionally enormous finite bound encloses every finite
native float dimension at scale 16 (including diameter and rounding margin) and
the legacy int slime domain. An unknown registry ID has no official constructor,
shape or positive irrelevance proof: its position receives that global-size
box, and it refuses only if this envelope reaches the attempted player scope.
No unknown ID becomes an empty hitbox or gains a guessed default mob size.

Consequently all legal vanilla world coordinates are inside an unknown ID's
fallback. This is deliberate: a small useful universal radius cannot be proved
from an unknown shape, particularly with the legacy size domain. IDs with known
generated geometry use their own actual envelope instead. This bound is a
fail-closed fallback inside the pinned vanilla/native numeric contract; it does
not promise support for arbitrary modded geometry, nonfinite packets or missing
relationship history. The unit test explicitly checks the finite outside-bound
case without claiming that such coordinates are legal gameplay positions.

## Reproduction and evidence

```
python3 -B scripts/run_entity_scope_oracle.py --downloads /path/to/originals --work /tmp/entity-scope
python3 -B scripts/generate_entity_scope.py --downloads /path/to/original-client-inputs --work /tmp/entity-client-audit
python3 -B scripts/generate_entity_scope.py --check
cargo test --locked --lib client::entity::scope::tests -j2 -- --test-threads=2
cargo build --locked --features native --example climbing_control_probe -j2
python3 -B scripts/run_contact_scope_all_types_live.py --accept-eula --version 1.16.1 --binary target/debug/examples/climbing_control_probe --jars /path/to/originals
python3 -B scripts/run_contact_scope_all_types_live.py --accept-eula --version 1.21.11 --binary target/debug/examples/climbing_control_probe --jars /path/to/originals
```

The full checks and original-server results are recorded in the accompanying
live evidence. The walking fixtures contain varied far and nearby non-touching
sets at approximately 40 and 18 blocks (including actual spider, slime, bee, golem, phantom/bat, squid, boat and
armor stand), without falling blocks. Touching spider still refuses typed before
admitting a new controlled owner. The same connection resumes after removal.
