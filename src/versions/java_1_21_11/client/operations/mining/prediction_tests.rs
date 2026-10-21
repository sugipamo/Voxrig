//! One-client motion contract; the loopback peer supplies no position echo.
use super::*;

async fn settle(f: &mut Fixture, controls: &[SurvivalControl]) -> SurvivalMotionRecord {
    let start = f.api.start_predicted_survival_path(controls).await.unwrap();
    assert_eq!(start.contract, SurvivalMotionContract::Predicted);
    for _ in controls {
        assert_eq!(
            read_packet(&mut f.peer, None).await.unwrap().0,
            ids::play_serverbound::PLAYER_INPUT
        );
        assert_eq!(
            read_packet(&mut f.peer, None).await.unwrap().0,
            ids::play_serverbound::POSITION_LOOK
        );
    }
    timeout(Duration::from_secs(1), async {
        loop {
            let r = f.api.survival_motion().await.unwrap();
            if r.status != SurvivalMotionStatus::Running {
                break r;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn predicted_motion_retains_provenance_and_matches_explicit_hypothetical_contract() {
    let mut f = Fixture::new().await;
    f.session
        .state
        .lock()
        .await
        .world
        .seed_replay_cell(TARGET, 0);
    let region = crate::Region {
        min: [-2, -1, -2],
        max: [4, 6, 2],
    };
    let captured = f.api.capture_survival_scene(region).await.unwrap();
    let controls: Vec<_> = (0..16)
        .map(|t| SurvivalControl {
            yaw: 0.,
            input: SurvivalInput {
                forward: i8::from(t < 4),
                ..Default::default()
            },
        })
        .collect();
    let future = captured
        .scenario_with_motion_contract(SurvivalMotionContract::Predicted)
        .after_path(&controls)
        .unwrap();
    let independent_future = captured.scenario().after_path(&controls).unwrap();
    let r = settle(&mut f, &controls).await;
    assert_eq!(r.status, SurvivalMotionStatus::Predicted);
    assert!(r.observed.is_none() && r.initial_watch.is_none() && r.final_watch.is_none());
    assert!(r.observer_connection_id.is_none());
    let standing = f.api.standing_context().await.unwrap();
    assert!(matches!(
        standing.position_basis,
        StandingPositionBasis::Predicted { .. }
    ));
    future
        .aim_requirement()
        .validate_standing(&standing)
        .unwrap();
    assert!(
        independent_future
            .aim_requirement()
            .validate_standing(&standing)
            .is_err()
    );
    assert_eq!(
        f.session
            .state
            .lock()
            .await
            .motion
            .received_pose
            .as_ref()
            .unwrap()
            .position,
        captured.source().position
    );
    let next = vec![
        SurvivalControl {
            yaw: -90.,
            input: Default::default()
        };
        3
    ];
    let preview = f.api.preview_survival_path(&next).await.unwrap();
    assert_eq!(
        preview.initial_frame,
        future.preview_path(&next).unwrap().initial_frame
    );
    assert_eq!(preview.frames, future.preview_path(&next).unwrap().frames);
    let fresh = f.api.capture_survival_scene(region).await.unwrap();
    assert!(matches!(
        fresh.scenario().aim_requirement(),
        HypotheticalAimRequirement::PredictedEndpoint { .. }
    ));
    let json = serde_json::to_value(&standing.position_basis).unwrap();
    assert_eq!(json["kind"], "predicted");
    assert!(json.get("observed").is_none());
    assert_eq!(
        json["planning_reserve"],
        serde_json::json!([0.0625, 0., 0.0625])
    );
    f.api.look([0., 0.]).await.unwrap();
    assert_eq!(
        read_packet(&mut f.peer, None).await.unwrap().0,
        ids::play_serverbound::LOOK
    );
    f.stop().await;
}

#[tokio::test]
async fn predicted_motion_refuses_correction_impulse_and_changed_generation_before_more_sends() {
    let mut f = Fixture::new().await;
    let controls = [SurvivalControl {
        yaw: 0.,
        input: Default::default(),
    }; 3];
    assert_eq!(
        settle(&mut f, &controls).await.status,
        SurvivalMotionStatus::Predicted
    );
    let (motion, player, generation) = {
        let s = f.session.state.lock().await;
        (
            s.motion.clone(),
            s.operations.local_player.clone(),
            s.loading.generation,
        )
    };
    for problem in ["correction", "impulse", "generation"] {
        {
            let mut s = f.session.state.lock().await;
            s.motion = motion.clone();
            s.operations.local_player = player.clone();
            s.loading.generation = generation;
            match problem {
                "correction" => {
                    let mut pose = s.motion.received_pose.clone().unwrap();
                    pose.receive_sequence += 1;
                    s.motion.receive(pose);
                }
                "impulse" => s.operations.local_player.velocity.as_mut().unwrap().value[0] = 0.1,
                _ => s.loading.generation += 1,
            }
        }
        assert!(f.api.standing_context().await.is_err(), "{problem}");
        assert!(
            f.api
                .start_predicted_survival_path(&controls)
                .await
                .is_err(),
            "{problem}"
        );
        assert!(f.api.select_hotbar(1).await.is_err(), "{problem}");
    }
    assert!(
        timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    f.stop().await;
}

#[tokio::test]
async fn predicted_motion_checks_current_support_and_terminal_geometry_without_replaying() {
    let mut f = Fixture::new().await;
    let controls = [SurvivalControl {
        yaw: 0.,
        input: Default::default(),
    }; 3];
    assert_eq!(
        settle(&mut f, &controls).await.status,
        SurvivalMotionStatus::Predicted
    );
    f.session
        .state
        .lock()
        .await
        .world
        .seed_replay_cell([0, 0, 0], 0);
    assert!(f.api.preview_survival_path(&controls).await.is_err());
    assert!(
        f.api
            .place_survival_cube([0, 0, 0], crate::BlockFace::Up)
            .await
            .is_err()
    );
    f.session
        .state
        .lock()
        .await
        .world
        .seed_replay_cell([0, 1, 0], 1);
    assert!(f.api.standing_context().await.is_err());
    assert!(
        timeout(Duration::from_millis(20), read_packet(&mut f.peer, None))
            .await
            .is_err()
    );
    f.stop().await;
}

#[tokio::test]
async fn interrupted_predicted_writer_never_yields_a_continuation_candidate() {
    let f = Fixture::new().await;
    let writer = f.session.writer.lock().await;
    let controls = [SurvivalControl {
        yaw: 0.,
        input: Default::default(),
    }; 3];
    f.api
        .start_predicted_survival_path(&controls)
        .await
        .unwrap();
    wait_motion_writer_blocked(&f).await;
    f.session.stop();
    drop(writer);
    let record = timeout(Duration::from_secs(1), async {
        loop {
            let r = f.api.survival_motion().await.unwrap();
            if r.status == SurvivalMotionStatus::RequiresInspection {
                break r;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(record.attempted_tick, 1);
    assert_eq!(record.dispatched_ticks, 0);
    assert!(!record.status.is_continuation_candidate());
    assert!(f.api.standing_context().await.is_err());
    f.stop().await;
}

// These scripted packets exercise client contracts, not vanilla movement,
// collision, jump height, delayed mining, or attribution of a placed block.
#[tokio::test]
async fn synthetic_motion_permits_placement_with_exact_provenance() {
    for predicted in [false, true] {
        for scenario in ["walk", "jump", "wall_retreat"] {
            let jump = scenario == "jump";
            let wall = scenario == "wall_retreat";
            eprintln!("synthetic contract predicted={predicted}, case={scenario}");
            let mut f = Fixture::new().await;
            let mut observer = Fixture::new_id(43).await;
            observer.observe_mover().await;
            f.change(TARGET, &native("air")).await;
            if wall {
                f.change([0, 1, 2], &native("stone")).await;
            }
            let mut stack = vec![0, 2];
            stack.extend(36i16.to_be_bytes());
            stack.push(1);
            put_varint(&mut stack, default_item("dirt", 1).unwrap().item_id);
            stack.extend([0, 0]);
            f.receive(ids::play_clientbound::SET_SLOT, &stack).await;
            tokio::time::pause();
            // A blocking task inhibits Tokio's automatic clock jumps while
            // kernel TCP readiness is pending. Dropping the sender also releases
            // it on panic; no real sleep or unbounded stdin worker is involved.
            let (clock_release, clock_wait) = std::sync::mpsc::channel::<()>();
            let clock = tokio::task::spawn_blocking(move || clock_wait.recv());
            let hand_sequence = f.session.state.lock().await.sequence;
            let received_pose = f
                .session
                .state
                .lock()
                .await
                .motion
                .received_pose
                .clone()
                .unwrap();
            let controls: Vec<_> = (0..if wall { 60 } else { 30 })
                .map(|tick| SurvivalControl {
                    yaw: 0.0,
                    input: SurvivalInput {
                        forward: if wall {
                            i8::from(tick < 20) - i8::from((30..34).contains(&tick))
                        } else {
                            i8::from(!jump && tick < 4)
                        },
                        jump: jump && tick == 0,
                        ..Default::default()
                    },
                })
                .collect();
            let run = if predicted {
                f.api
                    .start_predicted_survival_path(&controls)
                    .await
                    .unwrap()
            } else {
                let inputs: Vec<_> = controls.iter().map(|c| c.input).collect();
                f.api
                    .start_survival_motion(0.0, &inputs, &observer.api)
                    .await
                    .unwrap()
            };
            assert_eq!(
                run.preview
                    .frames
                    .iter()
                    .any(|frame| frame.horizontal_collision),
                wall
            );
            if jump {
                assert!(
                    run.preview
                        .frames
                        .iter()
                        .any(|frame| frame.position[1] > 1.5)
                );
            }
            // Let the owner register its first interval before advancing past
            // the 50 ms tick (including timer-wheel rounding).
            tokio::task::yield_now().await;
            for (control, frame) in controls.iter().zip(&run.preview.frames) {
                tokio::time::advance(Duration::from_millis(51)).await;
                let bits = u8::from(control.input.forward > 0)
                    | (u8::from(control.input.forward < 0) << 1)
                    | (u8::from(control.input.jump) << 4);
                assert_eq!(
                    read_packet(&mut f.peer, None).await.unwrap(),
                    (ids::play_serverbound::PLAYER_INPUT, vec![bits])
                );
                let mut position = Vec::new();
                for v in frame.position {
                    position.extend(v.to_be_bytes());
                }
                position.extend([0; 8]); // yaw and pitch.
                position
                    .push(u8::from(frame.on_ground) | (u8::from(frame.horizontal_collision) << 1));
                assert_eq!(
                    read_packet(&mut f.peer, None).await.unwrap(),
                    (ids::play_serverbound::POSITION_LOOK, position)
                );
            }
            timeout(Duration::from_secs(1), async {
                while f.api.survival_motion().await.unwrap().status == SurvivalMotionStatus::Running
                {
                    tokio::time::advance(Duration::from_millis(1)).await;
                }
            })
            .await
            .unwrap();
            let endpoint = run.preview.frames.last().unwrap().position;
            if !predicted {
                assert_eq!(
                    f.api.survival_motion().await.unwrap().status,
                    SurvivalMotionStatus::AwaitingObservation
                );
                // A matching cached spawn cannot release placement. The exact
                // endpoint arrives through the observer's real receiver later.
                assert!(
                    f.api
                        .place_survival_cube([2, 0, 0], crate::BlockFace::Up)
                        .await
                        .is_err()
                );
                observer.mover_position(endpoint).await;
                timeout(Duration::from_secs(1), async {
                    while f.api.survival_motion().await.unwrap().status
                        == SurvivalMotionStatus::AwaitingObservation
                    {
                        tokio::time::advance(Duration::from_millis(50)).await;
                    }
                })
                .await
                .unwrap();
            }
            let standing = f.api.standing_context().await.unwrap();
            match &standing.position_basis {
                StandingPositionBasis::Predicted {
                    run_id,
                    received_pose_sequence,
                    ..
                } if predicted => {
                    assert_eq!(*run_id, run.run_id);
                    assert_eq!(*received_pose_sequence, received_pose.receive_sequence);
                    let settled = f.api.survival_motion().await.unwrap();
                    assert!(settled.observed.is_none() && settled.observer_connection_id.is_none());
                }
                StandingPositionBasis::PredictedAndObserved {
                    run_id,
                    observer_connection_id,
                    observed,
                    ..
                } if !predicted => {
                    assert_eq!(*run_id, run.run_id);
                    assert_eq!(*observer_connection_id, observer.session.id);
                    assert_eq!(observed.position, endpoint);
                }
                other => panic!("unexpected standing provenance: {other:?}"),
            }
            assert_eq!(
                f.session
                    .state
                    .lock()
                    .await
                    .motion
                    .received_pose
                    .as_ref()
                    .unwrap()
                    .receive_sequence,
                received_pose.receive_sequence
            );
            assert!(!f.api.player_state().await.unwrap().position_from_server);
            let d: [f64; 3] =
                std::array::from_fn(|i| [2.5, 1.0, 0.5][i] - standing.eye_position[i]);
            let rotation = [
                (-d[0]).atan2(d[2]).to_degrees() as f32,
                (-d[1]).atan2(d[0].hypot(d[2])).to_degrees() as f32,
            ];
            f.api.look(rotation).await.unwrap();
            let mut look = Vec::new();
            for v in rotation {
                look.extend(v.to_be_bytes());
            }
            look.push(1);
            assert_eq!(
                read_packet(&mut f.peer, None).await.unwrap(),
                (ids::play_serverbound::LOOK, look)
            );
            let intent = f
                .api
                .place_survival_cube([2, 0, 0], crate::BlockFace::Up)
                .await
                .unwrap();
            assert_eq!(intent.position, endpoint);
            assert_eq!(intent.held_receive_sequence, hand_sequence);
            assert_eq!(intent.target, [2, 1, 0]);
            let mut bytes = vec![0]; // Main hand.
            bytes.extend(pack_position([2, 0, 0]).to_be_bytes());
            bytes.push(1); // Up.
            // Native lookup-table trigonometry quantizes the intended center.
            assert!((intent.cursor[0] - 0.5).abs() < 1e-3, "{:?}", intent.cursor);
            assert_eq!(intent.cursor[1], 1.0);
            assert!((intent.cursor[2] - 0.5).abs() < 1e-3, "{:?}", intent.cursor);
            for v in intent.cursor {
                bytes.extend(v.to_be_bytes());
            }
            bytes.extend([0, 0]);
            put_varint(&mut bytes, intent.sequence);
            assert_eq!(
                read_packet(&mut f.peer, None).await.unwrap(),
                (ids::play_serverbound::BLOCK_PLACE, bytes)
            );
            // ACK + consumed material still lack the target-specific receipt.
            let mut ack = vec![];
            put_varint(&mut ack, intent.sequence);
            f.receive(ids::play_clientbound::ACKNOWLEDGE_PLAYER_DIGGING, &ack)
                .await;
            f.receive(ids::play_clientbound::SET_SLOT, &[0, 3, 0, 36, 0])
                .await;
            assert!(matches!(
                f.api.observe_survival_placement(&intent).await.unwrap(),
                PlacementStatus::Pending { .. }
            ));
            f.change(intent.target, &native("dirt")).await;
            let PlacementStatus::ObservedPlaced { observation } =
                f.api.observe_survival_placement(&intent).await.unwrap()
            else {
                panic!("expected both synthetic placement receipts");
            };
            assert_eq!(observation.intent, intent);
            assert_eq!(observation.held_after, InventorySlot::Empty);
            assert!(observation.inventory_receive_sequence > intent.after_sequence);
            assert!(observation.target_receive_sequence > observation.inventory_receive_sequence);
            assert_eq!(observation.acknowledged_sequence, intent.sequence);
            {
                let no_packet = timeout(Duration::from_millis(5), read_packet(&mut f.peer, None));
                tokio::pin!(no_packet);
                std::future::poll_fn(|cx| {
                    assert!(no_packet.as_mut().poll(cx).is_pending());
                    std::task::Poll::Ready(())
                })
                .await;
                tokio::time::advance(Duration::from_millis(6)).await;
                assert!(no_packet.await.is_err());
            }
            clock_release.send(()).unwrap();
            clock.await.unwrap().unwrap();
            observer.stop().await;
            f.stop().await;
            tokio::time::resume();
        }
    }
}
