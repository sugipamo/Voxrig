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
    /// Synchronous irreversible transport fencing independent of capture/writer locks.
    ConnectionRevocation,
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
    /// Received entity spawn/despawn lifetimes, without current motion or metadata.
    EntityObservation,
    /// Common general-entity interaction.
    EntityInteraction,
    /// Actual own-player passenger relationships and original mounted receipts.
    VehicleObservation,
    /// Owned dismount request, actual absence, and explicit neutral input.
    VehicleDismount,
    /// Exact received packets, bounded without resuming after overflow.
    PacketRecording,
    /// Detached selected received-state decoding, without execution authority.
    PacketReplay,
    /// Immutable bounded known dry-terrain capture and native-model prediction.
    SurvivalScene,
    /// Received scoreboard declarations, displays and entries.
    Scoreboard,
    /// Named ownership of independently configured Clients.
    ClientManagement,
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
            Feature::ConnectionRevocation
            | Feature::Registry
            | Feature::WorldObservation
            | Feature::PlayerObservation
            | Feature::BasicControls => Support::Available,
            Feature::CreativeControls => Support::Restricted(
                "default items; permitted flight steps <=4 blocks; loaded reachable targets",
            ),
            Feature::BlockTargeting | Feature::SurvivalTargeting => Support::Restricted(
                "healthy dry stationary normal posture, matching handle mode; reach <=4.5; audited static outlines; legacy passive full cubes, registered dry slabs/stairs, storage, crafting table and furnace; animated/world-dependent shapes remain incomplete",
            ),
            Feature::SurvivalPreview => Support::Restricted(
                "read-only 1..120 walking/jump inputs; healthy stationary normal posture; native defaults; loaded passive dry full cubes and originally registered dry slabs/stairs",
            ),
            Feature::SurvivalMining => Support::Restricted(
                "healthy dry standing; received default selected stack or empty hand and empty cursor; audited dry cubes/slabs/stairs; default native tool speed/gate and received target block tags; durability-only item data; retained explicit commands/target conflicts; removal does not permit continuation",
            ),
            Feature::SurvivalMiningRecovery => Support::Restricted(
                "direct unmodified vanilla; exclusively owned same offline profile/endpoint/version; closed source; once-only login claim; fresh join/identity/dry standing/received player inventory/target; old source remains blocked",
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
                "empty-hand audited storage/crafting-table/furnace-family activation with distinct dispatch/OPEN/full/cursor/modern processing facts; opening-bound close with observed resolved-data cursor return; constructor-verified ordinary slots and furnace PICKUP roles with received native fuel-tag guard/bucket capacity/output refusal; SWAP/QUICK_MOVE remain storage/player only; smelting/XP prediction, custom fuel rules and other special windows remain incomplete",
            ),
            Feature::SurvivalMovement | Feature::CreativeMovement => Support::Restricted(
                "1..120 dry walking/jump ticks with released-rest endpoint; retained intent/failure; predicted completion is not received acceptance",
            ),
            Feature::CheckedSurvival => match self.version {
                MinecraftVersion::Java1_16_1 => Support::NotImplemented,
                MinecraftVersion::Java1_21_11 => Support::Restricted(
                    "audited dry full-cube movement; plain inventory swaps; passive-cube placement; empty-hand dirt/stone mining and fresh recovery",
                ),
            },
            Feature::Crafting => Support::Restricted(
                "received recipe declarations/displays/book membership and player/table input grids with coherent registry/tag ownership; empty-hand table activation, ordinary input PICKUP and opening-bound close with cursor return; native input disposal on close is not locally predicted or a close ACK; empty or compatible actual held-cursor result PICKUP requires the entire result within effective cursor capacity and retains fresh combined cursor/full-grid consumption/remainders receipts; coherent Next/Maximum recipe planning and owned one-shot placement release only after actual grid/inventory conservation; actual original-UI ghost displays and safe material-shortage requests retain conserved empty inputs separately from placement/output; modern ghost responses have no recipe ID; player submitted-close basis can progress to actual received player zero without rebinding a table; shift-crafting remains incomplete",
            ),
            Feature::EntityObservation => Support::Restricted(
                "received spawn/despawn ledger; version-bound entity types and original spawn coordinates; opaque connection/world/spawn identity; current motion, metadata, hitboxes and health remain incomplete",
            ),
            Feature::EntityInteraction => Support::Restricted(
                "one native INTERACT or ATTACK on an original received lifetime; matching received handle mode; no auto-selection, cooldown, retry, reach/visibility proof or outcome ACK; position-specific interaction and broader entity state remain incomplete",
            ),
            Feature::VehicleObservation => Support::Restricted(
                "actual own-player passenger lists; unknown before applicable receipt, explicit same-vehicle absence after continuous mounted lifetime; source ordinal and optional original spawn lifetime; no current motion/vehicle physics or ground admission",
            ),
            Feature::VehicleDismount => Support::Restricted(
                "one actor-owned request on an original received mount, actual same-vehicle absence, then one explicit neutral input; matching live received mode/world, retained cancellation/closure/uncertain-I/O history and no replay; no causal server ACK, vehicle physics/control or ground continuation",
            ),
            Feature::PacketRecording => Support::Restricted(
                "exact configuration/play receive payloads and local position decoder inputs; 16MiB/65536 records; no authentication packets; from-connect capture is required for replay",
            ),
            Feature::PacketReplay => Support::Restricted(
                "complete from-connect history; exact native version decoder; selected received player/inventory/block facts only; legacy unhandled IDs explicit; saved facts never restore execution IDs or a Client",
            ),
            Feature::SurvivalScene => Support::Restricted(
                "immutable <=64 cells/axis and <=32768 loaded air/passive dry cubes/registered dry slabs/stairs; healthy stationary Survival defaults and complete standing halo; detached 1..120 input prediction without dispatch; edits/chaining remain version-specific",
            ),
            Feature::Scoreboard => Support::Restricted(
                "received objective/display/score/reset facts and raw legacy JSON/modern NBT presentation; <=4096 entries; not a complete server catalogue or renderer; other UI remains incomplete",
            ),
            Feature::ClientManagement => Support::Restricted(
                "explicit 1..64 named active/pending Clients; per-connection config/version/cache/registry; cancellation releases reservation; terminal shutdown closes external clones and pending connects; no automatic reconnect or event aggregation",
            ),
            Feature::RecordingAndReconstruction => Support::Restricted(
                "bounded raw receive recording and selected read-only decoder replay; live known dry-terrain scene capture/prediction; broader reconstruction/piston/history contracts remain version-specific",
            ),
        }
    }
}
