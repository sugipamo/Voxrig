//! Opt-in owned primitive operations. Composite helpers remain separate.
use super::{
    adapter::{CraftingTakeOps, InventoryClickOps, InventorySendOps, InventoryTransferOps},
    owned_operation::{OperationResult, OperationState},
    *,
};
use crate::{Error, Result};
use inventory::{
    InventoryClickButton, InventoryClickRecord, InventoryClickStage, InventorySendRecord,
    InventorySource, InventoryTransferRecord, InventoryTransferStage,
};
use std::{sync::Arc, time::Duration};

#[derive(Debug)]
struct ScopeChanged;
impl std::fmt::Display for ScopeChanged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("owned inventory operation world changed before admission")
    }
}
impl std::error::Error for ScopeChanged {}

tokio::task_local! { static EXPECTED_SESSION: SessionStamp; }
/// Called at the coherent prepare boundary; ordinary APIs have no extra fence.
pub(crate) fn check_expected_session(actual: SessionStamp) -> Result<()> {
    if EXPECTED_SESSION
        .try_with(|expected| *expected != actual)
        .unwrap_or(false)
    {
        return Err(Error::new(
            crate::ErrorKind::State,
            anyhow::Error::new(ScopeChanged),
        ));
    }
    Ok(())
}
fn scope(
    client: &Client,
    session: SessionStamp,
    source: InventorySource,
) -> Result<OperationScope> {
    let key = client.generation_key();
    if key.connection_id() != session.connection_id
        || key.version() != session.version
        || matches!(source, InventorySource::Container { screen } if screen.session() != session)
    {
        return Err(registry::invalid(
            "owned operation belongs to a different connection/world/opening",
        ));
    }
    Ok(OperationScope::Inventory { session, source })
}
// Passive capture can fail or find a new world without invoking any mutation.
// The native prepare boundary rechecks the same scope after this observation.
async fn initial_scope<T>(client: &Client, session: SessionStamp) -> Option<OperationResult<T>> {
    let error = match client.player_state().await {
        Ok(player) if player.session == session => return None,
        Ok(_) => inventory::unavailable("owned operation original world retired before handoff"),
        Err(error) => error,
    };
    Some(OperationResult::error(OperationOutcome::NotSent, error))
}

fn failed<T>(error: Error) -> OperationResult<T> {
    OperationResult::error(
        if error.diagnostic().downcast_ref::<ScopeChanged>().is_some() {
            OperationOutcome::NotSent
        } else {
            OperationOutcome::Unknown(OperationUnknownReason::Failed)
        },
        error,
    )
}
fn superseded<T>() -> OperationResult<T> {
    OperationResult::error(
        OperationOutcome::Unknown(OperationUnknownReason::Superseded),
        inventory::unavailable("original operation record superseded; successor not consumed"),
    )
}
async fn pause() {
    tokio::time::sleep(Duration::from_millis(5)).await;
}

/// Independent operational craft proof alongside the original strict diagnostic.
#[derive(Clone, Debug, serde::Serialize)]
pub struct OwnedCraftingRecord {
    /// Exact executor send; never a newly selected send.
    pub send: InventorySendRecord,
    /// Exact strict take, when still retained.
    pub strict: Option<CraftingTakeRecord>,
    /// Independent Crafted/External/inspection result. Continue explicitly.
    pub operational: Option<OperationalCraftingOutcome>,
}

async fn observe_send(
    client: Client,
    owner: Arc<OperationState<InventorySendRecord>>,
    mut record: InventorySendRecord,
) -> OperationResult<InventorySendRecord> {
    use inventory::InventorySendStatus as S;
    loop {
        owner.retain(record.clone());
        let outcome = match record.status {
            S::NotSent => Some(OperationOutcome::NotSent),
            S::Received | S::Diverged => Some(OperationOutcome::SentReceived),
            S::ResyncRequired => Some(if record.dispatched {
                OperationOutcome::SentUnconfirmed
            } else {
                OperationOutcome::Unknown(OperationUnknownReason::Failed)
            }),
            S::Pending => None,
        };
        if let Some(outcome) = outcome {
            return OperationResult::record(outcome, record);
        }
        pause().await;
        match dispatch!(&client.adapter, a => InventorySendOps::inventory_send_record(a).await) {
            Ok(Some(next)) if next.id == record.id => record = next,
            Ok(_) => return superseded(),
            Err(e) => return failed(e),
        }
    }
}

macro_rules! owned_inventory {
    ($api:ty, $mode:expr) => {
        impl $api {
            /// Begin one acceptance PICKUP and observe its exact native response /
            /// full-state boundary. Strict diagnostics remain separate.
            pub fn begin_send_inventory_click(&self, session: SessionStamp, source: InventorySource,
                slot: u16, button: InventoryClickButton, limits: OperationLimits)
                -> Result<OwnedOperation<InventorySendRecord>> {
                let original = scope(&self.client, session, source)?;
                let client = self.client.clone();
                self.client.begin_owned(original, limits, move |owner| EXPECTED_SESSION.scope(session, async move {
                    if let Some(refusal) = initial_scope(&client, session).await { return refusal; }
                    owner.handoff();
                    match dispatch!(&client.adapter, a => InventorySendOps::send_inventory_click(a, $mode, source, slot, button).await) {
                        Ok(record) => observe_send(client, owner, record).await,
                        Err(e) => failed(e),
                    }
                }))
            }
            /// Begin one acceptance QUICK_MOVE; receipt deadline/resync retains
            /// the original executor gate and never replays the original click.
            pub fn begin_send_inventory_transfer(&self, session: SessionStamp, source: InventorySource,
                slot: u16, limits: OperationLimits) -> Result<OwnedOperation<InventorySendRecord>> {
                let original = scope(&self.client, session, source)?;
                let client = self.client.clone();
                self.client.begin_owned(original, limits, move |owner| EXPECTED_SESSION.scope(session, async move {
                    if let Some(refusal) = initial_scope(&client, session).await { return refusal; }
                    owner.handoff();
                    match dispatch!(&client.adapter, a => InventorySendOps::send_inventory_transfer(a, $mode, source, slot).await) {
                        Ok(record) => observe_send(client, owner, record).await,
                        Err(e) => failed(e),
                    }
                }))
            }
            /// Begin one strict PICKUP, with bounded receipt observation. Requires
            /// the original received session even for the player screen. Drop is
            /// observer-only; deadlines never release the inventory gate.
            pub fn begin_inventory_click(&self, session: SessionStamp, source: InventorySource,
                slot: u16, button: InventoryClickButton, limits: OperationLimits)
                -> Result<OwnedOperation<InventoryClickRecord>> {
                let original = scope(&self.client, session, source)?;
                let client = self.client.clone();
                self.client.begin_owned(original, limits, move |owner| EXPECTED_SESSION.scope(session, async move {
                    if let Some(refusal) = initial_scope(&client, session).await { return refusal; }
                    owner.handoff();
                    let mut record = match dispatch!(&client.adapter, a => InventoryClickOps::click_inventory(a, $mode, source, slot, button).await) {
                        Ok(record) => record, Err(e) => return failed(e),
                    };
                    loop {
                        owner.retain(record.clone());
                        let outcome = match record.stage {
                            InventoryClickStage::NotSent => Some(OperationOutcome::NotSent),
                            InventoryClickStage::ObservedClicked | InventoryClickStage::ObservedUnchanged => Some(OperationOutcome::SentReceived),
                            InventoryClickStage::RequiresInspection => Some(if record.send.dispatched { OperationOutcome::SentUnconfirmed } else { OperationOutcome::Unknown(OperationUnknownReason::Failed) }),
                            InventoryClickStage::Pending => None,
                        };
                        if let Some(outcome) = outcome { return OperationResult::record(outcome, record); }
                        pause().await;
                        match dispatch!(&client.adapter, a => InventoryClickOps::inventory_click_record(a).await) {
                            Ok(Some(next)) if next.id == record.id => record = next,
                            Ok(_) => return superseded(), Err(e) => return failed(e),
                        }
                    }
                }))
            }
            /// Begin one strict QUICK_MOVE. Full writes, exact changed receipts,
            /// inspection and native response debt keep their existing meanings.
            pub fn begin_inventory_transfer(&self, session: SessionStamp, source: InventorySource,
                slot: u16, limits: OperationLimits) -> Result<OwnedOperation<InventoryTransferRecord>> {
                let original = scope(&self.client, session, source)?;
                let client = self.client.clone();
                self.client.begin_owned(original, limits, move |owner| EXPECTED_SESSION.scope(session, async move {
                    if let Some(refusal) = initial_scope(&client, session).await { return refusal; }
                    owner.handoff();
                    let mut record = match dispatch!(&client.adapter, a => InventoryTransferOps::transfer_inventory(a, $mode, source, slot).await) {
                        Ok(record) => record, Err(e) => return failed(e),
                    };
                    loop {
                        owner.retain(record.clone());
                        let outcome = match record.stage {
                            InventoryTransferStage::NotSent => Some(OperationOutcome::NotSent),
                            InventoryTransferStage::ObservedTransferred | InventoryTransferStage::ObservedUnchanged => Some(OperationOutcome::SentReceived),
                            InventoryTransferStage::RequiresInspection => Some(if record.send.dispatched { OperationOutcome::SentUnconfirmed } else { OperationOutcome::Unknown(OperationUnknownReason::Failed) }),
                            InventoryTransferStage::Pending => None,
                        };
                        if let Some(outcome) = outcome { return OperationResult::record(outcome, record); }
                        pause().await;
                        match dispatch!(&client.adapter, a => InventoryTransferOps::inventory_transfer_record(a).await) {
                            Ok(Some(next)) if next.id == record.id => record = next,
                            Ok(_) => return superseded(), Err(e) => return failed(e),
                        }
                    }
                }))
            }
            /// Begin one result PICKUP or QUICK_MOVE using the sealed received grid.
            /// Strict CraftingTakeRecord predicates and timeout inspection persist.
            pub fn begin_crafting_take(&self, grid: ReceivedCrafting,
                destination: CraftingResultDestination, limits: OperationLimits)
                -> Result<OwnedOperation<CraftingTakeRecord>> {
                let session = grid.session();
                scope(&self.client, session, InventorySource::Player)?;
                let original = OperationScope::Crafting { session, source: grid.source() };
                let client = self.client.clone();
                self.client.begin_owned(original, limits, move |owner| async move {
                    if let Some(refusal) = initial_scope(&client, session).await { return refusal; }
                    owner.handoff();
                    let mut record = match dispatch!(&client.adapter, a => CraftingTakeOps::take_crafting_result(a, $mode, &grid, destination).await) {
                        Ok(record) => record, Err(e) => return failed(e),
                    };
                    loop {
                        owner.retain(record.clone());
                        let outcome = match record.stage {
                            CraftingTakeStage::ObservedTaken | CraftingTakeStage::ObservedTransferred | CraftingTakeStage::NotApplied => Some(OperationOutcome::SentReceived),
                            CraftingTakeStage::RequiresInspection => Some(if record.send.dispatched { OperationOutcome::SentUnconfirmed } else { OperationOutcome::Unknown(OperationUnknownReason::Failed) }),
                            CraftingTakeStage::Pending => None,
                        };
                        if let Some(outcome) = outcome { return OperationResult::record(outcome, record); }
                        pause().await;
                        match dispatch!(&client.adapter, a => CraftingTakeOps::crafting_take_record(a).await) {
                            Ok(Some(next)) if next.id == record.id => record = next,
                            Ok(_) => return superseded(), Err(e) => return failed(e),
                        }
                    }
                })
            }
            /// Begin a single operational result take. Crafted/External and strict
            /// inspection are retained independently. This never calls continue.
            pub fn begin_operational_crafting_take(&self, grid: ReceivedCrafting,
                limits: OperationLimits) -> Result<OwnedOperation<OwnedCraftingRecord>> {
                let session = grid.session();
                scope(&self.client, session, InventorySource::Player)?;
                let original = OperationScope::Crafting { session, source: grid.source() };
                let client = self.client.clone();
                self.client.begin_owned(original, limits, move |owner: Arc<OperationState<OwnedCraftingRecord>>| async move {
                    if let Some(refusal) = initial_scope(&client, session).await { return refusal; }
                    owner.handoff();
                    let send = match dispatch!(&client.adapter, a => InventorySendOps::send_crafting_result(a, $mode, &grid, CraftingResultDestination::Cursor, true).await) {
                        Ok(record) => record, Err(e) => return failed(e),
                    };
                    let mut record = OwnedCraftingRecord { send, strict: None, operational: None };
                    loop {
                        // Operational owner selects by exact send ID. A successor
                        // cannot satisfy this observation, even in the same opening.
                        let operational = match client.operational_crafting_outcome(record.send.id).await {
                            Ok(outcome) => outcome, Err(e) => return failed(e),
                        };
                        record.strict = dispatch!(&client.adapter, a => CraftingTakeOps::crafting_take_record(a).await).ok().flatten().filter(|r| r.id == operational.take);
                        let stage = operational.stage;
                        record.operational = Some(operational);
                        owner.retain(record.clone());
                        if stage != OperationalCraftingStage::Pending {
                            return OperationResult::record(if stage == OperationalCraftingStage::Crafted { OperationOutcome::SentReceived } else { OperationOutcome::SentUnconfirmed }, record);
                        }
                        pause().await;
                        match dispatch!(&client.adapter, a => InventorySendOps::inventory_send_record(a).await) {
                            Ok(Some(next)) if next.id == record.send.id => record.send = next,
                            Ok(_) => return superseded(), Err(e) => return failed(e),
                        }
                    }
                })
            }
        }
    }
}
owned_inventory!(Survival, GameMode::Survival);
owned_inventory!(Creative, GameMode::Creative);
