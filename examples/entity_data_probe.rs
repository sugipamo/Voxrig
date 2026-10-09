//! Isolated-server probe for named entity data, the world time and eye-in-water.
//! Summons NoAI mobs with known data, reads them through `EntityObservation::data`,
//! then floods the player's eye cell. Prints `CMD <console command>` lines for a
//! driver script. VOXRIG_MINECRAFT_VERSION selects 1.16.1 or 1.21.11; VOXRIG_PORT the port.
use anyhow::{Context, ensure};
use std::io::Write;
use std::time::{Duration, Instant};
use voxrig::client::prelude::*;
use voxrig::client::{EntityDataField as F, EntityDataSource, EntityDataValue, EntityObservation};

fn cmd(line: String) -> anyhow::Result<()> {
    println!("CMD {line}");
    std::io::stdout().flush()?;
    Ok(())
}

fn value(entity: &EntityObservation, field: F) -> anyhow::Result<(EntityDataValue, bool)> {
    let reading = entity
        .data(field)
        .with_context(|| format!("{field:?} unknown"))?;
    let received = matches!(reading.source, EntityDataSource::Received { .. });
    println!(
        "  {field:?} index={} value={:?} received={received}",
        reading.index, reading.value
    );
    Ok((reading.value, received))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = std::env::var("VOXRIG_PORT")
        .unwrap_or_else(|_| "25565".into())
        .parse()?;
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() % 100_000);
    let name = format!("ProbeData{n}");
    let config = ConnectionConfig::offline_from_env(Server::new("127.0.0.1", port), &name)?;
    let client = Client::connect(config).await?;
    client.wait_until_ready().await?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let player = client.player_state().await?;
    let [x, y, z] = player
        .position
        .context("position")?
        .value
        .map(|v| v.floor() as i32);
    cmd(format!("gamemode creative {name}"))?;
    cmd("difficulty easy".into())?; // Peaceful removes hostile mobs on spawn.
    cmd(format!(
        "kill @e[type=!player,x={},y={y},z={},dx=12,dy=3,dz=3]",
        x + 2,
        z + 2
    ))?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let mobs = [
        ("creeper", "{NoAI:1b,powered:1b}"),
        ("slime", "{NoAI:1b,Size:2}"),
        ("zombie", "{NoAI:1b,IsBaby:1b}"),
        ("wolf", "{NoAI:1b,AngerTime:600}"),
        ("wolf", "{NoAI:1b}"),
    ];
    for (i, (kind, tag)) in mobs.iter().enumerate() {
        cmd(format!(
            "summon minecraft:{kind} {} {y} {} {tag}",
            x + 3 + 2 * i as i32,
            z + 3
        ))?;
    }
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let entities = client.entities().await?;
        let ours = entities
            .entities
            .iter()
            .filter(|e| e.motion.entity.spawn_position.value[2].floor() == f64::from(z + 3))
            .count();
        if ours >= mobs.len() {
            break;
        }
        ensure!(Instant::now() < deadline, "mobs not spawned");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    let entities = client.entities().await?;
    let time = client
        .player_state()
        .await?
        .world_time
        .context("world time")?;
    println!("TIME {:?}", time.value);
    for e in entities
        .entities
        .iter()
        .filter(|e| e.motion.entity.spawn_position.value[2].floor() == f64::from(z + 3))
    {
        println!(
            "ENTITY {:?} received_keys={:?}",
            e.motion.entity.type_name,
            e.metadata.keys().collect::<Vec<_>>()
        );
    }
    let at = |i: usize| {
        let spawn_x = f64::from(x + 3 + 2 * i as i32);
        entities
            .entities
            .iter()
            .find(|e| {
                let at = e.motion.entity.spawn_position.value;
                at[0].floor() == spawn_x && at[2].floor() == f64::from(z + 3)
            })
            .with_context(|| format!("mob {i}"))
    };
    let creeper = at(0)?;
    println!("CREEPER complete={}", creeper.metadata_complete);
    ensure!(
        value(creeper, F::MobFlags)?.0.as_i64() == Some(1),
        "NoAI flag"
    );
    ensure!(value(creeper, F::CreeperPowered)?.0 == EntityDataValue::Bool(true));
    ensure!(value(creeper, F::CreeperIgnited)?.0 == EntityDataValue::Bool(false));
    ensure!(value(creeper, F::CreeperSwellDir)?.0 == EntityDataValue::Int(-1));
    println!("SLIME");
    ensure!(value(at(1)?, F::SlimeSize)?.0 == EntityDataValue::Int(3));
    println!("ZOMBIE");
    ensure!(value(at(2)?, F::Baby)?.0 == EntityDataValue::Bool(true));
    for (i, expected) in [(3, true), (4, false)] {
        let wolf = at(i)?;
        let angry = wolf.angry(Some(time.value.game_time));
        println!("WOLF {i} angry={angry:?}");
        let _ = value(wolf, F::RemainingAngerTime).or_else(|_| value(wolf, F::AngerEndTime));
        ensure!(angry == Some(expected), "wolf {i} anger");
    }

    // Eyes: dry, then a water source in the eye cell (standing eye at y + 1.62).
    let dry = client.eye_in_water().await?;
    println!("EYE dry={dry:?}");
    ensure!(dry == Some(false));
    cmd(format!("setblock {x} {} {z} minecraft:water", y + 1))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let wet = loop {
        let wet = client.eye_in_water().await?;
        if wet == Some(true) || Instant::now() > deadline {
            break wet;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    println!("EYE wet={wet:?}");
    cmd(format!("setblock {x} {} {z} minecraft:air", y + 1))?;
    ensure!(wet == Some(true));
    cmd(format!(
        "kill @e[type=!player,x={},y={y},z={},dx=12,dy=3,dz=3]",
        x + 2,
        z + 2
    ))?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    client.disconnect().await?;
    println!("DONE");
    Ok(())
}
