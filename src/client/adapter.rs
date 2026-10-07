//! The version-independent contract every native adapter implements.
//!
//! [`VersionAdapter`] is the single list of operations `Client` exposes on all
//! versions. Adding a method here forces every adapter to implement it with the
//! same signature; [`dispatch!`] then routes a `Client` call to the selected one.
//! Version-only features stay outside this trait and are reached explicitly.
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
    PlayerObservation, ReceivedInventory, RespawnRecord,
};
use crate::{BlockFace, Region, Result};

/// Declares the trait once and implements it for both adapters by forwarding to
/// their native `common_*` methods. Signature drift is a compile error.
macro_rules! version_adapter {
    (
        $(
            $(#[$meta:meta])*
            fn $name:ident => $native:ident(&self $(, $arg:ident: $ty:ty)* $(,)?) -> $ret:ty;
        )*
        // 1.21.11 methods implemented on `Bot` rather than on `Operations`.
        session {
            $(
                $(#[$smeta:meta])*
                fn $sname:ident => $snative:ident(&self $(, $sarg:ident: $sty:ty)* $(,)?) -> $sret:ty;
            )*
        }
    ) => {
        /// Operations every version adapter provides with identical meaning.
        pub(crate) trait VersionAdapter {
            /// Exact native version of this adapter.
            const VERSION: crate::MinecraftVersion;
            /// Process-local transport identity.
            fn connection_id(&self) -> u64;
            $( $(#[$meta])* async fn $name(&self $(, $arg: $ty)*) -> Result<$ret>; )*
            $( $(#[$smeta])* async fn $sname(&self $(, $sarg: $sty)*) -> Result<$sret>; )*
        }

        impl VersionAdapter for crate::versions::java_1_16_1::Bot {
            const VERSION: crate::MinecraftVersion = crate::MinecraftVersion::Java1_16_1;
            fn connection_id(&self) -> u64 {
                crate::versions::java_1_16_1::Bot::connection_id(self)
            }
            $(
                async fn $name(&self $(, $arg: $ty)*) -> Result<$ret> {
                    crate::versions::java_1_16_1::Bot::$native(self $(, $arg)*).await
                }
            )*
            $(
                async fn $sname(&self $(, $sarg: $sty)*) -> Result<$sret> {
                    crate::versions::java_1_16_1::Bot::$snative(self $(, $sarg)*).await
                }
            )*
        }

        impl VersionAdapter for crate::versions::java_1_21_11::Bot {
            const VERSION: crate::MinecraftVersion = crate::MinecraftVersion::Java1_21_11;
            fn connection_id(&self) -> u64 {
                crate::versions::java_1_21_11::Bot::connection_id(self)
            }
            $(
                async fn $name(&self $(, $arg: $ty)*) -> Result<$ret> {
                    self.operations().$native($($arg),*).await
                }
            )*
            $(
                async fn $sname(&self $(, $sarg: $sty)*) -> Result<$sret> {
                    crate::versions::java_1_21_11::Bot::$snative(self $(, $sarg)*).await
                }
            )*
        }
    };
}

version_adapter! {
    // One ordinary mode-checked action; returns the native interaction sequence.
    fn execute => execute_common(
        &self, mode: GameMode, action: super::operations::Action<'_>,
    ) -> Option<i32>;

    // Session and world observation.
    fn connection_identity => common_connection_identity(&self) -> ConnectionIdentity;
    fn capture => common_capture(&self, region: Region) -> Capture;
    fn player_state => common_player_state(&self) -> PlayerObservation;
    fn respawn => common_respawn(&self) -> RespawnRecord;
    fn entity_spawns => common_entity_spawns(&self) -> EntitySpawns;
    fn entity_motion => common_entity_motion(&self, target: EntityId) -> EntityMotionObservation;
    fn vehicle_state => common_vehicle_state(&self) -> VehicleObservation;

    // Received inventory, screens and recipes.
    fn received_inventory => common_received_inventory(&self) -> ReceivedInventory;
    fn screen_state => common_screen_state(&self) -> ScreenObservation;
    fn received_recipes => common_received_recipes(&self) -> ReceivedRecipes;
    fn received_recipe_ghost => common_received_recipe_ghost(&self) -> Option<ReceivedRecipeGhost>;
    fn received_crafting_context =>
        common_received_crafting_context(&self) -> Option<ReceivedCraftingContext>;
    fn received_crafting => common_received_crafting(&self) -> Option<ReceivedCrafting>;
    fn recipe_book_materials => common_recipe_book_materials(
        &self, recipe: &RecipeId, crafts: u32, maximum_bound: u32,
    ) -> RecipeBookMaterials;

    // Containers.
    fn open_container =>
        common_open_container(&self, mode: GameMode, target: [i32; 3]) -> ContainerOpenRecord;
    fn container_open_record => common_container_open_record(&self) -> Option<ContainerOpenRecord>;
    fn close_container =>
        common_close_container(&self, mode: GameMode, screen: ScreenId) -> ContainerCloseRecord;
    fn container_close_record =>
        common_container_close_record(&self) -> Option<ContainerCloseRecord>;

    // Inventory clicks, transfers and swaps.
    fn click_inventory => common_click_inventory(
        &self, mode: GameMode, source: InventorySource, slot: u16, button: InventoryClickButton,
    ) -> InventoryClickRecord;
    fn inventory_click_record => common_inventory_click_record(&self) -> Option<InventoryClickRecord>;
    fn transfer_inventory => common_transfer_inventory(
        &self, mode: GameMode, source: InventorySource, slot: u16,
    ) -> InventoryTransferRecord;
    fn inventory_transfer_record =>
        common_inventory_transfer_record(&self) -> Option<InventoryTransferRecord>;
    fn swap_hotbar =>
        common_swap_hotbar(&self, mode: GameMode, main: u8, hotbar: u8) -> InventorySwapRecord;
    fn swap_container_hotbar => common_swap_container_hotbar(
        &self, mode: GameMode, screen: ScreenId, slot: u16, hotbar: u8,
    ) -> InventorySwapRecord;
    fn inventory_swap_record => common_inventory_swap_record(&self) -> Option<InventorySwapRecord>;

    // Crafting.
    fn place_recipe =>
        common_place_recipe(&self, mode: GameMode, plan: &RecipePlacementPlan) -> RecipePlacementRecord;
    fn recipe_placement_record =>
        common_recipe_placement_record(&self) -> Option<RecipePlacementRecord>;
    fn take_crafting_result => common_take_crafting_result(
        &self, mode: GameMode, grid: &ReceivedCrafting, destination: CraftingResultDestination,
    ) -> CraftingTakeRecord;
    fn crafting_take_record => common_crafting_take_record(&self) -> Option<CraftingTakeRecord>;

    // Blocks.
    fn target_block =>
        common_target_block(&self, mode: GameMode, distance: f64) -> BlockTargetObservation;
    fn place_cube => common_place_cube(&self, support: [i32; 3], face: BlockFace) -> PlacementRecord;
    fn placement_record => common_placement_record(&self) -> Option<PlacementRecord>;
    fn start_mining => common_start_mining(&self, target: [i32; 3], face: BlockFace) -> MiningRecord;
    fn mining_send => common_mining_send(&self, id: MiningId, action: MiningAction) -> MiningRecord;
    fn mining_record => common_mining_record(&self) -> Option<MiningRecord>;

    // Movement and vehicles.
    fn preview_path =>
        common_preview_path(&self, mode: GameMode, controls: &[SurvivalControl]) -> MotionPreview;
    fn start_predicted_path =>
        common_start_predicted_path(&self, mode: GameMode, controls: &[SurvivalControl]) -> MotionRecord;
    fn motion_record => common_motion_record(&self) -> Option<MotionRecord>;
    fn flight => common_flight(&self, command: FlightCommand) -> FlightRecord;
    fn dismount => common_dismount(&self, mode: GameMode, mount: MountId) -> DismountRecord;
    fn complete_dismount =>
        common_complete_dismount(&self, mode: GameMode, id: DismountId) -> DismountRecord;
    fn resume_ground => common_resume_ground(&self, mode: GameMode, id: DismountId) -> DismountRecord;
    fn dismount_record => common_dismount_record(&self) -> Option<DismountRecord>;
    fn vehicle_control => common_vehicle_control(
        &self, mode: GameMode, mount: MountId, inputs: &[VehicleInput],
    ) -> VehicleControlRecord;
    fn vehicle_control_record => common_vehicle_control_record(&self) -> Option<VehicleControlRecord>;

    session {
        // Connection lifecycle and diagnostics.
        fn wait_until_ready => wait_until_ready(&self) -> ();
        fn disconnect => disconnect(&self) -> ();
        fn start_packet_trace => start_packet_trace(&self, maximum_bytes: usize) -> ();
        fn stop_packet_trace => stop_packet_trace(&self) -> super::PacketTrace;
        // Configuration-phase registries and received UI state.
        fn server_registry_state => common_server_registry_state(&self) -> ServerRegistryObservation;
        fn scoreboard_state => common_scoreboard_state(&self) -> ScoreboardObservation;
        fn boss_bars => common_boss_bars(&self) -> BossBarsObservation;
        fn teams => common_teams(&self) -> TeamsObservation;
        fn player_list => common_player_list(&self) -> PlayerListObservation;
        fn titles => common_titles(&self) -> TitlesObservation;
        fn tab_list => common_tab_list(&self) -> TabListObservation;
        fn world_border => common_world_border(&self) -> WorldBorderObservation;
    }
}

/// Routes one expression to the selected adapter as `&impl VersionAdapter`.
///
/// ```ignore
/// dispatch!(&self.adapter, a => VersionAdapter::player_state(a).await)
/// ```
macro_rules! dispatch {
    ($adapter:expr, $a:ident => $body:expr) => {
        match $adapter {
            $crate::connection::Adapter::Java1_16_1(bot) => {
                let $a: &$crate::versions::java_1_16_1::Bot = bot;
                $body
            }
            $crate::connection::Adapter::Java1_21_11(bot) => {
                let $a: &$crate::versions::java_1_21_11::Bot = bot;
                $body
            }
        }
    };
}
pub(crate) use dispatch;

/// The selected adapter's [`VersionAdapter::VERSION`].
pub(crate) fn version_of(adapter: &crate::connection::Adapter) -> crate::MinecraftVersion {
    fn of<A: VersionAdapter>(_: &A) -> crate::MinecraftVersion {
        A::VERSION
    }
    dispatch!(adapter, a => of(a))
}
