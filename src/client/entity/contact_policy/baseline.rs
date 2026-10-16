mod policy_baseline {
    use crate::MinecraftVersion as V;
    use crate::client::entity::contact_policy::key::*;
    use crate::client::entity::contact_policy::{
        frozen_capabilities, frozen_locality, frozen_selectors, locality, table,
        validate::EXPANDED_PACKETS,
    };
    use crate::client::entity::{
        NativeSpawn, NativeSpawnMotion, SpawnLedger, contact_capabilities,
        owned_contacts::OwnedContacts,
    };
    use crate::client::{Aabb, ReceivedPose};
    use crate::client::{
        EntityHistoryKind, EntityHistoryValue, EntityId, LivingInterpolationPose,
        MAX_ENTITY_HISTORY_READ, SessionStamp,
    };
    use std::fmt::Write;

    fn production_tick(
        dispatch: crate::client::entity::contact_tick::ContactDispatch,
    ) -> table::TickModel {
        use crate::client::entity::contact_tick::ContactDispatch;
        match dispatch {
            ContactDispatch::InheritedLiving => table::TickModel::InheritedLiving,
            ContactDispatch::Chicken => table::TickModel::Chicken,
            ContactDispatch::RemotePlayer => table::TickModel::RemotePlayer,
        }
    }

    fn production_class(class: contact_capabilities::Classification) -> table::Class {
        match class {
            contact_capabilities::Classification::Qualified => table::Class::Qualified,
            contact_capabilities::Classification::ProvenIrrelevant => {
                table::Class::ProvenIrrelevant
            }
            contact_capabilities::Classification::BoundedUnknown => table::Class::BoundedUnknown,
            contact_capabilities::Classification::Unknown => table::Class::Unknown,
        }
    }

    // Frozen pre-step-3 allowlist. Do not replace with table-derived expectations.
    fn frozen_harmless_packet(version: V, name: &str, kind: &EntityHistoryKind) -> bool {
        use contact_capabilities::Classification;
        let policy = frozen_capabilities::lookup(version, name);
        match kind {
            EntityHistoryKind::Status { status, .. }
                if policy.status == Classification::BoundedUnknown =>
            {
                *status == 35 || (*status == 10 && name == "minecraft:sheep")
            }
            EntityHistoryKind::Animation {
                animation: 4 | 5, ..
            } => policy.animation == Classification::BoundedUnknown,
            _ => false,
        }
    }

    fn default_far_locality(profile: table::Profile, boat: bool) -> bool {
        // Only the fixed default-pose/far-query observation in step 1. This is
        // not an implementation of DeferredContact's lifetime/geometry proof.
        matches!(profile.locality, table::LocalityId::DefaultSpiderEnvelope)
            || matches!(profile.unqualified, table::Decision::PassThrough)
            || (matches!(profile.effects.solid, table::Class::ProvenIrrelevant)
                && matches!(
                    if boat {
                        profile.effects.incoming
                    } else {
                        profile.effects.walking
                    },
                    table::Class::ProvenIrrelevant
                ))
    }

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
    fn unresolved_and_absent_names_preserve_frozen_classifications_and_proofs() {
        for version in VERSIONS {
            for name in EntityType::ALL.iter().map(|entity| entity.name()).chain([
                "",
                "cow",
                "minecraft:missing",
                "other:cow",
                "minecraft:Cow",
            ]) {
                assert_eq!(
                    contact_capabilities::capabilities(version, name),
                    frozen_capabilities::lookup(version, name)
                );
                for packet in PacketKey::ALL {
                    let kind = match packet {
                        PacketKey::Status(status) => EntityHistoryKind::Status {
                            native_id: -1,
                            entity: None,
                            status,
                        },
                        PacketKey::Animation(animation) => EntityHistoryKind::Animation {
                            native_id: -1,
                            entity: None,
                            animation,
                        },
                        _ => continue,
                    };
                    assert_eq!(
                        contact_capabilities::harmless_packet(version, name, &kind),
                        frozen_harmless_packet(version, name, &kind),
                        "{version:?}/{name}/{packet:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn frozen_contact_policy_baseline() {
        let mut out = String::from(
            "contact-policy-baseline-v2\n\
base=fbd854f; union=159; versions=2; lanes=5; effects=7; packets=521\n\
packet-order=Spawn,Motion,Metadata,Equipment,Scale,Status(-128..127),Animation(0..255),Removed,Passengers,WorldChanged,OwnPositionCorrection\n\
rows=version/entity/lane/effect: shared-classification, harmless-packet-runs, ignored-callback-runs\n\
shared predicates are observations, not lane admission; spawn is owned walking; locality is default-pose far-query walking/boat\n",
        );
        let mut shadow = out.clone();
        let mut count = 0;
        for version in VERSIONS {
            for &entity_type in EntityType::ALL {
                let name = entity_type.name();
                if entity_type.native_id(version).is_none() {
                    writeln!(out, "{version:?}/{name}: absent").unwrap();
                    writeln!(shadow, "{version:?}/{name}: absent").unwrap();
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
                let profile = table::lookup(version, entity_type, OwnerLane::OwnedWalking);
                let actual_tick = admission
                    .as_ref()
                    .unwrap()
                    .world
                    .bodies
                    .values()
                    .next()
                    .map_or(table::TickModel::NoTickBody, |body| {
                        production_tick(body.dispatch)
                    });
                let old = frozen_selectors::owned(version, name);
                assert_eq!(profile.owned, old, "{version:?}/{name} frozen selectors");
                assert_eq!(
                    actual_tick, old.tick,
                    "{version:?}/{name} old Spawn dispatch"
                );
                assert_eq!(profile.owned.tick, actual_tick);
                let shadow_selector = format!("{:?}", profile.owned.tick);
                assert_eq!(shadow_selector, selector, "{version:?}/{name} tick");
                assert_eq!(
                    default_far_locality(profile, false),
                    deferred.check(query, false).is_ok(),
                    "{version:?}/{name} walking locality"
                );
                assert_eq!(
                    default_far_locality(profile, true),
                    deferred.check(query, true).is_ok(),
                    "{version:?}/{name} boat locality"
                );
                assert_eq!(
                    profile.effects.modeled(),
                    contact_capabilities::capabilities(version, name).modeled()
                );
                writeln!(
                    shadow,
                    "{version:?}/{name}: spawn={shadow_selector}; far_locality={}/{}; modeled={}",
                    default_far_locality(profile, false),
                    default_far_locality(profile, true),
                    profile.effects.modeled()
                )
                .unwrap();
                for &lane in OwnerLane::ALL {
                    let profile = table::lookup(version, entity_type, lane);
                    for &effect in Effect::ALL {
                        let mut classification = None;
                        let mut ignored = Vec::new();
                        let harmless = runs(PacketKey::ALL.into_iter().map(|packet| {
                            let caps = contact_capabilities::capabilities(version, name);
                            assert_eq!(caps, frozen_capabilities::lookup(version, name));
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
                            let production =
                                contact_capabilities::harmless_packet(version, name, &kind);
                            assert_eq!(production, frozen_harmless_packet(version, name, &kind));
                            assert_eq!(
                                profile.effects.get(effect),
                                production_class(class),
                                "{version:?}/{name}/{lane:?}/{effect:?}/{packet:?} classification"
                            );
                            let decision = EXPANDED_PACKETS[profile.packets as usize]
                                [effect.ordinal()][packet.ordinal()];
                            assert_eq!(
                                decision, production,
                                "{version:?}/{name}/{lane:?}/{effect:?}/{packet:?} packet"
                            );
                            let ignore = super::super::ignore_callback(version, name, &kind);
                            assert_eq!(
                                ignore,
                                matches!(packet, PacketKey::Status(_) | PacketKey::Animation(_))
                            );
                            ignored.push(ignore);
                            production
                        }));
                        let ignored_runs = runs(ignored.into_iter());
                        writeln!(
                            out,
                            "{lane:?}/{effect:?}: {:?}; {harmless}; {ignored_runs}",
                            classification.unwrap()
                        )
                        .unwrap();
                        let shadow_harmless = runs(PacketKey::ALL.into_iter().map(|packet| {
                            EXPANDED_PACKETS[profile.packets as usize][effect.ordinal()]
                                [packet.ordinal()]
                        }));
                        let decisions = super::super::validate::expand_packets(
                            table::PACKET_POLICIES[profile.packets as usize],
                            table::EVIDENCE_REQUIREMENTS.len(),
                        );
                        let shadow_ignored = runs(PacketKey::ALL.into_iter().map(|packet| {
                            !matches!(
                                decisions[effect.ordinal()][packet.ordinal()],
                                table::Decision::Refuse
                            )
                        }));
                        writeln!(
                            shadow,
                            "{lane:?}/{effect:?}: {:?}; {shadow_harmless}; {shadow_ignored}",
                            profile.effects.get(effect)
                        )
                        .unwrap();
                    }
                }
            }
        }
        assert_eq!(count, (107 + 157) * 5 * 7 * 521);
        writeln!(out, "evaluated-present-keys={count}").unwrap();
        writeln!(shadow, "evaluated-present-keys={count}").unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("data/client_api/contact_policy_step1.txt");
        if std::env::var_os("VOXRIG_UPDATE_CONTACT_BASELINE").is_some() {
            std::fs::write(&path, &out).unwrap();
        }
        let frozen = std::fs::read_to_string(path).unwrap();
        assert_eq!(
            shadow, frozen,
            "shadow table differs from frozen step-1 golden"
        );
        assert_eq!(
            frozen, out,
            "baseline changed; review before updating with VOXRIG_UPDATE_CONTACT_BASELINE=1"
        );
    }

    #[test]
    fn shadow_public_geometry_and_evidence_references() {
        use crate::client::entity::{EntityDataField, EntityDataValue, EntityGeometryModel};
        let oracle: serde_json::Value = serde_json::from_reader(flate2::read::GzDecoder::new(
            &include_bytes!("../../../../data/client_api/entity_owner_oracle.json.gz")[..],
        ))
        .unwrap();
        let envelopes: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../docs/evidence/contact-envelopes-20261015.json"
        ))
        .unwrap();
        for requirement in table::EVIDENCE_REQUIREMENTS {
            if let Some(path) = requirement.artifact {
                assert!(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join(path)
                        .is_file(),
                    "{path}"
                );
            }
        }
        for (version, key) in VERSIONS.into_iter().zip(["1.16.1", "1.21.11"]) {
            let fixture = &oracle["versions"][key];
            let spawned: std::collections::BTreeSet<_> = fixture["results"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|r| r["steps"].as_array().unwrap())
                .flat_map(|s| s["actions"].as_array().unwrap())
                .filter(|a| a["op"] == "spawn")
                .map(|a| a["type_name"].as_str().unwrap())
                .collect();
            for &entity in EntityType::ALL {
                if entity.native_id(version).is_none() {
                    continue;
                }
                let name = entity.name();
                let row = table::ENTITY_POLICIES[entity.ordinal()];
                assert_eq!(
                    matches!(row.vehicle, table::VehicleModel::Rigid),
                    crate::client::vehicle::collision::rigid(name),
                    "{key}/{name} rigid selector"
                );
                let (mut ledger, session, id) = spawn(version, name);
                ledger.receive_scale(
                    7,
                    crate::client::control::ReceivedAttribute {
                        base: 1.,
                        modifiers: vec![],
                    },
                    3,
                );
                if let Some(field) = version.table().entity_data(name).iter().find(|r| {
                    EntityDataField::Baby
                        .official_names()
                        .contains(&(r.owner, r.field))
                }) {
                    ledger.receive_metadata(
                        7,
                        [(field.index, EntityDataValue::Bool(false))],
                        true,
                        3,
                    );
                }
                let observation = ledger.capture_one(session, id, 3).unwrap();
                let actual = match observation.derived_geometry() {
                    Ok(geometry) => match geometry.model {
                        EntityGeometryModel::PlayerPose => table::GeometryModel::PlayerPose,
                        EntityGeometryModel::AgeablePose => table::GeometryModel::AgeablePose,
                    },
                    Err(crate::client::entity::EntityGeometryUnavailable::UnsupportedType) => {
                        table::GeometryModel::NotApplicable
                    }
                    Err(error) => panic!("{key}/{name}: {error:?}"),
                };
                let selected = table::semantics(version, entity, OwnerLane::PublicGeometry);
                assert_eq!(actual, frozen_selectors::public_geometry(name));
                assert_eq!(selected.geometry, actual, "{key}/{name} public geometry");
                assert_eq!(selected.tick, table::TickModel::NoTickBody);
                assert_eq!(selected.metadata, table::MetadataModel::NotApplicable);
                let profile = table::lookup(version, entity, OwnerLane::OwnedWalking);
                match profile.scenarios {
                    table::ScenarioSetId::None => {}
                    table::ScenarioSetId::Living
                    | table::ScenarioSetId::Chicken
                    | table::ScenarioSetId::RemotePlayer => {
                        assert!(
                            spawned.contains(name),
                            "{key}/{name} numerical scenarios missing"
                        );
                    }
                    table::ScenarioSetId::Excluded => assert!(
                        fixture["exclusions"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|e| e["type"] == name)
                    ),
                    table::ScenarioSetId::SpiderEnvelope => assert!(
                        envelopes["versions"][key]["types"]
                            .get(name.strip_prefix("minecraft:").unwrap())
                            .is_some()
                    ),
                }
            }
        }
    }

    #[test]
    fn locality_and_usage_match_independent_old_source() {
        for version in VERSIONS {
            // Includes present, absent and unresolved names. Expectations use
            // the original species expression and independent capability match.
            for name in EntityType::ALL.iter().map(|e| e.name()).chain([
                "",
                "spider",
                "minecraft:missing",
                "other:spider",
                "minecraft:Spider",
            ]) {
                let policy = locality::DeferredPolicy::new(version, name);
                let old_envelope = frozen_locality::envelope(version, name);
                let (_, _, entity) = spawn(version, "minecraft:cow");
                let far = Aabb {
                    min_x: 100.,
                    min_y: 65.,
                    min_z: 100.,
                    max_x: 101.,
                    max_y: 67.,
                    max_z: 101.,
                };
                for boat in [false, true] {
                    let lane = locality::owned_lane(boat, false);
                    let table_profile = EntityType::from_name(version, name)
                        .map_or(table::PROFILES[table::ProfileId::Unknown as usize], |e| {
                            table::lookup(version, e, lane)
                        });
                    let table_locality = table::LOCALITY_PROFILES[table_profile.locality as usize];
                    assert_eq!(
                        table_locality.handler == table::EnvelopeHandler::DeferredContact,
                        old_envelope
                    );
                    assert_eq!(policy.envelope(boat), old_envelope);
                    let old_relevant = frozen_locality::relevant(version, name, boat);
                    assert_eq!(policy.relevant(boat), old_relevant);
                    assert_eq!(
                        table_profile.effects.modeled(),
                        frozen_locality::modeled(version, name)
                    );
                    assert_eq!(
                        locality::modeled(version, name, lane),
                        frozen_locality::modeled(version, name)
                    );
                    for pose in [
                        None,
                        Some(LivingInterpolationPose {
                            position: [0.9, 65., 0.5],
                            rotation: [0.; 2],
                        }),
                    ] {
                        let deferred = contact_capabilities::DeferredContact::new(
                            version, entity, name, 2, pose,
                        );
                        assert_eq!(deferred.relevant(boat), old_relevant);
                        assert_eq!(
                            deferred.proves_irrelevant(far, boat),
                            !old_relevant || (old_envelope && pose.is_some())
                        );
                        assert_eq!(
                            deferred.check(far, boat).is_ok(),
                            policy.pass_through(boat)
                                || !old_relevant
                                || (old_envelope && pose.is_some())
                        );
                        let near = Aabb {
                            min_x: 0.,
                            max_x: 1.,
                            min_z: 0.,
                            max_z: 1.,
                            ..far
                        };
                        assert_eq!(deferred.proves_irrelevant(near, boat), !old_relevant);
                        assert_eq!(
                            deferred.check(near, boat).is_ok(),
                            policy.pass_through(boat) || !old_relevant
                        );
                    }
                    for packet in PacketKey::ALL {
                        let old = old_envelope && frozen_locality::revokes(packet);
                        assert_eq!(
                            table_locality.revokes.contains(&packet),
                            old,
                            "{version:?}/{name}/{lane:?}/{packet:?}"
                        );
                        assert_eq!(policy.revokes(packet), old);
                    }
                }
                for (boat, glide, expected) in [
                    (false, false, OwnerLane::OwnedWalking),
                    (false, true, OwnerLane::OwnedGlide),
                    (true, false, OwnerLane::OwnedBoat),
                    (true, true, OwnerLane::OwnedBoat),
                ] {
                    assert_eq!(locality::owned_lane(boat, glide), expected);
                    assert_eq!(
                        locality::modeled(version, name, expected),
                        frozen_locality::modeled(version, name)
                    );
                }
            }
        }
    }

    #[test]
    fn shadow_spider_revocations_match_current_reducer() {
        use crate::client::{
            control::Controls,
            physics::{Body, Environment},
        };
        fn step(model: &mut OwnedContacts, version: V) -> crate::Result<()> {
            model
                .tick_player(
                    &mut Body::new([100., 65., 100.]),
                    &Environment::defaults(version),
                    Controls::default(),
                    false,
                    None,
                    &mut |_| {
                        Ok(crate::NativeBlockState {
                            name: "minecraft:air".into(),
                            properties: Default::default(),
                        })
                    },
                )
                .map(|_| ())
        }
        let mut revocations = 0;
        for version in VERSIONS {
            for entity in [EntityType::Spider, EntityType::CaveSpider] {
                let profile = table::lookup(version, entity, OwnerLane::OwnedWalking);
                let locality = table::LOCALITY_PROFILES[profile.locality as usize];
                assert_eq!(locality.revokes.len(), 516);
                // Fixed old corpus: a missing table entry must not remove a case.
                for packet in PacketKey::ALL
                    .into_iter()
                    .filter(|p| frozen_locality::revokes(*p))
                {
                    assert!(locality.revokes.contains(&packet));
                    revocations += 1;
                    let (mut ledger, session, _) = spawn(version, entity.name());
                    let mut model = OwnedContacts::new(session, 10, 99, &ledger, 2).unwrap();
                    step(&mut model, version).unwrap();
                    ledger.history_context(version, 3, 3);
                    match packet {
                        PacketKey::Status(status) => ledger.history_signal(7, Some(status), None),
                        PacketKey::Animation(animation) => {
                            ledger.history_signal(7, None, Some(animation))
                        }
                        PacketKey::Metadata => ledger.receive_metadata(7, [], true, 3),
                        PacketKey::Equipment => ledger.receive_equipment(7, [], true, 3),
                        PacketKey::Scale => ledger.receive_scale(
                            7,
                            crate::client::control::ReceivedAttribute {
                                base: 1.,
                                modifiers: vec![],
                            },
                            3,
                        ),
                        PacketKey::Passengers => {
                            ledger.history_passengers(&crate::client::vehicle::NativePassengers {
                                vehicle: 7,
                                passengers: vec![],
                            })
                        }
                        _ => panic!("unexpected revocation {packet:?}"),
                    }
                    model.refresh(10, &ledger, 3).unwrap();
                    step(&mut model, version).unwrap();
                    assert!(model.world.bodies.is_empty());
                    assert_eq!(model.frame(0).bodies.len(), 0);
                }
            }
        }
        assert_eq!(revocations, 2_064);
    }

    #[test]
    fn representative_callbacks_pass_through_with_finite_proofs_unchanged() {
        for version in VERSIONS {
            for (name, status, animation) in [
                ("minecraft:rabbit", Some(1), None),
                ("minecraft:sheep", Some(10), None),
                ("minecraft:sheep", Some(35), None),
                ("minecraft:sheep", None, Some(4)),
                ("minecraft:sheep", None, Some(5)),
            ] {
                let (mut ledger, session, _) = spawn(version, name);
                let mut model = OwnedContacts::new(session, 10, 99, &ledger, 2).unwrap();
                ledger.history_context(version, 3, 3);
                ledger.history_signal(7, status, animation);
                let result = model.refresh(10, &ledger, 3);
                result.unwrap();
                assert_eq!(model.world.bodies.len(), 1);
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
