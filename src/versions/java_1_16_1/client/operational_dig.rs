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
            block(eye)?,
            block([eye[0], eye[1] + 1, eye[2]])?,
        )
    }
}

impl DigOps for Bot {
    fn dig_work(&self) -> crate::client::ledger::WorkGuard {
        self.connection.ledger().work()
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
            self.connection
                .dispatch_operation(
                    self.operation_context(0),
                    OperationClass::Cleanup,
                    0x1b,
                    &payload,
                )
                .await
                .map_err(dig::unavailable)?;
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
        dig::validate(initial, &current)?;
        let action = match action {
            Action::Start => crate::client::operations::Action::DigStart(target, face),
            Action::Finish => crate::client::operations::Action::DigFinish(target, face),
            Action::Abort => unreachable!(),
        };
        self.execute_common_unlocked(GameMode::Survival, action, None)
            .await
            .map(Dispatch::Sent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{ObservedValue, SlotKnowledge, ValueSource};

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
            let gate = bot.coherent_state_gate.lock().await;
            let queued = bot.clone_internal();
            let sending = tokio::spawn(async move {
                queued
                    .dig_dispatch(
                        &initial,
                        target,
                        crate::client::BlockFace::Up,
                        Action::Finish,
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
                assert!(result.is_err(), "{change}");
            }
            assert!(packets.try_recv().is_err());
            bot.disconnect().await.unwrap();
            server.await.unwrap();
        }
    }
}
