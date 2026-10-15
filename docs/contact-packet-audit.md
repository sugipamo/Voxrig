# Contact packet exemptions (step A)

Scope: exact contact reducer, Java 1.16.1 and 1.21.11 only. This is a deliberately
small audited set, not a claim that all other opcodes affect movement. Unknown
opcodes still refuse. Public entity history continues to retain every packet.

| Packet | Opcode | Qualified target | Native effect in both clients |
| --- | --- | --- | --- |
| Entity event/status | 10 | Sheep only | Sets `eatAnimationTick = 40`, returns without superclass dispatch |
| Entity event/status | 35 | All currently modeled types | Totem tracking particles and local sound; item activation display for the local player |
| Animate | 4 | All currently modeled types | Critical-hit tracking particle emitter |
| Animate | 5 | All currently modeled types | Enchanted-hit tracking particle emitter |

The modeled targets are player, cow, pig, zombie, sheep and creeper. The two
versions share the reducer; future species must qualify their contact dispatch
before entering it. No distance exemption, envelope, or approximate mode is used.

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
all animations: 2 calls the player's wake-up path and remains refused. Legacy 1
calls animateHurt; modern 1 is absent. Neither is in this allowlist. Swings 0/3
remain unaudited and refused, despite their usual visual purpose. Status 3 death
remains refused, as do unrecognized status/animation values.

## Typed failure policy

All explicit owned-contact failure sites construct a recovery class without
parsing diagnostic strings. Missing remote profile/game mode is Transient;
unsupported dispatch, geometry/pose/scale and serializers are Unsupported;
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
