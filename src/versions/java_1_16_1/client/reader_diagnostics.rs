//! Opt-in diagnostics; no gameplay/cache locks are acquired by the reporter.

use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Reading,
    Capture,
    MovementCapture,
    Applying,
    Yielding,
    ApplyGate,
}

impl Phase {
    fn index(self) -> usize {
        self as usize
    }
}

struct Progress {
    phase: Phase,
    since: Instant,
    packet_id: Option<i32>,
    phase_totals: [Duration; 6],
    decoded: u64,
    applied: u64,
    read_attempts: u64,
    read_polls: u64,
    last_poll: Option<Instant>,
    max_poll_gap: Duration,
    captures_started: u64,
    captures_completed: u64,
}

impl Progress {
    fn new() -> Self {
        Self {
            phase: Phase::Reading,
            since: Instant::now(),
            packet_id: None,
            phase_totals: [Duration::ZERO; 6],
            decoded: 0,
            applied: 0,
            read_attempts: 0,
            read_polls: 0,
            last_poll: None,
            max_poll_gap: Duration::ZERO,
            captures_started: 0,
            captures_completed: 0,
        }
    }

    fn enter(&mut self, phase: Phase, packet_id: Option<i32>) {
        let now = Instant::now();
        self.phase_totals[self.phase.index()] += now.duration_since(self.since);
        self.phase = phase;
        self.since = now;
        self.packet_id = packet_id;
        match phase {
            Phase::Capture | Phase::MovementCapture => self.captures_started += 1,
            Phase::ApplyGate => self.decoded += 1,
            _ => {}
        }
    }

    fn snapshot(
        &self,
        generation: u64,
        final_snapshot: bool,
        interval: Duration,
    ) -> serde_json::Value {
        let now = Instant::now();
        let age = now.duration_since(self.since);
        let mut totals = self.phase_totals;
        totals[self.phase.index()] += age;
        serde_json::json!({
            "stage":"reader_progress", "generation":generation,
            "final":final_snapshot, "report_interval_ms":interval.as_millis(),
            "phase":format!("{:?}", self.phase), "phase_age_ms":age.as_millis(),
            "packet_id":self.packet_id, "decoded":self.decoded, "applied":self.applied,
            "read_attempts":self.read_attempts, "read_polls":self.read_polls,
            "last_poll_age_ms":self.last_poll.map(|p| now.duration_since(p).as_millis()),
            "max_poll_gap_ms":self.max_poll_gap.as_millis(),
            "captures_started":self.captures_started, "captures_completed":self.captures_completed,
            "reading_ms":totals[0].as_millis(), "capture_ms":totals[1].as_millis(),
            "movement_capture_ms":totals[2].as_millis(), "applying_ms":totals[3].as_millis(),
            "yielding_ms":totals[4].as_millis(),
            "apply_gate_ms":totals[5].as_millis(),
        })
    }
}

pub(super) struct Diagnostics {
    progress: Option<Arc<Mutex<Progress>>>,
    reporter: Option<tokio::task::JoinHandle<()>>,
    generation: u64,
}

impl Diagnostics {
    pub(super) fn new(generation: u64) -> Self {
        let enabled = std::env::var_os("VOXRIG_TRACE_READER_PHASES").is_some_and(|v| v == "1")
            && std::env::var_os("VOXRIG_TRACE_PROTOCOL").is_some_and(|v| v == "1");
        Self::with_enabled(generation, enabled)
    }

    fn with_enabled(generation: u64, enabled: bool) -> Self {
        if !enabled {
            return Self {
                progress: None,
                reporter: None,
                generation,
            };
        }
        let progress = Arc::new(Mutex::new(Progress::new()));
        let observed = Arc::clone(&progress);
        let reporter = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut previous = Instant::now();
            loop {
                interval.tick().await;
                let now = Instant::now();
                let elapsed = now.duration_since(previous);
                previous = now;
                crate::lifecycle::emit_protocol_timing(|| {
                    observed
                        .lock()
                        .unwrap()
                        .snapshot(generation, false, elapsed)
                });
            }
        });
        Self {
            progress: Some(progress),
            reporter: Some(reporter),
            generation,
        }
    }

    fn update(&self, change: impl FnOnce(&mut Progress)) {
        if let Some(progress) = &self.progress {
            change(&mut progress.lock().unwrap());
        }
    }

    pub(super) fn enter(&self, phase: Phase, packet_id: Option<i32>) {
        self.update(|p| p.enter(phase, packet_id));
    }

    pub(super) fn read_started(&self) {
        self.update(|p| p.read_attempts += 1);
    }

    pub(super) fn capture_completed(&self) {
        self.update(|p| p.captures_completed += 1);
    }

    pub(super) fn applied(&self) {
        self.update(|p| p.applied += 1);
    }

    pub(super) fn poll<F: Future>(
        &self,
        future: Pin<&mut F>,
        cx: &mut Context<'_>,
    ) -> Poll<F::Output> {
        self.update(|p| {
            let now = Instant::now();
            if let Some(last) = p.last_poll {
                p.max_poll_gap = p.max_poll_gap.max(now.duration_since(last));
            }
            p.last_poll = Some(now);
            p.read_polls += 1;
        });
        future.poll(cx)
    }
}

impl Drop for Diagnostics {
    fn drop(&mut self) {
        if let Some(reporter) = &self.reporter {
            reporter.abort();
        }
        if let Some(progress) = &self.progress {
            crate::lifecycle::emit_protocol_timing(|| {
                progress
                    .lock()
                    .unwrap()
                    .snapshot(self.generation, true, Duration::ZERO)
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfinished_capture_is_visible_and_time_is_not_lost() {
        let mut p = Progress::new();
        p.enter(Phase::Capture, None);
        p.since = Instant::now() - Duration::from_millis(50);
        let snapshot = p.snapshot(1, false, Duration::from_secs(1));
        assert_eq!(snapshot["captures_started"], 1);
        assert_eq!(snapshot["captures_completed"], 0);
        assert!(snapshot["phase_age_ms"].as_u64().unwrap() >= 50);
        p.enter(Phase::Reading, None);
        assert!(p.phase_totals[Phase::Capture.index()] >= Duration::from_millis(50));
    }

    #[tokio::test]
    async fn poll_wrapper_preserves_pending_ready_and_exact_poll_count() {
        let diagnostics = Diagnostics::with_enabled(7, true);
        let mut calls = 0;
        let future = std::future::poll_fn(|cx| {
            calls += 1;
            if calls == 1 {
                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                Poll::Ready(42)
            }
        });
        tokio::pin!(future);
        let result = std::future::poll_fn(|cx| diagnostics.poll(future.as_mut(), cx)).await;
        assert_eq!(result, 42);
        assert_eq!(calls, 2);
        assert_eq!(
            diagnostics
                .progress
                .as_ref()
                .unwrap()
                .lock()
                .unwrap()
                .read_polls,
            2
        );
    }

    #[tokio::test]
    async fn dropping_diagnostics_aborts_reporter_and_disabled_has_no_task() {
        let diagnostics = Diagnostics::with_enabled(1, true);
        let abort = diagnostics.reporter.as_ref().unwrap().abort_handle();
        drop(diagnostics);
        tokio::task::yield_now().await;
        assert!(abort.is_finished());
        let disabled = Diagnostics::with_enabled(1, false);
        assert!(disabled.progress.is_none());
        assert!(disabled.reporter.is_none());
    }
}
