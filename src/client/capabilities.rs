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
                "healthy dry stationary normal posture, matching handle mode; reach <=4.5; audited static outlines; legacy passive full cubes and seven storage blocks; animated/world-dependent shapes remain incomplete",
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
                "player main slots 9..35 and hotbar 0..8; received player UI or explicit complete local close; modern actual player revision; default stacks/received empty cursor/two fresh destinations; native legacy resync/comparison response",
            ),
            Feature::ContainerObservation => Support::Restricted(
                "regular OPEN_WINDOW screens and supported stacks; constructor-verified storage layouts; modern non-default components/special entity windows remain incomplete",
            ),
            Feature::Containers => Support::Restricted(
                "empty-hand audited storage activation with distinct dispatch/OPEN/full/cursor/modern processing facts; opening-bound empty-cursor close; constructor-verified default storage/hotbar SWAP with two fresh destinations; general UI/clicks remain incomplete",
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
            Feature::Crafting
            | Feature::EntityInteraction
            | Feature::RecordingAndReconstruction => Support::NotImplemented,
        }
    }
}
