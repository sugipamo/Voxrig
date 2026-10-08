//! Isolated-server probe for common change notifications. Prints `READY x y z`,
//! then expects an operator to cause world, entity, chat, UI and inventory changes.
//! VOXRIG_MINECRAFT_VERSION selects 1.16.1 or 1.21.11; VOXRIG_PORT selects the port.
use std::collections::BTreeMap;
use std::io::Write;
use std::time::{Duration, Instant};
use voxrig::client::prelude::*;

fn label(kind: &EventKind) -> &'static str {
    match kind {
        EventKind::BlocksChanged { .. } => "BlocksChanged",
        EventKind::ChunkLoaded { .. } => "ChunkLoaded",
        EventKind::ChunkUnloaded { .. } => "ChunkUnloaded",
        EventKind::InventoryChanged => "InventoryChanged",
        EventKind::ScreenChanged => "ScreenChanged",
        EventKind::PlayerChanged => "PlayerChanged",
        EventKind::WorldChanged => "WorldChanged",
        EventKind::EntitySpawned { .. } => "EntitySpawned",
        EventKind::EntityRemoved { .. } => "EntityRemoved",
        EventKind::ChatReceived => "ChatReceived",
        EventKind::UiChanged => "UiChanged",
        EventKind::Disconnected => "Disconnected",
        _ => "Other",
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = std::env::var("VOXRIG_PORT")
        .unwrap_or_else(|_| "25565".into())
        .parse()?;
    let config = ConnectionConfig::offline_from_env(Server::new("127.0.0.1", port), "EventProbe")?;
    let client = Client::connect(config).await?;
    client.wait_until_ready().await?;

    let initial = client.events_after(0).await?;
    let mut join = BTreeMap::new();
    for event in &initial.events {
        *join.entry(label(&event.kind)).or_insert(0) += 1;
    }
    println!("JOIN {join:?}");

    let feet = client.player_state().await?.position.unwrap().value;
    let base = feet.map(|v| v.floor() as i32);
    println!("READY {} {} {}", base[0], base[1], base[2]);
    std::io::stdout().flush()?;

    let mut cursor = initial.cursor;
    let mut seen = BTreeMap::new();
    let mut last_sequence = initial.receive_sequence;
    let mut ordered = true;
    let deadline = Instant::now() + Duration::from_secs(8);
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        let Ok(log) = client.wait_for_events(cursor, left).await else {
            break;
        };
        for event in &log.events {
            ordered &= event.receive_sequence >= last_sequence;
            last_sequence = event.receive_sequence;
            *seen.entry(label(&event.kind)).or_insert(0) += 1;
            if let EventKind::BlocksChanged { min, max } = event.kind {
                let target = [base[0], base[1] + 3, base[2]];
                if (0..3).all(|a| min[a] <= target[a] && target[a] <= max[a]) {
                    println!("BLOCK covers setblock target: {min:?}..{max:?}");
                }
            }
        }
        cursor = log.cursor;
    }
    println!("SEEN {seen:?} ordered={ordered}");

    client.disconnect().await?;
    let after = client.events_after(cursor).await?;
    println!(
        "AFTER_CLOSE {:?}",
        after
            .events
            .iter()
            .map(|e| label(&e.kind))
            .collect::<Vec<_>>()
    );
    Ok(())
}
