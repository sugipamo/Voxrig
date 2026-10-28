// Java 1.21.11 Body late-correction monitoring through the TCP placement
// fixture. Mirrors the 1.16.1 checks: same evidence reducer, public facade and
// original packets where the fixture can send them.
use super::*;
use crate::body::evidence::Update;
use crate::body::{
    Action, ActionId, ActionStatus, Body, BodyConfig, HeadlessBody, MonitorCursor,
    MonitoringStatus, ReleaseStatus, SendStatus,
};

async fn survival() -> Fixture {
    let f = Fixture::new().await;
    f.api.bot.session.state.lock().await.operations.game_mode = Some(GameMode::Survival);
    f
}
fn body_with(f: &Fixture, retention: usize) -> Body<HeadlessBody> {
    Body::new(
        HeadlessBody::new(
            crate::Client::from_java_1_21_11(f.api.bot.clone()),
            BodyConfig {
                retention,
                settle_window: Duration::from_millis(20),
                action_timeout: Duration::from_millis(240),
            },
        )
        .unwrap(),
    )
}
fn body(f: &Fixture) -> Body<HeadlessBody> {
    body_with(f, BodyConfig::default().retention)
}
async fn complete(body: &Body<HeadlessBody>, id: ActionId) -> ActionStatus {
    timeout(Duration::from_secs(2), async {
        loop {
            let s = body.action_status(id).await.unwrap();
            if s.is_terminal() {
                return s;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
async fn wait_release(body: &Body<HeadlessBody>) {
    tokio::time::sleep(Duration::from_millis(15)).await;
    assert_eq!(
        body.release_control().await.unwrap(),
        ReleaseStatus::Released
    );
}
async fn monitored_look(f: &mut Fixture, body: &Body<HeadlessBody>, trust: bool) -> ActionId {
    let action = Action::Look {
        yaw: 12.,
        pitch: 4.,
    };
    let id = if trust {
        let sent = body.backend().send_and_trust(action).await.unwrap();
        assert!(matches!(sent.dispatch, SendStatus::Sent));
        sent.id
    } else {
        body.submit(action).await.unwrap()
    };
    f.expected_packet("look").await;
    assert!(matches!(complete(body, id).await, ActionStatus::Accepted));
    wait_release(body).await;
    id
}
/// Unrelated received-cell evidence; the bulk path the receiver itself uses.
async fn unrelated(f: &Fixture, count: usize) {
    let mut state = f.api.bot.session.state.lock().await;
    for _ in 0..count {
        let sequence = state.sequence;
        state
            .body_evidence
            .push(sequence, Update::Block([3, 1, 0], 1));
    }
}
/// Original own-position correction (teleport) packet.
async fn teleport(f: &mut Fixture) {
    let mut pose = vec![1];
    for v in [1.5f64, 1., 0.5, 0., 0., 0.] {
        pose.extend(v.to_be_bytes());
    }
    for v in [0f32, 0.] {
        pose.extend(v.to_be_bytes());
    }
    pose.extend(0u32.to_be_bytes());
    f.receive(ids::play_clientbound::POSITION, &pose).await;
}
fn place() -> Action {
    Action::PlaceBlock {
        pos: SUPPORT,
        face: crate::BlockFace::West,
        cursor: [0., 0.5, 0.5],
        hand: crate::Hand::Main,
        target: TARGET,
        expected: Some(native("dirt")),
    }
}
async fn place_sequence(f: &mut Fixture) -> i32 {
    let (packet, payload) = f.expected_packet("body BLOCK_PLACE").await;
    assert_eq!(packet, ids::play_serverbound::BLOCK_PLACE);
    let mut r = Reader::new(&payload);
    r.varint().unwrap();
    r.u64().unwrap();
    r.varint().unwrap();
    for _ in 0..3 {
        r.f32().unwrap();
    }
    r.bool().unwrap();
    r.bool().unwrap();
    r.varint().unwrap()
}

#[tokio::test]
async fn body_monitor_gap_reaches_upper_without_changing_original_acceptance() {
    let mut f = survival().await;
    let body = body(&f);
    let id = monitored_look(&mut f, &body, true).await;
    unrelated(&f, 4097).await;
    // A newer action in the same merged read still has a complete baseline.
    let recent = monitored_look(&mut f, &body, false).await;
    // The send-and-trust compatibility path keeps its historical result.
    assert!(matches!(
        body.action_status(id).await.unwrap(),
        ActionStatus::Accepted
    ));
    let events = body.events_after(Default::default(), 100).await.unwrap();
    assert!(!events.gap); // Body event retention is a different axis.
    assert!(matches!(
        body.action_observation(recent).await.unwrap().monitoring,
        MonitoringStatus::CompleteThrough { .. }
    ));
    let changes = body
        .monitor_changes_after(Default::default(), 100)
        .await
        .unwrap();
    assert!(!changes.gap);
    assert!(
        changes
            .changes
            .iter()
            .any(|c| c.action == id && matches!(c.monitoring, MonitoringStatus::Gap { .. }))
    );
    let observed = body.action_observation(id).await.unwrap();
    assert!(matches!(observed.acceptance, ActionStatus::Accepted));
    assert!(matches!(observed.outcome, ActionStatus::Accepted));
    assert!(matches!(observed.monitoring, MonitoringStatus::Gap { .. }));
    // A later retained rollback cannot be attributed across the lost interval.
    teleport(&mut f).await;
    let later = body.action_observation(id).await.unwrap();
    assert_eq!(later.monitoring, observed.monitoring);
    assert!(matches!(later.outcome, ActionStatus::Accepted));
    f.api.bot.session.state.lock().await.loading.generation += 1;
    assert_eq!(
        observed.monitoring,
        body.action_observation(id).await.unwrap().monitoring
    );
    assert!(
        body.monitor_changes_after(changes.cursor, 100)
            .await
            .unwrap()
            .changes
            .iter()
            .all(|change| change.action != id)
    );
    drop(body);
    f.stop().await;
}

#[tokio::test]
async fn body_monitor_timeout_and_error_are_visible_retryable_and_log_eviction_is_explicit() {
    let mut f = survival().await;
    let body = body_with(&f, 2);
    let id = monitored_look(&mut f, &body, false).await;
    let first = body.action_observation(id).await.unwrap();
    assert!(matches!(
        first.monitoring,
        MonitoringStatus::CompleteThrough { .. }
    ));
    let gate = f.api.bot.session.state.lock().await;
    let failed = body.action_observation(id).await.unwrap();
    assert!(matches!(failed.acceptance, ActionStatus::Accepted));
    assert!(
        matches!(failed.monitoring, MonitoringStatus::TemporarilyUnavailable { ref reason, .. } if reason.contains("timed out"))
    );
    // action_outcome keeps the historical result; the failed read is recorded
    // as monitoring state instead of being dropped.
    assert!(matches!(
        body.backend().action_outcome(id).await.unwrap(),
        ActionStatus::Accepted
    ));
    drop(gate);
    assert_eq!(
        body.action_observation(id).await.unwrap().monitoring,
        first.monitoring
    );
    // A retained session failure makes the evidence snapshot fail explicitly.
    f.api.bot.session.state.lock().await.failure = Some(crate::Error::new(
        ErrorKind::State,
        anyhow::anyhow!("injected evidence read failure"),
    ));
    let failed = body.action_observation(id).await.unwrap();
    assert!(
        matches!(failed.monitoring, MonitoringStatus::TemporarilyUnavailable { ref reason, .. } if reason.contains("injected evidence read failure"))
    );
    f.api.bot.session.state.lock().await.failure = None;
    assert_eq!(
        body.action_observation(id).await.unwrap().monitoring,
        first.monitoring
    );
    let log = body
        .monitor_changes_after(Default::default(), 1)
        .await
        .unwrap();
    assert!(log.gap);
    assert_eq!(log.changes.len(), 1);
    let tail = body.monitor_changes_after(log.cursor, 1).await.unwrap();
    assert!(!tail.gap);
    assert_eq!(tail.changes.len(), 1);
    assert!(
        body.monitor_changes_after(tail.cursor, 1)
            .await
            .unwrap()
            .changes
            .is_empty()
    );
    for cursor in [
        MonitorCursor {
            backend: id.backend + 1,
            ordinal: 0,
        },
        MonitorCursor {
            backend: 0,
            ordinal: 1,
        },
        MonitorCursor {
            backend: id.backend,
            ordinal: u64::MAX,
        },
    ] {
        assert_eq!(
            body.monitor_changes_after(cursor, 1)
                .await
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidInput
        );
    }
    drop(body);
    f.stop().await;
}

#[tokio::test]
async fn body_monitor_world_loss_is_visible_for_ordinary_submit_and_deduplicated() {
    let mut f = survival().await;
    let body = body(&f);
    let id = monitored_look(&mut f, &body, false).await;
    let before = body.action_observation(id).await.unwrap();
    f.api.bot.session.state.lock().await.loading.generation += 1;
    let events = body.events_after(Default::default(), 100).await.unwrap();
    assert!(!events.gap);
    let lost = body.action_observation(id).await.unwrap();
    assert!(matches!(lost.acceptance, ActionStatus::Accepted));
    assert!(
        matches!(lost.monitoring, MonitoringStatus::WorldChanged { after, .. } if matches!(before.monitoring, MonitoringStatus::CompleteThrough { cursor } if after == cursor))
    );
    let changes = body
        .monitor_changes_after(Default::default(), 100)
        .await
        .unwrap();
    assert_eq!(
        changes
            .changes
            .iter()
            .filter(
                |c| c.action == id && matches!(c.monitoring, MonitoringStatus::WorldChanged { .. })
            )
            .count(),
        1
    );
    assert!(
        body.monitor_changes_after(changes.cursor, 100)
            .await
            .unwrap()
            .changes
            .is_empty()
    );
    drop(body);
    f.stop().await;
}

#[tokio::test]
async fn body_monitor_chunk_loss_preserves_unknown_and_does_not_cross_lifetime() {
    let mut f = survival().await;
    let body = body(&f);
    let id = body.submit(place()).await.unwrap();
    place_sequence(&mut f).await; // never acknowledged
    assert!(matches!(
        complete(&body, id).await,
        ActionStatus::Unknown { .. }
    ));
    wait_release(&body).await;
    f.receive(ids::play_clientbound::UNLOAD_CHUNK, &[0; 8])
        .await;
    // A newer block cannot restore the missing lifetime or prove no rollback.
    f.block(TARGET, "stone").await;
    let observed = body.action_observation(id).await.unwrap();
    assert!(matches!(observed.acceptance, ActionStatus::Unknown { .. }));
    assert!(matches!(observed.outcome, ActionStatus::Unknown { .. }));
    assert!(matches!(
        observed.monitoring,
        MonitoringStatus::ChunkChanged { chunk: [0, 0], .. }
    ));
    let log = body
        .monitor_changes_after(Default::default(), 100)
        .await
        .unwrap();
    assert!(
        log.changes
            .iter()
            .any(|c| c.action == id && c.monitoring == observed.monitoring)
    );
    drop(body);
    f.stop().await;
}

#[tokio::test]
async fn body_monitor_ordinary_submit_reports_rollback_and_advances_through_unrelated_evidence() {
    let mut f = survival().await;
    let body = body(&f);
    let id = monitored_look(&mut f, &body, false).await;
    for _ in 0..2 {
        unrelated(&f, 2100).await;
        assert!(matches!(
            body.action_observation(id).await.unwrap().monitoring,
            MonitoringStatus::CompleteThrough { .. }
        ));
    }
    teleport(&mut f).await;
    let (a, b) = tokio::join!(body.action_observation(id), body.action_observation(id));
    let observed = a.unwrap();
    assert_eq!(observed.monitoring, b.unwrap().monitoring);
    assert!(matches!(observed.acceptance, ActionStatus::Accepted));
    assert!(matches!(observed.outcome, ActionStatus::Corrected { .. }));
    assert!(matches!(
        observed.monitoring,
        MonitoringStatus::CorrectionObserved { .. }
    ));
    // Compatibility action_status on an ordinary submit retains its result.
    assert!(matches!(
        body.action_status(id).await.unwrap(),
        ActionStatus::Accepted
    ));
    let log = body
        .monitor_changes_after(Default::default(), 100)
        .await
        .unwrap();
    assert_eq!(
        log.changes
            .iter()
            .filter(|c| matches!(c.monitoring, MonitoringStatus::CorrectionObserved { .. }))
            .count(),
        1
    );
    drop(body);
    f.stop().await;
}

#[tokio::test]
async fn body_monitor_trusted_place_reports_server_restored_before_after_acceptance() {
    let mut f = survival().await;
    let body = body(&f);
    let sent = body.backend().send_and_trust(place()).await.unwrap();
    assert!(matches!(sent.dispatch, SendStatus::Sent));
    let sequence = place_sequence(&mut f).await;
    f.ack(sequence).await;
    f.block(TARGET, "dirt").await;
    assert!(matches!(
        complete(&body, sent.id).await,
        ActionStatus::Accepted
    ));
    wait_release(&body).await;
    assert!(matches!(
        body.action_observation(sent.id).await.unwrap().monitoring,
        MonitoringStatus::CompleteThrough { .. }
    ));
    // The server later restores the original air after its processed boundary.
    f.block(TARGET, "air").await;
    assert!(matches!(
        body.action_status(sent.id).await.unwrap(),
        ActionStatus::Corrected { .. }
    ));
    let observed = body.action_observation(sent.id).await.unwrap();
    assert!(matches!(observed.acceptance, ActionStatus::Accepted));
    assert!(matches!(observed.outcome, ActionStatus::Corrected { .. }));
    assert!(matches!(
        observed.monitoring,
        MonitoringStatus::CorrectionObserved { .. }
    ));
    let events = body.events_after(Default::default(), 100).await.unwrap();
    assert_eq!(
        events
            .events
            .iter()
            .filter(|e| e.action == sent.id && matches!(e.status, ActionStatus::Corrected { .. }))
            .count(),
        1
    );
    drop(body);
    f.stop().await;
}
