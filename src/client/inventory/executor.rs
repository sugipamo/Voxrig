//! One execution right, independent of strict prediction/effect diagnostics.
//! Untagged contents update current state; only a separate native response can
//! discharge response debt. Full resync establishes an ordered state boundary.
use super::{InventorySource, unavailable};
use crate::Result;
use crate::client::SessionStamp;
use tokio::time::{Duration, Instant};

// Reservation/write admission must remain possible even when callers request
// an immediate post-write receipt timeout. Bound pre-write waiting separately.
const MIN_RESERVATION_TIMEOUT: Duration = Duration::from_secs(5);

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
    /// Original sender was destroyed before writing. No later write from it.
    NotSent,
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
/// Why authoritative inventory completion still needs inspection.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InventorySendInspectionReason {
    /// The original response group did not arrive within its receipt budget.
    ReceiptDeadline,
    /// The bounded no-effect probe also failed to establish a complete group.
    ReceiptDeadlineAfterResync,
    /// The no-effect probe's complete delivery could not be established.
    ResyncDeliveryUncertain,
    /// The original scope or delivery authority was invalidated.
    ScopeOrDeliveryChanged,
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
    /// Final receive boundary before the additional no-effect deadline probe.
    pub deadline_resync_after_sequence: Option<u64>,
    /// The probe's native action; the original action remains above.
    pub deadline_resync_action: Option<i16>,
    /// Fresh modern fence token for the probe, distinct from the original send.
    pub deadline_resync_token: Option<u64>,
    /// Complete no-effect probe write; never a replay of the original click.
    pub deadline_resync_dispatched: bool,
    /// Typed uncertainty; no divergent effect is inferred from missing receipts.
    pub inspection_reason: Option<InventorySendInspectionReason>,
}
impl InventorySendRecord {
    pub(crate) fn completion_after(&self) -> u64 {
        self.deadline_resync_after_sequence
            .unwrap_or(self.after_sequence)
    }
    pub(crate) fn completion_action(&self) -> Option<i16> {
        self.deadline_resync_action.or(self.legacy_action)
    }
    fn clear_completion(&mut self) {
        self.response_sequence = None;
        self.contents_sequence = None;
        self.cursor_sequence = None;
    }
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
    pub(crate) operational: Option<crate::client::crafting::operational::Owner>,
    pub(crate) changes: super::changes::InventoryChangeLedger,
    pub(super) current: Option<InventorySendRecord>,
    pub(super) inspection_send: Option<InventorySendRecord>,
    pub(super) inspected: Option<Box<super::InspectedCraftingRecovery>>,
    deadline: Option<Instant>,
    comparison: Option<super::outcome::Comparison>,
    pub(super) divergence: Option<super::InventoryDivergence>,
    pub(super) inspection_divergence: Option<super::InventoryDivergence>,
    ready: bool,
    timed_out: bool,
    own_actions: std::sync::Arc<super::own_action::OwnActionSink>,
}
impl Executor {
    pub(crate) fn operational_access(
        &mut self,
        id: InventorySendId,
        release: bool,
        session: SessionStamp,
        source: InventorySource,
        current: &super::InventorySnapshot,
        player: &crate::client::PlayerObservation,
    ) -> Result<crate::client::crafting::OperationalCraftingOutcome> {
        self.expire();
        if id.session() != session || id.source() != source {
            return Err(unavailable("operational crafting scope retired"));
        }
        let send = self
            .current
            .as_ref()
            .filter(|r| r.id == id)
            .ok_or_else(|| unavailable("operational crafting send superseded"))?;
        let owner = self
            .operational
            .as_mut()
            .filter(|o| o.outcome.send == id)
            .ok_or_else(|| unavailable("send has no operational crafting owner"))?;
        owner.player(player);
        owner.receive(send, current, self.ready);
        owner.claimed = true;
        if release {
            owner.release(current)?;
        }
        Ok(owner.outcome.clone())
    }

    /// Handoff the socket writer records own action frames into.
    pub(crate) fn own_action_sink(&self) -> std::sync::Arc<super::own_action::OwnActionSink> {
        self.own_actions.clone()
    }
    /// Open own-action windows written since the last call. Call on every
    /// received packet, before inventory deltas; `selected` is the current
    /// selection (received or submitted) and resolves the main-hand slot.
    pub(crate) fn own_action_pending(&self) -> bool {
        !self.own_actions.is_empty()
    }
    pub(crate) fn own_actions_received(&mut self, session: SessionStamp, selected: Option<u8>) {
        if !self.own_action_pending() {
            return;
        }
        let windows = self
            .own_actions
            .drain(session, selected, self.changes.own_ordinal());
        self.changes.own_actions(windows);
    }
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
    /// `(session, after_sequence)` of the current ordinary send when its full
    /// boundary ended `Diverged` with no observed effect. Strict records bound
    /// to the same pre-send boundary use it as their terminal unchanged proof.
    pub(crate) fn unchanged_boundary(&self) -> Option<(SessionStamp, u64)> {
        let record = self
            .current
            .as_ref()
            .filter(|r| r.status == InventorySendStatus::Diverged && !r.resync)?;
        self.divergence
            .as_ref()
            .filter(|d| {
                d.send == record.id
                    && matches!(d.intended, super::InventoryIntendedChange::Exact { .. })
                    && d.effect() == super::InventoryEffect::NoEffect
            })
            .map(|_| (record.id.session, record.after_sequence))
    }
    pub(crate) fn busy(&self) -> bool {
        self.current.as_ref().is_some_and(|r| {
            !matches!(
                r.status,
                InventorySendStatus::Received
                    | InventorySendStatus::Diverged
                    | InventorySendStatus::NotSent
            )
        })
    }
    pub(crate) fn owns(&self, session: SessionStamp, after: u64, action: Option<i16>) -> bool {
        self.current.as_ref().is_some_and(|r| {
            !matches!(
                r.status,
                InventorySendStatus::Received
                    | InventorySendStatus::Diverged
                    | InventorySendStatus::NotSent
            ) && r.id.session == session
                && r.after_sequence == after
                && r.legacy_action == action
        })
    }
    pub(crate) fn admit(&mut self) -> Result<()> {
        self.expire();
        if self
            .operational
            .as_ref()
            .is_some_and(|o| o.claimed && !o.released)
        {
            return Err(unavailable(
                "operational crafting owns inventory; inspect or explicitly continue",
            ));
        }
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
        if resync {
            if let Some(send) = self.current.as_ref().filter(|r| {
                self.operational
                    .as_ref()
                    .is_some_and(|o| o.outcome.send == r.id && !o.released)
            }) {
                self.inspection_send = Some(send.clone());
                self.inspection_divergence = self.divergence.clone();
            }
            if let Some(owner) = &mut self.operational {
                owner.fail("operational crafting required resynchronization");
            }
        }
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
            deadline_resync_after_sequence: None,
            deadline_resync_action: None,
            deadline_resync_token: None,
            deadline_resync_dispatched: false,
            inspection_reason: None,
        });
        // Reservations are finite too; a complete write starts a fresh receipt
        // timeout via written(). No Pending record may lose its deadline.
        self.deadline = Some(Instant::now() + timeout.max(MIN_RESERVATION_TIMEOUT));
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
        r.clear_completion();
        self.ready = false;
        self.deadline = Some(Instant::now() + timeout.max(MIN_RESERVATION_TIMEOUT));
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
        r.clear_completion();
        self.ready = false;
        Ok(())
    }
    // Only the original modern caller-owned attempt may supply this proof.
    pub(crate) fn not_sent(&mut self, id: InventorySendId) {
        if let Some(record) = self
            .current
            .as_mut()
            .filter(|r| r.id == id && !r.dispatched)
        {
            record.status = InventorySendStatus::NotSent;
            self.deadline = None;
            self.comparison = None;
            self.divergence = None;
            self.ready = false;
            self.timed_out = false;
        }
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
                self.timed_out = true;
                // Exact acceptance retains Pending throughout the first expiry
                // and bounded probe write. Pollers must not see a terminal gap
                // before the owned no-effect recovery has had its budget.
                if r.dispatched
                    && !r.resync
                    && r.deadline_resync_after_sequence.is_none()
                    && self.comparison.as_ref().is_some_and(|c| !c.crafting)
                {
                    return;
                }
                r.status = InventorySendStatus::ResyncRequired;
                r.inspection_reason = Some(if r.deadline_resync_after_sequence.is_some() {
                    InventorySendInspectionReason::ReceiptDeadlineAfterResync
                } else {
                    InventorySendInspectionReason::ReceiptDeadline
                });
                self.timed_out = true;
            }
        }
    }
    pub(crate) fn invalidate(&mut self) {
        if let Some(owner) = &mut self.operational {
            owner.fail("operational crafting scope or delivery invalidated");
        }
        if let Some(r) = self
            .current
            .as_mut()
            .filter(|r| r.status == InventorySendStatus::Pending)
        {
            r.status = InventorySendStatus::ResyncRequired;
            r.inspection_reason = Some(InventorySendInspectionReason::ScopeOrDeliveryChanged);
        }
    }
    /// Only exact acceptance operations use this owner. Strict crafting retains
    /// its separate #307 deadline and inspection contract.
    pub(crate) fn completion_waiting(&self, id: InventorySendId) -> bool {
        self.current
            .as_ref()
            .is_some_and(|r| r.id == id && !r.resync && r.status == InventorySendStatus::Pending)
            && self.comparison.as_ref().is_some_and(|c| !c.crafting)
    }
    pub(crate) fn recovery_needed(&mut self, id: InventorySendId) -> bool {
        self.expire();
        self.current.as_ref().is_some_and(|r| {
            r.id == id
                && r.dispatched
                && !r.resync
                && r.deadline_resync_after_sequence.is_none()
                && r.status == InventorySendStatus::Pending
                && self.timed_out
        }) && self.comparison.as_ref().is_some_and(|c| !c.crafting)
    }
    pub(crate) fn recovery_fence_token(&mut self, id: InventorySendId) -> Result<u64> {
        if !self.recovery_needed(id) {
            return Err(unavailable("inventory completion recovery superseded"));
        }
        self.next = self
            .next
            .checked_add(1)
            .filter(|v| *v <= i32::MAX as u64)
            .ok_or_else(|| unavailable("inventory fence identities exhausted"))?;
        self.current
            .as_mut()
            .expect("owned recovery")
            .deadline_resync_token = Some(self.next);
        Ok(self.next)
    }
    pub(crate) fn recovery_written(
        &mut self,
        id: InventorySendId,
        after: u64,
        action: Option<i16>,
        timeout: Duration,
    ) {
        if let Some(r) = self.current.as_mut().filter(|r| r.id == id) {
            r.deadline_resync_after_sequence = Some(after);
            r.deadline_resync_action = action;
            r.deadline_resync_dispatched = true;
            r.clear_completion();
            r.status = InventorySendStatus::Pending;
            r.inspection_reason = None;
            self.ready = false;
            self.deadline = Some(Instant::now() + timeout);
        }
    }
    pub(crate) fn recovery_failed(&mut self, id: InventorySendId) {
        if let Some(r) = self
            .current
            .as_mut()
            .filter(|r| r.id == id && r.status == InventorySendStatus::Pending)
        {
            r.status = InventorySendStatus::ResyncRequired;
            r.inspection_reason = Some(InventorySendInspectionReason::ResyncDeliveryUncertain);
            self.deadline = None;
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
                && r.dispatched
                && sequence > r.completion_after()
        }) {
            f(r);
            let complete = match (r.response_sequence, r.contents_sequence, r.cursor_sequence) {
                (Some(response), Some(contents), Some(cursor)) => {
                    if r.id.session.version == crate::MinecraftVersion::Java1_16_1 {
                        r.completion_after() < response && response < contents && contents < cursor
                    } else {
                        // Modern full contents carries its own cursor atomically.
                        r.completion_after() < contents
                            && contents == cursor
                            && r.completion_after() < response
                    }
                }
                _ => false,
            };
            if complete {
                self.ready = true;
                if self.comparison.is_none() {
                    r.status = InventorySendStatus::Received;
                    self.deadline = None;
                }
                // With an intention, only observe() can finish the comparison
                // at the applied packet boundary. Keep its timeout until then.
            }
        }
    }
    pub(crate) fn comparison(&mut self, mut comparison: super::outcome::Comparison) {
        if self
            .current
            .as_ref()
            .is_some_and(|r| r.id == comparison.send && !r.dispatched)
        {
            self.operational = comparison.operational.take();
            self.inspection_send = None;
            self.inspection_divergence = None;
            self.comparison = Some(comparison);
        }
    }
    #[cfg(test)]
    pub(crate) fn hint_for_test(&mut self, cause: super::InventoryCause) {
        if let Some(c) = &mut self.comparison {
            c.hint(cause);
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
        if let (Some(owner), Some(send)) = (&mut self.operational, &self.current) {
            owner.receive(send, &current, self.ready);
        }
        if self.current.as_ref().is_some_and(|r| {
            r.status == InventorySendStatus::Pending
                && current.receive_sequence > r.after_sequence
                && (r.id.session != current.session || r.id.source != current.source)
        }) {
            // This snapshot is from a new world/opening or local close scope.
            // It cannot discharge the old response debt or compare its intent.
            self.invalidate();
            return;
        }
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
            self.deadline = None;
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
            if r.completion_action() == Some(action) {
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
            if r.completion_action().is_none()
                && r.deadline_resync_token.unwrap_or(r.id.number) == token
            {
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
    #[tokio::test(start_paused = true)]
    async fn unwritten_reservation_and_bound_legacy_action_are_finite() {
        let s = session(MinecraftVersion::Java1_16_1);
        for bound in [false, true] {
            let mut e = Executor::default();
            let id = e
                .begin(
                    s,
                    InventorySource::Player,
                    10,
                    None,
                    Duration::from_secs(1),
                    false,
                )
                .unwrap();
            if bound {
                assert_eq!(
                    e.bind_legacy_action(s, InventorySource::Player, 1, 10, Duration::from_secs(1)),
                    Some(id)
                );
            }
            tokio::time::advance(MIN_RESERVATION_TIMEOUT).await;
            assert_eq!(
                e.record().unwrap().status,
                InventorySendStatus::ResyncRequired
            );
            assert!(!e.record().unwrap().dispatched);
        }
    }
    #[tokio::test(start_paused = true)]
    async fn zero_receipt_timeout_still_allows_one_complete_write() {
        let s = session(MinecraftVersion::Java1_16_1);
        let mut e = Executor::default();
        let id = e
            .begin(s, InventorySource::Player, 10, None, Duration::ZERO, false)
            .unwrap();
        assert_eq!(e.record().unwrap().status, InventorySendStatus::Pending);
        assert_eq!(
            e.bind_legacy_action(s, InventorySource::Player, 1, 10, Duration::ZERO),
            Some(id)
        );
        e.written(id, Duration::ZERO);
        let record = e.record().unwrap();
        assert!(record.dispatched);
        assert_eq!(record.status, InventorySendStatus::ResyncRequired);
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
    async fn completion_requires_version_order_and_post_write_receipts() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
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
            e.contents(s, src, 0, 11, !legacy);
            e.cursor(s, src, 12);
            assert!(e.record().unwrap().contents_sequence.is_none());
            e.refresh_boundary(id, 12).unwrap();
            e.written(id, Duration::from_secs(1));
            if legacy {
                e.cursor(s, src, 13);
                e.response(s, src, 5, 14);
                e.contents(s, src, 0, 15, false);
                assert_eq!(e.record().unwrap().status, InventorySendStatus::Pending);
                e.cursor(s, src, 16);
            } else {
                e.fence(s, src, id.number, 13);
                e.contents(s, src, 0, 14, true);
                assert_eq!(e.record().unwrap().status, InventorySendStatus::Received);
            }
            assert_eq!(e.record().unwrap().status, InventorySendStatus::Received);
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
        assert!(e.busy());
        e.contents(s, src, 0, 25, false);
        e.cursor(s, src, 26);
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
        assert!(e.busy());
        e.contents(s, src, 0, 31, false);
        e.cursor(s, src, 32);
        assert!(!e.busy());
    }
}
