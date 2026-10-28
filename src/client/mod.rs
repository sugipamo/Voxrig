//! Common client entry points and data. Wire formats and physics belong to adapters.
//!
//! Passive observation reads include [`Client::connection_status`], [`Client::player_state`],
//! [`Client::received_inventory`], [`Client::player_context`],
//! [`Client::server_registry_state`], [`Client::screen_state`],
//! [`Client::received_recipes`], [`Client::received_recipe_ghost`],
//! [`Client::received_crafting_context`], [`Client::recipe_book_materials`],
//! [`Client::received_crafting`], [`Client::capture`] and [`Client::observe_region`],
//! plus the [`Survival::player_state`] and [`Creative::player_state`] wrappers.
//! These reads never send packets, admit writes, initiate cleanup, or advance or
//! reconcile work. The guarantee is that the read itself causes no sends: a read
//! may wait for local state locks held by a receiver that sends protocol replies,
//! and other connection tasks may send while the read is pending. It does not
//! promise connection silence, fresh evidence, server acceptance or absence of
//! pending work. This contract is specific to these reads; operation-record
//! methods may reconcile work as documented on those methods.
pub(crate) mod adapter;
pub(crate) use adapter::dispatch;
pub mod blocks;
pub use blocks::{BlockRaycast, BlockRaycastObservation, BlockSearch};
pub(crate) mod books;
pub mod chat;
pub mod effects;
pub mod events;
pub mod explosions;
pub mod health;
pub use events::{ClientEvent, EventKind, EventLog};
pub use health::{
    HealthChange, HealthEvent, HealthLog, HealthRecvError, HealthSubscription,
    MAX_RETAINED_HEALTH_CHANGES, ReceivedHealth,
};
pub mod code_of_conduct;
pub mod resource_pack;
pub use resource_pack::ResourcePackRequest;
mod wait;
pub use chat::{ChatKind, ChatLog, ChatText, ReceivedChat};
mod capabilities;
mod config;
pub(crate) mod constructor;
pub mod container;
pub mod context;
pub mod control;
pub mod glide;
pub use context::{
    CooldownKey, DeathLocation, DefaultSpawnPosition, Experience, ItemCooldown, LoginConditions,
    PlayerAbilities, PlayerContextObservation, WeatherObservation, WorldDifficulty,
    WorldEntryContext, WorldViewObservation,
};
pub mod crafting;
pub(crate) mod enchantments;
pub(crate) mod entity;
pub use entity::{
    EntityContactFrame, EntityContactSample, EntityEquipmentEntry, EntityHistory,
    EntityHistoryCursor, EntityHistoryGap, EntityHistoryKind, EntityHistoryRecord,
    EntityHistoryValue, EntityMetadataEntry, EntityTargetCandidate, EntityTargetObservation,
    LivingContactBody, LivingContactCandidate, LivingContactDisposition, LivingContactIndex,
    LivingContactIndexedBody, LivingContactKind, LivingContactPlayer, LivingContactQuery,
    LivingContactQueryUnavailable, LivingContactResult, LivingContactsModel,
    LivingContactsUnavailable, MAX_ENTITY_HISTORY_BYTES, MAX_ENTITY_HISTORY_READ,
    MAX_ENTITY_HISTORY_RECORDS, MAX_ENTITY_HISTORY_VALUE_BYTES, MAX_ENTITY_TARGET_QUERY_BODIES,
    MAX_LIVING_CONTACT_CANDIDATES, MAX_LIVING_CONTACT_INDEX_BODIES,
    MAX_LIVING_CONTACT_INDEX_CHUNKS, MAX_LIVING_CONTACT_QUERY_CHUNKS, model_living_contacts,
};
pub mod flight;
pub(crate) mod fraction;
pub mod furnace;
pub(crate) mod motion_executor;
pub use flight::{FlightCommand, FlightLanding, FlightRecord, FlightStage};
pub(crate) mod chunk_context;
pub(crate) mod chunks;
pub(crate) mod dig;
pub(crate) mod dig_operation;
pub(crate) mod domain;
pub use dig_operation::{
    DigCancelOutcome, DigCompletion, DigDispatchEvidence, DigOperation, DigOperationId,
    DigOwnerExit, DigPhase, DigResult, DigSnapshot, DigWriteEvidence,
};
pub(crate) mod operational_dig;
pub use dig::{DigEstimate, DigRecord, DigRefusal};
pub use operational_dig::{DigCleanupRecord, DigCleanupStatus};
mod geometry;
mod hash_ops;
pub(crate) mod identifier;
pub mod inventory;
mod inventory_helpers;
#[cfg(test)]
pub(crate) use inventory_helpers::scripted_tests as inventory_helper_tests;
pub use inventory_helpers::{
    CraftOnceError, CraftOnceRecord, CraftOnceUnknownCause, CraftingFillRecord,
    InventoryMoveRecord, InventoryMoveStage,
};
mod item;
pub(crate) mod item_components;
pub(crate) mod item_constructor;
pub(crate) mod item_semantics;
pub(crate) mod item_use;
pub use item_use::UseItemRefusal;
pub(crate) mod placement_check;
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
mod drain;
pub(crate) mod ledger;
mod lifecycle;
pub(crate) mod shutdown;
pub use drain::{
    AdmissionId, DrainError, DrainErrorKind, DrainHandle, DrainReceipt, DrainScope, GenerationKey,
    QuiesceHandle, StopAdmission, StopCompletion, StopError, StopPermit, StopReceipt, StopTarget,
    StopTargetKind, WriteOrdinal,
};
pub use lifecycle::{ConnectionRevocation, ConnectionStatus};
pub use shutdown::{ShutdownError, ShutdownErrorKind};
pub(crate) mod login;
pub use furnace::{FurnaceObservation, FurnaceSlot};
pub mod manager;
pub mod maps;
pub use manager::ClientManager;
pub use maps::{MAX_RECEIVED_MAPS, MapIconKind, MapIdentity, MapObservation, ReceivedMapIcon};
pub mod nbt;
pub mod received_snapshot;
pub use received_snapshot::{
    ContainerStatus, CorrectionComponent, Coverage, FieldReceipt, FieldState, Invalidation,
    InvalidationReason, Presence, Receipt, ReceiveBoundary, ReceivedAttribute,
    ReceivedContainerSnapshot, ReceivedCorrectionFields, ReceivedDimensionGeometry,
    ReceivedDimensionSelection, ReceivedInventorySnapshot, ReceivedPlayerSnapshot,
    ReceivedRotationCorrection,
};
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
    EntityEyeHeight, EntityGeometry, EntityGeometryEvidence, EntityGeometryModel,
    EntityGeometryUnavailable, EntityId, EntityItemStackObservation, EntityItemStacksObservation,
    EntityMotionObservation, EntityMotionPacket, EntityObservation, EntityPartEvidence,
    EntityPartId, EntityPartKind, EntityPartModel, EntityPartObservation, EntityPartState,
    EntityPartUnavailable, EntityPose, EntityPoseReading, EntityPosition, EntityPositionCorrection,
    EntityScaleReading, EntitySleepingReading, EntitySpawn, EntitySpawns, EquipmentSlot,
    LivingEntityPacketReplay, LivingEntityTickOrder, LivingInterpolation, LivingInterpolationFrame,
    LivingInterpolationPose, LivingInterpolationUnavailable, LivingMotionPacket, LivingPacketFrame,
    LivingPacketGuards, LivingPacketModel, LivingPacketUnavailable, LivingPairPushDisposition,
    LivingPairPushInput, LivingPairPushModel, LivingPairPushUnavailable, LivingPushBody,
    LivingReplayFrame, LivingReplayUnavailable, LivingTickBody, LivingTickDispatch,
    LivingTickInstruction, LivingTickUnavailable, MAX_LIVING_TICK_BODIES, model_living_pair_push,
};
pub use geometry::{Aabb, BlockFace, BlockPos, Hand, Vec3};
pub use item::ItemProperties;
pub(crate) use item::{modern_prototype_components, modern_weight_defaults};
pub(crate) mod posture;
pub use observation::{
    Capture, ConnectionIdentity, Dimension, GameMode, Health, InventoryObservation, ItemComponent,
    ItemComponentPatch, ItemData, ItemStack, ObservedValue, PlayerObservation, ReceivedPose,
    SessionStamp, SlotKnowledge, ValueSource, WorldTime,
};
pub(crate) use observation::{LegacyReceipts, legacy_slot, received};
pub use operations::{Creative, DispatchReceipt, Survival};
pub use posture::{PlayerEyeHeight, PlayerPostureObservation};
pub use received_items::{ReceivedInventory, ReceivedItem, ReceivedSlot};
pub use recording::{PacketPhase, PacketRecord, PacketTrace};
pub use survival::{
    BlockTargetHit, BlockTargetObservation, BlockTargetPose, BlockTargetPoseSource,
    ContextualBlockTargetObservation, MiningEstimate,
};
pub use vehicle::{
    BoatCollisionBody, BoatCollisionGeometry, BoatCollisionSample, BoatFrame, BoatMotion,
    BoatVelocityUpdate, DismountGrounding, DismountId, DismountRecord, DismountStage,
    MAX_BOAT_COLLISION_BODIES, MAX_VEHICLE_CONTROL_TICKS, MountId, VehicleControlId,
    VehicleControlRecord, VehicleControlStage, VehicleInput, VehicleInputRelease,
    VehicleObservation, VehicleRelation,
};

/// Imports for consumers selecting their Minecraft version at setup.
pub mod prelude {
    pub use super::blocks::{BlockRaycast, BlockRaycastObservation, BlockSearch};
    pub use super::chat::{ChatKind, ChatLog, ChatText, ReceivedChat};
    pub use super::chunk_context::{
        BiomeVolume, BlockEntityKind, ChunkContextObservation, ChunkIdentity, HeightmapKind,
        ReceivedBlockEntity, ReceivedHeightmap,
    };
    pub use super::container::properties::{
        MAX_SCREEN_PROPERTIES, ScreenProperties, ScreenPropertyObservation,
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
        EntitiesObservation, EntityContactFrame, EntityContactSample, EntityEyeHeight,
        EntityGeometry, EntityGeometryEvidence, EntityGeometryModel, EntityGeometryUnavailable,
        EntityItemStackObservation, EntityItemStacksObservation, EntityObservation,
        EntityPartEvidence, EntityPartId, EntityPartKind, EntityPartModel, EntityPartObservation,
        EntityPartState, EntityPartUnavailable, EntityPose, EntityPoseReading, EntityScaleReading,
        EntitySleepingReading, EntityTargetCandidate, EntityTargetObservation, EquipmentSlot,
        LivingContactBody, LivingContactCandidate, LivingContactDisposition, LivingContactIndex,
        LivingContactIndexedBody, LivingContactKind, LivingContactPlayer, LivingContactQuery,
        LivingContactQueryUnavailable, LivingContactResult, LivingContactsModel,
        LivingContactsUnavailable, LivingEntityPacketReplay, LivingEntityTickOrder,
        LivingInterpolation, LivingInterpolationFrame, LivingInterpolationPose,
        LivingInterpolationUnavailable, LivingMotionPacket, LivingPacketFrame, LivingPacketGuards,
        LivingPacketModel, LivingPacketUnavailable, LivingPairPushDisposition, LivingPairPushInput,
        LivingPairPushModel, LivingPairPushUnavailable, LivingPushBody, LivingReplayFrame,
        LivingReplayUnavailable, LivingTickBody, LivingTickDispatch, LivingTickInstruction,
        LivingTickUnavailable, MAX_ENTITY_TARGET_QUERY_BODIES, MAX_LIVING_CONTACT_CANDIDATES,
        MAX_LIVING_CONTACT_INDEX_BODIES, MAX_LIVING_CONTACT_INDEX_CHUNKS,
        MAX_LIVING_CONTACT_QUERY_CHUNKS, MAX_LIVING_TICK_BODIES, model_living_contacts,
        model_living_pair_push,
    };
    pub use super::events::{ClientEvent, EventKind, EventLog};
    pub use super::health::{
        HealthChange, HealthEvent, HealthLog, HealthRecvError, HealthSubscription,
        MAX_RETAINED_HEALTH_CHANGES, ReceivedHealth,
    };
    pub use super::inventory::{
        HandSwapId, HandSwapRecord, HandSwapSend, HandSwapStage, InventoryClickButton,
        InventoryClickId, InventoryClickRecord, InventoryClickSource, InventoryClickStage,
        InventorySource, InventorySwapId, InventorySwapRecord, InventorySwapSource,
        InventorySwapStage, InventoryTransferId, InventoryTransferRecord, InventoryTransferStage,
        SelectedDropAmount, SelectedDropId, SelectedDropRecord, SelectedDropSend,
        SelectedDropStage,
    };
    pub use super::maps::{
        MAX_RECEIVED_MAPS, MapIconKind, MapIdentity, MapObservation, ReceivedMapIcon,
    };
    pub use super::nbt::{NbtCompound, NbtData, NbtEntry, NbtString, NbtValue};
    pub use super::observation::SlotKnowledge;
    pub use super::received_snapshot::{
        ContainerStatus, CorrectionComponent, Coverage, FieldReceipt, FieldState, Invalidation,
        InvalidationReason, Presence, Receipt, ReceiveBoundary, ReceivedAttribute,
        ReceivedContainerSnapshot, ReceivedCorrectionFields, ReceivedDimensionGeometry,
        ReceivedDimensionSelection, ReceivedInventorySnapshot, ReceivedPlayerSnapshot,
        ReceivedRotationCorrection,
    };
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
    pub use super::resource_pack::ResourcePackRequest;
    pub use super::survival::{
        BlockTargetHit, BlockTargetObservation, CapturedSurvivalScene, FluidSlabPlacementRecord,
        MiningEstimate, MiningId, MiningProfileRecovery, MiningRecord, MiningRecoveryAttempt,
        MiningRecoveryEvidence, MiningRecoveryMethod, MiningRecoveryTarget, MiningStage,
        MotionPreview, MotionRecord, MotionStatus, PlacementId, PlacementRecord, PlacementStage,
        RecoveredSurvivalClient, ReplaceableCubePlacementRecord, ScenePreview, SceneSource,
        SlabFluidState, SlabPlacementRecord, StairNeighborUpdate, StairPlacementRecord,
        StairShapeInput, SurvivalControl, SurvivalInput,
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
        VehicleControlRecord, VehicleControlStage, VehicleInput, VehicleInputRelease,
    };
    pub use super::vehicle::{
        BoatCollisionBody, BoatCollisionGeometry, BoatCollisionSample, MAX_BOAT_COLLISION_BODIES,
    };
    pub use super::{
        BlockFace, ChunkObservation, ConnectionStatus, CraftOnceError, CraftOnceRecord,
        CraftOnceUnknownCause, CraftingFillRecord, DigCancelOutcome, DigCompletion,
        DigDispatchEvidence, DigEstimate, DigOperation, DigOperationId, DigOwnerExit, DigPhase,
        DigRecord, DigRefusal, DigResult, DigSnapshot, DigWriteEvidence, Hand, InventoryMoveRecord,
        InventoryMoveStage, LoadedChunks, ObservedValue, PlacementCheck, PlayerAttribute,
        PlayerControl, PlayerEffect, PlayerEyeHeight, PlayerPostureObservation, UseItemRefusal,
        ValueSource,
    };
    pub use super::{
        Client, ClientLimits, ClientManager, ConnectionConfig, ConnectionIdentity,
        ConnectionRevocation, CraftingGridReturnPlan, CraftingGridReturnStep,
        CraftingGridUnreturnedSplit, CraftingResultDestination, CraftingSource, CraftingTakeId,
        CraftingTakeRecord, CraftingTakeStage, Creative, DismountGrounding, DismountId,
        DismountRecord, DismountStage, EntityId, EntityMotionObservation, EntityMotionPacket,
        EntityPosition, EntityPositionCorrection, EntitySpawn, EntitySpawns, Feature,
        FlightCommand, FlightLanding, FlightRecord, FlightStage, FurnaceObservation, FurnaceSlot,
        GameMode, ItemComponent, ItemComponentPatch, ItemData, ItemProperties, ItemStack, MountId,
        PlayerObservation, ReceivedCrafting, ReceivedCraftingContext, ReceivedInventory,
        ReceivedItem, ReceivedRecipe, ReceivedRecipeGhost, ReceivedRecipes, ReceivedSlot,
        RecipeBookMaterials, RecipeBookStock, RecipeCraftingCell, RecipeCraftingLayout,
        RecipeDisplay, RecipeId, RecipeIngredient, RecipePlacementAmount, RecipePlacementId,
        RecipePlacementPlan, RecipePlacementRecord, RecipePlacementSend, RecipePlacementStage,
        RecipeSlotDisplay, RecipeTrimDefinition, RecipeTrimPattern, RespawnRecord, RespawnStage,
        Server, Support, Survival, VehicleObservation, VehicleRelation,
    };
    pub use super::{
        DrainError, DrainErrorKind, DrainHandle, QuiesceHandle, StopError, StopPermit, StopTarget,
        StopTargetKind,
    };
    pub use super::{
        EntityEquipmentEntry, EntityHistory, EntityHistoryCursor, EntityHistoryGap,
        EntityHistoryKind, EntityHistoryRecord, EntityHistoryValue, EntityMetadataEntry,
        MAX_ENTITY_HISTORY_BYTES, MAX_ENTITY_HISTORY_READ, MAX_ENTITY_HISTORY_RECORDS,
        MAX_ENTITY_HISTORY_VALUE_BYTES,
    };
    pub use crate::{Error, ErrorKind, MinecraftVersion, NativeBlockState, Region, Result};
}
