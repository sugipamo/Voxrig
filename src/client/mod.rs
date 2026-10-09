//! Common client entry points and data. Wire formats and physics belong to adapters.
pub(crate) mod adapter;
pub(crate) use adapter::dispatch;
pub mod blocks;
pub use blocks::{BlockRaycast, BlockRaycastObservation, BlockSearch};
pub(crate) mod books;
pub mod chat;
pub mod events;
pub use events::{ClientEvent, EventKind, EventLog};
mod wait;
pub use chat::{ChatKind, ChatLog, ChatText, ReceivedChat};
mod capabilities;
mod config;
pub(crate) mod constructor;
pub mod container;
pub mod context;
pub mod control;
pub use context::{
    CooldownKey, DeathLocation, DefaultSpawnPosition, Experience, ItemCooldown, LoginConditions,
    PlayerAbilities, PlayerContextObservation, WeatherObservation, WorldDifficulty,
    WorldEntryContext, WorldViewObservation,
};
pub mod crafting;
pub(crate) mod enchantments;
pub(crate) mod entity;
pub use entity::{
    EntityHistory, EntityHistoryCursor, EntityHistoryGap, EntityHistoryKind, EntityHistoryRecord,
    MAX_ENTITY_HISTORY_READ, MAX_ENTITY_HISTORY_RECORDS,
};
pub mod flight;
pub(crate) mod fraction;
pub mod furnace;
pub use flight::{FlightCommand, FlightLanding, FlightRecord, FlightStage};
pub(crate) mod chunk_context;
pub(crate) mod chunks;
mod dig;
pub use dig::{DigEstimate, DigRecord};
mod geometry;
mod hash_ops;
pub(crate) mod identifier;
pub mod inventory;
mod inventory_helpers;
pub use inventory_helpers::{CraftOnceRecord, CraftingFillRecord};
mod item;
pub(crate) mod item_components;
pub(crate) mod item_constructor;
pub(crate) mod item_semantics;
pub(crate) mod item_use;
mod placement_check;
mod player_control;
pub(crate) mod player_facts;
pub use chunk_context::{
    BiomeVolume, BlockEntityKind, ChunkContextObservation, ChunkIdentity, HeightmapKind,
    ReceivedBlockEntity, ReceivedHeightmap,
};
pub use chunks::{ChunkObservation, LoadedChunks};
pub use placement_check::PlacementCheck;
pub use player_control::PlayerControl;
pub use player_facts::{PlayerAttribute, PlayerEffect};
mod lifecycle;
pub use lifecycle::{ConnectionRevocation, ConnectionStatus};
pub(crate) mod login;
pub use furnace::{FurnaceObservation, FurnaceSlot};
pub mod manager;
pub mod maps;
pub use manager::ClientManager;
pub use maps::{MAX_RECEIVED_MAPS, MapIconKind, MapIdentity, MapObservation, ReceivedMapIcon};
pub mod nbt;
pub(crate) mod observation;
pub(crate) mod operations;
pub(crate) mod profile;
mod received_items;
pub mod recording;
pub mod respawn;
pub use respawn::{RespawnRecord, RespawnStage};
pub(crate) mod physics;
pub mod registry;
pub(crate) mod selector;
pub(crate) mod selector_snbt;
pub mod survival;
#[cfg(test)]
pub(crate) mod tests;
pub(crate) mod text;
pub mod ui;
pub(crate) mod uri;
pub(crate) mod uuid;
pub mod vehicle;
pub use crate::connection::{Client, ConnectionConfig, Observation, ObservedBlock, Region};
pub use capabilities::{Capabilities, Feature, Restriction, Support};
pub use config::{ClientLimits, Server};
pub use crafting::{
    CraftingGridReturnPlan, CraftingGridReturnStep, CraftingGridUnreturnedSplit,
    CraftingResultDestination, CraftingSource, CraftingTakeId, CraftingTakeRecord,
    CraftingTakeStage, ReceivedCrafting, ReceivedCraftingContext, ReceivedRecipe,
    ReceivedRecipeGhost, ReceivedRecipes, RecipeBookMaterials, RecipeBookStock, RecipeCraftingCell,
    RecipeCraftingLayout, RecipeDisplay, RecipeId, RecipeIngredient, RecipePlacementAmount,
    RecipePlacementId, RecipePlacementPlan, RecipePlacementRecord, RecipePlacementSend,
    RecipePlacementStage, RecipeSlotDisplay, RecipeTrimDefinition, RecipeTrimPattern,
};
pub use entity::{
    EntitiesObservation, EntityDataField, EntityDataReading, EntityDataSource, EntityDataValue,
    EntityId, EntityMotionObservation, EntityObservation, EntityPartEvidence, EntityPartId,
    EntityPartKind, EntityPartModel, EntityPartObservation, EntityPartState, EntityPartUnavailable,
    EntityPosition, EntityPositionCorrection, EntitySpawn, EntitySpawns, EquipmentSlot,
};
pub use geometry::{Aabb, BlockFace, BlockPos, Hand, Vec3};
pub use item::ItemProperties;
pub(crate) use item::{modern_prototype_components, modern_weight_defaults};
pub use observation::{
    Capture, ConnectionIdentity, Dimension, GameMode, Health, InventoryObservation, ItemComponent,
    ItemComponentPatch, ItemData, ItemStack, ObservedValue, PlayerObservation, ReceivedPose,
    SessionStamp, SlotKnowledge, ValueSource, WorldTime,
};
pub(crate) use observation::{LegacyReceipts, legacy_slot, received};
pub use operations::{Creative, DispatchReceipt, Survival};
pub use received_items::{ReceivedInventory, ReceivedItem, ReceivedSlot};
pub use recording::{PacketPhase, PacketRecord, PacketTrace};
pub use survival::{BlockTargetHit, BlockTargetObservation, MiningEstimate};
pub use vehicle::{
    BoatCollisionBody, BoatCollisionSample, BoatFrame, BoatMotion, BoatVelocityUpdate,
    DismountGrounding, DismountId, DismountRecord, DismountStage, MAX_BOAT_COLLISION_BODIES,
    MAX_VEHICLE_CONTROL_TICKS, MountId, VehicleControlId, VehicleControlRecord,
    VehicleControlStage, VehicleInput, VehicleObservation, VehicleRelation,
};

/// Imports for consumers selecting their Minecraft version at setup.
pub mod prelude {
    pub use super::blocks::{BlockRaycast, BlockRaycastObservation, BlockSearch};
    pub use super::chat::{ChatKind, ChatLog, ChatText, ReceivedChat};
    pub use super::chunk_context::{
        BiomeVolume, BlockEntityKind, ChunkContextObservation, ChunkIdentity, HeightmapKind,
        ReceivedBlockEntity, ReceivedHeightmap,
    };
    pub use super::container::{
        ContainerCloseId, ContainerCloseRecord, ContainerCloseStage, ContainerOpenId,
        ContainerOpenProcessing, ContainerOpenRecord, ContainerOpenSend, ContainerOpenStage,
        ContainerOpenTargetState, ContainerScreen, CursorReturnPlanStep, PlayerScreenAccess,
        ScreenId, ScreenObservation,
    };
    pub use super::context::{
        CooldownKey, DeathLocation, DefaultSpawnPosition, Experience, ItemCooldown,
        LoginConditions, PlayerAbilities, PlayerContextObservation, WeatherObservation,
        WorldDifficulty, WorldEntryContext, WorldViewObservation,
    };
    pub use super::entity::{
        EntitiesObservation, EntityObservation, EntityPartEvidence, EntityPartId, EntityPartKind,
        EntityPartModel, EntityPartObservation, EntityPartState, EntityPartUnavailable,
        EquipmentSlot,
    };
    pub use super::events::{ClientEvent, EventKind, EventLog};
    pub use super::inventory::{
        InventoryClickButton, InventoryClickId, InventoryClickRecord, InventoryClickSource,
        InventoryClickStage, InventorySource, InventorySwapId, InventorySwapRecord,
        InventorySwapSource, InventorySwapStage, InventoryTransferId, InventoryTransferRecord,
        InventoryTransferStage,
    };
    pub use super::maps::{
        MAX_RECEIVED_MAPS, MapIconKind, MapIdentity, MapObservation, ReceivedMapIcon,
    };
    pub use super::nbt::{NbtCompound, NbtData, NbtEntry, NbtString, NbtValue};
    pub use super::observation::SlotKnowledge;
    pub use super::recording::{
        LocalPlayerBasis, PacketPhase, PacketRecord, PacketTrace, RecordedItemData,
        RecordedItemStack, RecordedSlotKnowledge, RecordedValue, ReplayedBlock, ReplayedInventory,
        ReplayedObservation,
    };
    pub use super::registry::{
        BuiltinRegistryId, ItemComponentDefinition, Registry, RegistryEntryId, RegistryId,
        RegistryKind, ServerRegistryEntry, ServerRegistryId, ServerRegistryObservation,
        ServerRegistryStamp, ServerRegistryTags,
    };
    pub use super::survival::{
        BlockTargetHit, BlockTargetObservation, CapturedSurvivalScene, MiningEstimate, MiningId,
        MiningProfileRecovery, MiningRecord, MiningRecoveryAttempt, MiningRecoveryEvidence,
        MiningRecoveryMethod, MiningRecoveryTarget, MiningStage, MotionPreview, MotionRecord,
        MotionStatus, PlacementId, PlacementRecord, PlacementStage, RecoveredSurvivalClient,
        ScenePreview, SceneSource, SurvivalControl, SurvivalInput,
    };
    pub use super::ui::{
        BossBarColor, BossBarFlags, BossBarOverlay, BossBarsObservation, PlayerChatSession,
        PlayerListEntry, PlayerListObservation, PlayerListing, PlayerProfile, PlayerProperty,
        ReceivedBossBar, ReceivedTeam, ScoreNumberFormat, ScoreboardObjective,
        ScoreboardObservation, ScoreboardRenderType, ScoreboardScore, TabListObservation,
        TabListText, TeamCollision, TeamColor, TeamParameters, TeamVisibility, TeamsObservation,
        TitleTiming, TitlesObservation, UiText, WorldBorderDuration, WorldBorderObservation,
        WorldBorderSize,
    };
    pub use super::vehicle::control::{
        BoatFrame, BoatMotion, BoatVelocityUpdate, MAX_VEHICLE_CONTROL_TICKS, VehicleControlId,
        VehicleControlRecord, VehicleControlStage, VehicleInput,
    };
    pub use super::vehicle::{BoatCollisionBody, BoatCollisionSample, MAX_BOAT_COLLISION_BODIES};
    pub use super::{
        BlockFace, ChunkObservation, ConnectionStatus, CraftOnceRecord, CraftingFillRecord,
        DigEstimate, DigRecord, Hand, LoadedChunks, ObservedValue, PlacementCheck, PlayerAttribute,
        PlayerControl, PlayerEffect, ValueSource,
    };
    pub use super::{
        Client, ClientLimits, ClientManager, ConnectionConfig, ConnectionIdentity,
        ConnectionRevocation, CraftingGridReturnPlan, CraftingGridReturnStep,
        CraftingGridUnreturnedSplit, CraftingResultDestination, CraftingSource, CraftingTakeId,
        CraftingTakeRecord, CraftingTakeStage, Creative, DismountGrounding, DismountId,
        DismountRecord, DismountStage, EntityId, EntityMotionObservation, EntityPosition,
        EntityPositionCorrection, EntitySpawn, EntitySpawns, Feature, FlightCommand, FlightLanding,
        FlightRecord, FlightStage, FurnaceObservation, FurnaceSlot, GameMode, ItemComponent,
        ItemComponentPatch, ItemData, ItemProperties, ItemStack, MountId, PlayerObservation,
        ReceivedCrafting, ReceivedCraftingContext, ReceivedInventory, ReceivedItem, ReceivedRecipe,
        ReceivedRecipeGhost, ReceivedRecipes, ReceivedSlot, RecipeBookMaterials, RecipeBookStock,
        RecipeCraftingCell, RecipeCraftingLayout, RecipeDisplay, RecipeId, RecipeIngredient,
        RecipePlacementAmount, RecipePlacementId, RecipePlacementPlan, RecipePlacementRecord,
        RecipePlacementSend, RecipePlacementStage, RecipeSlotDisplay, RecipeTrimDefinition,
        RecipeTrimPattern, RespawnRecord, RespawnStage, Server, Support, Survival,
        VehicleObservation, VehicleRelation,
    };
    pub use super::{
        EntityHistory, EntityHistoryCursor, EntityHistoryGap, EntityHistoryKind,
        EntityHistoryRecord, MAX_ENTITY_HISTORY_READ, MAX_ENTITY_HISTORY_RECORDS,
    };
    pub use crate::{Error, ErrorKind, MinecraftVersion, NativeBlockState, Region, Result};
}
