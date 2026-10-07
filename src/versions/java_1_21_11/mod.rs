//! Java 1.21.11 wire protocol (774), registries and client behavior.
pub mod checked;
mod client;
mod component_nbt;
#[allow(dead_code)]
mod ids;
pub(crate) use ids::play_clientbound as ClientboundIds;
pub(crate) mod item_components;
mod math;
mod piston_nbt;
pub mod reconstruction;
#[cfg(test)]
mod recovery_tests;
mod wire;
pub(crate) use wire::Reader as ScoreboardReader;
mod world;
pub use client::operations;
pub use client::players;
pub use client::raycast;
pub use client::recording;
pub(crate) use client::{Bot, replay_packets};
pub use client::{PacketRecord, PacketTrace};

/// Java 1.21.11 functionality outside the common `Client` API, obtained with
/// [`crate::Client::java_1_21_11`].
#[derive(Clone)]
pub struct NativeClient {
    client: crate::Client,
    bot: Bot,
}
impl NativeClient {
    pub(crate) fn new(client: crate::Client, bot: Bot) -> Self {
        Self { client, bot }
    }
    /// The version-selected client this handle belongs to.
    pub fn client(&self) -> &crate::Client {
        &self.client
    }
    /// Unrestricted native operations, including creative and command controls.
    pub fn operations(&self) -> operations::Operations {
        self.bot.operations()
    }
    /// The audited dry-cube survival contract. Creative and command controls are
    /// not reachable from this handle.
    pub fn checked_survival(&self) -> checked::Operations {
        checked::Operations::new(self.client.clone(), self.bot.operations())
    }
    /// Client piston reconstruction alongside the unchanged received cache.
    /// Inspect both the typed issue and cell availability; this is not server confirmation.
    pub async fn observe_client_region(
        &self,
        region: crate::Region,
    ) -> crate::Result<reconstruction::ClientObservation> {
        self.bot.observe_client_region(region).await
    }
    /// Shares immutable region cells within one receive/reconstruction generation.
    /// Every call still captures a new receive boundary and local frame.
    pub async fn observe_shared_client_region(
        &self,
        region: crate::Region,
    ) -> crate::Result<reconstruction::SharedClientRegion> {
        self.bot.observe_shared_client_region(region).await
    }
    /// Sends an ordinary use-on-block interaction without the common mode checks.
    /// Dispatch is not acceptance.
    pub async fn interact_block(
        &self,
        position: [i32; 3],
        face: crate::client::BlockFace,
    ) -> crate::Result<()> {
        self.bot.interact_block(position, face).await
    }
}

fn registry() -> &'static crate::block_state::StateRegistry {
    static REGISTRY: std::sync::OnceLock<crate::block_state::StateRegistry> =
        std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| {
        crate::block_state::StateRegistry::parse(include_str!(
            "../../../data/java_1_21_11/blocks.json"
        ))
        .expect("valid bundled 1.21.11 registry")
    })
}

/// Interprets an ID only in the Java 1.21.11 registry.
pub fn native_state(id: i32) -> crate::Result<crate::NativeBlockState> {
    registry().decode(id)
}

/// Resolves a complete native state using only the Java 1.21.11 registry.
pub fn state_id(state: &crate::NativeBlockState) -> crate::Result<i32> {
    registry().encode(state)
}

fn validate_state_id(id: i32) -> anyhow::Result<()> {
    registry().validate_id(id)?;
    Ok(())
}
