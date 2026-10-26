//! Public-API JSON-line driver for scripts/run_health_log_live.py.
use serde_json::{Value, json};
use std::{io::Write, time::Duration};
use tokio::io::{AsyncBufReadExt, BufReader};
use voxrig::client::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Client::connect(ConnectionConfig::offline_from_env(
        Server::new("127.0.0.1", std::env::var("VOXRIG_PORT")?.parse()?),
        "ClimbingProbe",
    )?)
    .await?;
    let mut health = client.subscribe_health()?;
    client.wait_until_ready().await?;
    println!("{}", json!({"ready":true}));
    std::io::stdout().flush()?;
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let input: Value = serde_json::from_str(&line)?;
        let result: anyhow::Result<Value> = async {
            Ok(match input["command"].as_str().unwrap_or("") {
                "log" => serde_json::to_value(
                    client
                        .health_changes_after(input["after"].as_u64().unwrap_or(0))
                        .await?,
                )?,
                "subscribe" => {
                    health = health.resubscribe();
                    json!({"baseline":health.baseline(), "sequence":health.sequence()})
                }
                "drain" => {
                    let mut events = Vec::new();
                    let end = loop {
                        match health.try_recv() {
                            Ok(event) => events.push(event),
                            Err(error) => break error.to_string(),
                        }
                    };
                    json!({"events":events, "end":end, "sequence":health.sequence()})
                }
                "recv" => serde_json::to_value(
                    tokio::time::timeout(Duration::from_secs(5), health.recv()).await??,
                )?,
                "respawn" => serde_json::to_value(client.respawn().await?)?,
                "disconnect" => {
                    client.disconnect().await?;
                    json!({"disconnected":true, "log":client.health_changes_after(0).await?})
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
