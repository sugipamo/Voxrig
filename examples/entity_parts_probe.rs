//! Read-only multipart receipt/model probe. No attacks or world edits.
//! VOXRIG_MINECRAFT_VERSION selects the version; VOXRIG_PORT selects localhost
//! port. VOXRIG_PROBE_SECONDS defaults to 15. Use an isolated test server.
use std::time::{Duration, Instant};
use voxrig::client::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = std::env::var("VOXRIG_PORT")
        .unwrap_or_else(|_| "25565".into())
        .parse()?;
    let seconds = std::env::var("VOXRIG_PROBE_SECONDS")
        .unwrap_or_else(|_| "15".into())
        .parse()?;
    let config = ConnectionConfig::offline_from_env(Server::new("127.0.0.1", port), "ProbeParts")?;
    let client = Client::connect(config).await?;
    client.wait_until_ready().await?;
    println!("READY ProbeParts");
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut previous = String::new();
    while Instant::now() < deadline {
        let captured = client.entities().await?;
        for parent in &captured.entities {
            if parent.motion.entity.type_name.as_deref() == Some("minecraft:ender_dragon") {
                let parts = parent.derived_parts();
                // Derived parts stay outside the received parent observation.
                let line = serde_json::to_string(&serde_json::json!({
                    "parent": parent.motion.entity.id,
                    "phase": parent.metadata.get(&15),
                    "health": parent.health,
                    "parts": parts,
                }))?;
                if line != previous {
                    println!("{line}");
                    previous = line;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    client.disconnect().await?;
    Ok(())
}
