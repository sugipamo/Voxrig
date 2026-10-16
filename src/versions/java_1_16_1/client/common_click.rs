//! Actor-owned ordinary PICKUP, retaining predictions separately from receipts.
use super::*;
use crate::client::{
    self as api,
    inventory::{self as contract, click},
};
use contract::{
    InventoryClickButton, InventoryClickId, InventoryClickRecord, InventoryClickSource,
    InventoryClickStage,
};
#[derive(Clone)]
pub(super) struct NativeInventoryClick {
    pub(super) record: InventoryClickRecord,
    pub(super) released: bool,
    receipt_deadline: Option<tokio::time::Instant>,
}
impl Bot {
    pub(super) async fn common_click_posture_changed(&self, pose: i32, receipt: u64) {
        if let Some(run) = super::lock_packet_state(&self.common_inventory_click)
            .await
            .as_mut()
        {
            if run.record.unresolved() {
                if let Some(predicate) =
                    click::Dependency::for_click(&run.record).posture_failure(pose)
                {
                    run.record.invalidation.get_or_insert(click::Invalidation {
                        predicate: predicate.into(),
                        change: click::StateChange::PostureChanged { pose },
                        receipt,
                    });
                    run.record.inspection(predicate);
                }
            }
        }
    }

    async fn common_click_send_owned(
        &self,
        id: InventoryClickId,
        revision: u64,
    ) -> Result<InventoryClickRecord> {
        let _gate = self.coherent_state_gate.lock().await;
        let result = async {
            let record = super::lock_packet_state(&self.common_inventory_click)
                .await
                .as_ref()
                .filter(|s| s.record.id == id)
                .map(|s| s.record.clone())
                .ok_or_else(|| contract::unavailable("click intent superseded"))?;
            if record.requires_inspection.is_some() {
                return Err(contract::unavailable("click interrupted before I/O"));
            }
            if let Some(predicate) = click::Dependency::for_click(&record)
                .posture_failure(self.local_pose.lock().await.unwrap_or(-1))
            {
                return Err(contract::unavailable(predicate));
            }
            let current = self.common_player_unlocked().await?;
            let screen = self
                .common_receipts
                .lock()
                .await
                .container
                .as_ref()
                .map(|s| s.capture(current.session));
            let slots = click::source_slots(&record, &current, screen.as_ref())?;
            if let Some(context) = &record.item_context {
                let registries = self
                    .common_receipts
                    .lock()
                    .await
                    .registries
                    .capture(current.session, current.receive_sequence);
                context.classify(
                    &record.source_before,
                    &record.source_before,
                    &record.prediction.source.value,
                    &registries,
                )?;
            }
            if !contract::same_received_value(
                slots
                    .get(usize::from(record.source_slot))
                    .and_then(Option::as_ref),
                &record.source_before,
            ) || !contract::same_received_value(
                current.inventory.cursor.as_ref(),
                &record.cursor_before,
            ) {
                return Err(contract::unavailable("click capture changed before I/O"));
            }
            {
                let inventory = self.inventory.read().await;
                if !inventory.pending_clicks.is_empty()
                    || inventory
                        .open_window
                        .as_ref()
                        .map_or(0, |s| i32::from(s.id))
                        != record.window_id()
                    || api::legacy_slot(inventory.cursor.as_ref())? != record.cursor_before.value
                {
                    return Err(contract::unavailable(
                        "legacy click cache/owner changed before I/O",
                    ));
                }
            }
            // Original native PICKUP returns source_before. Deliberately use
            // the other nonempty predecessor (or Empty) for actual full resync.
            let processing_insertion = record.source_slot <= 1
                && record.initial_screen.as_ref().is_some_and(|s| {
                    s.menu_name.as_deref().is_some_and(|menu| {
                        api::furnace::native_menu(record.initial.session.version, menu).is_some()
                    })
                });
            let processing_resync = processing_insertion
                && matches!(record.cursor_before.value, api::SlotKnowledge::Item { .. });
            let resync_item = match &record.cursor_before.value {
                api::SlotKnowledge::Item { item } if processing_resync => Some(item),
                _ => None,
            };
            let comparison = if let Some(item) = resync_item {
                // A furnace can consume its predecessor before applying PICKUP.
                // Empty may then MATCH the native return and suppress resync.
                // This comparison-only count exceeds ordinary slot capacities;
                // it is never a predicted/received cursor or placed inventory item.
                // Actual fresh source/cursor and transaction receipts still decide
                // completion; unusual server state cannot turn this into an ACK.
                let mut marker = item.clone();
                marker.count = i8::MAX as u32;
                api::SlotKnowledge::Item { item: marker }
            } else if matches!(record.source_before.value, api::SlotKnowledge::Empty) {
                record.cursor_before.value.clone()
            } else {
                api::SlotKnowledge::Empty
            };
            let native = match &comparison {
                api::SlotKnowledge::Empty => None,
                api::SlotKnowledge::Item { item } => Some(ItemStack {
                    item_id: item.id.value(),
                    count: item.count as i8,
                    nbt: match &item.data {
                        api::ItemData::Default => None,
                        api::ItemData::LegacyNbt { bytes } => Some(bytes.clone()),
                        api::ItemData::ModernComponents { .. } => {
                            unreachable!("legacy received stack cannot contain modern components")
                        }
                    },
                }),
                api::SlotKnowledge::Unavailable => unreachable!("validated predecessor"),
            };
            let action = self
                .connection
                .begin_window_click(id.attempt(), revision, record.window_id() as i8)
                .await
                .map_err(|e| contract::unavailable(format!("click reservation: {e:?}")))?;
            {
                let mut guard = super::lock_packet_state(&self.common_inventory_click).await;
                let r = &mut guard.as_mut().expect("retained").record;
                r.send.legacy_action = Some(action);
                r.send.legacy_comparison = Some(comparison);
                r.send.request_full_resync = processing_resync;
                r.send.after_sequence = self.protocol_packet_sequence.load(Ordering::Acquire);
            }
            if processing_resync {
                self.connection
                    .bounded_processing_click(
                        id.attempt(),
                        record.source_slot,
                        record.button.native(),
                        native,
                    )
                    .await?;
            } else {
                self.connection
                    .bounded_inventory_click(
                        id.attempt(),
                        record.source_slot,
                        record.button.native(),
                        native,
                    )
                    .await?;
            }
            let mut guard = super::lock_packet_state(&self.common_inventory_click).await;
            let run = guard.as_mut().expect("retained");
            run.receipt_deadline = Some(
                tokio::time::Instant::now()
                    + self.connection_options.inventory_click_receipt_timeout,
            );
            self.finite_work.lock().expect("finite work").expire_at(
                crate::client::ledger::finite::Family::Click,
                id.attempt(),
                run.receipt_deadline.expect("deadline"),
            );
            run.record.send.dispatched = true;
            Ok(run.record.clone())
        }
        .await;
        if let Err(e) = &result {
            self.interrupt_common_inventory_click(format!("click submission uncertain: {e}"))
                .await;
        }
        result
    }
    pub(super) async fn interrupt_common_inventory_click(&self, reason: impl std::fmt::Display) {
        if let Some(run) = super::lock_packet_state(&self.common_inventory_click)
            .await
            .as_mut()
        {
            run.record.inspection(reason);
        }
    }
    pub(super) async fn common_click_context_received(&self) -> Result<()> {
        self.reconcile_common_inventory_click().await
    }
    pub(super) async fn common_click_reply_received(&self, reply: WindowTransaction) {
        if let Some(run) = super::lock_packet_state(&self.common_inventory_click)
            .await
            .as_mut()
            .filter(|s| {
                s.record.unresolved()
                    && s.record.send.legacy_action == Some(reply.action)
                    && s.record.window_id() == i32::from(reply.window_id)
                    && reply.packet_sequence > s.record.send.after_sequence
            })
        {
            run.record
                .legacy_reply
                .get_or_insert(contract::InventoryTransactionReply {
                    window_id: reply.window_id,
                    action: reply.action,
                    accepted: reply.accepted,
                    receive_sequence: reply.packet_sequence,
                });
        }
    }
    async fn reconcile_common_inventory_click(&self) -> Result<()> {
        self.with_finite(crate::client::ledger::finite::Family::Click, async {
            let Some(snapshot) = super::lock_packet_state(&self.common_inventory_click)
                .await
                .clone()
            else {
                return Ok(());
            };
            if !snapshot.record.unresolved() {
                return Ok(());
            }
            if self.is_stopped() || self.connection_state() != ConnectionState::Ready {
                self.interrupt_common_inventory_click("click connection closed or uncertain")
                    .await;
                return Ok(());
            }
            let current = self.common_player_unlocked().await?;
            let screen = self
                .common_receipts
                .lock()
                .await
                .container
                .as_ref()
                .map(|s| s.capture(current.session));
            let complete = {
                let mut guard = super::lock_packet_state(&self.common_inventory_click).await;
                let record = &mut guard.as_mut().expect("retained").record;
                let registries = self
                    .common_receipts
                    .lock()
                    .await
                    .registries
                    .capture(current.session, current.receive_sequence);
                if !record.ready()
                    && snapshot
                        .receipt_deadline
                        .is_some_and(|deadline| tokio::time::Instant::now() >= deadline)
                {
                    record.inspection("click receipt deadline elapsed; inspect without replay");
                }
                click::receive_with_registries(record, &current, screen.as_ref(), &registries);
                // Complete at the receive boundary, before a later independent
                // inventory update can overwrite this already-observed outcome.
                // Inspection timing must not decide whether exact receipts count.
                record.ready()
            };
            if complete {
                match self
                    .connection
                    .finish_inventory_click(snapshot.record.id.attempt())
                    .await
                {
                    Ok(()) => {
                        let mut guard =
                            super::lock_packet_state(&self.common_inventory_click).await;
                        let run = guard.as_mut().expect("retained");
                        run.released = true;
                        run.record.stage = InventoryClickStage::ObservedClicked;
                    }
                    Err(e) => {
                        self.interrupt_common_inventory_click(format!(
                            "click completion gate: {e:?}"
                        ))
                        .await
                    }
                }
            }
            Ok(())
        })
        .await
    }
}

impl crate::client::adapter::InventoryClickOps for Bot {
    async fn click_inventory(
        &self,
        mode: api::GameMode,
        source: InventoryClickSource,
        slot: u16,
        button: InventoryClickButton,
    ) -> Result<InventoryClickRecord> {
        self.connection
            .ledger()
            .work()
            .run_scoped(async {
                let gate = self.coherent_state_gate.lock().await;
                self.common_motion_admission().await?;
                if let InventoryClickSource::Container { screen } = source {
                    if super::lock_packet_state(&self.common_container_close)
                        .await
                        .as_ref()
                        .is_some_and(|r| r.id.screen() == screen)
                    {
                        return Err(contract::unavailable(
                            "old opening has a retained close intent",
                        ));
                    }
                }
                let revision = self
                    .connection
                    .motion_admission_revision()
                    .await
                    .map_err(|e| contract::unavailable(format!("click admission: {e:?}")))?;
                if !self.inventory.read().await.pending_clicks.is_empty() {
                    return Err(contract::unavailable("native inventory click unresolved"));
                }
                let attempt = super::lock_packet_state(&self.common_inventory_click)
                    .await
                    .as_ref()
                    .map_or(Some(1), |p| p.record.id.attempt().checked_add(1))
                    .ok_or_else(|| contract::unavailable("inventory click attempts exhausted"))?;
                let initial = self.common_player_unlocked().await?;
                let screen = self
                    .common_receipts
                    .lock()
                    .await
                    .container
                    .as_ref()
                    .map(|s| s.capture(initial.session));
                let registries = self
                    .common_receipts
                    .lock()
                    .await
                    .registries
                    .capture(initial.session, initial.receive_sequence);
                let record = click::prepare_received(
                    (initial, registries),
                    mode,
                    source,
                    slot,
                    button,
                    attempt,
                    screen,
                )?;
                {
                    let inventory = self.inventory.read().await;
                    if inventory
                        .open_window
                        .as_ref()
                        .map_or(0, |s| i32::from(s.id))
                        != record.window_id()
                        || api::legacy_slot(inventory.cursor.as_ref())?
                            != record.cursor_before.value
                    {
                        return Err(contract::unavailable(
                            "legacy click received/cache UI basis disagrees",
                        ));
                    }
                }
                if let Some(predicate) = click::Dependency::for_click(&record)
                    .posture_failure(self.local_pose.lock().await.unwrap_or(-1))
                {
                    return Err(contract::unavailable(predicate));
                }
                let id = record.id;
                let work =
                    self.begin_finite(crate::client::ledger::finite::Family::Click, id.attempt());
                *super::lock_packet_state(&self.common_inventory_click).await =
                    Some(NativeInventoryClick {
                        record,
                        released: false,
                        receipt_deadline: None,
                    });
                let (reply, result) = tokio::sync::oneshot::channel();
                let bot = self.clone_internal();
                self.spawn_finite_work(work, async move {
                    let _ = reply.send(crate::client::ledger::finite::checked(
                        bot.common_click_send_owned(id, revision).await,
                    ));
                });
                drop(gate);
                result
                    .await
                    .map_err(|_| contract::unavailable("click owner result unavailable"))?
            })
            .await
    }
    async fn inventory_click_record(&self) -> Result<Option<InventoryClickRecord>> {
        // A stalled writer must not hide the already-retained intent. Complete
        // reconciliation owns its continuation if the inspection caller cancels.
        if let Ok(gate) = self.coherent_state_gate.clone().try_lock_owned() {
            let bot = self.clone_internal();
            let (reply, result) = tokio::sync::oneshot::channel();
            self.spawn_inspection(crate::client::ledger::finite::Family::Click, async move {
                let _gate = gate;
                let _ = reply.send(crate::client::ledger::finite::checked(
                    bot.reconcile_common_inventory_click().await,
                ));
            });
            result
                .await
                .map_err(|_| contract::unavailable("click inspection owner unavailable"))??;
        }
        Ok(super::lock_packet_state(&self.common_inventory_click)
            .await
            .as_ref()
            .map(|s| s.record.clone()))
    }
}
