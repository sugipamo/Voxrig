//! Static implementation support; not current game permissions or readiness.
use crate::MinecraftVersion;

/// Implementation status, separate from the game's existence of a feature.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum Support {
    /// Common contract implemented by this adapter.
    Available,
    /// Explicitly bounded implementation.
    Restricted(Restriction),
    /// Missing implementation in Voxrig, not proof that the game lacks it.
    NotImplemented,
}
/// The bounds of a restricted implementation. Exact preconditions are checked
/// on every action and documented in `doc`; this is not a permission decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct Restriction {
    /// One-line description of what is supported.
    pub summary: &'static str,
    /// Repository path of the document with the full contract.
    pub doc: &'static str,
}
/// Common API feature group.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Feature {
    /// Synchronous irreversible transport fencing independent of capture/writer locks.
    ConnectionRevocation,
    /// One owned dead-player respawn request and separate new-world receipt.
    Respawn,
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
    /// Retained default-item mining on audited dry geometry, with explicit stages and target receipts.
    SurvivalMining,
    /// Explicit once-only same-profile fresh recovery from retained mining.
    SurvivalMiningRecovery,
    /// One-shot passive-cube placement with separate target/material receipts.
    SurvivalPlacement,
    /// Connection-owned finite dry walking/jump dispatch under prediction contract.
    SurvivalMovement,
    /// Creative flight, default-stack writes and block interactions.
    CreativeControls,
    /// Finite ground walking/jump with flight inactive and received Creative mode.
    CreativeMovement,
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
    /// Received entity spawn/despawn lifetimes, without current motion or metadata.
    EntityObservation,
    /// Latest received entity spatial fields, independent from spawn history.
    EntityMotion,
    /// Common general-entity interaction.
    EntityInteraction,
    /// Actual own-player passenger relationships and original mounted receipts.
    VehicleObservation,
    /// Owned dismount request, actual absence, and explicit neutral input.
    VehicleDismount,
    /// Explicit local ground stop after actual dismount and neutral.
    VehicleGrounding,
    /// Finite native digital mounted-input run ending in explicit neutral.
    VehicleInput,
    /// Exact received packets, bounded without resuming after overflow.
    PacketRecording,
    /// Detached selected received-state decoding, without execution authority.
    PacketReplay,
    /// Immutable bounded known dry-terrain capture and native-model prediction.
    SurvivalScene,
    /// Received title/action-bar instructions, complete tab-list text and border fields.
    DisplayObservation,
    /// Received team declarations and scoreboard-holder membership.
    Teams,
    /// Received profile/list fields, independent of spatial player entities.
    PlayerList,
    /// Received boss-bar ADD, partial updates and REMOVE.
    BossBars,
    /// Received scoreboard declarations, displays and entries.
    Scoreboard,
    /// Named ownership of independently configured Clients.
    ClientManagement,
    /// Common recording and reconstruction API.
    RecordingAndReconstruction,
}
impl Feature {
    /// Every feature group, for discovery and tests.
    pub const ALL: &'static [Feature] = &[
        Feature::ConnectionRevocation,
        Feature::Respawn,
        Feature::Registry,
        Feature::WorldObservation,
        Feature::PlayerObservation,
        Feature::BasicControls,
        Feature::BlockTargeting,
        Feature::SurvivalTargeting,
        Feature::SurvivalPreview,
        Feature::SurvivalMining,
        Feature::SurvivalMiningRecovery,
        Feature::SurvivalPlacement,
        Feature::SurvivalMovement,
        Feature::CreativeControls,
        Feature::CreativeMovement,
        Feature::Containers,
        Feature::ContainerObservation,
        Feature::InventorySwap,
        Feature::InventoryClick,
        Feature::InventoryTransfer,
        Feature::Crafting,
        Feature::EntityObservation,
        Feature::EntityMotion,
        Feature::EntityInteraction,
        Feature::VehicleObservation,
        Feature::VehicleDismount,
        Feature::VehicleGrounding,
        Feature::VehicleInput,
        Feature::PacketRecording,
        Feature::PacketReplay,
        Feature::SurvivalScene,
        Feature::DisplayObservation,
        Feature::Teams,
        Feature::PlayerList,
        Feature::BossBars,
        Feature::Scoreboard,
        Feature::ClientManagement,
        Feature::RecordingAndReconstruction,
    ];
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
        const fn restricted(summary: &'static str, doc: &'static str) -> Support {
            Support::Restricted(Restriction { summary, doc })
        }
        match feature {
            Feature::ConnectionRevocation
            | Feature::Registry
            | Feature::WorldObservation
            | Feature::PlayerObservation
            | Feature::BasicControls => Support::Available,
            Feature::Respawn => restricted(
                "one owned request per world after received death; fresh state must be re-read",
                "docs/common-respawn.md",
            ),
            Feature::CreativeControls => restricted(
                "default items, flight steps up to 4 blocks, loaded reachable targets",
                "docs/common-creative-flight.md",
            ),
            Feature::BlockTargeting | Feature::SurvivalTargeting => restricted(
                "dry stationary standing, reach up to 4.5, audited static outlines",
                "docs/common-survival-targeting.md",
            ),
            Feature::SurvivalPreview => restricted(
                "read-only 1..120 walk/jump ticks on audited dry terrain",
                "docs/common-survival-motion.md",
            ),
            Feature::SurvivalMining => restricted(
                "default tool or empty hand on audited dry blocks; removal is not continuation",
                "docs/common-survival-mining.md",
            ),
            Feature::SurvivalMiningRecovery => restricted(
                "once-only same-profile fresh login on direct vanilla",
                "docs/common-mining-recovery.md",
            ),
            Feature::SurvivalPlacement => restricted(
                "one default passive cube with separate target and material receipts",
                "docs/common-survival-placement.md",
            ),
            Feature::InventorySwap => restricted(
                "player main/hotbar swaps with resolved item data and fresh receipts",
                "docs/common-inventory-swaps.md",
            ),
            Feature::InventoryClick => restricted(
                "ordinary PICKUP on player or audited container slots",
                "docs/common-inventory-clicks.md",
            ),
            Feature::InventoryTransfer => restricted(
                "one ordinary QUICK_MOVE on player or audited storage slots",
                "docs/common-inventory-transfers.md",
            ),
            Feature::ContainerObservation => restricted(
                "regular screens with verified storage/crafting layouts",
                "docs/common-container-observation.md",
            ),
            Feature::Containers => restricted(
                "empty-hand storage, crafting table and furnace open/close",
                "docs/common-container-open.md",
            ),
            Feature::SurvivalMovement | Feature::CreativeMovement => restricted(
                "1..120 dry walk/jump ticks; predicted completion is not server acceptance",
                "docs/common-survival-motion.md",
            ),
            Feature::Crafting => restricted(
                "received recipes, grid input clicks, result take and one-shot recipe placement",
                "docs/common-recipes.md",
            ),
            Feature::EntityObservation => restricted(
                "received spawn/despawn ledger; no metadata, hitbox or health",
                "docs/common-client-entities.md",
            ),
            Feature::EntityMotion => restricted(
                "latest received position, rotation and velocity samples",
                "docs/common-entity-motion.md",
            ),
            Feature::EntityInteraction => restricted(
                "one INTERACT or ATTACK on a received entity; no outcome ACK",
                "docs/common-client-entities.md",
            ),
            Feature::VehicleObservation => restricted(
                "own passenger relationships from received packets",
                "docs/common-vehicles.md",
            ),
            Feature::VehicleInput => restricted(
                "finite digital mounted inputs ending in neutral; no vehicle physics",
                "docs/common-vehicles.md",
            ),
            Feature::VehicleGrounding => restricted(
                "two released ground ticks after a completed dismount on dry support",
                "docs/common-dismount-grounding.md",
            ),
            Feature::VehicleDismount => restricted(
                "one dismount request, received absence, then neutral input",
                "docs/common-vehicles.md",
            ),
            Feature::PacketRecording => restricted(
                "exact receive payloads up to 16 MiB / 65536 records",
                "docs/common-recording-scenes.md",
            ),
            Feature::PacketReplay => restricted(
                "read-only decoding of a complete from-connect recording",
                "docs/common-recording-scenes.md",
            ),
            Feature::SurvivalScene => restricted(
                "immutable dry-terrain capture up to 64 cells/axis with motion prediction",
                "docs/common-recording-scenes.md",
            ),
            Feature::Teams => restricted(
                "received team declarations and membership, bounded to 4096 entries",
                "docs/common-teams-player-list.md",
            ),
            Feature::PlayerList => restricted(
                "received profiles and per-field updates, bounded to 4096 profiles",
                "docs/common-teams-player-list.md",
            ),
            Feature::DisplayObservation => restricted(
                "received titles, tab header/footer and world border fields",
                "docs/common-ui-display.md",
            ),
            Feature::BossBars => restricted(
                "received boss bars, bounded to 4096 entries",
                "docs/common-boss-bars.md",
            ),
            Feature::Scoreboard => restricted(
                "received objectives, displays and scores, bounded to 4096 entries",
                "docs/common-ui-context.md",
            ),
            Feature::ClientManagement => restricted(
                "1..64 named clients; no automatic reconnect",
                "docs/common-ui-manager.md",
            ),
            Feature::RecordingAndReconstruction => restricted(
                "raw recording, read-only replay and dry-terrain scenes",
                "docs/common-recording-scenes.md",
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restricted_features_link_existing_documents() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            for feature in Feature::ALL {
                if let Support::Restricted(restriction) =
                    Capabilities::for_version(version).support(*feature)
                {
                    assert!(restriction.summary.len() <= 100, "{feature:?}");
                    assert!(root.join(restriction.doc).is_file(), "{feature:?}");
                }
            }
        }
    }
}
