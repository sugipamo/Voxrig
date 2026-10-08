//! Isolated-server probe for the common bounded waits. Prints `TARGET x y z`,
//! then expects an operator to place stone there and send one chat line.
//! VOXRIG_MINECRAFT_VERSION selects 1.16.1 or 1.21.11; VOXRIG_PORT selects the port.
use std::io::Write;
use std::time::{Duration, Instant};
use voxrig::client::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = std::env::var("VOXRIG_PORT")
        .unwrap_or_else(|_| "25565".into())
        .parse()?;
    let config = ConnectionConfig::offline_from_env(Server::new("127.0.0.1", port), "WaitProbe")?;
    let client = Client::connect(config).await?;
    client.wait_until_ready().await?;

    let feet = client
        .player_state()
        .await?
        .position
        .ok_or_else(|| anyhow::anyhow!("no position"))?
        .value;
    let base = feet.map(|v| v.floor() as i32);
    let around = Region {
        min: [base[0] - 1, base[1] - 1, base[2] - 1],
        max: [base[0] + 1, base[1] + 1, base[2] + 1],
    };
    let loaded = client
        .wait_for_loaded(around, Duration::from_secs(10))
        .await?;
    println!("LOADED cells={}", loaded.world.blocks.len());

    let target = [base[0], base[1] + 3, base[2]];
    let timeout = client
        .wait_for_block(target, Duration::from_millis(300), |s| {
            s.is_some_and(|s| s.name == "minecraft:diamond_block")
        })
        .await;
    println!("TIMEOUT {:?}", timeout.err().map(|e| e.kind()));

    let cursor = client.chat_after(0).await?.receive_sequence;
    println!("TARGET {} {} {}", target[0], target[1], target[2]);
    std::io::stdout().flush()?;
    let started = Instant::now();
    let placed = client
        .wait_for_block(target, Duration::from_secs(20), |s| {
            s.is_some_and(|s| s.name == "minecraft:stone")
        })
        .await?;
    println!(
        "STONE seq={} after={:?}",
        placed.player.receive_sequence,
        started.elapsed()
    );
    let chat = client
        .wait_for_chat(cursor, Duration::from_secs(20))
        .await?;
    println!("CHAT messages={}", chat.messages.len());
    let next = client
        .wait_for_receive(chat.receive_sequence, Duration::from_secs(10))
        .await?;
    println!("RECEIVE {} > {}", next, chat.receive_sequence);
    client.disconnect().await?;
    let closed = client.wait_for_receive(next, Duration::from_secs(5)).await;
    println!("CLOSED {:?}", closed.err().map(|e| e.kind()));
    Ok(())
}
