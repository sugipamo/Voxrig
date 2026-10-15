# Private contact capability audit (Step C, 2026-10-15)

This policy covers the existing owned client contact lane in Java 1.16.1 and
1.21.11. It does not qualify complete entity ticks, AI, server travel, damage,
vehicle simulation, or public geometry APIs. The six modeled types remain
Cow, Pig, Zombie, Sheep, Creeper and RemotePlayer. The WIP's extra species
were not established by matching a few inherited methods and are withdrawn.

`Qualified` means the existing reducer implements the relevant effects within
that lane, subject to its existing input guards. `ProvenIrrelevant` means the
effect cannot enter that lane. `BoundedUnknown` means only a finite subset of
the dispatch has been audited; the rest remains unsupported. It never means a
spatial bound or permission to skip a distant body. `Unknown` is the default.

| Types (both versions) | Walking outgoing | Incoming pushability | Solid | Geometry | Metadata callback | Status | Animation |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Cow/Pig/Zombie/Sheep/Creeper/RemotePlayer | Qualified | Qualified | ProvenIrrelevant | Qualified | Qualified with existing guards | BoundedUnknown | BoundedUnknown |
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
the private Rust table plus docs/evidence/contact-capabilities-20261014.json.
The evidence also binds this review, metadata inputs and Step A packet audit.
No original bytecode, jars or mappings are published. Fingerprints make the
manual side-effect review traceable; they do not automate semantic proof.
The original owner numerical fixture remains an additional finite regression
source (including sheep/creeper overlap, native slots, baby metadata, boats and
exclusion probes), not proof that all WIP species are qualified.

The WIP interpolation hull lacked a proof of indirect and nonlocal effects.
It has been removed; Unknown/BoundedUnknown bodies always refuse before a
prediction can be sent. No deferred relevance mechanism is extended (Step B).
A lifetime's refusal retains its original Spawn trigger, including after later
metadata, mount or motion receipts, because that unsupported spawn remains the
cause. Removal and native-ID reuse re-evaluate the new lifetime independently.
History/provenance/lifetime loss stays Permanent, missing received profile/game
mode stays Transient, and unsupported capabilities stay Unsupported at their
construction sites. No reason-string parsing or public schema changes occur.
