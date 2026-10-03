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
    /// Creative flight, default-stack writes and block interactions.
    CreativeControls,
    /// Extra audited dry-cube survival contract.
    CheckedSurvival,
    /// Common container click operations.
    Containers,
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
            Feature::CheckedSurvival => match self.version {
                MinecraftVersion::Java1_16_1 => Support::NotImplemented,
                MinecraftVersion::Java1_21_11 => Support::Restricted(
                    "audited dry full-cube movement; plain inventory swaps; passive-cube placement; empty-hand dirt/stone mining and fresh recovery",
                ),
            },
            Feature::Containers
            | Feature::Crafting
            | Feature::EntityInteraction
            | Feature::RecordingAndReconstruction => Support::NotImplemented,
        }
    }
}
