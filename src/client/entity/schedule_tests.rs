use super::*;
use serde_json::Value;
use std::collections::BTreeMap;

fn pose(value: &Value) -> LivingInterpolationPose {
    LivingInterpolationPose {
        position: std::array::from_fn(|i| {
            f64::from_bits(
                u64::from_str_radix(value["position_bits"][i].as_str().unwrap(), 16).unwrap(),
            )
        }),
        rotation: std::array::from_fn(|i| {
            f32::from_bits(value["rotation_bits"][i].as_u64().unwrap() as u32)
        }),
    }
}
fn body(key: u64, native_id: i32) -> LivingTickBody {
    LivingTickBody {
        key,
        native_id,
        player: false,
        removed: false,
        passenger_of: None,
        passengers: Vec::new(),
        legacy_ticking_chunk: Some(true),
        legacy_in_chunk: Some(true),
        player_passengers: 0,
    }
}
fn match_pose(expected: &Value, actual: LivingInterpolationPose, label: &str) {
    let wanted = pose(expected);
    assert_eq!(
        wanted.position.map(f64::to_bits),
        actual.position.map(f64::to_bits),
        "{label}: position"
    );
    assert_eq!(
        wanted.rotation.map(f32::to_bits),
        actual.rotation.map(f32::to_bits),
        "{label}: rotation"
    );
}
struct Harness {
    version: crate::MinecraftVersion,
    order: LivingEntityTickOrder,
    models: BTreeMap<u64, LivingPacketModel>,
    active: BTreeMap<i32, u64>,
    local: BTreeMap<u64, bool>,
    ordinal: u64,
    frozen: bool,
}
impl Harness {
    fn new(version: crate::MinecraftVersion) -> Self {
        Self {
            version,
            order: LivingEntityTickOrder::new(version),
            models: BTreeMap::new(),
            active: BTreeMap::new(),
            local: BTreeMap::new(),
            ordinal: 0,
            frozen: false,
        }
    }
    fn action(&mut self, a: &Value) {
        let id = a["native_id"].as_i64().unwrap() as i32;
        let op = a["op"].as_str().unwrap();
        if op == "spawn" || op == "spawn_player" {
            let key = a["key"].as_u64().unwrap();
            let mut entry = body(key, id);
            entry.player = op == "spawn_player";
            self.order.admit(entry).unwrap();
            self.active.insert(id, key);
            self.local.insert(key, false);
            self.models.insert(
                key,
                LivingPacketModel::new(self.version, pose(&a["pose"]), 0).unwrap(),
            );
            return;
        }
        let key = self.active[&id];
        if op == "remove_membership" {
            self.order.remove_membership(id).unwrap();
            self.active.remove(&id);
            return;
        }
        let mut b = self.order.body(key).unwrap().clone();
        match op {
            "local" => {
                self.local.insert(key, a["value"].as_bool().unwrap());
            }
            "cancel" => {
                self.models.get_mut(&key).unwrap().cancel().unwrap();
            }
            "removed" => {
                b.removed = true;
                self.order.update(b).unwrap();
            }
            "passenger" => {
                let parent = self.active[&(a["vehicle"].as_i64().unwrap() as i32)];
                b.passenger_of = Some(parent);
                self.order.update(b).unwrap();
                let mut parent_body = self.order.body(parent).unwrap().clone();
                parent_body.passengers.push(key);
                self.order.update(parent_body).unwrap();
            }
            "unride" => {
                let parent = b.passenger_of.take().unwrap();
                self.order.update(b).unwrap();
                let mut parent_body = self.order.body(parent).unwrap().clone();
                parent_body.passengers.retain(|child| *child != key);
                self.order.update(parent_body).unwrap();
                self.models
                    .get_mut(&key)
                    .unwrap()
                    .set_model_pose(pose(&a["model_pose_after"]))
                    .unwrap();
            }
            "unticking" | "ticking" => {
                b.legacy_ticking_chunk = Some(op == "ticking");
                self.order.update(b).unwrap();
            }
            "freeze" | "unfreeze" => self.frozen = op == "freeze",
            "relative" | "rotation" | "absolute" => {
                let rotation = |v: &Value| {
                    std::array::from_fn(|i| v[i].as_i64().unwrap() as f32 * 360. / 256.)
                };
                let ground = a["ground"].as_bool().unwrap();
                let packet = match op {
                    "relative" => LivingMotionPacket::Relative {
                        delta: std::array::from_fn(|i| a["delta"][i].as_i64().unwrap() as i16),
                        rotation: a.get("rotation").map(rotation),
                        ground,
                    },
                    "rotation" => LivingMotionPacket::Rotation {
                        rotation: rotation(&a["rotation"]),
                        ground,
                    },
                    "absolute" => {
                        let position = std::array::from_fn(|i| a["position"][i].as_f64().unwrap());
                        let rotation =
                            std::array::from_fn(|i| a["rotation"][i].as_f64().unwrap() as f32);
                        if self.version == crate::MinecraftVersion::Java1_16_1 {
                            LivingMotionPacket::LegacyTeleport {
                                position,
                                rotation,
                                ground,
                            }
                        } else {
                            LivingMotionPacket::ModernPositionSync {
                                position,
                                rotation,
                                velocity: std::array::from_fn(|i| {
                                    a["velocity"][i].as_f64().unwrap()
                                }),
                                ground,
                            }
                        }
                    }
                    _ => unreachable!(),
                };
                self.ordinal += 1;
                let guards = LivingPacketGuards {
                    local_authoritative: self.local[&key],
                    ticking: true,
                };
                let model = self.models.get_mut(&key).unwrap();
                assert!(model.apply(self.ordinal, packet, guards).unwrap());
                let first = serde_json::to_value(model.frame()).unwrap();
                assert!(!model.apply(self.ordinal, packet, guards).unwrap());
                assert_eq!(first, serde_json::to_value(model.frame()).unwrap());
            }
            _ => panic!("unknown native action {op}"),
        }
    }
}

#[test]
fn original_packet_handlers_and_clientlevel_schedules_match_exact_model_bits() {
    use flate2::read::GzDecoder;
    use sha2::{Digest, Sha256};
    let native: Value = serde_json::from_reader(GzDecoder::new(
        &include_bytes!("../../../data/client_api/entity_tick_oracle.json.gz")[..],
    ))
    .unwrap();
    assert_eq!(
        native["runner_sha256"],
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("../../../scripts/run_entity_tick_oracle.py"))
        )
    );
    let mut stages = 0;
    let mut callbacks = 0;
    for (name, version, source, client, mappings) in [
        (
            "1.16.1",
            crate::MinecraftVersion::Java1_16_1,
            &include_bytes!("../../../scripts/entity_tick_oracle/1.16.1/EntityTickOracle.java")[..],
            "c9abbe8ee4fa490751ca70635340b7cf00db83ff",
            "ddf517a4f6750f4c15189de4e03246ae1f916cf5",
        ),
        (
            "1.21.11",
            crate::MinecraftVersion::Java1_21_11,
            &include_bytes!("../../../scripts/entity_tick_oracle/1.21.11/EntityTickOracle.java")[..],
            "ba2df812c2d12e0219c489c4cd9a5e1f0760f5bd",
            "031a68bebf55d824f66d6573d8c752f0e1bf232a",
        ),
    ] {
        let fixture = &native["versions"][name];
        assert_eq!(
            fixture["harness_sha256"],
            format!("{:x}", Sha256::digest(source))
        );
        assert_eq!(fixture["original_client_jar_sha1"], client);
        assert_eq!(fixture["client_mappings_sha1"], mappings);
        for row in fixture["results"].as_array().unwrap() {
            let mut h = Harness::new(version);
            for step in row["steps"].as_array().unwrap() {
                let label = format!("{name} seed{} stage{}", row["seed"], step["stage"]);
                for action in step["actions"].as_array().unwrap() {
                    h.action(action)
                }
                h.order.begin_tick(h.frozen).unwrap();
                for (i, tick) in step["ticks"].as_array().unwrap().iter().enumerate() {
                    let next = h.order.next_tick().unwrap().unwrap();
                    assert_eq!(
                        next.key,
                        tick["key"].as_u64().unwrap(),
                        "{label}: lifetime order"
                    );
                    assert_eq!(
                        i64::from(next.native_id),
                        tick["native_id"].as_i64().unwrap(),
                        "{label}: numeric ID order"
                    );
                    let model = h.models.get_mut(&next.key).unwrap();
                    if tick["dispatch"] == "ride" {
                        assert_eq!(next.dispatch, LivingTickDispatch::Ride, "{label}");
                    } else if tick["dispatch"] == "player_tick" {
                        assert_eq!(next.dispatch, LivingTickDispatch::Tick, "{label}");
                        match_pose(&tick["before"], model.frame().interpolation.pose, &label);
                        match_pose(&tick["after"], model.frame().interpolation.pose, &label);
                    } else {
                        assert_eq!(next.dispatch, LivingTickDispatch::Tick, "{label}");
                        match_pose(&tick["before"], model.frame().interpolation.pose, &label);
                        let frame = model
                            .step((version == crate::MinecraftVersion::Java1_21_11).then_some(true))
                            .unwrap();
                        match_pose(&tick["after"], frame.interpolation.pose, &label);
                        assert_eq!(
                            u64::from(frame.interpolation.remaining_steps),
                            tick["remaining_steps"].as_u64().unwrap(),
                            "{label}: remaining steps"
                        );
                    }
                    if i == 0 {
                        for action in step["during_tick"].as_array().unwrap() {
                            h.action(action)
                        }
                    }
                    callbacks += 1;
                }
                assert_eq!(
                    h.order.next_tick().unwrap(),
                    None,
                    "{label}: no extra dispatch"
                );
                h.order.finish_tick().unwrap();
                for state in step["states"].as_array().unwrap() {
                    let frame = h.models[&state["key"].as_u64().unwrap()].frame();
                    match_pose(&state["pose"], frame.interpolation.pose, &label);
                    assert_eq!(
                        u64::from(frame.interpolation.remaining_steps),
                        state["remaining_steps"].as_u64().unwrap(),
                        "{label}: retained state"
                    );
                }
                stages += 1;
            }
        }
    }
    assert_eq!(stages, 384);
    assert!(callbacks > 9000);
}

#[test]
fn missing_predicates_and_legacy_mutation_stop_without_inventing_dispatch() {
    use crate::MinecraftVersion as V;
    let mut legacy = LivingEntityTickOrder::new(V::Java1_16_1);
    let mut bad = body(1, 7);
    bad.legacy_ticking_chunk = None;
    legacy.admit(bad.clone()).unwrap();
    legacy.begin_tick(false).unwrap();
    assert_eq!(
        legacy.admit(body(2, 8)),
        Err(LivingTickUnavailable::UnsupportedLegacyMutation)
    );
    assert_eq!(
        legacy.remove_membership(7),
        Err(LivingTickUnavailable::UnsupportedLegacyMutation)
    );
    assert!(legacy.body(1).is_some());
    assert_eq!(
        legacy.update(bad),
        Err(LivingTickUnavailable::UnsupportedLegacyMutation)
    );
    assert_eq!(
        legacy.next_tick(),
        Err(LivingTickUnavailable::MissingPredicate)
    );
    assert_eq!(legacy.next_tick(), Err(LivingTickUnavailable::Invalidated));
    let mut modern = LivingEntityTickOrder::new(V::Java1_21_11);
    modern.admit(body(1, 7)).unwrap();
    modern.begin_tick(false).unwrap();
    assert_eq!(
        modern.begin_tick(false),
        Err(LivingTickUnavailable::TickAlreadyActive)
    );
    assert_eq!(
        modern.finish_tick(),
        Err(LivingTickUnavailable::TickNotFinished)
    );
    modern.invalidate();
    assert_eq!(modern.next_tick(), Err(LivingTickUnavailable::Invalidated));
    assert_eq!(
        modern.admit(body(2, 8)),
        Err(LivingTickUnavailable::Invalidated)
    );
}

#[test]
fn budgets_and_fresh_keys_preserve_retained_bodies_without_eviction() {
    use crate::MinecraftVersion as V;
    let mut order = LivingEntityTickOrder::new(V::Java1_21_11);
    for id in 0..MAX_LIVING_TICK_BODIES as i32 {
        order.admit(body(id as u64 + 1, id)).unwrap()
    }
    assert_eq!(
        order.admit(body(257, 256)),
        Err(LivingTickUnavailable::TooManyBodies)
    );
    assert_eq!(
        order.admit(body(256, 0)),
        Err(LivingTickUnavailable::InconsistentIdentity)
    );
    order.begin_tick(false).unwrap();
    order.remove_membership(42).unwrap();
    order.admit(body(257, 42)).unwrap();
    let mut keys = Vec::new();
    while let Some(i) = order.next_tick().unwrap() {
        keys.push(i.key)
    }
    order.finish_tick().unwrap();
    assert!(keys.contains(&43) && !keys.contains(&257));
    assert!(order.body(43).is_none() && order.body(257).is_some());
    order.begin_tick(false).unwrap();
    let mut keys = Vec::new();
    while let Some(i) = order.next_tick().unwrap() {
        keys.push(i.key)
    }
    order.finish_tick().unwrap();
    assert_eq!(keys.last(), Some(&257));
}

#[test]
fn packet_refusals_and_cancellation_preserve_receipt_and_model_boundaries() {
    use crate::MinecraftVersion as V;
    let initial = LivingInterpolationPose {
        position: [0.1, 65.2, -0.3],
        rotation: [0., 0.],
    };
    let guards = LivingPacketGuards {
        local_authoritative: false,
        ticking: true,
    };
    let mut model = LivingPacketModel::new(V::Java1_21_11, initial, 7).unwrap();
    let packet = LivingMotionPacket::Relative {
        delta: [512, 0, -512],
        rotation: Some([90., -45.]),
        ground: true,
    };
    assert!(model.apply(8, packet, guards).unwrap());
    let frame = model.frame();
    assert_eq!(
        frame.interpolation.pose.position.map(f64::to_bits),
        initial.position.map(f64::to_bits)
    );
    assert_ne!(frame.codec_base, initial.position);
    model.step(Some(true)).unwrap();
    assert_eq!(model.frame().applied_ordinal, 8);
    assert_eq!(model.frame().codec_base, frame.codec_base);
    let prior = serde_json::to_value(model.frame()).unwrap();
    assert_eq!(
        model.apply(6, packet, guards),
        Err(LivingPacketUnavailable::OutOfOrder)
    );
    assert_eq!(
        model.apply(9, LivingMotionPacket::Velocity([f64::NAN, 0., 0.]), guards),
        Err(LivingPacketUnavailable::NonFiniteState)
    );
    assert_eq!(
        model.apply(
            9,
            LivingMotionPacket::LegacyTeleport {
                position: [0.; 3],
                rotation: [0.; 2],
                ground: true
            },
            guards
        ),
        Err(LivingPacketUnavailable::UnsupportedPacket)
    );
    assert_eq!(serde_json::to_value(model.frame()).unwrap(), prior);
    model.cancel().unwrap();
    assert_eq!(model.frame().applied_ordinal, 8);
    model.invalidate();
    assert_eq!(
        model.apply(9, packet, guards),
        Err(LivingPacketUnavailable::Invalidated)
    );
    assert!(matches!(
        model.step(Some(true)),
        Err(LivingPacketUnavailable::Invalidated)
    ));
}

fn receipts(version: crate::MinecraftVersion) -> (SpawnLedger, SessionStamp, EntityHistory) {
    let stamp = SessionStamp {
        version,
        connection_id: 97,
        world_generation: 3,
    };
    let mut ledger = SpawnLedger::default();
    ledger.history_context(version, 3, 1);
    ledger
        .insert(
            version,
            NativeSpawn {
                id: 7,
                uuid: None,
                type_id: None,
                dedicated_type_name: Some("cow"),
                position: [0.1, 65.2, -0.3],
                living: Some(true),
            },
            1,
            16,
        )
        .unwrap();
    ledger.initialize_motion(
        7,
        NativeSpawnMotion {
            position: Some([0.1, 65.2, -0.3]),
            rotation: Some([0., 0.]),
            head_yaw: Some(0.),
            velocity: Some([0.; 3]),
        },
        1,
    );
    let page = ledger.history_after(stamp, 1, None, 10).unwrap();
    (ledger, stamp, page)
}

#[test]
fn original_operation_receipts_replay_once_without_reusing_sticky_rotation() {
    use crate::MinecraftVersion as V;
    for version in [V::Java1_16_1, V::Java1_21_11] {
        let (mut ledger, stamp, initial) = receipts(version);
        let mut replay = LivingEntityPacketReplay::from_spawn(&initial.records[0]).unwrap();
        let guards = LivingPacketGuards {
            local_authoritative: false,
            ticking: true,
        };
        ledger.history_context(version, 3, 2);
        ledger.receive_motion(
            version,
            7,
            NativeMotion::Relative {
                delta: [512, 0, 0],
                rotation: Some([90., -45.]),
                ground: true,
            },
            2,
        );
        let first = ledger
            .history_after(stamp, 2, Some(initial.next_cursor), 10)
            .unwrap();
        assert_eq!(replay.apply_page(&first, guards).unwrap(), 1);
        let frozen = serde_json::to_value(replay.frame()).unwrap();
        assert_eq!(replay.apply_page(&first, guards).unwrap(), 0);
        assert_eq!(serde_json::to_value(replay.frame()).unwrap(), frozen);
        replay
            .step((version == V::Java1_21_11).then_some(true))
            .unwrap();
        let before = replay.frame().model.interpolation.pose;
        ledger.history_context(version, 3, 3);
        ledger.receive_motion(
            version,
            7,
            NativeMotion::Relative {
                delta: [0, 0, 1],
                rotation: None,
                ground: false,
            },
            3,
        );
        let second = ledger
            .history_after(stamp, 3, Some(first.next_cursor), 10)
            .unwrap();
        let EntityHistoryKind::Motion(m) = &second.records[0].kind else {
            panic!()
        };
        assert_eq!(
            m.rotation.as_ref().unwrap().source,
            crate::client::ValueSource::Received { sequence: 2 }
        );
        let packet = m.packet.as_ref().unwrap();
        assert_eq!(
            packet.source,
            crate::client::ValueSource::Received { sequence: 3 }
        );
        assert!(matches!(
            packet.value,
            EntityMotionPacket::Relative { rotation: None, .. }
        ));
        assert_eq!(replay.apply_page(&second, guards).unwrap(), 1);
        let frame = replay.frame();
        let target = frame.model.interpolation.target.unwrap();
        assert_eq!(
            target.rotation[0].to_bits(),
            (if version == V::Java1_16_1 {
                before.rotation[0]
            } else {
                90.
            })
            .to_bits()
        );
        assert_eq!(
            frame.model.interpolation.pose.position.map(f64::to_bits),
            before.position.map(f64::to_bits)
        );
        assert_eq!(frame.applied_record, second.records[0].ordinal);
        let latest = ledger.capture_motion(stamp, m.entity.id, 3).unwrap();
        assert_eq!(
            serde_json::to_value(latest.packet).unwrap(),
            serde_json::to_value(m.packet.clone()).unwrap()
        );
    }
}

#[test]
fn replacement_world_gap_partial_and_correction_fences_irreversibly_stop_replay() {
    use crate::MinecraftVersion as V;
    let guards = LivingPacketGuards {
        local_authoritative: false,
        ticking: true,
    };
    for version in [V::Java1_16_1, V::Java1_21_11] {
        for failure in ["replacement", "world", "partial", "gap", "remove"] {
            let (mut ledger, stamp, initial) = receipts(version);
            let mut replay = LivingEntityPacketReplay::from_spawn(&initial.records[0]).unwrap();
            ledger.history_context(version, 3, 2);
            match failure {
                "replacement" => {
                    ledger
                        .insert(
                            version,
                            NativeSpawn {
                                id: 7,
                                uuid: None,
                                type_id: None,
                                dedicated_type_name: Some("cow"),
                                position: [0.; 3],
                                living: Some(true),
                            },
                            2,
                            16,
                        )
                        .unwrap();
                    ledger.initialize_motion(
                        7,
                        NativeSpawnMotion {
                            position: Some([0.; 3]),
                            rotation: Some([0.; 2]),
                            ..Default::default()
                        },
                        2,
                    );
                }
                "partial" => ledger.receive_metadata(7, [(6, EntityDataValue::Pose(0))], false, 2),
                "remove" => ledger.remove(7),
                "world" | "gap" => ledger.receive_motion(version, 7, NativeMotion::HeadYaw(1.), 2),
                _ => unreachable!(),
            }
            let mut page = ledger
                .history_after(stamp, 2, Some(initial.next_cursor), 10)
                .unwrap();
            if failure == "world" {
                page.session.world_generation += 1;
            }
            if failure == "gap" {
                page.gap = Some(EntityHistoryGap {
                    after: 1,
                    dropped_through: 2,
                });
            }
            let error = replay.apply_page(&page, guards).unwrap_err();
            assert_eq!(
                error,
                match failure {
                    "partial" => LivingReplayUnavailable::IncompleteEntityInput,
                    "gap" => LivingReplayUnavailable::HistoryGap,
                    _ => LivingReplayUnavailable::LifetimeChanged,
                }
            );
            assert!(!replay.frame().valid);
            assert_eq!(replay.frame().applied_record, initial.records[0].ordinal);
            assert_eq!(
                replay.apply_page(&page, guards),
                Err(LivingReplayUnavailable::Invalidated)
            );
            assert!(matches!(
                replay.step((version == V::Java1_21_11).then_some(true)),
                Err(LivingReplayUnavailable::Invalidated)
            ));
        }
    }
    let (mut ledger, stamp, initial) = receipts(V::Java1_21_11);
    let mut replay = LivingEntityPacketReplay::from_spawn(&initial.records[0]).unwrap();
    ledger.history_context(V::Java1_21_11, 3, 2);
    ledger.receive_motion(
        V::Java1_21_11,
        7,
        NativeMotion::Relative {
            delta: [512, 0, 0],
            rotation: None,
            ground: true,
        },
        2,
    );
    ledger.history_context(V::Java1_21_11, 3, 3);
    ledger.receive_motion(
        V::Java1_21_11,
        7,
        NativeMotion::Correction {
            change: EntityPositionCorrection {
                position: [1., 2., 3.],
                delta: [0.; 3],
                rotation: [0.; 2],
                flags: 1,
            },
            ground: false,
        },
        3,
    );
    let page = ledger
        .history_after(stamp, 3, Some(initial.next_cursor), 10)
        .unwrap();
    assert_eq!(
        replay.apply_page(&page, guards),
        Err(LivingReplayUnavailable::Packet(
            LivingPacketUnavailable::UnsupportedPacket
        ))
    );
    assert_eq!(replay.frame().applied_record, initial.records[0].ordinal);
    assert_eq!(replay.frame().model.codec_base, [0.1, 65.2, -0.3]);
    let EntityHistoryKind::Motion(m) = &page.records[1].kind else {
        panic!()
    };
    assert!(matches!(
        m.packet.as_ref().unwrap().value,
        EntityMotionPacket::Correction {
            change: EntityPositionCorrection { flags: 1, .. },
            ..
        }
    ));
}

#[test]
fn actual_retention_loss_and_unreceived_spawn_provenance_are_not_model_baselines() {
    use crate::MinecraftVersion as V;
    let guards = LivingPacketGuards {
        local_authoritative: false,
        ticking: true,
    };
    for version in [V::Java1_16_1, V::Java1_21_11] {
        let (mut ledger, stamp, initial) = receipts(version);
        let mut replay = LivingEntityPacketReplay::from_spawn(&initial.records[0]).unwrap();
        let mut forged = initial.records[0].clone();
        let EntityHistoryKind::Spawn(m) = &mut forged.kind else {
            panic!()
        };
        m.position.as_mut().unwrap().source = crate::client::ValueSource::Predicted;
        assert!(matches!(
            LivingEntityPacketReplay::from_spawn(&forged),
            Err(LivingReplayUnavailable::MissingSpawn)
        ));
        for sequence in 2..MAX_ENTITY_HISTORY_RECORDS as u64 + 3 {
            ledger.history_context(version, 3, sequence);
            ledger.receive_motion(version, 7, NativeMotion::HeadYaw(0.), sequence);
        }
        let page = ledger
            .history_after(
                stamp,
                MAX_ENTITY_HISTORY_RECORDS as u64 + 2,
                Some(initial.next_cursor),
                MAX_ENTITY_HISTORY_READ,
            )
            .unwrap();
        assert!(page.gap.is_some());
        assert_eq!(
            replay.apply_page(&page, guards),
            Err(LivingReplayUnavailable::HistoryGap)
        );
        assert!(!replay.frame().valid);
    }
}
