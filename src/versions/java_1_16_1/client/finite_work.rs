//! Private finite owners, separate from continuous producer lifetimes.
use super::*;
use crate::client::ledger::{WorkGuard, finite::Family};
use std::future::Future;
impl Bot {
    pub(super) async fn with_finite<F: Future>(&self, family: Family, future: F) -> F::Output {
        let work = self.finite_work.lock().expect("finite work").child(family);
        let output = if let Some(work) = work {
            work.run_scoped(future).await
        } else {
            future.await
        };
        self.reconcile_finite().await;
        output
    }
    pub(super) fn spawn_inspection<F>(
        &self,
        family: Family,
        future: F,
    ) -> impl Future<Output = ()> + Send
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let work = (self.connection_state() == ConnectionState::Ready)
            .then(|| self.finite_work.lock().expect("finite work").child(family))
            .flatten();
        let future = Box::pin(future);
        let inspection = if let Some(work) = work {
            // Accepted reconciliation stays connection-owned. Its existing
            // reply channel need not wait for unrelated finite-book cleanup.
            self.spawn_finite_work(work, future);
            None
        } else {
            // Read-only history inspection remains available after shutdown.
            Some(future)
        };
        async move {
            if let Some(inspection) = inspection {
                inspection.await;
            }
        }
    }

    pub(super) fn begin_finite(&self, family: Family, id: u64) -> WorkGuard {
        self.finite_work
            .lock()
            .expect("finite work")
            .begin(self.connection.ledger(), family, id)
    }
    pub(super) fn spawn_finite_result<F, T, E>(
        &self,
        future: F,
    ) -> crate::client::shutdown::Task<std::result::Result<T, E>>
    where
        F: Future<Output = std::result::Result<T, E>> + Send + 'static,
        T: Send + 'static,
        E: Send + 'static,
    {
        self.spawn_finite(async move { crate::client::ledger::finite::checked(future.await) })
    }
    pub(super) fn spawn_finite<F>(&self, future: F) -> crate::client::shutdown::Task<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.spawn_finite_work(self.connection.ledger().work(), future)
    }
    pub(super) fn spawn_finite_work<F>(
        &self,
        work: WorkGuard,
        future: F,
    ) -> crate::client::shutdown::Task<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let bot = self.clone_internal();
        work.spawn(async move {
            let output = future.await;
            bot.reconcile_finite().await;
            output
        })
    }
    pub(super) async fn reconcile_finite(&self) {
        let execution = lock_packet_state(&self.inventory_executor).await.record();
        self.finite_work.lock().expect("finite work").reconcile(
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
        if let Some(execution) = execution
            .filter(|r| r.status == crate::client::inventory::InventorySendStatus::ResyncRequired)
        {
            if let Some(r) = super::lock_packet_state(&self.common_inventory_click)
                .await
                .as_mut()
                .filter(|r| {
                    r.record.initial.session == execution.id.session()
                        && r.record.send.after_sequence == execution.after_sequence
                })
            {
                r.record.inspection(
                    "inventory response/state deadline or opening boundary requires resync",
                );
            }
            if let Some(r) = super::lock_packet_state(&self.common_inventory_transfer)
                .await
                .as_mut()
                .filter(|r| {
                    r.record.initial.session == execution.id.session()
                        && r.record.send.after_sequence == execution.after_sequence
                })
            {
                r.record.inspection(
                    "inventory response/state deadline or opening boundary requires resync",
                );
            }
            if let Some(r) = super::lock_packet_state(&self.common_crafting_take)
                .await
                .as_mut()
                .filter(|r| {
                    r.record.initial.session == execution.id.session()
                        && r.record.send.after_sequence == execution.after_sequence
                })
            {
                r.record.inspection(
                    "inventory response/state deadline or opening boundary requires resync",
                );
            }
        }

        let state = super::lock_packet_state(&self.common_inventory_swap)
            .await
            .as_ref()
            .map(|r| {
                (
                    r.record.id.attempt(),
                    !self
                        .inventory_executor
                        .try_lock()
                        .map_or(true, |e| e.busy()),
                    r.record.requires_inspection.is_some(),
                )
            });
        self.finite_work
            .lock()
            .expect("finite work")
            .reconcile(Family::Swap, state);
        let state = super::lock_packet_state(&self.common_inventory_click)
            .await
            .as_ref()
            .map(|r| {
                (
                    r.record.id.attempt(),
                    !self
                        .inventory_executor
                        .try_lock()
                        .map_or(true, |e| e.busy()),
                    r.record.requires_inspection.is_some(),
                )
            });
        self.finite_work
            .lock()
            .expect("finite work")
            .reconcile(Family::Click, state);
        let state = super::lock_packet_state(&self.common_inventory_transfer)
            .await
            .as_ref()
            .map(|r| {
                (
                    r.record.id.attempt(),
                    !self
                        .inventory_executor
                        .try_lock()
                        .map_or(true, |e| e.busy()),
                    r.record.requires_inspection.is_some(),
                )
            });
        self.finite_work
            .lock()
            .expect("finite work")
            .reconcile(Family::Transfer, state);
        let state = super::lock_packet_state(&self.common_crafting_take)
            .await
            .as_ref()
            .map(|r| {
                (
                    r.record.id.attempt(),
                    !self
                        .inventory_executor
                        .try_lock()
                        .map_or(true, |e| e.busy()),
                    r.record.requires_inspection.is_some(),
                )
            });
        self.finite_work
            .lock()
            .expect("finite work")
            .reconcile(Family::Crafting, state);
        let state = super::lock_packet_state(&self.common_recipe_placement)
            .await
            .as_ref()
            .map(|r| {
                (
                    r.record.id.attempt(),
                    r.released,
                    r.record.requires_inspection.is_some(),
                )
            });
        self.finite_work
            .lock()
            .expect("finite work")
            .reconcile(Family::Recipe, state);
        let state = super::lock_packet_state(&self.common_placement)
            .await
            .as_ref()
            .map(|r| {
                (
                    r.record.id.attempt(),
                    r.released,
                    r.record.requires_inspection.is_some(),
                )
            });
        self.finite_work
            .lock()
            .expect("finite work")
            .reconcile(Family::Placement, state);
        let state = super::lock_packet_state(&self.common_container_open)
            .await
            .as_ref()
            .map(|r| {
                (
                    r.record.id.attempt(),
                    r.released,
                    r.record.requires_inspection.is_some(),
                )
            });
        self.finite_work
            .lock()
            .expect("finite work")
            .reconcile(Family::ContainerOpen, state);
        let state = super::lock_packet_state(&self.common_mining)
            .await
            .as_ref()
            .map(|r| {
                (
                    r.record.id.attempt(),
                    r.record.stage == crate::client::survival::MiningStage::ObservedRemoved,
                    r.record.requires_inspection.is_some(),
                )
            });
        self.finite_work
            .lock()
            .expect("finite work")
            .reconcile(Family::Mining, state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::adapter::PathMotionOps;
    use tokio::time::{Duration, timeout};

    #[tokio::test]
    async fn ledger_native_timer_cancellation_preserves_no_automatic_release() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        let owner = bot.clone_internal();
        let task = tokio::spawn(async move {
            owner
                .use_item_for(Hand::Main, Duration::from_secs(60))
                .await
        });
        assert_eq!(packets.recv().await.unwrap().0, 0x2e);
        let mut drain = bot.connection.ledger().cut(false);
        assert!(drain.inspect().is_none());
        let idle_writer = bot.writer.lock().await;
        drop(idle_writer);
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(drain.wait().await.is_err());
        assert!(packets.try_recv().is_err());
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn ledger_native_dig_timer_drop_keeps_start_without_finish() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        let target = BlockPos { x: 9, y: 65, z: 8 };
        bot.world.lock().await.set_block_for_test(target, 1);
        let owner = bot.clone_internal();
        let task = tokio::spawn(async move { owner.dig_block(target, BlockFace::Up).await });
        let (id, start) = timeout(Duration::from_secs(1), packets.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(id, 0x1b);
        assert_eq!(start[0], 0);
        let mut drain = bot.connection.ledger().cut(false);
        assert!(drain.inspect().is_none());
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(drain.wait().await.is_err());
        assert!(packets.try_recv().is_err());
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn ledger_dropped_operational_caller_waits_for_abort_on_original_connection() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        {
            let mut receipts = bot.common_receipts.lock().await;
            receipts.inventory.window_id = Some(0);
            receipts.inventory.slots = (0..46)
                .map(|_| {
                    Some(crate::client::received(
                        crate::client::SlotKnowledge::Empty,
                        1,
                    ))
                })
                .collect();
            receipts.selected_hotbar = Some(crate::client::received(0, 1));
        }
        let target = BlockPos { x: 9, y: 65, z: 8 };
        bot.world.lock().await.set_block_for_test(target, 1);
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let owner = client.clone();
        let caller =
            tokio::spawn(
                async move { owner.survival().dig([9, 65, 8], crate::BlockFace::Up).await },
            );
        assert_eq!(
            timeout(Duration::from_secs(1), packets.recv())
                .await
                .unwrap()
                .unwrap()
                .1[0],
            0
        );
        let writer = bot.writer.lock().await;
        let mut drain = bot.connection.ledger().cut(false);
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        tokio::task::yield_now().await;
        assert!(drain.inspect().is_none());
        drop(writer);
        let (id, abort) = timeout(Duration::from_secs(1), packets.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(id, 0x1b);
        assert_eq!(abort[0], 1);
        assert!(
            timeout(Duration::from_secs(1), drain.wait())
                .await
                .unwrap()
                .is_err()
        );
        assert_eq!(
            client.survival().dig_cleanup_record().unwrap().status,
            crate::client::DigCleanupStatus::Aborted
        );
        release.send(()).unwrap();
        drop(client);
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn ledger_native_and_common_share_finite_debt_between_motion_ticks() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        // Native and common clones use the exact same transport ledger.
        bot.swing_arm(Hand::Main).await.unwrap();
        assert_eq!(packets.recv().await.unwrap().0, 0x2b);
        let controls = [crate::client::survival::SurvivalControl {
            yaw: 0.,
            input: Default::default(),
        }; 3];
        let cached = bot
            .start_predicted_path(crate::client::GameMode::Survival, &controls)
            .await
            .unwrap();
        assert_eq!(cached.dispatched_ticks, 0);
        assert_eq!(packets.recv().await.unwrap().0, 0x13);
        let mut drain = bot.connection.ledger().cut(false);
        let idle_writer = bot.writer.lock().await;
        assert!(!bot.common_receipts.lock().await.pending_dispatch);
        assert!(drain.inspect().is_none());
        drop(idle_writer);
        // No public record getter drives completion.
        timeout(Duration::from_secs(2), drain.wait())
            .await
            .unwrap()
            .unwrap();
        for _ in 0..2 {
            assert_eq!(packets.recv().await.unwrap().0, 0x13);
        }
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }
}
