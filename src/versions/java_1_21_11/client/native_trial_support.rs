//! Private live-trial contract. This entire module is compiled only for tests.
use super::*;
use anyhow::{Result as TrialResult, bail, ensure};
use serde::Deserialize;
use serde_json::json;
use std::{net::SocketAddr, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

const SPEC: &str = include_str!("../../../../scripts/fixtures/native_trials_1_21_11.json");
const SETUP: Duration = Duration::from_secs(60);
const READY: Duration = Duration::from_secs(5);
const BOUNDS: Region = Region {
    min: [-5, -61, -5],
    max: [10, -54, 6],
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    schema_version: u32,
    minecraft: String,
    control: serde_json::Value,
    trials: Vec<Trial>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Trial {
    id: String,
    test: String,
    port_env: String,
    output_env: String,
    profiles: Vec<Profile>,
    inventory: Vec<Supply>,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    name: String,
    role: String,
    position: [f64; 3],
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Supply {
    slot: usize,
    item: String,
    count: u8,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    setup: String,
    regions: Vec<FixtureRegion>,
    target_material: Option<String>,
    action_offset_ms: Option<u64>,
    observation_ms: u64,
    permitted_external_edits: Vec<ExternalEdit>,
    expected_evidence: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureRegion {
    role: String,
    min: [i32; 3],
    max: [i32; 3],
    material: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExternalEdit {
    phase: String,
    target: [i32; 3],
    materials: Vec<String>,
    deadline_from_action_ms: Option<u64>,
}
impl Fixture {
    fn parse(text: &str) -> TrialResult<Self> {
        let spec: Self = serde_json::from_str(text)?;
        ensure!(
            spec.schema_version == 1 && spec.minecraft == "1.21.11",
            "fixture version"
        );
        ensure!(
            spec.control["endpoint_env"] == "NATIVE_TRIAL_CONTROL",
            "control endpoint contract"
        );
        ensure!(
            spec.control["setup_timeout_ms"] == 60000
                && spec.control["readiness_timeout_ms"] == 5000,
            "fixture timeouts"
        );
        let ids = [
            "mining_raw",
            "mining_api",
            "mining_recovery",
            "movement_observed",
            "movement_predicted",
        ];
        ensure!(
            spec.trials.iter().map(|t| t.id.as_str()).eq(ids),
            "ordered trial IDs"
        );
        let tests = [
            "native_survival_mining_finish_abort_and_disconnect",
            "native_survival_mining_api_finish_abort_and_disconnect",
            "native_survival_same_profile_mining_recovery",
            "native_survival_walk_jump_collision_and_place",
            "native_survival_predicted_walk_jump_collision_and_place",
        ];
        for (trial, test) in spec.trials.iter().zip(tests) {
            ensure!(trial.test == test, "trial test name");
            let movement = trial.id.starts_with("movement");
            ensure!(
                trial.port_env
                    == if movement {
                        "NATIVE_MOVEMENT_PORT"
                    } else {
                        "NATIVE_MINING_PORT"
                    },
                "game port variable"
            );
            ensure!(
                trial.output_env
                    == if movement {
                        "NATIVE_MOVEMENT_OUTPUT"
                    } else {
                        "NATIVE_MINING_OUTPUT"
                    },
                "output variable"
            );
            let mut names = if movement {
                vec!["walk", "jump", "wall"]
            } else {
                vec![
                    "normal_finish",
                    "early_finish_abort",
                    "external_air_immediate_replacement",
                    "early_finish_disconnect",
                ]
            };
            if trial.id == "mining_raw" {
                names.remove(2);
            }
            if trial.id == "mining_recovery" {
                names.pop();
            }
            ensure!(
                trial.cases.iter().map(|c| c.id.as_str()).eq(names),
                "ordered case IDs"
            );
            ensure!(
                trial.profiles.len() == 2
                    && trial.profiles[0].name == "NatMineBot"
                    && trial.profiles[1].name == "NatMineView",
                "profiles"
            );
            ensure!(
                trial.profiles[0].role == "actor" && !trial.profiles[1].role.is_empty(),
                "profile roles"
            );
            let viewer_role = match trial.id.as_str() {
                "mining_api" => "retirement_authority_and_comparison",
                "movement_observed" => "endpoint_admission_and_comparison",
                _ => "comparison_only",
            };
            ensure!(trial.profiles[1].role == viewer_role, "observer role");
            ensure!(
                trial.profiles[0].position == [0.5, -60.0, 0.5]
                    && trial.profiles[1].position == [0.5, -60.0, 4.5],
                "initial pose"
            );
            if movement || trial.id == "mining_recovery" {
                ensure!(trial.inventory.len() == 1, "single supplied stack");
                let supply = &trial.inventory[0];
                ensure!(
                    (supply.slot, supply.item.as_str(), supply.count)
                        == if movement {
                            (9, "dirt", 3)
                        } else {
                            (36, "cobblestone", 1)
                        },
                    "supplied inventory contract"
                );
            } else {
                ensure!(trial.inventory.is_empty(), "empty mining inventory");
            }
            for s in &trial.inventory {
                ensure!(
                    (9..45).contains(&s.slot)
                        && s.count > 0
                        && operations::default_item(&s.item, s.count).is_ok(),
                    "inventory supply"
                );
            }
            for case in &trial.cases {
                ensure!(
                    case.setup
                        == if movement {
                            "initial_only"
                        } else {
                            "reset_before_case"
                        },
                    "setup policy"
                );
                ensure!(!case.expected_evidence.is_empty(), "expected evidence");
                ensure!(
                    case.action_offset_ms
                        == if movement {
                            None
                        } else {
                            Some(if case.id == "normal_finish" { 1100 } else { 50 })
                        },
                    "action offset"
                );
                ensure!(
                    case.observation_ms == if movement { 0 } else { 9000 },
                    "observation window"
                );
                ensure!(
                    case.target_material.as_deref()
                        == if movement {
                            None
                        } else {
                            Some(if case.id == "normal_finish" {
                                "dirt"
                            } else {
                                "stone"
                            })
                        },
                    "target material"
                );
                ensure!(
                    case.regions.iter().any(|r| r.role == "floor")
                        && case.regions.iter().any(|r| r.role == "air"),
                    "floor/air required"
                );
                for r in &case.regions {
                    ensure!(
                        ["floor", "air", "support", "wall", "target"].contains(&r.role.as_str()),
                        "region role"
                    );
                    ensure!(
                        (0..3).all(|i| r.min[i] <= r.max[i]
                            && r.min[i] >= BOUNDS.min[i]
                            && r.max[i] <= BOUNDS.max[i]),
                        "bounded region"
                    );
                    ensure!(
                        ["air", "stone", "dirt"].contains(&r.material.as_str()),
                        "region material"
                    );
                }
                let external = case.id == "external_air_immediate_replacement";
                let wall_edit = trial.id == "movement_predicted" && case.id == "wall";
                ensure!(
                    case.permitted_external_edits.len() == usize::from(external || wall_edit),
                    "permitted edit count"
                );
                ensure!(
                    case.regions.iter().any(|r| r.role
                        == if movement {
                            if case.id == "walk" {
                                "floor"
                            } else if case.id == "jump" {
                                "support"
                            } else {
                                "wall"
                            }
                        } else {
                            "target"
                        }),
                    "case geometry"
                );
                for e in &case.permitted_external_edits {
                    ensure!(
                        (external
                            && e.phase == "external_edit"
                            && e.target == [2, -59, 0]
                            && e.materials == ["air", "stone"]
                            && e.deadline_from_action_ms == Some(3000))
                            || (wall_edit
                                && e.phase == "install_wall"
                                && e.target == [4, -60, 0]
                                && e.materials == ["dirt"]
                                && e.deadline_from_action_ms.is_none()),
                        "external edit contract"
                    );
                }
            }
        }
        Ok(spec)
    }
}

pub(super) struct Control {
    stream: BufReader<tokio::net::TcpStream>,
    run_id: String,
    trial_id: String,
    request_id: u64,
    port: u16,
    usable: bool,
    fixture: Fixture,
}
impl Control {
    pub(super) async fn from_env(trial: &str, port: u16) -> Self {
        let endpoint =
            std::env::var("NATIVE_TRIAL_CONTROL").expect("NATIVE_TRIAL_CONTROL required");
        let run = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        Self::open(&endpoint, &run, trial, port, SETUP)
            .await
            .unwrap()
    }
    async fn open(
        endpoint: &str,
        run: &str,
        trial: &str,
        port: u16,
        budget: Duration,
    ) -> TrialResult<Self> {
        let addr: SocketAddr = endpoint.parse()?;
        ensure!(
            addr.ip().is_loopback() && addr.port() != 0 && port != 0,
            "explicit loopback endpoint required"
        );
        let fixture = Fixture::parse(SPEC)?;
        ensure!(
            fixture.trials.iter().any(|t| t.id == trial),
            "unknown trial"
        );
        let stream = timeout(budget, tokio::net::TcpStream::connect(addr)).await??;
        Ok(Self {
            stream: BufReader::new(stream),
            run_id: run.into(),
            trial_id: trial.into(),
            request_id: 0,
            port,
            usable: true,
            fixture,
        })
    }
    pub(super) fn evidence(&self) -> serde_json::Value {
        json!({"schema_version":1,"run_id":self.run_id,"trial_id":self.trial_id,"game_port":self.port})
    }
    fn trial(&self) -> &Trial {
        self.fixture
            .trials
            .iter()
            .find(|t| t.id == self.trial_id)
            .unwrap()
    }
    fn case(&self, case: &str) -> &Case {
        self.trial()
            .cases
            .iter()
            .find(|c| c.id == case)
            .expect("declared case")
    }
    pub(super) async fn request(
        &mut self,
        case: &str,
        phase: &str,
        action: Option<Instant>,
    ) -> TrialResult<()> {
        let budget = if let Some(start) = action {
            let edit = self
                .case(case)
                .permitted_external_edits
                .iter()
                .find(|e| e.phase == phase)
                .expect("permitted edit");
            let deadline =
                Duration::from_millis(edit.deadline_from_action_ms.expect("action deadline"));
            deadline
                .checked_sub(start.elapsed())
                .filter(|v| !v.is_zero())
                .ok_or_else(|| anyhow::anyhow!("external edit deadline already expired"))?
        } else {
            SETUP
        };
        self.exchange(case, phase, action, budget).await
    }
    async fn exchange(
        &mut self,
        case: &str,
        phase: &str,
        action: Option<Instant>,
        budget: Duration,
    ) -> TrialResult<()> {
        ensure!(self.usable, "control channel unusable after failure");
        self.case(case);
        self.request_id += 1;
        let request = json!({"schema_version":1,"run_id":self.run_id,"trial_id":self.trial_id,"case_id":case,"request_id":self.request_id,"phase":phase,"game_port":self.port,"action_elapsed_ms":action.map(|s| s.elapsed().as_millis())});
        // A failed/cancelled exchange is never retried: the edit may already have happened.
        self.usable = false;
        timeout(budget, async {
            let mut bytes = serde_json::to_vec(&request)?;
            bytes.push(b'\n');
            self.stream.get_mut().write_all(&bytes).await?;
            let mut reply = String::new();
            (&mut self.stream).take(4097).read_line(&mut reply).await?;
            ensure!(!reply.is_empty(), "control connection closed");
            ensure!(
                reply.len() <= 4096 && reply.ends_with('\n'),
                "invalid control frame"
            );
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Reply {
                schema_version: u32,
                run_id: String,
                trial_id: String,
                case_id: String,
                request_id: u64,
                phase: String,
                status: String,
            }
            let r: Reply = serde_json::from_str(&reply)?;
            ensure!(
                r.schema_version == 1
                    && r.run_id == self.run_id
                    && r.trial_id == self.trial_id
                    && r.case_id == case
                    && r.request_id == self.request_id
                    && r.phase == phase,
                "stale, duplicate or mismatched control reply"
            );
            ensure!(
                r.status == "ready",
                "controller did not acknowledge readiness"
            );
            Ok::<_, anyhow::Error>(())
        })
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "control timeout: run={} case={case} phase={phase}",
                self.run_id
            )
        })??;
        self.usable = true;
        Ok(())
    }
    pub(super) async fn ready(&self, bot: &Bot, viewer: &Bot, case: &str) -> TrialResult<()> {
        let result = wait_predicates(READY, || self.missing(bot, viewer, case)).await;
        let missing = result.as_ref().err().cloned().unwrap_or_default();
        // Retained in the test log even when a panic prevents final evidence serialization.
        eprintln!(
            "{}",
            json!({"readiness":{"run_id":self.run_id,"case_id":case,"missing_predicates":missing,"timed_out":result.is_err()}})
        );
        if result.is_err() {
            bail!("fixture readiness timeout: {}", missing.join(", "));
        }
        Ok(())
    }
    async fn missing(&self, bot: &Bot, viewer: &Bot, case: &str) -> Vec<String> {
        let mut missing = Vec::new();
        let movement_later = self.trial_id.starts_with("movement") && case != "walk";
        for (client, profile) in [
            (bot, &self.trial().profiles[0]),
            (viewer, &self.trial().profiles[1]),
        ] {
            let prefix = &profile.name;
            let Ok(p) = client.operations().player_state().await else {
                missing.push(format!("{prefix}.player_state"));
                continue;
            };
            if !p.interaction_loading.notification_dispatched() {
                missing.push(format!("{prefix}.loaded"));
            }
            if p.game_mode != Some(operations::GameMode::Survival) {
                missing.push(format!("{prefix}.survival"));
            }
            if !p
                .local_player
                .health
                .as_ref()
                .is_some_and(|h| h.health > 0.)
            {
                missing.push(format!("{prefix}.health"));
            }
            if (!movement_later || prefix == "NatMineView")
                && (!p.position_from_server
                    || !p.position.is_some_and(|p| {
                        (0..3).all(|i| (p[i] - profile.position[i]).abs() < 0.001)
                    }))
            {
                missing.push(format!("{prefix}.received_pose"));
            }
            if !client
                .operations()
                .standing_context()
                .await
                .is_ok_and(|c| c.on_ground && !c.submerged)
            {
                missing.push(format!("{prefix}.standing_geometry"));
            }
            if prefix == "NatMineBot" {
                for slot in 9..45 {
                    let expected = self
                        .trial()
                        .inventory
                        .iter()
                        .find(|s| (if movement_later { 36 } else { s.slot }) == slot)
                        .map(|s| operations::InventorySlot::Item {
                            item: operations::default_item(
                                &s.item,
                                s.count
                                    - if movement_later {
                                        if case == "jump" { 1 } else { 2 }
                                    } else {
                                        0
                                    },
                            )
                            .unwrap(),
                        })
                        .unwrap_or(operations::InventorySlot::Empty);
                    if p.inventory.slots.get(slot) != Some(&expected) {
                        missing.push(format!("inventory.slot_{slot}"));
                    }
                }
            }
        }
        if !viewer
            .operations()
            .visible_players()
            .await
            .is_ok_and(|v| v.players.iter().any(|p| p.name == "NatMineBot"))
        {
            missing.push("viewer.actor_visible".into());
        }
        // Compare received blocks on both connections. Overlay later declarations over air.
        let mut cells = std::collections::BTreeMap::new();
        for r in &self.case(case).regions {
            // Later movement cases preserve the initial floor/air plus earlier placements.
            if movement_later && (r.role == "air" || r.role == "floor") {
                continue;
            }
            for x in r.min[0]..=r.max[0] {
                for y in r.min[1]..=r.max[1] {
                    for z in r.min[2]..=r.max[2] {
                        cells.insert([x, y, z], r.material.as_str());
                    }
                }
            }
        }
        // One bounded received snapshot per connection, rather than one lock per cell.
        for (client, label) in [(bot, "actor"), (viewer, "viewer")] {
            match client.observe_region(BOUNDS).await {
                Ok(observed) => {
                    for (pos, mat) in &cells {
                        if !observed.blocks.iter().any(|b| {
                            b.position == *pos
                                && b.state
                                    .as_ref()
                                    .is_some_and(|s| s.name == format!("minecraft:{mat}"))
                        }) {
                            missing.push(format!("{label}.geometry.{pos:?}.{mat}"));
                            break;
                        }
                    }
                }
                Err(_) => missing.push(format!("{label}.geometry")),
            }
        }
        missing
    }
}

async fn wait_predicates<F, Fut>(
    budget: Duration,
    mut sample: F,
) -> std::result::Result<(), Vec<String>>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Vec<String>>,
{
    let mut missing = vec!["received_state".to_owned()];
    if timeout(budget, async {
        loop {
            missing = sample().await;
            if missing.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .is_err()
    {
        return Err(missing);
    }
    Ok(())
}

pub(super) fn game_port(var: &str) -> u16 {
    let port = std::env::var(var)
        .expect("explicit native trial port required")
        .parse()
        .expect("valid native trial port");
    assert_ne!(port, 0, "nonzero native trial port required");
    port
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    async fn peer(mode: &'static str) -> (Control, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = listener.local_addr().unwrap();
        assert_ne!(endpoint.port(), 25572);
        let task = tokio::spawn(async move {
            let (stream, _) = timeout(Duration::from_secs(2), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut stream = BufReader::new(stream);
            let mut line = String::new();
            timeout(Duration::from_secs(2), stream.read_line(&mut line))
                .await
                .unwrap()
                .unwrap();
            let request: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["game_port"], 31001);
            if mode == "closed" {
                return;
            }
            if mode == "timeout" {
                std::future::pending::<()>().await;
            }
            let mut reply = request;
            reply.as_object_mut().unwrap().remove("game_port");
            reply.as_object_mut().unwrap().remove("action_elapsed_ms");
            reply["status"] = json!("ready");
            match mode {
                "wrong_case" => reply["case_id"] = json!("early_finish_abort"),
                "wrong_run" => reply["run_id"] = json!("previous-run"),
                "stale" => reply["request_id"] = json!(0),
                _ => (),
            }
            let mut bytes = serde_json::to_vec(&reply).unwrap();
            bytes.push(b'\n');
            if mode == "duplicate" {
                bytes.extend(bytes.clone());
            }
            stream.get_mut().write_all(&bytes).await.unwrap();
            if mode == "duplicate" {
                line.clear();
                timeout(Duration::from_secs(2), stream.read_line(&mut line))
                    .await
                    .unwrap()
                    .unwrap();
            }
        });
        let control = Control::open(
            &endpoint.to_string(),
            "test-run",
            "mining_api",
            31001,
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        (control, task)
    }

    #[tokio::test]
    async fn non_default_loopback_endpoint() {
        let (mut control, task) = peer("ok").await;
        control
            .request("normal_finish", "setup", None)
            .await
            .unwrap();
        task.await.unwrap();
        assert!(
            Control::open(
                "192.0.2.1:1234",
                "run",
                "mining_api",
                1,
                Duration::from_millis(10)
            )
            .await
            .is_err()
        );
        assert!(
            Control::open(
                "127.0.0.1:0",
                "run",
                "mining_api",
                1,
                Duration::from_millis(10)
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn wrong_case_run_and_stale_replies_poison_channel() {
        for mode in ["wrong_case", "wrong_run", "stale"] {
            let (mut control, task) = peer(mode).await;
            let error = control
                .request("normal_finish", "setup", None)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("mismatched"), "{mode}: {error}");
            assert!(
                control
                    .request("normal_finish", "setup", None)
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("unusable")
            );
            task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn duplicate_reply_cannot_acknowledge_next_case() {
        let (mut control, task) = peer("duplicate").await;
        control
            .request("normal_finish", "setup", None)
            .await
            .unwrap();
        assert!(
            control
                .request("early_finish_abort", "setup", None)
                .await
                .unwrap_err()
                .to_string()
                .contains("duplicate")
        );
        task.await.unwrap();
    }

    #[tokio::test]
    async fn closed_connection_is_bounded() {
        let (mut control, task) = peer("closed").await;
        assert!(
            control
                .request("normal_finish", "setup", None)
                .await
                .unwrap_err()
                .to_string()
                .contains("closed")
        );
        task.await.unwrap();
    }

    #[tokio::test]
    async fn silent_peer_times_out_without_blocking_worker() {
        let (mut control, task) = peer("timeout").await;
        let result = control
            .exchange("normal_finish", "setup", None, Duration::from_millis(30))
            .await;
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("timeout: run=test-run case=normal_finish phase=setup")
        );
        assert!(!control.usable);
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    }

    #[tokio::test]
    async fn external_edit_budget_is_relative_to_action() {
        let (mut control, task) = peer("ok").await;
        let error = control
            .request(
                "external_air_immediate_replacement",
                "external_edit",
                Some(Instant::now() - Duration::from_secs(3)),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("deadline already expired"));
        // No request is sent for an expired action.
        task.abort();
        let _ = task.await;
    }

    #[tokio::test(start_paused = true)]
    async fn readiness_requires_received_state_and_names_timeout_predicate() {
        let mut samples = 0;
        wait_predicates(Duration::from_secs(1), || {
            samples += 1;
            std::future::ready(if samples < 3 {
                vec!["inventory.slot_36".into(), "actor.geometry".into()]
            } else {
                Vec::new()
            })
        })
        .await
        .unwrap();
        assert_eq!(samples, 3);
        let missing = wait_predicates(Duration::from_secs(1), || {
            std::future::ready(vec!["NatMineBot.received_pose".into()])
        })
        .await
        .unwrap_err();
        assert_eq!(missing, ["NatMineBot.received_pose"]);
        let missing = wait_predicates(Duration::from_secs(1), std::future::pending)
            .await
            .unwrap_err();
        assert_eq!(missing, ["received_state"]);
    }

    #[test]
    fn fixture_spec_parse_and_validation() {
        let fixture = Fixture::parse(SPEC).unwrap();
        assert_eq!(fixture.trials.len(), 5);
        assert_eq!(
            fixture.trials.iter().map(|t| t.cases.len()).sum::<usize>(),
            16
        );
        for path in [
            "schema",
            "case",
            "region",
            "offset",
            "window",
            "edit",
            "inventory",
            "wall_edit",
            "unknown",
        ] {
            let mut spec: serde_json::Value = serde_json::from_str(SPEC).unwrap();
            match path {
                "schema" => spec["schema_version"] = json!(2),
                "case" => spec["trials"][0]["cases"][0]["id"] = json!("typo"),
                "region" => spec["trials"][0]["cases"][0]["regions"][0]["max"][0] = json!(-6),
                "offset" => spec["trials"][0]["cases"][0]["action_offset_ms"] = json!(50),
                "window" => spec["trials"][0]["cases"][0]["observation_ms"] = json!(100),
                "edit" => {
                    spec["trials"][1]["cases"][2]["permitted_external_edits"][0]["deadline_from_action_ms"] =
                        json!(4000)
                }
                "inventory" => spec["trials"][2]["inventory"][0]["slot"] = json!(46),
                "wall_edit" => {
                    spec["trials"][4]["cases"][2]["permitted_external_edits"][0] =
                        spec["trials"][1]["cases"][2]["permitted_external_edits"][0].clone();
                }
                _ => spec["public_option"] = json!(true),
            }
            assert!(
                Fixture::parse(&spec.to_string()).is_err(),
                "accepted invalid {path}"
            );
        }
    }
}
