//! Disposable Java 1.21.11 server probe; forward CMD lines to its console.
use anyhow::{Context, ensure};
use std::{
    io::Write,
    time::{Duration, Instant},
};
use voxrig::client::{
    explosions::{ExplosionPresentation, ReceivedExplosion},
    prelude::*,
};

fn command(line: &str) -> anyhow::Result<()> {
    println!("CMD {line}");
    std::io::stdout().flush()?;
    Ok(())
}
async fn observe(
    client: &Client,
    after: u64,
    event_cursor: u64,
) -> anyhow::Result<ReceivedExplosion> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let log = client.explosions_after(after).await?;
        if let Some(record) = log.explosions.into_iter().next() {
            ensure!(
                client
                    .events_after(event_cursor)
                    .await?
                    .events
                    .iter()
                    .any(|e| e.receive_sequence == record.receive_sequence
                        && e.kind == EventKind::ExplosionReceived),
                "notification ordinal mismatch"
            );
            ensure!(
                matches!(
                    record.explosion.presentation,
                    ExplosionPresentation::Decoded { .. }
                ),
                "vanilla presentation undecoded"
            );
            println!("OBS {}", serde_json::to_string(&record)?);
            std::io::stdout().flush()?;
            return Ok(record);
        }
        ensure!(Instant::now() < deadline, "explosion observation timed out");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
async fn scenario(client: &Client) -> anyhow::Result<()> {
    for line in [
        "gamerule minecraft:spawn_mobs false",
        "gamerule minecraft:mob_griefing false",
        "fill -8 64 -4 24 64 4 minecraft:stone",
        "fill -8 65 -4 24 68 4 minecraft:air",
        "gamemode survival Q12Observer",
        "effect give Q12Observer minecraft:resistance infinite 255 true",
        "tp Q12Observer 0.5 65 0.5 0 0",
    ] {
        command(line)?;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    client.start_packet_trace(4_194_304).await?;
    let mut records = Vec::new();
    for (name, command_text) in [
        ("tnt_far", "summon minecraft:tnt 18.5 65 0.5 {fuse:1}"),
        ("tnt_near", "summon minecraft:tnt 2.5 65 0.5 {fuse:1}"),
        (
            "creeper_far",
            "summon minecraft:creeper 18.5 65 0.5 {Fuse:1s,ignited:1b,NoAI:1b}",
        ),
        (
            "creeper_near",
            "summon minecraft:creeper 2.5 65 0.5 {Fuse:1s,ignited:1b,NoAI:1b}",
        ),
        (
            "wind_far",
            "summon minecraft:wind_charge 18.5 65.1 0.5 {Motion:[0.0d,-1.0d,0.0d]}",
        ),
        (
            "wind_near",
            "summon minecraft:wind_charge 2.5 65.1 0.5 {Motion:[0.0d,-1.0d,0.0d]}",
        ),
    ] {
        command("tp Q12Observer 0.5 65 0.5 0 0")?;
        command("fill -8 64 -4 24 64 4 minecraft:stone")?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let after = client.explosions_after(0).await?.receive_sequence;
        let event_cursor = client.events_after(0).await?.cursor;
        command(command_text)?;
        let record = observe(client, after, event_cursor)
            .await
            .with_context(|| name)?;
        ensure!(
            record.explosion.player_knockback.is_some() == name.ends_with("near"),
            "unexpected impulse presence for {name}"
        );
        ensure!(
            record.session
                == records
                    .first()
                    .map_or(record.session, |r: &ReceivedExplosion| r.session),
            "world ownership changed"
        );
        if let Some(previous) = records.last() {
            ensure!(
                previous.receive_sequence < record.receive_sequence,
                "receive order changed"
            );
        }
        records.push(record);
    }
    Ok(())
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = std::env::var("VOXRIG_PORT")
        .unwrap_or_else(|_| "25565".into())
        .parse()?;
    let client = Client::connect(ConnectionConfig::offline(
        Server::new("127.0.0.1", port),
        "Q12Observer",
        MinecraftVersion::Java1_21_11,
    ))
    .await?;
    client.wait_until_ready().await?;
    let result = scenario(&client).await;
    let trace = client.stop_packet_trace().await?;
    if let Ok(path) = std::env::var("VOXRIG_EFFECT_TRACE") {
        std::fs::write(path, serde_json::to_vec_pretty(&trace)?)?;
    }
    let closed = client.disconnect().await;
    result?;
    closed?;
    ensure!(
        client.explosions_after(0).await?.explosions.len() == 6,
        "closed history missing"
    );
    println!(
        "PASS tnt creeper wind optional_impulse notification_order world_owner closed_history"
    );
    Ok(())
}
