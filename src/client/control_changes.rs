//! Connection-owned remote contact degradation history (Java 1.16.1).
use super::super::SessionStamp;
use crate::{Error, ErrorKind, MovementRefusal, Result};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

/// Maximum retained first-degradation records, independent of gameplay events.
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

impl crate::Client {
    /// Read the Java 1.16.1 connection's bounded remote-model degradation log.
    /// Cursor 0 starts history; State means retention loss, InvalidInput a future
    /// cursor. Java 1.21.11 returns Unsupported. Readable after disconnect.
    pub async fn control_changes_after(&self, after_sequence: u64) -> Result<ControlChangeLog> {
        crate::client::dispatch!(&self.adapter, a => super::super::adapter::ControlOps::control_changes_after(a, after_sequence).await)
    }
}

#[derive(Default)]
struct State {
    sequence: u64,
    dropped_through: u64,
    changes: VecDeque<ControlChange>,
}

#[derive(Default)]
pub(crate) struct ControlChangeLedger(Mutex<State>);
impl ControlChangeLedger {
    pub(crate) fn record(&self, degradation: ControlDegradation) {
        let mut state = self.0.lock().expect("control log poisoned");
        state.sequence += 1;
        let sequence = state.sequence;
        state.changes.push_back(ControlChange {
            sequence,
            degradation,
        });
        if state.changes.len() > MAX_RETAINED_CONTROL_CHANGES {
            state.dropped_through = state
                .changes
                .pop_front()
                .expect("nonempty control log")
                .sequence;
        }
    }

    pub(crate) fn after(&self, after_sequence: u64) -> Result<ControlChangeLog> {
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
        Ok(ControlChangeLog {
            sequence: state.sequence,
            changes: state
                .changes
                .iter()
                .filter(|c| c.sequence > after_sequence)
                .cloned()
                .collect(),
        })
    }
}

pub(crate) struct ControlChangeSource {
    pub ledger: Arc<ControlChangeLedger>,
    pub session: SessionStamp,
    pub receive_sequence: u64,
}
