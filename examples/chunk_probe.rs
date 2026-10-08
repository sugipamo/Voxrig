//! Isolated-server probe for whole-column reads (`Client::loaded_chunks`, `Client::chunk`).
//! Prints `CMD <console command>` lines for a driver script, then checks received
//! sky light at the player, block light around a torch set from the console, and the
//! cost of reading every loaded column.
//! VOXRIG_MINECRAFT_VERSION selects 1.16.1 or 1.21.11; VOXRIG_PORT selects the port.
use anyhow::{Context, bail};
use std::io::Write;
use std::time::{Duration, Instant};
use voxrig::client::prelude::*;

const NAME: &str = "ProbeChunk";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = std::env::var("VOXRIG_PORT")
        .unwrap_or_else(|_| "25565".into())
        .parse()?;
    let config = ConnectionConfig::offline_from_env(Server::new("127.0.0.1", port), NAME)?;
    let client = Client::connect(config).await?;
    client.wait_until_ready().await?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let feet = client
        .player_state()
        .await?
        .position
        .context("position")?
        .value;
    let [x, y, z] = feet.map(|v| v.floor() as i32);
    let at = |p: [i32; 3]| [p[0].div_euclid(16), p[2].div_euclid(16)];
    let loaded = client.loaded_chunks().await?;
    let started = Instant::now();
    let mut cells = 0usize;
    for _ in 0..20 {
        for &c in &loaded.chunks {
            let chunk = client.chunk(c).await?.context("listed column")?;
            cells += chunk.height as usize * 256;
        }
    }
    println!(
        "COLUMNS {} read_all_x20={:?} cells_per_read={}",
        loaded.chunks.len(),
        started.elapsed(),
        cells / 20
    );
    let column = client.chunk(at([x, y, z])).await?.context("own column")?;
    println!(
        "AT_PLAYER state={:?} below={:?} sky={:?} block={:?}",
        column.block([x, y, z]).map(|b| b.name),
        column.block([x, y - 1, z]).map(|b| b.name),
        column.sky_light([x, y, z]),
        column.block_light([x, y, z])
    );
    // A torch two cells east. The server applies light itself and, like vanilla, often
    // sends no light update to a nearby client; received light there becomes unknown.
    let torch = [x + 2, y, z];
    let before = client.chunk(at(torch)).await?.context("torch column")?;
    println!(
        "BEFORE_TORCH block_light={:?} sky={:?}",
        before.block_light(torch),
        before.sky_light(torch)
    );
    println!("CMD setblock {} {} {} torch", torch[0], torch[1], torch[2]);
    std::io::stdout().flush()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let column = client.chunk(at(torch)).await?.context("torch column")?;
        if column
            .block(torch)
            .is_some_and(|b| b.name == "minecraft:torch")
        {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let column = client.chunk(at(torch)).await?.context("torch column")?;
            println!(
                "AFTER_TORCH block_light={:?} sky={:?} neighbor_column={:?}",
                column.block_light(torch),
                column.sky_light(torch),
                client
                    .chunk([at(torch)[0] + 1, at(torch)[1]])
                    .await?
                    .and_then(|c| c.sky_light([torch[0] + 16, torch[1], torch[2]]))
            );
            break;
        }
        if Instant::now() > deadline {
            bail!("torch not received");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    println!("CMD setblock {} {} {} air", torch[0], torch[1], torch[2]);
    println!("DONE");
    std::io::stdout().flush()?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    client.disconnect().await?;
    Ok(())
}
