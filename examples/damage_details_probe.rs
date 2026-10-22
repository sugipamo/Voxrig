//! Java 1.21.11 disposable-server probe; forward CMD lines to its console.
use anyhow::{Context, ensure};
use std::{
    io::Write,
    time::{Duration, Instant},
};
use voxrig::client::{
    effects::{PlayEffect, ReceivedEffect},
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
    accept: impl Fn(&PlayEffect) -> bool,
) -> anyhow::Result<ReceivedEffect> {
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        let log = client.effects_after(after).await?;
        if let Some(record) = log.effects.into_iter().find(|r| accept(&r.effect)) {
            let kind = match &record.effect {
                PlayEffect::Damage(d) => EventKind::EntityDamaged {
                    native_id: d.entity_id,
                },
                PlayEffect::HurtAnimation { entity_id, .. } => EventKind::HurtAnimationReceived {
                    native_id: *entity_id,
                },
                PlayEffect::CombatEntered | PlayEffect::CombatEnded { .. } => {
                    EventKind::CombatChanged
                }
                PlayEffect::CombatKill { player_id, .. } => EventKind::PlayerKilled {
                    native_id: *player_id,
                },
                _ => anyhow::bail!("unexpected effect"),
            };
            ensure!(
                client
                    .events_after(0)
                    .await?
                    .events
                    .iter()
                    .any(|e| e.receive_sequence == record.receive_sequence && e.kind == kind),
                "notification sequence mismatch"
            );
            println!("OBS {}", serde_json::to_string(&record)?);
            std::io::stdout().flush()?;
            return Ok(record);
        }
        ensure!(
            Instant::now() < deadline,
            "damage/combat observation timed out"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
async fn scenario(client: &Client) -> anyhow::Result<()> {
    for line in [
        "gamerule minecraft:spawn_mobs false",
        "fill -8 64 -4 8 64 4 minecraft:stone",
        "fill -8 65 -4 8 68 4 minecraft:air",
        "gamemode survival Q6Observer",
        "tp Q6Observer 0.5 65 0.5 0 0",
        "summon minecraft:pig 2 65 0 {NoAI:1b,Silent:1b,Tags:[\"q6_direct\"]}",
        "summon minecraft:cow 3 65 0 {NoAI:1b,Silent:1b,Tags:[\"q6_cause\"]}",
    ] {
        command(line)?;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    let own = client
        .player_state()
        .await?
        .entity_id
        .context("own entity ID")?;
    let registries = client.server_registry_state().await?;
    println!(
        "REG {}",
        serde_json::json!({
            "stamp": registries.stamp(),
            "damage_type": registries.registries().get("minecraft:damage_type"),
        })
    );
    client.start_packet_trace(2_097_152).await?;
    let after = client.effects_after(0).await?.receive_sequence;
    command("damage Q6Observer 1 minecraft:generic at -12.5 64.25 8.125")?;
    observe(client, after, |e| matches!(e, PlayEffect::Damage(d) if d.entity_id == own && d.source_type_name == "minecraft:generic" && d.cause_entity_id.is_none() && d.direct_entity_id.is_none() && d.source_position == Some([-12.5,64.25,8.125]))).await?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let after = client.effects_after(0).await?.receive_sequence;
    command(
        "damage Q6Observer 1 minecraft:mob_attack by @e[tag=q6_direct,limit=1] from @e[tag=q6_cause,limit=1]",
    )?;
    let record = observe(client, after, |e| matches!(e, PlayEffect::Damage(d) if d.source_type_name == "minecraft:mob_attack" && d.cause_entity_id.is_some() && d.direct_entity_id.is_some() && d.cause_entity_id != d.direct_entity_id && d.source_position.is_none())).await?;
    if let PlayEffect::Damage(d) = &record.effect {
        let registries = client.server_registry_state().await?;
        ensure!(
            registries.resolve(&d.source_type)?.name == d.source_type_name,
            "received registry resolution differs"
        );
    }
    observe(client, after, |e| matches!(e, PlayEffect::HurtAnimation { entity_id, yaw } if *entity_id == own && yaw.is_finite())).await?;
    observe(client, after, |e| matches!(e, PlayEffect::CombatEntered)).await?;
    observe(
        client,
        after,
        |e| matches!(e, PlayEffect::CombatEnded { duration_ticks } if *duration_ticks >= 0),
    )
    .await?;
    let after = client.effects_after(0).await?.receive_sequence;
    command("damage Q6Observer 100 minecraft:generic_kill")?;
    let killed = observe(
        client,
        after,
        |e| matches!(e, PlayEffect::CombatKill { player_id, .. } if *player_id == own),
    )
    .await?;
    if let PlayEffect::CombatKill { message, .. } = killed.effect {
        let death = client.death_message().await?.context("death message")?;
        ensure!(
            death.value == message
                && death.source
                    == ValueSource::Received {
                        sequence: killed.receive_sequence
                    },
            "death message compatibility differs"
        );
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
        "Q6Observer",
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
        !client.effects_after(0).await?.effects.is_empty(),
        "closed history missing"
    );
    println!(
        "PASS damage source_position cause direct registry hurt combat_enter combat_end combat_kill closed_history"
    );
    Ok(())
}
