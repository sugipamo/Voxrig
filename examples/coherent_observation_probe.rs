use anyhow::{Result, ensure};
use zen_minecraft_client::{
    BlockPos, BotManager, CleanupDispatchOutcome, CleanupPrimitive, CoherentObservationRequest,
    ConnectionLifecycle, Player, PrimitiveDispatchOutcome, PrimitiveOperation, Server,
};

#[tokio::main]
async fn main() -> Result<()> {
    let port = std::env::var("MC_PORT")
        .unwrap_or_else(|_| "25566".to_owned())
        .parse()?;
    let manager = BotManager::new(Server::new("127.0.0.1", port));
    let bot = manager.connect(Player::offline("CoherentProbe")).await?;
    bot.wait_until_ready().await?;
    let _ = bot.observe(0).await?;

    let player = bot.player().await;
    let requested = vec![
        BlockPos {
            x: player.x.floor() as i32,
            y: player.y.floor() as i32 - 1,
            z: player.z.floor() as i32,
        },
        BlockPos {
            x: player.x.floor() as i32 - 1,
            y: player.y.floor() as i32,
            z: player.z.floor() as i32,
        },
    ];
    let request = CoherentObservationRequest {
        body_interest_generation: Some(7),
        observation_interest: requested.clone(),
        ..CoherentObservationRequest::default()
    };
    let first = bot.capture_coherent_observation(request.clone()).await?;
    let second = bot.capture_coherent_observation(request).await?;

    let context = bot.operation_context(second.sequence.get());
    let look = bot
        .dispatch_primitive(
            context,
            PrimitiveOperation::Look {
                x: second.player.x,
                y: second.player.y,
                z: second.player.z,
                yaw: second.player.yaw,
                pitch: second.player.pitch,
                on_ground: second.player.on_ground,
            },
        )
        .await
        .map_err(|error| anyhow::anyhow!("typed look dispatch failed: {error:?}"))?;
    ensure!(look == PrimitiveDispatchOutcome::Dispatched);
    let cleanup = bot
        .dispatch_cleanup(context, CleanupPrimitive::ControlClear)
        .await
        .map_err(|error| anyhow::anyhow!("control cleanup failed: {error:?}"))?;
    ensure!(cleanup == CleanupDispatchOutcome::AppliedLocally);

    ensure!(first.generation == bot.connection_generation());
    ensure!(second.generation == first.generation);
    ensure!(second.sequence.get() == first.sequence.get() + 1);
    ensure!(first.observation_interest.cells.len() == requested.len());
    ensure!(first.observation_interest.unloaded_cells == 0);
    ensure!(
        first.observation_interest.body_generation == Some(7),
        "interest generation changed"
    );
    ensure!(
        first
            .observation_interest
            .cells
            .iter()
            .map(|cell| cell.position)
            .eq(requested),
        "interest order changed"
    );
    ensure!(first.entities.len() <= 512);
    ensure!(first.events.len() <= 256);

    println!(
        "generation={} first_sequence={} second_sequence={} blocks={} light_unknown={} interest={} events={} events_omitted={} typed_look={look:?} control_cleanup={cleanup:?}",
        first.generation.get(),
        first.sequence.get(),
        second.sequence.get(),
        first.observation_interest.cells.len(),
        first.observation_interest.light_unknown_cells,
        first.observation_interest.cells.len(),
        first.events.len(),
        first.events_omitted,
    );

    manager.disconnect_all().await?;
    ensure!(
        bot.connection_lifecycle() == ConnectionLifecycle::Disconnected,
        "disconnect did not reach a confirmed terminal state"
    );
    Ok(())
}
