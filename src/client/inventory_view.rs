//! Passive inventory constituents from one native receive boundary.
use super::{PlayerObservation, container::ScreenObservation, inventory::InventorySnapshot};

/// Immutable player and screen observations captured under one adapter receive lock.
/// Values keep their original provenance; the boundary is not a server tick,
/// freshness guarantee, successful craft, or permission to send another operation.
#[derive(Clone, Debug, serde::Serialize)]
pub struct InventoryViewCapture {
    player: PlayerObservation,
    screen: ScreenObservation,
}
impl InventoryViewCapture {
    /// Player state, including canonical inventory and local player-screen access.
    pub fn player(&self) -> &PlayerObservation {
        &self.player
    }
    /// Screen state at exactly the player's session and receive sequence.
    /// Use `active_screen()` to exclude a locally closed or mismatched opening.
    pub fn screen(&self) -> &ScreenObservation {
        &self.screen
    }
    /// Received comparison basis using the same active-screen rule as history.
    pub fn snapshot(&self) -> InventorySnapshot {
        InventorySnapshot::capture(&self.player, self.screen.screen.clone())
    }
}
impl crate::Client {
    /// Capture player/inventory and screen under one native receive lock.
    /// Sends no packets, admits no writes and never reconciles work. Other tasks
    /// may send while waiting. Missing fields remain unknown and local submitted
    /// or predicted fields retain their provenance. No retries or server round trip.
    /// The entire read has a five-second budget, including lock acquisition;
    /// expiry returns `ErrorKind::Timeout` without a partial capture.
    pub async fn capture_inventory_view(&self) -> crate::Result<InventoryViewCapture> {
        use super::adapter::CoreOps;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let (player, screen, _) =
                super::dispatch!(&self.adapter, a => CoreOps::inventory_move_capture(a).await)?;
            Ok(InventoryViewCapture { player, screen })
        })
        .await
        .map_err(|_| {
            crate::Error::new(
                crate::ErrorKind::Timeout,
                anyhow::anyhow!("inventory view capture deadline"),
            )
        })?
    }
}
