//! Isolated-server probe for survival digging (`Survival::dig`).
//! Prints `CMD <console command>` lines for a driver script, then digs blocks set from
//! the console with different tools and effects, checking the server removes each one.
//! VOXRIG_MINECRAFT_VERSION selects 1.16.1 or 1.21.11; VOXRIG_PORT selects the port.
use anyhow::{Context, ensure};
use std::io::Write;
use std::time::{Duration, Instant};
use voxrig::client::prelude::*;

fn cmd(line: String) -> anyhow::Result<()> {
    println!("CMD {line}");
    std::io::stdout().flush()?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = std::env::var("VOXRIG_PORT")
        .unwrap_or_else(|_| "25565".into())
        .parse()?;
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() % 100_000);
    let name = format!("ProbeDig{n}");
    let config = ConnectionConfig::offline_from_env(Server::new("127.0.0.1", port), &name)?;
    let version = config.version;
    let client = Client::connect(config).await?;
    client.wait_until_ready().await?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let feet = client
        .player_state()
        .await?
        .position
        .context("position")?
        .value;
    let [x, y, z] = feet.map(|v| v.floor() as i32);
    let target = [x + 2, y, z];
    let give = |slot: &str, item: &str| match version {
        MinecraftVersion::Java1_16_1 => format!("replaceitem entity {name} {slot} {item}"),
        _ => format!("item replace entity {name} {slot} with {item}"),
    };
    let efficiency_pick = match version {
        MinecraftVersion::Java1_16_1 => {
            "diamond_pickaxe{Enchantments:[{id:\"minecraft:efficiency\",lvl:5s}]}".to_owned()
        }
        _ => "diamond_pickaxe[enchantments={efficiency:5}]".to_owned(),
    };
    for line in [
        format!("gamemode survival {name}"),
        format!("effect clear {name}"),
        format!("clear {name}"),
        give("hotbar.1", "wooden_pickaxe"),
        give("hotbar.2", "diamond_pickaxe"),
        give("hotbar.3", &efficiency_pick),
    ] {
        cmd(line)?;
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    let survival = client.survival();
    survival.look([270.0, 0.0]).await?;
    let grass = match version {
        MinecraftVersion::Java1_16_1 => "grass",
        _ => "short_grass",
    };
    let cases: [(&str, u8, Option<&str>); 9] = [
        ("dirt", 0, None),
        ("stone", 0, None),
        ("stone", 1, None),
        ("stone", 2, None),
        ("stone", 3, None),
        ("obsidian", 2, None),
        (grass, 0, None),
        ("stone", 1, Some("haste 30 1")),
        ("iron_ore", 1, None),
    ];
    for (block, slot, effect) in cases {
        cmd(format!(
            "setblock {} {} {} {block}",
            target[0], target[1], target[2]
        ))?;
        if let Some(effect) = effect {
            cmd(format!("effect give {name} {effect}"))?;
        }
        survival.select_hotbar(slot).await?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let column = client
                .chunk([target[0].div_euclid(16), target[2].div_euclid(16)])
                .await?
                .context("target column")?;
            let state = client.player_state().await?;
            let placed = column
                .block(target)
                .is_some_and(|b| b.name == format!("minecraft:{block}"));
            let effect_ready = effect.is_none() || state.effects.contains_key("minecraft:haste");
            let held_ready = slot == 0
                || state.inventory.slots[36 + usize::from(slot)]
                    .as_ref()
                    .is_some_and(|s| matches!(s.value, SlotKnowledge::Item { .. }));
            if placed && effect_ready && held_ready {
                break;
            }
            ensure!(Instant::now() < deadline, "setup for {block} not received");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        let started = Instant::now();
        let record = survival.dig(target, BlockFace::West).await?;
        println!(
            "DIG {block} slot={slot} effect={effect:?} ticks={} harvestable={} speed={} removed={} took={:?} final={:?}",
            record.estimate.ticks,
            record.estimate.harvestable,
            record.estimate.speed,
            record.removed,
            started.elapsed(),
            record.final_state.map(|s| s.name)
        );
        if effect.is_some() {
            cmd(format!("effect clear {name}"))?;
        }
    }
    println!("DONE");
    client.disconnect().await?;
    Ok(())
}
