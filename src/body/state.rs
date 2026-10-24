use super::ActionStatus;
use std::time::Duration;
use tokio::time::Instant;

/// Pure bounded state machine. Evidence collection belongs to the backend.
pub(super) struct Acceptance {
    pub status: ActionStatus,
    deadline: Instant,
    quiet: Duration,
    sent: Option<Instant>,
    needs_ack: bool,
    ack: Option<Instant>,
}
impl Acceptance {
    pub fn new(deadline: Instant, quiet: Duration, needs_ack: bool) -> Self {
        Self {
            status: ActionStatus::Pending,
            deadline,
            quiet,
            sent: None,
            needs_ack,
            ack: None,
        }
    }
    pub fn sent(&mut self, now: Instant) {
        self.sent = Some(now);
    }
    pub fn ack(&mut self, now: Instant) {
        self.ack.get_or_insert(now);
    }
    pub fn finish(&mut self, status: ActionStatus) {
        if !self.status.is_terminal() {
            self.status = status;
        }
    }
    pub fn expire(&mut self, now: Instant) {
        if now >= self.deadline {
            self.finish(ActionStatus::Unknown {
                reason: "absolute action deadline expired".into(),
            });
        }
    }
    pub fn advance(&mut self, now: Instant) {
        let ready = self.sent.and_then(|sent| {
            if self.needs_ack {
                self.ack.map(|ack| sent.max(ack))
            } else {
                Some(sent)
            }
        });
        if ready.is_some_and(|at| at + self.quiet <= now.min(self.deadline)) {
            self.finish(ActionStatus::Accepted);
        } else if now >= self.deadline {
            self.finish(ActionStatus::Unknown {
                reason: "acceptance deadline expired without sufficient evidence".into(),
            });
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn machine(ack: bool) -> (Instant, Acceptance) {
        let now = Instant::now();
        (
            now,
            Acceptance::new(
                now + Duration::from_secs(1),
                Duration::from_millis(100),
                ack,
            ),
        )
    }
    #[test]
    fn accepts_only_complete_send_and_covering_ack_then_quiet_window() {
        let (now, mut m) = machine(true);
        m.sent(now);
        m.advance(now + Duration::from_millis(200));
        assert!(matches!(m.status, ActionStatus::Pending));
        m.ack(now + Duration::from_millis(200));
        m.advance(now + Duration::from_millis(299));
        assert!(matches!(m.status, ActionStatus::Pending));
        m.advance(now + Duration::from_millis(300));
        assert!(matches!(m.status, ActionStatus::Accepted));
    }
    #[test]
    fn no_ack_actions_accept_without_server_echo() {
        let (now, mut m) = machine(false);
        m.sent(now);
        m.advance(now + Duration::from_millis(100));
        assert!(matches!(m.status, ActionStatus::Accepted));
    }
    #[test]
    fn unknown_on_missing_ack_or_incomplete_send() {
        for ack in [true, false] {
            let (now, mut m) = machine(ack);
            if ack {
                m.sent(now);
            }
            m.advance(now + Duration::from_secs(1));
            assert!(matches!(m.status, ActionStatus::Unknown { .. }));
        }
    }
    #[test]
    fn incomplete_settle_window_at_deadline_is_unknown() {
        let (now, mut m) = machine(true);
        m.sent(now);
        m.ack(now + Duration::from_millis(950));
        m.advance(now + Duration::from_secs(1));
        assert!(matches!(m.status, ActionStatus::Unknown { .. }));
    }
    #[test]
    fn status_reads_cannot_accept_unprocessed_corrections() {
        let (now, mut m) = machine(false);
        m.sent(now);
        m.expire(now + Duration::from_millis(900));
        assert!(matches!(m.status, ActionStatus::Pending));
        m.expire(now + Duration::from_secs(1));
        assert!(matches!(m.status, ActionStatus::Unknown { .. }));
    }
    #[test]
    fn reject_cancel_and_correction_are_immutable() {
        for outcome in [
            ActionStatus::Rejected {
                reason: "refused".into(),
            },
            ActionStatus::Cancelled,
            ActionStatus::Unknown {
                reason: "history gap".into(),
            },
        ] {
            let (now, mut m) = machine(false);
            m.finish(outcome.clone());
            m.sent(now);
            m.advance(now + Duration::from_secs(2));
            assert_eq!(
                serde_json::to_value(&m.status).unwrap(),
                serde_json::to_value(&outcome).unwrap()
            );
        }
    }
}
