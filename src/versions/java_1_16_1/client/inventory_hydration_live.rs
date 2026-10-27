//! Test-only LIVE driver. Forgetting knowledge reconstructs #424's pre-send
//! state; server packets and production comparison/execution remain unchanged.
use super::*;
use crate::client::prelude::*;
use std::io::Write;
use tokio::io::{AsyncBufReadExt, BufReader};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires run_inventory_hydration_live.py and a disposable 1.16.1 server"]
async fn driver() -> anyhow::Result<()> {
    let port = std::env::var("VOXRIG_PORT")?.parse()?;
    let client = Client::connect(ConnectionConfig::offline_from_env(
        Server::new("127.0.0.1", port),
        "ClimbingProbe",
    )?)
    .await?;
    client.wait_until_ready().await?;
    let crate::connection::Adapter::Java1_16_1(bot) = &client.adapter else {
        anyhow::bail!("1.16.1 only");
    };
    println!("{}", serde_json::json!({"ready": true}));
    std::io::stdout().flush()?;
    let survival = client.survival();
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let request: serde_json::Value = serde_json::from_str(&line)?;
        let response = match request["command"].as_str().unwrap() {
            "player" => serde_json::to_value(client.player_state().await?)?,
            "forget_grid" => {
                // This is deliberate test-only loss of local knowledge, never
                // a fake server value. Exercise the production send from None.
                let _gate = bot.coherent_state_gate.lock().await;
                bot.common_receipts.lock().await.inventory.slots[..5].fill(None);
                serde_json::json!({"forgotten": [0, 1, 2, 3, 4]})
            }
            "send_inventory_click" => serde_json::to_value(
                survival
                    .send_inventory_click(
                        inv::InventorySource::Player,
                        request["slot"].as_u64().unwrap().try_into()?,
                        inv::InventoryClickButton::Left,
                    )
                    .await?,
            )?,
            "inventory_send_record" => {
                let record = survival.inventory_send_record().await?;
                let divergence = match &record {
                    Some(record) => client.inventory_divergence(record.id).await?,
                    None => None,
                };
                serde_json::json!({"record": record, "divergence": divergence})
            }
            "inventory_record" => serde_json::to_value(survival.inventory_click_record().await?)?,
            "disconnect" => {
                client.disconnect().await?;
                println!("{}", serde_json::json!({"disconnected": true}));
                std::io::stdout().flush()?;
                return Ok(());
            }
            command => anyhow::bail!("unknown test command {command}"),
        };
        println!("{response}");
        std::io::stdout().flush()?;
    }
    anyhow::bail!("LIVE driver ended without disconnect")
}
