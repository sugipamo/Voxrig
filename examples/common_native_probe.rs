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
async fn wait_swap(
    client: &Client,
) -> anyhow::Result<voxrig::client::inventory::InventorySwapRecord> {
    let record = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let record = client
                .survival()
                .inventory_swap_record()
                .await?
                .context("inventory record missing")?;
            match record.stage {
                InventorySwapStage::ObservedSwapped => return Ok::<_, anyhow::Error>(record),
                InventorySwapStage::RequiresInspection => {
                    anyhow::bail!("inventory interrupted: {:?}", record.requires_inspection)
                }
                _ => tokio::time::sleep(Duration::from_millis(25)).await,
            }
        }
    })
    .await??;
    anyhow::ensure!(record.send.dispatched, "swap not dispatched");
    anyhow::ensure!(
        record
            .source_receipt
            .as_ref()
            .is_some_and(|r| r.value == record.hotbar_before.value)
            && record
                .hotbar_receipt
                .as_ref()
                .is_some_and(|r| r.value == record.source_before.value),
        "wrong swap destinations"
    );
    for receipt in [&record.source_receipt, &record.hotbar_receipt] {
        anyhow::ensure!(
            matches!(receipt.as_ref().unwrap().source,voxrig::client::ValueSource::Received{sequence} if sequence>record.send.after_sequence),
            "stale swap receipt"
        );
    }
    if let Some(action) = record.send.legacy_action {
        anyhow::ensure!(
            record.legacy_reply.as_ref().is_some_and(
                |r| r.action == action && r.receive_sequence > record.send.after_sequence
            ),
            "missing fresh native transaction response"
        );
    } else {
        anyhow::ensure!(
            record.legacy_reply.is_none(),
            "invented modern legacy transaction"
        );
    }
    Ok(record)
}
async fn wait_pickup(
    client: &Client,
) -> anyhow::Result<voxrig::client::inventory::InventoryClickRecord> {
    tokio::time::timeout(Duration::from_secs(15),async {
        loop {
            let record=client.survival().inventory_click_record().await?.context("click record missing")?;
            match record.stage {
                InventoryClickStage::ObservedClicked => {
                    anyhow::ensure!(record.send.dispatched && record.requires_inspection.is_none(),"click dispatch/conflict");
                    for (receipt,prediction) in [(record.source_receipt.as_ref().context("source missing")?,&record.prediction.source),
                        (record.cursor_receipt.as_ref().context("cursor missing")?,&record.prediction.cursor)] {
                        anyhow::ensure!(receipt.value==prediction.value && matches!(receipt.source,voxrig::client::ValueSource::Received{sequence} if sequence>record.send.after_sequence),"click requires fresh actual source/cursor");
                    }
                    if let Some(action)=record.send.legacy_action {
                        anyhow::ensure!(record.legacy_reply.as_ref().is_some_and(|r|r.action==action && i32::from(r.window_id)==record.window_id() && r.receive_sequence>record.send.after_sequence),"native click reply missing");
                    } else {anyhow::ensure!(record.legacy_reply.is_none(),"invented click reply");}
                    return Ok::<_,anyhow::Error>(record);
                }
                InventoryClickStage::RequiresInspection=>anyhow::bail!("click interrupted: {:?}",record.requires_inspection),
                _=>tokio::time::sleep(Duration::from_millis(25)).await,
            }
        }
    }).await?
}
async fn wait_transfer(
    client: &Client,
) -> anyhow::Result<voxrig::client::inventory::InventoryTransferRecord> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let record = client.survival().inventory_transfer_record().await?.context("transfer record missing")?;
            anyhow::ensure!(record.requires_inspection.is_none(), "transfer interrupted: {:?}", record.requires_inspection);
            if record.stage == InventoryTransferStage::ObservedTransferred {
                anyhow::ensure!(record.send.dispatched && !record.changed_slots.is_empty(), "transfer dispatch missing");
                anyhow::ensure!(record.cursor_inspected.as_ref().is_some_and(|c| c.value == record.cursor_before.value && matches!(c.source, voxrig::client::ValueSource::Received{..})), "unchanged cursor inspection missing");
                for change in &record.changed_slots {
                    let actual = change.receipt.as_ref().context("changed slot receipt missing")?;
                    anyhow::ensure!(actual.value == change.prediction.value && matches!(actual.source, voxrig::client::ValueSource::Received{sequence} if sequence > record.send.after_sequence), "transfer requires fresh exact changed slots");
                }
                if let Some(action) = record.send.legacy_action {
                    anyhow::ensure!(record.legacy_reply.as_ref().is_some_and(|r| r.action == action && i32::from(r.window_id) == record.window_id() && r.receive_sequence > record.send.after_sequence), "native transfer reply missing");
                } else { anyhow::ensure!(record.legacy_reply.is_none() && record.legacy_return_prediction.is_none(), "invented legacy transfer fact"); }
                return Ok::<_, anyhow::Error>(record);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }).await?
}
fn stack_count(value: &SlotKnowledge) -> u32 {
    match value {
        SlotKnowledge::Empty => 0,
        SlotKnowledge::Item { item } => item.count,
        _ => u32::MAX,
    }
}
async fn inventory_probe(client: &Client) -> anyhow::Result<()> {
    let ready = client.player_state().await?;
    let initial_sequence = ready.receive_sequence;
    emit("swap_ready", ready)?;
    let mut commands = BufReader::new(tokio::io::stdin()).lines();
    while let Some(command) = commands.next_line().await? {
        match command.as_str() {
            "swap_baseline" => {
                let player=wait_player(client,|p|p.game_mode==Some(GameMode::Survival)&&p.received_pose.as_ref().is_some_and(|pose|pose.receive_sequence>initial_sequence&&pose.position==[0.5,65.0,0.5])
                &&p.inventory.window_id==Some(0)&&p.inventory.cursor.as_ref().is_some_and(|c|c.value==SlotKnowledge::Empty)
                &&p.inventory.slots[9].as_ref().is_some_and(|s|matches!(&s.value,SlotKnowledge::Item{item} if item.name=="minecraft:stone"&&item.count==3))
                &&p.inventory.slots[36].as_ref().is_some_and(|s|matches!(&s.value,SlotKnowledge::Item{item} if item.name=="minecraft:dirt"&&item.count==2))
                &&p.inventory.slots[37].as_ref().is_some_and(|s|s.value==SlotKnowledge::Empty)).await?;
                emit("swap_baseline", player)?;
            }
            "swap_start" => {
                let record = client.survival().swap_hotbar(9, 0).await?;
                anyhow::ensure!(record.send.dispatched, "swap incomplete");
                anyhow::ensure!(
                    client.survival().swap_hotbar(9, 0).await.is_err(),
                    "swap replay allowed"
                );
                emit("swap_start", record)?;
            }
            "swap_observed" => {
                emit("swap_observed", wait_swap(client).await?)?;
            }
            "swap_creative" => {
                wait_player(client, |p| p.game_mode == Some(GameMode::Creative)).await?;
                let before = client
                    .creative()
                    .inventory_swap_record()
                    .await?
                    .context("first swap lost")?;
                let record = client.creative().swap_hotbar(9, 1).await?;
                anyhow::ensure!(
                    record.id != before.id && record.hotbar_before.value == SlotKnowledge::Empty,
                    "new empty destination missing"
                );
                anyhow::ensure!(record.send.dispatched, "creative ordinary click incomplete");
                emit("swap_creative", record)?;
            }
            "swap_empty_observed" => {
                emit("swap_empty_observed", wait_swap(client).await?)?;
            }
            "swap_disconnect" => {
                client.disconnect().await?;
                let record = client
                    .creative()
                    .inventory_swap_record()
                    .await?
                    .context("closed swap history lost")?;
                anyhow::ensure!(
                    record.stage == InventorySwapStage::ObservedSwapped,
                    "closed completed history changed"
                );
                emit("swap_disconnected", record)?;
                return Ok(());
            }
            _ => anyhow::bail!("unexpected inventory fixture command"),
        }
    }
    anyhow::bail!("inventory fixture ended without disconnect")
}
async fn container_probe(client: &Client) -> anyhow::Result<()> {
    use voxrig::client::{ValueSource, container::ScreenObservation};
    let ready = client.player_state().await?;
    let initial_sequence = ready.receive_sequence;
    emit("container_ready", ready)?;
    let mut commands = BufReader::new(tokio::io::stdin()).lines();
    let mut opening = None;
    let mut content_sequence = 0;
    while let Some(command) = commands.next_line().await? {
        match command.as_str() {
            "cursor_close_audit_open_survival" | "cursor_close_audit_open_creative" => {
                let mode = if command.ends_with("survival") {
                    GameMode::Survival
                } else {
                    GameMode::Creative
                };
                wait_player(client, |p| {
                    p.game_mode == Some(mode)
                        && p.received_pose
                            .as_ref()
                            .is_some_and(|pose| pose.position == [0.5, 65., 0.5])
                })
                .await?;
                wait_barrel_target(client, mode, "false").await?;
                let record = if mode == GameMode::Survival {
                    client.survival().open_container([0, 65, 2]).await?
                } else {
                    client.creative().open_container([0, 65, 2]).await?
                };
                emit(&command, record)?;
            }
            "cursor_close_audit_opened" => {
                emit(&command, wait_container_open(client).await?)?;
            }
            "cursor_close_audit_pickup" => {
                let record = client
                    .survival()
                    .container_open_record()
                    .await?
                    .context("audit opening absent")?;
                let source = InventorySource::Container {
                    screen: record
                        .observed_screen
                        .context("audit received screen absent")?
                        .id,
                };
                let pickup = if record.mode == GameMode::Survival {
                    client
                        .survival()
                        .click_inventory(source, 0, InventoryClickButton::Left)
                        .await?
                } else {
                    client
                        .creative()
                        .click_inventory(source, 0, InventoryClickButton::Left)
                        .await?
                };
                emit(&command, pickup)?;
            }
            "cursor_close_audit_holding" => {
                let record = wait_pickup(client).await?;
                anyhow::ensure!(
                    record.source_receipt.as_ref().unwrap().value == SlotKnowledge::Empty
                        && stack_count(&record.cursor_receipt.as_ref().unwrap().value) == 5,
                    "native audit must hold received stone 5"
                );
                emit(&command, record)?;
            }
            "cursor_close_audit_forced" => {
                let screen = tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        let screen = client.screen_state().await?;
                        if screen.screen.is_none() {
                            return Ok::<_, anyhow::Error>(screen);
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await??;
                emit(
                    &command,
                    serde_json::json!({"screen":screen,"player":client.player_state().await?}),
                )?;
            }
            "cursor_close_audit_disconnect" => {
                client.disconnect().await?;
                emit(&command, serde_json::json!({"version":client.version()}))?;
                return Ok(());
            }
            "barrel_open_creative" | "barrel_open_survival" => {
                let mode = if command == "barrel_open_creative" {
                    GameMode::Creative
                } else {
                    GameMode::Survival
                };
                wait_player(client, |p| p.game_mode == Some(mode)).await?;
                let closed = wait_barrel_target(client, mode, "false").await?;
                anyhow::ensure!(
                    closed
                        .hit
                        .as_ref()
                        .context("closed barrel missing")?
                        .position
                        == [0, 65, 2],
                    "barrel target differs"
                );
                let record = if mode == GameMode::Creative {
                    client.creative().open_container([0, 65, 2]).await?
                } else {
                    client.survival().open_container([0, 65, 2]).await?
                };
                anyhow::ensure!(
                    record.send.dispatched && record.target.state.properties["open"] == "false",
                    "barrel admission/dispatch differs"
                );
                emit(&command, record)?;
            }
            "barrel_observed_creative" | "barrel_observed_survival" => {
                let mode = if command == "barrel_observed_creative" {
                    GameMode::Creative
                } else {
                    GameMode::Survival
                };
                let record = wait_container_open(client).await?;
                anyhow::ensure!(
                    record.mode == mode && record.target.state.name == "minecraft:barrel",
                    "barrel record differs"
                );
                let screen = record
                    .observed_screen
                    .as_ref()
                    .context("barrel screen missing")?;
                anyhow::ensure!(
                    matches!(&screen.slots[0],Some(v) if matches!(&v.value,SlotKnowledge::Item { item } if item.name=="minecraft:stone" && item.count==5)),
                    "barrel contents differ"
                );
                let live = wait_barrel_target(client, mode, "true").await?;
                emit("barrel_received_open_flag", live)?;
                emit(&command, record)?;
            }
            "barrel_close_creative" | "barrel_close_survival" => {
                let record = client
                    .survival()
                    .container_open_record()
                    .await?
                    .context("barrel open missing")?;
                let screen = record.observed_screen.context("barrel opening missing")?.id;
                let close = if command == "barrel_close_creative" {
                    client.creative().close_container(screen).await?
                } else {
                    client.survival().close_container(screen).await?
                };
                anyhow::ensure!(close.dispatched, "barrel close incomplete");
                emit(&command, close)?;
            }
            "container_target_creative" | "container_target_survival" => {
                let mode = if command == "container_target_creative" {
                    GameMode::Creative
                } else {
                    GameMode::Survival
                };
                wait_player(client, |p| p.game_mode == Some(mode)).await?;
                let query = tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        let result = if mode == GameMode::Creative {
                            client.creative().target_block(4.5).await
                        } else {
                            client.survival().target_block(4.5).await
                        };
                        if let Ok(query) = result {
                            if query
                                .hit
                                .as_ref()
                                .is_some_and(|hit| hit.position == [0, 65, 2])
                            {
                                return Ok::<_, anyhow::Error>(query);
                            }
                        }
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                })
                .await??;
                let hit = query.hit.as_ref().context("storage hit missing")?;
                anyhow::ensure!(
                    hit.state.name == "minecraft:chest"
                        && hit.face == BlockFace::North
                        && (hit.point[2] - 2.0625).abs() < 1e-9,
                    "native inset chest outline differs"
                );
                anyhow::ensure!(
                    query.initial.game_mode == Some(mode),
                    "targeting mode differs"
                );
                emit(&command, query)?;
            }
            "container_player_swap_survival" | "container_player_swap_creative" => {
                let mode = if command == "container_player_swap_survival" {
                    GameMode::Survival
                } else {
                    GameMode::Creative
                };
                let player = wait_player(client, |p| p.game_mode == Some(mode)).await?;
                let close = client
                    .survival()
                    .container_close_record()
                    .await?
                    .context("close record missing")?;
                anyhow::ensure!(
                    close.dispatched,
                    "player UI cannot resume from incomplete close"
                );
                if command == "container_player_swap_survival" {
                    anyhow::ensure!(
                        player.inventory.player_screen
                            == Some(PlayerScreenAccess::SubmittedClose { close: close.id }),
                        "local player UI close basis missing"
                    );
                }
                let submitted = if mode == GameMode::Survival {
                    client.survival().swap_hotbar(9, 0).await?
                } else {
                    client.creative().swap_hotbar(9, 0).await?
                };
                anyhow::ensure!(
                    submitted.source == InventorySwapSource::PlayerMain
                        && submitted.window_id() == 0
                        && submitted.send.dispatched,
                    "wrong resumed player swap intent"
                );
                anyhow::ensure!(
                    client.survival().swap_hotbar(9, 0).await.is_err(),
                    "resumed player swap replay admitted"
                );
                emit(&command, submitted)?;
            }
            "container_player_taken" | "container_player_returned" => {
                let completed = wait_swap(client).await?;
                let expected = command == "container_player_taken";
                anyhow::ensure!(
                    matches!(
                        completed.source_receipt.as_ref().unwrap().value,
                        SlotKnowledge::Empty
                    ) == expected,
                    "wrong resumed player main destination"
                );
                let close = client
                    .survival()
                    .container_close_record()
                    .await?
                    .context("close history missing")?;
                anyhow::ensure!(
                    client
                        .creative()
                        .swap_container_hotbar(close.id.screen(), 0, 0)
                        .await
                        .is_err(),
                    "closed storage opening reused"
                );
                emit(&command, completed)?;
            }
            "container_close_creative" | "container_close_survival" => {
                use voxrig::client::container::ContainerCloseStage;
                let mode = if command == "container_close_creative" {
                    GameMode::Creative
                } else {
                    GameMode::Survival
                };
                wait_player(client, |p| p.game_mode == Some(mode)).await?;
                let screen = opening.context("opening missing")?;
                let record = if mode == GameMode::Survival {
                    client.survival().close_container(screen).await?
                } else {
                    client.creative().close_container(screen).await?
                };
                anyhow::ensure!(
                    record.dispatched && record.id.screen() == screen,
                    "close write incomplete"
                );
                anyhow::ensure!(
                    matches!(
                        record.stage,
                        ContainerCloseStage::Dispatched | ContainerCloseStage::ObservedClosed
                    ),
                    "close uncertain"
                );
                anyhow::ensure!(
                    client.survival().close_container(screen).await.is_err()
                        && client.creative().close_container(screen).await.is_err(),
                    "close replay admitted"
                );
                anyhow::ensure!(
                    client
                        .creative()
                        .swap_container_hotbar(screen, 0, 0)
                        .await
                        .is_err(),
                    "closed opening clicked"
                );
                emit(&command, record)?;
            }
            "container_closed_change" => {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let capture = client.screen_state().await?;
                let close = client
                    .survival()
                    .container_close_record()
                    .await?
                    .context("close record missing")?;
                anyhow::ensure!(
                    close.dispatched && capture.session == close.initial.session,
                    "close basis missing"
                );
                anyhow::ensure!(
                    matches!(capture.player_screen,Some(PlayerScreenAccess::SubmittedClose { close:owning }) if owning==close.id)
                        || (capture.player_screen == Some(PlayerScreenAccess::Received)
                            && capture.active_window == Some(0)),
                    "explicit player UI missing after close"
                );
                if let Some(s) = capture.screen.as_ref() {
                    anyhow::ensure!(Some(s.id) == opening, "unrelated opening received");
                }
                // A complete client write neither drains in-flight updates nor
                // orders an independent RCON command after native close handling.
                // Keep actual updates as receive evidence instead of inventing silence.
                anyhow::ensure!(
                    client
                        .creative()
                        .swap_container_hotbar(close.id.screen(), 0, 0)
                        .await
                        .is_err(),
                    "closed opening clicked"
                );
                emit(&command, capture)?;
            }
            "container_reopen" => {
                wait_player(client, |p| p.game_mode == Some(GameMode::Survival)).await?;
                let record = client.survival().open_container([0, 65, 2]).await?;
                anyhow::ensure!(
                    record.send.dispatched && record.requires_inspection.is_none(),
                    "survival activation incomplete"
                );
                emit(&command, record)?;
            }
            "container_reopened" => {
                let capture = tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        let capture = client.screen_state().await?;
                        if capture.screen.as_ref().is_some_and(|s| Some(s.id) != opening && s.full_contents_sequence.is_some()
                            && matches!(&s.slots[0],Some(v) if matches!(&v.value,SlotKnowledge::Item { item } if item.name=="minecraft:stone" && item.count==11))
                            && capture.cursor.as_ref().is_some_and(|v| v.value==SlotKnowledge::Empty)) { return Ok::<_,anyhow::Error>(capture); }
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                }).await??;
                let record = wait_container_open(client).await?;
                anyhow::ensure!(
                    record.mode == GameMode::Survival
                        && record.observed_screen.as_ref().map(|s| s.id)
                            == capture.screen.as_ref().map(|s| s.id),
                    "survival opening facts differ"
                );
                emit("container_reopen_completed", record)?;
                opening = capture.screen.as_ref().map(|s| s.id);
                emit(&command, capture)?;
            }
            "container_baseline" => {
                let baseline = wait_player(client, |p| {
                    p.game_mode == Some(GameMode::Creative)
                        && p.received_pose.as_ref().is_some_and(|pose| pose.receive_sequence > initial_sequence && pose.position == [0.5,65.0,0.5])
                        && p.inventory.window_id == Some(0)
                        && p.inventory.cursor.as_ref().is_some_and(|c| c.value == SlotKnowledge::Empty)
                        && p.inventory.slots[36].as_ref().is_some_and(|c| c.value == SlotKnowledge::Empty)
                        && matches!(&p.inventory.slots[9],Some(c) if matches!(&c.value,SlotKnowledge::Item{item} if item.name=="minecraft:dirt" && item.count==2))
                }).await?;
                wait_block(client, [0, 65, 2], "minecraft:chest").await?;
                anyhow::ensure!(
                    client.screen_state().await?.screen.is_none(),
                    "old screen retained on fresh source"
                );
                emit("container_baseline", baseline)?;
            }
            "container_open" => {
                let dispatch = client.creative().open_container([0, 65, 2]).await?;
                anyhow::ensure!(
                    dispatch.send.dispatched && dispatch.requires_inspection.is_none(),
                    "creative activation incomplete"
                );
                emit("container_open", dispatch)?;
            }
            "container_observed" | "container_changed" => {
                let expected = if command == "container_observed" {
                    3
                } else {
                    7
                };
                let capture: ScreenObservation = tokio::time::timeout(Duration::from_secs(15),async {
                    loop {
                        let capture = client.screen_state().await?;
                        let matches = capture.screen.as_ref().is_some_and(|screen| {
                            screen.menu_name.as_deref()==Some("minecraft:generic_9x3")
                                && screen.full_contents_sequence.is_some()
                                && screen.slots.len()==63
                                && matches!(&screen.slots[0],Some(v) if matches!(&v.value,SlotKnowledge::Item{item} if item.name=="minecraft:stone" && item.count==expected) && matches!(v.source,ValueSource::Received{sequence} if sequence>content_sequence))
                                && matches!(&screen.slots[27],Some(v) if matches!(&v.value,SlotKnowledge::Item{item} if item.name=="minecraft:dirt" && item.count==2))
                                && screen.slots[54].as_ref().is_some_and(|v| v.value==SlotKnowledge::Empty)
                                && capture.cursor.as_ref().is_some_and(|v| v.value==SlotKnowledge::Empty)
                        });
                        if matches { return Ok::<_,anyhow::Error>(capture); }
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                }).await??;
                if command == "container_observed" {
                    let record = wait_container_open(client).await?;
                    anyhow::ensure!(
                        record.mode == GameMode::Creative
                            && record.observed_screen.as_ref().map(|s| s.id)
                                == capture.screen.as_ref().map(|s| s.id),
                        "creative opening facts differ"
                    );
                    emit("container_open_completed", record)?;
                }
                let screen = capture.screen.as_ref().context("screen missing")?;
                anyhow::ensure!(
                    screen.id.session() == capture.session
                        && capture.active_window == Some(screen.id.window_id()),
                    "screen identity mismatch"
                );
                let layout = screen.layout.as_ref().context("native layout missing")?;
                anyhow::ensure!(
                    layout.total_slots == 63
                        && layout.player_slots.len() == 36
                        && layout.player_slots[0].screen_slot == 27
                        && layout.player_slots[0].player_slot == 9
                        && layout.player_slots[27].screen_slot == 54
                        && layout.player_slots[27].player_slot == 36,
                    "wrong native player mapping"
                );
                let player = client.player_state().await?;
                anyhow::ensure!(
                    player.inventory.slots[9] == screen.slots[27]
                        && player.inventory.slots[36] == screen.slots[54],
                    "player projection mismatch"
                );
                if let Some(id) = opening {
                    anyhow::ensure!(id == screen.id, "slot update replaced opening identity");
                } else {
                    opening = Some(screen.id);
                }
                if let ValueSource::Received { sequence } = screen.slots[0].as_ref().unwrap().source
                {
                    content_sequence = sequence;
                }
                emit(&command, capture)?;
            }
            "container_disconnect" => {
                let close = client.survival().container_close_record().await?;
                let open = client
                    .survival()
                    .container_open_record()
                    .await?
                    .context("open history missing")?;
                client.disconnect().await?;
                let retained = client
                    .creative()
                    .container_open_record()
                    .await?
                    .context("closed open history missing")?;
                anyhow::ensure!(
                    retained.id == open.id
                        && retained.stage
                            == voxrig::client::container::ContainerOpenStage::ObservedContents,
                    "closed open history changed"
                );
                emit("container_open_disconnected", retained)?;
                anyhow::ensure!(
                    client.screen_state().await.is_err(),
                    "closed screen treated as live"
                );
                anyhow::ensure!(
                    client
                        .creative()
                        .container_close_record()
                        .await?
                        .map(|r| r.id)
                        == close.map(|r| r.id),
                    "close history lost after disconnect"
                );
                emit("container_disconnected", serde_json::json!({"closed":true}))?;
                return Ok(());
            }
            "container_swap_survival" | "container_swap_creative" => {
                let mode = if command == "container_swap_survival" {
                    GameMode::Survival
                } else {
                    GameMode::Creative
                };
                wait_player(client, |p| p.game_mode == Some(mode)).await?;
                let screen = opening.context("received opening missing")?;
                let submitted = if mode == GameMode::Survival {
                    client
                        .survival()
                        .swap_container_hotbar(screen, 0, 0)
                        .await?
                } else {
                    client
                        .creative()
                        .swap_container_hotbar(screen, 0, 0)
                        .await?
                };
                anyhow::ensure!(submitted.send.dispatched, "container click incomplete");
                anyhow::ensure!(
                    submitted.source == InventorySwapSource::Container { screen }
                        && submitted.source_slot == 0
                        && submitted.hotbar_screen_slot == 54,
                    "wrong swap capture"
                );
                anyhow::ensure!(
                    client
                        .creative()
                        .swap_container_hotbar(screen, 0, 0)
                        .await
                        .is_err(),
                    "duplicate container click admitted"
                );
                emit(&command, submitted)?;
            }
            "container_transfer_survival"
            | "container_transfer_creative"
            | "player_transfer_survival"
            | "player_transfer_creative"
            | "pumpkin_transfer_survival"
            | "pumpkin_transfer_creative_armor"
            | "pumpkin_transfer_creative_hotbar"
            | "helmet_transfer_survival"
            | "helmet_transfer_creative" => {
                let mode = if command.ends_with("survival") {
                    GameMode::Survival
                } else {
                    GameMode::Creative
                };
                wait_player(client, |p| p.game_mode == Some(mode)).await?;
                let source = if command.starts_with("container_transfer") {
                    InventorySource::Container {
                        screen: opening.context("opening missing")?,
                    }
                } else {
                    InventorySource::Player
                };
                let slot = match command.as_str() {
                    "container_transfer_survival" => 0,
                    "container_transfer_creative" => 62,
                    "player_transfer_survival" => 9,
                    "player_transfer_creative" | "pumpkin_transfer_creative_hotbar" => 36,
                    "pumpkin_transfer_creative_armor" | "helmet_transfer_creative" => 5,
                    _ => 10,
                };
                if command == "pumpkin_transfer_survival" || command == "helmet_transfer_survival" {
                    let name = if command.starts_with("pumpkin") {
                        "minecraft:carved_pumpkin"
                    } else {
                        "minecraft:diamond_helmet"
                    };
                    let count = if command.starts_with("pumpkin") { 7 } else { 1 };
                    wait_player(client, |p| p.inventory.slots[10].as_ref().is_some_and(|s| matches!(&s.value, SlotKnowledge::Item{item} if item.name == name && item.count == count)) && p.inventory.slots[5].as_ref().is_some_and(|s| s.value == SlotKnowledge::Empty)).await?;
                }
                let record = if mode == GameMode::Survival {
                    client.survival().transfer_inventory(source, slot).await?
                } else {
                    client.creative().transfer_inventory(source, slot).await?
                };
                anyhow::ensure!(
                    record.send.dispatched && record.source_slot == slot && record.mode == mode,
                    "transfer submission differs"
                );
                emit(&command, record)?;
            }
            "container_transfer_taken"
            | "container_transfer_returned"
            | "player_transfer_taken"
            | "player_transfer_returned"
            | "pumpkin_transfer_equipped"
            | "pumpkin_transfer_armor_returned"
            | "pumpkin_transfer_hotbar_equipped"
            | "helmet_transfer_equipped"
            | "helmet_transfer_returned" => {
                let record = wait_transfer(client).await?;
                if command == "container_transfer_taken" {
                    anyhow::ensure!(
                        record
                            .changed_slots
                            .iter()
                            .any(|s| s.slot == 62 && s.player_slot == Some(44)),
                        "storage transfer must use native reverse order"
                    );
                }
                if command == "pumpkin_transfer_equipped" {
                    anyhow::ensure!(
                        record.changed_slots.len() == 3
                            && record
                                .changed_slots
                                .iter()
                                .any(|s| s.slot == 5 && stack_count(&s.prediction.value) == 1)
                            && record
                                .changed_slots
                                .iter()
                                .any(|s| s.slot == 36 && stack_count(&s.prediction.value) == 6),
                        "one native QUICK_MOVE must equip one pumpkin and move remaining six"
                    );
                }
                emit(&command, record)?;
            }
            "transfer_fixture_cleared" => {
                emit("transfer_fixture_before_wait", client.player_state().await?)?;
                let result = wait_player(client, |p| {
                    [5, 10, 36, 44].iter().all(|i| {
                        p.inventory.slots[*i]
                            .as_ref()
                            .is_some_and(|s| s.value == SlotKnowledge::Empty)
                    }) && p
                        .inventory
                        .cursor
                        .as_ref()
                        .is_some_and(|c| c.value == SlotKnowledge::Empty)
                })
                .await;
                if result.is_err() {
                    emit("transfer_fixture_timeout", client.player_state().await?)?;
                }
                emit(&command, result?)?;
            }
            "container_pickup_survival"
            | "container_pickup_creative_one"
            | "container_pickup_creative_return" => {
                let mode = if command == "container_pickup_survival" {
                    GameMode::Survival
                } else {
                    GameMode::Creative
                };
                wait_player(client, |p| p.game_mode == Some(mode)).await?;
                let source = InventoryClickSource::Container {
                    screen: opening.context("opening missing")?,
                };
                let button = if command == "container_pickup_creative_return" {
                    InventoryClickButton::Left
                } else {
                    InventoryClickButton::Right
                };
                let record = if mode == GameMode::Survival {
                    client.survival().click_inventory(source, 0, button).await?
                } else {
                    client.creative().click_inventory(source, 0, button).await?
                };
                anyhow::ensure!(
                    client
                        .creative()
                        .click_inventory(source, 0, button)
                        .await
                        .is_err(),
                    "duplicate pickup admitted"
                );
                emit(&command, record)?;
            }
            "container_pickup_split" | "container_pickup_one" | "container_pickup_returned" => {
                let record = wait_pickup(client).await?;
                let expected = match command.as_str() {
                    "container_pickup_split" => (3, 4),
                    "container_pickup_one" => (4, 3),
                    _ => (7, 0),
                };
                anyhow::ensure!(
                    (
                        stack_count(&record.source_receipt.as_ref().unwrap().value),
                        stack_count(&record.cursor_receipt.as_ref().unwrap().value)
                    ) == expected,
                    "unexpected storage pickup outcome"
                );
                emit(&command, record)?;
            }
            "player_pickup_survival"
            | "player_pickup_creative_one"
            | "player_pickup_creative_return"
            | "player_pickup_creative_retake"
            | "player_pickup_creative_restore" => {
                let mode = if command == "player_pickup_survival" {
                    GameMode::Survival
                } else {
                    GameMode::Creative
                };
                wait_player(client, |p| p.game_mode == Some(mode)).await?;
                let slot = if command == "player_pickup_creative_one"
                    || command == "player_pickup_creative_retake"
                {
                    36
                } else {
                    9
                };
                let button = if command == "player_pickup_creative_one" {
                    InventoryClickButton::Right
                } else {
                    InventoryClickButton::Left
                };
                let record = if mode == GameMode::Survival {
                    client
                        .survival()
                        .click_inventory(InventoryClickSource::Player, slot, button)
                        .await?
                } else {
                    client
                        .creative()
                        .click_inventory(InventoryClickSource::Player, slot, button)
                        .await?
                };
                emit(&command, record)?;
            }
            "player_pickup_taken"
            | "player_pickup_one"
            | "player_pickup_returned"
            | "player_pickup_retaken"
            | "player_pickup_restored" => {
                let record = wait_pickup(client).await?;
                let expected = match command.as_str() {
                    "player_pickup_taken" => (0, 2),
                    "player_pickup_one" => (1, 1),
                    "player_pickup_returned" => (1, 0),
                    "player_pickup_retaken" => (0, 1),
                    _ => (2, 0),
                };
                anyhow::ensure!(
                    (
                        stack_count(&record.source_receipt.as_ref().unwrap().value),
                        stack_count(&record.cursor_receipt.as_ref().unwrap().value)
                    ) == expected,
                    "unexpected player pickup outcome"
                );
                emit(&command, record)?;
            }
            "container_swap_taken" | "container_swap_returned" => {
                let completed = wait_swap(client).await?;
                let expected_empty = command == "container_swap_taken";
                anyhow::ensure!(
                    matches!(
                        &completed.source_receipt.as_ref().unwrap().value,
                        SlotKnowledge::Empty
                    ) == expected_empty,
                    "wrong container destination"
                );
                let screen = client.screen_state().await?;
                anyhow::ensure!(
                    screen
                        .screen
                        .as_ref()
                        .is_some_and(|s| Some(s.id) == opening),
                    "exchange replaced opening"
                );
                emit(&command, completed)?;
            }
            _ => anyhow::bail!("unexpected container fixture command"),
        }
    }
    anyhow::bail!("container fixture ended without disconnect")
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port: u16 = std::env::var("VOXRIG_PORT")?.parse()?;
    let config =
        ConnectionConfig::offline_from_env(Server::new("127.0.0.1", port), "UnifiedProbe")?;
    let client = Client::connect(config).await?;
    client.wait_until_ready().await?;
    if std::env::var("VOXRIG_NATIVE_SCENARIO").ok().as_deref() == Some("container") {
        return container_probe(&client).await;
    }
    if std::env::var("VOXRIG_NATIVE_SCENARIO").ok().as_deref() == Some("inventory") {
        return inventory_probe(&client).await;
    }
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

async fn wait_container_open(
    client: &Client,
) -> anyhow::Result<voxrig::client::container::ContainerOpenRecord> {
    use voxrig::client::container::ContainerOpenStage;
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let record = client
                .survival()
                .container_open_record()
                .await?
                .context("activation history missing")?;
            anyhow::ensure!(
                record.requires_inspection.is_none(),
                "activation uncertain: {:?}",
                record.requires_inspection
            );
            if record.stage == ContainerOpenStage::ObservedContents {
                anyhow::ensure!(
                    record.send.dispatched
                        && record.observed_screen.is_some()
                        && record.received_cursor.is_some(),
                    "opening received facts incomplete"
                );
                if client.version() == MinecraftVersion::Java1_21_11 {
                    anyhow::ensure!(
                        record.protocol_processing.as_ref().is_some_and(|ack| Some(
                            ack.acknowledged_sequence
                        ) >= record
                            .send
                            .interaction_sequence),
                        "processing ACK missing"
                    );
                }
                return Ok(record);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await?
}

async fn wait_barrel_target(
    client: &Client,
    mode: GameMode,
    open: &str,
) -> anyhow::Result<BlockTargetObservation> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let query = if mode == GameMode::Creative {
                client.creative().target_block(4.5).await
            } else {
                client.survival().target_block(4.5).await
            };
            if let Ok(query) = query {
                if query.hit.as_ref().is_some_and(|hit| {
                    hit.position == [0, 65, 2]
                        && hit.state.name == "minecraft:barrel"
                        && hit.state.properties.get("open").map(String::as_str) == Some(open)
                }) {
                    return Ok(query);
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await?
}
