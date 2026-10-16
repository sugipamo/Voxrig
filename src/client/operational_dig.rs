//! Connection-owned operational digging. Checks and packet writes share adapter state locks.
use super::dig::{DigContext, DigEstimate, DigRecord};
use super::{BlockFace, GameMode, PlayerObservation, SessionStamp, adapter::DigOps};
use crate::{Error, ErrorKind, NativeBlockState, Result};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::oneshot;

#[derive(Default)]
pub(crate) struct Coordinator {
    operation: Arc<tokio::sync::Mutex<()>>,
    cleanup: Mutex<Option<DigCleanupRecord>>,
}
impl Coordinator {
    pub(crate) fn active(&self) -> bool {
        self.operation.try_lock().is_err()
    }
}
/// Local cancellation cleanup; dispatch is not a server acknowledgement.
#[derive(Clone, Debug, serde::Serialize)]
pub struct DigCleanupRecord {
    /// Original transport and world.
    pub session: SessionStamp,
    /// Original target, never a replacement world target.
    pub target: [i32; 3],
    /// Cleanup submission outcome.
    pub status: DigCleanupStatus,
    /// Original cancellation/admission error and any cleanup failure.
    pub reason: String,
    /// ABORT interaction sequence when provided by the protocol.
    pub interaction_sequence: Option<i32>,
}
/// Outcome of cleanup on the original connection and world only.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub enum DigCleanupStatus {
    /// Cleanup has been retained before attempting I/O.
    Pending,
    /// One complete ABORT write; no server acknowledgement claimed.
    Aborted,
    /// No packet sent into a changed world or mode.
    ContextChanged,
    /// Cleanup was refused or I/O failed; inspect instead of retrying.
    Failed,
}
#[derive(Clone)]
pub(crate) struct Inputs {
    pub player: PlayerObservation,
    pub state: NativeBlockState,
    pub estimate: DigEstimate,
}
#[derive(Clone, Copy)]
pub(crate) enum Action {
    Start,
    Finish,
    Abort,
}
impl Action {
    pub(crate) fn status(self) -> i32 {
        match self {
            Self::Start => 0,
            Self::Finish => 2,
            Self::Abort => 1,
        }
    }
}
pub(crate) enum Dispatch {
    Sent(Option<i32>),
    Changed(Option<NativeBlockState>),
    /// No packet sent into a changed world or mode.
    ContextChanged,
}
pub(crate) fn unavailable(message: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::State, anyhow::anyhow!("{message}"))
}
pub(crate) fn eye_cell(player: &PlayerObservation) -> Result<([i32; 3], f64)> {
    let feet = player
        .position
        .as_ref()
        .ok_or_else(|| unavailable("dig position unavailable"))?
        .value;
    let eye_y = feet[1] + 1.62;
    let probe = if player.session.version == crate::MinecraftVersion::Java1_16_1 {
        eye_y - f64::from(0.11111111f32)
    } else {
        eye_y
    };
    Ok((
        [
            feet[0].floor() as i32,
            probe.floor() as i32,
            feet[2].floor() as i32,
        ],
        eye_y,
    ))
}
pub(crate) fn inputs(
    player: PlayerObservation,
    state: NativeBlockState,
    on_ground: bool,
    at_eye: NativeBlockState,
    above_eye: NativeBlockState,
) -> Result<Inputs> {
    if player.game_mode != Some(GameMode::Survival) {
        return Err(unavailable("dig requires received survival mode"));
    }
    let (cell, eye_y) = eye_cell(&player)?;
    let version = player.session.version;
    let state_id = super::registry::Registry::for_version(version)
        .resolve_block_state(&state)?
        .value();
    let eye_in_water = super::dig::eye_in_water(version, eye_y, &at_eye, &above_eye, cell[1])?;
    let estimate = super::dig::estimate(
        version,
        state_id,
        &DigContext {
            player: &player,
            on_ground,
            eye_in_water,
        },
    )?;
    Ok(Inputs {
        player,
        state,
        estimate,
    })
}
pub(crate) fn validate(initial: &Inputs, current: &Inputs) -> Result<()> {
    if initial.player.session != current.player.session
        || current.player.game_mode != Some(GameMode::Survival)
    {
        return Err(unavailable(
            "dig original world or mode changed before dispatch",
        ));
    }
    let selected = initial
        .player
        .selected_hotbar
        .as_ref()
        .ok_or_else(|| unavailable("dig selection unavailable"))?
        .value;
    if current.player.selected_hotbar.as_ref().map(|s| s.value) != Some(selected)
        || !super::dig::mining_hand_compatible(
            initial.player.session.version,
            super::dig::slot(&initial.player, 36 + usize::from(selected))?,
            super::dig::slot(&current.player, 36 + usize::from(selected))?,
        )?
        || initial.estimate != current.estimate
    {
        return Err(unavailable(
            "dig selected hand or timing prerequisites changed before dispatch",
        ));
    }
    Ok(())
}
struct CancelOnDrop {
    cancelled: Arc<AtomicBool>,
    _signal: oneshot::Sender<()>,
}
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
impl super::Survival {
    pub(crate) async fn operational_dig(
        &self,
        target: [i32; 3],
        face: BlockFace,
    ) -> Result<DigRecord> {
        let (signal, receive) = oneshot::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let _cancel = CancelOnDrop {
            cancelled: cancelled.clone(),
            _signal: signal,
        };
        let owner = self.clone();
        let work = super::dispatch!(&self.client.adapter, a => DigOps::dig_work(a))?;
        super::dispatch!(&self.client.adapter, a => super::adapter::DrainOps::drain_ledger(a))
            .tasks()
            .spawn(
                work.run_result(
                    async move { owner.dig_owned(target, face, cancelled, receive).await },
                ),
            )
            .await
            .map_err(|e| unavailable(format!("dig owner failed: {e}")))?
    }
    /// Latest original cancellation/error cleanup; never permission to replay.
    pub fn dig_cleanup_record(&self) -> Option<DigCleanupRecord> {
        let coordinator = super::dispatch!(&self.client.adapter, a => DigOps::dig_coordinator(a));
        coordinator.cleanup.lock().expect("dig cleanup").clone()
    }
    async fn dig_owned(
        &self,
        target: [i32; 3],
        face: BlockFace,
        cancelled: Arc<AtomicBool>,
        mut receive: oneshot::Receiver<()>,
    ) -> Result<DigRecord> {
        let coordinator = super::dispatch!(&self.client.adapter, a => DigOps::dig_coordinator(a));
        let _exclusive = coordinator
            .operation
            .clone()
            .try_lock_owned()
            .map_err(|_| unavailable("another operational dig owns this connection"))?;
        let initial =
            super::dispatch!(&self.client.adapter, a => DigOps::dig_snapshot(a, target).await)?;
        let unresolved = coordinator
            .cleanup
            .lock()
            .expect("dig cleanup")
            .as_ref()
            .filter(|r| {
                r.session == initial.player.session
                    && matches!(
                        r.status,
                        DigCleanupStatus::Pending | DigCleanupStatus::Failed
                    )
            })
            .cloned();
        if let Some(record) = unresolved {
            return Err(Error::new(
                ErrorKind::OperationUnconfirmed,
                anyhow::anyhow!(
                    "prior dig cleanup is unresolved for {:?} in {:?}: {}; inspect without replay",
                    record.target,
                    record.session,
                    record.reason
                ),
            ));
        }
        if cancelled.load(Ordering::Acquire) {
            return Err(unavailable("dig cancelled before START"));
        }
        let start = super::dispatch!(&self.client.adapter, a => DigOps::dig_dispatch(a, &initial, target, face, Action::Start, &cancelled).await)?;
        let mut sequences = Vec::new();
        match start {
            Dispatch::Sent(sequence) => sequences.extend(sequence),
            _ => return Err(unavailable("dig target/context changed before START")),
        }
        let finish = if initial.estimate.ticks > 0 {
            let wait = std::time::Duration::from_millis((initial.estimate.ticks + 1) * 50);
            tokio::select! {
                biased;
                _ = &mut receive => Err(unavailable("dig caller cancelled after START")),
                _ = tokio::time::sleep(wait) => {
                    super::dispatch!(&self.client.adapter, a => DigOps::dig_dispatch(a, &initial, target, face, Action::Finish, &cancelled).await)
                }
            }
        } else {
            Ok(Dispatch::Sent(None))
        };
        match finish {
            Err(error) => {
                let cleanup = self
                    .cleanup_dig(
                        &coordinator,
                        &initial,
                        target,
                        face,
                        &cancelled,
                        error.to_string(),
                    )
                    .await;
                if cleanup.status == DigCleanupStatus::Failed {
                    return Err(Error::new(
                        ErrorKind::OperationUnconfirmed,
                        anyhow::Error::new(error).context(format!(
                            "dig cleanup unresolved for {:?} in {:?}: {}",
                            cleanup.target, cleanup.session, cleanup.reason
                        )),
                    ));
                }
                return Err(error);
            }
            Ok(Dispatch::Changed(state)) => {
                return Ok(DigRecord {
                    target,
                    estimate: initial.estimate,
                    removed: super::dig::is_air(state.as_ref()),
                    final_state: state,
                    sequences,
                });
            }
            Ok(Dispatch::ContextChanged) => {
                return Err(unavailable("dig original context changed"));
            }
            Ok(Dispatch::Sent(sequence)) => sequences.extend(sequence),
        }
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Err(unavailable("dig caller cancelled after completed dispatch"));
            }
            let capture = self
                .client
                .capture(crate::Region {
                    min: target,
                    max: target,
                })
                .await?;
            if capture.player.session != initial.player.session {
                return Err(unavailable("dig world changed while awaiting target"));
            }
            let state = capture
                .world
                .blocks
                .into_iter()
                .next()
                .and_then(|b| b.state);
            if state.as_ref() != Some(&initial.state) || tokio::time::Instant::now() >= deadline {
                if state.as_ref() == Some(&initial.state) {
                    let _ = super::ledger::finite::checked::<(), _>(Err(()));
                }
                return Ok(DigRecord {
                    target,
                    estimate: initial.estimate,
                    removed: super::dig::is_air(state.as_ref()),
                    final_state: state,
                    sequences,
                });
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
    async fn cleanup_dig(
        &self,
        coordinator: &Coordinator,
        initial: &Inputs,
        target: [i32; 3],
        face: BlockFace,
        cancelled: &AtomicBool,
        reason: String,
    ) -> DigCleanupRecord {
        let mut record = DigCleanupRecord {
            session: initial.player.session,
            target,
            status: DigCleanupStatus::Pending,
            reason,
            interaction_sequence: None,
        };
        *coordinator.cleanup.lock().expect("dig cleanup") = Some(record.clone());
        let result = super::dispatch!(&self.client.adapter, a => DigOps::dig_dispatch(a, initial, target, face, Action::Abort, cancelled).await);
        match result {
            Ok(Dispatch::Sent(sequence)) => {
                record.status = DigCleanupStatus::Aborted;
                record.interaction_sequence = sequence;
            }
            Ok(_) => record.status = DigCleanupStatus::ContextChanged,
            Err(e) => {
                record.status = DigCleanupStatus::Failed;
                record.reason.push_str(&format!("; ABORT failed: {e}"));
            }
        }
        *coordinator.cleanup.lock().expect("dig cleanup") = Some(record.clone());
        record
    }
}
