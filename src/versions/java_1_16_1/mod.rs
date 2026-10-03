//! Minecraft Java 1.16.1 implementation, registry and movement behavior.
//! Existing APIs are re-exported at the crate root for compatibility.

pub mod chat;
pub mod client;
pub(crate) mod collision;
pub mod entity;
pub(crate) use crate::error;
pub mod interaction;
pub mod inventory;
pub mod lifecycle;
pub mod manager;
pub mod map;
/// Coherent local pose and mining environment observation.
pub mod mining_environment;
pub mod observation;
pub mod operation;
pub mod physics;
pub mod progress;
pub(crate) mod protocol {
    pub(crate) use crate::protocol::*;
    pub const PROTOCOL_VERSION: i32 = 736;
}
pub mod registry;
pub mod server_registry;
pub use crate::snapshot;
pub mod survival;
pub mod ui;
pub mod unstable;
pub mod world;

/// Decodes a state ID exclusively using the Java 1.16.1 registry.
fn state_registry() -> &'static crate::block_state::StateRegistry {
    static STATES: std::sync::OnceLock<crate::block_state::StateRegistry> =
        std::sync::OnceLock::new();
    STATES.get_or_init(|| {
        crate::block_state::StateRegistry::parse(include_str!("../../../data/blocks.json"))
            .expect("bundled Java 1.16.1 state definitions are valid")
    })
}

/// Decodes an ID exclusively using the Java 1.16.1 registry.
pub fn native_state(id: i32) -> crate::Result<crate::NativeBlockState> {
    state_registry().decode(id)
}

/// Resolves a complete state exclusively using the Java 1.16.1 registry.
pub fn state_id(state: &crate::NativeBlockState) -> crate::Result<i32> {
    state_registry().encode(state)
}

/// Common imports for applications that control one or more clients.
pub mod prelude {
    pub use crate::{
        BlockFace, BlockPos, Bot, BotManager, ClickMode, ControlState, Error, ErrorKind, Event,
        Hand, Player, Result, Server,
    };
}

// Root re-exports are retained as the concise and backwards-compatible API.
pub use chat::{ChatMessage, PlayerList, PlayerListEntry, PlayerProperty};
pub use client::{
    BlockActionEvent, Bot, CLIENT_INFO, ChatMode, ClientCapabilities, ClientInfo, ClientSettings,
    ConnectionOptions, Event, ExplosionEvent, MainHand, NbtQueryResponse, Player, ProtocolInfo,
    ResourcePackRequest, ResourcePackStatus, Server, SoundEvent, SoundSource, StopSoundEvent,
    TabCompletion, TabCompletionMatch, WorldEvent, WorldParticleEvent, WorldViewState,
};
pub use collision::{
    BlockCollision, BlockCollisionShape, BlockSupportSurface, block_collision,
    block_collision_shapes, block_support_surface,
};
pub use entity::{
    EntityKind, EntityRaycastHit, EntityState, EntityTracker, MetadataValue, ParticleData,
};
pub use error::{Error, ErrorKind, Result};
pub use interaction::{
    BlockBreakProgress, BlockFace, BlockPos, BlockRaycastHit, DiggingAcknowledgement, DiggingInfo,
    DiggingStatus, Hand, PlacementInfo, PlacementRequest,
};
pub use inventory::{
    ClickMode, EquipmentSlot, InventoryState, ItemCollected, ItemStack, MerchantOffer,
    MerchantOffers, OpenWindow, PendingClick, SlotUpdate, WindowProperty, WindowTransaction,
};
pub use lifecycle::{
    ConnectionGeneration, ConnectionState, OperationAdmissionError, OperationClass,
    OperationContext, ProtocolTransaction,
};
pub use manager::{BotEvent, BotManager};
pub use map::{MapData, MapIcon, MapRectangle, MapStore, MapUpdate};
pub use observation::{
    BlockCatalogIdentity, BlockPhysicalDescriptor, BlockPhysicalDescriptorLookup, BlockQuery,
    BlockQueryCoverage, BlockQuerySnapshot, BlockRegistryIdentity, CaptureIdentity,
    CoherentInterestCell, CoherentLightState, CoherentObservation, CoherentObservationInterest,
    CoherentObservationRequest, CoherentWorldTime, EntityDimensions, GeometryBlock, GeometryQuery,
    GeometrySnapshot, InventoryFact, InventorySlotFact, LoadedGeometrySection,
    LoadedGeometrySnapshot, MovementBlock, MovementSnapshot, MovementSnapshotRequest,
    ObservationSequence, ObservedEntity, OpenFurnaceObservation,
    PlacedBlockPhysicalDescriptorLookup,
};
pub use operation::{
    AcknowledgedOperation, CleanupDispatchOutcome, CleanupOperation, CleanupRequest,
    DiagnosticCorrelationId, DispatchError, DispatchOutcome, EquipOperation,
    InteractionSneakRequirement, Operation, OperationRequest, SlotExpectation, SlotPrediction,
    WindowClick, WindowClickSequence, WindowPrediction,
};
pub use physics::{
    Aabb, ControlState, CorrectionReason, MotionState, PhysicsMeasurementEpoch, PhysicsMetrics,
    PositionCorrection, Vec3, VehicleControl, VehiclePose,
};
pub use progress::{
    AdvancementDefinition, AdvancementDisplay, AdvancementProgress, AdvancementState,
    RecipeBookState, StatisticsState,
};
pub use registry::{
    MiningInfo, RawBlockBoundingBox, RawBlockMovementRegistryFact, RawRecipe, RecipeResult,
    block_movement_registry_facts, block_name_from_state, block_state_properties,
    entity_dimensions, entity_name, item_id, item_name, mining_info, recipes_for_output,
    sound_name,
};
pub use server_registry::{
    CommandNode, CommandNodeKind, CommandTree, Ingredient, ServerRecipe, ServerRecipeData,
    ServerRecipes, ServerTags,
};
pub use snapshot::Snapshot;
pub use survival::{
    Attribute, AttributeModifier, CombatEvent, Difficulty, Experience, GameStateChange,
    RespawnState, SpawnPosition, StatusEffect, SurvivalState, Vitals,
};
pub use ui::{BossBar, ScoreboardObjective, Team, TitleState, UiState, UiUpdateKind, WorldBorder};
pub use unstable::UnstableBot;
pub use world::{
    BlockEntityData, BlockObservation, BlockRegion, ChunkPos, ChunkSnapshot, ChunkStorageStats,
    EnvironmentState, FluidKind, SharedChunkStorage,
};
