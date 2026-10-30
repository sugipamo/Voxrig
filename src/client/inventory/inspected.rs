//! Explicit replanning authority; historical verdicts are never repaired.
use super::*;
use crate::client::{FieldState, ReceivedPlayerSnapshot, crafting::OperationalCraftingStage};

/// Why inspection recovery did not release an owner. No packet is sent.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InspectedRecoveryRejection {
    /// Original ID no longer identifies the retained claimed owner.
    UnknownOwner,
    /// Owner is pending, successfully crafted, or already released.
    NotInspectionTerminal,
    /// Connection/world/opening differs from the original or current scope.
    ScopeMismatch,
    /// Full contents/cursor are missing, incomplete or older than the send.
    IncompleteOrOldBasis,
    /// Inventory receipts changed after the supplied capture.
    StaleBasis,
    /// A send, native write, own action or response/state debt remains.
    PendingDebt,
    /// Adapter gate/state access did not finish within the five-second budget.
    RecoveryDeadline,
}
/// Separate recovery receipt. Retain it before another operation replaces history.
#[derive(Clone, Debug, serde::Serialize)]
pub struct InspectedCraftingRecovery {
    /// Unmodified failure verdict, including External and first inspection reason.
    pub original: crate::client::crafting::OperationalCraftingOutcome,
    /// Original send record, retained even across an explicit no-effect resync.
    pub original_send: InventorySendRecord,
    /// Original divergence, including received causes, retained across resync.
    pub original_divergence: Option<InventoryDivergence>,
    /// Original strict diagnostic record at recovery; its meaning is unchanged.
    pub original_take: crate::client::crafting::CraftingTakeRecord,
    /// Received current inventory for replanning, including the occupied cursor.
    pub basis: InventorySnapshot,
    /// Supplied immutable snapshot's capture boundary.
    pub basis_sequence: u64,
    /// Actual full packet used for release.
    pub contents_sequence: u64,
    /// Actual carried-stack receipt used for release.
    pub cursor_sequence: u64,
}
/// Typed recovery result, separate from transport/state access errors.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum InspectedCraftingRecoveryResult {
    /// This owner's gate was released; no old intent was replayed.
    Released {
        /// Original verdict and independent release evidence.
        recovery: Box<InspectedCraftingRecovery>,
    },
    /// Gate and verdict were left intact.
    Rejected {
        /// Failed admission predicate.
        reason: InspectedRecoveryRejection,
    },
}
fn ordinal<T>(value: &FieldState<T>) -> Option<u64> {
    match value {
        FieldState::Received(v) => Some(v.receipt().ordinal()),
        _ => None,
    }
}
fn full(snapshot: &ReceivedPlayerSnapshot, source: InventorySource) -> Option<u64> {
    match source {
        InventorySource::Player => ordinal(snapshot.inventory().last_full_contents()),
        InventorySource::Container { screen } => snapshot
            .container()
            .filter(|c| c.id() == screen)
            .and_then(|c| ordinal(c.last_full_contents())),
    }
}
impl executor::Executor {
    pub(crate) fn inspected_crafting_recovery(
        &self,
        id: InventorySendId,
    ) -> Option<InspectedCraftingRecovery> {
        self.inspected
            .as_ref()
            .filter(|r| r.original.send == id)
            .map(|r| (**r).clone())
    }

    /// Caller holds the adapter's receive/write gate and has checked liveness.
    pub(crate) fn recover_inspected_crafting(
        &mut self,
        id: InventorySendId,
        basis: &ReceivedPlayerSnapshot,
        live: &ReceivedPlayerSnapshot,
        current: InventorySnapshot,
        take: Option<crate::client::crafting::CraftingTakeRecord>,
        native_pending: bool,
    ) -> InspectedCraftingRecoveryResult {
        use InspectedRecoveryRejection as R;
        let rejected = |reason| InspectedCraftingRecoveryResult::Rejected { reason };
        self.expire();
        if id.session() != current.session
            || id.source() != current.source
            || basis.boundary().session() != current.session
        {
            return rejected(R::ScopeMismatch);
        }
        let Some(owner) = self
            .operational
            .as_ref()
            .filter(|o| o.claimed && o.outcome.send == id)
        else {
            return rejected(R::UnknownOwner);
        };
        if owner.scope_retired {
            return rejected(R::ScopeMismatch);
        }
        if owner.released || owner.outcome.stage != OperationalCraftingStage::RequiresInspection {
            return rejected(R::NotInspectionTerminal);
        }
        let Some(original_send) = self
            .current
            .as_ref()
            .filter(|r| r.id == id)
            .or_else(|| self.inspection_send.as_ref().filter(|r| r.id == id))
            .cloned()
        else {
            return rejected(R::UnknownOwner);
        };
        let Some(take) = take.filter(|r| r.id == owner.outcome.take) else {
            return rejected(R::UnknownOwner);
        };
        // A full packet is not an ACK/fence. Even a timed-out response debt must
        // first be discharged by the existing ordered no-effect resync protocol.
        if native_pending
            || take.stage == crate::client::CraftingTakeStage::Pending
            || self.busy()
            || self.own_action_pending()
            || self.current.as_ref().is_none_or(|r| {
                r.id.session() != current.session
                    || r.id.source() != current.source
                    || !matches!(
                        r.status,
                        InventorySendStatus::Received | InventorySendStatus::Diverged
                    )
            })
        {
            return rejected(R::PendingDebt);
        }
        let basis_source =
            basis
                .container()
                .map_or(InventorySource::Player, |c| InventorySource::Container {
                    screen: c.id(),
                });
        if basis_source != current.source {
            return rejected(R::ScopeMismatch);
        }
        let Some(contents) = full(basis, current.source) else {
            return rejected(R::IncompleteOrOldBasis);
        };
        let Some(cursor) = ordinal(basis.inventory().cursor()) else {
            return rejected(R::IncompleteOrOldBasis);
        };
        let completion = self.current.as_ref().expect("checked current");
        if contents <= original_send.completion_after()
            || cursor < contents
            || basis.boundary().ordinal() < cursor
            || completion.contents_sequence.is_none_or(|s| contents < s)
            || completion.cursor_sequence.is_none_or(|s| cursor < s)
        {
            return rejected(R::IncompleteOrOldBasis);
        }
        // An actual full packet establishes coverage; supported later slot
        // receipts may refine it. Every constituent must still be current.
        let slots: Vec<_> = match current.source {
            InventorySource::Player => (0..46)
                .map(|i| {
                    (
                        basis.inventory().slot(i).ok(),
                        live.inventory().slot(i).ok(),
                    )
                })
                .collect(),
            InventorySource::Container { .. } => match (basis.container(), live.container()) {
                (Some(a), Some(b))
                    if a.contents_coverage() == crate::client::Coverage::Complete
                        && a.layout().is_some_and(|l| {
                            l.total_slots == b.layout().map_or(0, |l| l.total_slots)
                        }) =>
                {
                    (0..a.layout().expect("checked layout").total_slots)
                        .map(|i| (a.slot(i).ok(), b.slot(i).ok()))
                        .collect()
                }
                _ => return rejected(R::IncompleteOrOldBasis),
            },
        };
        if slots.iter().any(|(a, _)| {
            a.and_then(ordinal)
                .is_none_or(|s| s < contents || s > basis.boundary().ordinal())
        }) {
            return rejected(R::IncompleteOrOldBasis);
        }
        if full(live, current.source) != Some(contents)
            || ordinal(live.inventory().cursor()) != Some(cursor)
            || slots
                .iter()
                .any(|(a, b)| a.and_then(ordinal) != b.and_then(ordinal))
            || (0..46).any(|i| {
                basis.inventory().slot(i).ok().and_then(ordinal)
                    != live.inventory().slot(i).ok().and_then(ordinal)
            })
        {
            return rejected(R::StaleBasis);
        }
        let recovery = InspectedCraftingRecovery {
            original: owner.outcome.clone(),
            original_send,
            original_divergence: self
                .divergence
                .as_ref()
                .filter(|d| d.send == id)
                .or_else(|| self.inspection_divergence.as_ref().filter(|d| d.send == id))
                .cloned(),
            original_take: take,
            basis: current,
            basis_sequence: basis.boundary().ordinal(),
            contents_sequence: contents,
            cursor_sequence: cursor,
        };
        // Release only local admission. Neither original verdict nor received
        // state is rewritten, and no transport/debt authority is discarded.
        self.operational.as_mut().expect("checked owner").released = true;
        self.inspected = Some(Box::new(recovery.clone()));
        InspectedCraftingRecoveryResult::Released {
            recovery: Box::new(recovery),
        }
    }
}
impl crate::Client {
    /// Read the last recovery receipt for this ID, including after a later write.
    /// One recovery is retained per connection; callers should retain their copy.
    pub async fn inspected_crafting_recovery(
        &self,
        original_id: InventorySendId,
    ) -> Result<Option<InspectedCraftingRecovery>> {
        crate::client::dispatch!(&self.adapter, a => crate::client::adapter::EventOps::inspected_crafting_recovery(a, original_id).await)
    }

    /// Release a claimed terminal inspection owner using immutable received full
    /// contents + cursor from `received_player_snapshot()`. Same scope, newer
    /// than the original send, still-current receipts and zero pending debt are
    /// required. Explicit `resync_inventory()` may establish the boundary first.
    /// Replan from the returned basis; External/Diverged never become success.
    /// Strict diagnostic gates remain unchanged. Sends no packets. Gate/state
    /// access has a five-second total budget; expiry returns RecoveryDeadline.
    pub async fn recover_inspected_operational_crafting(
        &self,
        original_id: InventorySendId,
        basis: &ReceivedPlayerSnapshot,
    ) -> Result<InspectedCraftingRecoveryResult> {
        match tokio::time::timeout(std::time::Duration::from_secs(5), async {
            crate::client::dispatch!(&self.adapter, a => crate::client::adapter::EventOps::recover_inspected_crafting(a, original_id, basis).await)
        }).await {
            Ok(result) => result,
            Err(_) => Ok(InspectedCraftingRecoveryResult::Rejected {
                reason: InspectedRecoveryRejection::RecoveryDeadline,
            }),
        }
    }
}
