use super::*;
use crate::client::{
    GameMode,
    adapter::DigOps,
    operational_dig::{self as dig, Action, Dispatch, Inputs},
};

impl Bot {
    async fn dig_snapshot_unlocked(&self, target: [i32; 3]) -> Result<Inputs> {
        let player = self.common_player_unlocked().await?;
        let (eye, _) = dig::eye_cell(&player)?;
        let on_ground = self.player.lock().await.on_ground;
        let vertical_velocity = Some(self.motion.lock().await.velocity.y);
        let world = self.world.lock().await;
        let block = |at: [i32; 3]| -> Result<crate::NativeBlockState> {
            crate::versions::java_1_16_1::native_state(
                world
                    .block(at[0], at[1], at[2])
                    .ok_or_else(|| dig::unavailable("dig cells are not loaded"))?,
            )
        };
        dig::inputs(
            player,
            block(target)?,
            on_ground,
            vertical_velocity,
            block(eye)?,
            block([eye[0], eye[1] + 1, eye[2]])?,
        )
    }
}

impl DigOps for Bot {
    fn dig_work(&self) -> Result<crate::client::ledger::WorkGuard> {
        self.connection.ledger().try_work()
    }
    fn dig_coordinator(&self) -> Arc<dig::Coordinator> {
        self.operational_dig.clone()
    }
    async fn dig_snapshot(&self, target: [i32; 3]) -> Result<Inputs> {
        self.wait_until_ready().await?;
        let _gate = self.coherent_state_gate.lock().await;
        self.dig_snapshot_unlocked(target).await
    }
    async fn dig_dispatch(
        &self,
        initial: &Inputs,
        target: [i32; 3],
        face: crate::client::BlockFace,
        action: Action,
        cancelled: &AtomicBool,
    ) -> Result<Dispatch> {
        let _gate = self.coherent_state_gate.lock().await;
        let player = self.common_player_unlocked().await?;
        let same = player.session == initial.player.session
            && player.game_mode == Some(GameMode::Survival);
        if matches!(action, Action::Abort) {
            if !same {
                return Ok(Dispatch::ContextChanged);
            }
            if self.is_stopped()
                || self.connection_state() != ConnectionState::Ready
                || self.common_receipts.lock().await.pending_dispatch
            {
                return Err(dig::unavailable(
                    "dig cleanup connection unavailable or dispatch unresolved",
                ));
            }
            let mut payload = vec![1];
            payload.extend(
                BlockPos {
                    x: target[0],
                    y: target[1],
                    z: target[2],
                }
                .packed()
                .to_be_bytes(),
            );
            payload.push(face as u8);
            self.common_receipts.lock().await.pending_dispatch = true;
            let outcome = self
                .connection
                .dispatch_operation(
                    self.operation_context(0),
                    OperationClass::Cleanup,
                    0x1b,
                    &payload,
                )
                .await
                .map_err(dig::unavailable)?;
            // Keep uncertainty retained just as execute_common_unlocked does on
            // error. Only a proven complete write can resolve this dispatch.
            confirm_abort_write(outcome)?;
            self.common_receipts.lock().await.pending_dispatch = false;
            return Ok(Dispatch::Sent(None));
        }
        if cancelled.load(Ordering::Acquire) {
            return Err(dig::unavailable("dig caller cancelled before dispatch"));
        }
        if !same {
            return Err(dig::unavailable(
                "dig original world or mode changed before dispatch",
            ));
        }
        let state = self
            .world
            .lock()
            .await
            .block(target[0], target[1], target[2])
            .map(crate::versions::java_1_16_1::native_state)
            .transpose()?;
        if state.as_ref() != Some(&initial.state) {
            return Ok(Dispatch::Changed(state));
        }
        let current = self.dig_snapshot_unlocked(target).await?;
        dig::validate(initial, &current, action)?;
        let action = match action {
            Action::Start => crate::client::operations::Action::DigStart(target, face),
            Action::Finish { .. } => crate::client::operations::Action::DigFinish(target, face),
            Action::Abort => unreachable!(),
        };
        self.execute_common_unlocked(GameMode::Survival, action, None)
            .await
            .map(Dispatch::Sent)
    }
    async fn dig_dispatch_tracked(
        &self,
        initial: &Inputs,
        target: [i32; 3],
        face: crate::client::BlockFace,
        action: Action,
        operation: &Arc<crate::client::dig_operation::OperationState>,
    ) -> Result<Dispatch> {
        // Capacity first; final resnapshot/validation then commit/enqueue with no await.
        let permit = self.connection.reserve_dig().await?;
        let _gate = self.coherent_state_gate.lock().await;
        let player = self.common_player_unlocked().await?;
        if player.session != initial.player.session || player.game_mode != Some(GameMode::Survival)
        {
            operation.context_changed();
            if matches!(action, Action::Abort) {
                return Ok(Dispatch::ContextChanged);
            }
            return Err(dig::unavailable(
                "dig original world or mode changed before dispatch",
            ));
        }
        if self.is_stopped() || self.connection_state() != ConnectionState::Ready {
            return Err(crate::Error::new(
                crate::ErrorKind::Disconnected,
                anyhow::anyhow!("dig connection unavailable"),
            ));
        }
        if self.common_receipts.lock().await.pending_dispatch {
            return Err(dig::unavailable("dig dispatch remains unresolved"));
        }
        if !matches!(action, Action::Abort) {
            self.common_motion_admission_inner(false).await?;
            let state = self
                .world
                .lock()
                .await
                .block(target[0], target[1], target[2])
                .map(crate::versions::java_1_16_1::native_state)
                .transpose()?;
            if state.as_ref() != Some(&initial.state) {
                if operation.cancelled.load(Ordering::Acquire) {
                    operation.cooperative();
                    return Ok(Dispatch::Cancelled);
                }
                return Ok(Dispatch::Changed(state));
            }
            let current = self.dig_snapshot_unlocked(target).await?;
            dig::validate(initial, &current, action)?;
            self.common_reach(target, [0.5; 3]).await?;
            self.observe_stop_context().await;
        }
        let mut payload = vec![action.status() as u8];
        payload.extend(
            BlockPos {
                x: target[0],
                y: target[1],
                z: target[2],
            }
            .packed()
            .to_be_bytes(),
        );
        payload.push(face as u8);
        let mut receipts = self.common_receipts.lock().await;
        let class = if matches!(action, Action::Abort) {
            OperationClass::Cleanup
        } else {
            OperationClass::Normal
        };
        let Some(reply) = permit.enqueue(
            self.operation_context(0),
            class,
            payload,
            operation.clone(),
            action,
        ) else {
            return Ok(Dispatch::Cancelled);
        };
        receipts.pending_dispatch = true;
        drop(receipts);
        let result = reply.wait().await;
        if matches!(result, Ok(Dispatch::Sent(_)))
            || operation.evidence(action) == crate::client::DigWriteEvidence::NotSent
        {
            self.common_receipts.lock().await.pending_dispatch = false;
        }
        result
    }
}

fn confirm_abort_write(outcome: crate::DispatchOutcome) -> Result<()> {
    #[allow(unreachable_patterns)] // DispatchOutcome is non_exhaustive.
    match outcome {
        crate::DispatchOutcome::Dispatched | crate::DispatchOutcome::Acknowledged => Ok(()),
        crate::DispatchOutcome::DeliveryUnknown => Err(crate::Error::new(
            crate::ErrorKind::UncertainDispatch,
            anyhow::anyhow!("dig cleanup ABORT delivery unknown"),
        )),
        crate::DispatchOutcome::Rejected => Err(crate::Error::new(
            crate::ErrorKind::Rejected,
            anyhow::anyhow!("dig cleanup ABORT rejected"),
        )),
        _ => Err(dig::unavailable(format!(
            "dig cleanup ABORT write unconfirmed: {outcome:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{ObservedValue, SlotKnowledge, ValueSource};

    #[test]
    fn abort_requires_a_proven_complete_write() {
        assert!(confirm_abort_write(crate::DispatchOutcome::Dispatched).is_ok());
        assert!(confirm_abort_write(crate::DispatchOutcome::Acknowledged).is_ok());
        assert_eq!(
            confirm_abort_write(crate::DispatchOutcome::DeliveryUnknown)
                .unwrap_err()
                .kind(),
            crate::ErrorKind::UncertainDispatch
        );
        assert_eq!(
            confirm_abort_write(crate::DispatchOutcome::Rejected)
                .unwrap_err()
                .kind(),
            crate::ErrorKind::Rejected
        );
    }

    #[tokio::test]
    async fn abort_write_failure_retains_failed_cleanup_and_success_is_aborted() {
        use crate::client::{DigCleanupStatus, ledger::testing::Control};
        use tokio::time::{Duration, timeout};

        // Exercise partial-frame and flush failures through DispatchPrimitive,
        // which replies with Ok(DeliveryUnknown), plus a complete-write control.
        for failure in [Some(false), Some(true), None] {
            let (bot, mut packets, _release, server) =
                super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
            super::super::common_motion::tests::seed_motion(&bot).await;
            {
                let mut receipts = bot.common_receipts.lock().await;
                receipts.inventory.window_id = Some(0);
                receipts.inventory.slots = (0..46)
                    .map(|_| Some(crate::client::received(SlotKnowledge::Empty, 1)))
                    .collect();
                receipts.selected_hotbar = Some(crate::client::received(0, 1));
            }
            let target = [9, 65, 8];
            let dirt =
                crate::client::registry::Registry::for_version(crate::MinecraftVersion::Java1_16_1)
                    .resolve_block_state(&crate::NativeBlockState {
                        name: "minecraft:dirt".into(),
                        properties: Default::default(),
                    })
                    .unwrap()
                    .value();
            bot.world
                .lock()
                .await
                .set_block_for_test(BlockPos { x: 9, y: 65, z: 8 }, dirt);
            let session = bot.dig_snapshot(target).await.unwrap().player.session;
            let client = crate::Client::from_java_1_16_1(bot.clone());
            let owner = client.clone();
            let caller =
                tokio::spawn(
                    async move { owner.survival().dig(target, crate::BlockFace::Up).await },
                );
            let (id, start) = timeout(Duration::from_secs(2), packets.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!((id, start[0]), (0x1b, 0));
            let control = Control::default();
            control.release(usize::MAX, true);
            if let Some(flush) = failure {
                control.fail(if flush { None } else { Some(1) }, flush);
            }
            {
                let _gate = bot.coherent_state_gate.lock().await;
                bot.writer.lock().await.inner.test_replace(control.writer());
                // Refuse STOP before writing, so the failing write is ABORT.
                bot.common_receipts.lock().await.selected_hotbar =
                    Some(crate::client::received(1, 2));
            }
            let error = timeout(Duration::from_secs(3), caller)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err();
            let record = client.survival().dig_cleanup_record().unwrap();
            assert_eq!(record.session, session);
            assert_eq!(record.target, target);
            assert!(
                record
                    .reason
                    .contains("dig selected hand or timing prerequisites changed")
            );
            assert_eq!(record.interaction_sequence, None);
            assert_eq!(
                bot.common_receipts.lock().await.pending_dispatch,
                failure.is_some()
            );
            assert!(packets.try_recv().is_err(), "STOP must not be sent");
            if failure.is_some() {
                assert_eq!(record.status, DigCleanupStatus::Failed);
                assert!(
                    record
                        .reason
                        .contains("; ABORT failed: dig cleanup ABORT delivery unknown")
                );
                assert_eq!(error.kind(), crate::ErrorKind::OperationUnconfirmed);
                assert!(format!("{:#}", error.diagnostic()).contains(&record.reason));
                // The primitive failure also terminates the actor/ledger; a
                // second dig on this original session cannot dispatch or replay.
                assert!(
                    client
                        .survival()
                        .dig(target, crate::BlockFace::Up)
                        .await
                        .is_err()
                );
                assert_eq!(
                    client.survival().dig_cleanup_record().unwrap().reason,
                    record.reason
                );
                assert!(bot.common_receipts.lock().await.pending_dispatch);
                assert_eq!(control.progress().2, 0);
                if failure == Some(false) {
                    assert_eq!(control.bytes().len(), 1);
                }
            } else {
                assert_eq!(record.status, DigCleanupStatus::Aborted);
                assert_eq!(error.kind(), crate::ErrorKind::State);
                assert!(!record.reason.contains("ABORT failed"));
                let bytes = control.bytes();
                let (id, abort) = crate::protocol::read_packet(&mut bytes.as_slice(), None)
                    .await
                    .unwrap();
                assert_eq!((id, abort[0]), (0x1b, 1));
                assert_eq!(control.progress().2, 1);
            }
            if failure.is_some() {
                assert!(bot.disconnect().await.is_err());
                bot.writer.lock().await.inner.shutdown().await.unwrap();
            } else {
                bot.disconnect().await.unwrap();
            }
            timeout(Duration::from_secs(2), server)
                .await
                .unwrap()
                .unwrap();
        }
    }

    #[tokio::test]
    async fn queued_finish_rechecks_world_hand_ground_and_target_under_the_write_gate() {
        for change in ["world", "hand", "ground", "target"] {
            let (bot, mut packets, _release, server) =
                super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
            super::super::common_motion::tests::seed_motion(&bot).await;
            {
                let mut receipts = bot.common_receipts.lock().await;
                receipts.inventory.window_id = Some(0);
                receipts.inventory.slots = (0..46)
                    .map(|_| {
                        Some(ObservedValue {
                            value: SlotKnowledge::Empty,
                            source: ValueSource::Received { sequence: 1 },
                        })
                    })
                    .collect();
                receipts.selected_hotbar = Some(ObservedValue {
                    value: 0,
                    source: ValueSource::Received { sequence: 1 },
                });
            }
            let target = [9, 65, 8];
            let dirt =
                crate::client::registry::Registry::for_version(crate::MinecraftVersion::Java1_16_1)
                    .resolve_block_state(&crate::NativeBlockState {
                        name: "minecraft:dirt".into(),
                        properties: Default::default(),
                    })
                    .unwrap()
                    .value();
            bot.world
                .lock()
                .await
                .set_block_for_test(BlockPos { x: 9, y: 65, z: 8 }, dirt);
            let initial = bot.dig_snapshot(target).await.unwrap();
            let initial_y = bot.player.lock().await.y;
            let initial_vy = bot.motion.lock().await.velocity.y;
            let gate = bot.coherent_state_gate.lock().await;
            let queued = bot.clone_internal();
            let sending = tokio::spawn(async move {
                queued
                    .dig_dispatch(
                        &initial,
                        target,
                        crate::client::BlockFace::Up,
                        Action::Finish {
                            elapsed: std::time::Duration::from_millis(5702),
                        },
                        &AtomicBool::new(false),
                    )
                    .await
            });
            tokio::task::yield_now().await;
            assert!(!sending.is_finished());
            match change {
                "world" => bot.common_receipts.lock().await.generation += 1,
                "hand" => {
                    bot.common_receipts
                        .lock()
                        .await
                        .selected_hotbar
                        .as_mut()
                        .unwrap()
                        .value = 1
                }
                "ground" => bot.player.lock().await.on_ground = false,
                "target" => bot
                    .world
                    .lock()
                    .await
                    .set_block_for_test(BlockPos { x: 9, y: 65, z: 8 }, 1),
                _ => unreachable!(),
            }
            drop(gate);
            let result = sending.await.unwrap();
            if change == "target" {
                assert!(matches!(result, Ok(Dispatch::Changed(_))));
            } else {
                let error = result.err().expect(change);
                if matches!(change, "hand" | "ground") {
                    let message = error.to_string();
                    assert!(
                        message.contains(" (at STOP, 5702 ms after START): "),
                        "{message}"
                    );
                    assert!(message.ends_with(&format!(
                        "; y {initial_y:.2} -> {initial_y:.2}, vy {initial_vy:.4} -> {initial_vy:.4}"
                    )), "{message}");
                }
            }
            assert!(packets.try_recv().is_err());
            bot.disconnect().await.unwrap();
            server.await.unwrap();
        }
    }
}

#[cfg(test)]
#[path = "dig_handle_tests.rs"]
mod handle_tests;
