//! Connection-owned PlayerControl state transitions (both versions).
//!
//! The executor publishes the owner's committed `ControlStatus` after each
//! state-changing critical section, so Started/Paused/Resumed/Stopped/Expired
//! are visible without re-querying `control_record()`. Retention, cursor and
//! gap semantics match `control_changes_after`; policies are unchanged.
#![doc = include_str!("../../docs/control-state-changes.md")]
use super::super::SessionStamp;
use super::{ControlChangeGap, ControlKind, ControlSession, ControlStatus};
use crate::{Error, ErrorKind, Result};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

/// Typed stop cause derived from the reason the owner recorded when it stopped.
/// The original reason text is kept beside it; `Other` is never a guess.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ControlStopCause {
    /// `stop_control*` or quiesce by the caller/Body.
    Requested,
    /// Respawn, dimension change or reconfiguration replaced the owner's world.
    WorldChanged,
    /// The received game mode differs from the admitted mode.
    GameModeChanged,
    /// Received abilities/flying or actual Elytra state left the admitted model.
    FlightChanged,
    /// The player is riding; vehicle motion is outside the engine.
    Vehicle,
    /// A received explosion impulse this owner cannot model.
    Explosion,
    /// Vehicle or explosion motion outside the engine, not distinguishable here.
    UnmodeledMotion,
    /// A received own-pose correction invalidated a strict or glide owner.
    Correction,
    /// Received scale/attributes left the admitted model.
    ModelChanged,
    /// The received health reached zero.
    Died,
    /// The owner finished its own model: path dispatched, flight landed, glide ended.
    Completed,
    /// A movement/input write failed; inspect before reuse.
    WriteFailed,
    /// The connection stopped or is not ready.
    Disconnected,
    /// The engine could not predict a tick (terrain, contacts, item use).
    Prediction,
    /// A later owner replaced this one before its stop was observed.
    Replaced,
    /// Unclassified reason; inspect `reason`.
    Other,
}

impl ControlStopCause {
    /// Classify a recorded stop reason. Exact adapter literals first; error
    /// texts with a typed movement refusal are prediction failures.
    pub(crate) fn classify(reason: &str, refusal: bool) -> Self {
        let has = |needle: &str| reason.contains(needle);
        if has("write failed") {
            Self::WriteFailed
        } else if reason.starts_with("world changed") {
            Self::WorldChanged
        } else if reason == "stopped by request" || reason == "neutralized during quiesce" {
            Self::Requested
        } else if reason == "game mode changed" {
            Self::GameModeChanged
        } else if reason == "riding" {
            Self::Vehicle
        } else if reason == "vehicle or explosion motion outside the engine" {
            Self::UnmodeledMotion
        } else if has("explosion") {
            Self::Explosion
        } else if has("correction") {
            Self::Correction
        } else if reason == "player died" {
            Self::Died
        } else if reason == "disconnected" || has("connection") || has("not ready") {
            Self::Disconnected
        } else if has("fully dispatched") || has("landed") || has("glide model") {
            Self::Completed
        } else if reason == "flying"
            || has("Elytra")
            || has("glide ended")
            || has("glider equipment")
            || reason.starts_with("flight ")
        {
            Self::FlightChanged
        } else if has("scale") {
            Self::ModelChanged
        } else if refusal {
            Self::Prediction
        } else {
            Self::Other
        }
    }
}

/// One state transition of a PlayerControl owner.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ControlStateEvent {
    /// The owner was admitted and is running.
    Started,
    /// A tick could not be predicted; nothing is sent while paused.
    Paused {
        /// `ControlStatus::Paused` reason.
        reason: String,
    },
    /// A paused owner predicted and continued again.
    Resumed,
    /// The owner ended (record status `Stopped`).
    Stopped {
        /// Typed cause derived from `reason`.
        cause: ControlStopCause,
        /// `ControlStatus::Stopped` reason, unchanged.
        reason: String,
    },
    /// The owner ended because a deadline expired (record status `Stopped`),
    /// such as a best-effort path's evidence deadline.
    Expired {
        /// `ControlStatus::Stopped` reason, unchanged.
        reason: String,
    },
}

/// A frozen transition with the owner and receive boundary that published it.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ControlTransition {
    /// Connection and world of the owner.
    pub session: SessionStamp,
    /// Selected control owner; later owners never relabel this record.
    pub session_id: u64,
    /// Model admitted for the owner.
    pub control_kind: ControlKind,
    /// Latest applied packet ordinal when the transition was committed. For
    /// receipt-driven stops this is at or after the causing packet.
    pub receive_sequence: u64,
    /// Local model ticks completed at the transition.
    pub model_tick: u64,
    /// Movement ticks completely written at the transition; never acceptance.
    pub dispatched_ticks: u64,
    /// What changed.
    pub event: ControlStateEvent,
}

/// One transition with an independent connection-local cursor.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ControlStateChange {
    /// Monotonic ordinal starting at one; unrelated to packet ordinals and to
    /// the other control logs' cursors.
    pub sequence: u64,
    /// Frozen transition.
    pub transition: ControlTransition,
}

/// Complete retained suffix of control state transitions.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ControlStateLog {
    /// Cursor to use for the next read; unchanged by reads.
    pub sequence: u64,
    /// Transitions in commit order, including replaced/stopped owners.
    pub changes: Vec<ControlStateChange>,
}

impl crate::Client {
    /// Read the connection's bounded log of PlayerControl state transitions
    /// (Started/Paused/Resumed/Stopped/Expired), on both versions. Cursor 0
    /// starts history; State (diagnostic `ControlChangeGap`) means retention
    /// loss, InvalidInput a future cursor. Readable after disconnect.
    pub async fn control_state_changes_after(
        &self,
        after_sequence: u64,
    ) -> Result<ControlStateLog> {
        crate::client::dispatch!(&self.adapter, a => super::super::adapter::ControlOps::control_state_changes_after(a, after_sequence).await)
    }
}

#[derive(Default)]
struct State {
    sequence: u64,
    dropped_through: u64,
    changes: VecDeque<ControlStateChange>,
}

/// Bounded cursor log; the same retention as the degradation log.
#[derive(Default)]
pub(crate) struct ControlStateLedger(Mutex<State>);
impl ControlStateLedger {
    fn record(&self, transition: ControlTransition) {
        let mut state = self.0.lock().expect("control state log poisoned");
        state.sequence += 1;
        let sequence = state.sequence;
        state.changes.push_back(ControlStateChange {
            sequence,
            transition,
        });
        if state.changes.len() > super::MAX_RETAINED_CONTROL_CHANGES {
            state.dropped_through = state
                .changes
                .pop_front()
                .expect("nonempty control state log")
                .sequence;
        }
    }

    pub(crate) fn after(&self, after_sequence: u64) -> Result<ControlStateLog> {
        let state = self.0.lock().expect("control state log poisoned");
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
                anyhow::anyhow!("control state cursor is ahead of the log"),
            ));
        }
        Ok(ControlStateLog {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Running,
    Paused,
    Stopped,
}

struct Published {
    session_id: u64,
    phase: Phase,
    session: SessionStamp,
    control_kind: ControlKind,
    model_tick: u64,
    dispatched_ticks: u64,
}

/// Executor-owned publisher: compares the selected owner's committed status
/// with the last published one. Adapters call `publish` while still holding
/// the executor lock that committed the status.
#[derive(Default)]
pub(crate) struct ControlStateTracker {
    pub(crate) ledger: Arc<ControlStateLedger>,
    last: Option<Published>,
}

impl ControlStateTracker {
    /// `hint` refines an ambiguous stop reason with adapter state (for
    /// example a received explosion behind "vehicle or explosion" motion).
    pub(crate) fn publish(
        &mut self,
        owner: Option<&ControlSession>,
        session: SessionStamp,
        receive_sequence: u64,
        hint: Option<ControlStopCause>,
    ) {
        let Some(owner) = owner else {
            return;
        };
        let phase = match owner.status {
            ControlStatus::Running => Phase::Running,
            ControlStatus::Paused { .. } => Phase::Paused,
            ControlStatus::Stopped { .. } => Phase::Stopped,
        };
        let record = |event, published: &Published| ControlTransition {
            session: published.session,
            session_id: published.session_id,
            control_kind: published.control_kind,
            receive_sequence,
            model_tick: published.model_tick,
            dispatched_ticks: published.dispatched_ticks,
            event,
        };
        let previous = match self.last.take() {
            Some(last) if last.session_id == owner.id => Some(last.phase),
            Some(last) => {
                // The replaced owner's final status is no longer readable.
                if last.phase != Phase::Stopped {
                    let event = ControlStateEvent::Stopped {
                        cause: ControlStopCause::Replaced,
                        reason: "replaced by a later control owner".into(),
                    };
                    self.ledger.record(record(event, &last));
                }
                None
            }
            None => None,
        };
        let current = Published {
            session_id: owner.id,
            phase,
            session,
            control_kind: owner.kind,
            model_tick: owner.tick,
            dispatched_ticks: owner.dispatched_ticks,
        };
        let mut events = Vec::new();
        match (previous, &owner.status) {
            // A model handed over already stopped (finite handoff) never ran.
            (None, ControlStatus::Stopped { .. }) => {}
            (None, status) => {
                events.push(ControlStateEvent::Started);
                if let ControlStatus::Paused { reason } = status {
                    events.push(ControlStateEvent::Paused {
                        reason: reason.clone(),
                    });
                }
            }
            (Some(Phase::Stopped), _) => {}
            (Some(Phase::Running), ControlStatus::Paused { reason }) => {
                events.push(ControlStateEvent::Paused {
                    reason: reason.clone(),
                });
            }
            (Some(Phase::Paused), ControlStatus::Running) => {
                events.push(ControlStateEvent::Resumed);
            }
            (Some(_), ControlStatus::Stopped { reason }) => {
                events.push(stopped(reason, owner.movement_refusal.is_some(), hint));
            }
            (Some(_), _) => {}
        }
        for event in events {
            self.ledger.record(record(event, &current));
        }
        self.last = Some(current);
    }
}

fn stopped(reason: &str, refusal: bool, hint: Option<ControlStopCause>) -> ControlStateEvent {
    if reason.contains("deadline expired") {
        return ControlStateEvent::Expired {
            reason: reason.into(),
        };
    }
    let mut cause = ControlStopCause::classify(reason, refusal);
    if cause == ControlStopCause::UnmodeledMotion {
        cause = hint.unwrap_or(cause);
    }
    ControlStateEvent::Stopped {
        cause,
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MinecraftVersion;
    use crate::client::control::Received;

    fn stamp() -> SessionStamp {
        SessionStamp {
            version: MinecraftVersion::Java1_21_11,
            connection_id: 4,
            world_generation: 2,
        }
    }

    fn owner(id: u64) -> ControlSession {
        let received = Received {
            pose: None,
            velocity: None,
            using_item: None,
            abilities_sequence: None,
            environment: crate::client::physics::Environment::defaults(
                MinecraftVersion::Java1_21_11,
            ),
        };
        ControlSession::new(
            MinecraftVersion::Java1_21_11,
            id,
            [0.5, 64.0, 0.5],
            &received,
        )
    }

    fn events(tracker: &ControlStateTracker) -> Vec<(u64, u64, ControlStateEvent)> {
        tracker
            .ledger
            .after(0)
            .unwrap()
            .changes
            .into_iter()
            .map(|c| {
                (
                    c.transition.session_id,
                    c.transition.receive_sequence,
                    c.transition.event,
                )
            })
            .collect()
    }

    #[test]
    fn transitions_are_published_once_in_commit_order() {
        let mut tracker = ControlStateTracker::default();
        let mut session = owner(1);
        tracker.publish(Some(&session), stamp(), 10, None);
        tracker.publish(Some(&session), stamp(), 11, None);
        session.status = ControlStatus::Paused {
            reason: "terrain".into(),
        };
        tracker.publish(Some(&session), stamp(), 12, None);
        tracker.publish(Some(&session), stamp(), 13, None);
        session.status = ControlStatus::Running;
        tracker.publish(Some(&session), stamp(), 14, None);
        session.status = ControlStatus::Stopped {
            reason: "world changed (respawn or dimension)".into(),
        };
        tracker.publish(Some(&session), stamp(), 15, None);
        tracker.publish(Some(&session), stamp(), 16, None);
        tracker.publish(None, stamp(), 17, None);
        assert_eq!(
            events(&tracker),
            vec![
                (1, 10, ControlStateEvent::Started),
                (
                    1,
                    12,
                    ControlStateEvent::Paused {
                        reason: "terrain".into()
                    }
                ),
                (1, 14, ControlStateEvent::Resumed),
                (
                    1,
                    15,
                    ControlStateEvent::Stopped {
                        cause: ControlStopCause::WorldChanged,
                        reason: "world changed (respawn or dimension)".into()
                    }
                ),
            ]
        );
        let first = &tracker.ledger.after(0).unwrap().changes[0].transition;
        assert_eq!(
            (first.session, first.control_kind),
            (stamp(), ControlKind::Ground)
        );
    }

    #[test]
    fn replacement_handoff_expiry_and_hints() {
        let mut tracker = ControlStateTracker::default();
        let running = owner(1);
        tracker.publish(Some(&running), stamp(), 1, None);
        // Replaced while still running: the old owner is closed explicitly.
        let mut handed_over = owner(2);
        handed_over.status = ControlStatus::Stopped {
            reason: "finite executor terminated; strict proof retained separately".into(),
        };
        tracker.publish(Some(&handed_over), stamp(), 2, None);
        let mut path = owner(3);
        tracker.publish(Some(&path), stamp(), 3, None);
        path.status = ControlStatus::Stopped {
            reason: "best-effort path evidence deadline expired".into(),
        };
        tracker.publish(Some(&path), stamp(), 4, None);
        let mut explosion = owner(4);
        tracker.publish(Some(&explosion), stamp(), 5, None);
        explosion.status = ControlStatus::Stopped {
            reason: "vehicle or explosion motion outside the engine".into(),
        };
        tracker.publish(
            Some(&explosion),
            stamp(),
            6,
            Some(ControlStopCause::Explosion),
        );
        let log = events(&tracker);
        assert_eq!(log.len(), 6, "{log:?}");
        assert!(matches!(
            &log[1],
            (
                1,
                2,
                ControlStateEvent::Stopped {
                    cause: ControlStopCause::Replaced,
                    ..
                }
            )
        ));
        assert_eq!(log[2], (3, 3, ControlStateEvent::Started));
        assert!(matches!(&log[3], (3, 4, ControlStateEvent::Expired { .. })));
        assert!(matches!(
            &log[5],
            (
                4,
                6,
                ControlStateEvent::Stopped {
                    cause: ControlStopCause::Explosion,
                    ..
                }
            )
        ));
    }

    #[test]
    fn adapter_stop_reasons_classify_to_typed_causes() {
        use ControlStopCause::*;
        for (reason, refusal, cause) in [
            ("stopped by request", false, Requested),
            ("neutralized during quiesce", false, Requested),
            ("world changed (respawn or dimension)", false, WorldChanged),
            (
                "world changed (respawn, dimension or reconfiguration)",
                false,
                WorldChanged,
            ),
            ("game mode changed", false, GameModeChanged),
            ("flying", false, FlightChanged),
            (
                "actual Elytra glide requires a separate owner",
                false,
                FlightChanged,
            ),
            ("actual glide ended", false, FlightChanged),
            (
                "glider equipment unavailable or unusable",
                false,
                FlightChanged,
            ),
            ("flight permission unavailable", false, FlightChanged),
            ("riding", false, Vehicle),
            (
                "vehicle or explosion motion outside the engine",
                false,
                UnmodeledMotion,
            ),
            ("glide correction requires a new owner", false, Correction),
            ("own correction discarded contact origin", true, Correction),
            ("unmodeled player scale", false, ModelChanged),
            ("player died", false, Died),
            ("disconnected", false, Disconnected),
            ("play state is not ready", false, Disconnected),
            (
                "best-effort path fully dispatched; acceptance is separate",
                false,
                Completed,
            ),
            ("creative flight model landed", false, Completed),
            (
                "glide model landed or reached a ladder; actual flag is separate",
                false,
                Completed,
            ),
            (
                "write failed; inspect before reuse: closed",
                false,
                WriteFailed,
            ),
            ("stopped; release write failed: closed", false, WriteFailed),
            ("received terrain is not loaded", true, Prediction),
            ("position attempt IDs exhausted", false, Other),
        ] {
            assert_eq!(
                ControlStopCause::classify(reason, refusal),
                cause,
                "{reason}"
            );
        }
    }

    #[test]
    fn state_log_reports_eviction_and_future_cursor() {
        let mut tracker = ControlStateTracker::default();
        for id in 0..super::super::MAX_RETAINED_CONTROL_CHANGES as u64 + 3 {
            tracker.publish(Some(&owner(id)), stamp(), id, None);
        }
        // Each replacement closes the previous owner: two records per owner but the first.
        let tail = 2 * (super::super::MAX_RETAINED_CONTROL_CHANGES as u64 + 3) - 1;
        let error = tracker.ledger.after(0).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::State);
        let gap = *error
            .diagnostic()
            .downcast_ref::<ControlChangeGap>()
            .unwrap();
        assert_eq!(gap.sequence, tail);
        assert_eq!(
            tracker
                .ledger
                .after(gap.resume_after)
                .unwrap()
                .changes
                .len(),
            super::super::MAX_RETAINED_CONTROL_CHANGES
        );
        assert_eq!(
            tracker.ledger.after(tail + 1).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
        assert!(tracker.ledger.after(tail).unwrap().changes.is_empty());
    }
}
