//! Common-API connection/read/disconnect driver for isolated vanilla servers.
use std::io::Write;
use tokio::io::{AsyncBufReadExt, BufReader};
use voxrig::client::prelude::*;

#[tokio::main(worker_threads = 4)]
async fn main() -> anyhow::Result<()> {
    let port = std::env::var("VOXRIG_PORT")?.parse()?;
    let mut clients = Vec::new();
    let mut cursors = Vec::new();
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
                cursors.push(0_u64);
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
            "sample" => {
                let ms = request["ms"].as_u64().unwrap_or(1000).min(5000);
                let mut tasks = tokio::task::JoinSet::new();
                for (index, client) in clients.iter().cloned().enumerate() {
                    let mut cursor = cursors[index];
                    tasks.spawn(async move {
                        let started = std::time::Instant::now();
                        let mut captures = 0;
                        let mut max_capture_ms = 0;
                        let mut sequence = 0;
                        let mut entity_count = 0;
                        let mut wolf_count = 0;
                        while started.elapsed().as_millis() < u128::from(ms) {
                            let capture_started = std::time::Instant::now();
                            client.player_state().await?;
                            client.loaded_chunks().await?;
                            let entities = client.entities().await?;
                            entity_count = entities.entities.len();
                            wolf_count = entities.entities.iter().filter(|e|
                                e.motion.entity.type_name.as_deref() == Some("minecraft:wolf")
                            ).count();
                            let events = client.events_after(cursor).await?;
                            cursor = events.cursor;
                            sequence = events.receive_sequence;
                            captures += 3;
                            max_capture_ms = max_capture_ms.max(capture_started.elapsed().as_millis());
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        }
                        Ok::<_, anyhow::Error>((index, cursor, serde_json::json!({
                            "index": index, "captures": captures, "max_capture_ms": max_capture_ms,
                            "receive_sequence": sequence, "entity_count": entity_count, "wolf_count": wolf_count,
                        })))
                    });
                }
                let mut samples = Vec::new();
                while let Some(result) = tasks.join_next().await {
                    let (index, cursor, value) = result??;
                    cursors[index] = cursor;
                    samples.push(value);
                }
                samples.sort_by_key(|v| v["index"].as_u64());
                serde_json::json!({"samples": samples})
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
