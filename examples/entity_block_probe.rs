//! Isolated-server probe for common entity state, block search and raycasts.
//! Prints `READY x y z`, then expects an operator to summon an armor stand at
//! x+2, give it a helmet and set its health to 7.
//! VOXRIG_MINECRAFT_VERSION selects 1.16.1 or 1.21.11; VOXRIG_PORT selects the port.
use std::io::Write;
use std::time::Duration;
use voxrig::client::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = std::env::var("VOXRIG_PORT")
        .unwrap_or_else(|_| "25565".into())
        .parse()?;
    let config = ConnectionConfig::offline_from_env(Server::new("127.0.0.1", port), "ProbeEB")?;
    let client = Client::connect(config).await?;
    client.wait_until_ready().await?;
    let feet = client.player_state().await?.position.unwrap().value;
    let base = feet.map(|v| v.floor() as i32);
    // Readiness does not imply that nearby terrain has arrived.
    client
        .wait_for_loaded(
            Region {
                min: [base[0] - 4, base[1] - 4, base[2] - 4],
                max: [base[0] + 4, base[1] + 4, base[2] + 4],
            },
            Duration::from_secs(10),
        )
        .await?;

    let ground = client
        .raycast_blocks([feet[0], feet[1] + 1.62, feet[2]], [0.0, -1.0, 0.0], 8.0)
        .await?;
    println!("RAY_DOWN {:?}", ground.result);
    let up = client
        .raycast_blocks([feet[0], feet[1] + 1.62, feet[2]], [0.0, 1.0, 0.0], 8.0)
        .await?;
    println!("RAY_UP {:?}", up.result);
    let search = client
        .find_blocks(
            Region {
                min: [base[0] - 4, base[1] - 4, base[2] - 4],
                max: [base[0] + 4, base[1], base[2] + 4],
            },
            &["minecraft:grass_block"],
        )
        .await?;
    println!(
        "GRASS matches={} unloaded={}",
        search.matches.len(),
        search.unloaded
    );

    println!("READY {} {} {}", base[0], base[1], base[2]);
    std::io::stdout().flush()?;
    tokio::time::sleep(Duration::from_secs(4)).await;
    let all = client.entities().await?;
    for entity in &all.entities {
        if entity.motion.entity.type_name.as_deref() == Some("minecraft:armor_stand") {
            println!(
                "STAND health={:?} equipment={:?} box={:?}",
                entity.health.as_ref().map(|h| h.value),
                entity
                    .equipment
                    .iter()
                    .map(|(slot, item)| (
                        *slot,
                        match &item.value {
                            SlotKnowledge::Item { item } => item.name.clone(),
                            other => format!("{other:?}"),
                        }
                    ))
                    .collect::<Vec<_>>(),
                entity
                    .bounding_box
                    .map(|b| [b.min_x, b.min_y, b.max_x, b.max_y])
            );
        }
    }
    println!("ENTITIES {}", all.entities.len());
    client.disconnect().await?;
    Ok(())
}
