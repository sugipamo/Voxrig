//! JSON-line driver for `scripts/run_climbing_control.py`; common API only.
use std::io::Write;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use voxrig::client::control::Controls;
use voxrig::client::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = std::env::var("VOXRIG_PORT")?.parse()?;
    let client = Client::connect(ConnectionConfig::offline_from_env(
        Server::new("127.0.0.1", port),
        "ClimbingProbe",
    )?)
    .await?;
    client.wait_until_ready().await?;
    println!("{}", serde_json::json!({"ready":true}));
    std::io::stdout().flush()?;
    let survival = client.survival();
    let mut mounted = None;
    let creative_vehicle = std::env::var("VOXRIG_VEHICLE_MODE").as_deref() == Ok("creative");
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let request: serde_json::Value = serde_json::from_str(&line)?;
        let value = match request["command"].as_str().unwrap_or_default() {
            "prepare" => {
                let target: [f64; 3] = serde_json::from_value(request["position"].clone())?;
                tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        let state = client.player_state().await?;
                        if state.received_pose.as_ref().is_some_and(|p| {
                            p.position
                                .iter()
                                .zip(target)
                                .all(|(a, b)| (a - b).abs() < 0.01)
                        }) {
                            break Ok::<_, anyhow::Error>(());
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await??;
                client
                    .wait_for_loaded(
                        Region {
                            min: [-4, 60, -4],
                            max: [4, 84, 4],
                        },
                        Duration::from_secs(10),
                    )
                    .await?;
                // Chunk receipt precedes subsequent block/entity updates.
                tokio::time::sleep(Duration::from_millis(200)).await;
                serde_json::to_value(client.player_state().await?)?
            }
            "respawn" => serde_json::to_value(client.respawn().await?)?,
            "player" => serde_json::to_value(client.player_state().await?)?,
            "inventory_record" => serde_json::to_value(survival.inventory_click_record().await?)?,
            "inventory_click" => {
                use voxrig::client::inventory::{
                    InventoryClickButton as Button, InventoryClickStage, InventorySource,
                };
                let slot = request["slot"].as_u64().unwrap().try_into()?;
                let button = if request["button"] == "right" {
                    Button::Right
                } else {
                    Button::Left
                };
                let mode = client.player_state().await?.game_mode;
                let result = if mode == Some(GameMode::Creative) {
                    client
                        .creative()
                        .click_inventory(InventorySource::Player, slot, button)
                        .await
                } else {
                    survival
                        .click_inventory(InventorySource::Player, slot, button)
                        .await
                };
                if request["expect_rejected"] == true {
                    let error =
                        result.expect_err("invalid equipment fixture unexpectedly admitted");
                    serde_json::json!({"rejected":true, "kind":format!("{:?}",error.kind()),
                        "message":error.to_string()})
                } else {
                    let pending = result?;
                    let complete = tokio::time::timeout(Duration::from_secs(5), async {
                        loop {
                            let current = survival.inventory_click_record().await?.unwrap();
                            anyhow::ensure!(
                                current.id == pending.id,
                                "another click replaced original attempt"
                            );
                            match current.stage {
                                InventoryClickStage::ObservedClicked => {
                                    break Ok::<_, anyhow::Error>(current);
                                }
                                InventoryClickStage::RequiresInspection => {
                                    anyhow::bail!("click requires inspection: {current:?}")
                                }
                                InventoryClickStage::Pending => {}
                                _ => anyhow::bail!("unreviewed click stage"),
                            }
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                    })
                    .await??;
                    serde_json::to_value(complete)?
                }
            }
            "vehicle" => serde_json::to_value(client.vehicle_state().await?)?,
            "player_context" => serde_json::to_value(client.player_context().await?)?,
            "vehicle_record" => serde_json::to_value(client.vehicle_control_record().await?)?,
            "capture" => serde_json::to_value(
                client
                    .capture(Region {
                        min: [0, 64, 0],
                        max: [0, 66, 0],
                    })
                    .await?,
            )?,
            "look" => {
                survival
                    .look(serde_json::from_value(request["rotation"].clone())?)
                    .await?;
                serde_json::to_value(client.player_state().await?)?
            }
            "revoke" => {
                let revoked = client.revoke_connection();
                let rejected = client.player_state().await.is_err();
                println!(
                    "{}",
                    serde_json::json!({"revoked":revoked,"player_rejected":rejected})
                );
                std::io::stdout().flush()?;
                return Ok(());
            }
            "revoke_control" => {
                let revoked = client.revoke_connection();
                let rejected = survival
                    .request_ground_jump(request["session_id"].as_u64().unwrap())
                    .await
                    .is_err();
                let record = tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        let record = survival.control_record().await?.unwrap();
                        if matches!(
                            record.status,
                            voxrig::client::control::ControlStatus::Stopped { .. }
                        ) {
                            break Ok::<_, anyhow::Error>(record);
                        }
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                })
                .await??;
                println!(
                    "{}",
                    serde_json::json!({"revoked": revoked, "jump_rejected": rejected, "record": record})
                );
                std::io::stdout().flush()?;
                return Ok(());
            }
            "start" => serde_json::to_value(survival.start_control().await?)?,
            "jump" => {
                match survival
                    .request_ground_jump(request["session_id"].as_u64().unwrap())
                    .await
                {
                    Ok(record) => serde_json::json!({"request": record}),
                    Err(error) => serde_json::json!({"error": error.to_string()}),
                }
            }
            "restart" => {
                let stopped = survival.stop_control().await?;
                let started = survival.start_control().await?;
                let first = tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        let record = survival.control_record().await?.unwrap();
                        if record.dispatched_ticks > 0 {
                            break Ok::<_, anyhow::Error>(record);
                        }
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                })
                .await??;
                serde_json::json!({"stopped":stopped,"started":started,"first":first})
            }
            "keys" => {
                let controls: Controls = serde_json::from_value(request["controls"].clone())?;
                serde_json::to_value(survival.set_controls(controls).await?)?
            }
            "ticks" => {
                let initial = survival.control_record().await?.unwrap().dispatched_ticks;
                let count = request["count"].as_u64().unwrap();
                let record = tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        let record = survival.control_record().await?.unwrap();
                        if record.dispatched_ticks >= initial + count {
                            break Ok::<_, anyhow::Error>(record);
                        }
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                })
                .await;
                let record = match record {
                    Ok(record) => record?,
                    Err(error) => anyhow::bail!(
                        "control tick wait failed: {error}; retained record: {:?}",
                        survival.control_record().await?
                    ),
                };
                serde_json::to_value(record)?
            }
            "record" => serde_json::to_value(survival.control_record().await?)?,
            "mount" => {
                let entity_type = request["type"].as_str().unwrap();
                let target = tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        if let Some(entity) = client
                            .entity_spawns()
                            .await?
                            .entities
                            .into_iter()
                            .find(|e| e.type_name.as_deref() == Some(entity_type))
                        {
                            break Ok::<_, anyhow::Error>(entity.id);
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await??;
                if creative_vehicle {
                    client
                        .creative()
                        .interact_entity(target, Hand::Main, false)
                        .await?;
                } else {
                    survival.interact_entity(target, Hand::Main, false).await?;
                }
                let vehicle = tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        let vehicle = client.vehicle_state().await?;
                        if let Some(VehicleRelation::Mounted { mount }) =
                            vehicle.relation.as_ref().map(|r| &r.value)
                        {
                            if mount.vehicle() == Some(target) {
                                mounted = Some(*mount);
                                break Ok::<_, anyhow::Error>(vehicle);
                            }
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await??;
                serde_json::to_value(vehicle)?
            }
            "drive_cancelled_waiter" => {
                let inputs: Vec<VehicleInput> = serde_json::from_value(request["inputs"].clone())?;
                let timed = tokio::time::timeout(Duration::from_millis(80), async {
                    if creative_vehicle {
                        client
                            .creative()
                            .start_vehicle_control(mounted.unwrap(), &inputs)
                            .await
                    } else {
                        survival
                            .start_vehicle_control(mounted.unwrap(), &inputs)
                            .await
                    }
                })
                .await;
                anyhow::ensure!(
                    timed.is_err(),
                    "finite waiter did not remain pending for cancellation"
                );
                let pending = client.vehicle_control_record().await?.unwrap();
                anyhow::ensure!(
                    pending.stage == VehicleControlStage::Running,
                    "cancelled waiter lost running owner"
                );
                let complete = tokio::time::timeout(Duration::from_secs(12), async {
                    loop {
                        let record = client.vehicle_control_record().await?.unwrap();
                        anyhow::ensure!(
                            record.id == pending.id,
                            "cancelled finite owner was replaced"
                        );
                        if record.stage != VehicleControlStage::Running {
                            break Ok::<_, anyhow::Error>(record);
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await??;
                serde_json::json!({"cancelled_waiter": true, "pending": pending,
                    "error": complete.requires_inspection, "record": complete})
            }
            "drive" | "drive_until_interrupted" => {
                let inputs: Vec<VehicleInput> = serde_json::from_value(request["inputs"].clone())?;
                let result = if creative_vehicle {
                    client
                        .creative()
                        .start_vehicle_control(mounted.unwrap(), &inputs)
                        .await
                } else {
                    survival
                        .start_vehicle_control(mounted.unwrap(), &inputs)
                        .await
                };
                if request["command"] == "drive_until_interrupted" {
                    serde_json::json!({
                        "error": result.err().map(|e| e.to_string()),
                        "record": client.vehicle_control_record().await?,
                    })
                } else {
                    serde_json::to_value(result?)?
                }
            }
            "dismount" => {
                let record = if creative_vehicle {
                    client.creative().dismount(mounted.unwrap()).await?
                } else {
                    survival.dismount(mounted.unwrap()).await?
                };
                tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        let current = client.dismount_record().await?.unwrap();
                        if current.stage == DismountStage::ObservedUnmounted {
                            break Ok::<_, anyhow::Error>(());
                        }
                        if current.requires_inspection.is_some() {
                            anyhow::bail!("dismount interrupted");
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await??;
                let complete = if creative_vehicle {
                    client.creative().complete_dismount(record.id).await?
                } else {
                    survival.complete_dismount(record.id).await?
                };
                let stale_result = if creative_vehicle {
                    client
                        .creative()
                        .start_vehicle_control(mounted.unwrap(), &[VehicleInput::default()])
                        .await
                } else {
                    survival
                        .start_vehicle_control(mounted.unwrap(), &[VehicleInput::default()])
                        .await
                };
                anyhow::ensure!(stale_result.is_err());
                serde_json::to_value(complete)?
            }
            "stop" => serde_json::to_value(survival.stop_control().await?)?,
            "wait" => {
                tokio::time::sleep(Duration::from_millis(request["ms"].as_u64().unwrap())).await;
                serde_json::to_value(survival.control_record().await?)?
            }
            "disconnect" => {
                client.disconnect().await?;
                tokio::time::sleep(Duration::from_millis(150)).await;
                println!(
                    "{}",
                    serde_json::to_value(survival.control_record().await?)?
                );
                std::io::stdout().flush()?;
                return Ok(());
            }
            "context_disconnect" => {
                client.disconnect().await?;
                tokio::time::sleep(Duration::from_millis(150)).await;
                println!("{}", serde_json::to_value(client.player_context().await?)?);
                std::io::stdout().flush()?;
                return Ok(());
            }
            other => anyhow::bail!("unknown command: {other}"),
        };
        println!("{value}");
        std::io::stdout().flush()?;
    }
    client.disconnect().await?;
    Ok(())
}
