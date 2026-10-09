//! Basic player inputs with an explicit expected received game mode.
use super::{DispatchReceipt, GameMode, operations::Action};
use crate::{Client, Result};

/// View and held-slot inputs. A handle neither changes nor grants game mode.
/// The adapter rechecks the expected mode and existing mutation admission under
/// its state boundary before sending. A retained handle refuses a different mode.
#[derive(Clone)]
pub struct PlayerControl {
    client: Client,
    mode: GameMode,
}

impl Client {
    /// Basic inputs for an expected received mode, including Adventure/Spectator.
    /// Look supports all four modes with native admission. Hotbar selection
    /// supports Survival/Creative/Adventure; Spectator returns `Unsupported`
    /// without a packet because vanilla ignores held-item selection in that mode.
    ///
    /// ```no_run
    /// use voxrig::client::prelude::*;
    /// async fn aim(client: &Client) -> Result<()> {
    ///     let mode = client.player_state().await?.game_mode
    ///         .ok_or_else(|| Error::new(ErrorKind::State, anyhow::anyhow!("mode unavailable")))?;
    ///     client.player_control(mode).look([45.0, 0.0]).await?;
    ///     Ok(())
    /// }
    /// ```
    pub fn player_control(&self, mode: GameMode) -> PlayerControl {
        PlayerControl {
            client: self.clone(),
            mode,
        }
    }
}

impl PlayerControl {
    /// Submit finite native angles (pitch -90..90), retaining rotation provenance.
    /// Modern Survival/Adventure require supported stationary standing geometry;
    /// unavailable geometry or unresolved motion refuses before sending. Complete
    /// dispatch is not a server acknowledgement or acceptance of the view.
    pub async fn look(&self, rotation: [f32; 2]) -> Result<DispatchReceipt> {
        self.client.execute(self.mode, Action::Look(rotation)).await
    }

    /// Select index 0..8 in Survival/Creative/Adventure. Spectator is unsupported.
    /// Success records submitted selection, not a received inventory/item echo.
    /// Existing pending-dispatch, quarantine and operation admission still apply.
    pub async fn select_hotbar(&self, slot: u8) -> Result<DispatchReceipt> {
        self.client
            .execute(self.mode, Action::SelectHotbar(slot))
            .await
    }
}
