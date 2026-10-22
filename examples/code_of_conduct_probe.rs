//! Code-of-conduct acceptance on an isolated offline Java 1.21.11 server.
//! VOXRIG_PORT selects the port; VOXRIG_CONDUCT must match the server's text.
use std::time::Duration;
use voxrig::client::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = std::env::var("VOXRIG_PORT")
        .unwrap_or_else(|_| "25565".into())
        .parse()?;
    let expected = std::env::var("VOXRIG_CONDUCT")?;
    let client = Client::connect(ConnectionConfig::offline(
        Server::new("127.0.0.1", port),
        "ConductProbe",
        MinecraftVersion::Java1_21_11,
    ))
    .await?;
    client.wait_until_ready().await?;
    let receipt = client
        .code_of_conduct()
        .await?
        .ok_or_else(|| anyhow::anyhow!("no code of conduct received"))?;
    anyhow::ensure!(receipt.value == expected, "code-of-conduct text differs");
    let log = client.events_after(0).await?;
    anyhow::ensure!(log.events.iter().any(|event| {
        event.kind == EventKind::CodeOfConductReceived
            && receipt.source
                == ValueSource::Received {
                    sequence: event.receive_sequence,
                }
    }));
    println!("CONDUCT {}", serde_json::to_string(&receipt)?);
    let before = log.receive_sequence;
    println!("READY sequence={before}");
    for _ in 0..35 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        anyhow::ensure!(client.connection_status().await == ConnectionStatus::Ready);
    }
    let after = client.events_after(log.cursor).await?.receive_sequence;
    anyhow::ensure!(after > before, "no receive progress while connected");
    println!("STAY_CONNECTED seconds=35 sequence={after}");
    client.disconnect().await?;
    anyhow::ensure!(client.code_of_conduct().await? == Some(receipt));
    println!("PASS receipt readable after disconnect");
    Ok(())
}
