# Contact scope regression review (2026-10-25)

This extends decision A in [player-contact-scope.md](player-contact-scope.md):
complete received model origin and explicit numerical ticks. It makes no claim
about server wall time, AI, packets in flight, or unobserved teleportation. No
contact pass-through preference is used as evidence of spatial exclusion.

## Exclusion-only species

Both official clients and servers are pinned by Mojang's version manifests.
`scripts/audit_contact_scope_regression.py` verifies the original JAR/mapping
hashes, remaps the servers, and records method owners and all ancestor method
fingerprints in [the evidence](evidence/contact-scope-regression-20261025.json).
The existing capability and packet audit data, spider proof, numerical
qualification set and every existing original-bytecode fingerprint are unchanged.
The player-scope audit's Rust/document source bindings and the spider-envelope
audit's `owned_contacts.rs` source binding necessarily change with this fix;
these bind the actual query consumers, not only the unchanged spider provider. A separate evidence file binds the additional review below.

The new exclusion-only species are Fox, Skeleton/Stray/WitherSkeleton, EnderMan,
Wolf, Horse/Donkey/Mule/SkeletonHorse/ZombieHorse, Cod/Salmon/TropicalFish/Pufferfish,
Husk/Drowned, and ChestMinecart. They remain unsupported numerical contacts near
the player, with `MovementRefusal::Scoped` and entity/packet/geometry context.
This does not qualify their combat, pathfinding, taming, travel, or full baseTick.
Unreviewed types still fail closed; there is no guessed global vanilla envelope.

* The reviewed living species use the inherited nonauthoritative LivingEntity
  interpolation lane. Their species AI steps either delegate, change visual or
  input/velocity state, or gate authoritative work on the server. Fish flopping
  changes delta movement; it does not run travel on the remote client lane.
  Fox sleeping flags inhibit input, EnderMan's client AI adds particles, Wolf's
  client tick animates shaking, and horse standing/eating affects visual and
  rider placement. None adds unbounded client feet displacement to this lane.
  Team/owner predicates can change whether a local pair occurs, not its range.
* Every admitted spawn, codec base, interpolation target and raw absolute endpoint
  remains in a monotone hull. The inherited client interpolation is convex;
  delta movement and local-pair impulses do not drive remote travel here.
  Thus the additional per-tick **position displacement outside this hull is
  zero**, over every explicit tick horizon, without a server-time speed guess.
  Received teleports and metadata bed placement still enlarge the hull before
  selection. Nonfinite, unknown or incomplete inputs still revoke the proof.
* Dimensions come from the version's generated official EntityType table, at
  native float precision. Adult dimensions bound baby variants; modern Salmon's
  large type additionally multiplies them by **1.5** (`Salmon.getSalmonScale`).
  Pufferfish's native scale cases are 0.5, 0.7 and 1.0, so the EntityType full
  dimensions bound every puff. The existing full-width/one-block margin encloses
  native float rounding, legacy refreshDimensions anchoring and pair inflation.
  Unknown pose, malformed common age/health fields, spin, and non-unit scale
  retain strict refusal. Retained species metadata/callbacks cannot exceed these
  reviewed shapes or introduce a nonlocal pair effect.
* Ordinary ChestMinecart uses AbstractMinecart's legacy client lerp/reapply lane
  and modern OldMinecartBehavior's interpolation/reapply lane. Gravity, rail
  travel, pickup and cart push dispatch are on the server side of those tick
  branches. Its shape comes from the official table. The experimental modern
  `MOVE_MINECART` weighted path is **not** admitted by this proof: receiving it
  latches a private, lifetime-scoped `MoveMinecart` refusal before the next frame,
  including after a new owner/checkpoint. Removal/ID reuse ends that lifetime.
  No synthetic motion/metadata receipt is inserted in public history.
* Relationship groups retain the existing conservative attachment expansion and
  whole-group selection. All ancestors' rider/attachment hooks are included in
  the additional fingerprints. A relevant rider on a moving item/projectile
  carrier remains unsupported: the carrier's irrelevance as an isolated body
  does not establish a finite rider placement proof.

## Positive noncontact proof

ItemEntity, ExperienceOrb, Arrow, SpectralArrow and ThrownTrident do not belong
to the living pair-push dispatch. They inherit Entity's false `isPushable` and
null legacy collide box / false modern `canBeCollidedWith`; the controlled
player's local selector does not acquire them as contact bodies. Their own tick,
metadata, event and projectile-hit paths do not apply client contact impulses to
the player in this lane (server hit/damage/pickup is distinct). They are therefore
irrelevant at any distance, independently of position/velocity. A group is
omitted this way only when **all** its members have positive no-contact proofs.
No unknown geometry is treated as an empty hitbox.

The reviewed common living status codes (hurt/death, shield sounds, equipment
break/swap and particles) retain a finite shape/placement bound for audited
non-player living types. Unknown callbacks still refuse. In particular, original
LivingEntity event **3** sets non-player health to zero and calls `die`; the owned
contact reducer now sets its derived alive/pushable predicate false until later
actual metadata/removal. This is not a synthetic received health value. The
corresponding client death callbacks and inherited `die` methods are fingerprinted
in the added audit. This closes the death-before-despawn gap exercised by churn.

## Relationship recovery and in-place scope updates

Preserving rider placements now inspects only connected groups affected by the
relationship event. A malformed unrelated entity can no longer make that
preservation poison the connection. Unavailable relationship history is keyed by
carrier and clears when fresh complete passenger membership arrives, including
an empty list; despawning that carrier also retires the error. If lost placement
cannot be bounded, refusal continues until that fresh state, with typed context.
A fresh complete packet is not interpreted as an acknowledgement of prior input.

Graph closure includes a two-block terrain-cell footprint. Previously selected
nearby bodies have an eight-block retention margin, so removing a bridging spawn
does not discard their derived state for one boundary. Both margins select extra
work; final attempted-cell and before/after-sweep checks remain mandatory.

An initially excluded, zero-velocity body with no received displacement or bed
placement, no unproven callback, and no previously advanced local contact can be
rehydrated in place from the complete receipt reducer. Its skipped numerical
position/velocity steps are identity operations in the declared lane. Existing
active numerical bodies, owner ID, native order, origin and tick count are kept.
Only state proven equivalent is restored; omitted interpolation, nonzero motion,
a prior numerical/contact state, history gaps and unknown terrain still refuse
before a movement write when re-entry cannot be proved. This is a rare typed stop,
not an automatic reset of the contact model or a server-current position claim.

## Respawn and selected slot

Official `PlayerList.respawn`/`ServerPlayer.restoreFrom` and `Inventory.replaceWith`
show that selection transfer depends on the respawn/keep-inventory path. The
respawn packet's data-copy flags are not a keep-inventory acknowledgement, and
neither version unconditionally echoes the held slot after normal death.
Both official ClientPacketListener handlers create a new local player rather
than providing a universally received selected-slot value.

Voxrig therefore **re-sends** its last known held-slot input after RESPawn, using
that version's actual serverbound held-slot packet. The selection stays missing
until the complete send succeeds and then has `ValueSource::Submitted`. A failed
or uncertain dispatch cannot publish it as received. An actual subsequent
clientbound held-slot packet overrides it with its own receipt ordinal. Unknown
or interrupted pre-respawn selections stay unknown. This works with both
keep-inventory settings without inferring server inventory or server acceptance.
The ordinary fresh-world pose/health/slot/readiness requirements still apply.

## Inventory move boundary

`move_inventory` now captures player, screen and registries under one adapter
receive lock, rather than hoping that eight rounds of independent reads share a
sequence. Entity churn can no longer produce `move could not capture one received
boundary` after a successful pickup. Existing click records, cursor provenance,
conflict/timeout inspection and no-replay behavior remain intact. There is no
blind automatic cursor cleanup after an uncertain click.

## Final local evidence

Both original servers passed the same native-feature consumer: far fox/skeleton/
chest minecart plus adjacent drops, eight nearby spawn/death/despawn cycles with
one control owner, and slot 5 → death/respawn → dig without manual reselection.
The [summary](evidence/contact-scope-regression-live-20261025.json) binds the fixed
SDK binary and source hashes to the compressed complete reports and packet traces.
It records an actual held-slot frame after the actual RESPawn frame on both
versions. `--all-targets` tests pass for default and native (1,626 unit tests each,
eight existing ignores; four native integration tests), and doctests pass.
