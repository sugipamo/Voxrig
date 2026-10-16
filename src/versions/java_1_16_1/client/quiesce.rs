//! Public drain integration without native-only authority bridges.
use super::*;
use crate::client::ledger::{Failure, stops::StopAction};
use crate::client::{SessionStamp, StopError, StopReceipt, StopTarget};

impl Bot {
    pub(super) async fn observe_stop_context(&self) {
        let own = super::lock_packet_state(&self.player).await.entity_id;
        let mode = super::read_session_cache(&self.survival)
            .await
            .game_mode
            .and_then(|id| crate::client::GameMode::decode(id & 7).ok());
        let receipts = super::lock_packet_state(&self.common_receipts).await;
        let session = SessionStamp {
            version: crate::MinecraftVersion::Java1_16_1,
            connection_id: self.connection_generation().get(),
            world_generation: receipts.generation,
        };
        let vehicle = receipts.vehicles.capture(
            session,
            self.protocol_packet_sequence.load(Ordering::Acquire),
            own,
            &receipts.entities,
        );
        let mount = vehicle.relation.and_then(|r| match r.value {
            crate::client::VehicleRelation::Mounted { mount } => Some(mount),
            _ => None,
        });
        self.connection.ledger().stop_context(session, mode, mount);
        let using = receipts.using_item.as_ref().and_then(|u| match u.source {
            crate::client::ValueSource::Received { sequence } => Some((sequence, u.value)),
            _ => None,
        });
        let selected = receipts.selected_hotbar.as_ref().map(|s| s.value);
        let slot = if using.is_some_and(|(_, h)| h == Some(crate::client::Hand::Off)) {
            Some(45)
        } else {
            selected.map(|s| 36 + usize::from(s))
        };
        let hand = slot
            .and_then(|slot| receipts.inventory.slots.get(slot))
            .and_then(Option::as_ref)
            .map(|s| s.value.clone());
        self.connection
            .ledger()
            .observe_hand(using, Some((selected, hand)));
    }
    async fn selected_stop(&self, target: StopTarget, action: StopAction) -> Result<()> {
        if let StopAction::CommonMovement(id) = action {
            return self.quiesce_movement(target, id).await;
        }
        let gate = self.coherent_state_gate.lock().await;
        self.observe_stop_context().await;
        if !self.connection.ledger().stop_valid(target) {
            return Err(crate::Error::new(
                crate::ErrorKind::State,
                anyhow::anyhow!("selected stop context changed"),
            ));
        }
        match action {
            StopAction::NativeMovement => {
                // Neutral local input and explicit sprint/sneak release; no position prediction.
                self.connection
                    .replace_control(
                        self.operation_context(0),
                        OperationClass::Cleanup,
                        ControlState::default(),
                    )
                    .await
                    .map_err(|e| {
                        crate::Error::new(crate::ErrorKind::State, anyhow::anyhow!("{e}"))
                    })?;
                // An active common owner owns wire posture; this selected native
                // target only releases its dormant local input in that case.
                if !self.common_control.lock().await.active() {
                    self.send_entity_action(4).await?;
                    self.send_entity_action(1).await?;
                }
            }
            StopAction::ItemUse => {
                let mut payload = vec![5];
                payload.extend(0u64.to_be_bytes());
                payload.push(0);
                self.connection
                    .dispatch_protocol_work(0x1b, &payload, Some(self.connection.ledger().work()))
                    .await?;
                let received = self.control_received().await;
                self.common_control.lock().await.item_released(&received);
            }
            StopAction::Vehicle => {
                let mut payload = vec![0; 8];
                payload.push(0);
                self.connection
                    .dispatch_protocol_work(0x1d, &payload, Some(self.connection.ledger().work()))
                    .await?;
                self.connection
                    .dispatch_protocol_work(0x16, &[0, 0], Some(self.connection.ledger().work()))
                    .await?;
            }
            StopAction::Dig { position, face } => {
                let mut events = self.subscribe();
                if let Some(id) = target.mining_id() {
                    let mut retained = super::lock_packet_state(&self.common_mining).await;
                    let record = &mut retained
                        .as_mut()
                        .filter(|r| r.record.id == id)
                        .ok_or_else(|| {
                            crate::Error::new(
                                crate::ErrorKind::State,
                                anyhow::anyhow!("selected mining owner changed"),
                            )
                        })?
                        .record;
                    record.abort = Some(crate::client::survival::MiningSend {
                        after_sequence: self.protocol_packet_sequence.load(Ordering::Acquire),
                        interaction_sequence: None,
                        dispatched: false,
                    });
                }
                let mut payload = vec![1];
                payload.extend(
                    BlockPos {
                        x: position[0],
                        y: position[1],
                        z: position[2],
                    }
                    .packed()
                    .to_be_bytes(),
                );
                payload.push(face as u8);
                self.connection
                    .dispatch_protocol_work(0x1b, &payload, Some(self.connection.ledger().work()))
                    .await?;
                if let Some(id) = target.mining_id() {
                    if let Some(run) = super::lock_packet_state(&self.common_mining)
                        .await
                        .as_mut()
                        .filter(|r| r.record.id == id)
                    {
                        run.record
                            .abort
                            .as_mut()
                            .expect("selected abort retained")
                            .dispatched = true;
                    }
                }
                drop(gate); // The reader must apply the matching cleanup acknowledgement.
                timeout(self.connection_options.protocol_ack_timeout, async {
                    loop {
                        if let Event::DiggingAcknowledged(ack) =
                            next_operation_event(&mut events).await?
                        {
                            if [ack.position.x, ack.position.y, ack.position.z] == position
                                && ack.status == 1
                                && ack.successful
                            {
                                return Ok::<(), crate::Error>(());
                            }
                        }
                    }
                })
                .await
                .map_err(|_| {
                    self.connection.ledger().fail(Failure::TransactionExpired);
                    crate::Error::new(
                        crate::ErrorKind::Timeout,
                        anyhow::anyhow!("neutral ABORT cleanup timeout"),
                    )
                })?
                .map_err(|error| {
                    crate::Error::new(
                        crate::ErrorKind::UncertainDispatch,
                        anyhow::anyhow!("neutral ABORT outcome unknown: {error}"),
                    )
                })?;
                if let Some(id) = target.mining_id() {
                    self.connection
                        .complete_neutral_mining(id.attempt())
                        .await
                        .map_err(|e| {
                            crate::Error::new(crate::ErrorKind::State, anyhow::anyhow!("{e}"))
                        })?;
                }
                return Ok(());
            }
            StopAction::CommonMovement(_) => unreachable!(),
        }
        self.connection.ledger().clear_target(target);
        Ok(())
    }
}
impl crate::client::adapter::DrainOps for Bot {
    fn drain_ledger(&self) -> crate::client::ledger::Ledger {
        self.connection.ledger().clone()
    }
    fn drain_ready(&self) -> bool {
        self.connection_state() == ConnectionState::Ready && !self.stopped.load(Ordering::Acquire)
    }
    async fn stop_targets(&self) -> std::result::Result<Vec<StopTarget>, StopError> {
        let _gate = self.coherent_state_gate.lock().await;
        self.observe_stop_context().await;
        let targets = self.connection.ledger().stop_targets()?;
        let receipts = super::lock_packet_state(&self.common_receipts).await;
        if receipts
            .using_item
            .as_ref()
            .and_then(|u| u.value)
            .is_some_and(|hand| !self.connection.ledger().known_item_stop(hand))
        {
            self.connection
                .ledger()
                .fail(Failure::TransactionOutcomeUnknown);
            return Err(StopError::Drain(
                self.connection
                    .ledger()
                    .error(crate::client::DrainErrorKind::TransactionOutcomeUnknown),
            ));
        }
        Ok(targets)
    }
    async fn neutralize(&self, target: StopTarget) -> std::result::Result<StopReceipt, StopError> {
        let owner = self.clone_internal();
        self.connection
            .ledger()
            .neutralize(target, move |action| async move {
                owner.selected_stop(target, action).await
            })
            .await
    }
}

#[cfg(test)]
mod tests;
