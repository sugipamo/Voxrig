//! Common-API JSON driver for the isolated multipart dispatch server fixture.
use std::io::Write;
use tokio::io::{AsyncBufReadExt, BufReader};
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
    println!("{}", serde_json::json!({"ready": true}));
    std::io::stdout().flush()?;
    let mut targets = Vec::<EntityPartId>::new();
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let request: serde_json::Value = serde_json::from_str(&line)?;
        let command = request["command"].as_str().unwrap_or_default();
        let value = match command {
            "capture" | "remember" => {
                let capture = client.entities().await?;
                let parents: Vec<_> = capture
                    .entities
                    .iter()
                    .filter(|parent| {
                        parent.motion.entity.type_name.as_deref() == Some("minecraft:ender_dragon")
                    })
                    .collect();
                if command == "remember" {
                    targets.clear();
                    if let Some(parts) = parents
                        .first()
                        .and_then(|parent| parent.derived_parts().ok())
                    {
                        targets.extend(parts.into_iter().filter_map(|part| match part.state {
                            EntityPartState::Derived { target, .. } => Some(target),
                            _ => None,
                        }));
                    }
                }
                serde_json::json!({
                    "session": capture.session,
                    "parents": parents.iter().map(|parent| serde_json::json!({
                        "received": parent,
                        "parts": parent.derived_parts(),
                    })).collect::<Vec<_>>(),
                    "targets": targets,
                })
            }
            "player" => serde_json::to_value(client.player_state().await?)?,
            "attack" => {
                let index = request["index"].as_u64().unwrap_or(0) as usize;
                let target = *targets.get(index).ok_or_else(|| {
                    anyhow::anyhow!("no remembered derived target at index {index}")
                })?;
                let sneaking = request["sneaking"].as_bool().unwrap_or(false);
                let result = if request["mode"].as_str() == Some("creative") {
                    client.creative().attack_entity_part(target, sneaking).await
                } else {
                    client.survival().attack_entity_part(target, sneaking).await
                };
                match result {
                    Ok(receipt) => serde_json::json!({"ok": true, "receipt": receipt}),
                    Err(error) => serde_json::json!({"ok": false, "error": error.to_string()}),
                }
            }
            "disconnect" => {
                client.disconnect().await?;
                serde_json::json!({"disconnected": true})
            }
            _ => anyhow::bail!("unknown command: {command}"),
        };
        println!("{}", serde_json::to_string(&value)?);
        std::io::stdout().flush()?;
        if command == "disconnect" {
            break;
        }
    }
    Ok(())
}
