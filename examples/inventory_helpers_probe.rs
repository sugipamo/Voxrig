//! Isolated-server probe for the inventory helpers: `compact_inventory`, and
//! `craft_once` in the player 2×2 grid and at a crafting table. Prints
//! `CMD <console command>` lines for a driver script.
//! VOXRIG_MINECRAFT_VERSION selects 1.16.1 or 1.21.11; VOXRIG_PORT selects the port.
use anyhow::{Context, ensure};
use std::collections::BTreeMap;
use std::io::Write;
use std::time::{Duration, Instant};
use voxrig::client::prelude::*;

fn cmd(line: String) -> anyhow::Result<()> {
    println!("CMD {line}");
    std::io::stdout().flush()?;
    Ok(())
}

/// Item name -> (total count, occupied slots) over player slots 9..44.
async fn tally(client: &Client) -> anyhow::Result<BTreeMap<String, (u32, usize)>> {
    let state = client.player_state().await?;
    let mut tally = BTreeMap::new();
    for slot in &state.inventory.slots[9..45] {
        if let Some(SlotKnowledge::Item { item }) = slot.as_ref().map(|s| &s.value) {
            let entry = tally.entry(item.name.clone()).or_insert((0, 0));
            entry.0 += item.count;
            entry.1 += 1;
        }
    }
    Ok(tally)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = std::env::var("VOXRIG_PORT")
        .unwrap_or_else(|_| "25565".into())
        .parse()?;
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() % 100_000);
    let name = format!("ProbeInv{n}");
    let config = ConnectionConfig::offline_from_env(Server::new("127.0.0.1", port), &name)?;
    let version = config.version;
    let client = Client::connect(config).await?;
    client.wait_until_ready().await?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let [x, y, z] = client
        .player_state()
        .await?
        .position
        .context("position")?
        .value
        .map(|v| v.floor() as i32);
    let give = |slot: &str, item: &str, count: u32| match version {
        MinecraftVersion::Java1_16_1 => {
            format!("replaceitem entity {name} {slot} {item} {count}")
        }
        _ => format!("item replace entity {name} {slot} with {item} {count}"),
    };
    for line in [
        format!("gamemode survival {name}"),
        format!("clear {name}"),
        format!(
            "fill {} {y} {} {} {} {} air",
            x - 2,
            z - 2,
            x + 3,
            y + 2,
            z + 2
        ),
        format!(
            "fill {} {} {} {} {} {} stone",
            x - 2,
            y - 1,
            z - 2,
            x + 3,
            y - 1,
            z + 2
        ),
        format!(
            "tp {name} {} {y} {} 270 40",
            f64::from(x) + 0.5,
            f64::from(z) + 0.5
        ),
        give("inventory.0", "oak_planks", 5),
        give("inventory.10", "oak_planks", 7),
        give("hotbar.3", "oak_planks", 60),
        give("inventory.20", "cobblestone", 3),
        give("inventory.5", "cobblestone", 2),
        format!("setblock {} {y} {z} crafting_table", x + 2),
    ] {
        cmd(line)?;
    }
    let survival = client.survival();
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let tally = tally(&client).await?;
        if tally.get("minecraft:oak_planks") == Some(&(72, 3))
            && tally.get("minecraft:cobblestone") == Some(&(5, 2))
        {
            break;
        }
        ensure!(Instant::now() < deadline, "setup not received: {tally:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;

    let clicks = survival.compact_inventory().await?;
    let after = tally(&client).await?;
    println!("COMPACT clicks={} tally={after:?}", clicks.len());
    ensure!(
        after.get("minecraft:oak_planks") == Some(&(72, 2)),
        "planks not merged"
    );
    ensure!(
        after.get("minecraft:cobblestone") == Some(&(5, 1)),
        "cobblestone not merged"
    );

    // Player 2x2: two planks in a column make four sticks.
    let planks = Some("oak_planks");
    let record = survival.craft_once(&[planks, None, planks, None]).await?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let after = tally(&client).await?;
    println!(
        "CRAFT2 clicks={} result={:?} take={:?} tally={after:?}",
        record.fill.clicks.len(),
        record.fill.result.as_ref().map(|r| (&r.name, r.count)),
        record.take.stage,
    );
    ensure!(
        after.get("minecraft:stick").map(|t| t.0) == Some(4),
        "sticks not crafted"
    );
    ensure!(
        after.get("minecraft:oak_planks").map(|t| t.0) == Some(70),
        "planks not used"
    );

    // Crafting table 3x3: a wooden pickaxe.
    survival.look([270.0, 40.0]).await?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    survival.open_container([x + 2, y, z]).await?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let screen = loop {
        if let Some(screen) = client.screen_state().await?.screen {
            if screen.menu_name.as_deref() == Some("minecraft:crafting") {
                break screen.id;
            }
        }
        ensure!(Instant::now() < deadline, "crafting table did not open");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    let stick = Some("stick");
    let record = survival
        .craft_once(&[planks, planks, planks, None, stick, None, None, stick, None])
        .await?;
    println!(
        "CRAFT3 clicks={} result={:?} take={:?}",
        record.fill.clicks.len(),
        record.fill.result.as_ref().map(|r| (&r.name, r.count)),
        record.take.stage,
    );
    survival.close_container(screen).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let after = tally(&client).await?;
    println!("TABLE tally={after:?}");
    ensure!(
        after.get("minecraft:wooden_pickaxe").map(|t| t.0) == Some(1),
        "pickaxe not crafted"
    );
    ensure!(
        after.get("minecraft:oak_planks").map(|t| t.0) == Some(67),
        "planks not used"
    );
    ensure!(
        after.get("minecraft:stick").map(|t| t.0) == Some(2),
        "sticks not used"
    );
    println!("DONE");
    client.disconnect().await?;
    Ok(())
}
