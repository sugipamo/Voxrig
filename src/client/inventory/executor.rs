//! One execution right, independent of strict prediction/effect diagnostics.
//! Untagged contents update current state; only a separate native response can
//! discharge response debt. Full resync establishes an ordered state boundary.
use super::{InventorySource, unavailable};
use crate::Result;
use crate::client::SessionStamp;
use tokio::time::{Duration, Instant};

/// Identity of a single send. Never reusable across a connection/world/opening.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct InventorySendId {
    pub(crate) session: SessionStamp,
    pub(crate) source: InventorySource,
    pub(crate) number: u64,
}
impl InventorySendId {
    /// Owning connection and world.
    pub fn session(self) -> SessionStamp {
        self.session
    }
    /// Opening-bound source.
    pub fn source(self) -> InventorySource {
        self.source
    }
    /// Monotonic connection-local send number (also the modern suggestion fence token).
    pub fn number(self) -> u64 {
        self.number
    }
}
/// Receive progress and effect comparison for an inventory send.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InventorySendStatus {
    /// Reserved or sent; native response and full received state are still required.
    Pending,
    /// Native response/full state arrived and the declared intended effect agrees.
    Received,
    /// Native response/full state arrived, but the plan must use current receipts.
    /// Read `Client::inventory_divergence` for comparison and cause evidence.
    Diverged,
    /// Deadline or opening change; stop clicks and request resynchronization.
    ResyncRequired,
}
/// Acceptance receipt, separate from the strict click/transfer/crafting record.
#[derive(Clone, Debug, serde::Serialize)]
pub struct InventorySendRecord {
    /// Original send identity.
    pub id: InventorySendId,
    /// Native 1.16.1 action number; absent on modern.
    pub legacy_action: Option<i16>,
    /// Receive boundary before the complete write.
    pub after_sequence: u64,
    /// Complete original frame write; never a server acceptance claim.
    pub dispatched: bool,
    /// Response debt, never inferred from a slot packet.
    pub response_sequence: Option<u64>,
    /// Actual full contents receipt in this opening.
    pub contents_sequence: Option<u64>,
    /// Actual cursor receipt, independent of ACK/prediction.
    pub cursor_sequence: Option<u64>,
    /// Receive progress and declared effect comparison.
    pub status: InventorySendStatus,
    /// This is an explicit, no-effect resync probe, never a replay of the old intent.
    pub resync: bool,
}
impl InventorySendRecord {
    pub(crate) fn window(&self) -> i32 {
        match self.id.source {
            InventorySource::Player => 0,
            InventorySource::Container { screen } => screen.window_id(),
        }
    }
}
#[derive(Default)]
pub(crate) struct Executor {
    next: u64,
    pub(crate) changes: super::changes::InventoryChangeLedger,
    current: Option<InventorySendRecord>,
    deadline: Option<Instant>,
    comparison: Option<super::outcome::Comparison>,
    divergence: Option<super::InventoryDivergence>,
    ready: bool,
    timed_out: bool,
}
impl Executor {
    pub(crate) fn record(&mut self) -> Option<InventorySendRecord> {
        self.expire();
        self.current.clone()
    }
    pub(crate) fn divergence(
        &self,
        id: InventorySendId,
    ) -> Result<Option<super::InventoryDivergence>> {
        if !self.current.as_ref().is_some_and(|r| r.id == id)
            && !self.divergence.as_ref().is_some_and(|d| d.send == id)
        {
            return Err(unavailable(
                "inventory send was superseded; retain divergence before the next send",
            ));
        }
        Ok(self.divergence.clone())
    }
    pub(crate) fn busy(&self) -> bool {
        self.current.as_ref().is_some_and(|r| {
            !matches!(
                r.status,
                InventorySendStatus::Received | InventorySendStatus::Diverged
            )
        })
    }
    pub(crate) fn owns(&self, session: SessionStamp, after: u64, action: Option<i16>) -> bool {
        self.current.as_ref().is_some_and(|r| {
            !matches!(
                r.status,
                InventorySendStatus::Received | InventorySendStatus::Diverged
            ) && r.id.session == session
                && r.after_sequence == after
                && r.legacy_action == action
        })
    }
    pub(crate) fn admit(&mut self) -> Result<()> {
        self.expire();
        if self.busy() {
            return Err(unavailable(
                "inventory executor awaiting response/state; resync after timeout",
            ));
        }
        Ok(())
    }
    pub(crate) fn begin(
        &mut self,
        session: SessionStamp,
        source: InventorySource,
        after: u64,
        action: Option<i16>,
        timeout: Duration,
        resync: bool,
    ) -> Result<InventorySendId> {
        self.expire();
        if !resync {
            self.admit()?;
        } else if self
            .current
            .as_ref()
            .is_some_and(|r| r.status == InventorySendStatus::Pending)
        {
            return Err(unavailable(
                "inventory response still pending; wait for deadline before resync",
            ));
        }
        self.next = self
            .next
            .checked_add(1)
            .filter(|v| *v <= i32::MAX as u64)
            .ok_or_else(|| unavailable("inventory send identities exhausted"))?;
        let id = InventorySendId {
            session,
            source,
            number: self.next,
        };
        let inherited = if resync {
            self.comparison
                .take()
                .filter(|c| c.before.session == session && c.before.source == source)
                .map(|mut c| {
                    c.interrupted = true;
                    c.hint(if self.timed_out {
                        super::InventoryCause::TimedOutResync
                    } else {
                        super::InventoryCause::External
                    });
                    c
                })
        } else {
            None
        };
        self.comparison = inherited;
        self.divergence = None;
        self.ready = false;
        self.timed_out = false;
        self.current = Some(InventorySendRecord {
            id,
            legacy_action: action,
            after_sequence: after,
            dispatched: false,
            response_sequence: None,
            contents_sequence: None,
            cursor_sequence: None,
            status: InventorySendStatus::Pending,
            resync,
        });
        self.deadline = None;
        let _ = timeout; // receipt timeout starts at complete write, via written()
        Ok(id)
    }
    pub(crate) fn bind_legacy_action(
        &mut self,
        session: SessionStamp,
        source: InventorySource,
        action: i16,
        after: u64,
        timeout: Duration,
    ) -> Option<InventorySendId> {
        let r = self.current.as_mut().filter(|r| {
            r.status == InventorySendStatus::Pending
                && r.id.session == session
                && r.id.source == source
                && r.legacy_action.is_none()
        })?;
        r.legacy_action = Some(action);
        r.after_sequence = after;
        self.deadline = None;
        let _ = timeout;
        Some(r.id)
    }
    pub(crate) fn cancel_unsent(&mut self, session: SessionStamp, after: u64) {
        if !self
            .current
            .as_ref()
            .is_some_and(|r| r.id.session == session && r.after_sequence == after)
        {
            return;
        }
        if self
            .current
            .as_ref()
            .is_some_and(|r| r.legacy_action.is_none())
        {
            self.current = None;
            self.deadline = None;
        } else {
            self.invalidate();
        }
    }
    pub(crate) fn refresh_boundary(&mut self, id: InventorySendId, after: u64) -> Result<()> {
        let r = self
            .current
            .as_mut()
            .filter(|r| r.id == id && r.status == InventorySendStatus::Pending && !r.dispatched)
            .ok_or_else(|| unavailable("inventory reservation superseded before send"))?;
        r.after_sequence = after;
        Ok(())
    }
    pub(crate) fn invalidate_owned(&mut self, id: InventorySendId) {
        if self.current.as_ref().is_some_and(|r| r.id == id) {
            self.invalidate();
        }
    }
    pub(crate) fn written(&mut self, id: InventorySendId, timeout: Duration) {
        if let Some(r) = self
            .current
            .as_mut()
            .filter(|r| r.id == id && r.status == InventorySendStatus::Pending)
        {
            r.dispatched = true;
            self.deadline = Some(Instant::now() + timeout);
        }
    }
    pub(crate) fn expire(&mut self) {
        if self.deadline.is_some_and(|d| Instant::now() >= d) {
            if let Some(r) = self
                .current
                .as_mut()
                .filter(|r| r.status == InventorySendStatus::Pending)
            {
                r.status = InventorySendStatus::ResyncRequired;
                self.timed_out = true;
            }
        }
    }
    pub(crate) fn invalidate(&mut self) {
        if let Some(r) = self
            .current
            .as_mut()
            .filter(|r| r.status == InventorySendStatus::Pending)
        {
            r.status = InventorySendStatus::ResyncRequired;
        }
    }
    fn update(
        &mut self,
        session: SessionStamp,
        source: InventorySource,
        sequence: u64,
        f: impl FnOnce(&mut InventorySendRecord),
    ) {
        self.expire();
        if let Some(r) = self.current.as_mut().filter(|r| {
            r.status == InventorySendStatus::Pending
                && r.id.session == session
                && r.id.source == source
                && sequence > r.after_sequence
        }) {
            f(r);
            if r.dispatched
                && r.response_sequence.is_some()
                && r.contents_sequence.is_some()
                && r.cursor_sequence.is_some()
            {
                self.ready = true;
                if self.comparison.is_none() {
                    r.status = InventorySendStatus::Received;
                }
                self.deadline = None;
            }
        }
    }
    pub(crate) fn comparison(&mut self, comparison: super::outcome::Comparison) {
        if self
            .current
            .as_ref()
            .is_some_and(|r| r.id == comparison.send && !r.dispatched)
        {
            self.comparison = Some(comparison);
        }
    }
    pub(crate) fn rejected(&mut self) {
        if let Some(c) = &mut self.comparison {
            c.hint(super::InventoryCause::Rejected);
        }
    }
    pub(crate) fn collected(
        &mut self,
        session: SessionStamp,
        source: InventorySource,
        entity: i32,
    ) {
        if self.current.as_ref().is_some_and(|r| {
            r.id.session == session
                && r.id.source == source
                && r.dispatched
                && matches!(
                    r.status,
                    InventorySendStatus::Pending | InventorySendStatus::ResyncRequired
                )
        }) {
            if let Some(c) = &mut self.comparison {
                c.interrupted = true;
                if let Some(super::InventoryCause::Collected { entity_ids }) = c
                    .causes
                    .iter_mut()
                    .find(|h| matches!(h, super::InventoryCause::Collected { .. }))
                {
                    if !entity_ids.contains(&entity)
                        && entity_ids.len() < super::MAX_RETAINED_INVENTORY_CHANGES
                    {
                        entity_ids.push(entity);
                    }
                } else {
                    c.hint(super::InventoryCause::Collected {
                        entity_ids: vec![entity],
                    });
                }
            }
        }
    }
    pub(crate) fn active_send(&self) -> Option<InventorySendId> {
        self.current
            .as_ref()
            .filter(|r| {
                r.dispatched
                    && matches!(
                        r.status,
                        InventorySendStatus::Pending | InventorySendStatus::ResyncRequired
                    )
            })
            .map(|r| r.id)
    }
    pub(crate) fn receive(&mut self, current: super::InventorySnapshot, crafting_complete: bool) {
        let send = self
            .active_send()
            .filter(|id| id.session() == current.session && id.source() == current.source);
        self.observe(current.clone(), crafting_complete);
        let hints = if send.is_some() {
            self.comparison
                .as_ref()
                .map(|c| c.causes.as_slice())
                .or_else(|| self.divergence.as_ref().map(|d| d.causes.as_slice()))
                .unwrap_or_default()
        } else {
            &[]
        };
        self.changes.receive(current, send, hints);
    }
    pub(crate) fn observe(&mut self, current: super::InventorySnapshot, crafting_complete: bool) {
        self.expire();
        let Some(r) = self.current.as_mut().filter(|r| {
            r.dispatched
                && r.status == InventorySendStatus::Pending
                && r.id.session == current.session
                && r.id.source == current.source
        }) else {
            return;
        };
        let Some(comparison) = &mut self.comparison else {
            return;
        };
        let agrees = comparison.inspect(&current, crafting_complete, self.ready);
        if self.ready {
            if agrees {
                r.status = InventorySendStatus::Received;
            } else {
                r.status = InventorySendStatus::Diverged;
                self.divergence = Some(comparison.divergence(current));
            }
            self.comparison = None;
        }
    }
    pub(crate) fn response(
        &mut self,
        session: SessionStamp,
        source: InventorySource,
        action: i16,
        sequence: u64,
    ) {
        self.update(session, source, sequence, |r| {
            if r.legacy_action == Some(action) {
                r.response_sequence = Some(sequence);
            }
        });
    }
    pub(crate) fn fence(
        &mut self,
        session: SessionStamp,
        source: InventorySource,
        token: u64,
        sequence: u64,
    ) {
        self.update(session, source, sequence, |r| {
            if r.legacy_action.is_none() && r.id.number == token {
                r.response_sequence = Some(sequence);
            }
        });
    }
    pub(crate) fn contents(
        &mut self,
        session: SessionStamp,
        source: InventorySource,
        window: i32,
        sequence: u64,
        includes_cursor: bool,
    ) {
        self.update(session, source, sequence, |r| {
            if r.window() == window {
                r.contents_sequence = Some(sequence);
                if includes_cursor {
                    r.cursor_sequence = Some(sequence);
                }
            }
        });
    }
    pub(crate) fn cursor(&mut self, session: SessionStamp, source: InventorySource, sequence: u64) {
        self.update(session, source, sequence, |r| {
            r.cursor_sequence = Some(sequence)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MinecraftVersion;
    fn session(version: MinecraftVersion) -> SessionStamp {
        SessionStamp {
            version,
            connection_id: 42,
            world_generation: 7,
        }
    }
    #[test]
    fn stale_worker_cannot_rebind_or_invalidate_replacement_reservation() {
        let s = session(MinecraftVersion::Java1_21_11);
        let mut e = Executor::default();
        let old = e
            .begin(
                s,
                InventorySource::Player,
                10,
                None,
                Duration::from_secs(5),
                false,
            )
            .unwrap();
        e.invalidate_owned(old);
        let new = e
            .begin(
                s,
                InventorySource::Player,
                20,
                None,
                Duration::from_secs(5),
                true,
            )
            .unwrap();
        assert!(e.refresh_boundary(old, 21).is_err());
        e.invalidate_owned(old);
        e.written(old, Duration::from_secs(5));
        let current = e.record().unwrap();
        assert_eq!(current.id, new);
        assert_eq!(current.after_sequence, 20);
        assert!(!current.dispatched);
        assert_eq!(current.status, InventorySendStatus::Pending);
        e.refresh_boundary(new, 22).unwrap();
        e.written(new, Duration::from_secs(5));
        assert!(e.refresh_boundary(new, 23).is_err());
    }
    #[tokio::test(start_paused = true)]
    async fn response_and_received_state_are_independent_in_both_orders() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            for response_first in [false, true] {
                let mut e = Executor::default();
                let s = session(version);
                let src = InventorySource::Player;
                let legacy = version == MinecraftVersion::Java1_16_1;
                let id = e
                    .begin(
                        s,
                        src,
                        10,
                        legacy.then_some(5),
                        Duration::from_secs(1),
                        false,
                    )
                    .unwrap();
                let sent = e.current.as_ref().unwrap().id;
                e.written(sent, Duration::from_secs(1));
                let response = |e: &mut Executor| {
                    if legacy {
                        e.response(s, src, 5, 12);
                    } else {
                        e.fence(s, src, id.number, 12);
                    }
                };
                if response_first {
                    response(&mut e);
                    assert!(e.busy());
                }
                e.contents(s, src, 0, 11, !legacy);
                if legacy || !response_first {
                    assert!(e.busy());
                }
                if legacy {
                    e.cursor(s, src, 13);
                }
                if !response_first {
                    assert!(e.busy());
                    response(&mut e);
                }
                assert_eq!(e.record().unwrap().status, InventorySendStatus::Received);
            }
        }
    }
    #[tokio::test(start_paused = true)]
    async fn missing_ack_stops_clicks_and_late_ack_cannot_finish_resync_or_next_send() {
        let s = session(MinecraftVersion::Java1_16_1);
        let src = InventorySource::Player;
        let mut e = Executor::default();
        e.begin(s, src, 10, Some(5), Duration::from_secs(1), false)
            .unwrap();
        let sent = e.current.as_ref().unwrap().id;
        e.written(sent, Duration::from_secs(1));
        e.contents(s, src, 0, 11, false);
        e.cursor(s, src, 12);
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(e.admit().is_err());
        assert_eq!(
            e.record().unwrap().status,
            InventorySendStatus::ResyncRequired
        );
        e.begin(s, src, 20, Some(6), Duration::from_secs(1), true)
            .unwrap();
        let sent = e.current.as_ref().unwrap().id;
        e.written(sent, Duration::from_secs(1));
        e.response(s, src, 5, 21);
        e.contents(s, src, 0, 22, false);
        e.cursor(s, src, 23);
        assert!(e.busy());
        // accepted=false is also a protocol response; contents still required.
        e.response(s, src, 6, 24);
        assert!(!e.busy());
        e.begin(s, src, 25, Some(7), Duration::from_secs(1), false)
            .unwrap();
        let sent = e.current.as_ref().unwrap().id;
        e.written(sent, Duration::from_secs(1));
        e.response(s, src, 5, 26);
        e.contents(s, src, 0, 27, false);
        e.cursor(s, src, 28);
        assert!(e.busy());
        let mut other = s;
        other.world_generation += 1;
        e.response(other, src, 7, 29);
        assert!(e.busy());
        e.response(s, src, 7, 30);
        assert!(!e.busy());
    }
}
