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
        "place" | "break" | "activate" => {
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
            } else if v["kind"] == "activate" {
                Action::ActivateBlock {
                    pos,
                    face,
                    cursor: serde_json::from_value(v["cursor"].clone())?,
                    hand: Hand::Main,
                }
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
    let mut config = BodyConfig::default();
    if let Ok(ms) = std::env::var("VOXRIG_BODY_TIMEOUT_MS") {
        config.action_timeout = Duration::from_millis(ms.parse()?);
    }
    if let Ok(retention) = std::env::var("VOXRIG_BODY_RETENTION") {
        config.retention = retention.parse()?;
    }
    let body = Body::new(HeadlessBody::new(client.clone(), config)?);
    let mut click_watch = None::<InventoryClickWatch>;
    let mut saved_screen = None;
    println!("{}", json!({"ready":true}));
    std::io::stdout().flush()?;
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let v: Json = serde_json::from_str(&line)?;
        let result: anyhow::Result<Json> = async {
            Ok(match v["command"].as_str().unwrap_or("") {
                "inventory_send" | "inventory_transfer_send" => {
                    use voxrig::client::inventory::{InventorySource, InventoryClickButton};
                    let source=if v["old_screen"]==true {InventorySource::Container{screen:saved_screen.ok_or_else(||anyhow::anyhow!("no saved screen"))?}}
                        else if v["container"]==true {InventorySource::Container{screen:client.screen_state().await?.screen.ok_or_else(||anyhow::anyhow!("no received screen"))?.id}} else {InventorySource::Player};
                    let slot=v["slot"].as_u64().unwrap().try_into()?;
                    let button=if v["right"]==true {InventoryClickButton::Right}else{InventoryClickButton::Left};
                    let record=if v["command"]=="inventory_transfer_send" {client.survival().send_inventory_transfer(source,slot).await?}
                        else {client.survival().send_inventory_click(source,slot,button).await?};
                    serde_json::to_value(record)?
                }
                "inventory_send_record" => serde_json::to_value(client.survival().inventory_send_record().await?)?,
                "inventory_resync" => serde_json::to_value(client.survival().resync_inventory().await?)?,
                "transfer_record" => serde_json::to_value(client.survival().inventory_transfer_record().await?)?,
                "crafting_record" => serde_json::to_value(client.survival().crafting_take_record().await?)?,
                "crafting" => serde_json::to_value(client.received_crafting().await?)?,
                "crafting_send" => {
                    let grid=client.received_crafting().await?.ok_or_else(||anyhow::anyhow!("no crafting grid"))?;
                    serde_json::to_value(client.survival().send_crafting_result(&grid,voxrig::client::crafting::CraftingResultDestination::Cursor).await?)?
                }
                "close_screen" => {
                    let screen=client.screen_state().await?.screen.ok_or_else(||anyhow::anyhow!("no received screen"))?.id;
                    saved_screen=Some(screen);
                    serde_json::to_value(client.survival().close_container(screen).await?)?
                }
                "click" => {
                    let source = if v["container"].as_bool() == Some(true) {
                        voxrig::client::inventory::InventoryClickSource::Container {
                            screen: client.screen_state().await?.screen.unwrap().id,
                        }
                    } else {
                        voxrig::client::inventory::InventoryClickSource::Player
                    };
                    let mode = client.player_state().await?.game_mode.unwrap();
                    let record = match mode {
                        GameMode::Creative => {
                            client
                                .creative()
                                .click_inventory(
                                    source,
                                    v["slot"].as_u64().unwrap().try_into()?,
                                    voxrig::client::inventory::InventoryClickButton::Left,
                                )
                                .await?
                        }
                        _ => {
                            client
                                .survival()
                                .click_inventory(
                                    source,
                                    v["slot"].as_u64().unwrap().try_into()?,
                                    voxrig::client::inventory::InventoryClickButton::Left,
                                )
                                .await?
                        }
                    };
                    click_watch = Some(body.backend().watch_inventory_click(record.id).await?);
                    serde_json::to_value(record)?
                }
                "click_record" => {
                    serde_json::to_value(client.survival().inventory_click_record().await?)?
                }
                "click_status" => {
                    serde_json::to_value(click_watch.as_ref().unwrap().status().await)?
                }
                "action_observation" => serde_json::to_value(
                    body.action_observation(serde_json::from_value(v["id"].clone())?).await?,
                )?,
                "monitor_changes" => serde_json::to_value(
                    body.monitor_changes_after(
                        if v["cursor"].is_null() { MonitorCursor::default() } else { serde_json::from_value(v["cursor"].clone())? },
                        v["maximum"].as_u64().unwrap_or(4096).try_into()?,
                    ).await?,
                )?,
                "outcome" => serde_json::to_value(
                    body.backend()
                        .action_outcome(serde_json::from_value(v["id"].clone())?)
                        .await?,
                )?,
                "dig_estimate" => serde_json::to_value(
                    client
                        .survival()
                        .dig_estimate(serde_json::from_value(v["pos"].clone())?)
                        .await?,
                )?,
                "strict_dig" => serde_json::to_value(
                    client
                        .survival()
                        .dig(serde_json::from_value(v["pos"].clone())?, BlockFace::North)
                        .await?,
                )?,
                "player" => serde_json::to_value(client.player_state().await?)?,
                "rotation_snapshot" => {
                    use voxrig::client::{CorrectionComponent, FieldState};
                    let s = client.received_player_snapshot().await?;
                    let angle = |c: &CorrectionComponent<f32>| match c {
                        CorrectionComponent::Absolute(v) => json!({"absolute": v}),
                        CorrectionComponent::Relative(v) => json!({"relative": v}),
                        _ => json!(null),
                    };
                    json!({
                        "boundary": s.boundary().ordinal(),
                        "rotation_correction": match s.rotation_correction() {
                            FieldState::Received(f) => json!({
                                "ordinal": f.receipt().ordinal(),
                                "rotation": f.value().rotation().iter().map(angle).collect::<Vec<_>>(),
                            }),
                            other => json!(format!("{other:?}")),
                        },
                        "absolute_rotation": match s.absolute_rotation() {
                            FieldState::Received(f) => json!({
                                "ordinal": f.receipt().ordinal(),
                                "value": f.value(),
                            }),
                            FieldState::Missing => json!("missing"),
                            _ => json!("invalidated"),
                        },
                    })
                }
                "body_use_item_expect" => {
                    let id = body
                        .backend()
                        .use_item_expect(
                            Hand::Main,
                            v["slot"].as_u64().unwrap().try_into()?,
                            client.registry().item(v["item"].as_str().unwrap())?.id,
                        )
                        .await?;
                    serde_json::to_value(id)?
                }
                "release_use_item" => {
                    serde_json::to_value(client.survival().release_use_item().await?)?
                }
                "path_start" => {
                    let controls: Vec<voxrig::client::survival::SurvivalControl> =
                        serde_json::from_value(v["controls"].clone())?;
                    serde_json::to_value(client.survival().start_predicted_path(&controls).await?)?
                }
                "path_record" => serde_json::to_value(client.survival().motion_record().await?)?,
                "control_states" => serde_json::to_value(
                    client
                        .control_state_changes_after(v["after"].as_u64().unwrap_or(0))
                        .await?,
                )?,
                "path_best_effort_start" => {
                    let controls: Vec<voxrig::client::survival::SurvivalControl> =
                        serde_json::from_value(v["controls"].clone())?;
                    let mode = client.player_state().await?.game_mode.unwrap();
                    serde_json::to_value(
                        client
                            .player_control(mode)
                            .start_path_best_effort(&controls)
                            .await?,
                    )?
                }
                "best_effort_start" => {
                    let mode = client.player_state().await?.game_mode.unwrap();
                    serde_json::to_value(
                        client
                            .player_control(mode)
                            .start_control_best_effort()
                            .await?,
                    )?
                }
                "walking_diagnostics" => {
                    let mode = client.player_state().await?.game_mode.unwrap();
                    serde_json::to_value(
                        client
                            .player_control(mode)
                            .walking_diagnostics_for(v["session_id"].as_u64().unwrap())
                            .await?,
                    )?
                }
                "control_changes" => {
                    serde_json::to_value(client.player_control(voxrig::client::GameMode::Survival)
                        .control_changes_after(v["after"].as_u64().unwrap_or(0)).await?)?
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
                "control_set_for" => {
                    let mode = client.player_state().await?.game_mode.unwrap();
                    serde_json::to_value(
                        client
                            .player_control(mode)
                            .set_controls_for(
                                v["session_id"].as_u64().unwrap(),
                                serde_json::from_value(v["controls"].clone())?,
                            )
                            .await?,
                    )?
                }
                "finite_start" => {
                    let controls: Vec<SurvivalControl> = serde_json::from_value(v["controls"].clone())?;
                    json!({"record":client.survival().start_predicted_path(&controls).await?})
                }
                "finite_record" => serde_json::to_value(client.survival().motion_record().await?)?,
                "path_best_effort" => {
                    let controls: Vec<SurvivalControl> = serde_json::from_value(v["controls"].clone())?;
                    json!({"record":client.player_control(GameMode::Survival).start_path_best_effort(&controls).await?})
                }
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
                "place_cube" => {
                    let Action::PlaceBlock {
                        pos,
                        face,
                        cursor,
                        hand,
                        ..
                    } = parse_action(&v)?
                    else {
                        anyhow::bail!("expected placement");
                    };
                    serde_json::to_value(body.place_cube(pos, face, cursor, hand).await?)?
                }
                "strict_open" => serde_json::to_value(
                    client
                        .survival()
                        .open_container(serde_json::from_value(v["pos"].clone())?)
                        .await?,
                )?,
                "container_open_record" => {
                    serde_json::to_value(client.survival().container_open_record().await?)?
                }
                "screen" => serde_json::to_value(client.screen_state().await?)?,
                "placement_warnings" => {
                    let Action::PlaceBlock { pos, face, .. } = parse_action(&v)? else {
                        anyhow::bail!("expected placement");
                    };
                    serde_json::to_value(
                        body.backend().cube_placement_diagnostics(pos, face).await?,
                    )?
                }
                "strict_place" => serde_json::to_value(
                    client
                        .survival()
                        .place_cube(serde_json::from_value(v["pos"].clone())?, BlockFace::South)
                        .await?,
                )?,
                "send_and_trust" => serde_json::to_value(
                    body.send_and_trust(ActionRequest {
                        action: parse_action(&v)?,
                        world: v
                            .get("world")
                            .map(|w| {
                                Ok::<_, anyhow::Error>(BodyWorld {
                                    connection: w["connection"].as_u64().unwrap(),
                                    generation: w["generation"].as_u64().unwrap(),
                                })
                            })
                            .transpose()?,
                    })
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
                "prepare" | "prepare_pose" => {
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
                    if v["command"] == "prepare" {
                        client
                            .wait_for_loaded(
                                Region {
                                    min: [-4, 60, -4],
                                    max: [4, 70, 4],
                                },
                                Duration::from_secs(10),
                            )
                            .await?;
                    }
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
