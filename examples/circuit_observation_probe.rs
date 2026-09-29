//! Observe a prepared isolated circuit through ordinary lever inputs.
//! Configuration describes bounds and inputs; setup/independent assertions stay external.
use std::{
    io::{self, Write},
    time::Duration,
};
use voxrig::{BlockFace, Client, ConnectionConfig, MinecraftVersion, Region, Server};

#[derive(serde::Deserialize)]
struct Config {
    fixture: String,
    min: [i32; 3],
    max: [i32; 3],
    input: [i32; 3],
    waits_ms: Vec<u64>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config: Config =
        serde_json::from_reader(std::fs::File::open(std::env::var("PROBE_CONFIG")?)?)?;
    anyhow::ensure!(
        !config.waits_ms.is_empty()
            && config.waits_ms.len() <= 8
            && config.waits_ms.iter().all(|t| (500..=10000).contains(t)),
        "bounded input schedule required"
    );
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(std::env::var("TRACE_OUTPUT")?)?;
    let client = Client::connect(ConnectionConfig::offline(
        Server::new("127.0.0.1", std::env::var("MC_PORT")?.parse()?),
        "CircuitProbe",
        MinecraftVersion::Java1_21_11,
    ))
    .await?;
    client.wait_until_ready().await?;
    println!("READY_FOR_FIXTURE_AND_TELEPORT; press enter after setup");
    io::stdout().flush()?;
    tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        io::stdin().read_line(&mut line)
    })
    .await??;
    tokio::time::sleep(Duration::from_secs(5)).await;
    let region = Region {
        min: config.min,
        max: config.max,
    };
    let before = client.observe_region(region).await?;
    anyhow::ensure!(
        before.blocks.len() <= 1000 && before.blocks.iter().all(|b| b.state.is_some()),
        "fully loaded bounded fixture required"
    );
    client.start_packet_trace(16_777_216).await?;
    let mut transient = Vec::new();
    let mut settled = Vec::new();
    let mut inputs = Vec::new();
    for wait in config.waits_ms {
        let boundary = client.observe_client_region(region).await?;
        client.interact_block(config.input, BlockFace::Up).await?;
        inputs.push(serde_json::json!({"before_sequence":boundary.received.receive_sequence,"before_frame":boundary.client_tick,"wait_ms":wait}));
        let started = tokio::time::Instant::now();
        while started.elapsed() < Duration::from_millis(wait) {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let sample = client.observe_client_region(region).await?;
            // Preserve all packet/frame boundaries, but sample late settled states less often.
            if started.elapsed() < Duration::from_millis(1800) {
                transient.push(sample);
            }
        }
        settled.push(client.observe_client_region(region).await?);
    }
    let after_client = client.observe_client_region(region).await?;
    let trace = client.stop_packet_trace().await?;
    let record = serde_json::json!({"fixture":config.fixture,"before":before,"trace":trace,"inputs":inputs,"settled":settled,"transient":transient,"after_client":after_client});
    serde_json::to_writer(file, &record)?;
    client.disconnect().await?;
    println!(
        "TRACE_RETAINED complete={} packets={} issue={}",
        record["trace"]["complete"],
        record["trace"]["records"].as_array().unwrap().len(),
        record["after_client"]["issue"]
    );
    anyhow::ensure!(
        record["trace"]["complete"] == true
            && record["transient"]
                .as_array()
                .unwrap()
                .iter()
                .all(|s| s["issue"].is_null())
            && record["after_client"]["issue"].is_null(),
        "incomplete client reconstruction; capture retained"
    );
    Ok(())
}
