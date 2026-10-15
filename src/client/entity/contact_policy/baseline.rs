mod policy_baseline {
    use crate::client::entity::contact_policy::key::*;
    use crate::client::entity::{
        NativeSpawn, NativeSpawnMotion, SpawnLedger, contact_capabilities,
        owned_contacts::OwnedContacts,
    };
    use crate::client::{Aabb, ReceivedPose};
    use crate::client::{
        EntityHistoryKind, EntityHistoryValue, EntityId, LivingInterpolationPose,
        MAX_ENTITY_HISTORY_READ, SessionStamp,
    };
    use crate::{MinecraftVersion as V, MovementPacketTrigger, MovementRefusal};
    use std::fmt::Write;

    fn receipts(version: V) -> (SpawnLedger, SessionStamp) {
        let session = SessionStamp {
            version,
            connection_id: 9,
            world_generation: 3,
        };
        let mut ledger = SpawnLedger::default();
        ledger.contact_local_player(99);
        ledger.history_context(version, 3, 1);
        ledger
            .insert(
                version,
                NativeSpawn {
                    id: 7,
                    uuid: Some([1; 16]),
                    type_id: None,
                    dedicated_type_name: Some("cow"),
                    position: [0.9, 65., 0.5],
                    living: Some(true),
                },
                1,
                16,
            )
            .unwrap();
        ledger.initialize_motion(
            7,
            NativeSpawnMotion {
                position: Some([0.9, 65., 0.5]),
                rotation: Some([0.; 2]),
                head_yaw: Some(0.),
                velocity: Some([0.; 3]),
            },
            1,
        );
        (ledger, session)
    }

    fn spawn(version: V, name: &'static str) -> (SpawnLedger, SessionStamp, EntityId) {
        let (mut ledger, session) = receipts(version);
        ledger.remove(7);
        ledger.history_context(version, 3, 2);
        ledger
            .insert(
                version,
                NativeSpawn {
                    id: 7,
                    uuid: Some([2; 16]),
                    type_id: None,
                    dedicated_type_name: Some(name.strip_prefix("minecraft:").unwrap()),
                    position: [0.9, 65., 0.5],
                    living: Some(true),
                },
                2,
                16,
            )
            .unwrap();
        ledger.initialize_motion(
            7,
            NativeSpawnMotion {
                position: Some([0.9, 65., 0.5]),
                rotation: Some([0.; 2]),
                head_yaw: Some(0.),
                velocity: Some([0.; 3]),
            },
            2,
        );
        let ledger_entity = ledger.identity(session, 7).unwrap();
        (ledger, session, ledger_entity)
    }

    // Run-length encoding retains every opcode outcome, without a huge repeated
    // matrix. No hash or production-generated expected values hide differences.
    fn runs(values: impl Iterator<Item = bool>) -> String {
        let mut output = String::new();
        let mut previous = None;
        let mut count = 0;
        for value in values {
            if previous != Some(value) && count != 0 {
                write!(output, "{}:{count},", u8::from(previous.unwrap())).unwrap();
                count = 0;
            }
            previous = Some(value);
            count += 1;
        }
        if let Some(value) = previous {
            write!(output, "{}:{count}", u8::from(value)).unwrap();
        }
        output
    }

    #[test]
    fn frozen_contact_policy_baseline() {
        let mut out = String::from(
            "contact-policy-baseline-v1\n\
base=fbd854f; union=159; versions=2; lanes=5; effects=7; packets=521\n\
packet-order=Spawn,Motion,Metadata,Equipment,Scale,Status(-128..127),Animation(0..255),Removed,Passengers,WorldChanged,OwnPositionCorrection\n\
rows=version/entity/lane/effect: shared-classification, harmless-packet-runs\n\
shared predicates are observations, not lane admission; spawn is owned walking; locality is default-pose far-query walking/boat\n",
        );
        let mut count = 0;
        for version in VERSIONS {
            for &entity_type in EntityType::ALL {
                let name = entity_type.name();
                if entity_type.native_id(version).is_none() {
                    writeln!(out, "{version:?}/{name}: absent").unwrap();
                    continue;
                }
                let (ledger, session, entity) = spawn(version, name);
                let admission = OwnedContacts::new(session, 10, 99, &ledger, 2);
                let selector = match &admission {
                    Ok(model) => model.world.bodies.values().next().map_or_else(
                        || "NoTickBody".to_owned(),
                        |body| format!("{:?}", body.dispatch),
                    ),
                    Err(error) => format!("Refused:{:?}", error.movement_refusal().unwrap()),
                };
                let deferred = contact_capabilities::DeferredContact::new(
                    version,
                    entity,
                    name,
                    2,
                    Some(LivingInterpolationPose {
                        position: [0.9, 65., 0.5],
                        rotation: [0.; 2],
                    }),
                );
                let query = Aabb {
                    min_x: 100.,
                    min_y: 65.,
                    min_z: 100.,
                    max_x: 101.,
                    max_y: 67.,
                    max_z: 101.,
                };
                writeln!(
                    out,
                    "{version:?}/{name}: spawn={selector}; far_locality={}/{}; modeled={}",
                    deferred.check(query, false).is_ok(),
                    deferred.check(query, true).is_ok(),
                    contact_capabilities::capabilities(version, name).modeled()
                )
                .unwrap();
                for &lane in OwnerLane::ALL {
                    for &effect in Effect::ALL {
                        let mut classification = None;
                        let harmless = runs(PacketKey::ALL.into_iter().map(|packet| {
                            let caps = contact_capabilities::capabilities(version, name);
                            let class = match effect {
                                Effect::WalkingPush => caps.walking,
                                Effect::IncomingPush => caps.incoming,
                                Effect::SolidCollision => caps.solid,
                                Effect::Geometry => caps.geometry,
                                Effect::Metadata => caps.metadata,
                                Effect::Status => caps.status,
                                Effect::Animation => caps.animation,
                            };
                            assert!(classification.is_none_or(|previous| previous == class));
                            classification = Some(class);
                            let kind = match packet {
                                PacketKey::Status(status) => EntityHistoryKind::Status {
                                    native_id: 7,
                                    entity: Some(entity),
                                    status,
                                },
                                PacketKey::Animation(animation) => EntityHistoryKind::Animation {
                                    native_id: 7,
                                    entity: Some(entity),
                                    animation,
                                },
                                PacketKey::Spawn | PacketKey::Motion => {
                                    let history = ledger
                                        .history_after(session, 2, None, MAX_ENTITY_HISTORY_READ)
                                        .unwrap();
                                    let motion = history
                                        .records
                                        .into_iter()
                                        .find_map(|r| match r.kind {
                                            EntityHistoryKind::Spawn(m) => Some(m),
                                            _ => None,
                                        })
                                        .unwrap();
                                    if packet == PacketKey::Spawn {
                                        EntityHistoryKind::Spawn(motion)
                                    } else {
                                        EntityHistoryKind::Motion(motion)
                                    }
                                }
                                PacketKey::Metadata => EntityHistoryKind::Metadata {
                                    native_id: 7,
                                    entity: Some(entity),
                                    entries: EntityHistoryValue::RetentionLimit,
                                    complete: false,
                                },
                                PacketKey::Equipment => EntityHistoryKind::Equipment {
                                    native_id: 7,
                                    entity: Some(entity),
                                    entries: EntityHistoryValue::RetentionLimit,
                                    complete: false,
                                },
                                PacketKey::Scale => EntityHistoryKind::Scale {
                                    native_id: 7,
                                    entity: Some(entity),
                                    attribute: EntityHistoryValue::RetentionLimit,
                                },
                                PacketKey::Removed => EntityHistoryKind::Removed {
                                    native_id: 7,
                                    entity: Some(entity),
                                },
                                PacketKey::Passengers => EntityHistoryKind::Passengers {
                                    native_id: 7,
                                    entity: Some(entity),
                                    passengers: EntityHistoryValue::RetentionLimit,
                                },
                                PacketKey::WorldChanged => EntityHistoryKind::WorldChanged {
                                    previous_generation: 3,
                                },
                                PacketKey::OwnPositionCorrection => {
                                    EntityHistoryKind::OwnPositionCorrection {
                                        pose: ReceivedPose {
                                            position: [0.; 3],
                                            rotation: [0.; 2],
                                            receive_sequence: 2,
                                        },
                                        velocity: None,
                                    }
                                }
                            };
                            assert_eq!(PacketKey::from_history(&kind), packet);
                            count += 1;
                            contact_capabilities::harmless_packet(version, name, &kind)
                        }));
                        writeln!(
                            out,
                            "{lane:?}/{effect:?}: {:?}; {harmless}",
                            classification.unwrap()
                        )
                        .unwrap();
                    }
                }
            }
        }
        assert_eq!(count, (107 + 157) * 5 * 7 * 521);
        writeln!(out, "evaluated-present-keys={count}").unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("data/client_api/contact_policy_step1.txt");
        if std::env::var_os("VOXRIG_UPDATE_CONTACT_BASELINE").is_some() {
            std::fs::write(&path, &out).unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            out,
            "baseline changed; review before updating with VOXRIG_UPDATE_CONTACT_BASELINE=1"
        );
    }

    #[test]
    fn representative_callback_context() {
        for version in VERSIONS {
            for (name, status, animation) in [
                ("minecraft:rabbit", Some(1), None),
                ("minecraft:sheep", Some(10), None),
                ("minecraft:sheep", Some(35), None),
                ("minecraft:sheep", None, Some(4)),
                ("minecraft:sheep", None, Some(5)),
            ] {
                let (mut ledger, session, entity) = spawn(version, name);
                let mut model = OwnedContacts::new(session, 10, 99, &ledger, 2).unwrap();
                ledger.history_context(version, 3, 3);
                ledger.history_signal(7, status, animation);
                let result = model.refresh(10, &ledger, 3);
                if name == "minecraft:rabbit" {
                    let error = result.unwrap_err();
                    let refusal = error.movement_refusal().unwrap();
                    assert!(matches!(refusal, MovementRefusal::Unsupported { .. }));
                    let context = refusal.context();
                    assert_eq!(context.entity, Some(entity));
                    assert_eq!(context.entity_type.as_deref(), Some(name));
                    assert_eq!(context.native_entity_id, Some(7));
                    assert_eq!(
                        context.packet,
                        Some(MovementPacketTrigger {
                            packet_type: "Status".into(),
                            opcode: 1
                        })
                    );
                } else {
                    result.unwrap();
                }
            }
            let caps = contact_capabilities::capabilities(version, "minecraft:wolf");
            use contact_capabilities::Classification::Unknown;
            assert_eq!(
                [
                    caps.walking,
                    caps.incoming,
                    caps.solid,
                    caps.geometry,
                    caps.metadata,
                    caps.status,
                    caps.animation
                ],
                [Unknown; 7]
            );
        }
    }
}
