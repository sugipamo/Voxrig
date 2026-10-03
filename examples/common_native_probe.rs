//! Fixed common-API operations for scripts/run_common_native.py's isolated servers.
//! The controller verifies results through server RCON, separately from this cache.
use anyhow::Context;
use std::{io::Write, time::Duration};
use tokio::io::{AsyncBufReadExt, BufReader};
use voxrig::{BlockFace, Region, client::prelude::*};

fn emit(stage: &str, value: impl serde::Serialize) -> anyhow::Result<()> {
    println!("{}", serde_json::json!({"stage":stage,"value":value}));
    std::io::stdout().flush()?;
    Ok(())
}
async fn wait_player(
    client: &Client,
    predicate: impl Fn(&PlayerObservation) -> bool,
) -> anyhow::Result<PlayerObservation> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let player = client.player_state().await?;
            if predicate(&player) {
                return Ok(player);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await?
}
async fn wait_block(client: &Client, position: [i32; 3], name: &str) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let capture = client
                .capture(Region {
                    min: position,
                    max: position,
                })
                .await
                .context("native teleport/mode/abilities baseline")?;
            if capture.world.blocks[0]
                .state
                .as_ref()
                .is_some_and(|state| state.name == name)
            {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await?
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port: u16 = std::env::var("VOXRIG_PORT")?.parse()?;
    let config =
        ConnectionConfig::offline_from_env(Server::new("127.0.0.1", port), "UnifiedProbe")?;
    let client = Client::connect(config).await?;
    client.wait_until_ready().await?;
    let ready = client.player_state().await?;
    let initial_sequence = ready.receive_sequence;
    emit("ready", &ready)?;
    let creative = client.creative();
    let mut commands = BufReader::new(tokio::io::stdin()).lines();
    while let Some(command) = commands.next_line().await? {
        match command.as_str() {
            "baseline" => {
                let player = wait_player(&client, |player| {
                    player.game_mode == Some(GameMode::Creative)
                        && player.may_fly == Some(true)
                        && player.received_pose.as_ref().is_some_and(|pose| {
                            pose.receive_sequence > initial_sequence
                                && pose.position == [0.5, 65.0, 0.5]
                        })
                })
                .await?;
                wait_block(&client, [0, 65, 1], "minecraft:stone")
                    .await
                    .context("native target stone baseline")?;
                wait_block(&client, [1, 65, 0], "minecraft:air")
                    .await
                    .context("native placement air baseline")?;
                anyhow::ensure!(
                    creative.look([0.0, 91.0]).await.is_err(),
                    "invalid pitch dispatched"
                );
                anyhow::ensure!(
                    client.survival().select_hotbar(0).await.is_err(),
                    "survival mutation admitted in creative mode"
                );
                emit("baseline", player)?;
            }
            "inventory" => {
                let item = creative.set_hotbar(1, Some(("minecraft:stone", 3))).await?;
                let selection = creative.select_hotbar(1).await?;
                emit(
                    "inventory",
                    serde_json::json!({"item_dispatch":item,"selection_dispatch":selection,"observation":client.player_state().await?}),
                )?;
            }
            "flight" => {
                let permission = creative.set_flying(true).await?;
                let step = creative.move_flying([0.5, 66.0, 0.5], [0.0, 0.0]).await?;
                anyhow::ensure!(
                    creative
                        .move_flying([6.5, 66.0, 0.5], [0.0, 0.0])
                        .await
                        .is_err(),
                    "overlong flight admitted"
                );
                emit(
                    "flight",
                    serde_json::json!({"permission_dispatch":permission,"position_dispatch":step,"observation":client.player_state().await?}),
                )?;
            }
            "break" => {
                let receipt = creative.break_block([0, 65, 1], BlockFace::Up).await?;
                wait_block(&client, [0, 65, 1], "minecraft:air").await?;
                emit("break", receipt)?;
            }
            "place" => {
                creative.look([-90.0, 69.0]).await?;
                let receipt = creative
                    .use_on_block([1, 64, 0], BlockFace::Up, [0.5, 1.0, 0.5])
                    .await
                    .context("native creative placement dispatch")?;
                wait_block(&client, [1, 65, 0], "minecraft:stone").await?;
                emit("place", receipt)?;
            }
            "survival_guard" => {
                let player = wait_player(&client, |player| {
                    player.game_mode == Some(GameMode::Survival)
                })
                .await?;
                anyhow::ensure!(
                    creative
                        .set_hotbar(0, Some(("minecraft:diamond", 1)))
                        .await
                        .is_err(),
                    "creative inventory mutation admitted after survival receipt"
                );
                client.survival().select_hotbar(1).await?;
                emit("survival_guard", player)?;
            }
            "disconnect" => {
                client.disconnect().await?;
                emit(
                    "disconnected",
                    serde_json::json!({"version":client.version()}),
                )?;
                return Ok(());
            }
            _ => anyhow::bail!("unknown trial command"),
        }
    }
    anyhow::bail!("controller closed before explicit disconnect")
}
