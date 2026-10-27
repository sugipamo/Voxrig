//! JSON-line BotManager observer used by run_manager_gap_live.py.
use std::sync::Arc;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    sync::{Mutex, broadcast::error::RecvError},
};
use voxrig::versions::java_1_16_1::{BotManager, ConnectionOptions, Event, Player, Server};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use std::io::Write;
    let port = std::env::var("VOXRIG_PORT")?.parse()?;
    let mut options = ConnectionOptions::default();
    options.event_channel_capacity = 1;
    let manager = BotManager::with_options(Server::new("127.0.0.1", port), options);
    let mut stream = manager.subscribe();
    let samples = Arc::new(Mutex::new(Vec::new()));
    let collector = tokio::spawn({
        let samples = samples.clone();
        async move {
            loop {
                let value = match stream.recv().await {
                    Ok(event) => match event.event {
                        Event::ManagerGap {
                            generation,
                            skipped,
                        } => serde_json::json!({
                            "kind":"gap", "username":event.username,
                            "generation":generation.get(), "skipped":skipped}),
                        Event::GameStateChange(change) => serde_json::json!({
                            "kind":"game_state", "username":event.username,
                            "reason":change.reason, "value":change.value}),
                        Event::Disconnected { .. } => serde_json::json!({"kind":"disconnected"}),
                        _ => continue,
                    },
                    Err(RecvError::Lagged(skipped)) => {
                        serde_json::json!({"kind":"aggregate_gap", "skipped":skipped})
                    }
                    Err(RecvError::Closed) => break,
                };
                samples.lock().await.push(value);
            }
        }
    });
    let bot = manager.connect(Player::offline("ClimbingProbe")).await?;
    bot.wait_until_ready().await?;
    println!("{}", serde_json::json!({"ready":true}));
    std::io::stdout().flush()?;
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let command: serde_json::Value = serde_json::from_str(&line)?;
        let value = match command["command"].as_str().unwrap_or_default() {
            "inspect" => serde_json::json!({"generation":bot.connection_generation().get(),
                "game_mode":bot.survival_state().await.game_mode,
                "samples":std::mem::take(&mut *samples.lock().await)}),
            "disconnect" => {
                manager.disconnect_all().await?;
                println!("{}", serde_json::json!({"disconnected":true}));
                std::io::stdout().flush()?;
                collector.abort();
                return Ok(());
            }
            other => anyhow::bail!("unknown command: {other}"),
        };
        println!("{value}");
        std::io::stdout().flush()?;
    }
    manager.disconnect_all().await?;
    collector.abort();
    Ok(())
}
