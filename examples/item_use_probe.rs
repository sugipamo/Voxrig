//! Isolated-server probe for the common item-use operations.
//! Prints `CMD <console command>` lines for an operator (or a driver script) to run on the
//! server console, then eats a golden apple, raises and lowers a shield, draws and shoots a
//! bow, places a torch, pulls a lever and opens a door, reading only received state.
//! VOXRIG_MINECRAFT_VERSION selects 1.16.1 or 1.21.11; VOXRIG_PORT selects the port.
use anyhow::{Context, bail, ensure};
use std::io::Write;
use std::time::{Duration, Instant};
use voxrig::client::control::Controls;
use voxrig::client::prelude::*;

const NAME: &str = "ProbeUse";

fn cmd(line: String) -> anyhow::Result<()> {
    println!("CMD {line}");
    std::io::stdout().flush()?;
    Ok(())
}

fn give(version: MinecraftVersion, slot: &str, item: &str, count: u8) -> String {
    match version {
        MinecraftVersion::Java1_16_1 => format!("replaceitem entity {NAME} {slot} {item} {count}"),
        _ => format!("item replace entity {NAME} {slot} with {item} {count}"),
    }
}

async fn wait_player(
    client: &Client,
    what: &str,
    accept: impl Fn(&PlayerObservation) -> bool,
) -> anyhow::Result<(PlayerObservation, Duration)> {
    let started = Instant::now();
    loop {
        let state = client.player_state().await?;
        if accept(&state) {
            return Ok((state, started.elapsed()));
        }
        if started.elapsed() > Duration::from_secs(5) {
            bail!(
                "timed out waiting for {what}: using_item={:?}",
                state.using_item
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn count(state: &PlayerObservation, slot: usize) -> u32 {
    match state.inventory.slots.get(slot).and_then(|s| s.as_ref()) {
        Some(ObservedValue {
            value: SlotKnowledge::Item { item },
            ..
        }) => item.count,
        _ => 0,
    }
}

fn using(state: &PlayerObservation) -> Option<Option<Hand>> {
    state.using_item.as_ref().map(|u| u.value)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = std::env::var("VOXRIG_PORT")
        .unwrap_or_else(|_| "25565".into())
        .parse()?;
    let config = ConnectionConfig::offline_from_env(Server::new("127.0.0.1", port), NAME)?;
    let version = config.version;
    let client = Client::connect(config).await?;
    client.wait_until_ready().await?;
    let feet = client
        .player_state()
        .await?
        .position
        .context("position")?
        .value;
    let [x, y, z] = feet.map(|v| v.floor() as i32);
    for line in [
        format!("gamemode survival {NAME}"),
        format!("clear {NAME}"),
        format!("effect clear {NAME}"),
        format!(
            "fill {} {} {} {} {} {} air",
            x - 3,
            y,
            z - 3,
            x + 3,
            y + 3,
            z + 4
        ),
        format!(
            "fill {} {} {} {} {} {} stone",
            x - 3,
            y - 1,
            z - 3,
            x + 3,
            y - 1,
            z + 4
        ),
        format!(
            "tp {NAME} {} {y} {} 0 0",
            f64::from(x) + 0.5,
            f64::from(z) + 0.5
        ),
        format!(
            "setblock {} {y} {} lever[face=floor,facing=north]",
            x + 1,
            z + 2
        ),
        format!(
            "setblock {} {y} {} oak_door[half=lower,facing=north]",
            x - 1,
            z + 2
        ),
        format!(
            "setblock {} {} {} oak_door[half=upper,facing=north]",
            x - 1,
            y + 1,
            z + 2
        ),
        give(version, "hotbar.0", "golden_apple", 2),
        give(version, "hotbar.1", "bow", 1),
        give(version, "hotbar.2", "arrow", 4),
        give(version, "hotbar.3", "torch", 4),
        give(version, "weapon.offhand", "shield", 1),
    ] {
        cmd(line)?;
    }
    tokio::time::sleep(Duration::from_secs(3)).await;
    let survival = client.survival();
    survival.look([0.0, 30.0]).await?;
    wait_player(&client, "items", |s| {
        count(s, 36) == 2
            && count(s, 37) == 1
            && count(s, 38) == 4
            && count(s, 39) == 4
            && count(s, 45) == 1
    })
    .await?;
    println!(
        "SETUP done using_item={:?}",
        client.player_state().await?.using_item
    );

    // Golden apple: always edible, 32 ticks.
    survival.select_hotbar(0).await?;
    let receipt = survival.use_item(Hand::Main).await?;
    let (_, started) =
        wait_player(&client, "eating", |s| using(s) == Some(Some(Hand::Main))).await?;
    let (state, finished) = wait_player(&client, "eaten", |s| {
        using(s) == Some(None) && count(s, 36) == 1
    })
    .await?;
    println!(
        "EAT receipt_seq={:?} using_after={started:?} done_after={finished:?} apples={}",
        receipt.interaction_sequence,
        count(&state, 36)
    );

    // Shield in the off hand: raise, hold, lower.
    survival.use_item(Hand::Off).await?;
    wait_player(&client, "shield up", |s| using(s) == Some(Some(Hand::Off))).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    survival.release_use_item().await?;
    let (_, lowered) = wait_player(&client, "shield down", |s| using(s) == Some(None)).await?;
    println!("SHIELD lowered_after={lowered:?}");

    // Bow: draw for 1.2 s and release; the arrow is consumed by the server.
    survival.select_hotbar(1).await?;
    survival.use_item(Hand::Main).await?;
    wait_player(&client, "bow drawn", |s| using(s) == Some(Some(Hand::Main))).await?;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    survival.release_use_item().await?;
    let (state, _) = wait_player(&client, "arrow shot", |s| {
        using(s) == Some(None) && count(s, 38) == 3
    })
    .await?;
    println!("BOW arrows={}", count(&state, 38));

    // Torch on the floor block two cells ahead.
    survival.select_hotbar(3).await?;
    let receipt = survival
        .use_on_block(
            [x, y - 1, z + 2],
            BlockFace::Up,
            [0.5, 1.0, 0.5],
            Hand::Main,
        )
        .await?;
    client
        .wait_for_block([x, y, z + 2], Duration::from_secs(3), |b| {
            b.is_some_and(|b| b.name == "minecraft:torch")
        })
        .await?;
    let (state, _) = wait_player(&client, "torch used", |s| count(s, 39) == 3).await?;
    println!(
        "TORCH placed receipt_seq={:?} torches={}",
        receipt.interaction_sequence,
        count(&state, 39)
    );

    // Empty hand on a lever and a door.
    survival.select_hotbar(4).await?;
    survival
        .use_on_block(
            [x + 1, y, z + 2],
            BlockFace::Up,
            [0.5, 0.1, 0.5],
            Hand::Main,
        )
        .await?;
    client
        .wait_for_block([x + 1, y, z + 2], Duration::from_secs(3), |b| {
            b.is_some_and(|b| b.properties.get("powered").map(String::as_str) == Some("true"))
        })
        .await?;
    println!("LEVER powered");
    survival
        .use_on_block(
            [x - 1, y, z + 2],
            BlockFace::South,
            [0.5, 0.5, 1.0],
            Hand::Main,
        )
        .await?;
    client
        .wait_for_block([x - 1, y, z + 2], Duration::from_secs(3), |b| {
            b.is_some_and(|b| b.properties.get("open").map(String::as_str) == Some("true"))
        })
        .await?;
    println!("DOOR opened");
    // Out-of-reach targets are refused before anything is sent.
    let far = survival
        .use_on_block(
            [x, y - 1, z + 9],
            BlockFace::Up,
            [0.5, 1.0, 0.5],
            Hand::Main,
        )
        .await;
    ensure!(far.is_err(), "out-of-reach target accepted");
    println!("FAR refused: {}", far.unwrap_err());

    // Held-key movement with the shield raised, then lowered, then raised again.
    survival.use_item(Hand::Off).await?;
    wait_player(&client, "shield up", |s| using(s) == Some(Some(Hand::Off))).await?;
    survival.start_control().await?;
    let ahead = Controls {
        forward: 1,
        sprint: true,
        ..Default::default()
    };
    survival.set_controls(ahead).await?;
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let r = survival.control_record().await?.context("record")?;
    let f = r.frame.clone().context("frame")?;
    println!(
        "CONTROL_SHIELD status={:?} using={:?} sprinting={} pos={:?}",
        r.status, f.using_item, f.sprinting, f.position
    );
    ensure!(
        f.using_item.is_some() && !f.sprinting,
        "shield slowdown not applied"
    );
    survival.release_use_item().await?;
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let f = survival
        .control_record()
        .await?
        .context("record")?
        .frame
        .context("frame")?;
    println!(
        "CONTROL_LOWERED using={:?} sprinting={}",
        f.using_item, f.sprinting
    );
    ensure!(
        f.using_item.is_none() && f.sprinting,
        "release did not end the slowdown"
    );
    survival.use_item(Hand::Off).await?;
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let f = survival
        .control_record()
        .await?
        .context("record")?
        .frame
        .context("frame")?;
    println!(
        "CONTROL_RAISED_WHILE_MOVING using={:?} sprinting={}",
        f.using_item, f.sprinting
    );
    survival.set_controls(Controls::default()).await?;
    tokio::time::sleep(Duration::from_millis(1000)).await;
    survival.release_use_item().await?;
    let r = survival.control_record().await?.context("record")?;
    survival.stop_control().await?;
    let f = r.frame.context("frame")?;
    println!(
        "FINAL corrections={} status={:?} pos={:?}",
        r.corrections, r.status, f.position
    );
    cmd(format!("data get entity {NAME} Pos"))?;
    tokio::time::sleep(Duration::from_millis(1000)).await;
    println!("DONE");
    client.disconnect().await?;
    Ok(())
}
