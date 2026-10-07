//! Isolated-server probe for continuous control (`Survival::start_control`).
//! Prints `READY x y z`, waits for an operator to build a water pool ahead
//! (z+10..z+16, two blocks deep), then walks, sprints, sprint-jumps, sneaks and
//! swims through the pool. Prints each phase's record and `FINAL x y z`.
//! VOXRIG_MINECRAFT_VERSION selects 1.16.1 or 1.21.11; VOXRIG_PORT selects the port.
use std::io::Write;
use std::time::Duration;
use voxrig::client::control::Controls;
use voxrig::client::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = std::env::var("VOXRIG_PORT")
        .unwrap_or_else(|_| "25565".into())
        .parse()?;
    let config = ConnectionConfig::offline_from_env(Server::new("127.0.0.1", port), "ProbeCtl")?;
    let client = Client::connect(config).await?;
    client.wait_until_ready().await?;
    let feet = client.player_state().await?.position.unwrap().value;
    let base = feet.map(|v| v.floor() as i32);
    println!("READY {} {} {}", base[0], base[1], base[2]);
    std::io::stdout().flush()?;
    tokio::time::sleep(Duration::from_secs(5)).await;
    client
        .wait_for_loaded(
            Region {
                min: [base[0] - 4, base[1] - 4, base[2] - 4],
                max: [base[0] + 4, base[1] + 4, base[2] + 28],
            },
            Duration::from_secs(10),
        )
        .await?;
    let survival = client.survival();
    let record = survival.start_control().await?;
    println!("START {record:?}");
    let walk = |forward: i8, jump: bool, sneak: bool, sprint: bool| Controls {
        forward,
        strafe: 0,
        jump,
        sneak,
        sprint,
        yaw: 0.0,
        pitch: 0.0,
    };
    let phases = [
        ("walk", walk(1, false, false, false), 20),
        ("sprint", walk(1, false, false, true), 10),
        ("sprint_jump", walk(1, true, false, true), 12),
        ("sneak", walk(1, false, true, false), 10),
        ("rest", walk(0, false, false, false), 10),
        ("swim_across", walk(1, true, false, false), 80),
        ("rest_after", walk(0, false, false, false), 20),
    ];
    for (name, controls, ticks) in phases {
        survival.set_controls(controls).await?;
        tokio::time::sleep(Duration::from_millis(50 * ticks)).await;
        let r = survival.control_record().await?.unwrap();
        let f = r.frame.as_ref().unwrap();
        println!(
            "PHASE {name} status={:?} sent={} corrections={} velocity_updates={} pos={:?} ground={} water={} pose={:?}",
            r.status,
            r.dispatched_ticks,
            r.corrections,
            r.velocity_updates,
            f.position,
            f.on_ground,
            f.in_water,
            f.pose
        );
    }
    // A server teleport mid-session: applied as a correction; held keys continue.
    survival.set_controls(walk(1, false, false, false)).await?;
    println!("TELEPORT_NOW");
    std::io::stdout().flush()?;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    survival.set_controls(walk(0, false, false, false)).await?;
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let r = survival.control_record().await?.unwrap();
    println!(
        "AFTER_TELEPORT status={:?} corrections={} pos={:?}",
        r.status,
        r.corrections,
        r.frame.as_ref().unwrap().position
    );
    let last = survival.stop_control().await?.unwrap();
    let p = last.frame.as_ref().unwrap().position;
    println!("STOP {:?} corrections={}", last.status, last.corrections);
    println!("FINAL {} {} {}", p[0], p[1], p[2]);
    std::io::stdout().flush()?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    client.disconnect().await?;
    Ok(())
}
