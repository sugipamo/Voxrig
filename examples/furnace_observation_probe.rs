use anyhow::{Result, ensure};
use std::time::Duration;
use tokio::time::{sleep, timeout};
use voxrig::versions::java_1_16_1::{
    BlockFace, BlockPos, BotManager, CoherentObservationRequest, DispatchOutcome, Hand, Operation,
    Player, Server,
};

#[tokio::main]
async fn main() -> Result<()> {
    let port = std::env::var("MC_PORT")
        .unwrap_or_else(|_| "25566".to_owned())
        .parse()?;
    let target = BlockPos { x: 1, y: 4, z: 0 };
    let manager = BotManager::new(Server::new("127.0.0.1", port));
    let bot = manager.connect(Player::offline("FurnObsProbe")).await?;
    bot.wait_until_ready().await?;
    let _ = bot.observe(0).await?;
    let source = bot
        .capture_coherent_observation(CoherentObservationRequest::default())
        .await?;

    let outcome = bot
        .dispatch_operation(
            bot.operation_context(source.sequence.get()),
            Operation::BlockInteraction {
                hand: Hand::Main,
                position: target,
                face: BlockFace::Up,
                cursor: [0.5, 1.0, 0.5],
                inside_block: false,
                sneak: voxrig::versions::java_1_16_1::InteractionSneakRequirement::not_required(),
            },
        )
        .await?;
    ensure!(outcome == DispatchOutcome::Dispatched);

    let observation = timeout(Duration::from_secs(5), async {
        loop {
            let observation = bot
                .capture_coherent_observation(CoherentObservationRequest::default())
                .await?;
            if observation.open_furnace.is_some() {
                return Ok::<_, anyhow::Error>(observation);
            }
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await??;
    let furnace = observation.open_furnace.as_ref().expect("checked above");
    ensure!(
        furnace.position == target,
        "furnace position was not correlated"
    );
    ensure!(furnace.window.window_type == 13, "wrong window type");
    ensure!(furnace.slots.len() >= 3, "furnace slots were not captured");

    println!(
        "generation={} sequence={} furnace=({},{},{}) window_id={} slots={} properties={}",
        observation.generation.get(),
        observation.sequence.get(),
        furnace.position.x,
        furnace.position.y,
        furnace.position.z,
        furnace.window.id,
        furnace.slots.len(),
        furnace.properties.len(),
    );
    manager.disconnect_all().await?;
    Ok(())
}
