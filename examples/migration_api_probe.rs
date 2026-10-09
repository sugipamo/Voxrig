//! Common-only JSON-line driver for long rays and mode-checked basic inputs.
use std::{io::Write, time::Duration};
use tokio::io::{AsyncBufReadExt, BufReader};
use voxrig::client::prelude::*;

async fn execute(
    client: &Client,
    request: &serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    Ok(match request["command"].as_str().unwrap_or_default() {
        "player" => serde_json::to_value(client.player_state().await?)?,
        "chunks" => serde_json::to_value(client.loaded_chunks().await?)?,
        "prepare" => {
            let target: [f64; 3] = serde_json::from_value(request["position"].clone())?;
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let player = client.player_state().await?;
                    if player
                        .received_pose
                        .as_ref()
                        .is_some_and(|p| p.position == target)
                    {
                        break Ok::<_, voxrig::Error>(player);
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await??;
            client
                .wait_for_loaded(
                    Region {
                        min: [-4, 64, -4],
                        max: [4, 67, 4],
                    },
                    Duration::from_secs(15),
                )
                .await?;
            tokio::time::sleep(Duration::from_millis(200)).await;
            serde_json::to_value(client.player_state().await?)?
        }
        "loaded" => {
            let region = Region {
                min: serde_json::from_value(request["min"].clone())?,
                max: serde_json::from_value(request["max"].clone())?,
            };
            let capture = client
                .wait_for_loaded(region, Duration::from_secs(20))
                .await?;
            serde_json::json!({"session":capture.player.session,"sequence":capture.player.receive_sequence})
        }
        "raycast" => serde_json::to_value(
            client
                .raycast_blocks(
                    serde_json::from_value(request["origin"].clone())?,
                    serde_json::from_value(request["direction"].clone())?,
                    request["distance"]
                        .as_f64()
                        .ok_or_else(|| anyhow::anyhow!("distance required"))?,
                )
                .await?,
        )?,
        "look" | "hotbar" => {
            let mode: GameMode = serde_json::from_value(request["mode"].clone())?;
            let input = client.player_control(mode);
            let receipt = if request["command"] == "look" {
                input
                    .look(serde_json::from_value(request["rotation"].clone())?)
                    .await?
            } else {
                input
                    .select_hotbar(serde_json::from_value(request["slot"].clone())?)
                    .await?
            };
            serde_json::json!({"receipt":receipt,"player":client.player_state().await?})
        }
        "respawn" => serde_json::to_value(client.respawn().await?)?,
        "disconnect" => {
            client.disconnect().await?;
            serde_json::json!({"disconnected":true})
        }
        _ => anyhow::bail!("unknown command"),
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
    println!("{}", serde_json::json!({"ready":true}));
    std::io::stdout().flush()?;
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let request: serde_json::Value = serde_json::from_str(&line)?;
        let output = match execute(&client, &request).await {
            Ok(value) => serde_json::json!({"ok":true,"result":value}),
            Err(error) => {
                serde_json::json!({"ok":false,"kind":error.downcast_ref::<voxrig::Error>().map(|e| format!("{:?}", e.kind())),"error":format!("{error:#}")})
            }
        };
        println!("{output}");
        std::io::stdout().flush()?;
        if request["command"] == "disconnect" {
            break;
        }
    }
    Ok(())
}
