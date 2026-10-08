//! Change notifications common to both versions, read with a cursor.
//!
//! An event says what changed and at which receive sequence; read the new state
//! through the observation APIs. Each connection keeps the latest events in a
//! bounded log, and reading behind that window is an error, so a gap is never
//! presented as a complete history. See `docs/event-stream-design.md`.
use crate::{Error, ErrorKind, Result};
use std::collections::VecDeque;
use std::time::Duration;

/// Maximum retained events per connection.
pub const MAX_RETAINED_EVENTS: usize = 4096;

/// What changed. New kinds may be added.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum EventKind {
    /// Block states changed inside these inclusive bounds (a superset for
    /// multi-block packets).
    BlocksChanged {
        /// Minimum x, y and z.
        min: [i32; 3],
        /// Maximum x, y and z.
        max: [i32; 3],
    },
    /// A chunk column arrived.
    ChunkLoaded {
        /// Chunk x.
        x: i32,
        /// Chunk z.
        z: i32,
    },
    /// A chunk column was unloaded.
    ChunkUnloaded {
        /// Chunk x.
        x: i32,
        /// Chunk z.
        z: i32,
    },
    /// Player inventory, cursor or selected hotbar slot.
    InventoryChanged,
    /// An open screen, its contents or properties.
    ScreenChanged,
    /// Own position, health, experience, abilities or game mode.
    PlayerChanged,
    /// Join, respawn, dimension change or reconfiguration.
    WorldChanged,
    /// An entity was spawned. Resolve it through `Client::entity_spawns`.
    EntitySpawned {
        /// Native entity id within the current world.
        native_id: i32,
    },
    /// An entity was removed.
    EntityRemoved {
        /// Native entity id within the current world.
        native_id: i32,
    },
    /// A chat message was received; read it with `Client::chat_after`.
    ChatReceived,
    /// Scoreboard, boss bar, team, player list, title, tab list or world border.
    UiChanged,
    /// The connection closed. No later event follows; the server's reason, when it
    /// sent one, is `Client::disconnect_reason`.
    Disconnected,
    /// Received motion, rotation, velocity, entity data or equipment of an entity.
    EntityUpdated {
        /// Native entity id within the current world.
        native_id: i32,
    },
    /// Received entity status (`ClientboundEntityEventPacket`); codes are version-specific.
    EntityStatus {
        /// Native entity id within the current world.
        native_id: i32,
        /// Native status code.
        status: i8,
    },
    /// An entity was hurt: 1.16.1 statuses 2, 33, 36, 37 and 44, or a 1.21.11 damage event.
    EntityDamaged {
        /// Native entity id within the current world.
        native_id: i32,
    },
    /// The own player died; the message is `Client::death_message`.
    PlayerKilled {
        /// Native entity id of the player.
        native_id: i32,
    },
}

/// One change notification.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ClientEvent {
    /// Receive sequence of the packet that caused it; the same axis as observations.
    pub receive_sequence: u64,
    /// Time from the creation of the connection's event log to recording this event
    /// (the packet's application), on the client's clock.
    pub received_after: Duration,
    /// What changed.
    pub kind: EventKind,
}

/// Events after a caller cursor.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EventLog {
    /// Pass this as the next cursor. It is the ordinal of the newest retained
    /// event, not a receive sequence.
    pub cursor: u64,
    /// Receive sequence through which this log is complete.
    pub receive_sequence: u64,
    /// Events after the requested cursor, oldest first.
    pub events: Vec<ClientEvent>,
}

/// Bounded per-connection event log. Ordinals are assigned per event so that
/// several events from one packet can be read separately.
pub(crate) struct EventLedger {
    events: VecDeque<(u64, ClientEvent)>,
    next: u64,
    dropped_through: u64,
    closed: bool,
    started: std::time::Instant,
}

impl Default for EventLedger {
    fn default() -> Self {
        Self {
            events: VecDeque::new(),
            next: 0,
            dropped_through: 0,
            closed: false,
            started: std::time::Instant::now(),
        }
    }
}

/// 1.16.1 entity statuses that mean "hurt" (LivingEntity.handleEntityEvent).
pub(crate) const LEGACY_HURT_STATUSES: [i8; 5] = [2, 33, 36, 37, 44];

impl EventLedger {
    pub(crate) fn record(&mut self, receive_sequence: u64, kind: EventKind) {
        if self.closed {
            return;
        }
        if kind == EventKind::Disconnected {
            self.closed = true;
        }
        self.next += 1;
        self.events.push_back((
            self.next,
            ClientEvent {
                receive_sequence,
                received_after: self.started.elapsed(),
                kind,
            },
        ));
        if self.events.len() > MAX_RETAINED_EVENTS {
            let (ordinal, _) = self.events.pop_front().expect("non-empty event log");
            self.dropped_through = ordinal;
        }
    }

    pub(crate) fn after(&self, cursor: u64, receive_sequence: u64) -> Result<EventLog> {
        if cursor < self.dropped_through {
            return Err(Error::new(
                ErrorKind::State,
                anyhow::anyhow!("events before the cursor were dropped; re-observe state"),
            ));
        }
        if cursor > self.next {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                anyhow::anyhow!("event cursor is ahead of the log"),
            ));
        }
        Ok(EventLog {
            cursor: self.next,
            receive_sequence,
            events: self
                .events
                .iter()
                .filter(|(ordinal, _)| *ordinal > cursor)
                .map(|(_, event)| *event)
                .collect(),
        })
    }

    pub(crate) fn closed(&self) -> bool {
        self.closed
    }
}

/// Bounds of a set of changed block positions.
pub(crate) fn bounds(positions: impl IntoIterator<Item = [i32; 3]>) -> Option<EventKind> {
    let mut positions = positions.into_iter();
    let first = positions.next()?;
    let (mut min, mut max) = (first, first);
    for p in positions {
        for axis in 0..3 {
            min[axis] = min[axis].min(p[axis]);
            max[axis] = max[axis].max(p[axis]);
        }
    }
    Some(EventKind::BlocksChanged { min, max })
}

impl super::Client {
    /// Last received death message of the own player (native JSON in 1.16.1, NBT in
    /// 1.21.11), kept until another one arrives. Readable after the connection closes.
    pub async fn death_message(&self) -> Result<Option<super::ObservedValue<super::ui::UiText>>> {
        crate::client::dispatch!(&self.adapter, a => crate::client::adapter::EventOps::death_message(a).await)
    }

    /// Text of the server's kick message, when the server closed the connection with
    /// one. Readable after the connection closes.
    pub async fn disconnect_reason(&self) -> Result<Option<super::ui::UiText>> {
        crate::client::dispatch!(&self.adapter, a => crate::client::adapter::EventOps::disconnect_reason(a).await)
    }

    /// Events after `cursor` (0 for everything retained). Readable after the
    /// connection closes. Fails if older events were dropped.
    pub async fn events_after(&self, cursor: u64) -> Result<EventLog> {
        super::dispatch!(&self.adapter, a => super::adapter::EventOps::events_after(a, cursor).await)
    }

    /// Wait until at least one event follows `cursor`. A closed connection
    /// still returns its final `Disconnected` event before failing.
    pub async fn wait_for_events(&self, cursor: u64, limit: Duration) -> Result<EventLog> {
        tokio::time::timeout(limit, async {
            loop {
                let log = self.events_after(cursor).await?;
                if !log.events.is_empty() {
                    return Ok(log);
                }
                self.wait_for_receive(log.receive_sequence, Duration::MAX)
                    .await?;
            }
        })
        .await
        .map_err(|_| {
            Error::new(
                ErrorKind::Timeout,
                anyhow::anyhow!("timed out waiting for events"),
            )
        })?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursors_read_each_event_once_and_report_dropped_history() {
        let mut ledger = EventLedger::default();
        ledger.record(5, EventKind::ChatReceived);
        ledger.record(5, EventKind::UiChanged);
        let first = ledger.after(0, 0).unwrap();
        assert_eq!(first.events.len(), 2);
        assert!(ledger.after(first.cursor, 0).unwrap().events.is_empty());
        assert!(ledger.after(first.cursor + 1, 0).is_err());
        for sequence in 0..MAX_RETAINED_EVENTS as u64 {
            ledger.record(6 + sequence, EventKind::InventoryChanged);
        }
        assert_eq!(ledger.after(0, 0).unwrap_err().kind(), ErrorKind::State);
        assert_eq!(
            ledger.after(2, 0).unwrap().events.len(),
            MAX_RETAINED_EVENTS
        );
    }

    #[test]
    fn nothing_is_recorded_after_disconnect() {
        let mut ledger = EventLedger::default();
        ledger.record(1, EventKind::Disconnected);
        ledger.record(2, EventKind::ChatReceived);
        let log = ledger.after(0, 0).unwrap();
        assert_eq!(log.events.len(), 1);
        assert!(ledger.closed());
    }

    #[test]
    fn bounds_cover_all_positions() {
        assert_eq!(
            bounds([[1, 5, -2], [-3, 7, 4], [0, 6, 0]]),
            Some(EventKind::BlocksChanged {
                min: [-3, 5, -2],
                max: [1, 7, 4]
            })
        );
        assert_eq!(bounds([]), None);
    }
}
