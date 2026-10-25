//! Bounded, additive outcome view over a strict PICKUP receipt.
use crate::client::{
    Client, ValueSource,
    adapter::{CoreOps, InventoryClickOps, dispatch},
    inventory::{InventoryClickId, InventoryClickRecord, InventoryClickSource},
};
use crate::{Error, ErrorKind, Result};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::Mutex,
    time::{Instant, timeout},
};

/// Outcome of a receipt watch. Unknown does not authorize replay or clear the
/// original receipt. A fresh contradictory result can later change Unknown to
/// Corrected; Accepted/Corrected are retained historical outcomes.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ReceiptStatus {
    /// Still awaiting actual source/cursor receipts.
    Pending,
    /// Complete write and fresh matching source/cursor (plus legacy transaction).
    Accepted,
    /// Actual source/cursor contradicted the predicted result.
    Corrected,
    /// Deadline, lost original identity, unavailable data or uncertain delivery.
    Unknown,
}
struct State {
    status: ReceiptStatus,
    record: InventoryClickRecord,
}
/// Independent bounded view of one original inventory click. Clones share the
/// deadline and result. Reads never send the click again or clear any gate.
#[derive(Clone)]
pub struct InventoryClickWatch {
    client: Client,
    state: Arc<Mutex<State>>,
    deadline: Instant,
    read_budget: Duration,
    uncertain: Arc<AtomicBool>,
}
impl InventoryClickWatch {
    pub(crate) async fn new(
        client: Client,
        id: InventoryClickId,
        budget: Duration,
        read_budget: Duration,
    ) -> Result<Self> {
        let deadline = Instant::now() + budget;
        let record = timeout(budget, async {
            dispatch!(&client.adapter, a => InventoryClickOps::inventory_click_record(a).await)
        })
        .await
        .map_err(|_| {
            Error::new(
                ErrorKind::Timeout,
                anyhow::anyhow!("receipt watch capture deadline"),
            )
        })??
        .filter(|r| r.id == id)
        .ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidInput,
                anyhow::anyhow!("foreign, expired or superseded inventory click"),
            )
        })?;
        Ok(Self {
            client,
            state: Arc::new(Mutex::new(State {
                status: ReceiptStatus::Pending,
                record,
            })),
            deadline,
            read_budget,
            uncertain: Arc::new(AtomicBool::new(false)),
        })
    }
    /// Inspect once, bounded by the original deadline while Pending and by the
    /// settle window afterward. After Unknown, fresh contradictory receipts for
    /// the same retained intent still report Corrected. Late agreement remains
    /// Unknown. No automatic replay, cursor return, rollback or gate release.
    pub async fn status(&self) -> ReceiptStatus {
        let read = async {
            let mut state = self.state.lock().await;
            if matches!(
                state.status,
                ReceiptStatus::Accepted | ReceiptStatus::Corrected
            ) {
                return Ok(state.status.clone());
            }
            if Instant::now() >= self.deadline || self.uncertain.load(Ordering::Acquire) {
                state.status = ReceiptStatus::Unknown;
            }
            let record = dispatch!(&self.client.adapter, a => InventoryClickOps::inventory_click_record(a).await)?;
            let Some(record) = record.filter(|r| r.id == state.record.id) else {
                state.status = ReceiptStatus::Unknown;
                return Ok(state.status.clone());
            };
            let (player, screen, registries) =
                dispatch!(&self.client.adapter, a => CoreOps::inventory_move_capture(a).await)?;
            if player.session != record.initial.session || player.game_mode != Some(record.mode) {
                state.status = ReceiptStatus::Unknown;
                return Ok(state.status.clone());
            }
            let slots = match record.source {
                InventoryClickSource::Player
                    if player.inventory.window_id == Some(0)
                        && player.inventory.player_screen
                            == record.initial.inventory.player_screen =>
                {
                    &player.inventory.slots
                }
                InventoryClickSource::Container { screen: id } => {
                    let Some(screen) = screen.screen.as_ref().filter(|s| {
                        s.id == id && player.inventory.window_id == Some(id.window_id())
                    }) else {
                        state.status = ReceiptStatus::Unknown;
                        return Ok(state.status.clone());
                    };
                    &screen.slots
                }
                _ => {
                    state.status = ReceiptStatus::Unknown;
                    return Ok(state.status.clone());
                }
            };
            let values = [
                slots
                    .get(usize::from(record.source_slot))
                    .and_then(Option::as_ref),
                player.inventory.cursor.as_ref(),
            ];
            let mut fresh = true;
            let mut agrees = true;
            let mut contradiction = false;
            for (value, before, after, observed) in [
                (
                    values[0],
                    &record.source_before,
                    &record.prediction.source.value,
                    record.source_receipt.is_some(),
                ),
                (
                    values[1],
                    &record.cursor_before,
                    &record.prediction.cursor.value,
                    record.cursor_receipt.is_some(),
                ),
            ] {
                let Some(value) = value.filter(|v| matches!(v.source, ValueSource::Received { sequence } if sequence > record.send.after_sequence)) else { fresh = false; continue; };
                let (matches_before, matches_after) = if let Some(context) = &record.item_context {
                    context.classify(value, before, after, &registries)?
                } else {
                    (value.value == before.value, value.value == *after)
                };
                agrees &= matches_after;
                contradiction |= !matches_after && (!matches_before || observed);
            }
            if Instant::now() >= self.deadline {
                state.status = ReceiptStatus::Unknown;
            }
            let reply = player.session.version == crate::MinecraftVersion::Java1_21_11
                || record.legacy_reply.is_some();
            if record.send.dispatched && contradiction {
                state.status = ReceiptStatus::Corrected;
            }
            if record.send.dispatched && fresh && reply {
                if !agrees {
                    state.status = ReceiptStatus::Corrected;
                } else if state.status == ReceiptStatus::Pending {
                    state.status = ReceiptStatus::Accepted;
                }
            }
            state.record = record;
            Ok::<_, Error>(state.status.clone())
        };
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        let budget = if remaining.is_zero() {
            self.read_budget
        } else {
            remaining.min(self.read_budget)
        };
        match timeout(budget, read).await {
            Ok(Ok(status)) => status,
            _ => {
                self.uncertain.store(true, Ordering::Release);
                ReceiptStatus::Unknown
            }
        }
    }
}
