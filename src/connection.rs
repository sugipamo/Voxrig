//! Version-selected client API. Local observations never claim server confirmation.

use crate::client::adapter::{CoreOps, SessionOps};
use crate::versions::java_1_16_1 as legacy;
use crate::{Error, ErrorKind, MinecraftVersion, NativeBlockState, Result};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static NEXT_CONNECTION_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_connection_id() -> u64 {
    NEXT_CONNECTION_ID.fetch_add(1, Ordering::Relaxed)
}

/// Connection settings with an explicit, immutable game version.
#[derive(Clone, Debug)]
pub struct ConnectionConfig {
    /// Server address; no automatic protocol downgrade is performed.
    pub server: crate::client::Server,
    /// Offline player name.
    pub username: String,
    /// Version of both wire packets and native registry IDs.
    pub version: MinecraftVersion,
    /// Resource and timeout limits implemented by every adapter.
    pub limits: crate::client::ClientLimits,
}

impl ConnectionConfig {
    /// Reads the exact supported version from `VOXRIG_MINECRAFT_VERSION`.
    /// Missing, non-Unicode, and unsupported values fail before network I/O.
    /// Supporting new releases/blocks requires an updated Voxrig build.
    pub fn offline_from_env(
        server: crate::client::Server,
        username: impl Into<String>,
    ) -> Result<Self> {
        let value = std::env::var("VOXRIG_MINECRAFT_VERSION").map_err(|error| {
            Error::new(
                ErrorKind::InvalidInput,
                anyhow::anyhow!("VOXRIG_MINECRAFT_VERSION: {error}"),
            )
        })?;
        Ok(Self::offline(server, username, value.parse()?))
    }

    /// Validates common inputs without opening a connection.
    pub fn validate(&self) -> Result<()> {
        if !(3..=16).contains(&self.username.len())
            || !self
                .username
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_')
            || self.server.host.is_empty()
            || self.server.port == 0
            || self.limits.max_chunks == 0
            || [
                self.limits.connect_timeout,
                self.limits.login_packet_timeout,
                self.limits.play_packet_timeout,
                self.limits.ready_timeout,
            ]
            .iter()
            .any(Duration::is_zero)
        {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                anyhow::anyhow!("invalid offline identity, server address, or client limits"),
            ));
        }
        Ok(())
    }
    /// Creates an explicit offline-mode connection configuration.
    pub fn offline(
        server: crate::client::Server,
        username: impl Into<String>,
        version: MinecraftVersion,
    ) -> Self {
        Self {
            server,
            username: username.into(),
            version,
            limits: crate::client::ClientLimits::default(),
        }
    }
}

/// Inclusive region, independent of a protocol's chunk representation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
    /// Minimum x, y and z coordinates.
    pub min: [i32; 3],
    /// Maximum x, y and z coordinates.
    pub max: [i32; 3],
}

impl Region {
    /// Validates geometry and returns the bounded number of cells.
    pub fn volume(self) -> Result<usize> {
        let mut volume = 1u64;
        for axis in 0..3 {
            if self.min[axis] < -30_000_000
                || self.max[axis] > 30_000_000
                || self.min[axis] > self.max[axis]
            {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    anyhow::anyhow!("invalid region bounds"),
                ));
            }
            let width = (i64::from(self.max[axis]) - i64::from(self.min[axis]) + 1) as u64;
            volume = volume.checked_mul(width).ok_or_else(|| {
                Error::new(
                    ErrorKind::ResourceLimit,
                    anyhow::anyhow!("region volume overflow"),
                )
            })?;
        }
        if volume > 262_144 {
            return Err(Error::new(
                ErrorKind::ResourceLimit,
                anyhow::anyhow!("region exceeds 262144 cells"),
            ));
        }
        Ok(volume as usize)
    }
}

/// One cell in a complete local region observation.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ObservedBlock {
    /// Absolute x, y and z coordinates.
    pub position: [i32; 3],
    /// `None` means unavailable chunk data, never inferred air.
    pub state: Option<NativeBlockState>,
}

/// Local state at one cache revision, bound to its version and connection.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Observation<B = Vec<ObservedBlock>> {
    /// Exact version used to interpret registry IDs.
    pub version: MinecraftVersion,
    /// Process-local connection identity; revisions from different connections
    /// are incomparable. Persisted captures also need their owning process/run
    /// identity: this counter starts again in a new OS process.
    pub connection_id: u64,
    /// World-domain revision, not a server game tick.
    pub revision: u64,
    /// Last applied receive sequence when supported by the adapter; never a server tick.
    pub receive_sequence: Option<u64>,
    /// Local elapsed time of the snapshot.
    pub captured_at: Duration,
    /// Requested inclusive bounds.
    pub region: Region,
    /// Includes air and explicitly unavailable cells.
    pub blocks: B,
}

#[derive(Clone)]
pub(crate) enum Adapter {
    Java1_16_1(Box<legacy::Bot>),
    Java1_21_11(crate::versions::java_1_21_11::Bot),
}

/// Version-specific access returned by [`Client::native`].
pub enum Native<'a> {
    /// The established Java 1.16.1 API.
    Java1_16_1(&'a legacy::Bot),
    /// Java 1.21.11 reconstruction, unrestricted operations and the checked contract.
    Java1_21_11(crate::versions::java_1_21_11::NativeClient),
}

fn wrong_version(expected: MinecraftVersion) -> Error {
    Error::new(
        ErrorKind::Unsupported,
        anyhow::anyhow!(
            "this client does not use the Java {} adapter",
            expected.name()
        ),
    )
}

/// A client whose protocol, registry and behavior belong to one version adapter.
#[derive(Clone)]
pub struct Client {
    pub(crate) adapter: Adapter,
}

impl Client {
    #[cfg(test)]
    pub(crate) fn from_java_1_16_1(bot: legacy::Bot) -> Self {
        Self {
            adapter: Adapter::Java1_16_1(Box::new(bot)),
        }
    }
    /// Common static implementation support, separate from live permissions.
    pub fn capabilities(&self) -> crate::client::Capabilities {
        crate::client::Capabilities::for_version(self.version())
    }
    /// Registry bound to this client's immutable selected version.
    pub fn registry(&self) -> crate::client::registry::Registry {
        crate::client::registry::Registry::for_version(self.version())
    }
    /// Capture registries and tags actually received on this connection.
    /// Static bundled registry IDs and server-assigned configuration IDs are distinct.
    ///
    /// ```no_run
    /// use voxrig::client::prelude::*;
    /// async fn inspect(client: &Client) -> Result<()> {
    ///     let state = client.server_registry_state().await?;
    ///     if client.version() == MinecraftVersion::Java1_21_11 {
    ///         let id = state.find("minecraft:enchantment", "minecraft:unbreaking")?;
    ///         assert_eq!(state.resolve(&id)?.name, "minecraft:unbreaking");
    ///     }
    ///     Ok(())
    /// }
    /// ```
    pub async fn server_registry_state(
        &self,
    ) -> Result<crate::client::registry::ServerRegistryObservation> {
        crate::client::dispatch!(&self.adapter, a => CoreOps::server_registry_state(a).await)
    }
    /// Read the UUID/name actually received in LOGIN_SUCCESS, with this session stamp.
    /// Profile identity alone does not prove old-player retirement or recovery.
    pub async fn connection_identity(&self) -> Result<crate::client::ConnectionIdentity> {
        crate::client::dispatch!(&self.adapter, a => CoreOps::connection_identity(a).await)
    }
    /// Capture received entity spawns which have not been removed in this world.
    /// Coordinates retain their original spawn ordinal; current movement, metadata
    /// and hitboxes are not inferred. Opaque targets are rechecked before dispatch.
    pub async fn entity_spawns(&self) -> Result<crate::client::EntitySpawns> {
        crate::client::dispatch!(&self.adapter, a => CoreOps::entity_spawns(a).await)
    }
    /// Capture the latest received motion fields for an original spawn lifetime.
    /// Packet targets and velocity samples do not establish current native physics.
    pub async fn entity_motion(
        &self,
        target: crate::client::EntityId,
    ) -> Result<crate::client::EntityMotionObservation> {
        crate::client::dispatch!(&self.adapter, a => CoreOps::entity_motion(a, target).await)
    }
    /// Capture actual own-player passenger relationships. Before an applicable
    /// receipt, the relationship is unknown. A received dismount does not prove
    /// default stationary motion or authorize a ground operation.
    pub async fn vehicle_state(&self) -> Result<crate::client::VehicleObservation> {
        crate::client::dispatch!(&self.adapter, a => CoreOps::vehicle_state(a).await)
    }
    /// Survival-mode handle available on each adapter. Does not change game mode.
    pub fn survival(&self) -> crate::client::Survival {
        crate::client::Survival {
            client: self.clone(),
        }
    }
    /// Creative-mode handle available on each adapter. Does not grant creative permission.
    pub fn creative(&self) -> crate::client::Creative {
        crate::client::Creative {
            client: self.clone(),
        }
    }
    /// Common player and received inventory captured under one adapter lock boundary.
    pub async fn player_state(&self) -> Result<crate::client::PlayerObservation> {
        crate::client::dispatch!(&self.adapter, a => CoreOps::player_state(a).await)
    }
    /// Capture actual inventory receipts and their registry owner together.
    /// Retain this immutable value when inspecting server-assigned IDs in item
    /// data; separately captured registries can belong to a later configuration.
    /// Local inventory predictions are excluded.
    ///
    /// ```no_run
    /// use voxrig::client::prelude::*;
    /// async fn inspect(client: &Client) -> Result<()> {
    ///     let inventory = client.received_inventory().await?;
    ///     if let Some(item) = inventory.slot(9)?.and_then(ReceivedSlot::item) {
    ///         let id = item.registry_state().find_entry("minecraft:item", &item.stack().name)?;
    ///         println!("{}: {}", item.registry_state().entry_name(&id)?, item.stack().count);
    ///     }
    ///     Ok(())
    /// }
    /// ```
    pub async fn received_inventory(&self) -> Result<crate::client::ReceivedInventory> {
        crate::client::dispatch!(&self.adapter, a => CoreOps::received_inventory(a).await)
    }
    /// Capture the actual received recipe catalogue and its registry/tag owner.
    /// Displays and book membership do not predict inventory or authorize crafting.
    pub async fn received_recipes(&self) -> Result<crate::client::ReceivedRecipes> {
        crate::client::dispatch!(&self.adapter, a => CoreOps::received_recipes(a).await)
    }

    /// Last actual ghost UI packet with its original opening and frozen context.
    /// It is display evidence, not received ingredients or crafted output.
    pub async fn received_recipe_ghost(
        &self,
    ) -> Result<Option<crate::client::ReceivedRecipeGhost>> {
        crate::client::dispatch!(&self.adapter, a => CoreOps::received_recipe_ghost(a).await)
    }
    /// Capture recipes/tags, player inventory and the active player/table grid
    /// together. Other active UIs return None. This is read-only planning data,
    /// not permission to mutate the grid or evidence of recipe consumption.
    pub async fn received_crafting_context(
        &self,
    ) -> Result<Option<crate::client::ReceivedCraftingContext>> {
        crate::client::dispatch!(&self.adapter, a => CoreOps::received_crafting_context(a).await)
    }

    /// Inventory-only recipe-book material assignment and bounded maximum.
    /// Captures actual recipes/tags and main/hotbar receipts at one adapter boundary.
    /// Positive native batch sizes are required. Missing stock/requirements are errors.
    /// This excludes grid/cursor stock and is not a placement or consumption plan.
    pub async fn recipe_book_materials(
        &self,
        recipe: &crate::client::RecipeId,
        crafts: u32,
        maximum_bound: u32,
    ) -> Result<crate::client::RecipeBookMaterials> {
        crate::client::dispatch!(&self.adapter, a => CoreOps::recipe_book_materials(a, recipe, crafts, maximum_bound).await)
    }

    /// Actual player/table crafting inputs and displayed result, with native
    /// topology and registry ownership captured together. Other active UIs
    /// return None. Missing receipts do not become empty ingredients.
    ///
    /// ```no_run
    /// use voxrig::client::prelude::*;
    /// async fn inspect(client: &Client) -> Result<()> {
    ///     if let Some(grid) = client.received_crafting().await? {
    ///         let [width, height] = grid.dimensions();
    ///         for y in 0..height {
    ///             for x in 0..width {
    ///                 if let Some(input) = grid.input(x, y)? {
    ///                     println!("({x},{y}): {:?}", input.value());
    ///                 }
    ///             }
    ///         }
    ///     }
    ///     Ok(())
    /// }
    /// ```
    pub async fn received_crafting(&self) -> Result<Option<crate::client::ReceivedCrafting>> {
        crate::client::dispatch!(&self.adapter, a => CoreOps::received_crafting(a).await)
    }
    /// Actual open-container contents and cursor at one native capture boundary.
    /// Numeric window IDs may be reused; use the session-bound screen identity.
    pub async fn screen_state(&self) -> Result<crate::client::container::ScreenObservation> {
        crate::client::dispatch!(&self.adapter, a => CoreOps::screen_state(a).await)
    }
    /// Capture player, inventory and a received region at one adapter boundary.
    /// Missing data stays unavailable; neither local physics nor a capture is server confirmation.
    pub async fn capture(&self, region: Region) -> Result<crate::client::Capture> {
        region.volume()?;
        crate::client::dispatch!(&self.adapter, a => CoreOps::capture(a, region).await)
    }
    pub(crate) async fn execute(
        &self,
        mode: crate::client::GameMode,
        action: crate::client::operations::Action<'_>,
    ) -> Result<crate::client::DispatchReceipt> {
        let (connection_id, interaction_sequence) = crate::client::dispatch!(&self.adapter, a => (
            SessionOps::connection_id(a),
            CoreOps::execute(a, mode, action).await?,
        ));
        Ok(crate::client::DispatchReceipt {
            version: self.version(),
            connection_id,
            interaction_sequence,
        })
    }
    pub(crate) fn from_java_1_21_11(bot: crate::versions::java_1_21_11::Bot) -> Self {
        Self {
            adapter: Adapter::Java1_21_11(bot),
        }
    }
    /// Version-specific functionality outside the common API.
    pub fn native(&self) -> Native<'_> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => Native::Java1_16_1(bot),
            Adapter::Java1_21_11(bot) => Native::Java1_21_11(
                crate::versions::java_1_21_11::NativeClient::new(self.clone(), bot.clone()),
            ),
        }
    }
    /// The Java 1.16.1 `Bot`, or `Unsupported` for other versions.
    pub fn java_1_16_1(&self) -> Result<&legacy::Bot> {
        match self.native() {
            Native::Java1_16_1(bot) => Ok(bot),
            _ => Err(wrong_version(MinecraftVersion::Java1_16_1)),
        }
    }
    /// The Java 1.21.11 native handle, or `Unsupported` for other versions.
    pub fn java_1_21_11(&self) -> Result<crate::versions::java_1_21_11::NativeClient> {
        match self.native() {
            Native::Java1_21_11(native) => Ok(native),
            _ => Err(wrong_version(MinecraftVersion::Java1_21_11)),
        }
    }
    /// Captures exact incoming packets for a bounded diagnostic interval on either version.
    /// Late-start traces retain original ordinals but cannot be replayed without
    /// an initial baseline; use `connect_recorded` for replayable histories.
    pub async fn start_packet_trace(&self, maximum_bytes: usize) -> Result<()> {
        crate::client::dispatch!(&self.adapter, a => SessionOps::start_packet_trace(a, maximum_bytes).await)
    }

    /// Finish exact received evidence. Overflow and original ordinals are retained.
    pub async fn stop_packet_trace(&self) -> Result<crate::client::PacketTrace> {
        crate::client::dispatch!(&self.adapter, a => SessionOps::stop_packet_trace(a).await)
    }

    /// Connects using only the selected version. Unsupported adapters fail before I/O.
    pub async fn connect(config: ConnectionConfig) -> Result<Self> {
        Self::connect_with_packet_trace(config, None).await
    }
    /// Connect with capture enabled before the first configuration/play packet.
    /// Authentication/login packets are excluded. No packet gap is hidden by
    /// starting capture after connect or wait_until_ready.
    pub async fn connect_recorded(config: ConnectionConfig, maximum_bytes: usize) -> Result<Self> {
        crate::client::recording::validate_limit(maximum_bytes)?;
        Self::connect_with_packet_trace(config, Some(maximum_bytes)).await
    }
    async fn connect_with_packet_trace(
        config: ConnectionConfig,
        trace_limit: Option<usize>,
    ) -> Result<Self> {
        config.validate()?;
        match config.version {
            MinecraftVersion::Java1_16_1 => {
                let bot = legacy::Bot::connect_with_packet_trace(
                    config.server,
                    legacy::Player::offline(config.username),
                    Arc::new(legacy::SharedChunkStorage::default()),
                    config.limits.legacy(),
                    trace_limit,
                )
                .await?;
                Ok(Self {
                    adapter: Adapter::Java1_16_1(Box::new(bot)),
                })
            }
            MinecraftVersion::Java1_21_11 => Ok(Self {
                adapter: Adapter::Java1_21_11(
                    crate::versions::java_1_21_11::Bot::connect_with_packet_trace(
                        config,
                        trace_limit,
                    )
                    .await?,
                ),
            }),
        }
    }

    /// Exact version of this connection.
    pub fn version(&self) -> MinecraftVersion {
        crate::client::adapter::version_of(&self.adapter)
    }

    /// Waits for the selected adapter's initial playable state.
    pub async fn wait_until_ready(&self) -> Result<()> {
        crate::client::dispatch!(&self.adapter, a => SessionOps::wait_until_ready(a).await)
    }

    /// Gets all cells under one world lock; this does not send confirmation commands.
    pub async fn observe_region(&self, region: Region) -> Result<Observation> {
        region.volume()?;
        match &self.adapter {
            Adapter::Java1_21_11(bot) => bot.observe_region(region).await,
            Adapter::Java1_16_1(bot) => {
                if region.min[1] < 0 || region.max[1] > 255 {
                    return Err(Error::new(
                        ErrorKind::InvalidInput,
                        anyhow::anyhow!("region is outside Java 1.16.1 dimension height"),
                    ));
                }
                let snapshot = bot.observe_region_snapshot(region).await?;
                let mut blocks = Vec::with_capacity(snapshot.value.len());
                for block in snapshot.value {
                    blocks.push(ObservedBlock {
                        position: [block.x, block.y, block.z],
                        state: block.state_id.map(legacy::native_state).transpose()?,
                    });
                }
                Ok(Observation {
                    version: self.version(),
                    connection_id: bot.connection_id(),
                    revision: snapshot.revision,
                    receive_sequence: None,
                    captured_at: snapshot.captured_at,
                    region,
                    blocks,
                })
            }
        }
    }

    /// Irreversibly fence this transport without waiting for observation, writer
    /// admission, pending writes or normal disconnect cleanup. All clones share
    /// the fence. Native receive tasks are aborted and shutdown is scheduled.
    ///
    /// The returned local fact does not certify transport closure or server-side
    /// stillness. Already admitted writes may have unknown effects. Do not retry
    /// their commands or reuse this Client; retain diagnostics for inspection.
    ///
    /// ```no_run
    /// use voxrig::client::prelude::*;
    /// fn quarantine(client: &Client) -> ConnectionRevocation {
    ///     client.revoke_connection()
    /// }
    /// ```
    #[must_use]
    pub fn revoke_connection(&self) -> crate::client::ConnectionRevocation {
        let connection_id = match &self.adapter {
            Adapter::Java1_16_1(bot) => {
                let revoked = bot.revoke_connection();
                revoked.generation().get()
            }
            Adapter::Java1_21_11(bot) => {
                bot.revoke_connection();
                bot.connection_id()
            }
        };
        crate::client::ConnectionRevocation::new(self.version(), connection_id)
    }

    /// Ends the connection. A clone refers to the same session. This may wait
    /// for cleanup or writer shutdown; use `revoke_connection` for local fencing.
    pub async fn disconnect(&self) -> Result<()> {
        crate::client::dispatch!(&self.adapter, a => SessionOps::disconnect(a).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_volume_is_checked_before_allocation() {
        assert_eq!(
            Region {
                min: [-16, 0, 0],
                max: [15, 1, 0]
            }
            .volume()
            .unwrap(),
            64
        );
        assert!(
            Region {
                min: [1, 0, 0],
                max: [0, 0, 0]
            }
            .volume()
            .is_err()
        );
        assert!(
            Region {
                min: [0, 0, 0],
                max: [1000, 1000, 1000]
            }
            .volume()
            .is_err()
        );
        assert!(
            Region {
                min: [i32::MIN, 0, 0],
                max: [i32::MAX, 0, 0]
            }
            .volume()
            .is_err()
        );
    }

    #[tokio::test]
    async fn invalid_modern_identity_is_rejected_before_network_io() {
        let config = ConnectionConfig::offline(
            legacy::Server::new("invalid.invalid", 1),
            "",
            MinecraftVersion::Java1_21_11,
        );
        let error = match Client::connect(config).await {
            Ok(_) => panic!("invalid identity connected"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }

    #[tokio::test]
    async fn legacy_adapter_binds_snapshot_to_session_and_marks_unloaded_cells() {
        use crate::protocol::{get_varint, read_packet, write_packet};
        use tokio::net::TcpListener;
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let (_, payload) = read_packet(&mut stream, None).await.unwrap();
                assert_eq!(get_varint(&mut payload.as_slice()).unwrap(), 736);
                let (_, login) = read_packet(&mut stream, None).await.unwrap();
                write_packet(
                    &mut stream,
                    None,
                    2,
                    &crate::client::login::test_legacy_success(&login),
                )
                .await
                .unwrap();
                // No chunk arrives; the client must not label missing data as air.
                while read_packet(&mut stream, None).await.is_ok() {}
            }
        });
        let config = ConnectionConfig::offline(
            legacy::Server::new("127.0.0.1", port),
            "Observe",
            MinecraftVersion::Java1_16_1,
        );
        let region = Region {
            min: [-1, 0, -1],
            max: [0, 0, 0],
        };
        let one = Client::connect(config.clone()).await.unwrap();
        assert_eq!(
            crate::versions::java_1_21_11::checked::SurvivalCapabilities::for_version(
                one.version()
            )
            .checked_contract,
            None
        );
        assert!(matches!(one.java_1_21_11(), Err(e) if e.kind() == ErrorKind::Unsupported));
        let identity = one.connection_identity().await.unwrap();
        assert_eq!(identity.uuid, [3; 16]);
        assert_eq!(identity.name, "Observe");
        let first = one.observe_region(region).await.unwrap();
        assert_eq!(first.blocks.len(), 4);
        assert_eq!(identity.session.connection_id, first.connection_id);
        assert!(first.blocks.iter().all(|b| b.state.is_none()));
        assert_eq!(first.version, MinecraftVersion::Java1_16_1);
        assert!(
            one.observe_region(Region {
                min: [0, -1, 0],
                max: [0, 0, 0]
            })
            .await
            .is_err()
        );
        one.disconnect().await.unwrap();
        assert!(one.observe_region(region).await.is_err());
        let two = Client::connect(config).await.unwrap();
        let second = two.observe_region(region).await.unwrap();
        assert_ne!(first.connection_id, second.connection_id);
        two.disconnect().await.unwrap();
        server.await.unwrap();
    }
}
