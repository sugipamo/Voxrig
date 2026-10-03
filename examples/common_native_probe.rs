//! Fixed common-API operations for scripts/run_common_native.py's isolated servers.
//! The controller verifies results through server RCON, separately from this cache.
use anyhow::Context;
use std::{io::Write, time::Duration};
use tokio::io::{AsyncBufReadExt, BufReader};
use voxrig::{
    BlockFace, Region,
    client::{SlotKnowledge, prelude::*},
};

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
// A fresh connection isolates mining's unresolved continuation boundary from
// the earlier motion scenario. Both versions execute this exact consumer.
async fn mining_probe(client: &Client) -> anyhow::Result<()> {
    let ready = client.player_state().await?;
    let initial_sequence = ready.receive_sequence;
    emit("mining_ready", ready)?;
    let mut commands = BufReader::new(tokio::io::stdin()).lines();
    while let Some(command) = commands.next_line().await? {
        match command.as_str() {
            "mining_baseline" => {
                let player = wait_player(client, |p| {
                    p.game_mode == Some(GameMode::Survival)
                        && p.received_pose.as_ref().is_some_and(|pose| {
                            pose.receive_sequence > initial_sequence
                                && pose.position == [0.5, 65.0, 0.5]
                        })
                        && p.inventory.window_id == Some(0)
                        && p.inventory
                            .cursor
                            .as_ref()
                            .is_some_and(|c| c.value == SlotKnowledge::Empty)
                        && p.inventory.slots[36]
                            .as_ref()
                            .is_some_and(|c| c.value == SlotKnowledge::Empty)
                        && p.health.as_ref().is_some_and(|h| h.value.health > 0.0)
                })
                .await?;
                wait_block(client, [0, 65, 3], "minecraft:stone").await?;
                emit("mining_baseline", player)?;
            }
            "mining_start" => {
                let ops = client.survival();
                ops.select_hotbar(0).await?;
                tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        match ops.look([0.0, 20.48]).await {
                            Ok(_) => return Ok::<_, anyhow::Error>(()),
                            Err(e) => {
                                eprintln!("mining awaiting stationary native context: {e}");
                                tokio::time::sleep(Duration::from_millis(50)).await;
                            }
                        }
                    }
                })
                .await??;
                let target = tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        match ops.target_block(4.5).await {
                            Ok(target) => return Ok::<_, anyhow::Error>(target),
                            Err(error) => {
                                eprintln!(
                                    "mining awaiting received dry standing geometry: {error}"
                                );
                                tokio::time::sleep(Duration::from_millis(50)).await;
                            }
                        }
                    }
                })
                .await??;
                let hit = target.hit.context("mining first outline missing")?;
                anyhow::ensure!(
                    hit.position == [0, 65, 3] && hit.face == BlockFace::North,
                    "unexpected mining first hit"
                );
                let started = ops.start_mining(hit.position, hit.face).await?;
                anyhow::ensure!(started.start.dispatched, "incomplete START");
                anyhow::ensure!(
                    ops.select_hotbar(1).await.is_err(),
                    "competing mining action admitted"
                );
                anyhow::ensure!(
                    ops.start_mining(hit.position, hit.face).await.is_err(),
                    "duplicate mining admitted"
                );
                emit("mining_start", started)?;
            }
            "mining_finish" => {
                let ops = client.survival();
                let current = ops
                    .mining_record()
                    .await?
                    .context("retained mining missing")?;
                anyhow::ensure!(
                    current.stage == MiningStage::Mining,
                    "mining interrupted before FINISH: {:?}",
                    current.requires_inspection
                );
                let sent = ops.finish_mining(current.id).await?;
                anyhow::ensure!(
                    sent.finish.as_ref().is_some_and(|f| f.dispatched),
                    "incomplete FINISH"
                );
                anyhow::ensure!(
                    ops.finish_mining(current.id).await.is_err(),
                    "FINISH replay admitted"
                );
                let removed = tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        let record = ops
                            .mining_record()
                            .await?
                            .context("retained mining missing")?;
                        match record.stage {
                            MiningStage::ObservedRemoved => return Ok::<_, anyhow::Error>(record),
                            MiningStage::RequiresInspection => anyhow::bail!(
                                "native mining interrupted: {:?}",
                                record.requires_inspection
                            ),
                            _ => tokio::time::sleep(Duration::from_millis(25)).await,
                        }
                    }
                })
                .await??;
                anyhow::ensure!(
                    !removed.continuation_validated,
                    "air granted unsafe continuation"
                );
                anyhow::ensure!(
                    removed
                        .target_receipt
                        .as_ref()
                        .is_some_and(|r| r.state.name == "minecraft:air"
                            && r.receive_sequence > removed.start.after_sequence),
                    "missing fresh target air"
                );
                anyhow::ensure!(
                    ops.look([0.0; 2]).await.is_err(),
                    "mining removal released source"
                );
                anyhow::ensure!(
                    ops.abort_mining(current.id).await.is_err(),
                    "ABORT after removal admitted"
                );
                emit("mining_finish", removed)?;
            }
            "mining_disconnect" => {
                client.disconnect().await?;
                let record = client
                    .survival()
                    .mining_record()
                    .await?
                    .context("closed mining diagnostics missing")?;
                emit("mining_disconnected", record)?;
                return Ok(());
            }
            _ => anyhow::bail!("unexpected mining fixture command"),
        }
    }
    anyhow::bail!("mining fixture ended without disconnect")
}
// Another fresh connection gives placement its own material and receipt baseline.
async fn placement_probe(client: &Client) -> anyhow::Result<()> {
    let ready = client.player_state().await?;
    let initial_sequence = ready.receive_sequence;
    emit("placement_ready", ready)?;
    let mut commands = BufReader::new(tokio::io::stdin()).lines();
    while let Some(command) = commands.next_line().await? {
        match command.as_str() {
            "placement_baseline" => {
                let player=wait_player(client,|p|p.game_mode==Some(GameMode::Survival)
                    && p.received_pose.as_ref().is_some_and(|pose|pose.receive_sequence>initial_sequence && pose.position==[0.5,65.0,0.5])
                    && p.inventory.window_id==Some(0) && p.inventory.cursor.as_ref().is_some_and(|c|c.value==SlotKnowledge::Empty)
                    && p.inventory.slots[36].as_ref().is_some_and(|s|matches!(&s.value,SlotKnowledge::Item{item} if item.name=="minecraft:dirt" && item.count==3))
                ).await?;
                wait_block(client, [2, 65, 0], "minecraft:stone").await?;
                wait_block(client, [1, 65, 0], "minecraft:air").await?;
                emit("placement_baseline", player)?;
            }
            "placement_start" => {
                let ops = client.survival();
                ops.select_hotbar(0).await?;
                tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        if let Err(error) = ops.look([-90.0, 40.0]).await {
                            eprintln!("placement awaiting standing look: {error}");
                        } else {
                            match ops.target_block(4.5).await {
                                Ok(target) => {
                                    let hit =
                                        target.hit.context("placement first outline missing")?;
                                    anyhow::ensure!(
                                        hit.position == [2, 65, 0] && hit.face == BlockFace::West,
                                        "unexpected placement first hit"
                                    );
                                    return Ok::<_, anyhow::Error>(());
                                }
                                Err(error) => {
                                    eprintln!("placement awaiting dry standing geometry: {error}")
                                }
                            }
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                })
                .await??;
                let sent = ops.place_cube([2, 65, 0], BlockFace::West).await?;
                anyhow::ensure!(
                    sent.send.dispatched && sent.target == [1, 65, 0],
                    "incomplete/wrong placement"
                );
                // Regardless of timing, the occupied original site cannot be blindly resent.
                anyhow::ensure!(
                    ops.place_cube([2, 65, 0], BlockFace::West).await.is_err(),
                    "same-site placement replay admitted"
                );
                emit("placement_start", sent)?;
            }
            "placement_observed" => {
                let ops = client.survival();
                let placed = tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        let record = ops
                            .placement_record()
                            .await?
                            .context("placement record missing")?;
                        match record.stage {
                            PlacementStage::ObservedPlaced => {
                                return Ok::<_, anyhow::Error>(record);
                            }
                            PlacementStage::RequiresInspection => anyhow::bail!(
                                "placement interrupted: {:?}",
                                record.requires_inspection
                            ),
                            _ => tokio::time::sleep(Duration::from_millis(25)).await,
                        }
                    }
                })
                .await??;
                anyhow::ensure!(
                    placed
                        .target_receipt
                        .as_ref()
                        .is_some_and(|r| r.value.name == "minecraft:dirt"),
                    "missing target receipt"
                );
                anyhow::ensure!(placed.material_receipt.as_ref().is_some_and(|r|matches!(&r.value,SlotKnowledge::Item{item} if item.name=="minecraft:dirt" && item.count==2)),"missing exact one-material consumption");
                for source in [
                    placed.target_receipt.as_ref().unwrap().source,
                    placed.material_receipt.as_ref().unwrap().source,
                ] {
                    anyhow::ensure!(
                        matches!(source,voxrig::client::ValueSource::Received{sequence} if sequence>placed.send.after_sequence),
                        "stale placement receipt"
                    );
                }
                if let Some(sequence) = placed.send.interaction_sequence {
                    anyhow::ensure!(
                        placed
                            .processing
                            .as_ref()
                            .is_some_and(|p| p.sequence >= sequence
                                && p.receive_sequence > placed.send.after_sequence),
                        "missing fresh native processing"
                    );
                }
                ops.select_hotbar(0).await?;
                emit("placement_observed", placed)?;
            }
            "placement_disconnect" => {
                client.disconnect().await?;
                emit(
                    "placement_disconnected",
                    client
                        .survival()
                        .placement_record()
                        .await?
                        .context("closed placement diagnostics missing")?,
                )?;
                return Ok(());
            }
            _ => anyhow::bail!("unexpected placement fixture command"),
        }
    }
    anyhow::bail!("placement fixture ended without disconnect")
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port: u16 = std::env::var("VOXRIG_PORT")?.parse()?;
    let config =
        ConnectionConfig::offline_from_env(Server::new("127.0.0.1", port), "UnifiedProbe")?;
    let client = Client::connect(config).await?;
    client.wait_until_ready().await?;
    if std::env::var("VOXRIG_NATIVE_SCENARIO").ok().as_deref() == Some("placement") {
        return placement_probe(&client).await;
    }
    if std::env::var("VOXRIG_NATIVE_SCENARIO").ok().as_deref() == Some("mining") {
        return mining_probe(&client).await;
    }
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
            "survival_preview" => {
                let controls: Vec<_> = (0..35)
                    .map(|tick| SurvivalControl {
                        yaw: 35.57,
                        input: SurvivalInput {
                            forward: i8::from(tick < 5),
                            jump: tick == 0,
                            ..Default::default()
                        },
                    })
                    .collect();
                let preview = tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        match client.survival().preview_path(&controls).await {
                            Ok(preview) => return Ok::<_, anyhow::Error>(preview),
                            Err(error) => {
                                eprintln!("preview awaiting stationary native context: {error}");
                                tokio::time::sleep(Duration::from_millis(50)).await;
                            }
                        }
                    }
                })
                .await
                .context("native stationary preview deadline")??;
                anyhow::ensure!(
                    preview.initial.game_mode == Some(GameMode::Survival),
                    "mode mismatch"
                );
                anyhow::ensure!(preview.frames.len() == controls.len(), "truncated preview");
                anyhow::ensure!(
                    preview.frames.last().is_some_and(|frame| frame.resting),
                    "unsettled preview"
                );
                anyhow::ensure!(
                    preview.frames.iter().any(|frame| frame.position[1] > 66.2),
                    "jump not modelled"
                );
                anyhow::ensure!(
                    preview.initial_frame.position == [0.5, 65.0, 0.5],
                    "unexpected native preview origin"
                );
                anyhow::ensure!(
                    client.survival().preview_path(&[]).await.is_err(),
                    "empty path admitted"
                );
                emit("survival_preview", preview)?;
            }
            "survival_target" => {
                let ops = client.survival();
                // Aim at the creative fixture's stone; same consumer math on both versions.
                ops.look([-90.0, 48.24]).await?;
                let before = client.player_state().await?;
                let query = ops.target_block(4.5).await?;
                let hit = query
                    .hit
                    .as_ref()
                    .context("native fixture target missing")?;
                anyhow::ensure!(hit.position == [1, 65, 0], "wrong first target");
                anyhow::ensure!(
                    hit.state.name == "minecraft:stone",
                    "wrong received target state"
                );
                anyhow::ensure!(hit.face == BlockFace::Up, "wrong entry face");
                anyhow::ensure!(
                    (hit.point[1] - 66.0).abs() < 1e-10,
                    "hit off native top face"
                );
                let after = client.player_state().await?;
                anyhow::ensure!(
                    before.position == after.position,
                    "target query changed position"
                );
                anyhow::ensure!(
                    before.received_pose == after.received_pose,
                    "target query rewrote receipt"
                );
                anyhow::ensure!(ops.target_block(4.501).await.is_err(), "overreach admitted");
                emit("survival_target", query)?;
            }
            "survival_motion" => {
                let controls: Vec<_> = (0..35)
                    .map(|tick| SurvivalControl {
                        yaw: 35.57,
                        input: SurvivalInput {
                            forward: i8::from(tick < 5),
                            jump: tick == 0,
                            ..Default::default()
                        },
                    })
                    .collect();
                let ops = client.survival();
                let started = ops.start_predicted_path(&controls).await?;
                emit("motion_started", &started)?;
                anyhow::ensure!(
                    ops.select_hotbar(1).await.is_err(),
                    "competing action admitted during motion"
                );
                anyhow::ensure!(
                    ops.start_predicted_path(&controls).await.is_err(),
                    "duplicate motion admitted"
                );
                let completed = tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        let record = ops
                            .motion_record()
                            .await?
                            .context("finite motion record missing")?;
                        if record.status != MotionStatus::Running {
                            return Ok::<_, anyhow::Error>(record);
                        }
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                })
                .await
                .context("finite motion deadline")??;
                anyhow::ensure!(
                    completed.status == MotionStatus::Predicted,
                    "finite motion interrupted: {:?}",
                    completed.problem
                );
                anyhow::ensure!(
                    completed.dispatched_ticks == 35 && completed.attempted_tick == 35,
                    "motion not fully dispatched"
                );
                let after = client.player_state().await?;
                anyhow::ensure!(
                    after.received_pose == completed.preview.initial.received_pose,
                    "received pose replaced prediction"
                );
                anyhow::ensure!(
                    after.position.as_ref().map(|p| p.value)
                        == completed.preview.frames.last().map(|f| f.position),
                    "local endpoint mismatch"
                );
                emit("survival_motion", completed)?;
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
