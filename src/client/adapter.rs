//! The version-independent contract every native adapter implements.
//!
//! Each trait groups one concern and is implemented next to the native code for
//! both versions (Java 1.16.1 `Bot` and Java 1.21.11 `Operations`). Diverging
//! signatures fail to compile. [`VersionAdapter`] is the union of all of them,
//! and [`dispatch!`] routes a `Client` call to the selected adapter.
//! Version-only features stay outside these traits and are reached through
//! `Client::native`.
//!
//! Call trait methods as `Trait::method(a, ..)`: several adapters also have
//! inherent methods with the same name and different meaning (for example the
//! Java 1.16.1 `Bot::respawn` or the Java 1.21.11 `Operations::player_state`),
//! and method-call syntax silently prefers those.
use super::container::{ContainerCloseRecord, ContainerOpenRecord, ScreenId, ScreenObservation};
use super::crafting::{
    CraftingResultDestination, CraftingTakeRecord, ReceivedCrafting, ReceivedCraftingContext,
    ReceivedRecipeGhost, ReceivedRecipes, RecipeBookMaterials, RecipeId, RecipePlacementPlan,
    RecipePlacementRecord,
};
use super::flight::{FlightCommand, FlightRecord};
use super::inventory::{
    InventoryClickButton, InventoryClickRecord, InventorySource, InventorySwapRecord,
    InventoryTransferRecord,
};
use super::registry::ServerRegistryObservation;
use super::survival::mining::MiningAction;
use super::survival::{
    BlockTargetObservation, MiningId, MiningRecord, MotionPreview, MotionRecord, PlacementRecord,
    SurvivalControl,
};
use super::ui::{
    BossBarsObservation, PlayerListObservation, ScoreboardObservation, TabListObservation,
    TeamsObservation, TitlesObservation, WorldBorderObservation,
};
use super::vehicle::{
    DismountId, DismountRecord, MountId, VehicleControlRecord, VehicleInput, VehicleObservation,
};
use super::{
    Capture, ConnectionIdentity, EntityId, EntityMotionObservation, EntitySpawns, GameMode,
    PacketTrace, PlayerObservation, ReceivedInventory, RespawnRecord,
};
use crate::versions::{java_1_16_1, java_1_21_11};
use crate::{BlockFace, MinecraftVersion, Region, Result};

/// Connection lifecycle and packet diagnostics.
pub(crate) trait SessionOps {
    /// Exact native version of this adapter.
    const VERSION: MinecraftVersion;
    /// Process-local transport identity.
    fn connection_id(&self) -> u64;
    async fn wait_until_ready(&self) -> Result<()>;
    async fn disconnect(&self) -> Result<()>;
    async fn start_packet_trace(&self, maximum_bytes: usize) -> Result<()>;
    async fn stop_packet_trace(&self) -> Result<PacketTrace>;
}

/// Connection identity, received-state observation and the basic one-shot actions.
pub(crate) trait CoreOps {
    async fn connection_identity(&self) -> Result<ConnectionIdentity>;
    async fn server_registry_state(&self) -> Result<ServerRegistryObservation>;
    async fn player_state(&self) -> Result<PlayerObservation>;
    async fn received_inventory(&self) -> Result<ReceivedInventory>;
    async fn respawn(&self) -> Result<RespawnRecord>;
    async fn entity_spawns(&self) -> Result<EntitySpawns>;
    async fn entity_motion(&self, target: EntityId) -> Result<EntityMotionObservation>;
    async fn vehicle_state(&self) -> Result<VehicleObservation>;
    async fn screen_state(&self) -> Result<ScreenObservation>;
    async fn capture(&self, region: Region) -> Result<Capture>;
    async fn received_recipes(&self) -> Result<ReceivedRecipes>;
    async fn received_recipe_ghost(&self) -> Result<Option<ReceivedRecipeGhost>>;
    async fn received_crafting_context(&self) -> Result<Option<ReceivedCraftingContext>>;
    async fn received_crafting(&self) -> Result<Option<ReceivedCrafting>>;
    async fn recipe_book_materials(
        &self,
        recipe: &RecipeId,
        crafts: u32,
        maximum_bound: u32,
    ) -> Result<RecipeBookMaterials>;
    async fn execute(
        &self,
        mode: GameMode,
        action: super::operations::Action<'_>,
    ) -> Result<Option<i32>>;
}

/// Chat dispatch and received chat history.
pub(crate) trait ChatOps {
    async fn send_chat(&self, message: &str) -> Result<()>;
    async fn send_command(&self, command: &str) -> Result<()>;
    async fn chat_after(&self, cursor: u64) -> Result<super::ChatLog>;
}

/// Received scoreboard, boss bar, team, player-list, title, tab-list and border state.
pub(crate) trait UiOps {
    async fn scoreboard_state(&self) -> Result<ScoreboardObservation>;
    async fn boss_bars(&self) -> Result<BossBarsObservation>;
    async fn teams(&self) -> Result<TeamsObservation>;
    async fn player_list(&self) -> Result<PlayerListObservation>;
    async fn titles(&self) -> Result<TitlesObservation>;
    async fn tab_list(&self) -> Result<TabListObservation>;
    async fn world_border(&self) -> Result<WorldBorderObservation>;
}

/// Opening and closing one audited container screen.
pub(crate) trait ContainerOps {
    async fn open_container(&self, mode: GameMode, target: [i32; 3])
    -> Result<ContainerOpenRecord>;
    async fn container_open_record(&self) -> Result<Option<ContainerOpenRecord>>;
    async fn close_container(
        &self,
        mode: GameMode,
        screen: ScreenId,
    ) -> Result<ContainerCloseRecord>;
    async fn container_close_record(&self) -> Result<Option<ContainerCloseRecord>>;
}

/// Ordinary PICKUP clicks.
pub(crate) trait InventoryClickOps {
    async fn click_inventory(
        &self,
        mode: GameMode,
        source: InventorySource,
        slot: u16,
        button: InventoryClickButton,
    ) -> Result<InventoryClickRecord>;
    async fn inventory_click_record(&self) -> Result<Option<InventoryClickRecord>>;
}

/// Hotbar swaps in the player screen or an open container.
pub(crate) trait InventorySwapOps {
    async fn swap_hotbar(
        &self,
        mode: GameMode,
        main: u8,
        hotbar: u8,
    ) -> Result<InventorySwapRecord>;
    async fn swap_container_hotbar(
        &self,
        mode: GameMode,
        screen: ScreenId,
        slot: u16,
        hotbar: u8,
    ) -> Result<InventorySwapRecord>;
    async fn inventory_swap_record(&self) -> Result<Option<InventorySwapRecord>>;
}

/// Ordinary QUICK_MOVE transfers.
pub(crate) trait InventoryTransferOps {
    async fn transfer_inventory(
        &self,
        mode: GameMode,
        source: InventorySource,
        slot: u16,
    ) -> Result<InventoryTransferRecord>;
    async fn inventory_transfer_record(&self) -> Result<Option<InventoryTransferRecord>>;
}

/// Taking a crafting result.
pub(crate) trait CraftingTakeOps {
    async fn take_crafting_result(
        &self,
        mode: GameMode,
        grid: &ReceivedCrafting,
        destination: CraftingResultDestination,
    ) -> Result<CraftingTakeRecord>;
    async fn crafting_take_record(&self) -> Result<Option<CraftingTakeRecord>>;
}

/// Recipe-book placement into a crafting grid.
pub(crate) trait RecipePlacementOps {
    async fn place_recipe(
        &self,
        mode: GameMode,
        plan: &RecipePlacementPlan,
    ) -> Result<RecipePlacementRecord>;
    async fn recipe_placement_record(&self) -> Result<Option<RecipePlacementRecord>>;
}

/// Retained mining with explicit start, finish and abort.
pub(crate) trait MiningOps {
    async fn start_mining(&self, target: [i32; 3], face: BlockFace) -> Result<MiningRecord>;
    async fn mining_send(&self, id: MiningId, action: MiningAction) -> Result<MiningRecord>;
    async fn mining_record(&self) -> Result<Option<MiningRecord>>;
}

/// One-shot passive cube placement.
pub(crate) trait PlacementOps {
    async fn place_cube(&self, support: [i32; 3], face: BlockFace) -> Result<PlacementRecord>;
    async fn placement_record(&self) -> Result<Option<PlacementRecord>>;
}

/// Read-only queries from a dry standing pose: block target and path preview.
pub(crate) trait StandingQueryOps {
    async fn target_block(&self, mode: GameMode, distance: f64) -> Result<BlockTargetObservation>;
    async fn preview_path(
        &self,
        mode: GameMode,
        controls: &[SurvivalControl],
    ) -> Result<MotionPreview>;
}

/// Finite predicted walking/jump paths.
pub(crate) trait PathMotionOps {
    async fn start_predicted_path(
        &self,
        mode: GameMode,
        controls: &[SurvivalControl],
    ) -> Result<MotionRecord>;
    async fn motion_record(&self) -> Result<Option<MotionRecord>>;
}

/// Creative flight commands.
pub(crate) trait FlightOps {
    async fn flight(&self, command: FlightCommand) -> Result<FlightRecord>;
}

/// Dismount, mounted input and ground continuation.
pub(crate) trait VehicleOps {
    async fn dismount(&self, mode: GameMode, mount: MountId) -> Result<DismountRecord>;
    async fn complete_dismount(&self, mode: GameMode, id: DismountId) -> Result<DismountRecord>;
    async fn resume_ground(&self, mode: GameMode, id: DismountId) -> Result<DismountRecord>;
    async fn dismount_record(&self) -> Result<Option<DismountRecord>>;
    async fn vehicle_control(
        &self,
        mode: GameMode,
        mount: MountId,
        inputs: &[VehicleInput],
    ) -> Result<VehicleControlRecord>;
    async fn vehicle_control_record(&self) -> Result<Option<VehicleControlRecord>>;
}

/// Every cross-version operation. Implemented automatically for any type that
/// implements all of the concern traits.
pub(crate) trait VersionAdapter:
    SessionOps
    + CoreOps
    + UiOps
    + ChatOps
    + ContainerOps
    + InventoryClickOps
    + InventorySwapOps
    + InventoryTransferOps
    + CraftingTakeOps
    + RecipePlacementOps
    + MiningOps
    + PlacementOps
    + StandingQueryOps
    + PathMotionOps
    + FlightOps
    + VehicleOps
{
}
impl<T> VersionAdapter for T where
    T: SessionOps
        + CoreOps
        + UiOps
        + ChatOps
        + ContainerOps
        + InventoryClickOps
        + InventorySwapOps
        + InventoryTransferOps
        + CraftingTakeOps
        + RecipePlacementOps
        + MiningOps
        + PlacementOps
        + StandingQueryOps
        + PathMotionOps
        + FlightOps
        + VehicleOps
{
}

// Lifecycle methods are the adapters' own public API; these impls only expose them.
impl SessionOps for java_1_16_1::Bot {
    const VERSION: MinecraftVersion = MinecraftVersion::Java1_16_1;
    fn connection_id(&self) -> u64 {
        java_1_16_1::Bot::connection_id(self)
    }
    async fn wait_until_ready(&self) -> Result<()> {
        java_1_16_1::Bot::wait_until_ready(self).await
    }
    async fn disconnect(&self) -> Result<()> {
        java_1_16_1::Bot::disconnect(self).await
    }
    async fn start_packet_trace(&self, maximum_bytes: usize) -> Result<()> {
        java_1_16_1::Bot::start_packet_trace(self, maximum_bytes).await
    }
    async fn stop_packet_trace(&self) -> Result<PacketTrace> {
        java_1_16_1::Bot::stop_packet_trace(self).await
    }
}
impl SessionOps for java_1_21_11::operations::Operations {
    const VERSION: MinecraftVersion = MinecraftVersion::Java1_21_11;
    fn connection_id(&self) -> u64 {
        self.bot().connection_id()
    }
    async fn wait_until_ready(&self) -> Result<()> {
        self.bot().wait_until_ready().await
    }
    async fn disconnect(&self) -> Result<()> {
        self.bot().disconnect().await
    }
    async fn start_packet_trace(&self, maximum_bytes: usize) -> Result<()> {
        self.bot().start_packet_trace(maximum_bytes).await
    }
    async fn stop_packet_trace(&self) -> Result<PacketTrace> {
        self.bot().stop_packet_trace().await
    }
}

/// Routes one expression to the selected adapter as `&impl VersionAdapter`.
///
/// ```ignore
/// dispatch!(&self.adapter, a => CoreOps::player_state(a).await)
/// ```
macro_rules! dispatch {
    ($adapter:expr, $a:ident => $body:expr) => {
        match $adapter {
            $crate::connection::Adapter::Java1_16_1(bot) => {
                let $a: &$crate::versions::java_1_16_1::Bot = bot;
                $body
            }
            $crate::connection::Adapter::Java1_21_11(bot) => {
                let operations = bot.operations();
                let $a: &$crate::versions::java_1_21_11::operations::Operations = &operations;
                $body
            }
        }
    };
}
pub(crate) use dispatch;

/// The selected adapter's [`SessionOps::VERSION`].
pub(crate) fn version_of(adapter: &crate::connection::Adapter) -> MinecraftVersion {
    fn of<A: VersionAdapter>(_: &A) -> MinecraftVersion {
        A::VERSION
    }
    dispatch!(adapter, a => of(a))
}
