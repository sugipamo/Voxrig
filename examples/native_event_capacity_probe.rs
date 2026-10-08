//! JSON-line observer for the native-event capacity migration contract.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use std::io::Write;
    use tokio::{
        io::{AsyncBufReadExt, BufReader},
        sync::broadcast::error::TryRecvError,
    };
    use voxrig::client::prelude::*;
    use voxrig::versions::java_1_16_1::Event;
    let port = std::env::var("VOXRIG_PORT")?.parse()?;
    let mut config =
        ConnectionConfig::offline_from_env(Server::new("127.0.0.1", port), "ClimbingProbe")?;
    if config.version == voxrig::MinecraftVersion::Java1_16_1 {
        config.limits.native_event_channel_capacity = Some(8192);
    }
    let capacity = config.limits.native_event_channel_capacity;
    let client = Client::connect(config).await?;
    client.wait_until_ready().await?;
    let mut native = if capacity.is_some() {
        Some(client.java_1_16_1()?.subscribe())
    } else {
        None
    };
    println!("{}", serde_json::json!({"ready": true}));
    std::io::stdout().flush()?;
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let command: serde_json::Value = serde_json::from_str(&line)?;
        let value = match command["command"].as_str().unwrap_or_default() {
            "inspect" => {
                serde_json::json!({"capacity":capacity,"player":client.player_state().await?})
            }
            "drain" => {
                let mut total = 0;
                let mut samples = Vec::new();
                if let Some(native) = &mut native {
                    loop {
                        match native.try_recv() {
                            Ok(event) => {
                                total += 1;
                                let (kind, entity) = match event {
                                    Event::EntitySpawned(entity) => ("spawn", entity),
                                    Event::EntityUpdated(entity) => ("update", entity),
                                    _ => continue,
                                };
                                if entity.type_name == Some("armor_stand") {
                                    samples.push(serde_json::json!({"kind":kind,"id":entity.entity_id,
                                        "uuid":entity.uuid,"position":[entity.position.x,entity.position.y,entity.position.z],
                                        "velocity":[entity.velocity.x,entity.velocity.y,entity.velocity.z]}));
                                }
                            }
                            Err(TryRecvError::Empty) => break,
                            Err(error) => anyhow::bail!("native event source lost events: {error}"),
                        }
                    }
                }
                let common_gap = match client.events_after(0).await {
                    Ok(_) => false,
                    Err(error) if error.kind() == voxrig::ErrorKind::State => true,
                    Err(error) => return Err(error.into()),
                };
                serde_json::json!({"capacity":capacity,"total":total,"samples":samples,"common_gap":common_gap})
            }
            "disconnect" => {
                client.disconnect().await?;
                println!("{}", serde_json::json!({"disconnected":true}));
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
