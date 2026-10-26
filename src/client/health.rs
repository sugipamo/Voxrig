//! Received HP history, isolated from gameplay event retention.
#![doc = include_str!("../../docs/health-log.md")]
use super::{Health, SessionStamp};
use crate::{Error, ErrorKind, Result};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::sync::Notify;

/// Maximum retained health/lifecycle records per connection, independent of events.
pub const MAX_RETAINED_HEALTH_CHANGES: usize = 4096;

/// Original update_health payload and its packet provenance. Never predicted.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct ReceivedHealth {
    /// Connection/version/world at application.
    pub session: SessionStamp,
    /// Original connection-local packet ordinal, not the health log cursor.
    pub receive_sequence: u64,
    /// Raw server values, including invalid floating point values.
    pub health: Health,
}
impl ReceivedHealth {
    /// Whether HP is finite and nonnegative. Zero means death and is valid.
    /// This does not validate food/saturation or impose a maximum HP.
    pub fn is_valid(&self) -> bool {
        self.health.health.is_finite() && self.health.health >= 0.0
    }
}

/// One received health or lifecycle change.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub enum HealthEvent {
    /// Every applied update_health, even equal or invalid values.
    Vitals(ReceivedHealth),
    /// Join, respawn, dimension change or reconfiguration; invalidates old HP.
    WorldChanged(SessionStamp),
    /// Terminal connection boundary. No later record follows.
    Disconnected,
}
/// One health log entry with its own cursor and original packet boundary.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct HealthChange {
    /// Monotonic health/lifecycle ordinal within this connection (starts at 1).
    pub sequence: u64,
    /// Original packet ordinal; None for a transport/local disconnect.
    pub receive_sequence: Option<u64>,
    /// Original received evidence or lifecycle boundary.
    pub event: HealthEvent,
}
/// Complete retained suffix after a health cursor.
#[derive(Clone, Debug, serde::Serialize)]
pub struct HealthLog {
    /// Pass as after_sequence on the next read; distinct from packet ordinals.
    pub sequence: u64,
    /// Changes in application order. No batch limit or acknowledgement side effect.
    pub changes: Vec<HealthChange>,
    /// Terminal boundary has been published (and remains in the log).
    pub closed: bool,
}
/// A subscription read result. Lag counts only overwritten health/lifecycle entries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HealthRecvError {
    /// No new record yet.
    Empty,
    /// This many unread records were evicted. Cursor advances to the retained head;
    /// a later WorldChanged never hides this error. Consumers must fail closed.
    Lagged(u64),
    /// Terminal record was consumed, or subscription began after closure.
    Closed,
}
impl std::fmt::Display for HealthRecvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("health log is empty"),
            Self::Lagged(n) => write!(f, "{n} unread health changes were evicted"),
            Self::Closed => f.write_str("health log is closed"),
        }
    }
}
impl std::error::Error for HealthRecvError {}

/// Independent cursor over the connection's sole health log. Dropping it never
/// affects another reader or retention. Async receive is cancellation safe.
pub struct HealthSubscription {
    log: Arc<HealthLedger>,
    cursor: u64,
    baseline: Option<ReceivedHealth>,
}
impl HealthSubscription {
    /// Latest received HP in the current world at the atomic subscription cut.
    /// None before first HP, after a world boundary, or after disconnect. Use this
    /// with subsequent records instead of reading player_state then subscribing.
    pub fn baseline(&self) -> Option<ReceivedHealth> {
        self.baseline
    }
    /// Health cursor at the last read/admission cut, usable with health_changes_after.
    pub fn sequence(&self) -> u64 {
        self.cursor
    }
    /// Read one record in O(1); Empty does not advance the cursor.
    pub fn try_recv(&mut self) -> std::result::Result<HealthEvent, HealthRecvError> {
        let state = self.log.state.lock().expect("health log poisoned");
        if self.cursor < state.dropped_through {
            let lost = state.dropped_through - self.cursor;
            self.cursor = state.dropped_through;
            return Err(HealthRecvError::Lagged(lost));
        }
        let offset = (self.cursor - state.dropped_through) as usize;
        if let Some(change) = state.changes.get(offset) {
            self.cursor = change.sequence;
            return Ok(change.event);
        }
        Err(if state.closed {
            HealthRecvError::Closed
        } else {
            HealthRecvError::Empty
        })
    }
    /// Wait for a record or closure, without polling or forwarding any broadcast.
    pub async fn recv(&mut self) -> std::result::Result<HealthEvent, HealthRecvError> {
        loop {
            let log = self.log.clone();
            let notified = log.changed.notified();
            tokio::pin!(notified);
            // Register before checking the cursor, including the empty-to-wait race.
            notified.as_mut().enable();
            match self.try_recv() {
                Err(HealthRecvError::Empty) => notified.await,
                result => return result,
            }
        }
    }
    /// Establish a new atomic baseline/tail cut. Does not change this reader.
    /// Explicitly discards history for the NEW subscription, including prior gaps.
    pub fn resubscribe(&self) -> Self {
        self.log.subscribe()
    }
}

impl crate::Client {
    /// Read the health-only log after its own sequence (initially 0), on either
    /// version, also after disconnect. State means actual retention loss;
    /// InvalidInput means a future cursor. No gameplay events occupy this log.
    pub async fn health_changes_after(&self, after_sequence: u64) -> Result<HealthLog> {
        super::dispatch!(&self.adapter, a => super::adapter::EventOps::health_log(a).after(after_sequence))
    }
    /// Atomically capture latest received HP and the producer tail. Only later
    /// records are delivered. Supported by both adapters; sends no packets.
    pub fn subscribe_health(&self) -> Result<HealthSubscription> {
        super::dispatch!(&self.adapter, a => Ok(super::adapter::EventOps::health_log(a).subscribe()))
    }
}

#[derive(Default)]
struct State {
    changes: VecDeque<HealthChange>,
    sequence: u64,
    dropped_through: u64,
    closed: bool,
}
/// Sole receipt history; producers call it directly under their application gate.
#[derive(Default)]
pub(crate) struct HealthLedger {
    state: Mutex<State>,
    changed: Notify,
}
impl HealthLedger {
    pub(crate) fn record(&self, receive_sequence: Option<u64>, event: HealthEvent) -> bool {
        let mut state = self.state.lock().expect("health log poisoned");
        if state.closed {
            return false;
        }
        state.sequence += 1;
        let sequence = state.sequence;
        state.closed = matches!(event, HealthEvent::Disconnected);
        state.changes.push_back(HealthChange {
            sequence,
            receive_sequence,
            event,
        });
        if state.changes.len() > MAX_RETAINED_HEALTH_CHANGES {
            state.dropped_through = state
                .changes
                .pop_front()
                .expect("nonempty health log")
                .sequence;
        }
        drop(state);
        self.changed.notify_waiters();
        true
    }
    pub(crate) fn subscribe(self: &Arc<Self>) -> HealthSubscription {
        let state = self.state.lock().expect("health log poisoned");
        let baseline = state.changes.back().and_then(|r| match r.event {
            HealthEvent::Vitals(v) => Some(v),
            HealthEvent::WorldChanged(_) | HealthEvent::Disconnected => None,
        });
        HealthSubscription {
            log: self.clone(),
            cursor: state.sequence,
            baseline,
        }
    }
    pub(crate) fn after(&self, after_sequence: u64) -> Result<HealthLog> {
        let state = self.state.lock().expect("health log poisoned");
        if after_sequence < state.dropped_through {
            return Err(Error::new(
                ErrorKind::State,
                anyhow::anyhow!(
                    "{} unread health changes were evicted",
                    state.dropped_through - after_sequence
                ),
            ));
        }
        if after_sequence > state.sequence {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                anyhow::anyhow!("health cursor is ahead of the log"),
            ));
        }
        Ok(HealthLog {
            sequence: state.sequence,
            changes: state
                .changes
                .iter()
                .skip((after_sequence - state.dropped_through) as usize)
                .copied()
                .collect(),
            closed: state.closed,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    pub(crate) enum Packet {
        Health(f32),
        Noise,
        World,
        Close,
    }
    pub(crate) fn payload(hp: f32) -> Vec<u8> {
        let mut p = hp.to_be_bytes().to_vec();
        p.push(17);
        p.extend(3.25f32.to_be_bytes());
        p
    }
    pub(crate) fn vitals(event: HealthEvent) -> ReceivedHealth {
        match event {
            HealthEvent::Vitals(v) => v,
            _ => panic!("expected HP: {event:?}"),
        }
    }
    // Run against both adapters' actual ordered packet application entry points.
    pub(crate) async fn contract<F: std::future::Future<Output = ()>>(
        client: &crate::Client,
        mut apply: impl FnMut(Packet) -> F,
    ) {
        let mut first = client.subscribe_health().unwrap();
        assert!(first.baseline().is_none());
        apply(Packet::Health(20.)).await;
        let initial = vitals(first.recv().await.unwrap());
        assert!(initial.is_valid());
        assert_eq!(
            initial.session,
            client.connection_identity().await.unwrap().session
        );
        assert_eq!(
            client.player_state().await.unwrap().health.unwrap().value,
            initial.health
        );
        assert_eq!(initial.health.food, 17);
        assert_eq!(initial.health.saturation, 3.25);
        let mut paused = client.subscribe_health().unwrap();
        let mut independent = first.resubscribe();
        assert_eq!(paused.baseline(), Some(initial));
        assert_eq!(paused.try_recv(), Err(HealthRecvError::Empty));
        let cut = paused.sequence();
        let event_cut = client.events_after(0).await.unwrap().cursor;
        for hp in [20., 4., 20.] {
            apply(Packet::Health(hp)).await;
            for _ in 0..10_000 {
                apply(Packet::Noise).await;
            }
        }
        // A slow multi-second action never requires this subscription to progress.
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        assert_eq!(
            client.events_after(event_cut).await.unwrap_err().kind(),
            ErrorKind::State
        );
        let log = client.health_changes_after(cut).await.unwrap();
        assert_eq!(log.changes.len(), 3);
        let values: Vec<_> = log.changes.iter().map(|c| vitals(c.event)).collect();
        assert_eq!(
            values.iter().map(|v| v.health.health).collect::<Vec<_>>(),
            [20., 4., 20.]
        );
        assert!(
            values
                .windows(2)
                .all(|w| w[0].receive_sequence < w[1].receive_sequence)
        );
        assert!(values.iter().all(|v| v.session == initial.session));
        // Rejected capture: retain the cut; repeated reads preserve all evidence.
        assert_eq!(
            client.health_changes_after(cut).await.unwrap().changes,
            log.changes
        );
        let mut minimum = 20f32;
        for expected in &values {
            let actual = vitals(paused.try_recv().unwrap());
            assert_eq!(actual, *expected);
            minimum = minimum.min(actual.health.health);
        }
        assert_eq!(minimum, 4.);
        // Accepted cut acknowledges only this cursor; partial draining invents no gap.
        assert_eq!(paused.try_recv(), Err(HealthRecvError::Empty));
        assert_eq!(vitals(independent.try_recv().unwrap()).health.health, 20.);
        let fresh = independent.resubscribe();
        assert_eq!(fresh.baseline().unwrap().health.health, 20.);
        assert_eq!(vitals(independent.try_recv().unwrap()).health.health, 4.);
        assert_eq!(vitals(first.try_recv().unwrap()).health.health, 20.);
        assert_eq!(vitals(first.try_recv().unwrap()).health.health, 4.);
        for invalid in [
            f32::from_bits(0x7fc01234),
            f32::INFINITY,
            f32::NEG_INFINITY,
            -1.,
        ] {
            apply(Packet::Health(invalid)).await;
            let received = vitals(paused.recv().await.unwrap());
            assert_eq!(received.health.health.to_bits(), invalid.to_bits());
            assert!(!received.is_valid());
            assert_eq!(
                client
                    .subscribe_health()
                    .unwrap()
                    .baseline()
                    .unwrap()
                    .health
                    .health
                    .to_bits(),
                invalid.to_bits()
            );
        }
        apply(Packet::World).await;
        let HealthEvent::WorldChanged(new) = paused.recv().await.unwrap() else {
            panic!("world boundary")
        };
        assert_eq!(new.connection_id, initial.session.connection_id);
        assert_ne!(new.world_generation, initial.session.world_generation);
        assert!(client.subscribe_health().unwrap().baseline().is_none());
        apply(Packet::Health(10.)).await;
        assert_eq!(vitals(paused.recv().await.unwrap()).session, new);
        let mut slow = client.subscribe_health().unwrap();
        let slow_cut = slow.sequence();
        for _ in 0..MAX_RETAINED_HEALTH_CHANGES + 3 {
            apply(Packet::Health(20.)).await;
        }
        apply(Packet::World).await;
        apply(Packet::Health(8.)).await;
        apply(Packet::Close).await;
        assert_eq!(slow.try_recv(), Err(HealthRecvError::Lagged(6)));
        assert_eq!(
            client
                .health_changes_after(slow_cut)
                .await
                .unwrap_err()
                .kind(),
            ErrorKind::State
        );
        let mut worlds = 0;
        let mut last = None;
        while let Ok(event) = slow.try_recv() {
            if matches!(event, HealthEvent::WorldChanged(_)) {
                worlds += 1;
            }
            last = Some(event);
        }
        assert_eq!(worlds, 1);
        assert_eq!(last, Some(HealthEvent::Disconnected));
        assert_eq!(slow.recv().await, Err(HealthRecvError::Closed));
        let mut closed = client.subscribe_health().unwrap();
        assert!(closed.baseline().is_none());
        assert_eq!(closed.recv().await, Err(HealthRecvError::Closed));
        let end = client.health_changes_after(slow.sequence()).await.unwrap();
        assert!(end.closed && end.changes.is_empty());
        assert_eq!(
            client
                .health_changes_after(end.sequence + 1)
                .await
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidInput
        );
    }

    fn event(sequence: u64) -> HealthEvent {
        HealthEvent::Vitals(ReceivedHealth {
            session: SessionStamp {
                version: crate::MinecraftVersion::Java1_16_1,
                connection_id: 42,
                world_generation: 1,
            },
            receive_sequence: sequence,
            health: Health {
                health: 20.,
                food: 20,
                saturation: 5.,
            },
        })
    }
    #[tokio::test]
    async fn health_recv_wakes_all_and_cancel_does_not_consume() {
        let log = Arc::new(HealthLedger::default());
        let mut a = log.subscribe();
        let mut b = a.resubscribe();
        let mut pending = Box::pin(a.recv());
        std::future::poll_fn(|cx| {
            assert!(pending.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        drop(pending);
        let producer = async {
            tokio::task::yield_now().await;
            log.record(Some(1), event(1));
        };
        let (a, b, ()) = tokio::join!(a.recv(), b.recv(), producer);
        assert_eq!(a.unwrap(), event(1));
        assert_eq!(b.unwrap(), event(1));
        let mut waiting = log.subscribe();
        let (closed, ()) = tokio::join!(waiting.recv(), async {
            tokio::task::yield_now().await;
            log.record(None, HealthEvent::Disconnected);
        });
        assert_eq!(closed.unwrap(), HealthEvent::Disconnected);
        assert_eq!(waiting.recv().await, Err(HealthRecvError::Closed));
    }
    #[test]
    fn health_admission_is_atomic_with_publication() {
        let log = Arc::new(HealthLedger::default());
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let producer = {
            let log = log.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                for sequence in 1..=1000 {
                    barrier.wait();
                    log.record(Some(sequence), event(sequence));
                    barrier.wait();
                }
            })
        };
        for sequence in 1..=1000 {
            barrier.wait();
            let mut reader = log.subscribe();
            barrier.wait();
            let baseline = reader.baseline().map(|v| v.receive_sequence).unwrap_or(0);
            assert!(baseline == sequence || baseline == sequence - 1);
            if baseline == sequence {
                assert_eq!(reader.try_recv(), Err(HealthRecvError::Empty));
            } else {
                assert_eq!(
                    vitals(reader.try_recv().unwrap()).receive_sequence,
                    sequence
                );
            }
        }
        producer.join().unwrap();
    }
}
