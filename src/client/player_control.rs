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
    /// Begin stopping this exact controller owner. The handle owns bounded
    /// observation; dropping it does not cancel release. Successful local stop
    /// returns SentUnconfirmed when release writes completed, or NotSent for
    /// an original no-op stop, with its ControlRecord; never a receipt
    /// of physical rest. Deadline returns Unknown and preserves the native owner.
    pub fn begin_stop_for(
        &self,
        session_id: u64,
        limits: super::OperationLimits,
    ) -> Result<super::OwnedOperation<super::control::ControlRecord>> {
        use super::{OperationOutcome, OperationScope, owned_operation::OperationResult};
        let owner = self.clone();
        self.client.begin_owned(
            OperationScope::Control { session_id },
            limits,
            move |state| async move {
                use super::owned_operation::STOP_OBSERVER;
                let observation =
                    std::sync::Arc::new(std::sync::Mutex::new(OperationOutcome::NotSent));
                state.handoff();
                let result = STOP_OBSERVER
                    .scope(observation.clone(), owner.stop_control_for(session_id))
                    .await;
                let outcome = *observation.lock().unwrap_or_else(|p| p.into_inner());
                match result {
                    Ok(record) => OperationResult::record(outcome, record),
                    Err(error) => OperationResult::error(outcome, error),
                }
            },
        )
    }

    /// Read this connection's control owner transitions (Started, Paused,
    /// Resumed, Stopped with a typed cause, Expired) after a log cursor
    /// (initially 0), on both versions, without re-querying `control_record`.
    /// State reports eviction; InvalidInput a future cursor. Policies are unchanged.
    pub async fn control_state_changes_after(
        &self,
        after_sequence: u64,
    ) -> Result<super::control::ControlStateLog> {
        self.client
            .control_state_changes_after(after_sequence)
            .await
    }

    /// Read Java 1.16.1 remote-model degradations after a connection-local log
    /// cursor (initially 0), including replaced owners and after disconnect.
    /// State reports eviction; InvalidInput reports a future cursor.
    /// Java 1.21.11 returns Unsupported. Does not send or grant acceptance.
    pub async fn control_changes_after(
        &self,
        after_sequence: u64,
    ) -> Result<super::control::ControlChangeLog> {
        self.client.control_changes_after(after_sequence).await
    }

    /// Read received own-pose corrections over this connection's control owners
    /// after a log cursor (initially 0), on both versions. Every packet is kept,
    /// including several between two ticks; `ControlRecord.corrections` stays the
    /// applied count. State reports eviction; InvalidInput a future cursor.
    /// A record is a superseded submission, never a teleport/rejection cause, and
    /// the owner keeps Running.
    pub async fn control_corrections_after(
        &self,
        after_sequence: u64,
    ) -> Result<super::control::ControlCorrectionLog> {
        self.client.control_corrections_after(after_sequence).await
    }

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

    /// Start ground movement with best-effort entity and terrain prediction.
    /// If contact admission, refresh or prediction fails, retain its diagnostic,
    /// discard the entire remote contact model for this owner and predict using
    /// local terrain only. Unavailable terrain uses coordinate probes of at most
    /// 0.1 blocks horizontally and 0.1 vertically per tick, for at most 20 ticks
    /// without a new received pose or a usable local terrain tick. At the bound,
    /// the same owner pauses and retries terrain; it does not invent support.
    /// With no vertical momentum, probes keep their height. Speculative probes
    /// never claim on-ground or apply jumps. See `walking_diagnostics_for`.
    /// World, owner, own-player and mode checks still apply.
    /// On Java 1.16.1, the ordinary returned record carries `degradation`; later
    /// withdrawals are retained by `control_changes_after`.
    /// Passenger/relationship history gaps, scope re-entry/reselection and
    /// sweep overruns are warnings through `contact_diagnostic_for`, never a
    /// request to replace this owner. Corrections re-anchor on the received pose
    /// and discard any remote numerical state tied to the previous origin.
    /// Dispatch is not acceptance: inspect corrections or use `body::Move` on
    /// both versions for bounded acceptance. `start_control` remains strict.
    pub async fn start_control_best_effort(&self) -> Result<super::control::ControlRecord> {
        crate::client::dispatch!(&self.client.adapter, a => ControlOps::start_control_best_effort(a, self.mode).await)
    }

    /// Walk 1..120 route inputs using the best-effort ground owner, without
    /// exact-zero velocity, full floor support or a settled endpoint requirement.
    /// Each input is written once at 50 ms spacing. The connection owns the run
    /// after admission, even if the caller drops its future. Corrections re-anchor
    /// the same owner and remaining inputs; they never replay earlier inputs.
    /// Input replacement refuses while this finite path owns the schedule.
    /// Completion releases keys; inspect `walking_diagnostics_for` and corrections.
    /// Dispatch is not acceptance. A bounded evidence deadline stops a stalled
    /// route; the strict `Survival::start_predicted_path` remains diagnostic.
    pub async fn start_path_best_effort(
        &self,
        controls: &[super::survival::SurvivalControl],
    ) -> Result<super::control::ControlRecord> {
        super::survival::model::validate_controls(controls)?;
        crate::client::dispatch!(&self.client.adapter, a => ControlOps::start_path_best_effort(a, self.mode, controls).await)
    }

    /// Terrain warnings and finite route progress for this exact owner, including
    /// after stopping. A stale ID refuses. Reading diagnostics sends no packets,
    /// replenishes no speculative walking budget and grants no acceptance.
    pub async fn walking_diagnostics_for(
        &self,
        session_id: u64,
    ) -> Result<super::control::WalkingDiagnostics> {
        crate::client::dispatch!(&self.client.adapter, a => ControlOps::walking_diagnostics_for(a, session_id).await)
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
