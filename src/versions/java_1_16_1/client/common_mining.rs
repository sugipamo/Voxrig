//! Retained legacy mining, with exact target receipts and actor-owned command stages.
use super::*;
use crate::client::{
    ObservedValue, SlotKnowledge, ValueSource,
    survival::{
        MiningAction, MiningId, MiningInventoryChange, MiningInventoryChangeKind,
        MiningProtocolObservation, MiningRecord, MiningSend, MiningStage, MiningTargetReceipt,
        mining,
    },
};

#[derive(Clone)]
pub(super) struct NativeMiningRun {
    pub(super) record: MiningRecord,
    movement_revision: u64,
    movement_attribute: Option<Attribute>,
}
fn inspection(run: &mut NativeMiningRun, reason: impl std::fmt::Display) {
    if run.record.stage == MiningStage::ObservedRemoved {
        return;
    }
    if let Some(change) = &mut run.record.inventory_change {
        change.sole_cause = false;
    }
    run.record
        .requires_inspection
        .get_or_insert_with(|| reason.to_string());
    run.record.stage = MiningStage::RequiresInspection;
}
fn known_empty(value: Option<&ObservedValue<SlotKnowledge>>) -> bool {
    matches!(
        value,
        Some(ObservedValue {
            value: SlotKnowledge::Empty,
            source: ValueSource::Received { .. }
        })
    )
}
impl Bot {
    pub(crate) async fn common_claim_profile_recovery(
        &self,
        id: MiningId,
        config: &crate::ConnectionConfig,
        target: crate::client::survival::MiningRecoveryTarget,
    ) -> Result<()> {
        let _gate = self.coherent_state_gate.lock().await;
        if !self.is_stopped()
            || config.server != self.server
            || config.username != self.login_profile.name
            || config.version != crate::MinecraftVersion::Java1_16_1
        {
            return Err(mining::unavailable(
                "recovery requires the closed original endpoint/profile/version",
            ));
        }
        let mut retained = self.common_mining.lock().await;
        let run = retained
            .as_mut()
            .filter(|m| m.record.id == id)
            .ok_or_else(|| mining::unavailable("recovery belongs to another mining attempt"))?;
        if run.record.recovery_attempt.is_some() {
            return Err(mining::unavailable(
                "mining recovery already attempted; no second login",
            ));
        }
        target.validate_baseline(&run.record.baseline)?;
        run.record.recovery_attempt = Some(crate::client::survival::MiningRecoveryAttempt {
            method: crate::client::survival::MiningRecoveryMethod::SameProfileLogin,
            target,
        });
        Ok(())
    }
    async fn mining_prepared(
        &self,
        target: [i32; 3],
        face: crate::BlockFace,
        owner: Option<MiningId>,
    ) -> Result<crate::client::survival::BlockTargetObservation> {
        let query = self.common_target_unlocked(4.5, owner).await?;
        if query.initial.received_pose.is_none() || query.initial.dimension.is_none() {
            return Err(mining::unavailable(
                "mining requires received own-pose and world identity",
            ));
        }
        let hit = query
            .hit
            .as_ref()
            .ok_or_else(|| mining::unavailable("no native first outline for mining"))?;
        if hit.position != target || hit.face != face {
            return Err(mining::unavailable(
                "mining target/face differs from native first outline",
            ));
        }
        crate::client::survival::mining_tools::material(
            crate::MinecraftVersion::Java1_16_1,
            &hit.state,
        )?;
        let position = query
            .initial
            .position
            .as_ref()
            .ok_or_else(|| mining::unavailable("mining position unavailable"))?
            .value;
        if crate::client::survival::mining_tools::removes_support(
            crate::MinecraftVersion::Java1_16_1,
            &hit.state,
            target,
            position,
        )? {
            return Err(mining::unavailable(
                "mining may not remove its foot support",
            ));
        }
        let selected = query
            .initial
            .selected_hotbar
            .as_ref()
            .filter(|s| s.value <= 8)
            .ok_or_else(|| mining::unavailable("ordered hand selection unavailable"))?;
        let inventory = &query.initial.inventory;
        let legacy = self.inventory.read().await;
        if inventory.window_id != Some(0)
            || !known_empty(inventory.cursor.as_ref())
            || !inventory.slots[36 + usize::from(selected.value)]
                .as_ref()
                .is_some_and(|s| {
                    matches!(s.source, ValueSource::Received { .. })
                        && s.value != SlotKnowledge::Unavailable
                })
            || legacy.cursor.is_some()
            || legacy.open_window.is_some()
            || !legacy.pending_clicks.is_empty()
        {
            return Err(mining::unavailable(
                "mining needs received selected hand, empty cursor and supported player screen without pending clicks",
            ));
        }
        self.mining_estimate(&query.initial, &hit.state).await?;
        Ok(query)
    }
    async fn mining_estimate(
        &self,
        player: &crate::client::PlayerObservation,
        state: &crate::NativeBlockState,
    ) -> Result<crate::client::survival::MiningEstimate> {
        let selected = player
            .selected_hotbar
            .as_ref()
            .ok_or_else(|| mining::unavailable("selected hand missing"))?;
        let hand = player.inventory.slots[36 + usize::from(selected.value)]
            .as_ref()
            .ok_or_else(|| mining::unavailable("received mining hand missing"))?;
        let receipts = self.common_receipts.lock().await;
        let registries = receipts
            .registries
            .capture(player.session, player.receive_sequence);
        crate::client::survival::mining_tools::estimate(
            crate::MinecraftVersion::Java1_16_1,
            state,
            &hand.value,
            Some(&registries),
        )
    }
    pub(crate) async fn common_start_mining(
        &self,
        target: [i32; 3],
        face: crate::BlockFace,
    ) -> Result<MiningRecord> {
        let gate = self.coherent_state_gate.lock().await;
        self.common_motion_admission().await?;
        let revision = self
            .connection
            .motion_admission_revision()
            .await
            .map_err(|e| mining::unavailable(format!("mining admission rejected: {e:?}")))?;
        let query = self.mining_prepared(target, face, None).await?;
        let initial = query.initial;
        let baseline = query.hit.expect("prepared hit").state;
        let estimate = self.mining_estimate(&initial, &baseline).await?;
        let id = MiningId::new(initial.session, 1); // No in-session replacement/continuation.
        let record = MiningRecord {
            id,
            target,
            face,
            estimated_wait_ms: estimate.wait_ms(),
            estimate,
            start: MiningSend {
                after_sequence: initial.receive_sequence,
                interaction_sequence: None,
                dispatched: false,
            },
            initial,
            baseline,
            finish: None,
            abort: None,
            target_receipt: None,
            protocol: None,
            inventory_change: None,
            requires_inspection: None,
            stage: MiningStage::Mining,
            recovery_attempt: None,
            continuation_validated: false,
        };
        *self.common_mining.lock().await = Some(NativeMiningRun {
            record,
            movement_revision: self.motion.lock().await.revision(),
            movement_attribute: super::common_motion::legacy_movement_attribute(
                &**self.survival.read().await,
            )
            .cloned(),
        });
        // Retain before spawning the owner. Cancellation of this waiter neither clears
        // the intent nor resends. The task performs the single native command.
        let (reply, result) = tokio::sync::oneshot::channel();
        let bot = self.clone_internal();
        tokio::spawn(async move {
            let _ = reply.send(
                bot.mining_send_owned(id, MiningAction::Start, Some(revision))
                    .await,
            );
        });
        drop(gate);
        result
            .await
            .map_err(|_| mining::unavailable("mining owner result missing"))?
    }
    pub(crate) async fn common_mining_send(
        &self,
        id: MiningId,
        action: MiningAction,
    ) -> Result<MiningRecord> {
        if action == MiningAction::Start {
            return Err(mining::unavailable("START may not be replayed"));
        }
        let gate = self.coherent_state_gate.lock().await;
        {
            let mut guard = self.common_mining.lock().await;
            let run = guard
                .as_mut()
                .filter(|m| m.record.id == id)
                .ok_or_else(|| {
                    mining::unavailable("mining id belongs to another connection/attempt")
                })?;
            if !run.record.start.dispatched
                || run.record.stage == MiningStage::ObservedRemoved
                || run.record.requires_inspection.is_some()
                || run.record.abort.is_some()
                || (action == MiningAction::Finish && run.record.finish.is_some())
            {
                return Err(mining::unavailable(
                    "mining command cannot resume or replay this stage",
                ));
            }
            let attempt = MiningSend {
                after_sequence: self.protocol_packet_sequence.load(Ordering::Acquire),
                interaction_sequence: None,
                dispatched: false,
            };
            if action == MiningAction::Abort {
                run.record.abort = Some(attempt);
            } else {
                run.record.finish = Some(attempt);
                run.record.stage = MiningStage::PendingAfterFinish;
            }
        }
        let (reply, result) = tokio::sync::oneshot::channel();
        let bot = self.clone_internal();
        tokio::spawn(async move {
            let _ = reply.send(bot.mining_send_owned(id, action, None).await);
        });
        drop(gate);
        result
            .await
            .map_err(|_| mining::unavailable("mining owner result missing"))?
    }
    async fn mining_send_owned(
        &self,
        id: MiningId,
        action: MiningAction,
        revision: Option<u64>,
    ) -> Result<MiningRecord> {
        let _gate = self.coherent_state_gate.lock().await;
        let result = async {
            let run = self
                .common_mining
                .lock()
                .await
                .as_ref()
                .filter(|m| m.record.id == id)
                .cloned()
                .ok_or_else(|| mining::unavailable("mining owner missing"))?;
            if run.record.requires_inspection.is_some()
                || run.record.stage == MiningStage::ObservedRemoved
            {
                return Err(mining::unavailable(
                    "mining context interrupted before command dispatch",
                ));
            }
            if action != MiningAction::Abort {
                let prepared = self
                    .mining_prepared(run.record.target, run.record.face, Some(id))
                    .await?;
                if prepared.initial.session != id.session()
                    || prepared.initial.position.as_ref().map(|p| p.value)
                        != run.record.initial.position.as_ref().map(|p| p.value)
                    || prepared.initial.rotation != run.record.initial.rotation
                    || prepared.initial.selected_hotbar != run.record.initial.selected_hotbar
                    || prepared.initial.inventory.slots[36
                        + usize::from(run.record.initial.selected_hotbar.as_ref().unwrap().value)]
                    .as_ref()
                    .map(|s| &s.value)
                        != run.record.initial.inventory.slots[36
                            + usize::from(
                                run.record.initial.selected_hotbar.as_ref().unwrap().value,
                            )]
                        .as_ref()
                        .map(|s| &s.value)
                    || prepared.hit.as_ref().map(|h| &h.state) != Some(&run.record.baseline)
                    || self.motion.lock().await.revision() != run.movement_revision
                {
                    return Err(mining::unavailable(
                        "mining baseline/pose/selection changed before command",
                    ));
                }
            } else if self.connection_state() != ConnectionState::Ready {
                return Err(mining::unavailable("mining connection unavailable"));
            }
            {
                let mut guard = self.common_mining.lock().await;
                let record = &mut guard.as_mut().expect("retained mining").record;
                let sequence = self.protocol_packet_sequence.load(Ordering::Acquire);
                match action {
                    MiningAction::Start => record.start.after_sequence = sequence,
                    MiningAction::Finish => {
                        record
                            .finish
                            .as_mut()
                            .expect("retained finish")
                            .after_sequence = sequence
                    }
                    MiningAction::Abort => {
                        record
                            .abort
                            .as_mut()
                            .expect("retained abort")
                            .after_sequence = sequence
                    }
                }
            }
            if let Some(revision) = revision {
                self.connection
                    .begin_bounded_mining(
                        id.attempt(),
                        revision,
                        BlockPos {
                            x: run.record.target[0],
                            y: run.record.target[1],
                            z: run.record.target[2],
                        },
                        run.record.face as u8,
                    )
                    .await
                    .map_err(|e| {
                        mining::unavailable(format!("mining exclusive acquisition rejected: {e:?}"))
                    })?;
            }
            self.connection.bounded_mining(id.attempt(), action).await?;
            let mut guard = self.common_mining.lock().await;
            let record = &mut guard.as_mut().expect("retained mining").record;
            match action {
                MiningAction::Start => record.start.dispatched = true,
                MiningAction::Finish => {
                    record.finish.as_mut().expect("retained finish").dispatched = true
                }
                MiningAction::Abort => {
                    record.abort.as_mut().expect("retained abort").dispatched = true
                }
            }
            Ok(record.clone())
        }
        .await;
        if let Err(e) = &result {
            self.interrupt_common_mining(e).await;
        }
        result
    }
    pub(super) async fn interrupt_common_mining(&self, reason: impl std::fmt::Display) {
        if let Some(run) = self.common_mining.lock().await.as_mut() {
            inspection(run, reason);
        }
    }
    pub(crate) async fn common_mining_record(&self) -> Result<Option<MiningRecord>> {
        let _gate = self.coherent_state_gate.lock().await;
        self.reconcile_common_mining(true).await?;
        Ok(self
            .common_mining
            .lock()
            .await
            .as_ref()
            .map(|m| m.record.clone()))
    }
    async fn reconcile_common_mining(&self, confirm_removal: bool) -> Result<()> {
        let Some(snapshot) = self.common_mining.lock().await.clone() else {
            return Ok(());
        };
        if snapshot.record.stage == MiningStage::ObservedRemoved {
            return Ok(());
        }
        if self.is_stopped() || self.connection_state() != ConnectionState::Ready {
            self.interrupt_common_mining("mining connection closed or unavailable")
                .await;
            return Ok(());
        }
        let current = self.common_player_unlocked().await?;
        if current.session != snapshot.record.id.session()
            || current.game_mode != Some(crate::client::GameMode::Survival)
            || current.position.as_ref().map(|p| p.value)
                != snapshot.record.initial.position.as_ref().map(|p| p.value)
            || current.rotation != snapshot.record.initial.rotation
            || *self.local_pose.lock().await != Some(0)
            || !current
                .health
                .as_ref()
                .is_some_and(|h| h.value.health > 0.0)
            || self.motion.lock().await.revision() != snapshot.movement_revision
        {
            self.interrupt_common_mining("mining player/world context changed")
                .await;
        }
        if {
            let survival = self.survival.read().await;
            survival.flying
                || !survival.effects.is_empty()
                || super::common_motion::legacy_movement_attribute(&survival)
                    != snapshot.movement_attribute.as_ref()
        } || !self.player.lock().await.on_ground
            || self.control().await != ControlState::default()
        {
            self.interrupt_common_mining(
                "mining flight/effects/attributes/grounded controls changed",
            )
            .await;
        }
        if let Err(error) = self
            .common_preview_core(
                &[crate::client::survival::SurvivalControl {
                    yaw: current.rotation[0],
                    input: Default::default(),
                }],
                Some(snapshot.record.id),
            )
            .await
        {
            self.interrupt_common_mining(format!("mining standing geometry changed: {error}"))
                .await;
        }
        let selected = snapshot
            .record
            .initial
            .selected_hotbar
            .as_ref()
            .expect("prepared selection")
            .value;
        let hand = current.inventory.slots[36 + usize::from(selected)].as_ref();
        let kind = if current.selected_hotbar != snapshot.record.initial.selected_hotbar {
            Some(MiningInventoryChangeKind::SelectionChanged)
        } else if current.inventory.window_id != Some(0)
            || !known_empty(current.inventory.cursor.as_ref())
        {
            Some(MiningInventoryChangeKind::PlayerScreenChanged)
        } else if !self.inventory.read().await.pending_clicks.is_empty() {
            Some(MiningInventoryChangeKind::InventoryUnavailable)
        } else if !hand.is_some_and(|h| {
            matches!(h.source, ValueSource::Received { .. })
                && Some(&h.value)
                    == snapshot.record.initial.inventory.slots[36 + usize::from(selected)]
                        .as_ref()
                        .map(|s| &s.value)
        }) {
            Some(MiningInventoryChangeKind::SelectedHandChanged)
        } else {
            None
        };
        if let Err(error) = self
            .mining_estimate(&current, &snapshot.record.baseline)
            .await
        {
            self.interrupt_common_mining(format!("mining tool prerequisites changed: {error}"))
                .await;
        }
        if let Some(kind) = kind {
            let mut guard = self.common_mining.lock().await;
            let run = guard.as_mut().expect("retained mining");
            if run.record.inventory_change.is_none() {
                run.record.inventory_change = Some(MiningInventoryChange {
                    kind,
                    receive_sequence: current.receive_sequence,
                    selection: current.selected_hotbar.clone(),
                    original_hand: hand.map_or(SlotKnowledge::Unavailable, |h| h.value.clone()),
                    hand_receive_sequence: hand.and_then(|h| match h.source {
                        ValueSource::Received { sequence } => Some(sequence),
                        _ => None,
                    }),
                    window_id: current.inventory.window_id,
                    cursor: current
                        .inventory
                        .cursor
                        .as_ref()
                        .map_or(SlotKnowledge::Unavailable, |c| c.value.clone()),
                    unsupported_components: false,
                    sole_cause: run.record.requires_inspection.is_none(),
                });
                run.record.requires_inspection.get_or_insert_with(|| {
                    "received inventory prerequisites changed during mining".into()
                });
                run.record.stage = MiningStage::RequiresInspection;
            }
        }
        let mut guard = self.common_mining.lock().await;
        let run = guard.as_mut().expect("retained mining");
        if run.record.requires_inspection.is_none() {
            let target = run.record.target;
            let id = self
                .world
                .lock()
                .await
                .block(target[0], target[1], target[2]);
            if let Some(id) = id {
                if let Some(receipt) = run.record.target_receipt.as_ref().filter(|r| {
                    confirm_removal
                        && run.record.start.dispatched
                        && r.receive_sequence > run.record.start.after_sequence
                }) {
                    let state = crate::versions::java_1_16_1::native_state(id)?;
                    if state == receipt.state
                        && matches!(
                            state.name.as_str(),
                            "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
                        )
                    {
                        run.record.stage = MiningStage::ObservedRemoved;
                    }
                }
            } else {
                inspection(run, "mining target unloaded");
            }
        }
        Ok(())
    }
    pub(super) async fn common_mining_context_received(&self) -> Result<()> {
        // Per-packet latch runs before later packets can restore an empty hand/mode.
        let tool = self.common_mining.lock().await.as_ref().is_some_and(|r| {
            let p = &r.record.initial;
            p.selected_hotbar.as_ref().is_some_and(|s| {
                p.inventory.slots[36 + usize::from(s.value)]
                    .as_ref()
                    .is_some_and(|h| matches!(h.value, SlotKnowledge::Item { .. }))
            })
        });
        // A tool may wear in a later slot packet. Preserve the already checked
        // exact-target air at its own receive boundary before that later change.
        self.reconcile_common_mining(tool).await
    }
    pub(super) async fn common_mining_chunk_changed(&self, chunk: [i32; 2]) {
        let mut guard = self.common_mining.lock().await;
        if let Some(run) = guard.as_mut().filter(|m| {
            [
                m.record.target[0].div_euclid(16),
                m.record.target[2].div_euclid(16),
            ] == chunk
        }) {
            inspection(run, "mining target chunk unloaded or replaced");
        }
    }
    pub(super) async fn common_mining_target_received(
        &self,
        position: [i32; 3],
        state_id: i32,
        sequence: u64,
        reply: Option<(i32, bool)>,
    ) -> Result<()> {
        let mut guard = self.common_mining.lock().await;
        let Some(run) = guard.as_mut().filter(|m| m.record.target == position) else {
            return Ok(());
        };
        let state = crate::versions::java_1_16_1::native_state(state_id)?;
        if sequence <= run.record.start.after_sequence {
            return Ok(());
        }
        if let Some((action, accepted)) = reply {
            run.record.protocol = Some(MiningProtocolObservation::LegacyReply {
                action,
                accepted,
                state: state.clone(),
                receive_sequence: sequence,
            });
            if !accepted {
                inspection(run, "legacy mining command response rejected");
            }
        }
        if run.record.stage == MiningStage::ObservedRemoved {
            return Ok(());
        }
        if self
            .world
            .lock()
            .await
            .block(position[0], position[1], position[2])
            != Some(state_id)
        {
            inspection(run, "mining target receipt without loaded baseline");
            return Ok(());
        }
        if state != run.record.baseline
            && !matches!(
                state.name.as_str(),
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            )
        {
            inspection(run, "mining target changed to unexpected non-air state");
        }
        run.record.target_receipt = Some(MiningTargetReceipt {
            state,
            receive_sequence: sequence,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const TARGET: [i32; 3] = [8, 66, 11];
    async fn seed(bot: &Bot) {
        super::super::common_motion::tests::seed_motion(bot).await;
        bot.survival.write().await.dimension = Some("minecraft:overworld".into());
        let mut slots = vec![0, 0, 46];
        slots.extend([0; 46]);
        bot.apply_packet(0x14, slots).await.unwrap();
        bot.apply_packet(0x16, vec![255, 255, 255, 0])
            .await
            .unwrap();
        bot.world.lock().await.set_block_for_test(
            BlockPos {
                x: TARGET[0],
                y: TARGET[1],
                z: TARGET[2],
            },
            1,
        );
    }
    async fn change(bot: &Bot, position: [i32; 3], state: i32) {
        let mut payload = BlockPos {
            x: position[0],
            y: position[1],
            z: position[2],
        }
        .packed()
        .to_be_bytes()
        .to_vec();
        put_varint(&mut payload, state);
        bot.apply_packet(0x0b, payload).await.unwrap();
    }
    #[tokio::test]
    async fn common_tool_mining_keeps_received_removal_before_later_durability() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed(&bot).await;
        bot.apply_packet(
            0x5b,
            crate::client::survival::mining_tools::test_block_tag_packet(
                crate::MinecraftVersion::Java1_16_1,
            ),
        )
        .await
        .unwrap();
        let mut item = vec![0, 0, 36, 1];
        put_varint(&mut item, crate::item_id("iron_pickaxe").unwrap());
        item.push(1);
        let mut pristine = item.clone();
        pristine.push(0);
        bot.apply_packet(0x16, pristine).await.unwrap();
        let client = crate::Client::from_java_1_16_1(bot.clone());
        client.survival().select_hotbar(0).await.unwrap();
        packets.recv().await.unwrap();
        let started = crate::client::tests::common_tool_mining_start_scenario(
            &client,
            TARGET,
            crate::BlockFace::North,
        )
        .await;
        packets.recv().await.unwrap();
        client.survival().finish_mining(started.id).await.unwrap();
        packets.recv().await.unwrap();
        change(&bot, TARGET, 0).await;
        // Do not poll the operation between exact-target air and later wear.
        item.extend([10, 0, 0, 3, 0, 6]);
        item.extend(b"Damage");
        item.extend([0, 0, 0, 1, 0]);
        bot.apply_packet(0x16, item).await.unwrap();
        let removed = client.survival().mining_record().await.unwrap().unwrap();
        assert_eq!(removed.stage, MiningStage::ObservedRemoved);
        assert!(removed.inventory_change.is_none() && !removed.continuation_validated);
        assert!(client.survival().select_hotbar(1).await.is_err());
        drop(release);
        drop(client);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn common_tool_mining_uses_received_stack_and_latches_tool_replacement() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed(&bot).await;
        bot.apply_packet(
            0x5b,
            crate::client::survival::mining_tools::test_block_tag_packet(
                crate::MinecraftVersion::Java1_16_1,
            ),
        )
        .await
        .unwrap();
        let mut item = vec![0, 0, 36, 1];
        put_varint(&mut item, crate::item_id("iron_pickaxe").unwrap());
        item.extend([1, 0]);
        bot.apply_packet(0x16, item.clone()).await.unwrap();
        let client = crate::Client::from_java_1_16_1(bot.clone());
        client.survival().select_hotbar(0).await.unwrap();
        packets.recv().await.unwrap();
        let record = crate::client::tests::common_tool_mining_start_scenario(
            &client,
            TARGET,
            crate::BlockFace::North,
        )
        .await;
        assert_eq!(packets.recv().await.unwrap().1[0], 0);
        bot.apply_packet(0x16, vec![0, 0, 36, 0]).await.unwrap();
        bot.apply_packet(0x16, item).await.unwrap();
        change(&bot, TARGET, 0).await;
        let retained = client.survival().mining_record().await.unwrap().unwrap();
        assert_eq!(retained.id, record.id);
        assert_eq!(retained.stage, MiningStage::RequiresInspection);
        assert_eq!(
            retained.inventory_change.unwrap().kind,
            MiningInventoryChangeKind::SelectedHandChanged
        );
        assert!(client.survival().finish_mining(record.id).await.is_err());
        drop(release);
        drop(client);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn common_profile_recovery_cancelled_login_keeps_shared_claim() {
        use crate::client::survival::MiningRecoveryTarget;
        use tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = Server::new("127.0.0.1", listener.local_addr().unwrap().port());
        let (mut bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        // Model the authenticated endpoint independently from the source wire
        // fixture, which never accepts a recovery login itself.
        bot.server = endpoint.clone();
        seed(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        client.survival().select_hotbar(0).await.unwrap();
        packets.recv().await.unwrap();
        let record = client
            .survival()
            .start_mining(TARGET, crate::BlockFace::North)
            .await
            .unwrap();
        packets.recv().await.unwrap();
        let ops = client.survival();
        assert!(
            ops.prepare_mining_profile_recovery(MiningId::new(record.id.session(), 2))
                .await
                .is_err()
        );
        let recovery = ops
            .prepare_mining_profile_recovery(record.id)
            .await
            .unwrap();
        let config = crate::ConnectionConfig::offline(
            endpoint,
            "LagProbe",
            crate::MinecraftVersion::Java1_16_1,
        );
        assert!(
            recovery
                .reconnect(config.clone(), MiningRecoveryTarget::OriginalOrAir)
                .await
                .is_err()
        );
        assert!(
            recovery
                .source_record()
                .await
                .unwrap()
                .recovery_attempt
                .is_none()
        );
        recovery.close_source().await.unwrap();
        let mut wrong = config.clone();
        wrong.server.port += 1;
        assert!(
            recovery
                .reconnect(wrong, MiningRecoveryTarget::OriginalOrAir)
                .await
                .is_err()
        );
        let mut wrong = config.clone();
        wrong.username = "Different".into();
        assert!(
            recovery
                .reconnect(wrong, MiningRecoveryTarget::OriginalOrAir)
                .await
                .is_err()
        );
        assert!(
            recovery
                .source_record()
                .await
                .unwrap()
                .recovery_attempt
                .is_none()
        );
        assert!(
            timeout(Duration::from_millis(10), listener.accept())
                .await
                .is_err()
        );
        let mut attempt =
            Box::pin(recovery.reconnect(config.clone(), MiningRecoveryTarget::OriginalOrAir));
        let (mut login, _) = timeout(Duration::from_secs(2), async {
            tokio::select! {
                accepted = listener.accept() => accepted.unwrap(),
                _ = &mut attempt => panic!("recovery returned before login"),
            }
        })
        .await
        .unwrap();
        let handshake = timeout(Duration::from_secs(2), async {
            tokio::select! {
                packet = read_packet(&mut login, None) => packet.unwrap(),
                _ = &mut attempt => panic!("recovery returned before handshake"),
            }
        })
        .await
        .unwrap();
        assert_eq!(handshake.0, 0);
        let (_, login_request) = timeout(Duration::from_secs(2), async {
            tokio::select! {
                packet = read_packet(&mut login, None) => packet.unwrap(),
                _ = &mut attempt => panic!("recovery returned before LOGIN_SUCCESS"),
            }
        })
        .await
        .unwrap();
        write_packet(
            &mut login,
            None,
            2,
            &crate::client::login::test_legacy_success(&login_request),
        )
        .await
        .unwrap();
        // Original 1.16.1 may send LOGIN_SUCCESS before old-player retirement.
        // Matching UUID/name without a fresh JOIN/position must stay pending.
        assert!(
            timeout(Duration::from_millis(50), &mut attempt)
                .await
                .is_err()
        );
        drop(attempt);
        assert!(
            timeout(Duration::from_secs(2), read_packet(&mut login, None))
                .await
                .unwrap()
                .is_err()
        );
        assert!(
            recovery
                .source_record()
                .await
                .unwrap()
                .recovery_attempt
                .is_some()
        );
        assert!(
            !recovery
                .source_record()
                .await
                .unwrap()
                .continuation_validated
        );
        assert!(
            recovery
                .clone()
                .reconnect(config, MiningRecoveryTarget::OriginalOrAir)
                .await
                .is_err()
        );
        assert!(
            ops.prepare_mining_profile_recovery(record.id)
                .await
                .is_err()
        );
        assert!(
            timeout(Duration::from_millis(10), listener.accept())
                .await
                .is_err()
        );
        drop(release);
        drop(client);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn common_mining_runs_identical_consumer_with_legacy_receipts_and_no_modern_sequence() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        client.survival().select_hotbar(0).await.unwrap();
        assert_eq!(packets.recv().await.unwrap().0, 0x24);
        let record = crate::client::tests::common_mining_start_scenario(
            &client,
            TARGET,
            crate::BlockFace::North,
        )
        .await;
        assert_eq!(record.start.interaction_sequence, None);
        let (id, p) = packets.recv().await.unwrap();
        assert_eq!(id, 0x1b);
        assert_eq!(p.len(), 10);
        assert_eq!(p[0], 0);
        let context = OperationContext {
            generation: bot.connection_generation(),
            source_observation_sequence: 0,
        };
        assert!(
            bot.connection
                .replace_control(context, OperationClass::Normal, ControlState::default())
                .await
                .is_err()
        );
        crate::client::tests::common_mining_finish_scenario(&client, record.id).await;
        for action in [2, 1] {
            let (id, p) = packets.recv().await.unwrap();
            assert_eq!(id, 0x1b);
            assert_eq!(p.len(), 10);
            assert_eq!(p[0], action);
        }
        change(&bot, [12, 66, 12], 0).await;
        assert!(
            client
                .survival()
                .mining_record()
                .await
                .unwrap()
                .unwrap()
                .target_receipt
                .is_none()
        );
        let mut ack = BlockPos {
            x: TARGET[0],
            y: TARGET[1],
            z: TARGET[2],
        }
        .packed()
        .to_be_bytes()
        .to_vec();
        put_varint(&mut ack, 0);
        put_varint(&mut ack, 2);
        ack.push(1);
        bot.apply_packet(0x07, ack).await.unwrap();
        crate::client::tests::common_mining_removal_scenario(&client, record.id).await;
        let observed = client.survival().mining_record().await.unwrap().unwrap();
        assert!(matches!(
            observed.protocol,
            Some(MiningProtocolObservation::LegacyReply {
                action: 2,
                accepted: true,
                ..
            })
        ));
        change(&bot, TARGET, 1).await;
        assert_eq!(
            client
                .survival()
                .mining_record()
                .await
                .unwrap()
                .unwrap()
                .stage,
            MiningStage::ObservedRemoved
        );
        assert_eq!(
            client
                .capture(crate::Region {
                    min: TARGET,
                    max: TARGET
                })
                .await
                .unwrap()
                .world
                .blocks[0]
                .state
                .as_ref()
                .unwrap()
                .name,
            "minecraft:stone"
        );
        assert!(
            timeout(Duration::from_millis(30), packets.recv())
                .await
                .is_err()
        );
        drop(release);
        drop(client);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn common_mining_cancelled_finish_waiter_keeps_single_owned_command_and_fresh_boundary() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        client.survival().select_hotbar(0).await.unwrap();
        packets.recv().await.unwrap();
        let record = client
            .survival()
            .start_mining(TARGET, crate::BlockFace::North)
            .await
            .unwrap();
        packets.recv().await.unwrap();
        change(&bot, [12, 66, 12], 0).await;
        let writer = bot.writer.lock().await;
        let ops = client.survival();
        let waiter = tokio::spawn(async move { ops.finish_mining(record.id).await });
        timeout(Duration::from_secs(1), async {
            loop {
                let run = bot.common_mining.lock().await.clone().unwrap();
                if let Some(finish) = run.record.finish {
                    assert!(!finish.dispatched);
                    assert!(finish.after_sequence > record.start.after_sequence);
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        drop(writer);
        let (id, payload) = timeout(Duration::from_secs(1), packets.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!((id, payload[0], payload.len()), (0x1b, 2, 10));
        let current = client.survival().mining_record().await.unwrap().unwrap();
        assert!(current.finish.unwrap().dispatched);
        assert_eq!(current.stage, MiningStage::PendingAfterFinish);
        assert!(client.survival().finish_mining(record.id).await.is_err());
        assert!(
            timeout(Duration::from_millis(30), packets.recv())
                .await
                .is_err()
        );
        drop(release);
        drop(client);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn common_mining_latches_transient_support_loss_before_restoration_and_target_air() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        client.survival().select_hotbar(0).await.unwrap();
        packets.recv().await.unwrap();
        let record = client
            .survival()
            .start_mining(TARGET, crate::BlockFace::North)
            .await
            .unwrap();
        packets.recv().await.unwrap();
        change(&bot, [8, 64, 8], 0).await;
        change(&bot, [8, 64, 8], 1).await;
        change(&bot, TARGET, 0).await;
        let current = client.survival().mining_record().await.unwrap().unwrap();
        assert_eq!(current.id, record.id);
        assert_eq!(current.stage, MiningStage::RequiresInspection);
        assert!(
            current
                .requires_inspection
                .unwrap()
                .contains("standing geometry")
        );
        assert!(client.survival().finish_mining(record.id).await.is_err());
        assert!(
            timeout(Duration::from_millis(30), packets.recv())
                .await
                .is_err()
        );
        drop(release);
        drop(client);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn common_mining_cached_air_and_unrelated_world_revision_cannot_confirm_removal() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        client.survival().select_hotbar(0).await.unwrap();
        packets.recv().await.unwrap();
        let record = client
            .survival()
            .start_mining(TARGET, crate::BlockFace::North)
            .await
            .unwrap();
        packets.recv().await.unwrap();
        bot.world
            .lock()
            .await
            .set_block_for_test(BlockPos { x: 8, y: 66, z: 11 }, 0);
        change(&bot, [12, 66, 12], 0).await;
        let current = client.survival().mining_record().await.unwrap().unwrap();
        assert_eq!(current.id, record.id);
        assert_eq!(current.stage, MiningStage::Mining);
        assert!(current.target_receipt.is_none());
        change(&bot, TARGET, 0).await;
        crate::client::tests::common_mining_removal_scenario(&client, record.id).await;
        drop(release);
        drop(client);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn common_mining_inventory_interruption_stays_latched_after_later_empty_hand_and_air() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        client.survival().select_hotbar(0).await.unwrap();
        packets.recv().await.unwrap();
        let record = client
            .survival()
            .start_mining(TARGET, crate::BlockFace::North)
            .await
            .unwrap();
        packets.recv().await.unwrap();
        let mut item = vec![0, 0, 36, 1];
        put_varint(&mut item, crate::item_id("dirt").unwrap());
        item.extend([1, 0]);
        bot.apply_packet(0x16, item).await.unwrap();
        let first = client
            .survival()
            .mining_record()
            .await
            .unwrap()
            .unwrap()
            .inventory_change
            .unwrap();
        bot.apply_packet(0x16, vec![0, 0, 36, 0]).await.unwrap();
        change(&bot, TARGET, 0).await;
        let current = client.survival().mining_record().await.unwrap().unwrap();
        assert_eq!(current.stage, MiningStage::RequiresInspection);
        assert_eq!(
            current.inventory_change.unwrap().receive_sequence,
            first.receive_sequence
        );
        assert_eq!(first.kind, MiningInventoryChangeKind::SelectedHandChanged);
        assert!(client.survival().finish_mining(record.id).await.is_err());
        assert!(client.survival().abort_mining(record.id).await.is_err());
        drop(release);
        drop(client);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
}
