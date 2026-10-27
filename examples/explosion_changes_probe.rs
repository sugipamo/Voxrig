//! Public-API JSON-line driver for scripts/run_explosion_changes_live.py.
use serde_json::{Value, json};
use std::io::Write;
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
        let result: anyhow::Result<Value> = async {
            Ok(match input["command"].as_str().unwrap_or("") {
                "explosions" => serde_json::to_value(
                    client
                        .explosions_after(input["after"].as_u64().unwrap_or(0))
                        .await?,
                )?,
                "events" => serde_json::to_value(
                    client
                        .events_after(input["cursor"].as_u64().unwrap_or(0))
                        .await?,
                )?,
                "blocks" => {
                    let positions: Vec<[i32; 3]> =
                        serde_json::from_value(input["positions"].clone())?;
                    let mut states = Vec::new();
                    for p in positions {
                        let chunk = client
                            .chunk([p[0].div_euclid(16), p[2].div_euclid(16)])
                            .await?;
                        states.push(chunk.and_then(|c| c.state_id(p)));
                    }
                    json!({"states":states})
                }
                "disconnect" => {
                    client.disconnect().await?;
                    serde_json::to_value(client.explosions_after(0).await?)?
                }
                _ => anyhow::bail!("unknown command"),
            })
        }
        .await;
        println!(
            "{}",
            result.unwrap_or_else(|e| json!({"error":e.to_string()}))
        );
        std::io::stdout().flush()?;
        if input["command"] == "disconnect" {
            break;
        }
    }
    Ok(())
}
