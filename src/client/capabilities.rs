//! Static implementation support; not current game permissions or readiness.
use crate::MinecraftVersion;

/// Implementation status, separate from the game's existence of a feature.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum Support {
    /// Common contract implemented by this adapter.
    Available,
    /// Explicitly bounded implementation.
    Restricted(
        /// Restriction, not a current permission decision.
        &'static str,
    ),
    /// Missing implementation in Voxrig, not proof that the game lacks it.
    NotImplemented,
}
/// Common API feature group.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Feature {
    /// Namespaced state and version-bound item lookup.
    Registry,
    /// Region observations, including missing cells.
    WorldObservation,
    /// Common own-player and received inventory observations.
    PlayerObservation,
    /// Mode-checked look and hotbar selection.
    BasicControls,
    /// Same static outline query on survival and creative handles.
    BlockTargeting,
    /// First static block outline from a coherent dry-standing capture.
    SurvivalTargeting,
    /// Read-only bounded walking/jump model preview under dry standing defaults.
    SurvivalPreview,
    /// Retained empty-hand dirt/stone mining, with explicit FINISH/ABORT and target receipts.
    SurvivalMining,
    /// One-shot passive-cube placement with separate target/material receipts.
    SurvivalPlacement,
    /// Connection-owned finite dry walking/jump dispatch under prediction contract.
    SurvivalMovement,
    /// Creative flight, default-stack writes and block interactions.
    CreativeControls,
    /// Extra audited dry-cube survival contract.
    CheckedSurvival,
    /// Ordinary received storage/hotbar exchanges; broader container commands remain.
    Containers,
    /// Received open-screen identity, contents, cursor and native layout.
    ContainerObservation,
    /// Received whole-default-stack player main/hotbar exchange.
    InventorySwap,
    /// Ordinary default-stack player/storage PICKUP with separate predictions/receipts.
    InventoryClick,
    /// Ordinary native-order transfer, including default equipment and partial capacity.
    InventoryTransfer,
    /// Common crafting operations.
    Crafting,
    /// Common general-entity interaction.
    EntityInteraction,
    /// Common recording and reconstruction API.
    RecordingAndReconstruction,
}
/// Static common API support for the selected build and version.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct Capabilities {
    /// Selected adapter version.
    pub version: MinecraftVersion,
}
impl Capabilities {
    /// Inspect support before opening a connection.
    pub const fn for_version(version: MinecraftVersion) -> Self {
        Self { version }
    }
    /// Discover implemented semantics. Readiness and permission are checked on each action.
    pub const fn support(self, feature: Feature) -> Support {
        match feature {
            Feature::Registry
            | Feature::WorldObservation
            | Feature::PlayerObservation
            | Feature::BasicControls => Support::Available,
            Feature::CreativeControls => Support::Restricted(
                "default items; permitted flight steps <=4 blocks; loaded reachable targets",
            ),
            Feature::BlockTargeting | Feature::SurvivalTargeting => Support::Restricted(
                "healthy dry stationary normal posture, matching handle mode; reach <=4.5; audited static outlines; legacy passive full cubes, seven storage blocks and crafting table; animated/world-dependent shapes remain incomplete",
            ),
            Feature::SurvivalPreview => Support::Restricted(
                "read-only 1..120 walking/jump inputs; healthy stationary normal survival posture; native defaults; loaded dry full cubes",
            ),
            Feature::SurvivalMining => Support::Restricted(
                "healthy dry standing; received empty selected hand/cursor; dirt/stone first outline; retained explicit commands/target conflicts; removal does not permit continuation",
            ),
            Feature::SurvivalPlacement => Support::Restricted(
                "healthy dry standing; default passive cubes; first-outline support/face; fresh target and one-material receipts; modern processing ACK also required",
            ),
            Feature::InventorySwap => Support::Restricted(
                "player main slots 9..35 and hotbar 0..8; received player UI or explicit complete local close; resolved NBT/components and effective capacity; received empty cursor/two fresh destinations; native legacy resync/comparison response",
            ),
            Feature::InventoryClick => Support::Restricted(
                "ordinary PICKUP with resolved NBT/components; player input slots 1..4 and inventory slots 9..44 or same audited storage/crafting opening including appended player; separate prediction and fresh source/cursor receipts; result/armor/offhand PICKUP, modern bundle override and unresolved item semantics remain incomplete",
            ),
            Feature::InventoryTransfer => Support::Restricted(
                "one ordinary native QUICK_MOVE; player slots 5..45 or same audited storage opening; resolved NBT/components and native effective capacity; preserve known received cursor; full write/all fresh changed slots/legacy reply; mode-specific armor pickup and effective equipment routing",
            ),
            Feature::ContainerObservation => Support::Restricted(
                "regular OPEN_WINDOW screens and constructor-verified storage/crafting layouts; lossless modern component boundaries, common custom metadata and effective scalar item properties; general component semantics and special entity windows remain incomplete",
            ),
            Feature::Containers => Support::Restricted(
                "empty-hand audited storage/crafting-table activation with distinct dispatch/OPEN/full/cursor/modern processing facts; opening-bound close with observed resolved-data cursor return; constructor-verified ordinary slots; SWAP/QUICK_MOVE remain storage/player only; general UI/special clicks and unresolved item semantics remain incomplete",
            ),
            Feature::SurvivalMovement => Support::Restricted(
                "1..120 dry walking/jump ticks with released-rest endpoint; retained intent/failure; predicted completion is not received acceptance",
            ),
            Feature::CheckedSurvival => match self.version {
                MinecraftVersion::Java1_16_1 => Support::NotImplemented,
                MinecraftVersion::Java1_21_11 => Support::Restricted(
                    "audited dry full-cube movement; plain inventory swaps; passive-cube placement; empty-hand dirt/stone mining and fresh recovery",
                ),
            },
            Feature::Crafting => Support::Restricted(
                "received player/table input grids and displayed result with coherent registry ownership; empty-hand table activation, ordinary input PICKUP and opening-bound close with cursor return; native input disposal on close is not locally predicted or a close ACK; recipe planning/result take/consumption/remainders remain incomplete",
            ),
            Feature::EntityInteraction | Feature::RecordingAndReconstruction => {
                Support::NotImplemented
            }
        }
    }
}
