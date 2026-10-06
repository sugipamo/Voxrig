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
async fn crafting_pickup(
    client: &Client,
    mode: GameMode,
    source: InventorySource,
    slot: u16,
    button: InventoryClickButton,
) -> anyhow::Result<InventoryClickRecord> {
    let sent = match mode {
        GameMode::Survival => {
            client
                .survival()
                .click_inventory(source, slot, button)
                .await?
        }
        _ => {
            client
                .creative()
                .click_inventory(source, slot, button)
                .await?
        }
    };
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let record = client
                .survival()
                .inventory_click_record()
                .await?
                .context("crafting click absent")?;
            anyhow::ensure!(
                record.id == sent.id && record.requires_inspection.is_none(),
                "crafting click conflict: {:?}",
                record.requires_inspection
            );
            if record.stage == InventoryClickStage::ObservedClicked {
                return Ok::<_, anyhow::Error>(record);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?
}
async fn crafting_result_take(
    client: &Client,
    mode: GameMode,
    expected: &str,
) -> anyhow::Result<CraftingTakeRecord> {
    let previous = client
        .survival()
        .inventory_click_record()
        .await?
        .context("crafting predecessor click absent")?;
    // A negative legacy comparison queues full contents followed by the cursor.
    // The click's own fresh source/cursor can precede those queued updates. Wait
    // for the actual full grid AND its cursor before capturing a take intent.
    let full_after = previous
        .legacy_reply
        .as_ref()
        .filter(|r| !r.accepted)
        .map(|r| r.receive_sequence)
        .or_else(|| {
            previous
                .send
                .request_full_resync
                .then_some(previous.send.after_sequence)
        });
    let grid = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let grid = client
                .received_crafting()
                .await?
                .context("crafting grid absent")?;
            let result = grid
                .result()
                .filter(|r| matches!(r.value(),SlotKnowledge::Item{item} if item.name==expected));
            if let Some(result) = result {
                let sequence = result.receive_sequence();
                let mut complete = true;
                if let Some(after) = full_after {
                    let screen = client.screen_state().await?;
                    let mut minimum_input = u64::MAX;
                    for y in 0..grid.dimensions()[1] {
                        for x in 0..grid.dimensions()[0] {
                            if let Some(input) = grid.input(x, y)? {
                                minimum_input = minimum_input.min(input.receive_sequence());
                            } else {
                                minimum_input = 0;
                            }
                        }
                    }
                    let full_sequence = match grid.source() {
                        CraftingSource::Table { screen: opening } => screen
                            .screen
                            .as_ref()
                            .filter(|s| s.id == opening)
                            .and_then(|s| s.full_contents_sequence),
                        CraftingSource::Player { .. } => Some(minimum_input),
                        _ => None,
                    };
                    let inventory = client.received_inventory().await?;
                    complete = full_sequence.is_some_and(|full| {
                        full > after
                            && minimum_input >= full
                            && sequence >= full
                            && inventory.session() == grid.session()
                            && inventory.cursor().is_some_and(|r| {
                                r.receive_sequence() >= full
                                    && matches!(r.value(), SlotKnowledge::Empty)
                            })
                    });
                }
                if complete {
                    return Ok::<_, anyhow::Error>(grid);
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .context("crafting result baseline wait")??;
    let sent = match mode {
        GameMode::Survival => client.survival().take_crafting_result(&grid).await?,
        _ => client.creative().take_crafting_result(&grid).await?,
    };
    let complete = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let record = client
                .survival()
                .crafting_take_record()
                .await?
                .context("crafting take absent")?;
            anyhow::ensure!(
                record.id == sent.id && record.requires_inspection.is_none(),
                "crafting take conflict: {:?}",
                record.requires_inspection
            );
            if record.stage == CraftingTakeStage::ObservedTaken {
                return Ok::<_, anyhow::Error>(record);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    let after = complete.after.as_ref().context("actual full grid absent")?;
    let sequence = after
        .result()
        .context("fresh result absent")?
        .receive_sequence();
    anyhow::ensure!(sequence > complete.send.after_sequence, "result not fresh");
    for y in 0..after.dimensions()[1] {
        for x in 0..after.dimensions()[0] {
            anyhow::ensure!(
                after
                    .input(x, y)?
                    .context("fresh input absent")?
                    .receive_sequence()
                    == sequence,
                "inputs not one actual full boundary"
            );
        }
    }
    let cursor = complete
        .cursor_receipt
        .as_ref()
        .context("output cursor absent")?;
    anyhow::ensure!(
        matches!(cursor.source, voxrig::client::ValueSource::Received {sequence} if sequence > complete.send.after_sequence)
            && matches!(&cursor.value, SlotKnowledge::Item {item} if item.name == expected),
        "output not freshly received"
    );
    if let Some(reply) = &complete.legacy_reply {
        anyhow::ensure!(!reply.accepted, "actual resync comparison reply absent");
    }
    // This sealed predecessor now differs. Refusal must not resend the take.
    let refusal = match mode {
        GameMode::Survival => client.survival().take_crafting_result(&grid).await,
        _ => client.creative().take_crafting_result(&grid).await,
    };
    anyhow::ensure!(refusal.is_err(), "old crafting snapshot was reused");
    Ok(complete)
}
async fn crafting_result_probe(
    client: &Client,
    mode: GameMode,
    cake: bool,
) -> anyhow::Result<serde_json::Value> {
    let initial = client
        .received_crafting()
        .await?
        .context("crafting grid absent")?;
    let source = match initial.source() {
        CraftingSource::Player { .. } => InventorySource::Player,
        CraftingSource::Table { screen } => InventorySource::Container { screen },
        _ => anyhow::bail!("unknown crafting source"),
    };
    let active_screen = client.screen_state().await?.screen;
    let source_slot = |canonical: usize| -> anyhow::Result<u16> {
        match source {
            InventorySource::Player => Ok(canonical as u16),
            InventorySource::Container { .. } => {
                // Original appended-player mapping; no hardcoded cross-version offset.
                let table = initial.input_source(0, 0)?.0;
                anyhow::ensure!(table == source, "table identity changed");
                Ok(active_screen
                    .as_ref()
                    .context("table absent")?
                    .layout
                    .as_ref()
                    .context("native mapping absent")?
                    .player_slots
                    .iter()
                    .find(|m| m.player_slot == canonical)
                    .context("mapped player slot absent")?
                    .screen_slot as u16)
            }
            _ => anyhow::bail!("unknown inventory source"),
        }
    };
    let mut clicks = Vec::new();
    let plans = if cake {
        anyhow::ensure!(initial.dimensions() == [3, 3], "cake table absent");
        vec![
            (9, vec![(0, 0)], false),
            (10, vec![(1, 0)], false),
            (11, vec![(2, 0)], false),
            (12, vec![(0, 1), (2, 1)], true),
            (13, vec![(1, 1)], false),
            (14, vec![(0, 2), (1, 2), (2, 2)], true),
        ]
    } else {
        anyhow::ensure!(initial.dimensions() == [2, 2], "sticks player grid absent");
        vec![(9, vec![(0, 0), (0, 0), (0, 1), (0, 1)], true)]
    };
    for (canonical, coordinates, split) in plans {
        clicks.push(
            crafting_pickup(
                client,
                mode,
                source,
                source_slot(canonical)?,
                InventoryClickButton::Left,
            )
            .await?,
        );
        for (x, y) in coordinates {
            let (input, slot) = initial.input_source(x, y)?;
            clicks.push(
                crafting_pickup(
                    client,
                    mode,
                    input,
                    slot,
                    if split {
                        InventoryClickButton::Right
                    } else {
                        InventoryClickButton::Left
                    },
                )
                .await?,
            );
        }
    }
    let take = crafting_result_take(
        client,
        mode,
        if cake {
            "minecraft:cake"
        } else {
            "minecraft:stick"
        },
    )
    .await?;
    let after = take.after.as_ref().context("taken grid absent")?;
    if cake {
        for y in 0..3 {
            for x in 0..3 {
                let value = after.input(x, y)?.context("cake input missing")?.value();
                anyhow::ensure!(
                    if y == 0 {
                        matches!(value,SlotKnowledge::Item {item} if item.name == "minecraft:bucket" && item.count == 1)
                    } else {
                        *value == SlotKnowledge::Empty
                    },
                    "native cake remainder differs"
                );
            }
        }
        anyhow::ensure!(
            *after.result().unwrap().value() == SlotKnowledge::Empty,
            "cake result not actually empty"
        );
    } else {
        anyhow::ensure!(
            matches!(after.result().unwrap().value(),SlotKnowledge::Item {item} if item.name == "minecraft:stick" && item.count == 4),
            "regenerated sticks result lost"
        );
        for y in 0..2 {
            anyhow::ensure!(
                after
                    .input(0, y)?
                    .is_some_and(|r| stack_count(r.value()) == 1),
                "native sticks input not consumed once"
            );
        }
        clicks.push(crafting_pickup(client, mode, source, 10, InventoryClickButton::Left).await?);
        for y in 0..2 {
            let (input, slot) = initial.input_source(0, y)?;
            clicks.push(
                crafting_pickup(client, mode, input, slot, InventoryClickButton::Left).await?,
            );
            clicks
                .push(crafting_pickup(client, mode, source, 9, InventoryClickButton::Left).await?);
        }
    }
    Ok(
        serde_json::json!({"initial":initial,"clicks":clicks,"take":take,"player_after":client.player_state().await?}),
    )
}
async fn crafting_input_probe(
    client: &Client,
    mode: GameMode,
) -> anyhow::Result<serde_json::Value> {
    let initial = client
        .received_crafting()
        .await?
        .context("player crafting unavailable")?;
    anyhow::ensure!(initial.dimensions() == [2, 2], "player grid differs");
    for y in 0..2 {
        for x in 0..2 {
            anyhow::ensure!(
                initial
                    .input(x, y)?
                    .is_some_and(|r| *r.value() == SlotKnowledge::Empty),
                "crafting input not received empty"
            );
        }
    }
    let (grid_source, grid_slot) = initial.input_source(0, 0)?;
    let mut records = Vec::new();
    let mut filled = None;
    for (source, slot, button, ingredient, carried) in [
        (
            InventorySource::Player,
            9,
            InventoryClickButton::Left,
            None,
            3,
        ),
        (
            grid_source,
            grid_slot,
            InventoryClickButton::Right,
            Some(1),
            2,
        ),
        (
            InventorySource::Player,
            9,
            InventoryClickButton::Left,
            None,
            0,
        ),
        (
            grid_source,
            grid_slot,
            InventoryClickButton::Left,
            Some(0),
            1,
        ),
        (
            InventorySource::Player,
            9,
            InventoryClickButton::Left,
            None,
            0,
        ),
    ] {
        let sent = match mode {
            GameMode::Survival => {
                client
                    .survival()
                    .click_inventory(source, slot, button)
                    .await?
            }
            GameMode::Creative => {
                client
                    .creative()
                    .click_inventory(source, slot, button)
                    .await?
            }
            _ => anyhow::bail!("unexpected crafting mode"),
        };
        let complete = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let record = client
                    .survival()
                    .inventory_click_record()
                    .await?
                    .context("crafting click missing")?;
                anyhow::ensure!(
                    record.id == sent.id && record.requires_inspection.is_none(),
                    "crafting click interrupted: {:?}",
                    record.requires_inspection
                );
                if record.stage == InventoryClickStage::ObservedClicked {
                    return Ok::<_, anyhow::Error>(record);
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await??;
        let inventory = client.received_inventory().await?;
        let cursor = inventory.cursor().context("crafting cursor missing")?;
        anyhow::ensure!(
            cursor.receive_sequence() > complete.send.after_sequence
                && stack_count(cursor.value()) == carried,
            "crafting cursor not freshly established"
        );
        if let Some(count) = ingredient {
            let crafting = tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    let value = client
                        .received_crafting()
                        .await?
                        .context("crafting disappeared")?;
                    if value.input(0, 0)?.is_some_and(|r| {
                        r.receive_sequence() > complete.send.after_sequence
                            && stack_count(r.value()) == count
                    }) && value.result().is_some_and(|r| {
                        r.receive_sequence() > complete.send.after_sequence
                            && if count == 0 {
                                *r.value() == SlotKnowledge::Empty
                            } else {
                                r.item().is_some_and(|i| {
                                    i.stack().name == "minecraft:oak_button" && i.stack().count == 1
                                })
                            }
                    }) {
                        return Ok::<_, anyhow::Error>(value);
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            })
            .await??;
            if count == 1 {
                filled = Some(crafting);
            }
        }
        records.push(complete);
    }
    let final_inventory = client.received_inventory().await?;
    anyhow::ensure!(
        final_inventory
            .slot(9)?
            .and_then(ReceivedSlot::item)
            .is_some_and(|i| i.stack().name == "minecraft:oak_planks" && i.stack().count == 3),
        "crafting input round trip changed material"
    );
    let cleared = client
        .received_crafting()
        .await?
        .context("final crafting unavailable")?;
    Ok(
        serde_json::json!({"mode":mode,"steps":records,"filled":filled,"cleared":cleared,"final_inventory":final_inventory,
        "authority_limits":"Actual input/cursor receipts and native displayed oak-button result only; no result take or recipe consumption is submitted or claimed."}),
    )
}
async fn crafting_table_fill_probe(
    client: &Client,
    mode: GameMode,
) -> anyhow::Result<serde_json::Value> {
    let grid = client
        .received_crafting()
        .await?
        .context("table grid unavailable")?;
    anyhow::ensure!(grid.dimensions() == [3, 3], "table dimensions differ");
    let CraftingSource::Table { screen: id } = grid.source() else {
        anyhow::bail!("received table identity missing")
    };
    for y in 0..3 {
        for x in 0..3 {
            anyhow::ensure!(
                grid.input(x, y)?
                    .is_some_and(|r| *r.value() == SlotKnowledge::Empty),
                "table input not received empty"
            );
        }
    }
    let screen = client
        .screen_state()
        .await?
        .screen
        .context("table screen missing")?;
    anyhow::ensure!(screen.id == id, "table opening changed");
    let player_slot = u16::try_from(
        screen
            .layout
            .as_ref()
            .context("table layout missing")?
            .player_slots
            .iter()
            .find(|m| m.player_slot == 9)
            .context("table main inventory mapping missing")?
            .screen_slot,
    )?;
    let (source, input_slot) = grid.input_source(2, 2)?;
    let mut steps = Vec::new();
    for (slot, button, count) in [
        (player_slot, InventoryClickButton::Left, 3),
        (input_slot, InventoryClickButton::Right, 2),
        (player_slot, InventoryClickButton::Right, 1),
    ] {
        let sent = match mode {
            GameMode::Survival => {
                client
                    .survival()
                    .click_inventory(source, slot, button)
                    .await?
            }
            GameMode::Creative => {
                client
                    .creative()
                    .click_inventory(source, slot, button)
                    .await?
            }
            _ => anyhow::bail!("unexpected table mode"),
        };
        let record = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let r = client
                    .survival()
                    .inventory_click_record()
                    .await?
                    .context("table input click missing")?;
                anyhow::ensure!(
                    r.id == sent.id && r.requires_inspection.is_none(),
                    "table input interrupted: {:?}",
                    r.requires_inspection
                );
                if r.stage == InventoryClickStage::ObservedClicked {
                    return Ok::<_, anyhow::Error>(r);
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await??;
        anyhow::ensure!(
            record
                .cursor_receipt
                .as_ref()
                .is_some_and(|v| stack_count(&v.value) == count),
            "table carried count differs"
        );
        steps.push(record);
    }
    let filled = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let value = client
                .received_crafting()
                .await?
                .context("filled table unavailable")?;
            let after = steps[1].send.after_sequence;
            if value.source() == (CraftingSource::Table { screen: id })
                && value.input(2, 2)?.is_some_and(|r| {
                    r.receive_sequence() > after
                        && r.item().is_some_and(|i| {
                            i.stack().name == "minecraft:oak_planks" && i.stack().count == 1
                        })
                })
                && value.result().is_some_and(|r| {
                    r.receive_sequence() > after
                        && r.item().is_some_and(|i| {
                            i.stack().name == "minecraft:oak_button" && i.stack().count == 1
                        })
                })
            {
                return Ok::<_, anyhow::Error>(value);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await??;
    let selection_dispatch = match mode {
        GameMode::Survival => client.survival().select_hotbar(2).await?,
        GameMode::Creative => client.creative().select_hotbar(2).await?,
        _ => anyhow::bail!("unexpected selection mode"),
    };
    let context = client
        .received_crafting_context()
        .await?
        .context("filled coherent table context missing")?;
    anyhow::ensure!(
        context.grid().source() == filled.source()
            && context.receive_sequence() >= filled.receive_sequence(),
        "coherent table boundary differs"
    );
    let recipe =
        find_stick_recipe(context.recipes(), client).context("table stick display missing")?;
    let layout = context.recipe_layout(recipe.id())?;
    anyhow::ensure!(
        layout.grid_dimensions() == [3, 3]
            && layout
                .cells()
                .iter()
                .map(|c| c.coordinate())
                .collect::<Vec<_>>()
                == [[1, 0], [1, 1]],
        "table stick layout differs"
    );
    let return_plan = context.grid_return_plan()?;
    anyhow::ensure!(
        return_plan.fits()
            && return_plan.unreturned_splits().is_empty()
            && return_plan.selected_hotbar().value == 2
            && return_plan.selected_hotbar().source == voxrig::client::ValueSource::Submitted
            && return_plan.steps().len() == 1
            && return_plan.steps()[0].input() == [2, 2]
            && return_plan.steps()[0].player_slot() == 9
            && return_plan.steps()[0].amount() == 1,
        "filled table return prediction differs"
    );
    let predicted = &return_plan
        .predictions()
        .iter()
        .find(|(i, _)| *i == 9)
        .context("table return prediction missing")?
        .1;
    anyhow::ensure!(
        predicted.source == voxrig::client::ValueSource::Predicted
            && stack_count(&predicted.value) == 2,
        "grid return improperly includes carried cursor material"
    );
    Ok(
        serde_json::json!({"steps":steps,"grid":filled,"inventory":client.received_inventory().await?,"player_source_slot":player_slot,"input_slot":input_slot,"context_sequence":context.receive_sequence(),"recipe_layout":layout,"grid_return_plan":return_plan,"selection_dispatch":selection_dispatch}),
    )
}
// A fresh connection isolates mining's unresolved continuation boundary from
// the earlier motion scenario. Both versions execute this exact consumer.
async fn mining_probe(
    client: &Client,
    recovery_config: Option<&ConnectionConfig>,
) -> anyhow::Result<()> {
    let mut recovered: Option<RecoveredSurvivalClient> = None;
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
            "mining_pending_finish" => {
                let ops = client.survival();
                let original = ops
                    .mining_record()
                    .await?
                    .context("pending source mining missing")?;
                let finished = ops.finish_mining(original.id).await?;
                let finish = finished.finish.as_ref().context("pending FINISH missing")?;
                let processed = tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        let record = ops
                            .mining_record()
                            .await?
                            .context("pending mining missing")?;
                        use voxrig::client::survival::MiningProtocolObservation;
                        let received = match record.protocol.as_ref() {
                            Some(MiningProtocolObservation::LegacyReply {
                                action: 2,
                                accepted: true,
                                state,
                                receive_sequence,
                            }) => {
                                state.name == "minecraft:stone"
                                    && *receive_sequence > finish.after_sequence
                            }
                            Some(MiningProtocolObservation::ModernProcessing {
                                sequence,
                                receive_sequence,
                            }) => {
                                finish
                                    .interaction_sequence
                                    .is_some_and(|expected| *sequence >= expected)
                                    && *receive_sequence > finish.after_sequence
                            }
                            _ => false,
                        };
                        if received {
                            return Ok::<_, anyhow::Error>(record);
                        }
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                })
                .await??;
                anyhow::ensure!(
                    processed.stage == MiningStage::PendingAfterFinish
                        && !processed.continuation_validated,
                    "early FINISH unexpectedly released mining"
                );
                anyhow::ensure!(
                    ops.look([0.0; 2]).await.is_err(),
                    "early FINISH admitted continuation"
                );
                emit("mining_pending_finish", processed)?;
            }
            "mining_recover" => {
                let config = recovery_config.context("recovery scenario configuration missing")?;
                let ops = client.survival();
                let original = ops
                    .mining_record()
                    .await?
                    .context("original mining missing")?;
                let watch = ops.prepare_mining_profile_recovery(original.id).await?;
                let clone = watch.clone();
                anyhow::ensure!(
                    watch
                        .reconnect(config.clone(), MiningRecoveryTarget::OriginalOrAir)
                        .await
                        .is_err(),
                    "open source admitted recovery"
                );
                anyhow::ensure!(
                    watch.source_record().await?.recovery_attempt.is_none(),
                    "open source consumed login claim"
                );
                watch.close_source().await?;
                let result = watch
                    .reconnect(config.clone(), MiningRecoveryTarget::OriginalOrAir)
                    .await?;
                anyhow::ensure!(
                    clone
                        .reconnect(config.clone(), MiningRecoveryTarget::OriginalOrAir)
                        .await
                        .is_err(),
                    "cloned watch admitted second login"
                );
                anyhow::ensure!(
                    ops.look([0.0; 2]).await.is_err(),
                    "recovery released old source"
                );
                anyhow::ensure!(
                    result
                        .client
                        .survival()
                        .finish_mining(original.id)
                        .await
                        .is_err(),
                    "fresh client accepted old mining ID"
                );
                anyhow::ensure!(
                    !result.evidence.original.continuation_validated
                        && result.evidence.original.recovery_attempt.is_some(),
                    "source claim/history lost"
                );
                emit("mining_recover", &result.evidence)?;
                recovered = Some(result);
            }
            "recovery_place" => {
                let fresh = &recovered.as_ref().context("fresh recovery missing")?.client;
                let ops = fresh.survival();
                ops.select_hotbar(1).await?;
                let pending =
                    std::env::var("VOXRIG_NATIVE_RECOVERY_CASE").ok().as_deref() == Some("pending");
                let (rotation, support) = if pending {
                    ([-90.0, 35.0], [2, 64, 0])
                } else {
                    ([0.0, 31.0], [0, 64, 3])
                };
                ops.look(rotation).await?;
                let hit = ops
                    .target_block(4.5)
                    .await?
                    .hit
                    .context("fresh placement support missing")?;
                anyhow::ensure!(
                    hit.position == support && hit.face == BlockFace::Up,
                    "unexpected fresh placement hit: {:?}",
                    hit
                );
                ops.place_cube(hit.position, hit.face).await?;
                let placed = tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        let record = ops
                            .placement_record()
                            .await?
                            .context("fresh placement missing")?;
                        match record.stage {
                            PlacementStage::ObservedPlaced => {
                                return Ok::<_, anyhow::Error>(record);
                            }
                            PlacementStage::RequiresInspection => anyhow::bail!(
                                "fresh placement interrupted: {:?}",
                                record.requires_inspection
                            ),
                            _ => tokio::time::sleep(Duration::from_millis(25)).await,
                        }
                    }
                })
                .await??;
                emit(
                    "recovery_place",
                    serde_json::json!({"placement": placed, "player":fresh.player_state().await?, "original":client.survival().mining_record().await?}),
                )?;
            }
            "recovery_disconnect" => {
                recovered
                    .as_ref()
                    .context("fresh recovery missing")?
                    .client
                    .disconnect()
                    .await?;
                emit(
                    "recovery_disconnect",
                    client.survival().mining_record().await?,
                )?;
                return Ok(());
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
fn find_stick_recipe(catalogue: &ReceivedRecipes, client: &Client) -> Option<ReceivedRecipe> {
    let registry = client.registry();
    let stick = registry.item("minecraft:stick").unwrap().id;
    let planks = registry.item("minecraft:oak_planks").unwrap().id;
    catalogue
        .entries()
        .iter()
        .find(|entry| {
            matches!(
                entry.display(),
                RecipeDisplay::Shaped {
                    width: 1,
                    height: 2,
                    ..
                }
            ) && entry
                .output_items(catalogue)
                .is_ok_and(|ids| ids.contains(&stick))
                && entry.requirements().is_some_and(|requirements| {
                    requirements.len() == 2
                        && requirements
                            .iter()
                            .all(|r| r.items(catalogue).is_ok_and(|ids| ids.contains(&planks)))
                })
        })
        .cloned()
}
fn has_cake_recipe(catalogue: &ReceivedRecipes, client: &Client) -> bool {
    let cake = client.registry().item("minecraft:cake").unwrap().id;
    catalogue.entries().iter().any(|r| {
        matches!(
            r.display(),
            RecipeDisplay::Shaped {
                width: 3,
                height: 3,
                ..
            }
        ) && r.unlocked() == Some(true)
            && r.output_items(catalogue)
                .is_ok_and(|ids| ids.contains(&cake))
    })
}
async fn received_recipe_fixture(
    client: &Client,
    unlocked: bool,
) -> anyhow::Result<ReceivedRecipes> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let catalogue = client.received_recipes().await?;
            let entry = find_stick_recipe(&catalogue, client);
            let observed = if unlocked {
                entry.as_ref().is_some_and(|r| r.unlocked() == Some(true))
                    && has_cake_recipe(&catalogue, client)
            } else if client.version() == voxrig::MinecraftVersion::Java1_16_1 {
                entry.as_ref().is_some_and(|r| r.unlocked() == Some(false))
            } else {
                entry.is_none()
            };
            if catalogue.book_initialized() && observed {
                return Ok::<_, anyhow::Error>(catalogue);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?
}
async fn container_probe(client: &Client) -> anyhow::Result<()> {
    use voxrig::client::{ValueSource, container::ScreenObservation};
    let ready = client.player_state().await?;
    let initial_sequence = ready.receive_sequence;
    emit("container_ready", ready)?;
    let mut commands = BufReader::new(tokio::io::stdin()).lines();
    let mut opening = None;
    let mut old_table_opening = None;
    let mut old_recipe: Option<ReceivedRecipe> = None;
    let mut content_sequence = 0;
    while let Some(command) = commands.next_line().await? {
        match command.as_str() {
            "recipe_catalogue" => {
                let catalogue = received_recipe_fixture(client, true).await?;
                let stick =
                    find_stick_recipe(&catalogue, client).context("native stick recipe missing")?;
                anyhow::ensure!(
                    stick.requirements().is_some_and(|r| r.len() == 2),
                    "two distinct crafting requirements missing"
                );
                let cake = client.registry().item("minecraft:cake")?.id;
                anyhow::ensure!(
                    catalogue.entries().iter().any(|r| matches!(
                        r.display(),
                        RecipeDisplay::Shaped {
                            width: 3,
                            height: 3,
                            ..
                        }
                    ) && r.unlocked() == Some(true)
                        && r.output_items(&catalogue)
                            .is_ok_and(|ids| ids.contains(&cake))),
                    "native cake arrangement missing"
                );
                old_recipe = Some(stick);
                emit(&command, &catalogue)?;
            }
            "recipe_materials" | "recipe_materials_named" | "recipe_materials_restored" => {
                let recipe = old_recipe.as_ref().context("native stick recipe missing")?;
                let named = command == "recipe_materials_named";
                let single = tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        let result = client.recipe_book_materials(recipe.id(), 1, 64).await?;
                        if result.stocks().iter().any(|(index, stock)| {
                            *index == 9
                                && stock.custom_named() == named
                                && stock.count() == if named { 0 } else { 3 }
                        }) {
                            return Ok::<_, anyhow::Error>(result);
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await??;
                let planks = client.registry().item("minecraft:oak_planks")?.id;
                if named {
                    anyhow::ensure!(
                        single.maximum() == 0 && single.assignment().is_none(),
                        "named stock counted as recipe-book material"
                    );
                } else {
                    anyhow::ensure!(
                        single.maximum() == 1
                            && single.assignment() == Some([planks, planks].as_slice()),
                        "three planks do not support one native stick batch"
                    );
                }
                let double = client.recipe_book_materials(recipe.id(), 2, 64).await?;
                anyhow::ensure!(
                    double.assignment().is_none(),
                    "three planks permit two stick batches"
                );
                let context = client
                    .received_crafting_context()
                    .await?
                    .context("coherent player crafting context missing")?;
                let layout = context.recipe_layout(recipe.id())?;
                let placement_next = context.recipe_placement_plan(
                    recipe.id(),
                    voxrig::client::RecipePlacementAmount::Next,
                )?;
                let placement_maximum = context.recipe_placement_plan(
                    recipe.id(),
                    voxrig::client::RecipePlacementAmount::Maximum,
                )?;
                anyhow::ensure!(
                    placement_next.material_maximum() == if named { 0 } else { 1 }
                        && placement_maximum.material_maximum() == if named { 0 } else { 1 }
                        && placement_next.can_place() == !named
                        && placement_maximum.can_place() == !named,
                    "coherent recipe placement preflight differs from actual native fixture"
                );
                anyhow::ensure!(
                    layout.grid_dimensions() == [2, 2]
                        && layout
                            .cells()
                            .iter()
                            .map(|c| c.coordinate())
                            .collect::<Vec<_>>()
                            == [[0, 0], [0, 1]]
                        && context.grid().receive_sequence()
                            == context.recipes().receive_sequence()
                        && context.grid().receive_sequence()
                            == context.inventory().receive_sequence(),
                    "player crafting context/layout differs"
                );
                emit(
                    &command,
                    serde_json::json!({"single":single,"double":double,"context_sequence":context.receive_sequence(),"recipe_layout":layout,"placement_next":placement_next,"placement_maximum":placement_maximum}),
                )?;
            }
            "recipe_removed" => {
                let catalogue = received_recipe_fixture(client, false).await?;
                let old = old_recipe.as_ref().context("original recipe missing")?;
                if client.version() == voxrig::MinecraftVersion::Java1_16_1 {
                    anyhow::ensure!(
                        catalogue.entry(old.id())?.unlocked() == Some(false),
                        "legacy book revoke lost declaration"
                    );
                } else {
                    anyhow::ensure!(
                        catalogue.entry(old.id()).is_err(),
                        "modern removed recipe still present"
                    );
                }
                emit(&command, &catalogue)?;
            }
            "recipe_readded" => {
                let catalogue = received_recipe_fixture(client, true).await?;
                let old = old_recipe.as_ref().context("original recipe missing")?;
                let new =
                    find_stick_recipe(&catalogue, client).context("readded recipe missing")?;
                if client.version() == voxrig::MinecraftVersion::Java1_16_1 {
                    anyhow::ensure!(
                        new.id() == old.id(),
                        "unchanged legacy declaration replaced"
                    );
                } else {
                    anyhow::ensure!(
                        new.id() != old.id() && catalogue.entry(old.id()).is_err(),
                        "reused native modern ID accepted as old entry"
                    );
                }
                emit(&command, &catalogue)?;
            }

            "cursor_close_audit_open_survival" | "cursor_close_audit_open_creative" => {
                let mode = if command.ends_with("survival") {
                    GameMode::Survival
                } else {
                    GameMode::Creative
                };
                wait_player(client, |p| {
                    p.game_mode == Some(mode)
                        && p.received_pose.as_ref().is_some_and(|pose| {
                            pose.receive_sequence > initial_sequence
                                && pose.position == [0.5, 65., 0.5]
                                && pose.rotation == [0., 35.]
                        })
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
            "cursor_close_audit_holding" | "cursor_return_holding" => {
                let record = wait_pickup(client).await?;
                anyhow::ensure!(
                    record.source_receipt.as_ref().unwrap().value == SlotKnowledge::Empty
                        && stack_count(&record.cursor_receipt.as_ref().unwrap().value) > 0,
                    "native audit must hold received nonempty cursor"
                );
                emit(&command, record)?;
            }
            "cursor_return_close" => {
                let held = wait_pickup(client).await?;
                let before = client.received_inventory().await?;
                let original = before
                    .cursor()
                    .and_then(|s| s.item())
                    .context("actual held item missing")?;
                let InventorySource::Container { screen } = held.source else {
                    anyhow::bail!("cursor return requires original storage opening")
                };
                let close = if held.mode == GameMode::Survival {
                    client.survival().close_container(screen).await?
                } else {
                    client.creative().close_container(screen).await?
                };
                anyhow::ensure!(
                    close.dispatched
                        && close.requires_inspection.is_none()
                        && !close.return_steps.is_empty(),
                    "return and close did not complete"
                );
                for step in &close.return_steps {
                    anyhow::ensure!(
                        step.stage
                            == voxrig::client::inventory::InventoryClickStage::ObservedClicked
                            && step.id.close() == Some(close.id),
                        "return child not actual/completed/owned"
                    );
                    for receipt in [&step.source_receipt, &step.cursor_receipt] {
                        anyhow::ensure!(
                            matches!(receipt, Some(voxrig::client::ObservedValue {
                            source: ValueSource::Received { sequence }, ..
                        }) if *sequence > step.send.after_sequence),
                            "return child lacks fresh actual receipt"
                        );
                    }
                }
                anyhow::ensure!(
                    client
                        .player_state()
                        .await?
                        .inventory
                        .cursor
                        .as_ref()
                        .is_some_and(|c| c.value == SlotKnowledge::Empty),
                    "actual cursor did not empty"
                );
                let after = client.received_inventory().await?;
                for step in &close.return_steps {
                    let mapping = step
                        .initial_screen
                        .as_ref()
                        .and_then(|s| s.layout.as_ref())
                        .and_then(|l| {
                            l.player_slots
                                .iter()
                                .find(|m| m.screen_slot == usize::from(step.source_slot))
                        })
                        .context("native cursor return mapping missing")?;
                    let actual = after
                        .slot(mapping.player_slot)?
                        .and_then(|s| s.item())
                        .context("native returned item missing")?;
                    anyhow::ensure!(
                        original.native_data_equivalent(&actual)?,
                        "cursor return changed native data fields"
                    );
                }
                let mut diagnostic = serde_json::to_value(close)?;
                diagnostic["native_data_equivalent"] = serde_json::json!(true);
                emit(&command, diagnostic)?;
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
            "table_open_survival"
            | "table_open_creative"
            | "table_reopen_survival"
            | "table_reopen_creative" => {
                let mode = if command.ends_with("survival") {
                    GameMode::Survival
                } else {
                    GameMode::Creative
                };
                wait_player(client, |p| {
                    p.game_mode == Some(mode)
                        && p.received_pose.as_ref().is_some_and(|r| {
                            r.position == [0.5, 65., 0.5] && r.rotation == [0., 35.]
                        })
                })
                .await?;
                let target = tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        let value = match mode {
                            GameMode::Survival => client.survival().target_block(4.5).await,
                            _ => client.creative().target_block(4.5).await,
                        };
                        if let Ok(value) = value {
                            if value.hit.as_ref().is_some_and(|h| {
                                h.position == [0, 65, 2]
                                    && h.state.name == "minecraft:crafting_table"
                            }) {
                                return Ok::<_, anyhow::Error>(value);
                            }
                        }
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                })
                .await??;
                anyhow::ensure!(
                    target
                        .hit
                        .context("table target missing")?
                        .state
                        .properties
                        .is_empty(),
                    "table properties differ"
                );
                let sent = match mode {
                    GameMode::Survival => client.survival().open_container([0, 65, 2]).await?,
                    _ => client.creative().open_container([0, 65, 2]).await?,
                };
                anyhow::ensure!(
                    sent.send.dispatched && sent.expected_menu == "minecraft:crafting",
                    "table intent differs"
                );
                emit(&command, sent)?;
            }
            "table_observed_survival" | "table_observed_creative" => {
                let record = wait_container_open(client).await?;
                let grid = client
                    .received_crafting()
                    .await?
                    .context("observed table grid missing")?;
                let id = record
                    .observed_screen
                    .as_ref()
                    .context("observed table screen missing")?
                    .id;
                anyhow::ensure!(
                    grid.dimensions() == [3, 3]
                        && grid.source() == CraftingSource::Table { screen: id },
                    "table grid/opening mismatch"
                );
                opening = Some(id);
                emit(&command, serde_json::json!({"record":record,"grid":grid}))?;
            }
            "result_sticks_survival"
            | "result_sticks_creative"
            | "result_cake_survival"
            | "result_cake_creative" => {
                let mode = if command.ends_with("survival") {
                    GameMode::Survival
                } else {
                    GameMode::Creative
                };
                emit(
                    &command,
                    crafting_result_probe(client, mode, command.starts_with("result_cake")).await?,
                )?;
            }
            "table_fill_survival" | "table_fill_creative" => {
                let mode = if command.ends_with("survival") {
                    GameMode::Survival
                } else {
                    GameMode::Creative
                };
                emit(&command, crafting_table_fill_probe(client, mode).await?)?;
            }
            "table_close_survival"
            | "table_close_creative"
            | "table_close_empty_survival"
            | "table_close_empty_creative" => {
                let mode = if command.ends_with("survival") {
                    GameMode::Survival
                } else {
                    GameMode::Creative
                };
                let id = opening.context("table opening missing")?;
                let before = client
                    .received_crafting()
                    .await?
                    .context("close table grid missing")?;
                let record = match mode {
                    GameMode::Survival => client.survival().close_container(id).await?,
                    _ => client.creative().close_container(id).await?,
                };
                anyhow::ensure!(record.dispatched, "table close incomplete");
                if command.starts_with("table_close_empty") {
                    anyhow::ensure!(
                        record.return_steps.is_empty(),
                        "empty table close unexpectedly returns cursor"
                    );
                } else {
                    anyhow::ensure!(
                        record.return_steps.len() == 1,
                        "table close cursor return differs"
                    );
                    old_table_opening = Some(id);
                }
                emit(
                    &command,
                    serde_json::json!({"record":record,"grid_before":before}),
                )?;
            }
            "table_after_close_survival" | "table_after_close_creative" => {
                let close = client
                    .survival()
                    .container_close_record()
                    .await?
                    .context("table close history missing")?;
                let after = close
                    .return_steps
                    .last()
                    .and_then(|r| r.source_receipt.as_ref())
                    .context("table close return receipt missing")?;
                let ValueSource::Received { sequence: after } = after.source else {
                    anyhow::bail!("table close return is not received")
                };
                let inventory = tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        let value = client.received_inventory().await?;
                        if value.slot(9)?.is_some_and(|r| {
                            r.receive_sequence() > after
                                && r.item().is_some_and(|i| {
                                    i.stack().name == "minecraft:oak_planks" && i.stack().count == 3
                                })
                        }) && value
                            .cursor()
                            .is_some_and(|r| *r.value() == SlotKnowledge::Empty)
                        {
                            return Ok::<_, anyhow::Error>(value);
                        }
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                })
                .await??;
                let grid = client
                    .received_crafting()
                    .await?
                    .context("player grid after table close missing")?;
                anyhow::ensure!(
                    grid.dimensions() == [2, 2],
                    "closed table still admitted as active crafting table"
                );
                emit(
                    &command,
                    serde_json::json!({"inventory":inventory,"grid":grid,"close":close}),
                )?;
            }
            "table_stale_refusal_survival" | "table_stale_refusal_creative" => {
                let mode = if command.ends_with("survival") {
                    GameMode::Survival
                } else {
                    GameMode::Creative
                };
                let old = old_table_opening.context("old table opening missing")?;
                let current = opening.context("new table opening missing")?;
                anyhow::ensure!(old != current, "table opening identity reused");
                let click = match mode {
                    GameMode::Survival => {
                        client
                            .survival()
                            .click_inventory(
                                InventorySource::Container { screen: old },
                                9,
                                InventoryClickButton::Left,
                            )
                            .await
                    }
                    _ => {
                        client
                            .creative()
                            .click_inventory(
                                InventorySource::Container { screen: old },
                                9,
                                InventoryClickButton::Left,
                            )
                            .await
                    }
                };
                let close = match mode {
                    GameMode::Survival => client.survival().close_container(old).await,
                    _ => client.creative().close_container(old).await,
                };
                anyhow::ensure!(
                    click.is_err() && close.is_err(),
                    "stale table operation admitted"
                );
                let grid = client
                    .received_crafting()
                    .await?
                    .context("new table lost after stale request")?;
                anyhow::ensure!(
                    grid.source() == CraftingSource::Table { screen: current },
                    "stale request changed current table"
                );
                emit(
                    &command,
                    serde_json::json!({"old":old,"current":current,"click_error":click.unwrap_err().to_string(),"close_error":close.unwrap_err().to_string(),"grid":grid}),
                )?;
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

            "crafting_input_survival" | "crafting_input_creative" => {
                let mode = if command.ends_with("survival") {
                    GameMode::Survival
                } else {
                    GameMode::Creative
                };
                emit(&command, crafting_input_probe(client, mode).await?)?;
            }
            "held_cursor_take_survival"
            | "held_cursor_take_creative"
            | "held_cursor_restore_survival"
            | "held_cursor_restore_creative" => {
                let restoring = command.starts_with("held_cursor_restore");
                let original = client.received_inventory().await?;
                let original_item = if restoring {
                    original.cursor().and_then(|s| s.item())
                } else {
                    original.slot(9)?.and_then(|s| s.item())
                }
                .context("held item missing")?;
                anyhow::ensure!(
                    original_item.stack().name == "minecraft:diamond_helmet"
                        && original_item.stack().count == 1,
                    "held fixture differs"
                );
                let source = if restoring { 10 } else { 9 };
                let sent = if command.ends_with("survival") {
                    client
                        .survival()
                        .click_inventory(
                            InventoryClickSource::Player,
                            source,
                            InventoryClickButton::Left,
                        )
                        .await?
                } else {
                    client
                        .creative()
                        .click_inventory(
                            InventoryClickSource::Player,
                            source,
                            InventoryClickButton::Left,
                        )
                        .await?
                };
                let complete = tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        let r = client
                            .survival()
                            .inventory_click_record()
                            .await?
                            .context("held click missing")?;
                        anyhow::ensure!(
                            r.id == sent.id && r.requires_inspection.is_none(),
                            "held click interrupted: {:?}",
                            r.requires_inspection
                        );
                        if r.stage == InventoryClickStage::ObservedClicked {
                            return Ok::<_, anyhow::Error>(r);
                        }
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                })
                .await??;
                let actual = client.received_inventory().await?;
                let actual_source = actual
                    .slot(source.into())?
                    .context("held click source missing")?;
                let actual_cursor = actual.cursor().context("held click cursor missing")?;
                for (slot, receipt) in [
                    (&actual_source, complete.source_receipt.as_ref()),
                    (&actual_cursor, complete.cursor_receipt.as_ref()),
                ] {
                    anyhow::ensure!(
                        slot.receive_sequence() > complete.send.after_sequence
                            && receipt.is_some_and(|r| &r.value == slot.value()),
                        "held click lacks actual fresh outcomes"
                    );
                }
                let destination = if restoring {
                    &actual_source
                } else {
                    &actual_cursor
                };
                let emptied = if restoring {
                    &actual_cursor
                } else {
                    &actual_source
                };
                anyhow::ensure!(
                    emptied.value() == &SlotKnowledge::Empty
                        && original_item.native_equivalent(
                            &destination.item().context("held destination missing")?
                        )?,
                    "held click changed item or left predecessor"
                );
                emit(
                    &command,
                    serde_json::json!({"record":complete,"native_equivalent":true,"holding":!restoring}),
                )?;
            }
            "held_cursor_transfer_survival" | "held_cursor_transfer_creative" => {
                let original = client.received_inventory().await?;
                let held = original
                    .cursor()
                    .and_then(|s| s.item())
                    .context("held transfer cursor missing")?;
                let source = original
                    .slot(9)?
                    .and_then(|s| s.item())
                    .context("held transfer source missing")?;
                anyhow::ensure!(
                    source.stack().name == "minecraft:dirt" && source.stack().count == 7,
                    "held transfer source differs"
                );
                let destination = (36..45)
                    .find(|&i| {
                        original
                            .slot(i)
                            .ok()
                            .flatten()
                            .is_some_and(|s| s.value() == &SlotKnowledge::Empty)
                    })
                    .context("held transfer destination unavailable")?;
                let sent = if command.ends_with("survival") {
                    client
                        .survival()
                        .transfer_inventory(InventorySource::Player, 9)
                        .await?
                } else {
                    client
                        .creative()
                        .transfer_inventory(InventorySource::Player, 9)
                        .await?
                };
                let complete = tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        let r = client
                            .survival()
                            .inventory_transfer_record()
                            .await?
                            .context("held transfer missing")?;
                        anyhow::ensure!(
                            r.id == sent.id && r.requires_inspection.is_none(),
                            "held transfer interrupted: {:?}",
                            r.requires_inspection
                        );
                        if r.stage == InventoryTransferStage::ObservedTransferred {
                            return Ok::<_, anyhow::Error>(r);
                        }
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                })
                .await??;
                let actual = client.received_inventory().await?;
                for index in [9, destination] {
                    let slot = actual
                        .slot(index)?
                        .context("held transfer changed slot missing")?;
                    let change = complete
                        .changed_slots
                        .iter()
                        .find(|c| c.player_slot == Some(index))
                        .context("held transfer planned change missing")?;
                    anyhow::ensure!(
                        slot.receive_sequence() > complete.send.after_sequence
                            && change
                                .receipt
                                .as_ref()
                                .is_some_and(|r| &r.value == slot.value()),
                        "held transfer lacks fresh actual slot"
                    );
                    if index == 9 {
                        anyhow::ensure!(
                            slot.value() == &SlotKnowledge::Empty,
                            "held source not empty"
                        );
                    } else {
                        anyhow::ensure!(
                            source.native_equivalent(&slot.item().context("held dirt missing")?)?,
                            "held transfer changed source item"
                        );
                    }
                }
                let cursor = actual
                    .cursor()
                    .context("held transfer cursor inspection missing")?;
                anyhow::ensure!(
                    held.native_equivalent(&cursor.item().context("held transfer lost cursor")?)?
                        && complete
                            .cursor_inspected
                            .as_ref()
                            .is_some_and(|r| &r.value == cursor.value()),
                    "held transfer changed carried item"
                );
                if complete.initial.session.version == MinecraftVersion::Java1_21_11 {
                    anyhow::ensure!(
                        complete.send.request_full_resync
                            && complete.send.sent_screen_revision != complete.send.screen_revision,
                        "data cursor did not request actual full resync"
                    );
                }
                emit(
                    &command,
                    serde_json::json!({"record":complete,"destination":destination,"cursor_native_equivalent":true,"source_native_equivalent":true}),
                )?;
            }
            "item_data_pickup_survival" | "item_data_pickup_creative" => {
                let original = client.received_inventory().await?;
                let original_item = original
                    .slot(9)?
                    .and_then(|s| s.item())
                    .context("data click source missing")?;
                let total = original_item.stack().count;
                let mut records = Vec::new();
                for (button, source_count, cursor_count) in [
                    (InventoryClickButton::Right, total / 2, total.div_ceil(2)),
                    (
                        InventoryClickButton::Right,
                        total / 2 + 1,
                        total.div_ceil(2) - 1,
                    ),
                    (InventoryClickButton::Left, total, 0),
                ] {
                    let record = if command.ends_with("survival") {
                        client
                            .survival()
                            .click_inventory(InventoryClickSource::Player, 9, button)
                            .await?
                    } else {
                        client
                            .creative()
                            .click_inventory(InventoryClickSource::Player, 9, button)
                            .await?
                    };
                    let completed = tokio::time::timeout(Duration::from_secs(15), async {
                        loop {
                            let current = client
                                .survival()
                                .inventory_click_record()
                                .await?
                                .context("data click record missing")?;
                            anyhow::ensure!(
                                current.id == record.id && current.requires_inspection.is_none(),
                                "data click interrupted: {:?}",
                                current.requires_inspection
                            );
                            if current.stage == InventoryClickStage::ObservedClicked {
                                return Ok::<_, anyhow::Error>(current);
                            }
                            tokio::time::sleep(Duration::from_millis(25)).await;
                        }
                    })
                    .await??;
                    let actual = client.received_inventory().await?;
                    for (slot, expected_count, diagnostic) in [
                        (
                            actual.slot(9)?.context("data source receipt missing")?,
                            source_count,
                            completed.source_receipt.as_ref(),
                        ),
                        (
                            actual.cursor().context("data cursor receipt missing")?,
                            cursor_count,
                            completed.cursor_receipt.as_ref(),
                        ),
                    ] {
                        anyhow::ensure!(
                            slot.receive_sequence() > completed.send.after_sequence
                                && diagnostic.is_some_and(|v| &v.value == slot.value()),
                            "data click outcome lacks fresh actual receipt"
                        );
                        if expected_count == 0 {
                            anyhow::ensure!(
                                slot.value() == &SlotKnowledge::Empty,
                                "data cursor did not empty"
                            );
                        } else {
                            let item = slot.item().context("data item missing")?;
                            anyhow::ensure!(
                                item.stack().count == expected_count
                                    && original_item.native_data_equivalent(&item)?,
                                "data click changed fields/count"
                            );
                        }
                    }
                    records.push(completed);
                }
                emit(
                    &command,
                    serde_json::json!({"records":records,"native_data_equivalent":true,"restored_count":total}),
                )?;
            }
            "armor_fixture_state" => {
                emit(&command, client.player_state().await?)?;
            }
            "item_armor_refuse" => {
                let before = client.survival().inventory_transfer_record().await?;
                let error = client
                    .survival()
                    .transfer_inventory(InventorySource::Player, 5)
                    .await
                    .unwrap_err();
                anyhow::ensure!(
                    error.kind() == voxrig::ErrorKind::InvalidInput
                        && error.to_string().contains("no effect"),
                    "armor refusal was not native no-effect: {error}"
                );
                let after = client.survival().inventory_transfer_record().await?;
                anyhow::ensure!(
                    before.as_ref().map(|r| r.id) == after.as_ref().map(|r| r.id),
                    "armor refusal created an intent"
                );
                emit(
                    &command,
                    serde_json::json!({"refused_before_send":true,"error":error.to_string()}),
                )?;
            }
            "item_data_transfer_survival"
            | "item_data_transfer_creative"
            | "item_equipment_transfer_survival"
            | "item_equipment_transfer_creative"
            | "item_armor_transfer_survival"
            | "item_armor_transfer_creative"
            | "item_data_evacuate_survival"
            | "item_data_evacuate_creative" => {
                let original = client.received_inventory().await?;
                let original_item = original
                    .slot(if command.starts_with("item_armor_transfer") {
                        5
                    } else {
                        9
                    })?
                    .and_then(|s| s.item())
                    .context("data transfer source missing")?;
                let total = original_item.stack().count;
                let mut records = Vec::new();
                let steps = if command.starts_with("item_data_evacuate") {
                    vec![(9, vec![(9, 0), (36, total)])]
                } else if command.starts_with("item_armor_transfer") {
                    anyhow::ensure!(total == 1, "armor fixture count differs");
                    vec![(5, vec![(5, 0), (9, 1)])]
                } else if command.starts_with("item_equipment_transfer") {
                    anyhow::ensure!(total == 3, "equipment fixture count differs");
                    vec![(9, vec![(9, 0), (5, 1), (36, 2)])]
                } else {
                    vec![
                        (9, vec![(9, 0), (36, total)]),
                        (36, vec![(9, total), (36, 0)]),
                    ]
                };
                for (source, expected) in steps {
                    let intent = if command.ends_with("survival") {
                        client
                            .survival()
                            .transfer_inventory(InventorySource::Player, source)
                            .await?
                    } else {
                        client
                            .creative()
                            .transfer_inventory(InventorySource::Player, source)
                            .await?
                    };
                    let completed = tokio::time::timeout(Duration::from_secs(15), async {
                        loop {
                            let current = client
                                .survival()
                                .inventory_transfer_record()
                                .await?
                                .context("data transfer record missing")?;
                            anyhow::ensure!(
                                current.id == intent.id && current.requires_inspection.is_none(),
                                "data transfer interrupted: {:?}",
                                current.requires_inspection
                            );
                            if current.stage == InventoryTransferStage::ObservedTransferred {
                                return Ok::<_, anyhow::Error>(current);
                            }
                            tokio::time::sleep(Duration::from_millis(25)).await;
                        }
                    })
                    .await??;
                    let actual = client.received_inventory().await?;
                    for (index, expected_count) in expected {
                        let slot = actual
                            .slot(index)?
                            .context("data transfer actual slot missing")?;
                        let change = completed
                            .changed_slots
                            .iter()
                            .find(|c| c.player_slot == Some(index))
                            .context("data transfer change missing")?;
                        anyhow::ensure!(
                            slot.receive_sequence() > completed.send.after_sequence
                                && change
                                    .receipt
                                    .as_ref()
                                    .is_some_and(|r| &r.value == slot.value()),
                            "data transfer lacks fresh actual outcome"
                        );
                        if expected_count == 0 {
                            anyhow::ensure!(
                                slot.value() == &SlotKnowledge::Empty,
                                "data transfer source did not empty"
                            );
                        } else {
                            let item = slot.item().context("data transfer item missing")?;
                            anyhow::ensure!(
                                item.stack().count == expected_count
                                    && original_item.native_data_equivalent(&item)?,
                                "data transfer changed fields/count"
                            );
                        }
                    }
                    anyhow::ensure!(
                        actual
                            .cursor()
                            .is_some_and(|s| s.value() == &SlotKnowledge::Empty),
                        "QUICK_MOVE changed empty cursor"
                    );
                    records.push(completed);
                }
                emit(
                    &command,
                    serde_json::json!({"records":records,"native_data_equivalent":true,"restored_count":total}),
                )?;
            }
            "item_data_swap_survival"
            | "item_data_swap_creative"
            | "item_data_return_survival"
            | "item_data_return_creative" => {
                let before = client.received_inventory().await?;
                let returning = command.starts_with("item_data_return");
                let record = if command.ends_with("survival") {
                    client.survival().swap_hotbar(9, 0).await?
                } else {
                    client.creative().swap_hotbar(9, 0).await?
                };
                anyhow::ensure!(record.send.dispatched, "data swap did not dispatch");
                let complete = wait_swap(client).await?;
                let after = client.received_inventory().await?;
                let expected = before
                    .slot(if returning { 36 } else { 9 })?
                    .and_then(|s| s.item())
                    .context("data predecessor missing")?;
                let actual = after
                    .slot(if returning { 9 } else { 36 })?
                    .and_then(|s| s.item())
                    .context("data destination missing")?;
                anyhow::ensure!(
                    expected.native_equivalent(&actual)?,
                    "data swap changed native item fields"
                );
                anyhow::ensure!(
                    after
                        .slot(if returning { 36 } else { 9 })?
                        .is_some_and(|s| s.value() == &SlotKnowledge::Empty),
                    "source was not emptied"
                );
                emit(
                    &command,
                    serde_json::json!({"record":complete,"native_item_equivalent":true,"registry_stamp":after.registry_state().stamp()}),
                )?;
            }
            "registry_state" => {
                let player = client.player_state().await?;
                let slot = player.inventory.slots[9]
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("custom-data slot unavailable"))?;
                let SlotKnowledge::Item { item } = &slot.value else {
                    anyhow::bail!("custom-data fixture item missing")
                };
                let custom_data = item
                    .custom_data()?
                    .ok_or_else(|| anyhow::anyhow!("custom-data fixture value missing"))?;
                let item_properties = item.properties()?;
                anyhow::ensure!(
                    custom_data
                        .root()
                        .get("VoxrigProbe")
                        .and_then(NbtValue::as_int)
                        .is_some(),
                    "common marker missing"
                );
                let registries = client.server_registry_state().await?;
                let unbreaking = if client.version() == MinecraftVersion::Java1_21_11 {
                    let id = registries.find("minecraft:enchantment", "minecraft:unbreaking")?;
                    let entry = registries.resolve(&id)?;
                    Some(serde_json::json!({"id": id, "entry": entry}))
                } else {
                    None
                };
                emit(
                    &command,
                    serde_json::json!({"received": registries, "unbreaking": unbreaking,"custom_data":custom_data,"custom_data_source":slot.source,"custom_data_session":player.session,"custom_data_item":slot.value,"item_properties":item_properties}),
                )?;
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
// A1: each mode runs on one connection, with no fixture mutation between steps.
// Version-specific protocol assertions live in adapter tests, not this consumer.
async fn workflow_look(client: &Client, mode: GameMode, target: [f64; 3]) -> anyhow::Result<()> {
    let p = client
        .player_state()
        .await?
        .position
        .context("workflow position missing")?
        .value;
    let dx = target[0] - p[0];
    let dz = target[2] - p[2];
    let rotation = [
        (-dx.atan2(dz)).to_degrees() as f32,
        ((p[1] + 1.62 - target[1]).atan2(dx.hypot(dz))).to_degrees() as f32,
    ];
    match mode {
        GameMode::Survival => {
            client.survival().look(rotation).await?;
        }
        GameMode::Creative => {
            client.creative().look(rotation).await?;
        }
        _ => anyhow::bail!("unsupported workflow mode"),
    }
    Ok(())
}
async fn basic_workflow_probe(client: &Client) -> anyhow::Result<()> {
    let ready = client.player_state().await?;
    let session = ready.session;
    emit("workflow_ready", &ready)?;
    let mut commands = BufReader::new(tokio::io::stdin()).lines();
    let mut mode = None;
    while let Some(command) = commands.next_line().await? {
        match command.as_str() {
            "workflow_baseline" => {
                let player = wait_player(client, |p| {
                    matches!(p.game_mode, Some(GameMode::Survival | GameMode::Creative))
                        && p.received_pose.as_ref().is_some_and(|pose| pose.receive_sequence > ready.receive_sequence && pose.position == [0.5,65.0,0.5])
                        && matches!(p.inventory.slots[9].as_ref().map(|v| &v.value), Some(SlotKnowledge::Item {item}) if item.name == "minecraft:oak_planks" && item.count == 2)
                        && matches!(p.inventory.slots[11].as_ref().map(|v| &v.value), Some(SlotKnowledge::Item {item}) if item.name == "minecraft:stone" && item.count == 2)
                        && matches!(p.inventory.slots[37].as_ref().map(|v| &v.value), Some(SlotKnowledge::Item {item}) if item.name == "minecraft:dirt" && item.count == 3)
                        && p.inventory.cursor.as_ref().is_some_and(|c| c.value == SlotKnowledge::Empty)
                }).await?;
                mode = player.game_mode;
                match mode.context("workflow mode absent")? {
                    GameMode::Survival => {
                        client.survival().select_hotbar(0).await?;
                    }
                    _ => {
                        client.creative().select_hotbar(0).await?;
                    }
                }
                wait_block(client, [0, 65, 2], "minecraft:chest").await?;
                emit(
                    &command,
                    client
                        .capture(Region {
                            min: [-2, 64, -2],
                            max: [3, 67, 3],
                        })
                        .await?,
                )?;
            }
            "workflow_move" => {
                let mode = mode.context("workflow baseline missing")?;
                let controls: Vec<_> = (0..30)
                    .map(|tick| SurvivalControl {
                        yaw: -90.0,
                        input: SurvivalInput {
                            forward: i8::from(tick < 3),
                            ..Default::default()
                        },
                    })
                    .collect();
                tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        let preview = match mode {
                            GameMode::Survival => client.survival().preview_path(&controls).await,
                            _ => client.creative().preview_path(&controls).await,
                        };
                        if preview.is_ok() {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                })
                .await?;
                let started = match mode {
                    GameMode::Survival => client.survival().start_predicted_path(&controls).await?,
                    _ => client.creative().start_predicted_path(&controls).await?,
                };
                let completed = tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        let record = client
                            .survival()
                            .motion_record()
                            .await?
                            .context("workflow motion missing")?;
                        anyhow::ensure!(record.run_id == started.run_id, "motion owner changed");
                        match record.status {
                            MotionStatus::Predicted => return Ok::<_, anyhow::Error>(record),
                            MotionStatus::Running => {
                                tokio::time::sleep(Duration::from_millis(25)).await
                            }
                            _ => anyhow::bail!("workflow motion interrupted: {:?}", record.problem),
                        }
                    }
                })
                .await??;
                emit("workflow_motion_record", completed)?;
                emit(&command, client.player_state().await?)?;
            }
            "workflow_storage" => {
                let mode = mode.context("workflow baseline missing")?;
                workflow_look(client, mode, [0.5, 65.5, 2.5]).await?;
                // Read-only readiness polling, followed by exactly one activation.
                tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        let target = match mode {
                            GameMode::Survival => client.survival().target_block(4.5).await,
                            _ => client.creative().target_block(4.5).await,
                        };
                        if target.is_ok_and(|t| t.hit.is_some_and(|h| h.position == [0, 65, 2])) {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                })
                .await?;
                let submitted = match mode {
                    GameMode::Survival => client.survival().open_container([0, 65, 2]).await?,
                    _ => client.creative().open_container([0, 65, 2]).await?,
                };
                let opened = wait_container_open(client).await?;
                anyhow::ensure!(opened.id == submitted.id, "opening owner changed");
                let screen = opened
                    .observed_screen
                    .as_ref()
                    .context("workflow screen missing")?;
                let slot = screen
                    .layout
                    .as_ref()
                    .context("workflow layout missing")?
                    .player_slots
                    .iter()
                    .find(|mapping| mapping.player_slot == 11)
                    .context("workflow player mapping missing")?
                    .screen_slot as u16;
                let source = InventorySource::Container { screen: screen.id };
                let sent = match mode {
                    GameMode::Survival => {
                        client.survival().transfer_inventory(source, slot).await?
                    }
                    _ => client.creative().transfer_inventory(source, slot).await?,
                };
                let transfer = wait_transfer(client).await?;
                anyhow::ensure!(transfer.id == sent.id, "transfer owner changed");
                let close = match mode {
                    GameMode::Survival => client.survival().close_container(screen.id).await?,
                    _ => client.creative().close_container(screen.id).await?,
                };
                anyhow::ensure!(close.dispatched, "workflow close not dispatched");
                emit(
                    &command,
                    serde_json::json!({"open":opened,"transfer":transfer,"close":close,"player":client.player_state().await?}),
                )?;
            }
            "workflow_craft" => {
                let mode = mode.context("workflow baseline missing")?;
                let grid = client
                    .received_crafting()
                    .await?
                    .context("workflow crafting grid missing")?;
                anyhow::ensure!(
                    grid.dimensions() == [2, 2]
                        && matches!(grid.source(), CraftingSource::Player { .. }),
                    "workflow player grid missing after close"
                );
                let mut clicks = vec![
                    crafting_pickup(
                        client,
                        mode,
                        InventorySource::Player,
                        9,
                        InventoryClickButton::Left,
                    )
                    .await?,
                ];
                for y in 0..2 {
                    let (source, slot) = grid.input_source(0, y)?;
                    clicks.push(
                        crafting_pickup(client, mode, source, slot, InventoryClickButton::Right)
                            .await?,
                    );
                }
                let take = crafting_result_take(client, mode, "minecraft:stick").await?;
                let after = take
                    .after
                    .as_ref()
                    .context("workflow result grid missing")?;
                for y in 0..2 {
                    for x in 0..2 {
                        anyhow::ensure!(
                            after
                                .input(x, y)?
                                .is_some_and(|r| *r.value() == SlotKnowledge::Empty),
                            "workflow ingredients remain"
                        );
                    }
                }
                anyhow::ensure!(
                    after
                        .result()
                        .is_some_and(|r| *r.value() == SlotKnowledge::Empty),
                    "workflow result not depleted"
                );
                clicks.push(
                    crafting_pickup(
                        client,
                        mode,
                        InventorySource::Player,
                        10,
                        InventoryClickButton::Left,
                    )
                    .await?,
                );
                emit(
                    &command,
                    serde_json::json!({"take":take,"clicks":clicks,"player":client.player_state().await?}),
                )?;
            }
            "workflow_place" => {
                let mode = mode.context("workflow baseline missing")?;
                match mode {
                    GameMode::Survival => {
                        client.survival().select_hotbar(1).await?;
                    }
                    _ => {
                        client.creative().select_hotbar(1).await?;
                    }
                }
                workflow_look(client, mode, [2.5, 65.0, 0.5]).await?;
                if mode == GameMode::Survival {
                    let sent = client
                        .survival()
                        .place_cube([2, 64, 0], BlockFace::Up)
                        .await?;
                    let placed = tokio::time::timeout(Duration::from_secs(15), async {
                        loop {
                            let record = client
                                .survival()
                                .placement_record()
                                .await?
                                .context("workflow placement missing")?;
                            anyhow::ensure!(record.id == sent.id, "placement owner changed");
                            match record.stage {
                                PlacementStage::ObservedPlaced => {
                                    return Ok::<_, anyhow::Error>(record);
                                }
                                PlacementStage::RequiresInspection => anyhow::bail!(
                                    "workflow placement interrupted: {:?}",
                                    record.requires_inspection
                                ),
                                _ => tokio::time::sleep(Duration::from_millis(25)).await,
                            }
                        }
                    })
                    .await??;
                    emit("workflow_placement_record", placed)?;
                } else {
                    client
                        .creative()
                        .use_on_block([2, 64, 0], BlockFace::Up, [0.5, 1.0, 0.5])
                        .await?;
                }
                wait_block(client, [2, 65, 0], "minecraft:dirt").await?;
                let final_capture = client
                    .capture(Region {
                        min: [2, 65, 0],
                        max: [2, 65, 0],
                    })
                    .await?;
                anyhow::ensure!(
                    final_capture.player.session == session,
                    "workflow reconnected"
                );
                anyhow::ensure!(
                    final_capture
                        .player
                        .inventory
                        .cursor
                        .as_ref()
                        .is_some_and(|c| c.value == SlotKnowledge::Empty),
                    "workflow cursor not empty"
                );
                emit(&command, final_capture)?;
            }
            "workflow_disconnect" => {
                client.disconnect().await?;
                emit(
                    &command,
                    serde_json::json!({"session":session,"version":client.version()}),
                )?;
                return Ok(());
            }
            _ => anyhow::bail!("unknown workflow command"),
        }
    }
    anyhow::bail!("workflow controller ended without disconnect")
}

async fn equipment_entity_probe(client: &Client) -> anyhow::Result<()> {
    let ready = client.player_state().await?;
    emit("a2_ready", &ready)?;
    let mut commands = BufReader::new(tokio::io::stdin()).lines();
    let mut mode = None;
    let mut sheep = None;
    let mut villager = None;
    while let Some(command) = commands.next_line().await? {
        match command.as_str() {
            "a2_baseline" => {
                let player = wait_player(client, |p| {
                    matches!(p.game_mode, Some(GameMode::Survival | GameMode::Creative))
                        && p.received_pose.as_ref().is_some_and(|r| r.receive_sequence > ready.receive_sequence && r.position == [0.5,65.0,0.5])
                        && matches!(p.inventory.slots[38].as_ref().map(|s| &s.value), Some(SlotKnowledge::Item {item}) if item.name == "minecraft:iron_boots" && item.count == 1)
                        && p.inventory.slots[8].as_ref().is_some_and(|s| s.value == SlotKnowledge::Empty)
                        && p.inventory.slots[36].as_ref().is_some_and(|s| s.value == SlotKnowledge::Empty)
                        && p.inventory.cursor.as_ref().is_some_and(|s| s.value == SlotKnowledge::Empty)
                }).await?;
                mode = player.game_mode;
                match mode.context("A2 mode absent")? {
                    GameMode::Survival => {
                        client.survival().select_hotbar(0).await?;
                    }
                    _ => {
                        client.creative().select_hotbar(0).await?;
                    }
                }
                let spawns = tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        let spawns = client.entity_spawns().await?;
                        sheep = spawns
                            .entities
                            .iter()
                            .filter(|e| {
                                e.type_name.as_deref() == Some("minecraft:sheep")
                                    && e.spawn_position.value == [2.5, 65.0, 0.5]
                            })
                            .max_by_key(|e| e.id.spawn_sequence())
                            .map(|e| e.id);
                        villager = spawns
                            .entities
                            .iter()
                            .filter(|e| {
                                e.type_name.as_deref() == Some("minecraft:villager")
                                    && e.spawn_position.value == [0.5, 65.0, 2.5]
                            })
                            .max_by_key(|e| e.id.spawn_sequence())
                            .map(|e| e.id);
                        if sheep.is_some() && villager.is_some() {
                            break Ok::<_, anyhow::Error>(spawns);
                        }
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                })
                .await??;
                anyhow::ensure!(spawns.session == player.session, "A2 mixed world baseline");
                emit(
                    &command,
                    serde_json::json!({"player":player,"spawns":spawns}),
                )?;
            }
            "a2_equip" => {
                let mode = mode.context("A2 baseline absent")?;
                let sent = match mode {
                    GameMode::Survival => {
                        client
                            .survival()
                            .transfer_inventory(InventorySource::Player, 38)
                            .await?
                    }
                    _ => {
                        client
                            .creative()
                            .transfer_inventory(InventorySource::Player, 38)
                            .await?
                    }
                };
                let record = tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        let record = client
                            .survival()
                            .inventory_transfer_record()
                            .await?
                            .context("A2 equipment transfer absent")?;
                        anyhow::ensure!(
                            record.id == sent.id && record.requires_inspection.is_none(),
                            "A2 equipment conflict: {:?}",
                            record.requires_inspection
                        );
                        if record.stage == InventoryTransferStage::ObservedTransferred {
                            break Ok::<_, anyhow::Error>(record);
                        }
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                })
                .await??;
                let player = client.player_state().await?;
                anyhow::ensure!(
                    record
                        .changed_slots
                        .iter()
                        .any(|s| s.player_slot == Some(8) && s.receipt.is_some()),
                    "A2 boots lack an actual feet-slot receipt"
                );
                anyhow::ensure!(
                    player.inventory.slots[38]
                        .as_ref()
                        .is_some_and(|s| s.value == SlotKnowledge::Empty),
                    "A2 boots source not empty"
                );
                emit(
                    &command,
                    serde_json::json!({"transfer":record,"player":player}),
                )?;
            }
            "a2_attack" => {
                let mode = mode.context("A2 baseline absent")?;
                let target = sheep.context("A2 sheep absent")?;
                let initial = client.player_state().await?;
                let wrong = match mode {
                    GameMode::Survival => client.creative().attack_entity(target, false).await,
                    _ => client.survival().attack_entity(target, false).await,
                };
                anyhow::ensure!(wrong.is_err(), "A2 wrong-mode entity request accepted");
                let dispatch = match mode {
                    GameMode::Survival => client.survival().attack_entity(target, false).await?,
                    _ => client.creative().attack_entity(target, false).await?,
                };
                emit(
                    &command,
                    serde_json::json!({"initial":initial,"target":target,"dispatch":dispatch,"wrong_mode":wrong.unwrap_err().to_string()}),
                )?;
            }
            "a2_retire" => {
                let mode = mode.context("A2 baseline absent")?;
                let target = sheep.context("A2 sheep absent")?;
                let spawns = tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        let spawns = client.entity_spawns().await?;
                        if !spawns.entities.iter().any(|e| e.id == target) {
                            break Ok::<_, anyhow::Error>(spawns);
                        }
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                })
                .await??;
                let rejected = match mode {
                    GameMode::Survival => client.survival().attack_entity(target, false).await,
                    _ => client.creative().attack_entity(target, false).await,
                };
                anyhow::ensure!(rejected.is_err(), "A2 retired entity accepted");
                emit(
                    &command,
                    serde_json::json!({"spawns":spawns,"rejected":rejected.unwrap_err().to_string()}),
                )?;
            }
            "a2_interact" => {
                let mode = mode.context("A2 baseline absent")?;
                let target = villager.context("A2 villager absent")?;
                let initial = client.player_state().await?;
                let dispatch = match mode {
                    GameMode::Survival => {
                        client
                            .survival()
                            .interact_entity(target, voxrig::client::Hand::Main, false)
                            .await?
                    }
                    _ => {
                        client
                            .creative()
                            .interact_entity(target, voxrig::client::Hand::Main, false)
                            .await?
                    }
                };
                let screen = tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        let screen = client.screen_state().await?;
                        if screen.screen.as_ref().is_some_and(|s| {
                            s.menu_name.as_deref() == Some("minecraft:merchant")
                                && s.id.opened_sequence() > initial.receive_sequence
                        }) {
                            break Ok::<_, anyhow::Error>(screen);
                        }
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                })
                .await??;
                emit(
                    &command,
                    serde_json::json!({"initial":initial,"target":target,"dispatch":dispatch,"screen":screen}),
                )?;
            }
            "a2_disconnect" => {
                client.disconnect().await?;
                emit(&command, serde_json::json!({"disconnected":true}))?;
                return Ok(());
            }
            _ => anyhow::bail!("unknown A2 command"),
        }
    }
    anyhow::bail!("A2 controller ended without disconnect")
}

// The same consumer records before the first configuration/play receive, then
// decodes only diagnostic facts. Scene forecasts own no live transport.
async fn recording_scene_probe(client: &Client) -> anyhow::Result<()> {
    let mut commands = BufReader::new(tokio::io::stdin()).lines();
    emit("a4_ready", client.player_state().await?)?;
    anyhow::ensure!(commands.next_line().await?.as_deref() == Some("a4_capture"));
    let player = wait_player(client, |p| p.game_mode == Some(GameMode::Survival)
        && p.received_pose.as_ref().is_some_and(|r|r.position==[0.5,65.0,0.5])
        && p.inventory.slots.get(36).and_then(Option::as_ref).is_some_and(|s|
            matches!(&s.value,SlotKnowledge::Item {item} if item.name=="minecraft:oak_planks" && item.count==3)))
        .await?;
    wait_block(client, [0, 65, 1], "minecraft:stone").await?;
    let region = Region {
        min: [-2, 63, -2],
        max: [5, 68, 3],
    };
    let scene = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match client.survival().capture_scene(region).await {
                Ok(scene) => return Ok::<_, anyhow::Error>(scene),
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
    })
    .await
    .context("stationary scene capture wait")??;
    let controls = (0..34)
        .map(|tick| SurvivalControl {
            yaw: 270.0,
            input: SurvivalInput {
                forward: if tick < 4 { 1 } else { 0 },
                ..Default::default()
            },
        })
        .collect::<Vec<_>>();
    let prediction = scene.preview_path(&controls)?;
    let live_prediction = client.survival().preview_path(&controls).await?;
    anyhow::ensure!(
        prediction.initial_frame == live_prediction.initial_frame
            && prediction.frames == live_prediction.frames
            && prediction.terminal_clearance == live_prediction.terminal_clearance,
        "detached/native forecast differs"
    );
    anyhow::ensure!(matches!(
        prediction.terminal_clearance,
        voxrig::client::survival::TerminalClearance::Admitted { .. }
    ));
    anyhow::ensure!(scene.preview_path(&[]).is_err());
    anyhow::ensure!(
        client
            .survival()
            .capture_scene(Region {
                min: [0, 65, 0],
                max: [0, 65, 0]
            })
            .await
            .is_err(),
        "omitted standing halo accepted"
    );
    let outside = vec![
        SurvivalControl {
            yaw: 270.0,
            input: SurvivalInput {
                forward: 1,
                ..Default::default()
            }
        };
        120
    ];
    anyhow::ensure!(
        scene.preview_path(&outside).is_err(),
        "missing captured geometry guessed"
    );
    let trace = client.stop_packet_trace().await?;
    let path = std::env::var("VOXRIG_NATIVE_RECORDING_PATH")?;
    std::fs::write(&path, serde_json::to_vec(&trace)?)?;
    let loaded: PacketTrace = serde_json::from_slice(&std::fs::read(&path)?)?;
    let query = Region {
        min: [0, 65, 1],
        max: [0, 65, 1],
    };
    let replay = loaded.replay(query, 256)?;
    let live = client.capture(query).await?;
    anyhow::ensure!(
        replay.dimension == live.player.dimension
            && replay.received_pose == live.player.received_pose
            && replay.game_mode == live.player.game_mode
            && replay.may_fly == live.player.may_fly
    );
    let health = live.player.health.as_ref().map(|r| RecordedValue {
        value: r.value,
        source: r.source,
    });
    anyhow::ensure!(
        replay.health == health,
        "replayed health provenance differs"
    );
    let slots = live
        .player
        .inventory
        .slots
        .iter()
        .map(|s| {
            s.as_ref().map(|r| RecordedValue {
                value: RecordedSlotKnowledge::from(&r.value),
                source: r.source,
            })
        })
        .collect::<Vec<_>>();
    let cursor = live
        .player
        .inventory
        .cursor
        .as_ref()
        .map(|r| RecordedValue {
            value: RecordedSlotKnowledge::from(&r.value),
            source: r.source,
        });
    anyhow::ensure!(
        replay.inventory.slots == slots
            && replay.inventory.cursor == cursor
            && replay.inventory.window_id == live.player.inventory.window_id
            && replay.inventory.screen_revision == live.player.inventory.screen_revision,
        "replayed inventory/data/provenance differs"
    );
    anyhow::ensure!(replay.blocks[0].state == live.world.blocks[0].state);
    emit(
        "a4_captured",
        serde_json::json!({"baseline":player,"live":live,"replay":replay,"trace_records":trace.records.len(),"trace_complete":trace.complete,"scene":prediction,"live_preview":live_prediction}),
    )?;
    anyhow::ensure!(commands.next_line().await?.as_deref() == Some("a4_detached"));
    wait_block(client, [1, 65, 0], "minecraft:stone").await?;
    // Immutable geometry survives actual live updates, then source disconnection.
    let before_close = scene.preview_path(&controls)?;
    anyhow::ensure!(before_close.frames == prediction.frames);
    client.disconnect().await?;
    anyhow::ensure!(client.survival().capture_scene(region).await.is_err());
    let detached = scene.preview_path(&controls)?;
    let offline = loaded.replay(query, 256)?;
    anyhow::ensure!(
        detached.initial_frame == prediction.initial_frame
            && detached.frames == prediction.frames
            && detached.terminal_clearance == prediction.terminal_clearance
            && offline == replay
    );
    emit(
        "a4_detached",
        serde_json::json!({"closed":true,"offline_replay":offline,"detached_preview":detached,"live_update_seen":true}),
    )?;
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port: u16 = std::env::var("VOXRIG_PORT")?.parse()?;
    let config =
        ConnectionConfig::offline_from_env(Server::new("127.0.0.1", port), "UnifiedProbe")?;
    let scenario = std::env::var("VOXRIG_NATIVE_SCENARIO").ok();
    let client = if scenario.as_deref() == Some("recording-scene") {
        Client::connect_recorded(config.clone(), 16_777_216).await?
    } else {
        Client::connect(config.clone()).await?
    };
    client.wait_until_ready().await?;
    if scenario.as_deref() == Some("recording-scene") {
        return recording_scene_probe(&client).await;
    }
    if std::env::var("VOXRIG_NATIVE_SCENARIO").ok().as_deref() == Some("equipment-entity") {
        return equipment_entity_probe(&client).await;
    }
    if std::env::var("VOXRIG_NATIVE_SCENARIO").ok().as_deref() == Some("basic-workflow") {
        return basic_workflow_probe(&client).await;
    }
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
        return mining_probe(&client, None).await;
    }
    if std::env::var("VOXRIG_NATIVE_SCENARIO").ok().as_deref() == Some("mining-recovery") {
        return mining_probe(&client, Some(&config)).await;
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
