//! A headless Minecraft Java 1.16.1 client for externally controlled agents.
//!
//! `zen-minecraft-client` implements the protocol-facing body of an agent: connection
//! management, world and entity observation, player state, physics, inventory,
//! crafting, containers, interaction, combat, chat, and structured sound events.
//! It deliberately does not provide pathfinding, semantic perception, planning,
//! memory, or an AI runtime. Those belong in a consumer crate.
//!
//! The crate targets Java Edition 1.16.1, protocol 736, on offline-mode servers.
//! It does not implement Microsoft authentication or online-mode encryption.
//!
//! # Quick start
//!
//! ```no_run
//! use zen_minecraft_client::prelude::*;
//!
//! # async fn run() -> anyhow::Result<()> {
//! let manager = BotManager::new(Server::new("127.0.0.1", 25565));
//! let bot = manager.connect(Player::offline("AgentOne")).await?;
//! bot.wait_until_ready().await?;
//!
//! let player = bot.player().await;
//! let nearby_blocks = bot.observe(4).await?;
//! let nearby_entities = bot.observe_entities(16.0).await;
//! println!("{player:?} {} {}", nearby_blocks.len(), nearby_entities.len());
//!
//! bot.set_control(ControlState {
//!     forward: true,
//!     ..ControlState::default()
//! }).await;
//! bot.jump().await?;
//! bot.clear_control().await;
//! bot.disconnect().await?;
//! # Ok(())
//! # }
//! ```
//!
//! See `docs/api.md` in the repository for the complete public API map and
//! `docs/architecture.md` for the ownership boundary with external controllers.
#![warn(missing_docs)]

pub mod chat;
pub mod client;
mod collision;
pub mod connection;
pub mod entity;
mod error;
pub mod interaction;
pub mod inventory;
pub mod manager;
pub mod map;
pub mod observation;
pub mod physics;
pub mod primitive;
pub mod progress;
mod protocol;
pub mod registry;
pub mod server_registry;
pub mod snapshot;
pub mod survival;
pub mod ui;
pub mod unstable;
pub mod world;

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
pub use connection::{
    ClientConnectionGeneration, ClientOperationClass, ConnectionLifecycle, OperationAdmissionError,
    OperationContext, ProtocolTransaction,
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
pub use manager::{BotEvent, BotManager};
pub use map::{MapData, MapIcon, MapRectangle, MapStore, MapUpdate};
pub use observation::{
    BlockCatalogIdentity, BlockPhysicalDescriptor, BlockPhysicalDescriptorLookup,
    BlockRegistryIdentity, CoherentInterestCell, CoherentLightState, CoherentObservation,
    CoherentObservationInterest, CoherentObservationRequest,
    CoherentWorldTime, LoadedGeometrySection, LoadedGeometrySnapshot, LoadedResourceCoverage,
    LoadedResourceQuery, LoadedResourceQuerySnapshot, ObservationSequence, OpenFurnaceObservation,
    PlacedBlockPhysicalDescriptorLookup, SensorCaptureIdentity, TraversalBlockFact,
    TraversalEntityDimensions, TraversalEntityFact, TraversalGeometryBlockFact,
    TraversalGeometryQuery, TraversalGeometrySnapshot, TraversalInventoryFact,
    TraversalInventorySlotFact, TraversalMovementFactsRequest, TraversalMovementFactsSnapshot,
};
pub use physics::{
    Aabb, ControlState, CorrectionReason, MotionState, PhysicsMeasurementEpoch, PhysicsMetrics,
    PositionCorrection, Vec3, VehicleControl, VehiclePose,
};
pub use primitive::{
    AcknowledgedPrimitive, CleanupDispatchOutcome, CleanupPrimitive, CleanupRequest,
    CraftAcceptedCacheEffects, CraftAcceptedSlotEffect, CraftClick, CraftExecution, EquipOperation,
    FurnaceExecution, FurnaceExecutionPhase, InteractionSneakRequirement,
    PrimitiveDiagnosticCorrelationId, PrimitiveDispatchError, PrimitiveDispatchOutcome,
    PrimitiveOperation, PrimitiveRequest, SlotExpectation,
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

/// Coherent local pose and mining environment observation.
pub mod mining_environment;
