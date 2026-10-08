//! Isolated-server probe for `Survival::placement_check`. Prints `CMD <console command>`
//! lines for a driver script, builds a flat floor, then checks and places stone against
//! cells that are clear, inside the own player, under a pig, under armor stands (marker
//! and not) and out of reach, comparing the check with the server's placement result.
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
    let name = format!("ProbePlc{n}");
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
    let give = match version {
        MinecraftVersion::Java1_16_1 => format!("replaceitem entity {name} hotbar.0 stone 64"),
        _ => format!("item replace entity {name} hotbar.0 with stone 64"),
    };
    let centre = |cx: i32, cz: i32| format!("{} {y} {}", f64::from(cx) + 0.5, f64::from(cz) + 0.5);
    for line in [
        "kill @e[type=pig]".to_owned(),
        "kill @e[type=armor_stand]".to_owned(),
        format!(
            "fill {} {} {} {} {} {} air",
            x - 3,
            y,
            z - 3,
            x + 7,
            y + 2,
            z + 3
        ),
        format!(
            "fill {} {} {} {} {} {} stone",
            x - 3,
            y - 1,
            z - 3,
            x + 7,
            y - 1,
            z + 3
        ),
        format!("tp {name} {}", centre(x, z)),
        format!("gamemode survival {name}"),
        format!("clear {name}"),
        give,
        format!("summon minecraft:pig {} {{NoAI:1b}}", centre(x, z + 2)),
        format!(
            "summon minecraft:armor_stand {} {{Marker:1b}}",
            centre(x - 2, z)
        ),
        format!("summon minecraft:armor_stand {}", centre(x - 2, z - 2)),
    ] {
        cmd(line)?;
    }
    let survival = client.survival();
    survival.select_hotbar(0).await?;
    // Wait for the floor, the stone and the three entities.
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let state = client.player_state().await?;
        let entities = client.entities().await?;
        let count = |kind: &str| {
            entities
                .entities
                .iter()
                .filter(|e| e.motion.entity.type_name.as_deref() == Some(kind))
                .filter(|e| e.bounding_box.is_some())
                .count()
        };
        let floor = survival
            .placement_check([x + 6, y - 1, z], BlockFace::Up)
            .await?
            .support_state
            .is_some_and(|s| s.name == "minecraft:stone");
        let stone = state.inventory.slots[36]
            .as_ref()
            .is_some_and(|s| matches!(s.value, SlotKnowledge::Item { .. }));
        if floor && stone && count("minecraft:pig") == 1 && count("minecraft:armor_stand") == 2 {
            break;
        }
        ensure!(Instant::now() < deadline, "setup not received");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    // (label, support, expected clear, place it?)
    let cases: [(&str, [i32; 3], bool, bool); 6] = [
        ("clear", [x + 2, y - 1, z], true, true),
        ("own-player", [x, y - 1, z], false, true),
        ("pig", [x, y - 1, z + 2], false, true),
        ("marker-stand", [x - 2, y - 1, z], true, true),
        ("armor-stand", [x - 2, y - 1, z - 2], false, true),
        // 1.16.1 servers accept use-on within 8 blocks of the feet: check only.
        ("out-of-reach", [x + 6, y - 1, z], false, false),
    ];
    for (label, support, expect_clear, place) in cases {
        let check = survival.placement_check(support, BlockFace::Up).await?;
        let mut placed = None;
        if place {
            client
                .survival()
                .use_on_block(support, BlockFace::Up, [0.5, 1.0, 0.5], Hand::Main)
                .await?;
            tokio::time::sleep(Duration::from_millis(1000)).await;
            let after = survival.placement_check(support, BlockFace::Up).await?;
            placed = Some(
                after
                    .target_state
                    .is_some_and(|s| s.name == "minecraft:stone"),
            );
        }
        println!(
            "CHECK {label} clear={} reachable={} face_distance={:.3} range={} player={} entities={:?} placed={placed:?}",
            check.clear(),
            check.reachable,
            check.face_distance,
            check.interaction_range,
            check.player_intersects_target_cell,
            check.blocking_entities,
        );
        ensure!(check.clear() == expect_clear, "{label}: unexpected check");
        if let Some(placed) = placed {
            ensure!(
                placed == expect_clear,
                "{label}: server disagreed with check"
            );
        }
    }
    println!("DONE");
    client.disconnect().await?;
    Ok(())
}
