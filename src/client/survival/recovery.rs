//! Explicit fresh connection recovery; old mining never gains continuation.
use super::{MiningId, MiningRecord, MotionPreview, SurvivalControl};
use crate::client::{
    ConnectionIdentity, GameMode, PlayerObservation, SlotKnowledge, Survival, ValueSource,
};
use crate::connection::Adapter;
use crate::{Client, ConnectionConfig, NativeBlockState, Result};

fn received_baselines(player: &PlayerObservation) -> bool {
    player.received_pose.is_some()
        && player.dimension.is_some()
        && player.health.is_some()
        && player.inventory.window_id == Some(0)
        && player.inventory.slots.len() == 46
        && player.inventory.cursor.as_ref().is_some_and(|c| {
            matches!(c.source, ValueSource::Received { .. }) && c.value == SlotKnowledge::Empty
        })
        && player.inventory.slots.iter().all(|s| {
            s.as_ref().is_some_and(|s| {
                matches!(s.source, ValueSource::Received { .. })
                    && s.value != SlotKnowledge::Unavailable
            })
        })
}

/// Native retirement method, distinct from a block receipt or action ACK.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MiningRecoveryMethod {
    /// Independently received exact profile removal.
    IndependentRemoval,
    /// Same-profile fresh login using audited direct vanilla lifecycle semantics.
    SameProfileLogin,
}
/// Caller-declared fresh target condition; recovery never edits the world.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "state", rename_all = "snake_case")]
pub enum MiningRecoveryTarget {
    /// Require the original baseline or an exact ordinary air state.
    Exact(NativeBlockState),
    /// Admit the original baseline or ordinary air for caller reconciliation.
    OriginalOrAir,
}
impl MiningRecoveryTarget {
    fn air(state: &NativeBlockState) -> bool {
        state.properties.is_empty()
            && matches!(
                state.name.as_str(),
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            )
    }
    pub(crate) fn validate_baseline(&self, baseline: &NativeBlockState) -> Result<()> {
        if let Self::Exact(state) = self {
            if state != baseline && !Self::air(state) {
                return Err(super::mining::unavailable(
                    "recovery requires air or the original supported baseline",
                ));
            }
        }
        Ok(())
    }
    pub(crate) fn allows_baseline(
        &self,
        state: &NativeBlockState,
        baseline: &NativeBlockState,
    ) -> bool {
        match self {
            Self::Exact(expected) => state == expected,
            Self::OriginalOrAir => state == baseline || Self::air(state),
        }
    }
}
/// Source's before-I/O claim. Failure/cancellation never permits another login.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MiningRecoveryAttempt {
    /// Audited retirement semantics.
    pub method: MiningRecoveryMethod,
    /// Required newly received target.
    pub target: MiningRecoveryTarget,
}

#[derive(Clone)]
enum Watch {
    Legacy,
    Modern(Box<crate::versions::java_1_21_11::operations::MiningProfileRecoveryWatch>),
}
/// Original live source/attempt binding. Clones share the source's login claim.
/// This handle has no Deserialize; a saved history cannot recreate it.
#[derive(Clone)]
pub struct MiningProfileRecovery {
    source: Client,
    original: MiningRecord,
    identity: ConnectionIdentity,
    watch: Watch,
}
impl Survival {
    /// Bind one original mining attempt without closing or reconnecting.
    /// Requires a direct, unmodified vanilla endpoint and exclusive ownership
    /// of this offline profile. Proxy/plugin/custom lifecycle is not audited.
    pub async fn prepare_mining_profile_recovery(
        &self,
        id: MiningId,
    ) -> Result<MiningProfileRecovery> {
        let original = self
            .mining_record()
            .await?
            .filter(|m| m.id == id && m.recovery_attempt.is_none())
            .ok_or_else(|| {
                super::mining::unavailable(
                    "recovery belongs to another or already claimed mining attempt",
                )
            })?;
        let identity = self.client.connection_identity().await?;
        if identity.session != id.session() {
            return Err(super::mining::unavailable("mining world/profile changed"));
        }
        let watch = match &self.client.adapter {
            Adapter::Java1_16_1(_) => Watch::Legacy,
            Adapter::Java1_21_11(bot) => Watch::Modern(Box::new(
                bot.operations().common_profile_recovery_watch(id).await?,
            )),
        };
        Ok(MiningProfileRecovery {
            source: self.client.clone(),
            original,
            identity,
            watch,
        })
    }
}
impl MiningProfileRecovery {
    /// Explicitly close the original source; this alone does not prove retirement.
    pub async fn close_source(&self) -> Result<()> {
        self.source.disconnect().await
    }
    /// Original retained record, including the once-only recovery claim.
    pub async fn source_record(&self) -> Result<MiningRecord> {
        self.source
            .survival()
            .mining_record()
            .await?
            .filter(|m| m.id == self.original.id)
            .ok_or_else(|| super::mining::unavailable("original mining record unavailable"))
    }
    /// Reconnect once after source closure. New join/player/site/inventory
    /// admission is required, including on 1.16.1 where LOGIN_SUCCESS precedes
    /// old-profile retirement. Never imports old history or replays commands.
    pub async fn reconnect(
        &self,
        config: ConnectionConfig,
        target: MiningRecoveryTarget,
    ) -> Result<RecoveredSurvivalClient> {
        config.validate()?;
        if config.version != self.source.version() || config.username != self.identity.name {
            return Err(super::mining::unavailable(
                "recovery requires original version/profile",
            ));
        }
        target.validate_baseline(&self.original.baseline)?;
        let ready_timeout = config.limits.ready_timeout;
        let client = match (&self.source.adapter, &self.watch) {
            (Adapter::Java1_16_1(bot), Watch::Legacy) => {
                bot.common_claim_profile_recovery(self.original.id, &config, target.clone())
                    .await?;
                Client::connect(config).await?
            }
            (Adapter::Java1_21_11(bot), Watch::Modern(watch)) => bot
                .operations()
                .reconnect_survival_mining_profile(watch, config, target.clone())
                .await?
                .client(),
            _ => {
                return Err(super::mining::unavailable(
                    "recovery adapter binding changed",
                ));
            }
        };
        // Any failed or cancelled admission closes the fresh connection. The
        // retained source claim still prevents an unintentional second login.
        let admission = tokio::time::timeout(ready_timeout, self.admit(&client, &target)).await;
        let evidence = match admission {
            Ok(Ok(evidence)) => evidence,
            Ok(Err(error)) => {
                client.disconnect().await?;
                return Err(error);
            }
            Err(error) => {
                client.disconnect().await?;
                return Err(super::mining::unavailable(error));
            }
        };
        Ok(RecoveredSurvivalClient { client, evidence })
    }
    async fn admit(
        &self,
        client: &Client,
        target: &MiningRecoveryTarget,
    ) -> Result<MiningRecoveryEvidence> {
        client.wait_until_ready().await?;
        loop {
            let identity = client.connection_identity().await?;
            if identity.uuid != self.identity.uuid
                || identity.name != self.identity.name
                || identity.session.connection_id == self.identity.session.connection_id
            {
                return Err(super::mining::unavailable("fresh profile identity differs"));
            }
            let player = client.player_state().await?;
            if player.dimension.as_ref().is_some_and(|dimension| {
                Some(dimension) != self.original.initial.dimension.as_ref()
            }) {
                return Err(super::mining::unavailable("fresh mining dimension differs"));
            }
            if player
                .game_mode
                .is_some_and(|mode| mode != GameMode::Survival)
            {
                return Err(super::mining::unavailable(
                    "fresh mining requires survival mode",
                ));
            }
            if received_baselines(&player) {
                let region = crate::Region {
                    min: self.original.target,
                    max: self.original.target,
                };
                let capture = client.capture(region).await?;
                if let Some(state) = capture.world.blocks[0].state.as_ref() {
                    if !target.allows_baseline(state, &self.original.baseline) {
                        return Err(super::mining::unavailable(
                            "fresh target differs from declared recovery condition",
                        ));
                    }
                    // Adapter preview performs dry, healthy, normal-pose,
                    // grounded, supported, stationary admission without dispatch.
                    if let Ok(standing) = client
                        .survival()
                        .preview_path(&[SurvivalControl {
                            yaw: 0.0,
                            input: Default::default(),
                        }])
                        .await
                    {
                        // A packet between the capture and preview must not
                        // combine inventory/site facts from different states.
                        if standing.initial.session != identity.session
                            || !received_baselines(&standing.initial)
                            || standing.initial.receive_sequence != capture.player.receive_sequence
                            || standing.world_revision != capture.world.revision
                        {
                            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                            continue;
                        }
                        return Ok(MiningRecoveryEvidence {
                            method: MiningRecoveryMethod::SameProfileLogin,
                            original: self.source_record().await?,
                            identity,
                            standing,
                            target: state.clone(),
                            target_receive_sequence: capture.player.receive_sequence,
                        });
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }
}
/// Closed original history and fresh admission, separate from later results.
#[derive(Clone, Debug, serde::Serialize)]
pub struct MiningRecoveryEvidence {
    /// Retirement boundary interpretation under the declared vanilla contract.
    pub method: MiningRecoveryMethod,
    /// Original history remains closed and continuation-invalid.
    pub original: MiningRecord,
    /// Fresh authenticated identity on a new connection/world.
    pub identity: ConnectionIdentity,
    /// Fresh stationary admission and read-only one-tick prediction.
    pub standing: MotionPreview,
    /// Freshly loaded original target cell.
    pub target: NativeBlockState,
    /// Fresh connection's capture boundary, incomparable to the source ordinal.
    pub target_receive_sequence: u64,
}
/// New common Client after admission. Original IDs remain unusable.
pub struct RecoveredSurvivalClient {
    /// Fresh connection for explicit observation, new planning and operations.
    pub client: Client,
    /// Retained admission facts, never a command replay permit.
    pub evidence: MiningRecoveryEvidence,
}
