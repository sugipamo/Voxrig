//! Finite workers keep the original root across every timer and nested send.
use super::*;
use crate::client::ledger::finite::Family;
use std::future::Future;
impl Session {
    pub(super) fn spawn_finite_result<F, T, E>(
        self: &Arc<Self>,
        future: F,
    ) -> crate::client::shutdown::Task<std::result::Result<T, E>>
    where
        F: Future<Output = std::result::Result<T, E>> + Send + 'static,
        T: Send + 'static,
        E: Send + 'static,
    {
        self.spawn_finite(async move { crate::client::ledger::finite::checked(future.await) })
    }
    pub(super) fn spawn_finite<F>(
        self: &Arc<Self>,
        future: F,
    ) -> crate::client::shutdown::Task<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.spawn_finite_work(self.ledger.work(), future)
    }
    pub(super) fn spawn_finite_work<F>(
        self: &Arc<Self>,
        work: WorkGuard,
        future: F,
    ) -> crate::client::shutdown::Task<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let session = self.clone();
        work.spawn(async move {
            let output = future.await;
            session.state.lock().await.reconcile_finite();
            output
        })
    }
}
impl State {
    pub(super) fn reconcile_finite(&mut self) {
        let execution = self.inventory_executor.record();
        self.finite_work.reconcile(
            Family::InventoryResync,
            execution.as_ref().filter(|r| r.resync).map(|r| {
                (
                    r.id.number(),
                    matches!(
                        r.status,
                        crate::client::inventory::InventorySendStatus::Received
                            | crate::client::inventory::InventorySendStatus::Diverged
                    ),
                    r.status == crate::client::inventory::InventorySendStatus::ResyncRequired,
                )
            }),
        );

        if let Some(execution) = self
            .inventory_executor
            .record()
            .filter(|r| r.status == crate::client::inventory::InventorySendStatus::ResyncRequired)
        {
            if let Some(r) = self.common_inventory_click.as_mut().filter(|r| {
                r.initial.session == execution.id.session()
                    && r.send.after_sequence == execution.after_sequence
            }) {
                r.inspection(
                    "inventory response/state deadline or opening boundary requires resync",
                );
            }
            if let Some(r) = self.common_inventory_transfer.as_mut().filter(|r| {
                r.initial.session == execution.id.session()
                    && r.send.after_sequence == execution.after_sequence
            }) {
                r.inspection(
                    "inventory response/state deadline or opening boundary requires resync",
                );
            }
            if let Some(r) = self.common_crafting_take.as_mut().filter(|r| {
                r.initial.session == execution.id.session()
                    && r.send.after_sequence == execution.after_sequence
            }) {
                r.inspection(
                    "inventory response/state deadline or opening boundary requires resync",
                );
            }
        }

        // A full boundary unchanged from `before` ends the strict record bound
        // to that send; without it the record would wait for receipts forever.
        let unchanged = self.inventory_executor.unchanged_boundary();
        if let Some(r) = self.common_inventory_click.as_mut() {
            r.observe_unchanged(unchanged);
        }
        if let Some(r) = self.common_inventory_transfer.as_mut() {
            r.observe_unchanged(unchanged);
        }

        self.finite_work.reconcile(
            Family::Swap,
            operations::inventory::common::swap_ledger_state(self),
        );
        self.finite_work.reconcile(
            Family::HandSwap,
            self.common_hand_swap.as_ref().map(|r| {
                (
                    r.id.attempt(),
                    !r.unresolved(),
                    r.requires_inspection.is_some(),
                )
            }),
        );
        self.finite_work.reconcile(
            Family::SelectedDrop,
            self.common_selected_drop.as_ref().map(|r| {
                (
                    r.id.attempt(),
                    !r.unresolved(),
                    r.requires_inspection.is_some(),
                )
            }),
        );
        for slot in 0..9 {
            self.finite_work.reconcile(
                Family::Creative(slot),
                Some((
                    u64::from(slot) + 1,
                    !operations::inventory::creative_pending(self, slot),
                    !self.ready || self.failure.is_some(),
                )),
            );
        }
        self.finite_work.reconcile(
            Family::Click,
            self.common_inventory_click.as_ref().map(|r| {
                (
                    r.id.attempt(),
                    r.ready() || !r.unresolved(),
                    r.requires_inspection.is_some(),
                )
            }),
        );
        self.finite_work.reconcile(
            Family::Transfer,
            self.common_inventory_transfer.as_ref().map(|r| {
                (
                    r.id.attempt(),
                    r.ready() || !r.unresolved(),
                    r.requires_inspection.is_some(),
                )
            }),
        );
        self.finite_work.reconcile(
            Family::Crafting,
            self.common_crafting_take.as_ref().map(|r| {
                (
                    r.id.attempt(),
                    !r.unresolved(),
                    r.requires_inspection.is_some(),
                )
            }),
        );
        self.finite_work.reconcile(
            Family::Recipe,
            self.common_recipe_placement.as_ref().map(|r| {
                (
                    r.id.attempt(),
                    !r.unresolved(),
                    r.requires_inspection.is_some(),
                )
            }),
        );
        self.finite_work.reconcile(
            Family::ContainerOpen,
            self.common_container_open.as_ref().map(|r| {
                (
                    r.id.attempt(),
                    !r.unresolved(),
                    r.requires_inspection.is_some(),
                )
            }),
        );
        self.finite_work.reconcile(
            Family::Placement,
            self.placement.as_ref().map(|r| {
                (
                    r.intent.sequence as u64,
                    r.is_resolved(),
                    r.requires_inspection.is_some(),
                )
            }),
        );
        self.finite_work.reconcile(
            Family::Mining,
            self.mining.as_ref().map(|r| {
                (
                    r.intent.start_sequence as u64,
                    r.is_resolved(),
                    r.requires_inspection.is_some(),
                )
            }),
        );
    }
}
