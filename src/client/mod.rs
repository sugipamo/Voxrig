//! Common client entry points and data. Wire formats and physics belong to adapters.
mod capabilities;
mod config;
pub mod container;
mod geometry;
pub mod inventory;
mod item;
pub mod nbt;
mod observation;
pub(crate) mod operations;
pub mod registry;
pub mod survival;
#[cfg(test)]
pub(crate) mod tests;
pub use crate::connection::{Client, ConnectionConfig, Observation, ObservedBlock, Region};
pub use capabilities::{Capabilities, Feature, Support};
pub use config::{ClientLimits, Server};
pub use geometry::{Aabb, BlockFace, BlockPos, Hand, Vec3};
pub use item::ItemProperties;
pub use observation::{
    Capture, Dimension, GameMode, Health, InventoryObservation, ItemComponent, ItemComponentPatch,
    ItemData, ItemStack, ObservedValue, PlayerObservation, ReceivedPose, SessionStamp,
    SlotKnowledge, ValueSource,
};
pub(crate) use observation::{LegacyReceipts, legacy_slot, received};
pub use operations::{Creative, DispatchReceipt, Survival};
pub use survival::{BlockTargetHit, BlockTargetObservation};

/// Imports for consumers selecting their Minecraft version at setup.
pub mod prelude {
    pub use super::container::{
        ContainerCloseId, ContainerCloseRecord, ContainerCloseStage, ContainerOpenId,
        ContainerOpenProcessing, ContainerOpenRecord, ContainerOpenSend, ContainerOpenStage,
        ContainerOpenTargetState, ContainerScreen, CursorReturnPlanStep, PlayerScreenAccess,
        ScreenId, ScreenObservation,
    };
    pub use super::inventory::{
        InventoryClickButton, InventoryClickId, InventoryClickRecord, InventoryClickSource,
        InventoryClickStage, InventorySource, InventorySwapId, InventorySwapRecord,
        InventorySwapSource, InventorySwapStage, InventoryTransferId, InventoryTransferRecord,
        InventoryTransferStage,
    };
    pub use super::nbt::{NbtCompound, NbtData, NbtEntry, NbtString, NbtValue};
    pub use super::registry::{
        ItemComponentDefinition, Registry, RegistryId, RegistryKind, ServerRegistryEntry,
        ServerRegistryId, ServerRegistryObservation, ServerRegistryStamp, ServerRegistryTags,
    };
    pub use super::survival::{
        BlockTargetHit, BlockTargetObservation, MiningId, MiningRecord, MiningStage, MotionPreview,
        MotionRecord, MotionStatus, PlacementId, PlacementRecord, PlacementStage, SurvivalControl,
        SurvivalInput,
    };
    pub use super::{
        Client, ClientLimits, ConnectionConfig, Creative, Feature, GameMode, ItemComponent,
        ItemComponentPatch, ItemData, ItemProperties, ItemStack, PlayerObservation, Server,
        Support, Survival,
    };
    pub use crate::{Error, ErrorKind, MinecraftVersion, NativeBlockState, Region, Result};
}
