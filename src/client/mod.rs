//! Common client entry points and data. Wire formats and physics belong to adapters.
mod capabilities;
mod config;
mod geometry;
pub mod inventory;
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
pub use observation::{
    Capture, Dimension, GameMode, Health, InventoryObservation, ItemData, ItemStack, ObservedValue,
    PlayerObservation, ReceivedPose, SessionStamp, SlotKnowledge, ValueSource,
};
pub(crate) use observation::{LegacyReceipts, legacy_slot, received};
pub use operations::{Creative, DispatchReceipt, Survival};

/// Imports for consumers selecting their Minecraft version at setup.
pub mod prelude {
    pub use super::inventory::{InventorySwapId, InventorySwapRecord, InventorySwapStage};
    pub use super::survival::{
        BlockTargetHit, BlockTargetObservation, MiningId, MiningRecord, MiningStage, MotionPreview,
        MotionRecord, MotionStatus, PlacementId, PlacementRecord, PlacementStage, SurvivalControl,
        SurvivalInput,
    };
    pub use super::{
        Client, ClientLimits, ConnectionConfig, Creative, Feature, GameMode, PlayerObservation,
        Server, Support, Survival,
    };
    pub use crate::{Error, ErrorKind, MinecraftVersion, NativeBlockState, Region, Result};
}
