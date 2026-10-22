//! Disposable Java 1.21.11 server probe. A driver forwards `CMD` lines to the
//! server console. Two ordinary clients verify sound, particles, pickup and
//! another player's crack progress. VOXRIG_PORT selects the isolated server.
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
    after: &mut u64,
    kind: EventKind,
    accept: impl Fn(&PlayEffect) -> bool,
) -> anyhow::Result<ReceivedEffect> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let log = client.effects_after(*after).await?;
        *after = log.receive_sequence;
        if let Some(record) = log.effects.into_iter().find(|e| accept(&e.effect)) {
            let events = client.events_after(0).await?;
            ensure!(
                events
                    .events
                    .iter()
                    .any(|e| e.receive_sequence == record.receive_sequence && e.kind == kind),
                "missing notification for payload"
            );
            println!("OBS {}", serde_json::to_string(&record)?);
            std::io::stdout().flush()?;
            return Ok(record);
        }
        ensure!(
            Instant::now() < deadline,
            "effect {kind:?} was not received"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
async fn scenario(observer: &Client, miner: &Client) -> anyhow::Result<()> {
    for line in [
        "gamerule minecraft:spawn_mobs false",
        "gamerule minecraft:do_weather_cycle false",
        "weather clear",
        "fill -8 64 -4 8 64 4 minecraft:stone",
        "fill -8 65 -4 8 68 4 minecraft:air",
        "gamemode survival Q5Observer",
        "gamemode survival Q5Miner",
        "tp Q5Observer -2.5 65 0.5 270 0",
        "tp Q5Miner 0.5 65 0.5 270 0",
        "clear Q5Observer",
        "clear Q5Miner",
    ] {
        command(line)?;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    miner.survival().select_hotbar(0).await?;
    observer.start_packet_trace(2_097_152).await?;
    let mut after = observer.effects_after(0).await?.receive_sequence;
    command("playsound minecraft:block.note_block.pling master Q5Observer -2.5 65 0.5 1 1")?;
    let sound = observe(observer, &mut after, EventKind::SoundReceived, |e| {
        matches!(e, PlayEffect::Sound(_))
    })
    .await?;
    if let PlayEffect::Sound(s) = sound.effect {
        ensure!(
            s.volume == 1.0 && s.pitch == 1.0,
            "sound volume/pitch differ"
        );
    }
    command("stopsound Q5Observer master minecraft:block.note_block.pling")?;
    observe(observer,&mut after,EventKind::SoundStopped,|e|matches!(e,PlayEffect::SoundStopped{source:Some(0),name:Some(name)} if name=="minecraft:block.note_block.pling")).await?;
    command(
        "particle minecraft:dust{color:[1.0,0.0,0.0],scale:1.25} -2.5 66 0.5 0.1 0.2 0.3 0.5 4 force Q5Observer",
    )?;
    observe(
        observer,
        &mut after,
        EventKind::ParticlesReceived,
        |e| matches!(e,PlayEffect::Particles(p) if p.native_id==14 && p.count==4),
    )
    .await?;
    command(
        "execute at Q5Observer run summon minecraft:item ~ ~ ~ {Item:{id:\"minecraft:stone\",count:3},PickupDelay:0s}",
    )?;
    let own = observer
        .player_state()
        .await?
        .entity_id
        .context("observer entity ID")?;
    observe(
        observer,
        &mut after,
        EventKind::ItemPickupReceived,
        |e| matches!(e,PlayEffect::ItemPickup{collector_id,amount:3,..} if *collector_id==own),
    )
    .await?;
    command("setblock 2 65 0 minecraft:stone")?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let digger = miner.clone();
    let mining =
        tokio::spawn(async move { digger.survival().dig([2, 65, 0], BlockFace::West).await });
    let miner_id = miner
        .player_state()
        .await?
        .entity_id
        .context("miner entity ID")?;
    let progress=observe(observer,&mut after,EventKind::BlockDestructionReceived,|e|matches!(e,PlayEffect::BlockDestruction{breaker_id,position:[2,65,0],progress:0..=9} if *breaker_id==miner_id)).await;
    // Complete the ordinary dig before closing both connections, even if the
    // observer's assertion failed. No raw injection or synthetic crack event.
    let finished = mining.await?;
    progress?;
    ensure!(finished?.removed, "miner did not observe removal");
    observe(observer, &mut after, EventKind::LevelEventReceived, |e| {
        matches!(
            e,
            PlayEffect::LevelEvent {
                event_id: 2001,
                position: [2, 65, 0],
                ..
            }
        )
    })
    .await?;
    let trace = observer.stop_packet_trace().await?;
    if let Ok(path) = std::env::var("VOXRIG_EFFECT_TRACE") {
        std::fs::write(path, serde_json::to_vec_pretty(&trace)?)?;
    }
    Ok(())
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = std::env::var("VOXRIG_PORT")
        .unwrap_or_else(|_| "25565".into())
        .parse()?;
    let observer = Client::connect(ConnectionConfig::offline(
        Server::new("127.0.0.1", port),
        "Q5Observer",
        MinecraftVersion::Java1_21_11,
    ))
    .await?;
    observer.wait_until_ready().await?;
    let miner = Client::connect(ConnectionConfig::offline(
        Server::new("127.0.0.1", port),
        "Q5Miner",
        MinecraftVersion::Java1_21_11,
    ))
    .await?;
    miner.wait_until_ready().await?;
    let result = scenario(&observer, &miner).await;
    let miner_closed = miner.disconnect().await;
    let observer_closed = observer.disconnect().await;
    result?;
    miner_closed?;
    observer_closed?;
    // Histories remain readable after closure.
    ensure!(!observer.effects_after(0).await?.effects.is_empty());
    println!("PASS sound stop_sound particles pickup block_destruction level_event closed_history");
    Ok(())
}
