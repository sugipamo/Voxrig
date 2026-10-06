//! Version-selected client API. Local observations never claim server confirmation.

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
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
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
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_server_registry_state().await,
            Adapter::Java1_21_11(bot) => bot.common_server_registry_state().await,
        }
    }
    /// Read the UUID/name actually received in LOGIN_SUCCESS, with this session stamp.
    /// Profile identity alone does not prove old-player retirement or recovery.
    pub async fn connection_identity(&self) -> Result<crate::client::ConnectionIdentity> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_connection_identity().await,
            Adapter::Java1_21_11(bot) => bot.operations().common_connection_identity().await,
        }
    }
    /// Capture received entity spawns which have not been removed in this world.
    /// Coordinates retain their original spawn ordinal; current movement, metadata
    /// and hitboxes are not inferred. Opaque targets are rechecked before dispatch.
    pub async fn entity_spawns(&self) -> Result<crate::client::EntitySpawns> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_entity_spawns().await,
            Adapter::Java1_21_11(bot) => bot.operations().common_entity_spawns().await,
        }
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
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_player_state().await,
            Adapter::Java1_21_11(bot) => bot.operations().common_player_state().await,
        }
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
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_received_inventory().await,
            Adapter::Java1_21_11(bot) => bot.operations().common_received_inventory().await,
        }
    }
    /// Capture the actual received recipe catalogue and its registry/tag owner.
    /// Displays and book membership do not predict inventory or authorize crafting.
    pub async fn received_recipes(&self) -> Result<crate::client::ReceivedRecipes> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_received_recipes().await,
            Adapter::Java1_21_11(bot) => bot.operations().common_received_recipes().await,
        }
    }

    /// Capture recipes/tags, player inventory and the active player/table grid
    /// together. Other active UIs return None. This is read-only planning data,
    /// not permission to mutate the grid or evidence of recipe consumption.
    pub async fn received_crafting_context(
        &self,
    ) -> Result<Option<crate::client::ReceivedCraftingContext>> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_received_crafting_context().await,
            Adapter::Java1_21_11(bot) => bot.operations().common_received_crafting_context().await,
        }
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
        match &self.adapter {
            Adapter::Java1_16_1(bot) => {
                bot.common_recipe_book_materials(recipe, crafts, maximum_bound)
                    .await
            }
            Adapter::Java1_21_11(bot) => {
                bot.operations()
                    .common_recipe_book_materials(recipe, crafts, maximum_bound)
                    .await
            }
        }
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
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_received_crafting().await,
            Adapter::Java1_21_11(bot) => bot.operations().common_received_crafting().await,
        }
    }
    /// Actual open-container contents and cursor at one native capture boundary.
    /// Numeric window IDs may be reused; use the session-bound screen identity.
    pub async fn screen_state(&self) -> Result<crate::client::container::ScreenObservation> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_screen_state().await,
            Adapter::Java1_21_11(bot) => bot.operations().common_screen_state().await,
        }
    }
    /// Capture player, inventory and a received region at one adapter boundary.
    /// Missing data stays unavailable; neither local physics nor a capture is server confirmation.
    pub async fn capture(&self, region: Region) -> Result<crate::client::Capture> {
        region.volume()?;
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_capture(region).await,
            Adapter::Java1_21_11(bot) => bot.operations().common_capture(region).await,
        }
    }
    pub(crate) async fn common_open_container(
        &self,
        mode: crate::client::GameMode,
        target: [i32; 3],
    ) -> Result<crate::client::container::ContainerOpenRecord> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_open_container(mode, target).await,
            Adapter::Java1_21_11(bot) => bot.operations().common_open_container(mode, target).await,
        }
    }
    pub(crate) async fn common_container_open_record(
        &self,
    ) -> Result<Option<crate::client::container::ContainerOpenRecord>> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_container_open_record().await,
            Adapter::Java1_21_11(bot) => bot.operations().common_container_open_record().await,
        }
    }
    pub(crate) async fn common_close_container(
        &self,
        mode: crate::client::GameMode,
        screen: crate::client::container::ScreenId,
    ) -> Result<crate::client::container::ContainerCloseRecord> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_close_container(mode, screen).await,
            Adapter::Java1_21_11(bot) => {
                bot.operations().common_close_container(mode, screen).await
            }
        }
    }
    pub(crate) async fn common_container_close_record(
        &self,
    ) -> Result<Option<crate::client::container::ContainerCloseRecord>> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_container_close_record().await,
            Adapter::Java1_21_11(bot) => bot.operations().common_container_close_record().await,
        }
    }
    pub(crate) async fn common_click_inventory(
        &self,
        mode: crate::client::GameMode,
        source: crate::client::inventory::InventoryClickSource,
        slot: u16,
        button: crate::client::inventory::InventoryClickButton,
    ) -> Result<crate::client::inventory::InventoryClickRecord> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => {
                bot.common_click_inventory(mode, source, slot, button).await
            }
            Adapter::Java1_21_11(bot) => {
                bot.operations()
                    .common_click_inventory(mode, source, slot, button)
                    .await
            }
        }
    }
    pub(crate) async fn common_take_crafting_result(
        &self,
        mode: crate::client::GameMode,
        grid: &crate::client::crafting::ReceivedCrafting,
    ) -> Result<crate::client::crafting::CraftingTakeRecord> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_take_crafting_result(mode, grid).await,
            Adapter::Java1_21_11(bot) => {
                bot.operations()
                    .common_take_crafting_result(mode, grid)
                    .await
            }
        }
    }
    pub(crate) async fn common_crafting_take_record(
        &self,
    ) -> Result<Option<crate::client::crafting::CraftingTakeRecord>> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_crafting_take_record().await,
            Adapter::Java1_21_11(bot) => bot.operations().common_crafting_take_record().await,
        }
    }
    pub(crate) async fn common_inventory_click_record(
        &self,
    ) -> Result<Option<crate::client::inventory::InventoryClickRecord>> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_inventory_click_record().await,
            Adapter::Java1_21_11(bot) => bot.operations().common_inventory_click_record().await,
        }
    }
    pub(crate) async fn common_transfer_inventory(
        &self,
        mode: crate::client::GameMode,
        source: crate::client::inventory::InventorySource,
        slot: u16,
    ) -> Result<crate::client::inventory::InventoryTransferRecord> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_transfer_inventory(mode, source, slot).await,
            Adapter::Java1_21_11(bot) => {
                bot.operations()
                    .common_transfer_inventory(mode, source, slot)
                    .await
            }
        }
    }
    pub(crate) async fn common_inventory_transfer_record(
        &self,
    ) -> Result<Option<crate::client::inventory::InventoryTransferRecord>> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_inventory_transfer_record().await,
            Adapter::Java1_21_11(bot) => bot.operations().common_inventory_transfer_record().await,
        }
    }
    pub(crate) async fn common_swap_hotbar(
        &self,
        mode: crate::client::GameMode,
        main: u8,
        hotbar: u8,
    ) -> Result<crate::client::inventory::InventorySwapRecord> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_swap_hotbar(mode, main, hotbar).await,
            Adapter::Java1_21_11(bot) => {
                bot.operations()
                    .common_swap_hotbar(mode, main, hotbar)
                    .await
            }
        }
    }
    pub(crate) async fn common_swap_container_hotbar(
        &self,
        mode: crate::client::GameMode,
        screen: crate::client::container::ScreenId,
        slot: u16,
        hotbar: u8,
    ) -> Result<crate::client::inventory::InventorySwapRecord> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => {
                bot.common_swap_container_hotbar(mode, screen, slot, hotbar)
                    .await
            }
            Adapter::Java1_21_11(bot) => {
                bot.operations()
                    .common_swap_container_hotbar(mode, screen, slot, hotbar)
                    .await
            }
        }
    }
    pub(crate) async fn common_inventory_swap_record(
        &self,
    ) -> Result<Option<crate::client::inventory::InventorySwapRecord>> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_inventory_swap_record().await,
            Adapter::Java1_21_11(bot) => bot.operations().common_inventory_swap_record().await,
        }
    }
    pub(crate) async fn place_common_cube(
        &self,
        support: [i32; 3],
        face: crate::BlockFace,
    ) -> Result<crate::client::survival::PlacementRecord> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_place_cube(support, face).await,
            Adapter::Java1_21_11(bot) => bot.operations().common_place_cube(support, face).await,
        }
    }
    pub(crate) async fn common_placement_record(
        &self,
    ) -> Result<Option<crate::client::survival::PlacementRecord>> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_placement_record().await,
            Adapter::Java1_21_11(bot) => bot.operations().common_placement_record().await,
        }
    }
    pub(crate) async fn start_common_mining(
        &self,
        target: [i32; 3],
        face: crate::BlockFace,
    ) -> Result<crate::client::survival::MiningRecord> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_start_mining(target, face).await,
            Adapter::Java1_21_11(bot) => bot.operations().common_start_mining(target, face).await,
        }
    }
    pub(crate) async fn send_common_mining(
        &self,
        id: crate::client::survival::MiningId,
        action: crate::client::survival::MiningAction,
    ) -> Result<crate::client::survival::MiningRecord> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_mining_send(id, action).await,
            Adapter::Java1_21_11(bot) => bot.operations().common_mining_send(id, action).await,
        }
    }
    pub(crate) async fn common_mining_record(
        &self,
    ) -> Result<Option<crate::client::survival::MiningRecord>> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_mining_record().await,
            Adapter::Java1_21_11(bot) => bot.operations().common_mining_record().await,
        }
    }
    pub(crate) async fn common_block_target(
        &self,
        mode: crate::client::GameMode,
        distance: f64,
    ) -> Result<crate::client::survival::BlockTargetObservation> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_target_block(mode, distance).await,
            Adapter::Java1_21_11(bot) => bot.operations().common_target_block(mode, distance).await,
        }
    }
    pub(crate) async fn preview_motion_path(
        &self,
        mode: crate::client::GameMode,
        controls: &[crate::client::survival::SurvivalControl],
    ) -> Result<crate::client::survival::MotionPreview> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_preview_path(mode, controls).await,
            Adapter::Java1_21_11(bot) => bot.operations().common_preview_path(mode, controls).await,
        }
    }
    pub(crate) async fn start_predicted_motion_path(
        &self,
        mode: crate::client::GameMode,
        controls: &[crate::client::survival::SurvivalControl],
    ) -> Result<crate::client::survival::MotionRecord> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_start_predicted_path(mode, controls).await,
            Adapter::Java1_21_11(bot) => {
                bot.operations()
                    .common_start_predicted_path(mode, controls)
                    .await
            }
        }
    }
    pub(crate) async fn survival_motion_record(
        &self,
    ) -> Result<Option<crate::client::survival::MotionRecord>> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_motion_record().await,
            Adapter::Java1_21_11(bot) => bot.operations().common_motion_record().await,
        }
    }
    pub(crate) async fn execute(
        &self,
        mode: crate::client::GameMode,
        action: crate::client::operations::Action<'_>,
    ) -> Result<crate::client::DispatchReceipt> {
        let (connection_id, interaction_sequence) = match &self.adapter {
            Adapter::Java1_16_1(bot) => {
                (bot.connection_id(), bot.execute_common(mode, action).await?)
            }
            Adapter::Java1_21_11(bot) => (
                bot.connection_id(),
                bot.operations().execute_common(mode, action).await?,
            ),
        };
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
    /// Version-specific modern controls. The established 1.16.1 Bot API coexists.
    pub fn java_1_21_11_operations(
        &self,
    ) -> Result<crate::versions::java_1_21_11::operations::Operations> {
        match &self.adapter {
            Adapter::Java1_21_11(bot) => Ok(bot.operations()),
            Adapter::Java1_16_1(_) => Err(Error::new(
                ErrorKind::Unsupported,
                anyhow::anyhow!("Java 1.21.11 operations require that version adapter"),
            )),
        }
    }
    /// Returns the Java 1.21.11 client reconstruction alongside the unchanged received cache.
    /// Inspect both the typed issue and cell availability; this is not server confirmation.
    pub async fn observe_client_region(
        &self,
        region: Region,
    ) -> Result<crate::versions::java_1_21_11::reconstruction::ClientObservation> {
        match &self.adapter {
            Adapter::Java1_21_11(bot) => bot.observe_client_region(region).await,
            Adapter::Java1_16_1(_) => Err(Error::new(
                ErrorKind::Unsupported,
                anyhow::anyhow!("client piston reconstruction is not implemented for Java 1.16.1"),
            )),
        }
    }
    /// Shares immutable region cells within one receive/reconstruction generation.
    /// Every call still captures a new receive boundary and local frame.
    pub async fn observe_shared_client_region(
        &self,
        region: Region,
    ) -> Result<crate::versions::java_1_21_11::reconstruction::SharedClientRegion> {
        match &self.adapter {
            Adapter::Java1_21_11(bot) => bot.observe_shared_client_region(region).await,
            Adapter::Java1_16_1(_) => Err(Error::new(
                ErrorKind::Unsupported,
                anyhow::anyhow!("client piston reconstruction is not implemented for Java 1.16.1"),
            )),
        }
    }
    /// Captures exact incoming packets for a bounded diagnostic interval.
    pub async fn start_packet_trace(&self, maximum_bytes: usize) -> Result<()> {
        match &self.adapter {
            Adapter::Java1_21_11(bot) => bot.start_packet_trace(maximum_bytes).await,
            Adapter::Java1_16_1(_) => Err(Error::new(
                ErrorKind::Unsupported,
                anyhow::anyhow!(
                    "packet capture is not implemented for the 1.16.1 compatibility adapter"
                ),
            )),
        }
    }

    /// Finishes a diagnostic capture; incomplete captures are explicitly marked.
    pub async fn stop_packet_trace(&self) -> Result<crate::versions::java_1_21_11::PacketTrace> {
        match &self.adapter {
            Adapter::Java1_21_11(bot) => bot.stop_packet_trace().await,
            Adapter::Java1_16_1(_) => Err(Error::new(
                ErrorKind::Unsupported,
                anyhow::anyhow!(
                    "packet capture is not implemented for the 1.16.1 compatibility adapter"
                ),
            )),
        }
    }

    /// Sends an ordinary use-on-block interaction. Dispatch is not acceptance.
    pub async fn interact_block(&self, position: [i32; 3], face: crate::BlockFace) -> Result<()> {
        match &self.adapter {
            Adapter::Java1_21_11(bot) => bot.interact_block(position, face).await,
            Adapter::Java1_16_1(bot) => {
                bot.place_block(
                    legacy::Hand::Main,
                    legacy::BlockPos {
                        x: position[0],
                        y: position[1],
                        z: position[2],
                    },
                    face,
                    [0.5; 3],
                    false,
                )
                .await
            }
        }
    }

    /// Connects using only the selected version. Unsupported adapters fail before I/O.
    pub async fn connect(config: ConnectionConfig) -> Result<Self> {
        config.validate()?;
        match config.version {
            MinecraftVersion::Java1_16_1 => {
                let bot = legacy::Bot::connect(
                    config.server,
                    legacy::Player::offline(config.username),
                    Arc::new(legacy::SharedChunkStorage::default()),
                    config.limits.legacy(),
                )
                .await?;
                Ok(Self {
                    adapter: Adapter::Java1_16_1(Box::new(bot)),
                })
            }
            MinecraftVersion::Java1_21_11 => Ok(Self {
                adapter: Adapter::Java1_21_11(
                    crate::versions::java_1_21_11::Bot::connect(config).await?,
                ),
            }),
        }
    }

    /// Exact version of this connection.
    pub fn version(&self) -> MinecraftVersion {
        match self.adapter {
            Adapter::Java1_16_1(_) => MinecraftVersion::Java1_16_1,
            Adapter::Java1_21_11(_) => MinecraftVersion::Java1_21_11,
        }
    }

    /// Waits for the selected adapter's initial playable state.
    pub async fn wait_until_ready(&self) -> Result<()> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.wait_until_ready().await,
            Adapter::Java1_21_11(bot) => bot.wait_until_ready().await,
        }
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

    /// Ends the connection. A clone refers to the same session.
    pub async fn disconnect(&self) -> Result<()> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.disconnect().await,
            Adapter::Java1_21_11(bot) => bot.disconnect().await,
        }
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
        assert_eq!(one.survival_capabilities().checked_contract, None);
        assert!(matches!(one.checked_survival(), Err(e) if e.kind() == ErrorKind::Unsupported));
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
