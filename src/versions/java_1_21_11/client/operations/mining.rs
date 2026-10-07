//! Retained native empty-hand mining and common default-tool dry mining.
use super::*;
use crate::diagnostic_projection::diagnostic_record;
use std::time::Duration;

#[cfg(test)]
mod tests;

diagnostic_record! {
    /// One owning connection's removal intent, stored before sending START.
    /// Serialized inspection cannot recreate a capability on another connection.
    #[derive(Clone, Debug, PartialEq, Serialize)]
    pub struct MiningIntent => RecordedMiningIntent {
        /// Owning connection.
        pub connection_id: u64,
        /// Receive boundary before START.
        pub after_sequence: u64,
        /// Globally increasing interaction sequence for START.
        pub start_sequence: i32,
        /// Owning dimension.
        pub dimension: String,
        /// Exact requested removal cell.
        pub target: [i32; 3],
        /// Native entry face ID 0..5.
        pub face_id: u8,
        /// Complete received predecessor; never inferred from the item registry.
        pub baseline: crate::NativeBlockState,
        /// Received stationary feet position before submission.
        pub position: [f64; 3],
        /// Main-hand slot selected by an ordered send or server update.
        pub selection: HotbarSelection,
        /// Receive sequence which established the original selected slot.
        pub held_receive_sequence: u64,
        /// Exact original native slot; later changed tools remain latched.
        pub held_stack: InventorySlot,
        /// Common default-item schedule; native compatibility remains empty-hand.
        pub estimate: Option<crate::client::survival::MiningEstimate>,
        /// Local scheduling estimate only; no server-tick or cancellation guarantee.
        pub estimated_wait_ms: u64,
    }
    diagnostic_serde {}
}
/// Separate send attempt retained even if its caller is cancelled.
#[derive(Clone, Debug, Serialize, serde::Deserialize, PartialEq)]
pub struct MiningSend {
    /// Connection-global interaction sequence.
    pub sequence: i32,
    /// Receive boundary before the send attempt.
    pub after_sequence: u64,
    /// Entire packet write completed; never a server acceptance receipt.
    pub dispatched: bool,
}
/// A target-specific received update, independent of global cache revision.
#[derive(Clone, Debug, Serialize, serde::Deserialize, PartialEq)]
pub struct MiningTargetReceipt {
    /// Exact native state from a block/section packet applied to the loaded target.
    pub state: crate::NativeBlockState,
    /// That packet's receive ordinal.
    pub receive_sequence: u64,
}
diagnostic_record! {
    /// Retained observation of the requested result; not a current world snapshot
    /// and does not prove which actor removed it.
    #[derive(Clone, Debug, Serialize)]
    pub struct MiningRemoval => RecordedMiningRemoval {
        /// Original intent and received predecessor.
        pub intent: MiningIntent,
        /// Fresh received air at the exact target.
        pub target_receipt: MiningTargetReceipt,
        /// False: observed air does not establish that server delayed mining has
        /// cleared. No next mutation is authorized on this connection yet.
        pub continuation_validated: bool,
    }
    diagnostic_serde {}
}
// Common ownership, with the native compatibility path preserved.
pub use crate::client::survival::MiningInventoryChangeKind;

diagnostic_record! {
/// First incompatible received inventory state. Diagnostic evidence only; does
/// not attribute an item to pickup, gathering, a player or a server command.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct MiningInventoryChange => RecordedMiningInventoryChange {
    /// Which prerequisite changed first.
    pub kind: MiningInventoryChangeKind,
    /// Receive ordinal at which the incompatible state was applied.
    pub receive_sequence: u64,
    /// Selection at that boundary, retaining send/receive provenance.
    pub selection: Option<HotbarSelection>,
    /// Contents of the original intent's selected player slot.
    pub original_hand: InventorySlot,
    /// Slot-specific receive boundary, if available.
    pub hand_receive_sequence: Option<u64>,
    /// Received active container (zero is the player screen).
    pub window_id: Option<i32>,
    /// Received cursor at that boundary.
    pub cursor: InventorySlot,
    /// Whether the inventory projection contains unsupported components.
    pub unsupported_components: bool,
    /// True only while no other inspection cause has been recorded.
    /// Earlier or later conflicts are never recategorized as inventory-only.
    pub sole_cause: bool,
}

    diagnostic_serde {}
}

const INVENTORY_CHANGED: &str = "received inventory prerequisites changed during mining";
diagnostic_record! {
    /// Last mining attempt, retained on errors, timeouts, context changes and closure.
    #[derive(Clone, Debug, Serialize)]
    pub struct MiningRecord => RecordedMiningRecord {
        /// Stored before the first possible mutation.
        pub intent: MiningIntent,
        /// Complete START frame was dispatched.
        pub start_dispatched: bool,
        /// At most one explicit FINISH attempt, retained before I/O.
        pub finish: Option<MiningSend>,
        /// At most one explicit ABORT attempt; never resolves delayed mining by itself.
        pub abort: Option<MiningSend>,
        /// Latest target-specific received packet after START.
        pub target_receipt: Option<MiningTargetReceipt>,
        /// Latched context/conflict reason. Later air cannot silently clear it.
        pub requires_inspection: Option<String>,
        /// First received inventory interruption, never cleared by a later empty hand.
        pub inventory_change: Option<MiningInventoryChange>,
        /// Result established by a read-only observation. Even Some does not currently
        /// authorize in-session continuation; that release remains unimplemented.
        pub removal: Option<MiningRemoval>,
        /// Once-only fresh login attempt, recorded before I/O. Shared by all recovery
        /// methods; cancellation never permits a different method to open another login.
        pub recovery_attempt: Option<MiningRecoveryAttempt>,
    }
    diagnostic_serde {}
}
diagnostic_record! {
    /// Explicit result states; a pending timeout is never a safe cancellation.
    #[derive(Clone, Debug, Serialize)]
    #[serde(tag = "status", rename_all = "snake_case")]
    pub enum MiningStatus => RecordedMiningStatus {
        /// START attempted, without a FINISH attempt or a confirmed result.
        Mining {
            /// Retained start and target evidence.
            record: MiningRecord,
        },
        /// FINISH attempted; ABORT/acknowledgement do not authorize another mutation.
        PendingAfterFinish {
            /// Retained finish/abort attempts; still unresolved.
            record: MiningRecord,
        },
        /// Exact target received as air after submission, with preserved context.
        ObservedRemoved {
            /// Received result, without attribution of its actor.
            observation: MiningRemoval,
        },
        /// Changed/missing context or target requires inspection; intent remains pending.
        RequiresInspection {
            /// Original intent and latched diagnosis.
            record: MiningRecord,
        },
    }
    diagnostic_serde { #[serde(tag = "status", rename_all = "snake_case")] }
}

fn unavailable(message: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::State, anyhow::anyhow!("{message}"))
}
fn owning(state: &State, intent: &MiningIntent, connection_id: u64) -> Result<()> {
    if intent.connection_id != connection_id
        || state.mining.as_ref().is_none_or(|m| m.intent != *intent)
    {
        return Err(unavailable(
            "mining intent belongs to another connection or attempt",
        ));
    }
    Ok(())
}
fn selected_hand(state: &State, tools: bool) -> Result<(HotbarSelection, u64)> {
    let inventory = &state.operations.inventory;
    let selection = state
        .operations
        .selected_hotbar
        .clone()
        .filter(|s| s.dispatched)
        .ok_or_else(|| unavailable("main-hand selection unavailable or incomplete"))?;
    let slot = 36 + usize::from(selection.slot);
    if inventory.window_id != Some(0)
        || inventory.cursor != InventorySlot::Empty
        || inventory.unsupported_components
        || inventory.pending_swap.is_some()
        || !inventory.pending_creative.is_empty()
        || (if tools {
            inventory.slots[slot] == InventorySlot::Unavailable
        } else {
            inventory.slots[slot] != InventorySlot::Empty
        })
    {
        return Err(unavailable(
            "mining requires a received supported selected hand and player screen",
        ));
    }
    let sequence = inventory.slot_sequences[slot]
        .ok_or_else(|| unavailable("selected hand receive evidence unavailable"))?;
    Ok((selection, sequence))
}
fn common_estimate(
    state: &State,
    connection_id: u64,
    baseline: &crate::NativeBlockState,
) -> Result<crate::client::survival::MiningEstimate> {
    let selection = selected_hand(state, true)?.0;
    let hand = common_slot(&state.operations.inventory.slots[36 + usize::from(selection.slot)])?;
    let registries = state.registries.capture(
        crate::client::SessionStamp {
            version: MinecraftVersion::Java1_21_11,
            connection_id,
            world_generation: state.loading.generation,
        },
        state.sequence,
    );
    crate::client::survival::mining_tools::estimate(
        MinecraftVersion::Java1_21_11,
        baseline,
        &hand,
        Some(&registries),
    )
}
fn prepared(
    state: &mut State,
    connection_id: u64,
    tick: u64,
    target: [i32; 3],
    face: u8,
    tools: bool,
) -> Result<(StandingContext, crate::NativeBlockState)> {
    if state.operations.game_mode != Some(GameMode::Survival) {
        return Err(unavailable("mining requires received survival mode"));
    }
    let standing = survival::context(state, connection_id, tick)?;
    if !standing.on_ground
        || standing.submerged
        || standing.support.contains(&target)
        || !standing
            .player
            .health
            .as_ref()
            .is_some_and(|h| h.health > 0.0)
    {
        return Err(unavailable(
            "mining requires known healthy dry ground and may not remove its foot support",
        ));
    }
    if standing.player.block_break_speed.map(|v| v.value) != Some(1.0)
        || standing.player.mining_efficiency.map(|v| v.value) != Some(0.0)
        || !standing.player.effect_updates.is_empty()
    {
        return Err(unavailable(
            "modified or unavailable mining conditions need inspection",
        ));
    }
    // An empty effect map is NOT a complete-list fence. The estimated wait below
    // is scheduling only; received removal remains the acceptance criterion.
    selected_hand(state, tools)?;
    let hit = super::super::raycast::stationary_outline_hit(state, standing.eye_position, 4.5)?
        .ok_or_else(|| unavailable("no available native outline hit for mining"))?;
    survival::uncertain_target(state, &standing, &hit)?;
    if hit.position != target || hit.face.map(|f| f as u8) != Some(face) {
        return Err(unavailable(
            "target/face differs from the current first native outline hit",
        ));
    }
    if tools {
        crate::client::survival::mining_tools::material(MinecraftVersion::Java1_21_11, &hit.state)?;
        common_estimate(state, connection_id, &hit.state)?;
    } else {
        admitted_material(&hit.state)?;
    }
    Ok((standing, hit.state))
}
// Shared material admission for real mining and hypothetical removal geometry.
pub(super) fn admitted_material(state: &crate::NativeBlockState) -> Result<()> {
    if !matches!(state.name.as_str(), "minecraft:dirt" | "minecraft:stone")
        || !state.properties.is_empty()
    {
        return Err(Error::new(
            ErrorKind::Unsupported,
            anyhow::anyhow!("empty-hand mining currently admits dirt and stone only"),
        ));
    }
    Ok(())
}
fn packet(intent: &MiningIntent, action: u8, sequence: i32) -> Vec<u8> {
    let mut payload = vec![action];
    payload.extend(pack_position(intent.target).to_be_bytes());
    payload.push(intent.face_id);
    put_varint(&mut payload, sequence);
    payload
}

impl Operations {
    /// Begin one bounded stationary empty-hand dirt/stone removal. No other
    /// mutation is allowed afterward, even on observed removal, until a separate
    /// continuation boundary is implemented and verified. Caller owns
    /// site/edit permissions; this native API cannot infer Blueprint permissions.
    pub async fn start_survival_mining(
        &self,
        target: [i32; 3],
        face: crate::BlockFace,
    ) -> Result<MiningIntent> {
        self.start_mining_in(target, face, false).await
    }
    async fn start_mining_in(
        &self,
        target: [i32; 3],
        face: crate::BlockFace,
        capture_common: bool,
    ) -> Result<MiningIntent> {
        let mut state = self.bot.session.state.lock().await;
        self.mutable(&state)?;
        let common = capture_common
            .then(|| self.common_player_unlocked(&state))
            .transpose()?;
        if let Some(initial) = &common {
            if initial.received_pose.is_none()
                || initial.dimension.is_none()
                || !matches!(
                    initial.inventory.cursor.as_ref(),
                    Some(crate::client::ObservedValue {
                        value: crate::client::SlotKnowledge::Empty,
                        source: crate::client::ValueSource::Received { .. },
                    })
                )
            {
                return Err(unavailable(
                    "common mining requires received own pose, dimension and empty cursor",
                ));
            }
        }
        let tick = self.bot.session.started.elapsed().as_millis() as u64 / 50;
        let (standing, baseline) = prepared(
            &mut state,
            self.bot.session.id,
            tick,
            target,
            face as u8,
            common.is_some(),
        )?;
        let (selection, held_receive_sequence) = selected_hand(&state, common.is_some())?;
        let held_stack = state.operations.inventory.slots[36 + usize::from(selection.slot)].clone();
        let estimate = common
            .as_ref()
            .map(|_| common_estimate(&state, self.bot.session.id, &baseline))
            .transpose()?;
        let intent = MiningIntent {
            connection_id: self.bot.session.id,
            after_sequence: state.sequence,
            start_sequence: self.next_sequence()?,
            dimension: standing.dimension,
            target,
            face_id: face as u8,
            position: standing.position,
            selection,
            held_receive_sequence,
            held_stack,
            estimated_wait_ms: if let Some(e) = &estimate {
                e.wait_ms()
            } else if baseline.name == "minecraft:dirt" {
                1100
            } else {
                8500
            },
            baseline,
            estimate,
        };
        state.common_mining = common.map(|initial| CommonMiningCapture {
            initial,
            start_sequence: intent.start_sequence,
        });
        state.mining = Some(MiningRecord {
            intent: intent.clone(),
            start_dispatched: false,
            finish: None,
            abort: None,
            target_receipt: None,
            requires_inspection: None,
            inventory_change: None,
            removal: None,
            recovery_attempt: None,
        });
        self.bot
            .session
            .send(
                ids::play_serverbound::BLOCK_DIG,
                &packet(&intent, 0, intent.start_sequence),
            )
            .await?;
        state
            .mining
            .as_mut()
            .expect("stored mining intent")
            .start_dispatched = true;
        Ok(intent)
    }

    /// Attempt FINISH once for the original target. An early finish can schedule
    /// a delayed server break. No elapsed duration or acknowledgement resolves it.
    pub async fn finish_survival_mining(&self, intent: &MiningIntent) -> Result<i32> {
        self.mining_send(intent, false).await
    }
    /// Attempt ABORT once. This stops ordinary native mining, but does not prove
    /// that a delayed FINISH was cleared; pending intent remains until observation.
    pub async fn abort_survival_mining(&self, intent: &MiningIntent) -> Result<i32> {
        self.mining_send(intent, true).await
    }
    async fn mining_send(&self, intent: &MiningIntent, abort: bool) -> Result<i32> {
        let mut state = self.bot.session.state.lock().await;
        self.ready(&state)?;
        owning(&state, intent, self.bot.session.id)?;
        let record = state.mining.as_ref().expect("owning intent");
        if !record.start_dispatched
            || record.removal.is_some()
            || record.requires_inspection.is_some()
            || (if abort {
                record.abort.is_some()
            } else {
                record.finish.is_some() || record.abort.is_some()
            })
            || state.world.dimension.as_ref().map(|d| &d.0) != Some(&intent.dimension)
        {
            return Err(unavailable(
                "mining send cannot resume/replay this stage or changed context",
            ));
        }
        if !abort {
            let tick = self.bot.session.started.elapsed().as_millis() as u64 / 50;
            let validation = prepared(
                &mut state,
                self.bot.session.id,
                tick,
                intent.target,
                intent.face_id,
                intent.estimate.is_some(),
            );
            match validation {
                Ok((standing, block))
                    if block == intent.baseline
                        && standing.position == intent.position
                        && selected_hand(&state, intent.estimate.is_some())?.0
                            == intent.selection
                        && state.operations.inventory.slots
                            [36 + usize::from(intent.selection.slot)]
                            == intent.held_stack => {}
                outcome => {
                    let reason = match outcome {
                        Err(e) => e.to_string(),
                        _ => "mining baseline or player context changed".into(),
                    };
                    mining_world_changed(&mut state, &reason);
                    return Err(unavailable(reason));
                }
            }
        }
        let sequence = self.next_sequence()?;
        let attempt = MiningSend {
            sequence,
            after_sequence: state.sequence,
            dispatched: false,
        };
        let record = state.mining.as_mut().expect("owning intent");
        if abort {
            record.abort = Some(attempt);
        } else {
            record.finish = Some(attempt);
        }
        self.bot
            .session
            .send(
                ids::play_serverbound::BLOCK_DIG,
                &packet(intent, if abort { 1 } else { 2 }, sequence),
            )
            .await?;
        let record = state.mining.as_mut().expect("owning intent");
        (if abort {
            &mut record.abort
        } else {
            &mut record.finish
        })
        .as_mut()
        .expect("send attempt")
        .dispatched = true;
        Ok(sequence)
    }

    /// Read-only result reconciliation. Once removal is observed, that result is
    /// retained as history; use a fresh world query for the current target.
    /// Exact target freshness is required for first confirmation; cached air,
    /// unrelated packets, acknowledgements and ABORT never stand in for removal.
    pub async fn observe_survival_mining(&self, intent: &MiningIntent) -> Result<MiningStatus> {
        let mut state = self.bot.session.state.lock().await;
        self.bot.session.check(&state)?;
        owning(&state, intent, self.bot.session.id)?;
        Ok(status(&mut state))
    }
    /// Wait without resending. Timeout returns a pending status and retains the
    /// intent; a cancelled future does the same. Closed connections expose history.
    pub async fn wait_survival_mining(
        &self,
        intent: &MiningIntent,
        maximum_wait: Duration,
    ) -> Result<MiningStatus> {
        if maximum_wait.is_zero() || maximum_wait > Duration::from_secs(30) {
            return Err(invalid(
                "mining wait must be greater than zero and at most 30 seconds",
            ));
        }
        let result = timeout(maximum_wait, async {
            loop {
                let notified = self.bot.session.changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                let observed = self.observe_survival_mining(intent).await?;
                if matches!(
                    observed,
                    MiningStatus::ObservedRemoved { .. } | MiningStatus::RequiresInspection { .. }
                ) {
                    return Ok(observed);
                }
                notified.await;
            }
        })
        .await;
        match result {
            Ok(result) => result,
            Err(_) => self.observe_survival_mining(intent).await,
        }
    }
    /// Convenience scheduling only. Cancellation never sends an automatic abort
    /// or clears the intent. Resume by inspecting the stored stage, not replaying.
    pub async fn dig_survival_cube(
        &self,
        target: [i32; 3],
        face: crate::BlockFace,
        maximum_wait: Duration,
    ) -> Result<MiningStatus> {
        if maximum_wait.is_zero() || maximum_wait > Duration::from_secs(30) {
            return Err(invalid(
                "mining wait must be greater than zero and at most 30 seconds",
            ));
        }
        let intent = self.start_survival_mining(target, face).await?;
        tokio::time::sleep(Duration::from_millis(intent.estimated_wait_ms)).await;
        let observed = self.observe_survival_mining(&intent).await?;
        if matches!(
            observed,
            MiningStatus::ObservedRemoved { .. } | MiningStatus::RequiresInspection { .. }
        ) {
            return Ok(observed);
        }
        if let Err(error) = self.finish_survival_mining(&intent).await {
            // A receive can invalidate prerequisites between observation and
            // FINISH admission. Return its evidence, never retry or hide I/O errors.
            if let Ok(MiningStatus::RequiresInspection { record }) =
                self.observe_survival_mining(&intent).await
            {
                if record.finish.is_none() && error.kind() == ErrorKind::State {
                    return Ok(MiningStatus::RequiresInspection { record });
                }
            }
            return Err(error);
        }
        self.wait_survival_mining(&intent, maximum_wait).await
    }
}

/// Called after atomic inventory/selection packet application, not just on poll.
pub(super) fn mining_inventory_received(state: &mut State) {
    let Some(record) = state.mining.as_mut() else {
        return;
    };
    if record.removal.is_some() || record.inventory_change.is_some() {
        return;
    }
    let inventory = &state.operations.inventory;
    let original_slot = 36 + usize::from(record.intent.selection.slot);
    let kind = if state.operations.selected_hotbar.as_ref() != Some(&record.intent.selection) {
        MiningInventoryChangeKind::SelectionChanged
    } else if inventory.window_id != Some(0) || inventory.cursor != InventorySlot::Empty {
        MiningInventoryChangeKind::PlayerScreenChanged
    } else if inventory.unsupported_components
        || inventory.pending_swap.is_some()
        || !inventory.pending_creative.is_empty()
    {
        MiningInventoryChangeKind::InventoryUnavailable
    } else if inventory.slots[original_slot] != record.intent.held_stack {
        MiningInventoryChangeKind::SelectedHandChanged
    } else {
        return;
    };
    record.inventory_change = Some(MiningInventoryChange {
        kind,
        receive_sequence: state.sequence,
        selection: state.operations.selected_hotbar.clone(),
        original_hand: inventory.slots[original_slot].clone(),
        hand_receive_sequence: inventory.slot_sequences[original_slot],
        window_id: inventory.window_id,
        cursor: inventory.cursor.clone(),
        unsupported_components: inventory.unsupported_components,
        sole_cause: record.requires_inspection.is_none(),
    });
    record
        .requires_inspection
        .get_or_insert_with(|| INVENTORY_CHANGED.into());
}

fn inspect_other(record: &mut MiningRecord, reason: &str) {
    if let Some(change) = &mut record.inventory_change {
        change.sole_cause = false;
    }
    record
        .requires_inspection
        .get_or_insert_with(|| reason.into());
}

fn status(state: &mut State) -> MiningStatus {
    let record = state.mining.as_mut().expect("owning intent checked");
    if let Some(observation) = &record.removal {
        return MiningStatus::ObservedRemoved {
            observation: observation.clone(),
        };
    }
    if state.world.dimension.as_ref().map(|d| &d.0) != Some(&record.intent.dimension)
        || !state.ready
    {
        inspect_other(record, "mining world context unavailable or changed");
    }
    let cell = state
        .reconstruction
        .cell(&state.world, record.intent.target);
    if cell.moving.is_some()
        || cell.state.is_none()
        || state.reconstruction.issue.is_some()
        || !state.reconstruction.recovery_chunks.is_empty()
    {
        inspect_other(record, "mining target reconstruction unavailable or moving");
    }
    if record.requires_inspection.is_some() {
        return MiningStatus::RequiresInspection {
            record: record.clone(),
        };
    }
    if let Some(receipt) = record.target_receipt.as_ref().filter(|receipt| {
        receipt.receive_sequence > record.intent.after_sequence
            && matches!(
                receipt.state.name.as_str(),
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            )
            && cell.state.as_ref() == Some(&receipt.state)
    }) {
        let observation = MiningRemoval {
            intent: record.intent.clone(),
            target_receipt: receipt.clone(),
            continuation_validated: false,
        };
        record.removal = Some(observation.clone());
        return MiningStatus::ObservedRemoved { observation };
    }
    if record.finish.is_some() {
        MiningStatus::PendingAfterFinish {
            record: record.clone(),
        }
    } else {
        MiningStatus::Mining {
            record: record.clone(),
        }
    }
}

pub(in crate::versions::java_1_21_11::client) fn mining_world_changed(
    state: &mut State,
    reason: &str,
) {
    if let Some(watch) = &mut state.retirement {
        watch
            .requires_inspection
            .get_or_insert_with(|| reason.into());
    }
    if let Some(record) = state.mining.as_mut().filter(|r| r.removal.is_none()) {
        inspect_other(record, reason);
    }
}
pub(in crate::versions::java_1_21_11::client) fn mining_chunk_changed(
    state: &mut State,
    chunk: [i32; 2],
) {
    if state.mining.as_ref().is_some_and(|m| {
        m.removal.is_none()
            && [
                m.intent.target[0].div_euclid(16),
                m.intent.target[2].div_euclid(16),
            ] == chunk
    }) {
        mining_world_changed(
            state,
            "target chunk unloaded or replaced; fresh inspection required",
        );
    }
}
pub(in crate::versions::java_1_21_11::client) fn mining_received(
    state: &mut State,
    changes: &[([i32; 3], i32)],
) -> anyhow::Result<()> {
    let Some(record) = &mut state.mining else {
        return Ok(());
    };
    if record.removal.is_some() {
        return Ok(());
    }
    for (position, id) in changes {
        if *position != record.intent.target {
            continue;
        }
        let block = super::super::super::native_state(*id)?;
        if state.world.block(*position) != Some(*id) {
            inspect_other(record, "target update without loaded baseline");
            continue;
        }
        if block != record.intent.baseline
            && !matches!(
                block.name.as_str(),
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            )
        {
            inspect_other(record, "target changed to an unexpected non-air state");
        }
        record.target_receipt = Some(MiningTargetReceipt {
            state: block,
            receive_sequence: state.sequence,
        });
    }
    Ok(())
}

/// Kept under the same state lock before START I/O; not captured after an await.
#[derive(Clone)]
pub(in crate::versions::java_1_21_11::client) struct CommonMiningCapture {
    initial: crate::client::PlayerObservation,
    start_sequence: i32,
}
impl CommonMiningCapture {
    fn id(&self) -> crate::client::survival::MiningId {
        crate::client::survival::MiningId::new(self.initial.session, self.start_sequence as u64 + 1)
    }
}
// Common mining keeps its dry standing prerequisites at every receive boundary.
// Restoration in a later packet must not erase a transient loss of support,
// body clearance, posture or the owning pose. This does not reinterpret a
// native-only mining record or turn target air into continuation authority.
pub(in crate::versions::java_1_21_11::client) fn common_mining_context_received(state: &mut State) {
    let Some(capture) = state.common_mining.clone() else {
        return;
    };
    if state.mining.as_ref().is_none_or(|m| m.removal.is_some()) {
        return;
    }
    let check = (|| -> Result<()> {
        let initial = &capture.initial;
        if state.operations.game_mode != Some(GameMode::Survival)
            || state.position != initial.position.as_ref().map(|p| p.value)
            || state.rotation != initial.rotation
            || state.loading.generation != initial.session.world_generation
            || state
                .motion
                .received_pose
                .as_ref()
                .map(|p| p.receive_sequence)
                != initial.received_pose.as_ref().map(|p| p.receive_sequence)
        {
            return Err(unavailable("mining player/world context changed"));
        }
        let standing = survival::context(
            state,
            initial.session.connection_id,
            state.reconstruction.tick,
        )?;
        if !standing.on_ground
            || !standing
                .player
                .health
                .as_ref()
                .is_some_and(|h| h.health > 0.0)
            || standing.player.block_break_speed.map(|v| v.value) != Some(1.0)
            || standing.player.mining_efficiency.map(|v| v.value) != Some(0.0)
            || !standing.player.effect_updates.is_empty()
        {
            return Err(unavailable(
                "mining healthy dry standing prerequisites changed",
            ));
        }
        if let Some(intent) = state
            .mining
            .as_ref()
            .map(|m| m.intent.clone())
            .filter(|i| i.estimate.is_some())
        {
            let registries = state.registries.capture(initial.session, state.sequence);
            crate::client::survival::mining_tools::estimate(
                MinecraftVersion::Java1_21_11,
                &intent.baseline,
                &common_slot(&intent.held_stack)?,
                Some(&registries),
            )?;
        }
        Ok(())
    })();
    if let Err(error) = check {
        mining_world_changed(state, &format!("common mining context changed: {error}"));
    } else if state.mining.as_ref().is_some_and(|m| {
        m.start_dispatched
            && m.intent.estimate.is_some()
            && m.intent.held_stack != InventorySlot::Empty
    }) {
        // Preserve fresh removal before a later durability packet, without
        // releasing this source or inventing a completion ACK.
        let _ = status(state);
    }
}
impl Operations {
    pub(crate) async fn common_profile_recovery_watch(
        &self,
        id: crate::client::survival::MiningId,
    ) -> Result<MiningProfileRecoveryWatch> {
        let intent = {
            let state = self.bot.session.state.lock().await;
            let capture = state
                .common_mining
                .as_ref()
                .filter(|c| c.id() == id)
                .ok_or_else(|| unavailable("recovery belongs to another common mining attempt"))?;
            state
                .mining
                .as_ref()
                .filter(|m| m.intent.start_sequence == capture.start_sequence)
                .ok_or_else(|| unavailable("common mining no longer owns native intent"))?
                .intent
                .clone()
        };
        self.prepare_survival_mining_profile_recovery(&intent).await
    }
}

impl crate::client::adapter::MiningOps for Operations {
    async fn start_mining(
        &self,
        target: [i32; 3],
        face: crate::BlockFace,
    ) -> Result<crate::client::survival::MiningRecord> {
        self.start_mining_in(target, face, true).await?;
        self.mining_record()
            .await?
            .ok_or_else(|| unavailable("common mining capture missing"))
    }
    async fn mining_send(
        &self,
        id: crate::client::survival::MiningId,
        action: crate::client::survival::MiningAction,
    ) -> Result<crate::client::survival::MiningRecord> {
        let intent = {
            let state = self.bot.session.state.lock().await;
            if !state.common_mining.as_ref().is_some_and(|m| m.id() == id) {
                return Err(unavailable(
                    "common mining id belongs to another connection/attempt",
                ));
            }
            state
                .mining
                .as_ref()
                .filter(|m| {
                    Some(m.intent.start_sequence)
                        == state.common_mining.as_ref().map(|c| c.start_sequence)
                })
                .ok_or_else(|| unavailable("common mining no longer owns native intent"))?
                .intent
                .clone()
        };
        match action {
            crate::client::survival::MiningAction::Start => {
                return Err(unavailable("START may not be replayed"));
            }
            crate::client::survival::MiningAction::Finish => {
                self.finish_survival_mining(&intent).await?;
            }
            crate::client::survival::MiningAction::Abort => {
                self.abort_survival_mining(&intent).await?;
            }
        }
        self.mining_record()
            .await?
            .ok_or_else(|| unavailable("common mining capture missing"))
    }
    async fn mining_record(&self) -> Result<Option<crate::client::survival::MiningRecord>> {
        use crate::client::{self as common, survival as api};
        let mut state = self.bot.session.state.lock().await;
        let Some(capture) = state.common_mining.clone() else {
            return Ok(None);
        };
        if !state
            .mining
            .as_ref()
            .is_some_and(|m| m.intent.start_sequence == capture.start_sequence)
        {
            return Err(unavailable(
                "retained common mining no longer owns native intent",
            ));
        }
        if self
            .bot
            .session
            .stopped
            .load(std::sync::atomic::Ordering::Acquire)
            || state.failure.is_some()
            || !state.ready
            || !matches!(state.phase, Phase::Play)
        {
            mining_world_changed(&mut state, "mining connection closed or uncertain");
        }
        if state.mining.as_ref().is_some_and(|m| {
            !m.start_dispatched
                && m.target_receipt
                    .as_ref()
                    .is_some_and(|r| r.state != m.intent.baseline)
        }) {
            mining_world_changed(
                &mut state,
                "target changed without a complete common START dispatch",
            );
        }
        let observed = status(&mut state);
        let native = state.mining.as_ref().expect("owning native mining");
        let stage = match observed {
            MiningStatus::Mining { .. } => api::MiningStage::Mining,
            MiningStatus::PendingAfterFinish { .. } => api::MiningStage::PendingAfterFinish,
            MiningStatus::ObservedRemoved { .. } => api::MiningStage::ObservedRemoved,
            MiningStatus::RequiresInspection { .. } => api::MiningStage::RequiresInspection,
        };
        let inventory_change = native
            .inventory_change
            .as_ref()
            .map(|change| -> Result<_> {
                Ok(api::MiningInventoryChange {
                    kind: change.kind,
                    receive_sequence: change.receive_sequence,
                    selection: change.selection.as_ref().map(|s| common::ObservedValue {
                        value: s.slot,
                        source: if s.from_server {
                            common::ValueSource::Received {
                                sequence: s.sequence,
                            }
                        } else {
                            common::ValueSource::Submitted
                        },
                    }),
                    original_hand: super::common_slot(&change.original_hand)?,
                    hand_receive_sequence: change.hand_receive_sequence,
                    window_id: change.window_id,
                    cursor: super::common_slot(&change.cursor)?,
                    unsupported_components: change.unsupported_components,
                    sole_cause: change.sole_cause,
                })
            })
            .transpose()?;
        let send = |s: &MiningSend| api::MiningSend {
            after_sequence: s.after_sequence,
            interaction_sequence: Some(s.sequence),
            dispatched: s.dispatched,
        };
        let protocol = state
            .operations
            .ack
            .zip(state.operations.ack_receive_sequence)
            .filter(|(sequence, receive)| {
                *sequence >= native.intent.start_sequence && *receive > native.intent.after_sequence
            })
            .map(
                |(sequence, receive_sequence)| api::MiningProtocolObservation::ModernProcessing {
                    sequence,
                    receive_sequence,
                },
            );
        Ok(Some(api::MiningRecord {
            id: capture.id(),
            initial: capture.initial,
            target: native.intent.target,
            face: [
                crate::BlockFace::Down,
                crate::BlockFace::Up,
                crate::BlockFace::North,
                crate::BlockFace::South,
                crate::BlockFace::West,
                crate::BlockFace::East,
            ][native.intent.face_id as usize],
            baseline: native.intent.baseline.clone(),
            estimated_wait_ms: native.intent.estimated_wait_ms,
            estimate: native
                .intent
                .estimate
                .clone()
                .ok_or_else(|| unavailable("common mining estimate missing"))?,
            start: api::MiningSend {
                after_sequence: native.intent.after_sequence,
                interaction_sequence: Some(native.intent.start_sequence),
                dispatched: native.start_dispatched,
            },
            finish: native.finish.as_ref().map(send),
            abort: native.abort.as_ref().map(send),
            target_receipt: native
                .target_receipt
                .as_ref()
                .map(|r| api::MiningTargetReceipt {
                    state: r.state.clone(),
                    receive_sequence: r.receive_sequence,
                }),
            protocol,
            inventory_change,
            requires_inspection: native.requires_inspection.clone(),
            stage,
            recovery_attempt: native.recovery_attempt.clone(),
            continuation_validated: false,
        }))
    }
}
