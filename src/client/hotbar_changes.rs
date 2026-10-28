//! Connection-owned selected-hotbar history (both versions).
#![doc = include_str!("../../docs/hotbar-changes.md")]
use super::SessionStamp;
use crate::{Error, ErrorKind, Result};
use std::{collections::VecDeque, sync::Mutex};

/// Maximum retained selected-hotbar records per connection, independent of events.
pub const MAX_RETAINED_HOTBAR_CHANGES: usize = 256;

/// Who wrote a selected-hotbar value. Neither is a server acknowledgement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub enum HotbarChangeOrigin {
    /// This connection completely wrote a serverbound held-slot packet
    /// (caller selection, Body selection or the respawn reassertion).
    Submitted,
    /// A clientbound held-slot packet was applied (join, cross-dimension
    /// teleport, pick-item, or any other server-side write).
    Received,
}

/// One applied selected-hotbar write, frozen at the time it was applied.
/// Equal writes are retained: a received packet that repeats `before` is
/// still a server write, never a confirmation of a submission.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct HotbarSelectionChange {
    /// Connection and world at application.
    pub session: SessionStamp,
    /// Latest applied packet ordinal; for Received the held-slot packet itself.
    /// This is a receive boundary, not a server acknowledgement of a submission.
    pub receive_sequence: u64,
    /// Who wrote `after`.
    pub origin: HotbarChangeOrigin,
    /// Slot of the previous record in this log; None before the first record.
    /// A local inventory reset without a write (respawn without metadata) is
    /// not a record, so `before` is the last written value.
    pub before: Option<u8>,
    /// Origin of the previous record; None before the first record.
    pub before_origin: Option<HotbarChangeOrigin>,
    /// Slot 0..=8 in effect after this write.
    pub after: u8,
    /// Received only: log cursor of the Submitted record whose slot this
    /// packet replaced with a different slot. Not a rejection: the server may
    /// have written its older value before applying the submission, leaving
    /// the server on the submitted slot. The original record is never rewritten.
    pub superseded_submission: Option<u64>,
}
impl HotbarSelectionChange {
    /// Whether the written slot differs from the previous record.
    pub fn changed(&self) -> bool {
        self.before != Some(self.after)
    }
}

/// One record with its own connection-local cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct HotbarChange {
    /// Monotonic ordinal starting at one; unrelated to packet ordinals.
    pub sequence: u64,
    /// Frozen write.
    pub change: HotbarSelectionChange,
}

/// Complete retained suffix of selected-hotbar writes after a cursor.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct HotbarChangeLog {
    /// Cursor to use for the next read; unchanged by reads.
    pub sequence: u64,
    /// Writes in application order.
    pub changes: Vec<HotbarChange>,
}

/// Retention loss carried by a State error's diagnostic. Consumers can
/// explicitly rebaseline at `sequence`, or read the surviving suffix at
/// `resume_after`. Neither choice reconstructs the missing writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HotbarChangeGap {
    /// Number of unread records that were evicted.
    pub missed: u64,
    /// Earliest readable cursor, immediately before the retained head.
    pub resume_after: u64,
    /// Tail at the failed read; explicit rebaseline can skip to this cursor.
    pub sequence: u64,
}
impl std::fmt::Display for HotbarChangeGap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} unread hotbar changes were evicted; retained after {}, tail {}",
            self.missed, self.resume_after, self.sequence
        )
    }
}
impl std::error::Error for HotbarChangeGap {}

impl crate::Client {
    /// Read the connection's bounded log of selected-hotbar writes (both
    /// versions), each with before/after and origin. Cursor 0 starts history;
    /// State (diagnostic [`HotbarChangeGap`]) means retention loss,
    /// InvalidInput a future cursor. Readable after disconnect. Sends nothing.
    pub async fn selected_hotbar_changes_after(
        &self,
        after_sequence: u64,
    ) -> Result<HotbarChangeLog> {
        super::dispatch!(&self.adapter, a => a.hotbar_changes_after(after_sequence).await)
    }
}

#[derive(Default)]
struct State {
    sequence: u64,
    dropped_through: u64,
    /// Last record's (slot, origin, cursor); survives eviction.
    current: Option<(u8, HotbarChangeOrigin, u64)>,
    retained: VecDeque<HotbarChange>,
}

#[derive(Default)]
pub(crate) struct HotbarChangeLedger(Mutex<State>);
impl HotbarChangeLedger {
    /// Record one completed write. Callers validate `slot` before applying it.
    pub(crate) fn record(
        &self,
        session: SessionStamp,
        receive_sequence: u64,
        origin: HotbarChangeOrigin,
        slot: u8,
    ) {
        debug_assert!(slot <= 8);
        let mut state = self.0.lock().expect("hotbar log poisoned");
        let superseded_submission = match (origin, state.current) {
            (
                HotbarChangeOrigin::Received,
                Some((value, HotbarChangeOrigin::Submitted, cursor)),
            ) if value != slot => Some(cursor),
            _ => None,
        };
        state.sequence += 1;
        let sequence = state.sequence;
        let change = HotbarSelectionChange {
            session,
            receive_sequence,
            origin,
            before: state.current.map(|(value, _, _)| value),
            before_origin: state.current.map(|(_, origin, _)| origin),
            after: slot,
            superseded_submission,
        };
        state.current = Some((slot, origin, sequence));
        state.retained.push_back(HotbarChange { sequence, change });
        if state.retained.len() > MAX_RETAINED_HOTBAR_CHANGES {
            state.dropped_through = state
                .retained
                .pop_front()
                .expect("nonempty hotbar log")
                .sequence;
        }
    }

    pub(crate) fn after(&self, after_sequence: u64) -> Result<HotbarChangeLog> {
        let state = self.0.lock().expect("hotbar log poisoned");
        if after_sequence < state.dropped_through {
            return Err(Error::new(
                ErrorKind::State,
                HotbarChangeGap {
                    missed: state.dropped_through - after_sequence,
                    resume_after: state.dropped_through,
                    sequence: state.sequence,
                },
            ));
        }
        if after_sequence > state.sequence {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                anyhow::anyhow!("hotbar cursor is ahead of the log"),
            ));
        }
        Ok(HotbarChangeLog {
            sequence: state.sequence,
            changes: state
                .retained
                .iter()
                .filter(|c| c.sequence > after_sequence)
                .copied()
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::HotbarChangeOrigin::{Received, Submitted};
    use super::*;

    fn session(world_generation: u64) -> SessionStamp {
        SessionStamp {
            version: crate::MinecraftVersion::Java1_16_1,
            connection_id: 7,
            world_generation,
        }
    }

    #[test]
    fn received_override_supersedes_latest_submission_without_rewriting_it() {
        let ledger = HotbarChangeLedger::default();
        ledger.record(session(1), 3, Received, 0);
        ledger.record(session(1), 5, Submitted, 4);
        // Server overrides the submission (pick-item / race) with another slot.
        ledger.record(session(1), 9, Received, 2);
        // A later identical received write supersedes nothing.
        ledger.record(session(1), 11, Received, 2);
        let log = ledger.after(0).unwrap();
        assert_eq!(log.sequence, 4);
        let c: Vec<_> = log.changes.iter().map(|c| c.change).collect();
        assert_eq!(c[0].before, None);
        assert_eq!(c[0].before_origin, None);
        assert!(c[0].changed());
        assert_eq!(
            (
                c[1].origin,
                c[1].before,
                c[1].after,
                c[1].superseded_submission
            ),
            (Submitted, Some(0), 4, None)
        );
        assert_eq!(
            (c[2].origin, c[2].before, c[2].before_origin, c[2].after),
            (Received, Some(4), Some(Submitted), 2)
        );
        assert_eq!(c[2].superseded_submission, Some(2));
        assert_eq!(c[2].receive_sequence, 9);
        assert!(!c[3].changed());
        assert_eq!(c[3].superseded_submission, None);
        assert_eq!(c[3].before_origin, Some(Received));
        // The superseded Submitted record is frozen.
        assert_eq!(ledger.after(1).unwrap().changes[0].change, c[1]);
    }

    #[test]
    fn received_equal_to_submission_is_not_superseded_or_confirmed() {
        let ledger = HotbarChangeLedger::default();
        ledger.record(session(1), 1, Submitted, 6);
        ledger.record(session(2), 4, Received, 6);
        let log = ledger.after(1).unwrap();
        let c = log.changes[0].change;
        assert_eq!(c.superseded_submission, None);
        assert!(!c.changed());
        assert_eq!(c.session.world_generation, 2);
    }

    #[test]
    fn cursor_gap_and_future_cursor() {
        let ledger = HotbarChangeLedger::default();
        let total = MAX_RETAINED_HOTBAR_CHANGES as u64 + 3;
        for i in 0..total {
            ledger.record(session(1), i, Submitted, (i % 9) as u8);
        }
        let error = ledger.after(0).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::State);
        let gap = error
            .diagnostic()
            .downcast_ref::<HotbarChangeGap>()
            .copied()
            .expect("typed gap");
        assert_eq!(
            gap,
            HotbarChangeGap {
                missed: 3,
                resume_after: 3,
                sequence: total
            }
        );
        let retained = ledger.after(gap.resume_after).unwrap();
        assert_eq!(retained.changes.len(), MAX_RETAINED_HOTBAR_CHANGES);
        assert_eq!(retained.changes[0].sequence, 4);
        // before still reflects the evicted predecessor.
        assert_eq!(retained.changes[0].change.before, Some(2));
        assert!(ledger.after(total).unwrap().changes.is_empty());
        assert_eq!(
            ledger.after(total + 1).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
    }
}
