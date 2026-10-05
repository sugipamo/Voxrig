//! One consumer entry point; version selection happens only during setup.
use voxrig::client::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let host = std::env::var("VOXRIG_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port = std::env::var("VOXRIG_PORT")
        .map_or(Ok(25565), |value| value.parse())
        .map_err(|error| Error::new(ErrorKind::InvalidInput, error))?;
    let username = std::env::var("VOXRIG_USERNAME").unwrap_or_else(|_| "VoxrigProbe".into());
    let config = ConnectionConfig::offline_from_env(Server::new(host, port), username)?;
    config.validate()?;
    println!(
        "Minecraft {} / protocol {}",
        config.version,
        config.version.protocol()
    );
    let registry = Registry::for_version(config.version);
    let block = registry.builtin_id("minecraft:block", "minecraft:stone")?;
    println!(
        "{} block entry = {}",
        registry.builtin_name(&block)?,
        block.value()
    );
    if std::env::args().any(|arg| arg == "--check") {
        println!("Setup valid; no network connection opened");
        return Ok(());
    }
    let client = Client::connect(config).await?;
    client.wait_until_ready().await?;
    let registries = client.server_registry_state().await?;
    let enchantment = registries.find_entry("minecraft:enchantment", "minecraft:unbreaking")?;
    println!(
        "{} entry = {}",
        registries.entry_name(&enchantment)?,
        enchantment.value()
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&client.player_state().await?)
            .map_err(|error| Error::new(ErrorKind::Other, error))?
    );
    client.disconnect().await
}
