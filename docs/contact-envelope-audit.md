# Conservative contact envelopes (Step B, 2026-10-15)

Step B kept the Step C capability table private and left its classifications unchanged.
`BoundedUnknown` does not mean spatially bounded. This separate proof permits
only default, unmounted Spider and CaveSpider lifetimes with complete supported
motion receipts and no unproven callback to be excluded from a movement tick.
Step 6 (Issue #128 option 3) now omits unqualified contact influence and keeps
owned control alive, including for Horse, Ghast, projectiles, Slime, Shulker and
unregistered names. Spider overlap or proof loss also falls back to pass-through.
The proof below describes the retained finite exclusion evidence; it no longer
constitutes a prerequisite for continuing control. See the
[behavior/API migration](contact-policy-consumers.md#step-6-intentional-pass-through-issue-128-option-3).
Historically, Step B left its then-existing six qualified numerical body types unchanged. No public API,
geometry qualification, full NPC simulation or numerical guarantee is added.

The current eligibility and revocation sets are generated from the private table.
They select the existing proof handler; its dynamic obligations below remain
mandatory for claiming proven exclusion, rather than policy omission. `python3 -B scripts/audit_contact_envelopes.py --check` compares the
independent evidence target sets and receipt domain with the export without jars.
`python3 -B scripts/contact_policy_export.py --check` checks this section.

<!-- contact-policy:envelopes:start -->
| Version | Profile | Eligible types | Lanes | Effects | Revocations |
| --- | --- | --- | --- | --- | --- |
| 1.16.1 | DefaultSpiderEnvelope | cave_spider, spider | OwnedWalking, OwnedGlide, OwnedBoat | WalkingPush, IncomingPush, SolidCollision | Metadata, Equipment, Scale, Passengers, Status(-128..127), Animation(0..255) |
| 1.21.11 | DefaultSpiderEnvelope | cave_spider, spider | OwnedWalking, OwnedGlide, OwnedBoat | WalkingPush, IncomingPush, SolidCollision | Metadata, Equipment, Scale, Passengers, Status(-128..127), Animation(0..255) |
<!-- contact-policy:envelopes:end -->

## Original owner review

This review reuses the Step C client/mapping hashes, resolved method owners,
packet audit and declared contact lane. `audit_contact_envelopes.py` remaps those
verified originals afresh and fingerprints **all methods** in Spider/CaveSpider,
their ancestors, EntityType, EntityGetter, ClientPacketListener, the boat class,
and modern InterpolationHandler. Legacy lerpTo resolves to LivingEntity; modern
moveOrInterpolateTo resolves to Entity and getInterpolation to LivingEntity. See the generated
[evidence](evidence/contact-envelopes-20261015.json). Fingerprints bind a manual
semantic review; a matching owner alone does not establish locality.

* Both Spider constructors delegate to Monster; CaveSpider delegates to Spider.
  Neither constructor installs a client vehicle or a special motion authority.
  Spider.tick delegates then changes the climbing flag only on the server.
  Monster.aiStep changes swing/no-action timers then delegates to the existing
  inherited Mob/LivingEntity client lane. Spider.onClimbable reads its climbing
  flag. This changes whether a local pair push is attempted, not its range.
  CaveSpider's poison attack and Spider's jockey/effect spawn preparation belong
  to server attack/spawn dispatch, not these client ticks. Goal/navigation
  creation is not client AI execution. There is no subclass client teleport,
  area impulse, attachment-to-player effect or nonlocal contact callback in this
  finite default lane. We do not infer the same for Horse, Ghast or other types.
* LivingEntity's nonauthoritative client AI stage interpolates position, damps
  velocity when not interpolating, clamps small velocity and runs the inherited
  pair selector. It does not use that velocity to run authoritative travel.
  The existing owner contract isolates full NPC baseTick/fluid/AI, as documented
  in Step C and the numerical owner fixture. This proof has the same boundary;
  it does not claim all autonomous original entity physics is simulated.
* Outgoing pushEntities queries the current box inflated by 0.2 horizontally;
  doPush/push only changes delta movement of the selected pair. Contact predicates
  cannot enlarge that query. Legacy getCollideBox is Entity's null result;
  modern canBeCollidedWith is Entity's false result. Boat's separate incoming
  isPushable selector remains potentially relevant. We assume it may be true;
  no predicate is guessed from the capability classification.
* Before metadata/scale/callback receipts, the shape is the original EntityType
  default width/height. No baby, pose, sleep, scale or mount change is inferred.
  Any metadata, equipment, scale, status or animation receipt removes the finite
  proof, including empty/partial data and unknown opcodes. Any passenger update
  involving the entity as vehicle or child also removes it; unavailable child
  membership removes all finite proofs. We have **not** proved Unknown callbacks
  near-only. The retained proof state records the first proof-breaking packet and
  its opcode, receive sequence and ordinal. Later motion cannot restore proof.

## Envelope and tick proof

A deferred body has no public numerical sample. Its separate packet codec model
is never ticked and is not presented as its current position. The possibility
hull contains spawn position, every codec base and interpolation target admitted
from the complete ordered stream, and raw absolute packet endpoints. Each anchor also encloses its X/Z clamped
to +/-30 million: legacy Entity.absMoveTo can clamp placement while the packet
codec retains the raw coordinates. Keeping both choices also conservatively
covers absolute placement variants in the modern client. Including
raw endpoints is essential for legacy tiny-teleport suppression: the real
interpolated pose may choose differently from the unstepped codec model. Relative
packets use the separate native codec base, not latest-received feet.

Every anchor is expanded by the full default width horizontally, default height
upwards, and one block on every face. This encloses the centered default box,
0.2 pair-selector inflation and rounding error within the admitted codec domain
(|x/z| <= 32 million, |y| <= 8 million). Default unmounted client interpolation
is a coordinatewise convex step between existing feet and target; modern target
translation is zero without a separate displacement. Neither pair impulses nor
velocity receipts move feet in this declared nonauthoritative lane. All later
positions therefore remain in this growing hull. It never shrinks based on
elapsed ticks, distance, visibility, latest target or a guessed native clock.
Nonfinite/out-of-domain/missing/unsupported motion removes the finite proof.

The retained proof establishes exclusion only if every relevant deferred envelope misses all of:

* Own initial box and the hull of own boxes before and after its native slot.
  Remote slots remain in their original dispatch order on both versions.
* Every terrain cell requested by the actual numerical physics queries. In
  `Level::geometry` those cells tile the complete attempted swept box, including
  step-up/down alternatives and a border, before collision clipping. A guard
  wraps the terrain provider before any cell enters the per-tick cache. A cache
  hit was therefore already checked in this tick. This also covers noCollision,
  support and boat water-snap queries; conservative failure to prove exclusion is allowed.
  Thus clipping against a wall cannot hide an unsupported box further along
  the requested sweep. Spectator movement, which skips collision queries, is
  covered by its before/after straight path.
* The separate expanded boat selector before/after its native slot, every fixed
  remote rigid boat collider, and boat swept/target geometry queries above.
* **Every** qualified remote body's current box and full remaining interpolation
  hull, including the modern shifted target used by the collision query. A sleep
  metadata displacement can shift that target beyond the ordinary target hull.
  Checking all peers invalidates exclusion even for a distant unrelated peer overlap.
  It proves there is no first edge from an omitted body to a numerically retained
  body, excluding indirect chains regardless of native order or chain length.
  Deferred spiders may interact with each other, but pair delta changes cannot
  move their feet in this lane or escape their independent possibility hulls.

Closed overlap (including touching), nonfinite or inverted query boxes, and
missing finite proof cannot establish nonintersection. The extra margins and
peer test are deliberately conservative. Step 6 permits these cases through
the table's PassThrough fallback, without presenting them as proven harmless.

## Receipt, lifetime and I/O boundaries

The owned boat sampler receives the exact deferred lifetimes from the retained
contact stage and leaves their decision to its envelope guards. It does not
apply the older latest-target distance fallback to those bodies or fabricate a
living collision box for them. Other admission or numerical failures still discard the
whole staged frame before retention/I/O.

The same reducer runs bootstrap, checkpoint, suffix and refresh. Monotone hulls
and revoked proof state survive eviction checkpoints. Removal drops the lifetime;
ID reuse starts a new envelope, with no inherited position or callback exemption.
World/provenance/history invalidation retains the existing typed terminal policy.
Metadata and mount changes are processed before a tick's queries. Qualified-body
metadata changes are reflected in its peer geometry, including sleeping targets.

A failed deferred proof no longer emits MovementRefusal::Unsupported. Existing
control/vehicle code still stages cloned bodies and contact models; true
admission/numerical failures retain their original recovery class and I/O guards.

## Reproduce

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/audit_contact_envelopes.py \
  --downloads /path/to/downloads --work /path/to/envelope-audit
cargo test --lib client::entity::owned_contacts::tests
```

Java 21+, Python 3 and the same fixed inputs as the Step C audit are required.
The evidence binds the review, evidence auditor, Step C artifact, reducers and physics
query sources. Rust regressions exercise both versions, native slots, far/near,
interpolation crossings, attempted sweeps clipped by terrain, indirect peer
contact, callback revocation, checkpoints, lifetimes and world boundaries.
Existing packet, qualified-type numerical, boat and control tests remain required.


## Step D source-binding refresh

[Step D](contact-capability-audit.md#step-d-ordinary-mob-review) independently
qualifies seven more ordinary types in both versions. It adds no envelope
provider or callback exemption. Legacy dimension refresh can leave a box
anchored at its prior minimum until an original setPos. The peer proof therefore
encloses both that current box and the body's recentered placement, plus the
existing interpolation/shifted-target hull. This preserves the indirect-effect
fence when newly qualified metadata changes dimensions. The same evidence auditor
refreshes the dated evidence's source hashes and exports the additional owner
columns; all original Step B method-owner bindings remain checked.

## Player scope after decision A (#246)

Player admission and per-tick revalidation now use the separate
[player scope proof](player-contact-scope.md), including qualified mounted
groups and transitive contact propagation. That proof is bound to the declared
model origin and explicit numerical ticks, not real-time server position.
The older spider deferred-policy proof and boat/numerical APIs above are unchanged.
