//! Adapter-specific neutral packets and original-owner validation.
use super::*;
use crate::client::ledger::stops::StopAction;
use crate::client::{SessionStamp, StopCompletion, StopError, StopTarget, VehicleRelation};
impl Session {
    pub(super) fn observe_stop_context(&self, state: &State) {
        let session = SessionStamp {
            version: MinecraftVersion::Java1_21_11,
            connection_id: self.id,
            world_generation: state.loading.generation,
        };
        let vehicle = state.vehicles.capture(
            session,
            state.sequence,
            state.operations.local_player.entity_id,
            &state.entities,
        );
        let mount = vehicle.relation.and_then(|r| match r.value {
            VehicleRelation::Mounted { mount } => Some(mount),
            _ => None,
        });
        self.ledger
            .stop_context(session, state.operations.game_mode, mount);
        let using = state
            .operations
            .local_player
            .using_item
            .map(|u| (u.receive_sequence, u.hand));
        let selected = state.operations.selected_hotbar.as_ref().map(|s| s.slot);
        let slot = if using.is_some_and(|(_, h)| h == Some(crate::client::Hand::Off)) {
            Some(45)
        } else {
            selected.map(|s| 36 + usize::from(s))
        };
        let hand = slot
            .and_then(|slot| state.operations.inventory.slots.get(slot))
            .and_then(|s| operations::common_slot(s).ok());
        self.ledger.observe_hand(using, Some((selected, hand)));
    }
}
impl operations::Operations {
    async fn selected_stop(&self, target: StopTarget, action: StopAction) -> Result<()> {
        if let StopAction::CommonMovement(id) = action {
            return self.quiesce_movement(target, id).await;
        }
        let mut state = self.bot.session.state.lock().await;
        self.bot.session.observe_stop_context(&state);
        if !self.bot.session.ledger.stop_valid(target) {
            return Err(Error::new(
                ErrorKind::State,
                anyhow::anyhow!("selected stop context changed"),
            ));
        }
        match action {
            StopAction::NativeMovement => {
                self.bot
                    .session
                    .send(ids::play_serverbound::PLAYER_INPUT, &[0])
                    .await?;
            }
            StopAction::ItemUse => {
                let mut payload = vec![5];
                payload.extend(0u64.to_be_bytes());
                payload.push(0);
                put_varint(&mut payload, 0);
                self.bot
                    .session
                    .send(ids::play_serverbound::BLOCK_DIG, &payload)
                    .await?;
                operations::continuous::item_released(&mut state);
            }
            StopAction::Vehicle => {
                self.bot
                    .session
                    .send(ids::play_serverbound::PLAYER_INPUT, &[0])
                    .await?;
                self.bot
                    .session
                    .send(ids::play_serverbound::STEER_BOAT, &[0, 0])
                    .await?;
            }
            StopAction::Dig { position, face } => {
                let sequence = self.next_sequence()?;
                let after_sequence = state.sequence;
                if let Some(record) = &mut state.mining {
                    record.abort = Some(operations::mining::MiningSend {
                        sequence,
                        after_sequence,
                        dispatched: false,
                    });
                }
                let mut payload = vec![1];
                payload.extend(operations::pack_position(position).to_be_bytes());
                payload.push(face as u8);
                put_varint(&mut payload, sequence);
                self.bot
                    .session
                    .send(ids::play_serverbound::BLOCK_DIG, &payload)
                    .await?;
                if let Some(record) = &mut state.mining {
                    record
                        .abort
                        .as_mut()
                        .expect("selected abort retained")
                        .dispatched = true;
                }
                drop(state);
                timeout(self.bot.session.limits.play_packet_timeout, async {
                    loop {
                        let notified = self.bot.session.changed.notified();
                        tokio::pin!(notified);
                        notified.as_mut().enable();
                        let state = self.bot.session.state.lock().await;
                        self.bot.session.check(&state)?;
                        if state.operations.ack.is_some_and(|ack| ack >= sequence) {
                            return Ok::<(), Error>(());
                        }
                        drop(state);
                        notified.await;
                    }
                })
                .await
                .map_err(|_| {
                    Error::new(
                        ErrorKind::Timeout,
                        anyhow::anyhow!("neutral ABORT cleanup timeout"),
                    )
                })?
                .map_err(|error| {
                    crate::Error::new(
                        crate::ErrorKind::UncertainDispatch,
                        anyhow::anyhow!("neutral ABORT outcome unknown: {error}"),
                    )
                })?;
                return Ok(());
            }
            StopAction::CommonMovement(_) => unreachable!(),
        }
        self.bot.session.ledger.clear_target(target);
        Ok(())
    }
}
impl crate::client::adapter::DrainOps for operations::Operations {
    fn drain_ledger(&self) -> Ledger {
        if let Ok(state) = self.bot.session.state.try_lock() {
            self.bot
                .session
                .ledger
                .set_ready(state.ready && state.phase == Phase::Play);
            self.bot.session.observe_stop_context(&state);
        }
        self.bot.session.ledger.clone()
    }
    fn drain_ready(&self) -> bool {
        self.bot.session.ledger.ready() && !self.bot.session.stopped.load(Ordering::Acquire)
    }
    async fn stop_targets(&self) -> std::result::Result<Vec<StopTarget>, StopError> {
        let state = self.bot.session.state.lock().await;
        self.bot.session.observe_stop_context(&state);
        let targets = self.bot.session.ledger.stop_targets()?;
        if state
            .operations
            .local_player
            .using_item
            .and_then(|u| u.hand)
            .is_some_and(|hand| !self.bot.session.ledger.known_item_stop(hand))
        {
            self.bot
                .session
                .ledger
                .fail(Failure::TransactionOutcomeUnknown);
            return Err(StopError::Drain(
                self.bot
                    .session
                    .ledger
                    .error(crate::client::DrainErrorKind::TransactionOutcomeUnknown),
            ));
        }
        Ok(targets)
    }
    fn enqueue_neutralize(
        &self,
        target: StopTarget,
    ) -> std::result::Result<StopCompletion, StopError> {
        let owner = self.clone();
        self.bot
            .session
            .ledger
            .enqueue_neutralize(target, move |action| async move {
                owner.selected_stop(target, action).await
            })
    }
}
