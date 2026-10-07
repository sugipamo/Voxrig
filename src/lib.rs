//! A headless Minecraft client with explicitly selected Java version adapters.
//!
//! `Voxrig` implements the protocol-facing body of an agent: connection
//! management, world and entity observation, player state, physics, inventory,
//! crafting, containers, interaction, combat, chat, and structured sound events.
//! It deliberately does not provide pathfinding, semantic perception, planning,
//! memory, or an AI runtime. Those belong in a consumer crate.
//!
//! The established `Bot` API targets Java 1.16.1 (736). [`Client`] also offers a
//! Java 1.21.11 (774) adapter for native block observations, packet traces,
//! outline targeting, creative inventory and block operations, direct movement,
//! received-result-checked survival inventory swaps, and bounded stationary
//! standing contact with own-player state observations. Bounded survival mining,
//! material-accounted placement and dry-cube walking/jump controls through
//! [`versions::java_1_21_11::NativeClient::checked_survival`] use separate intent, dispatch, prediction and result evidence.
//! Mining removal alone does not authorize another mutation, and motion is not
//! server-confirmed rest. [`versions::java_1_21_11::NativeClient::observe_client_region`] additionally
//! reconstructs a bounded set of piston and neighbor effects, including independent
//! moving carriers, while preserving the unchanged received-state view.
//! Neither adapter's local cache is independent confirmation of server state.
//! It does not implement Microsoft authentication or online-mode encryption.
//!
//! # Quick start
//!
//! ```no_run
//! use voxrig::client::prelude::*;
//!
//! # async fn run() -> anyhow::Result<()> {
//! // Set VOXRIG_MINECRAFT_VERSION to an exact supported release, such as 1.16.1.
//! let config = ConnectionConfig::offline_from_env(Server::default(), "AgentOne")?;
//! let client = Client::connect(config).await?;
//! client.wait_until_ready().await?;
//! let player = client.player_state().await?;
//! println!("{:?}", player.game_mode);
//! if player.game_mode == Some(GameMode::Creative) {
//!     client.creative().select_hotbar(0).await?;
//! } else if player.game_mode == Some(GameMode::Survival) {
//!     client.survival().select_hotbar(0).await?;
//! }
//! client.disconnect().await?;
//! # Ok(())
//! # }
//! ```
//!
//! See `docs/api.md` in the repository for the complete public API map and
//! `docs/architecture.md` for the ownership boundary with external controllers.
#![warn(missing_docs)]

pub mod block_state;
pub mod client;
pub mod connection;
mod diagnostic_projection;
mod error;
mod protocol;
pub mod snapshot;
pub mod versions;

// The established 1.16.1 API remains source-compatible and explicitly pinned.
pub use block_state::NativeBlockState;
pub use connection::{Client, ConnectionConfig, Native, Observation, ObservedBlock, Region};
pub use error::{Error, ErrorKind, Result};
pub use versions::MinecraftVersion;
// Java 1.16.1 types stay reachable inside the crate by their historical root
// paths; consumers name them under `versions::java_1_16_1`.
pub(crate) use versions::java_1_16_1::*;

/// Common imports for version-selected clients.
pub mod prelude {
    pub use crate::client::prelude::*;
}
