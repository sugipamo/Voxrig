//! JSON-line acceptance API driver for scripts/run_body_live.py (no native feature).
use serde_json::{Value as Json, json};
use std::{io::Write, time::Duration};
use tokio::io::{AsyncBufReadExt, BufReader};
use voxrig::client::control::Controls;
use voxrig::{body::*, client::prelude::*};

fn parse_action(v: &Json) -> anyhow::Result<Action> {
    Ok(match v["kind"].as_str().unwrap_or("") {
        "look" => Action::Look {
            yaw: v["yaw"].as_f64().unwrap() as f32,
            pitch: v["pitch"].as_f64().unwrap() as f32,
        },
        "select" => Action::SelectHotbar(v["slot"].as_u64().unwrap().try_into()?),
        "chat" => Action::Chat(v["text"].as_str().unwrap().into()),
        "command" => Action::Command(v["text"].as_str().unwrap().into()),
        "use" => Action::UseItem { hand: Hand::Main },
        "move" => Action::Move {
            controls: serde_json::from_value::<Controls>(v["controls"].clone())?,
            ticks: v["ticks"].as_u64().map(u32::try_from).transpose()?,
        },
        "place" | "break" => {
            let pos = serde_json::from_value(v["pos"].clone())?;
            let face = match v["face"].as_str().unwrap() {
                "Up" => BlockFace::Up,
                "Down" => BlockFace::Down,
                "North" => BlockFace::North,
                "South" => BlockFace::South,
                "West" => BlockFace::West,
                "East" => BlockFace::East,
                _ => anyhow::bail!("invalid face"),
            };
            if v["kind"] == "break" {
                Action::BreakBlock { pos, face }
            } else {
                Action::PlaceBlock {
                    pos,
                    face,
                    cursor: serde_json::from_value(v["cursor"].clone())?,
                    hand: if v["offhand"].as_bool() == Some(true) {
                        Hand::Off
                    } else {
                        Hand::Main
                    },
                    target: serde_json::from_value(v["target"].clone())?,
                    expected: if v["expected"].is_null() {
                        None
                    } else {
                        Some(serde_json::from_value(v["expected"].clone())?)
                    },
                }
            }
        }
        _ => anyhow::bail!("unknown action"),
    })
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Client::connect(ConnectionConfig::offline_from_env(
        Server::new("127.0.0.1", std::env::var("VOXRIG_PORT")?.parse()?),
        "ClimbingProbe",
    )?)
    .await?;
    client.wait_until_ready().await?;
    let body = Body::new(HeadlessBody::new(client.clone(), BodyConfig::default())?);
    println!("{}", json!({"ready":true}));
    std::io::stdout().flush()?;
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let v: Json = serde_json::from_str(&line)?;
        let result: anyhow::Result<Json> = async {
            Ok(match v["command"].as_str().unwrap_or("") {
                "path_start" => {
                    let controls: Vec<voxrig::client::survival::SurvivalControl> =
                        serde_json::from_value(v["controls"].clone())?;
                    serde_json::to_value(client.survival().start_predicted_path(&controls).await?)?
                }
                "path_record" => serde_json::to_value(client.survival().motion_record().await?)?,
                "best_effort_start" => {
                    let mode = client.player_state().await?.game_mode.unwrap();
                    serde_json::to_value(
                        client
                            .player_control(mode)
                            .start_control_best_effort()
                            .await?,
                    )?
                }
                "contact_diagnostic" => {
                    let mode = client.player_state().await?.game_mode.unwrap();
                    serde_json::to_value(
                        client
                            .player_control(mode)
                            .contact_diagnostic_for(v["session_id"].as_u64().unwrap())
                            .await?,
                    )?
                }
                "entities" => serde_json::to_value(client.entities().await?)?,
                "control_start" => serde_json::to_value(client.survival().start_control().await?)?,
                "control_set" => serde_json::to_value(
                    client
                        .survival()
                        .set_controls(serde_json::from_value(v["controls"].clone())?)
                        .await?,
                )?,
                "control_record" => {
                    serde_json::to_value(client.survival().control_record().await?)?
                }
                "control_stop" => serde_json::to_value(client.survival().stop_control().await?)?,
                "select" => serde_json::to_value(
                    client
                        .survival()
                        .select_hotbar(v["slot"].as_u64().unwrap().try_into()?)
                        .await?,
                )?,
                "strict_place" => serde_json::to_value(
                    client
                        .survival()
                        .place_cube(serde_json::from_value(v["pos"].clone())?, BlockFace::South)
                        .await?,
                )?,
                "submit" => serde_json::to_value(
                    body.submit(ActionRequest {
                        action: parse_action(&v)?,
                        world: None,
                    })
                    .await?,
                )?,
                "status" => serde_json::to_value(
                    body.action_status(serde_json::from_value(v["id"].clone())?)
                        .await?,
                )?,
                "cancel" => serde_json::to_value(
                    body.cancel(serde_json::from_value(v["id"].clone())?)
                        .await?,
                )?,
                "release" => serde_json::to_value(body.release_control().await?)?,
                "capabilities" => serde_json::to_value(body.capabilities())?,
                "events" => {
                    serde_json::to_value(body.events_after(EventCursor::default(), 4096).await?)?
                }
                "observe" => serde_json::to_value(
                    body.observe(ObserveRequest {
                        blocks: if v["blocks"].is_null() {
                            vec![]
                        } else {
                            serde_json::from_value(v["blocks"].clone())?
                        },
                        region: None,
                    })
                    .await?,
                )?,
                "prepare" => {
                    let wanted: [f64; 3] = serde_json::from_value(v["position"].clone())?;
                    tokio::time::timeout(Duration::from_secs(10), async {
                        loop {
                            let p = client.player_state().await?;
                            if p.received_pose.is_some_and(|p| {
                                p.position
                                    .iter()
                                    .zip(wanted)
                                    .all(|(a, b)| (a - b).abs() < 0.01)
                            }) {
                                break Ok::<_, anyhow::Error>(());
                            }
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                    })
                    .await??;
                    client
                        .wait_for_loaded(
                            Region {
                                min: [-4, 60, -4],
                                max: [4, 70, 4],
                            },
                            Duration::from_secs(10),
                        )
                        .await?;
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    serde_json::to_value(body.observe(ObserveRequest::default()).await?)?
                }
                "wait" => {
                    tokio::time::sleep(Duration::from_millis(v["ms"].as_u64().unwrap())).await;
                    json!(true)
                }
                "disconnect" => {
                    body.release_control().await?;
                    client.disconnect().await?;
                    json!(true)
                }
                _ => anyhow::bail!("unknown command"),
            })
        }
        .await;
        println!(
            "{}",
            match result {
                Ok(value) => value,
                Err(e) => json!({"error":e.to_string()}),
            }
        );
        std::io::stdout().flush()?;
        if v["command"] == "disconnect" {
            break;
        }
    }
    Ok(())
}
