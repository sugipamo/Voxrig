//! Common-API connection/read/disconnect driver for isolated vanilla servers.
use std::io::Write;
use tokio::io::{AsyncBufReadExt, BufReader};
use voxrig::client::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = std::env::var("VOXRIG_PORT")?.parse()?;
    let mut clients = Vec::new();
    println!("{}", serde_json::json!({"ready": true}));
    std::io::stdout().flush()?;
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let request: serde_json::Value = serde_json::from_str(&line)?;
        let command = request["command"].as_str().unwrap_or_default();
        let value = match command {
            "connect" => {
                let name = request["name"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("missing name"))?;
                let client = Client::connect(ConnectionConfig::offline_from_env(
                    Server::new("127.0.0.1", port),
                    name,
                )?)
                .await?;
                client.wait_until_ready().await?;
                clients.push(client);
                serde_json::json!({"connected": true, "index": clients.len() - 1})
            }
            "read" => {
                let index = request["index"].as_u64().unwrap_or(0) as usize;
                let client = &clients[index];
                let player = client.player_state().await?;
                let chunks = client.loaded_chunks().await?;
                serde_json::json!({"player": player, "chunk_count": chunks.chunks.len(),
                    "chunk_receive_sequence": chunks.receive_sequence})
            }
            "disconnect" => {
                let index = request["index"].as_u64().unwrap_or(0) as usize;
                clients[index].disconnect().await?;
                serde_json::json!({"disconnected": true})
            }
            "quit" => serde_json::json!({"quit": true}),
            _ => anyhow::bail!("unknown command: {command}"),
        };
        println!("{}", serde_json::to_string(&value)?);
        std::io::stdout().flush()?;
        if command == "quit" {
            break;
        }
    }
    Ok(())
}
