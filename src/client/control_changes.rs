//! Connection-owned control histories: remote contact degradation (Java 1.16.1)
//! and received pose corrections (both versions).
use super::super::SessionStamp;
use crate::{Error, ErrorKind, MovementRefusal, Result};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

/// Maximum retained records per control log, independent of gameplay events.
pub const MAX_RETAINED_CONTROL_CHANGES: usize = 256;

/// Boundary that discarded the remote numerical model, not a stopping reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub enum DegradationPhase {
    /// Initial contact admission failed; the owner still starts.
    Admission,
    /// Entity or team history no longer supports the admitted contact model.
    Refresh,
    /// A staged contact prediction failed without committing its remote state.
    Prediction,
    /// A received pose changed the numerical origin.
    Correction,
    /// Bounded unknown-terrain probing discarded the remote model.
    Terrain,
}

/// First remote-model withdrawal for one owner; retained even after stopping.
/// This is a client-model fact, never server acceptance of movement.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ControlDegradation {
    /// Connection and world that owned the discarded model.
    pub session: SessionStamp,
    /// Selected control owner; replacement never relabels this record.
    pub session_id: u64,
    /// Latest applied packet at withdrawal (actual pose packet for Correction).
    /// This is a receive boundary, not a server acknowledgement of the warning.
    pub receive_sequence: u64,
    /// Local model tick; admission is tick zero, predictions include the attempted tick.
    pub model_tick: u64,
    /// Where the first withdrawal occurred.
    pub phase: DegradationPhase,
    /// RemoteStateDropped wrapping the structured original cause.
    pub reason: MovementRefusal,
}

/// One degradation with an independent connection-local cursor.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ControlChange {
    /// Monotonic ordinal starting at one; unrelated to packet ordinals.
    pub sequence: u64,
    /// Frozen first withdrawal.
    pub degradation: ControlDegradation,
}

/// Complete retained suffix of remote-model withdrawals, not all control transitions.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ControlChangeLog {
    /// Cursor to use for the next read; unchanged by reads.
    pub sequence: u64,
    /// Changes in application order, including replaced/stopped owners.
    pub changes: Vec<ControlChange>,
}

/// Retention loss carried by a State error's diagnostic. Consumers can
/// explicitly rebaseline at sequence, or read the surviving suffix at resume_after.
/// Neither choice reconstructs the missing withdrawals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlChangeGap {
    /// Number of unread withdrawals that were evicted.
    pub missed: u64,
    /// Earliest readable cursor, immediately before the retained head.
    pub resume_after: u64,
    /// Tail at the failed read; explicit rebaseline can skip to this cursor.
    pub sequence: u64,
}
impl std::fmt::Display for ControlChangeGap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} unread control changes were evicted; retained after {}, tail {}",
            self.missed, self.resume_after, self.sequence
        )
    }
}
impl std::error::Error for ControlChangeGap {}

/// Last position this owner completely wrote; a submission, never an ACK.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct ControlSubmission {
    /// Local model tick whose position packet was written.
    pub model_tick: u64,
    /// Feet position carried by that packet.
    pub position: [f64; 3],
}

/// One received own-pose packet applied over a running control owner.
/// The protocol does not distinguish a teleport from a movement rejection:
/// this records that the local submission/model was superseded and by how
/// much, never a server-side cause.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ControlCorrection {
    /// Connection and world of the owner that received the pose.
    pub session: SessionStamp,
    /// Selected control owner; replacement never relabels this record.
    pub session_id: u64,
    /// Original receive ordinal of the position packet.
    pub receive_sequence: u64,
    /// Local model tick when the packet arrived (ticks completed so far).
    pub model_tick: u64,
    /// Latest written position before the packet; None before the first write.
    pub submitted: Option<ControlSubmission>,
    /// Local model feet the packet replaced. A preceding unapplied correction
    /// between two ticks counts, as the vanilla client applies each in order.
    pub model_position: [f64; 3],
    /// Received feet position, after relative flags were resolved.
    pub received_position: [f64; 3],
    /// `received_position - model_position` per axis.
    pub delta: [f64; 3],
    /// Euclidean length of `delta`.
    pub distance: f64,
    /// Velocity applied with the pose; None leaves the model velocity unchanged.
    pub received_velocity: Option<[f64; 3]>,
}

/// One correction with an independent connection-local cursor.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ControlCorrectionChange {
    /// Monotonic ordinal starting at one; unrelated to packet ordinals and to
    /// the degradation log's cursor.
    pub sequence: u64,
    /// Frozen correction.
    pub correction: ControlCorrection,
}

/// Complete retained suffix of received corrections over control owners.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ControlCorrectionLog {
    /// Cursor to use for the next read; unchanged by reads.
    pub sequence: u64,
    /// Corrections in receive order, including replaced/stopped owners.
    pub corrections: Vec<ControlCorrectionChange>,
}

impl crate::Client {
    /// Read the Java 1.16.1 connection's bounded remote-model degradation log.
    /// Cursor 0 starts history; State means retention loss, InvalidInput a future
    /// cursor. Java 1.21.11 returns Unsupported. Readable after disconnect.
    pub async fn control_changes_after(&self, after_sequence: u64) -> Result<ControlChangeLog> {
        crate::client::dispatch!(&self.adapter, a => super::super::adapter::ControlOps::control_changes_after(a, after_sequence).await)
    }

    /// Read the connection's bounded log of received own-pose corrections that
    /// arrived while a control owner was running or paused (both versions).
    /// Every packet is retained, including several between two ticks. Cursor 0
    /// starts history; State (diagnostic `ControlChangeGap`) means retention
    /// loss, InvalidInput a future cursor. Readable after disconnect. A record
    /// is a superseded submission, never a teleport/rejection cause.
    pub async fn control_corrections_after(
        &self,
        after_sequence: u64,
    ) -> Result<ControlCorrectionLog> {
        crate::client::dispatch!(&self.adapter, a => super::super::adapter::ControlOps::control_corrections_after(a, after_sequence).await)
    }
}

/// Bounded cursor log shared by the degradation and correction readers.
struct State<T> {
    sequence: u64,
    dropped_through: u64,
    retained: VecDeque<(u64, T)>,
}

pub(crate) struct Ledger<T>(Mutex<State<T>>);
impl<T> Default for Ledger<T> {
    fn default() -> Self {
        Self(Mutex::new(State {
            sequence: 0,
            dropped_through: 0,
            retained: VecDeque::new(),
        }))
    }
}
impl<T: Clone> Ledger<T> {
    fn push(&self, value: T) {
        let mut state = self.0.lock().expect("control log poisoned");
        state.sequence += 1;
        let sequence = state.sequence;
        state.retained.push_back((sequence, value));
        if state.retained.len() > MAX_RETAINED_CONTROL_CHANGES {
            state.dropped_through = state.retained.pop_front().expect("nonempty control log").0;
        }
    }

    fn suffix(&self, after_sequence: u64) -> Result<(u64, Vec<(u64, T)>)> {
        let state = self.0.lock().expect("control log poisoned");
        if after_sequence < state.dropped_through {
            return Err(Error::new(
                ErrorKind::State,
                ControlChangeGap {
                    missed: state.dropped_through - after_sequence,
                    resume_after: state.dropped_through,
                    sequence: state.sequence,
                },
            ));
        }
        if after_sequence > state.sequence {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                anyhow::anyhow!("control cursor is ahead of the log"),
            ));
        }
        Ok((
            state.sequence,
            state
                .retained
                .iter()
                .filter(|(sequence, _)| *sequence > after_sequence)
                .cloned()
                .collect(),
        ))
    }
}

pub(crate) type ControlChangeLedger = Ledger<ControlDegradation>;
impl Ledger<ControlDegradation> {
    pub(crate) fn record(&self, degradation: ControlDegradation) {
        self.push(degradation);
    }

    pub(crate) fn after(&self, after_sequence: u64) -> Result<ControlChangeLog> {
        let (sequence, retained) = self.suffix(after_sequence)?;
        Ok(ControlChangeLog {
            sequence,
            changes: retained
                .into_iter()
                .map(|(sequence, degradation)| ControlChange {
                    sequence,
                    degradation,
                })
                .collect(),
        })
    }
}

pub(crate) type ControlCorrectionLedger = Ledger<ControlCorrection>;
impl Ledger<ControlCorrection> {
    pub(crate) fn record(&self, correction: ControlCorrection) {
        self.push(correction);
    }

    pub(crate) fn after(&self, after_sequence: u64) -> Result<ControlCorrectionLog> {
        let (sequence, retained) = self.suffix(after_sequence)?;
        Ok(ControlCorrectionLog {
            sequence,
            corrections: retained
                .into_iter()
                .map(|(sequence, correction)| ControlCorrectionChange {
                    sequence,
                    correction,
                })
                .collect(),
        })
    }
}

/// Owner-scoped destination for received pose corrections.
pub(crate) struct ControlCorrectionSource {
    pub ledger: Arc<ControlCorrectionLedger>,
    pub session: SessionStamp,
}

pub(crate) struct ControlChangeSource {
    pub ledger: Arc<ControlChangeLedger>,
    pub session: SessionStamp,
    pub receive_sequence: u64,
}
