//! Common-API driver for scripts/run_world_decode_failure_live.py.
use serde_json::{Value, json};
use std::{io::Write, time::Duration};
use tokio::io::{AsyncBufReadExt, BufReader};
use voxrig::client::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Client::connect(ConnectionConfig::offline(
        Server::new("127.0.0.1", std::env::var("VOXRIG_PORT")?.parse()?),
        "ClimbingProbe",
        MinecraftVersion::Java1_16_1,
    ))
    .await?;
    client.wait_until_ready().await?;
    println!("{}", json!({"ready":true}));
    std::io::stdout().flush()?;
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let input: Value = serde_json::from_str(&line)?;
        let cursor = input["cursor"].as_u64().unwrap_or(0);
        let result: anyhow::Result<Value> = async {
            Ok(match input["command"].as_str().unwrap_or("") {
                "events" => serde_json::to_value(client.events_after(cursor).await?)?,
                "wait" => serde_json::to_value(
                    client
                        .wait_for_events(cursor, Duration::from_secs(5))
                        .await?,
                )?,
                "observe" => {
                    let chunk = client.chunk([0, 0]).await?;
                    let context = client.chunk_context([0, 0]).await?;
                    let loaded = client.loaded_chunks().await?;
                    json!({"chunk_present":chunk.is_some(), "context_present":context.is_some(),
                        "loaded":loaded.chunks.len(),
                        "state":chunk.as_ref().and_then(|c| c.state_id([0, 64, 0])),
                        "sky_light":chunk.as_ref().and_then(|c| c.sky_light([0, 65, 0])),
                        "block_light":chunk.as_ref().and_then(|c| c.block_light([0, 65, 0]))})
                }
                "context" => json!({"present":client.chunk_context([0, 0]).await?.is_some()}),
                "loaded" => json!({"count":client.loaded_chunks().await?.chunks.len()}),
                "snapshot" => match client.received_player_snapshot().await {
                    Ok(_) => json!({"ok":true}),
                    Err(e) => json!({"error":e.to_string(), "kind":format!("{:?}", e.kind())}),
                },
                "finish" => json!({"finished":true}),
                _ => anyhow::bail!("unknown command"),
            })
        }
        .await;
        println!(
            "{}",
            result.unwrap_or_else(|e| json!({"error":e.to_string()}))
        );
        std::io::stdout().flush()?;
        if input["command"] == "finish" {
            break;
        }
    }
    Ok(())
}
