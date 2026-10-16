# Contact packet exemptions (step A)

Scope: exact contact reducer, Java 1.16.1 and 1.21.11 only. This is a deliberately
small audited set, not a claim that all other opcodes affect movement. Unknown
opcodes now pass through without applying contact effects (Step 6, #128 option 3). Public entity history continues to retain every packet.

<!-- contact-policy:packets:start -->
| Version | Packet key (not wire ID) | Qualified targets |
| --- | --- | --- |
| 1.16.1 | Animation(4) | chicken, cow, creeper, giant, mooshroom, ocelot, pig, player, rabbit, sheep, silverfish, villager, wandering_trader, zombie |
| 1.16.1 | Animation(5) | chicken, cow, creeper, giant, mooshroom, ocelot, pig, player, rabbit, sheep, silverfish, villager, wandering_trader, zombie |
| 1.16.1 | Status(10) | sheep |
| 1.16.1 | Status(35) | chicken, cow, creeper, giant, mooshroom, ocelot, pig, player, rabbit, sheep, silverfish, villager, wandering_trader, zombie |
| 1.21.11 | Animation(4) | chicken, cow, creeper, giant, mooshroom, ocelot, pig, player, rabbit, sheep, silverfish, villager, wandering_trader, zombie |
| 1.21.11 | Animation(5) | chicken, cow, creeper, giant, mooshroom, ocelot, pig, player, rabbit, sheep, silverfish, villager, wandering_trader, zombie |
| 1.21.11 | Status(10) | sheep |
| 1.21.11 | Status(35) | chicken, cow, creeper, giant, mooshroom, ocelot, pig, player, rabbit, sheep, silverfish, villager, wandering_trader, zombie |
<!-- contact-policy:packets:end -->

These generated target/opcode sets describe the current private reducer in both
versions. Each species must qualify its contact dispatch before entering it.
No distance exemption, envelope, or approximate mode is used for these callbacks.
Status 10 sets Sheep's eating timer; Status 35 and Animation 4/5 produce the
visual/audio effects reviewed below. All other sub-opcodes use the separate PassThrough policy.
`python3 -B scripts/audit_contact_packets.py --check` verifies these finite proof
claims against the export without jars. `python3 -B scripts/contact_policy_export.py
--check` checks the generated section; see [consumer checks](contact-policy-consumers.md).

## Original handler audit

Inspected `javap -c -p` output from the original clients remapped with their
matching Mojang mappings. Original jar/mapping SHA-1s and inspected method SHA-256s
are recorded in `data/client_api/contact_packet_audit.json`. Reproduce with:

```sh
python3 scripts/audit_contact_packets.py --downloads /tmp/voxrig-downloads \
  --work /tmp/voxrig-opt1-owner --output data/client_api/contact_packet_audit.json
```

`run_entity_owner_oracle.py` prepares these named jars from verified original
inputs. The audit script records provenance and fingerprints; manual bytecode
inspection supplies the effect proof, not method names alone.

In both `ClientPacketListener.handleEntityEvent` implementations, 35 takes the
listener's special branch and never calls the target's virtual status handler.
The branch reads position and sound category and creates the totem emitter and
local sound; its local-player-only branch displays an item activation. It does
not write target motion, health, pose, dimensions, passenger state or contact
flags. The modern listener also special-cases 63; that opcode is not exempted.

Status 10 passes through the listener to `Sheep.handleEntityEvent`. That override
sets only the eating timer to 40 and returns; other statuses delegate to Animal.
All uses of the timer in Sheep were checked: the client `aiStep` decrements it
before an unconditional superclass call, and head-position/head-angle rendering
getters read it. The server AI method writes it, but the client event does not
invoke server AI. It cannot change interpolation, velocity, bounding box,
pushability, sleeping or later numerical contact callbacks.

In both `ClientPacketListener.handleAnimate` implementations, 4 and 5 go directly
to `ParticleEngine.createTrackingEmitter`, with CRIT and ENCHANTED_HIT respectively.
They invoke no target animation, swing or sleeping callback and perform no entity
state write. The `ParticleEngine.createTrackingEmitter` overloads and `TrackingEmitter`
constructors/tick were also inspected: emitters use their own random source,
read target velocity/coordinates and write only particle state. Particle emitters
read target coordinates for visual placement;
they do not change the target contact state. This qualification does not exempt
all animations: 2 calls the player's wake-up path and is ignored by contact reduction. Legacy 1
calls animateHurt; modern 1 is absent. Neither is in this allowlist. Swings 0/3
remain unaudited and now pass through. Status 3 death and unrecognized
status/animation values are ignored without applying their original effects.

## Typed failure policy

All explicit owned-contact failure sites construct a recovery class without
parsing diagnostic strings. Missing remote profile/game mode is Transient;
unsupported modeled geometry/pose/scale and serializers are Unsupported;
history loss, partial/retention loss, provenance, missing origin inputs, exhausted
bounds, correction, world change and discarded owners are Permanent for that
admission/owner. Lower contact errors retain their existing Unsupported category
when wrapped; other lower errors terminate this contact operation as Permanent.

`Error::movement_refusal()` exposes the same structured error on the existing
Result path. Packet reducer errors carry frozen lifetime, native ID, received type,
packet kind/opcode, receipt ordinal and sequence. Packets with no sub-opcode use
zero, not a fabricated wire packet ID. Checkpoints retain the typed value rather
than Display text. Admission and refresh rebind owner context; failure of refresh
still discards the owner and rolls back staged receipt changes.

Walking `ControlRecord.movement_refusal` and mounted
`VehicleControlRecord.movement_refusal` retain typed continuation failures alongside
the existing human-readable stopped/inspection reason, including release failures.
