//! JSON-line driver for scripts/run_owned_operation_live.py; common APIs only.
use serde::Serialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, future::Future, io::Write, pin::Pin, time::Duration};
use tokio::io::{AsyncBufReadExt, BufReader};
use voxrig::{body::*, client::prelude::*};

type JsonFuture<'a> = Pin<Box<dyn Future<Output = Value> + Send + 'a>>;
trait ProbeOperation: Send + Sync {
    fn snapshot(&self) -> Value;
    fn finished(&self) -> JsonFuture<'_>;
}
fn completion<T: Serialize>(done: &OperationCompletion<T>) -> Value {
    json!({"id":done.id,"scope":done.scope,"outcome":done.outcome,"record":done.record,
        "owner_joined":done.owner_joined,"failure":done.failure.as_ref().map(|f| json!({"kind":format!("{:?}",f.kind),"message":f.message}))})
}
impl<T: Serialize + Clone + Send + Sync + 'static> ProbeOperation for OwnedOperation<T> {
    fn snapshot(&self) -> Value {
        let state = self.snapshot();
        json!({"id":self.id(),"scope":self.scope(),"completion":state.completion.as_deref().map(completion),
            "joined":state.joined.as_deref().map(completion),"record":state.record})
    }
    fn finished(&self) -> JsonFuture<'_> {
        Box::pin(async move { completion(&*self.finished().await) })
    }
}
async fn retain<T: Serialize + Clone + Send + Sync + 'static>(
    operations: &mut BTreeMap<u64, Box<dyn ProbeOperation>>,
    operation: OwnedOperation<T>,
) -> Value {
    // Explicitly poll then drop an observation future, never the owner.
    let mut waiter = Box::pin(operation.finished());
    let ready =
        std::future::poll_fn(|cx| std::task::Poll::Ready(waiter.as_mut().poll(cx).is_ready()))
            .await;
    drop(waiter);
    let id = operation.id();
    operations.insert(id.ordinal, Box::new(operation));
    json!({"id":id.ordinal,"identity":id,"waiter_dropped":true,"first_poll_ready":ready})
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Client::connect(ConnectionConfig::offline_from_env(
        Server::new("127.0.0.1", std::env::var("VOXRIG_PORT")?.parse()?),
        "ClimbingProbe",
    )?)
    .await?;
    client.wait_until_ready().await?;
    let survival = client.survival();
    let body = HeadlessBody::new(client.clone(), BodyConfig::default())?;
    let mut operations = BTreeMap::<u64, Box<dyn ProbeOperation>>::new();
    println!("{}", json!({"ready":true}));
    std::io::stdout().flush()?;
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let request: Value = serde_json::from_str(&line)?;
        let result: anyhow::Result<Value> = async {
            Ok(match request["command"].as_str().unwrap_or_default() {
                "prepare" => {
                    let target: [f64; 3] = serde_json::from_value(request["position"].clone())?;
                    tokio::time::timeout(Duration::from_secs(10), async {
                        loop {
                            let state = client.player_state().await?;
                            if state.received_pose.as_ref().is_some_and(|p| {
                                p.receive_sequence > request["after_sequence"].as_u64().unwrap_or(0)
                                    && p.position
                                        .iter()
                                        .zip(target)
                                        .all(|(a, b)| (a - b).abs() < 0.01)
                            }) {
                                break Ok::<_, anyhow::Error>(());
                            }
                            tokio::task::yield_now().await;
                        }
                    })
                    .await??;
                    client
                        .wait_for_loaded(
                            Region {
                                min: [-3, 64, -3],
                                max: [3, 68, 3],
                            },
                            Duration::from_secs(10),
                        )
                        .await?;
                    json!({"prepared":true})
                }
                "player" => serde_json::to_value(client.player_state().await?)?,
                "crafting" => serde_json::to_value(client.received_crafting().await?)?,
                "record" => serde_json::to_value(survival.control_record().await?)?,
                "start" => serde_json::to_value(
                    client
                        .player_control(GameMode::Survival)
                        .start_control_best_effort()
                        .await?,
                )?,
                "keys" => {
                    let id = survival.control_record().await?.unwrap().session_id;
                    serde_json::to_value(
                        client
                            .player_control(GameMode::Survival)
                            .set_controls_for(
                                id,
                                serde_json::from_value(request["controls"].clone())?,
                            )
                            .await?,
                    )?
                }
                "click" => {
                    let session = client.player_state().await?.session;
                    let operation = survival.begin_inventory_click(
                        session,
                        InventorySource::Player,
                        request["slot"].as_u64().unwrap().try_into()?,
                        InventoryClickButton::Left,
                        OperationLimits::default(),
                    )?;
                    completion(&*operation.finished().await)
                }
                "send_click" => serde_json::to_value(
                    survival
                        .send_inventory_click(
                            InventorySource::Player,
                            request["slot"].as_u64().unwrap().try_into()?,
                            InventoryClickButton::Left,
                        )
                        .await?,
                )?,
                "resync" => serde_json::to_value(survival.resync_inventory().await?)?,
                "inventory_send_record" => {
                    serde_json::to_value(survival.inventory_send_record().await?)?
                }
                "click_record" => serde_json::to_value(survival.inventory_click_record().await?)?,
                "take_record" => serde_json::to_value(survival.crafting_take_record().await?)?,
                "continue" => {
                    let send = survival.inventory_send_record().await?.unwrap();
                    serde_json::to_value(client.continue_operational_crafting(send.id).await?)?
                }
                "begin" => {
                    let limits = OperationLimits {
                        run_budget: Duration::from_millis(
                            request["budget_ms"].as_u64().unwrap_or(3000),
                        ),
                        cleanup_budget: Duration::ZERO,
                    };
                    match request["kind"].as_str().unwrap() {
                        "send_click" => {
                            retain(
                                &mut operations,
                                survival.begin_send_inventory_click(
                                    client.player_state().await?.session,
                                    InventorySource::Player,
                                    request["slot"].as_u64().unwrap().try_into()?,
                                    InventoryClickButton::Left,
                                    limits,
                                )?,
                            )
                            .await
                        }
                        "send_transfer" => {
                            retain(
                                &mut operations,
                                survival.begin_send_inventory_transfer(
                                    client.player_state().await?.session,
                                    InventorySource::Player,
                                    request["slot"].as_u64().unwrap().try_into()?,
                                    limits,
                                )?,
                            )
                            .await
                        }
                        "stop" => {
                            retain(
                                &mut operations,
                                client.player_control(GameMode::Survival).begin_stop_for(
                                    request["session_id"].as_u64().unwrap(),
                                    limits,
                                )?,
                            )
                            .await
                        }
                        "click" => {
                            retain(
                                &mut operations,
                                survival.begin_inventory_click(
                                    client.player_state().await?.session,
                                    InventorySource::Player,
                                    request["slot"].as_u64().unwrap().try_into()?,
                                    InventoryClickButton::Left,
                                    limits,
                                )?,
                            )
                            .await
                        }
                        "transfer" => {
                            retain(
                                &mut operations,
                                survival.begin_inventory_transfer(
                                    client.player_state().await?.session,
                                    InventorySource::Player,
                                    request["slot"].as_u64().unwrap().try_into()?,
                                    limits,
                                )?,
                            )
                            .await
                        }
                        "take" | "shift_take" => {
                            let grid = client
                                .received_crafting()
                                .await?
                                .ok_or_else(|| anyhow::anyhow!("no received grid"))?;
                            retain(
                                &mut operations,
                                survival.begin_crafting_take(
                                    grid,
                                    if request["kind"] == "take" {
                                        CraftingResultDestination::Cursor
                                    } else {
                                        CraftingResultDestination::Inventory
                                    },
                                    limits,
                                )?,
                            )
                            .await
                        }
                        "operational" => {
                            let grid = client
                                .received_crafting()
                                .await?
                                .ok_or_else(|| anyhow::anyhow!("no received grid"))?;
                            retain(
                                &mut operations,
                                survival.begin_operational_crafting_take(grid, limits)?,
                            )
                            .await
                        }
                        "look" | "move" => {
                            let world = body.observe(ObserveRequest::default()).await?.world;
                            let action = if request["kind"] == "look" {
                                Action::Look {
                                    yaw: 37.,
                                    pitch: 13.,
                                }
                            } else {
                                Action::Move {
                                    controls: serde_json::from_value(request["controls"].clone())?,
                                    ticks: request["ticks"]
                                        .as_u64()
                                        .map(u32::try_from)
                                        .transpose()?,
                                }
                            };
                            retain(
                                &mut operations,
                                body.begin_send_and_trust(
                                    ActionRequest {
                                        action,
                                        world: Some(world),
                                    },
                                    limits,
                                )?,
                            )
                            .await
                        }
                        _ => anyhow::bail!("unknown operation"),
                    }
                }
                "snapshot" => operations
                    .get(&request["id"].as_u64().unwrap())
                    .unwrap()
                    .snapshot(),
                "wait" => {
                    operations
                        .get(&request["id"].as_u64().unwrap())
                        .unwrap()
                        .finished()
                        .await
                }
                "drop_handle" => {
                    operations.remove(&request["id"].as_u64().unwrap());
                    json!({"dropped":true})
                }
                "body_status" => serde_json::to_value(
                    body.action_status(serde_json::from_value(request["id"].clone())?)
                        .await?,
                )?,
                "body_release" => serde_json::to_value(body.release_control().await?)?,
                "retire" => {
                    let result = client
                        .retire_connection(tokio::time::Instant::now() + Duration::from_secs(2))
                        .await;
                    json!({"result":format!("{result:?}")})
                }
                "disconnect" => {
                    client.disconnect().await?;
                    json!({"disconnected":true})
                }
                _ => anyhow::bail!("unknown command"),
            })
        }
        .await;
        println!(
            "{}",
            match result {
                Ok(value) => value,
                Err(error) => json!({"error":error.to_string()}),
            }
        );
        std::io::stdout().flush()?;
        if request["command"] == "disconnect" {
            break;
        }
    }
    Ok(())
}
