//! Common client entry points and data. Wire formats and physics belong to adapters.
pub(crate) mod books;
mod capabilities;
mod config;
pub(crate) mod constructor;
pub mod container;
pub mod crafting;
pub(crate) mod enchantments;
pub(crate) mod fraction;
mod geometry;
mod hash_ops;
pub(crate) mod identifier;
pub mod inventory;
mod item;
pub(crate) mod item_components;
pub(crate) mod item_constructor;
pub(crate) mod item_semantics;
pub mod nbt;
mod observation;
pub(crate) mod operations;
pub(crate) mod profile;
mod received_items;
pub mod registry;
pub(crate) mod selector;
pub(crate) mod selector_snbt;
pub mod survival;
#[cfg(test)]
pub(crate) mod tests;
pub(crate) mod text;
pub(crate) mod uri;
pub(crate) mod uuid;
pub use crate::connection::{Client, ConnectionConfig, Observation, ObservedBlock, Region};
pub use capabilities::{Capabilities, Feature, Support};
pub use config::{ClientLimits, Server};
pub use crafting::{
    CraftingSource, CraftingTakeId, CraftingTakeRecord, CraftingTakeStage, ReceivedCrafting,
    ReceivedRecipe, ReceivedRecipes, RecipeBookMaterials, RecipeBookStock, RecipeDisplay, RecipeId,
    RecipeIngredient, RecipeSlotDisplay, RecipeTrimDefinition, RecipeTrimPattern,
};
pub use geometry::{Aabb, BlockFace, BlockPos, Hand, Vec3};
pub use item::ItemProperties;
pub(crate) use item::{modern_prototype_components, modern_weight_defaults};
pub use observation::{
    Capture, Dimension, GameMode, Health, InventoryObservation, ItemComponent, ItemComponentPatch,
    ItemData, ItemStack, ObservedValue, PlayerObservation, ReceivedPose, SessionStamp,
    SlotKnowledge, ValueSource,
};
pub(crate) use observation::{LegacyReceipts, legacy_slot, received};
pub use operations::{Creative, DispatchReceipt, Survival};
pub use received_items::{ReceivedInventory, ReceivedItem, ReceivedSlot};
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
        BuiltinRegistryId, ItemComponentDefinition, Registry, RegistryEntryId, RegistryId,
        RegistryKind, ServerRegistryEntry, ServerRegistryId, ServerRegistryObservation,
        ServerRegistryStamp, ServerRegistryTags,
    };
    pub use super::survival::{
        BlockTargetHit, BlockTargetObservation, MiningId, MiningRecord, MiningStage, MotionPreview,
        MotionRecord, MotionStatus, PlacementId, PlacementRecord, PlacementStage, SurvivalControl,
        SurvivalInput,
    };
    pub use super::{
        Client, ClientLimits, ConnectionConfig, CraftingSource, CraftingTakeId, CraftingTakeRecord,
        CraftingTakeStage, Creative, Feature, GameMode, ItemComponent, ItemComponentPatch,
        ItemData, ItemProperties, ItemStack, PlayerObservation, ReceivedCrafting,
        ReceivedInventory, ReceivedItem, ReceivedRecipe, ReceivedRecipes, ReceivedSlot,
        RecipeBookMaterials, RecipeBookStock, RecipeDisplay, RecipeId, RecipeIngredient,
        RecipeSlotDisplay, RecipeTrimDefinition, RecipeTrimPattern, Server, Support, Survival,
    };
    pub use crate::{Error, ErrorKind, MinecraftVersion, NativeBlockState, Region, Result};
}
