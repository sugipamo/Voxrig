//! Required-resource-pack negotiation on an isolated offline Java 1.21.11 server.
//! VOXRIG_PORT selects the port; the probe stays ready for 35 seconds.
use std::time::Duration;
use voxrig::client::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = std::env::var("VOXRIG_PORT")
        .unwrap_or_else(|_| "25565".into())
        .parse()?;
    let client = Client::connect(ConnectionConfig::offline(
        Server::new("127.0.0.1", port),
        "PackProbe",
        MinecraftVersion::Java1_21_11,
    ))
    .await?;
    client.wait_until_ready().await?;
    let request = client
        .resource_pack_request()
        .await?
        .ok_or_else(|| anyhow::anyhow!("no resource-pack request received"))?;
    anyhow::ensure!(request.value.forced, "server must require the pack");
    anyhow::ensure!(request.value.url.starts_with("https://"));
    anyhow::ensure!(request.value.hash.len() == 40);
    let log = client.events_after(0).await?;
    anyhow::ensure!(log.events.iter().any(|event| {
        event.kind
            == EventKind::ResourcePackRequested {
                uuid: request.value.uuid,
            }
            && request.source
                == ValueSource::Received {
                    sequence: event.receive_sequence,
                }
    }));
    println!("REQUEST {}", serde_json::to_string(&request)?);
    let before = log.receive_sequence;
    println!("READY sequence={before}");
    for _ in 0..35 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        anyhow::ensure!(client.connection_status().await == ConnectionStatus::Ready);
    }
    let after = client.events_after(log.cursor).await?.receive_sequence;
    anyhow::ensure!(
        after > before,
        "no receive progress while staying connected"
    );
    println!("STAY_CONNECTED seconds=35 sequence={after}");
    client.disconnect().await?;
    anyhow::ensure!(client.resource_pack_request().await? == Some(request));
    println!("PASS receipt readable after disconnect");
    Ok(())
}
