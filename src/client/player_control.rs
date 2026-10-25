//! Basic player inputs with an explicit expected received game mode.
use super::{DispatchReceipt, GameMode, adapter::ControlOps, operations::Action};
use crate::{Client, Result};

/// View, held-slot and ground movement inputs. A handle neither changes nor grants game mode.
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
    /// Own an Elytra start request with explicit first-tick aim/keys. Actual usable
    /// received chest equipment and an airborne model are required. The connection
    /// waits for the actual fall-flying flag before predicting or writing movement;
    /// complete request dispatch does not mean the server accepted glide.
    /// Stop releases ownership/keys only; pitch down and native landing end glide.
    pub async fn start_glide_control(
        &self,
        controls: super::control::Controls,
    ) -> Result<super::control::ControlRecord> {
        crate::client::dispatch!(&self.client.adapter, a => ControlOps::start_glide_control(a, self.mode, controls).await)
    }

    /// Start held Creative/Spectator flight with explicit first-tick keys.
    /// Creative flight must first be enabled by `Creative::set_flying(true)`;
    /// Spectator requires original flying abilities. Permission and speed must
    /// be received in this world. This never grants flight or changes game mode.
    /// Creative model landing ends this owner; ground continuation is explicit.
    pub async fn start_flight_control(
        &self,
        controls: super::control::Controls,
    ) -> Result<super::control::ControlRecord> {
        crate::client::dispatch!(&self.client.adapter, a => ControlOps::start_flight_control(a, self.mode, controls).await)
    }
    /// Start held ground movement in received Survival, Adventure or Creative.
    /// Flying, Spectator, riding and unmodeled geometry remain unsupported.
    /// Mode changes stop the owner; a handle never changes mode or enables flight.
    /// Released keys and current local aim seed the new connection-owned session.
    pub async fn start_control(&self) -> Result<super::control::ControlRecord> {
        crate::client::dispatch!(&self.client.adapter, a => ControlOps::start_control(a, self.mode).await)
    }

    /// Start ground movement with best-effort remote entity prediction.
    /// If contact admission, refresh or prediction fails, retain its diagnostic,
    /// discard the entire remote contact model for this owner and predict using
    /// local terrain only. World, owner, own-player and mode checks still apply.
    /// Passenger/relationship history gaps, scope re-entry/reselection and
    /// sweep overruns are warnings through `contact_diagnostic_for`, never a
    /// request to replace this owner. Corrections re-anchor on the received pose
    /// and discard any remote numerical state tied to the previous origin.
    /// Dispatch is not acceptance: inspect corrections or use `body::Move` on
    /// Java 1.21.11 for bounded acceptance. `start_control` remains strict.
    pub async fn start_control_best_effort(&self) -> Result<super::control::ControlRecord> {
        crate::client::dispatch!(&self.client.adapter, a => ControlOps::start_control_best_effort(a, self.mode).await)
    }

    /// Frozen cause of omitted contacts for the selected owner, including after
    /// it stops. None means no contact failure was omitted. A replaced ID refuses.
    /// This diagnostic does not establish server acceptance or invalidate control.
    /// `MovementRefusal::RemoteStateDropped` explicitly marks the warning; its
    /// inner cause's owner state and replacement advice concern the strict
    /// contact model. Read `ControlRecord.status` for the control owner's state.
    pub async fn contact_diagnostic_for(
        &self,
        session_id: u64,
    ) -> Result<Option<crate::MovementRefusal>> {
        crate::client::dispatch!(&self.client.adapter, a => ControlOps::contact_diagnostic_for(a, session_id).await)
    }

    /// Replace keys on the selected running owner in this handle's exact mode.
    /// A stale ID, different mode or world refuses without mutating another owner.
    pub async fn set_controls_for(
        &self,
        session_id: u64,
        controls: super::control::Controls,
    ) -> Result<super::control::ControlRecord> {
        crate::client::dispatch!(&self.client.adapter, a => ControlOps::set_controls(a, self.mode, Some(session_id), controls).await)
    }

    /// Queue one ground jump without changing held keys or automatically retrying.
    /// This is local admission; the retained outcome distinguishes model and dispatch.
    pub async fn request_ground_jump(
        &self,
        session_id: u64,
    ) -> Result<super::control::GroundJumpRequest> {
        crate::client::dispatch!(&self.client.adapter, a => ControlOps::request_ground_jump(a, self.mode, session_id).await)
    }

    /// Release only the selected owner in this handle's mode. The connection owns
    /// admitted release writes even if the waiter cancels. This is not physical rest.
    /// Matching stopped records remain readable after mode or world changes.
    pub async fn stop_control_for(&self, session_id: u64) -> Result<super::control::ControlRecord> {
        crate::client::dispatch!(&self.client.adapter, a => ControlOps::stop_control_in_mode(a, self.mode, Some(session_id)).await)?
            .ok_or_else(|| super::registry::invalid("selected control session is missing or was replaced"))
    }

    /// Inspect the connection's latest owner, including its original admitted mode.
    /// Inspection neither acquires inputs nor changes a retained failure.
    pub async fn control_record(&self) -> Result<Option<super::control::ControlRecord>> {
        crate::client::dispatch!(&self.client.adapter, a => ControlOps::control_record(a).await)
    }

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
