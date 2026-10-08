//! Isolated-server probe for common chat and command dispatch and received history.
//! VOXRIG_MINECRAFT_VERSION selects 1.16.1 or 1.21.11; VOXRIG_PORT selects the port.
use std::time::Duration;
use voxrig::client::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = std::env::var("VOXRIG_PORT")
        .unwrap_or_else(|_| "25565".into())
        .parse()?;
    let config = ConnectionConfig::offline_from_env(Server::new("127.0.0.1", port), "ChatProbe")?;
    let client = Client::connect(config).await?;
    client.wait_until_ready().await?;
    let start = client.chat_after(0).await?.receive_sequence;

    client.send_chat("hello from voxrig").await?;
    client.send_command("/me waves from voxrig").await?;
    tokio::time::sleep(Duration::from_millis(
        std::env::var("CHAT_WAIT_MS")
            .unwrap_or_else(|_| "1500".into())
            .parse()?,
    ))
    .await;

    let log = client.chat_after(start).await?;
    for message in &log.messages {
        let text = match &message.message {
            ChatText::Plain(text) => format!("plain {text:?}"),
            ChatText::Component(component) => serde_json::to_string(component)?
                .chars()
                .take(160)
                .collect(),
            ChatText::Undecoded => "undecoded".into(),
        };
        println!(
            "#{} {:?} sender={} name={} {text}",
            message.receive_sequence,
            message.kind,
            message.sender.is_some(),
            message.sender_name.is_some()
        );
    }
    client.disconnect().await?;
    Ok(())
}
