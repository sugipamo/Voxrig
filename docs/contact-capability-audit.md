# Private contact capability audit (Steps C/D, 2026-10-15)

This policy covers the existing owned client contact lane in Java 1.16.1 and
1.21.11. It does not qualify complete entity ticks, AI, server travel, damage,
vehicle simulation, or public geometry APIs. Step D adds Rabbit, Ocelot, Villager, WanderingTrader, Mooshroom, Silverfish
and Giant in both versions to Cow, Pig, Zombie, Sheep, Creeper and RemotePlayer.
Only the existing private owned numerical lane is extended. Public sampled
geometry APIs and complete entity simulation guarantees are unchanged.

`Qualified` means the existing reducer implements the relevant effects within
that lane, subject to its existing input guards. `ProvenIrrelevant` means the
effect cannot enter that lane. `BoundedUnknown` means only a finite subset of
the dispatch has been audited; the rest remains unsupported. It never means a
spatial bound or permission to skip a distant body. `Unknown` is the default.

| Types (both versions) | Walking outgoing | Incoming pushability | Solid | Geometry | Metadata callback | Status | Animation |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Cow/Pig/Zombie/Sheep/Creeper/RemotePlayer + Step D seven | Qualified | Qualified | ProvenIrrelevant | Qualified | Qualified with existing guards | BoundedUnknown | BoundedUnknown |
| Bat/ArmorStand | ProvenIrrelevant | ProvenIrrelevant | ProvenIrrelevant | Unknown, unused | ProvenIrrelevant to this lane | ProvenIrrelevant to this lane | ProvenIrrelevant to this lane |
| Horse/Spider/CaveSpider/Ghast | BoundedUnknown | Unknown | Unknown | Unknown | Unknown | Unknown | Unknown |
| All other registered types and unknown names | Unknown | Unknown | Unknown | Unknown | Unknown | Unknown | Unknown |

The last partial row records the WIP's inspected inherited outgoing entry
only. Tick, predicate, geometry and callback effects are not closed, so it
always refuses, including at arbitrary distance. Item, Arrow and other
projectiles likewise stay Unknown: a false pushability result alone does not
prove their tick/callback side effects irrelevant. Native slots are retained
without substituting received positions for model geometry.

## Method ownership and side effects

The generator resolves each original EntityType's class and inherited method
owners separately for each version. Player uses the actual RemotePlayer client
class, rather than EntityType's Player generic. All bytecode methods of the
reviewed classes and their ancestors, the boat selector, EntityGetter and
ItemBasedSteering are fingerprinted, including constructors,
synthetic callbacks and overloads. The following is a side-effect review of the
original `javap -c -p` output, supplementing the earlier owner/geometry/packet
oracle evidence. Owner equality alone does not feed the classifier.

* LivingEntity's outgoing `pushEntities`/`doPush`/`push(Entity)` calls are the
  existing modeled pair lane. Check alive, climbable, sleeping, noPhysics,
  shared vehicle, spectator, teams and original candidate order; incoming
  `isPushable` is a separate predicate (modern also tests spectator). RemotePlayer
  has its own constructor noPhysics and interpolation/motion dispatch; it is
  not assigned Mob behavior. Its animation wrapper adds no pair impulse.
* Mob's client AI-step delegates to LivingEntity. Looting/control changes are
  server guarded. Legacy Zombie's sunlight path calls the server-guarded
  `isSunBurnTick`; modern `burnUndead` has the same client exclusion. Zombie
  water conversion is also server guarded. Cow/Pig client AI-step uses the
  Animal/Ageable chain. Sheep decrements its eat timer before that chain;
  Ageable's forced-age branch only updates a timer/particles on the client.
* Creeper's tick updates swell, sound and (modern) game event. `explodeCreeper`
  guards the explosion/removal/effect-cloud path to the server. These wrappers
  do not add an unmodeled client contact impulse. Complete baseTick/travel is
  still outside the existing numerical owner contract.
* Geometry follows the existing audited pose, baby, sleeping-position and
  unit-scale reducers. Entity pose callback refreshes dimensions. Ageable and
  Zombie baby callbacks refresh dimensions then delegate. LivingEntity's sleep
  callback moves to the received bed position and the flags callback changes
  item-use bookkeeping; spin attack is refused. Pig's boost metadata callback
  calls ItemBasedSteering.onSynced (timer/boost fields), read by mounted travel,
  which this owner refuses. Sheep color/sheared and Creeper swell metadata do
  not alter this lane's geometry/push predicates. Unknown serializers, partial
  decode and retention gaps remain guarded, rather than assumed visual.
* Solid collision is distinct from pickability/blocksBuilding. The legacy boat
  candidate `Entity.getCollideBox` returns null in these classes. Boat's
  `getCollideAgainstBox` separately tests the candidate's `isPushable`, which is
  the incoming capability, not solid collision. EntityGetter's collision lambda
  combines those two boxes and excludes shared vehicles. Modern Entity's
  `canBeCollidedWith(Entity)` is constant false in these classes, independently
  of `isPickable`. No class/pose ancestry shortcut qualifies a new provider.
* Bat `isPushable` is constant false; `doPush` and `pushEntities` return without
  calls. Its resting tick can change its own position/velocity, but cannot
  enter either contact selector. ArmorStand is also unconditionally unpushable
  with no-op doPush. Its pushEntities only dispatches to nearby rideable
  minecarts, not walking players or boats. Its metadata refreshes marker/small
  dimensions/blocksBuilding; none changes those unconditional exclusions.
  This proof is for the existing owner lane, not remote minecart simulation.
  Unmodeled vehicles remain refused on walking; the existing boat rigid lane
  is unchanged. Geometry is explicitly Unknown rather than fabricated.
* Status and animation are independent callback capabilities. For modeled
  species, only Step A's audited sheep status 10, listener-owned status 35 and
  animations 4/5 are no-ops; all other codes refuse as typed Unsupported.
  See [packet side effects](contact-packet-audit.md). The finite audited subset
  does not qualify arbitrary entity callbacks. For the two excluded types,
  callback/geometry changes cannot turn the constant contact exclusions on;
  there is no modeled numerical body to update.

## Reproduction and limits

Run from the repository with Java 21+, Python 3 and rustfmt:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/audit_contact_capabilities.py \
  --downloads /path/to/downloads --work /path/to/contact-capability-work
```

Downloads must contain both original clients, Mojang client mappings and
SpecialSource-1.11.4-shaded.jar. The script verifies their fixed hashes,
rebuilds named clients, resolves owners, fingerprints methods, and generates
the private Rust table plus docs/evidence/contact-capabilities-20261015.json.
The 20261014 artifact remains the historical Step C input for Step B.
The evidence also binds this review, metadata inputs and Step A packet audit.
No original bytecode, jars or mappings are published. Fingerprints make the
manual side-effect review traceable; they do not automate semantic proof.
The original owner numerical fixture remains an additional finite regression
source (including sheep/creeper overlap, native slots, baby metadata, boats and
exclusion probes), not proof that all WIP species are qualified.

Step B separately proves finite default Spider/CaveSpider envelopes; see
[envelope audit](contact-envelope-audit.md). Step D extends no envelope exemption.
All other Unknown/BoundedUnknown bodies refuse globally, including at distance.
Spider/CaveSpider refuse when intersecting or unprovable and may proceed only
when the existing envelope proves nonintersection with every relevant query.
A lifetime's refusal retains its original Spawn trigger, including after later
metadata, mount or motion receipts, because that unsupported spawn remains the
cause. Removal and native-ID reuse re-evaluate the new lifetime independently.
History/provenance/lifetime loss stays Permanent, missing received profile/game
mode stays Transient, and unsupported capabilities stay Unsupported at their
construction sites. No reason-string parsing or public schema changes occur.

## Step D ordinary mob review

Review order starts with Chicken, then the requested passive species and the
additional ordinary types below. The generator resolves owners independently
on each original, exports the full ancestry fingerprints for qualified AND
rejected candidates, and records each blocker in the dated evidence. Team
ownership, dimension refresh, modern interpolation entry points and scale are
also explicit owner columns. Matching inheritance alone is never promotion.

Both versions qualify Rabbit, Ocelot, Villager, WanderingTrader, Mooshroom,
Silverfish and Giant. All inherit LivingEntity pair push, pushability and null
legacy / false modern solid collision. None overrides getTeam: Entity's UUID
scoreboard membership matches the existing reducer. None adds a contact impulse,
passenger exception, client geometry variant or alternate interpolation path.
The complete constructors, wrappers and metadata callbacks were reviewed, not
just push entry points. Existing unit-scale, equipment-completeness, spin-attack and mount guards remain
in place; unaudited status/animation values still refuse.
The existing Step A 35 and 4/5 handler proofs remain valid: status 35 and
animations 4/5 are listener-owned; no subclass callback is dispatched.

* Rabbit: Animal/Ageable chain followed by jump-animation timer changes. The
  client timer's setJumping(false) only changes jumping/sound state, with no
  authoritative travel. Jump control and variant 99 attack-goal changes are
  server AI paths; neither variant changes dimensions, teams or push predicates.
* Ocelot: Animal/Ageable chain; trusting metadata changes AI/particles, not
  geometry or team membership. Unlike Cat it has no TamableAnimal ancestor.
* Villager: AbstractVillager/Ageable chain; tick's unhappy counter and gossip
  bookkeeping do not change contact state. Profession/type/level metadata is
  read by trade, AI and display; sleeping position remains LivingEntity's
  existing bed displacement. startSleeping/stopSleeping overrides add brain
  bookkeeping, not a different received sleeping-position callback.
* WanderingTrader: AbstractVillager/Ageable chain, followed by maybeDespawn only
  on the server. The inherited baby accessor still refreshes geometry even
  though ordinary gameplay does not spawn a baby trader. It is tested rather
  than discarded as impossible.
* Mooshroom: Cow/Animal/Ageable chain. Red/brown variant metadata selects mushroom
  and stew/interaction behavior, not contact. Shearing/conversion is server or
  interaction work outside this lane; no client tick initiates it. Modern AbstractCow overrides baby dimensions: its static BABY_DIMENSIONS
  scales original EntityType.COW dimensions by 0.5f, then changes eye height
  only. Original COW and MOOSHROOM registered width/height are identical, so
  the existing half-size arithmetic is exact; native baby-boundary regressions
  check this independently. No variant changes those width/height values.
* Silverfish: Monster/PathfinderMob chain; tick writes body rotation then
  delegates. Its damage/hiding/infested-block goals are not client AI-step
  effects. It has no baby/size metadata or dimensions override.
* Giant: Monster/PathfinderMob chain, fixed registered dimensions. Eye height,
  attributes and walk-target overrides do not enter contact. LivingEntity
  isBaby is constant false; no baby/size variant is invented.

For every new type, original EntityType dimensions and LivingEntity dimension
owners supply the same arithmetic as the existing lane. Legacy ageables use
getScale (half for baby) then Entity's registered dimensions; modern ageables
use getAgeScale and default dimensions (the audited AbstractCow baby override
for Mooshroom is described above), then unit attribute scale. Sleeping
pose uses the inherited fixed dimensions. Other known nonplayer poses fall
back to those same default dimensions. Each version is audited separately;
no modern custom baby dimensions are copied to legacy or vice versa.

Legacy `Entity.refreshDimensions` has a second required geometry state:
shrinking width recenters the box, but equal/growing width retains the old
minimum and extends each maximum by the new float dimension promoted to double.
It does not move a client entity's position. The private body now retains this
box independently of feet, including through checkpoints, until an original
setPos (an actual interpolation step or received bed placement) rebuilds it.
Merely ticking without interpolation or receiving a velocity does not recenter.
The oracle exercises another sleeping/standing pair while the legacy actor is
unticking. Its legacy teleport packet-construction helper preserves both old
position AND old bounds around temporary constructor staging, so the harness
cannot accidentally recenter an anchored box before the original handler.
Modern refreshDimensions calls reapplyPosition immediately instead. Both versions'
server-only displacement branches remain outside this nonauthoritative lane.
The original sleeping -> standing Silverfish boat regression (legacy seed 49,
stage 7) exposed this distinction; it remains an exact comparison. Baby growth,
pose changes, velocity-only and no-interpolation ticks use the same reducer.
Step B's peer possibility hull additionally includes the recentered placement,
so retaining the anchored current box cannot hide a future peer contact.

The following candidates stay unqualified in BOTH versions. Geometry, incoming,
metadata and other unaudited lanes remain Unknown. Horse retains Step C's
partial BoundedUnknown outgoing annotation; that annotation admits nothing.
An incomplete side-effect or numerical proof is a blocker, even if a default
sample happens to match. These rows do not claim that implementing them would
necessarily require different numerical arithmetic.

| Type | Reason to remain unqualified |
| --- | --- |
| `chicken` | Client aiStep changes falling Y velocity; modern custom baby dimensions are not modeled. |
| `llama` | AbstractHorse client stand/eating geometry and llama size/strength/passenger hooks are not modeled. |
| `trader_llama` | Llama/AbstractHorse geometry and trader despawn/mount hooks are not closed. |
| `wolf` | TamableAnimal getTeam resolves owner/root-owner teams with version-specific precedence; reducer uses own UUID team. |
| `cat` | TamableAnimal getTeam resolves owner/root-owner teams with version-specific precedence; reducer uses own UUID team. |
| `fox` | Fox isSleeping reads species flags; reducer only handles LivingEntity sleeping-position metadata. |
| `iron_golem` | Species doPush target/random branch needs separate side-effect closure and numerical coverage. |
| `horse` | AbstractHorse tick/mount/baby owners lack a closed species proof after the historical legacy sleeping mismatch. |
| `donkey` | AbstractHorse standing/eating and chested-horse metadata/mount hooks are not closed. |
| `mule` | AbstractHorse standing/eating and chested-horse metadata/mount hooks are not closed. |
| `skeleton_horse` | AbstractHorse geometry plus SkeletonHorse aiStep/water overrides are not closed. |
| `zombie_horse` | AbstractHorse standing/eating and mount hooks are not closed. |
| `panda` | Panda rolling/sitting/on-back flags and client tick effects are not modeled. |
| `polar_bear` | Standing metadata changes dimensions outside the generic pose/baby reducer. |
| `dolphin` | Water/client movement hooks and version-specific age geometry are not closed. |
| `turtle` | Species baby dimension override and land/water aiStep are not modeled. |
| `bee` | Flight/anger/sting tick and metadata hooks are not closed. |
| `endermite` | Species tick lifetime/particles and client side-effect closure lack numerical coverage. |
| `enderman` | Species synchronized-data/teleport/target hooks are not closed. |
| `witch` | Species aiStep/item-use and metadata effects lack closed numerical coverage. |
| `vindicator` | Raider/illager tick, team and metadata overrides are not closed. |
| `pillager` | Raider/illager tick, team and metadata overrides are not closed. |
| `evoker` | Raider/spellcaster tick, team and spell metadata overrides are not closed. |
| `ravager` | Species push/stun/roar tick effects are not modeled. |
| `drowned` | Zombie subclass swim/travel/metadata overrides are not closed. |
| `husk` | Zombie subclass variant geometry/callbacks lack independent numerical coverage. |
| `zombified_piglin` | Species anger/age geometry and callbacks lack independent numerical coverage. |
| `zombie_villager` | Conversion and villager-variant metadata callbacks lack independent numerical coverage. |
| `snow_golem` | Species aiStep/environment side effects lack independent numerical coverage. |
| `skeleton` | Skeleton-specific conversion/item-use hooks lack full variant numerical coverage in both versions. |
| `stray` | Skeleton ancestry/conversion hooks lack full variant numerical coverage in both versions. |
| `wither_skeleton` | Skeleton ancestry/item-use hooks lack full variant numerical coverage in both versions. |

Every other registered type remains at its prior private classification;
unreviewed species retain Unknown. No assumptions about Mineflayer are used.

Reproduce the extended numerical fixture before the capability artifact:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/run_entity_owner_oracle.py \
  --downloads /tmp/voxrig-downloads --client-downloads /tmp/voxrig-downloads \
  --work /tmp/stepD-owner --output data/client_api/entity_owner_oracle.json.gz
PYTHONDONTWRITEBYTECODE=1 python3 scripts/audit_contact_capabilities.py \
  --downloads /tmp/voxrig-downloads --work /tmp/stepD-capabilities \
  --output docs/evidence/contact-capabilities-20261015.json
PYTHONDONTWRITEBYTECODE=1 python3 scripts/audit_contact_envelopes.py \
  --downloads /tmp/voxrig-downloads --work /tmp/stepD-envelopes
```

The last command refreshes Step B source bindings after Step D tests/docs change;
it extends neither its providers nor its proof. All commands verify the original
hashes and rebuild remaps. The original owner oracle now records 320 schedules /
3,840 stages; the SDK compares 208 qualified schedules / 2,496 stages (1,248 each
walking/boat), including all original six types. The remaining 1,344 raw stages
are unqualified observations, not an SDK guarantee. The seven additions each
have four schedules per version per scene with native order, interpolation,
velocity, sleeping-position transitions, removal/reinsertion and team rules.
Ageables also receive baby/adult metadata; all seven receive sleeping/standing
pose packets. Variant packets exercise rabbit 99/0, ocelot trust, mooshroom
red/brown and villager level changes through original handlers. Raw position,
velocity and bounds bits are compared without decimal parsing tolerance.
Fixed-size Giant/Silverfish have no
invented baby cases. Finite oracle cases supplement the owner review; they do
not replace it or establish complete NPC ticks or an autonomous client clock.
