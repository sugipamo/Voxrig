//! Isolated-server probe for entity facts and events: entity type and metadata,
//! entity update / damage / death events, the death message, a kick's reason and the
//! connection status. Prints `CMD <console command>` lines for a driver script.
//! VOXRIG_MINECRAFT_VERSION selects 1.16.1 or 1.21.11; VOXRIG_PORT selects the port.
use anyhow::{Context, bail, ensure};
use std::io::Write;
use std::time::{Duration, Instant};
use voxrig::client::EventKind;
use voxrig::client::prelude::*;

fn name() -> String {
    // A fresh player per run: a player left dead by an earlier run rejoins dead.
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() % 100_000);
    format!("ProbeEnt{n}")
}

fn cmd(line: String) -> anyhow::Result<()> {
    println!("CMD {line}");
    std::io::stdout().flush()?;
    Ok(())
}

/// Wait for an event matching `accept`, returning it and the new cursor.
async fn wait_event(
    client: &Client,
    mut cursor: u64,
    what: &str,
    accept: impl Fn(&EventKind) -> bool,
) -> anyhow::Result<(ClientEvent, u64)> {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let log = client
            .wait_for_events(cursor, Duration::from_secs(8))
            .await?;
        cursor = log.cursor;
        if let Some(event) = log.events.iter().find(|e| accept(&e.kind)) {
            return Ok((*event, cursor));
        }
        if Instant::now() > deadline {
            bail!("timed out waiting for {what}");
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = std::env::var("VOXRIG_PORT")
        .unwrap_or_else(|_| "25565".into())
        .parse()?;
    let name = name();
    let config = ConnectionConfig::offline_from_env(Server::new("127.0.0.1", port), &name)?;
    let client = Client::connect(config).await?;
    client.wait_until_ready().await?;
    println!("STATUS {:?}", client.connection_status().await);
    tokio::time::sleep(Duration::from_secs(2)).await;
    // A previous run may have left the player dead.
    if client
        .player_state()
        .await?
        .health
        .is_some_and(|h| h.value.health <= 0.0)
    {
        client.respawn().await?;
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let player = client.player_state().await?;
    let own = player.entity_id.context("own entity id")?;
    let [x, y, z] = player
        .position
        .context("position")?
        .value
        .map(|v| v.floor() as i32);
    let mut cursor = client.events_after(0).await?.cursor;

    // A pig without AI three blocks east.
    cmd(format!("gamemode survival {name}"))?;
    cmd("kill @e[type=pig]".into())?;
    cmd("kill @e[type=item]".into())?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    cmd(format!(
        "summon minecraft:pig {} {y} {} {{NoAI:1b}}",
        f64::from(x + 3) + 0.5,
        f64::from(z) + 0.5
    ))?;
    let deadline = Instant::now() + Duration::from_secs(8);
    let pig = loop {
        let entities = client.entities().await?;
        if let Some(pig) = entities.entities.iter().find(|e| {
            e.motion.entity.type_name.as_deref() == Some("minecraft:pig")
                && e.motion.entity.spawn_position.value[0] == f64::from(x + 3) + 0.5
                && e.health.as_ref().is_none_or(|h| h.value > 0.0)
        }) {
            break pig.motion.entity.id.native_id();
        }
        ensure!(Instant::now() < deadline, "pig not spawned");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    cursor = client.events_after(cursor).await?.cursor;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let entities = client.entities().await?;
    let observed = entities
        .entities
        .iter()
        .find(|e| e.motion.entity.id.native_id() == pig)
        .context("pig observation")?;
    println!(
        "PIG type={:?} living={:?} health={:?} metadata={:?}",
        observed.motion.entity.type_name,
        observed.living,
        observed.health.as_ref().map(|h| h.value),
        observed
            .metadata
            .iter()
            .map(|(k, v)| (*k, v.value.clone()))
            .collect::<Vec<_>>()
    );
    ensure!(observed.living == Some(true), "pig not reported as living");

    // Teleport: an entity update. Instant damage: a damage event (and 1.16.1 status 2).
    cmd(format!(
        "tp @e[type=pig] {} {y} {}",
        f64::from(x + 4) + 0.5,
        f64::from(z) + 0.5
    ))?;
    let (update, next) = wait_event(
        &client,
        cursor,
        "pig update",
        |k| matches!(k, EventKind::EntityUpdated { native_id } if *native_id == pig),
    )
    .await?;
    cursor = next;
    println!("UPDATED after={:?}", update.received_after);
    cmd("effect give @e[type=pig] instant_damage 1 0".into())?;
    let (damaged, next) = wait_event(
        &client,
        cursor,
        "pig damage",
        |k| matches!(k, EventKind::EntityDamaged { native_id } if *native_id == pig),
    )
    .await?;
    cursor = next;
    println!("DAMAGED after={:?}", damaged.received_after);

    // Own death: a kill event and a death message, then respawn.
    cmd(format!("kill {name}"))?;
    let (_, next) = wait_event(
        &client,
        cursor,
        "own death",
        |k| matches!(k, EventKind::PlayerKilled { native_id } if *native_id == own),
    )
    .await?;
    cursor = next;
    let message = client.death_message().await?.context("death message")?;
    println!(
        "DEATH message={}",
        match &message.value {
            UiText::LegacyJson { json } => json.clone(),
            other => format!("{other:?}").chars().take(60).collect(),
        }
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !client
        .player_state()
        .await?
        .health
        .is_some_and(|h| h.value.health <= 0.0)
    {
        ensure!(Instant::now() < deadline, "zero health not received");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    client.respawn().await?;
    let _ = wait_event(&client, cursor, "respawn", |k| {
        matches!(k, EventKind::WorldChanged | EventKind::PlayerChanged)
    })
    .await?;

    // A kick with a reason closes the connection.
    cmd(format!("kick {name} probe kick reason"))?;
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let status = client.connection_status().await;
        if matches!(status, ConnectionStatus::Closed | ConnectionStatus::Unknown) {
            // The reader may still be reading the last frames (the kick) for a moment.
            for _ in 0..20 {
                if client.disconnect_reason().await?.is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            println!(
                "KICKED status={status:?} reason={:?}",
                client.disconnect_reason().await?
            );
            break;
        }
        if Instant::now() > deadline {
            bail!("kick not observed (status {status:?})");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    println!("DONE");
    Ok(())
}
