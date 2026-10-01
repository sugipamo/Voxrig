//! Coherent, generation-bound client observation snapshots.

use std::{sync::Arc, time::Instant};

use tokio::sync::oneshot;

use crate::{
    BlockCollisionShape, BlockPos, BlockRegion, ClientConnectionGeneration, EntityState, Event,
    InventoryState, ItemStack, MotionState, OpenWindow, Player, RawBlockMovementRegistryFact,
    SurvivalState, WindowProperty,
};

/// Monotonic sequence of coherent captures within one connection generation.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ObservationSequence(u64);

impl ObservationSequence {
    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the generation-local numeric value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Bounded raw observation requested from the connection actor.
#[derive(Clone, Debug, PartialEq)]
pub struct CoherentObservationRequest {
    /// Entity radius around the player, in `0..=1024`.
    pub entity_radius: f64,
    /// Maximum entities returned, in `0..=512`.
    pub max_entities: u16,
    /// Maximum queued events returned, in `0..=256`.
    pub max_events: u16,
    /// Body-owned generation of the sparse observation interest, if active.
    pub body_interest_generation: Option<u64>,
    /// Exact bounded sparse cells selected by the Body.
    pub observation_interest: Vec<BlockPos>,
}

/// Bounded raw world request for the Rust traversal provider.
///
/// This is a client integration request, not a Body or JSON wire message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TraversalMovementFactsRequest {
    /// Generation that must own the returned snapshot.
    pub expected_generation: ClientConnectionGeneration,
    /// Inclusive block region required by the bounded provider.
    pub region: BlockRegion,
    /// Radius for raw entity facts around the captured player.
    pub entity_radius: u16,
    /// Maximum number of entities retained in the snapshot.
    pub max_entities: u16,
}

/// Bounded raw traversal-geometry query for a Body-selected region.
/// This is a fact read only; the client never selects a route or action.
#[derive(Clone, Debug, PartialEq)]
pub struct TraversalGeometryQuery {
    /// Capture lineage for the read. The connection generation must still
    /// match; block-geometry and inventory revisions are the caller's source
    /// snapshot and are refreshed by the coherent query.
    pub expected_capture: SensorCaptureIdentity,
    /// Exact protocol dimension required by the Body request.
    pub expected_dimension: String,
    /// Inclusive bounded region containing only traversal-relevant geometry.
    pub region: BlockRegion,
}

impl TraversalGeometryQuery {
    pub(crate) fn validate(&self) -> crate::Result<()> {
        let axis = |min: i32, max: i32| {
            let length = i64::from(max) - i64::from(min) + 1;
            (length > 0 && length <= 16).then_some(length)
        };
        let Some(x) = axis(self.region.min.x, self.region.max.x) else {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidInput,
                anyhow::anyhow!("traversal geometry x axis exceeds its bound"),
            ));
        };
        let Some(y) = axis(self.region.min.y, self.region.max.y) else {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidInput,
                anyhow::anyhow!("traversal geometry y axis exceeds its bound"),
            ));
        };
        let Some(z) = axis(self.region.min.z, self.region.max.z) else {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidInput,
                anyhow::anyhow!("traversal geometry z axis exceeds its bound"),
            ));
        };
        if self.region.min.y < 0
            || self.region.max.y > 255
            || x.checked_mul(y).and_then(|value| value.checked_mul(z)) > Some(4096)
        {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidInput,
                anyhow::anyhow!("traversal geometry region exceeds its bound"),
            ));
        }
        if self.expected_dimension.is_empty() {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidInput,
                anyhow::anyhow!("traversal geometry dimension is empty"),
            ));
        }
        Ok(())
    }
}

/// Bounded generation-scoped query over the currently loaded packet world.
/// This returns raw matching positions and coverage only; it grants no route,
/// action, or semantic authority to the client.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedResourceQuery {
    /// Connection generation that must own the coherent snapshot.
    pub expected_generation: ClientConnectionGeneration,
    /// Exact protocol dimension name expected by the requester.
    pub expected_dimension: String,
    /// Inclusive bounded region to inspect.
    pub region: BlockRegion,
    /// Canonical protocol block name without a namespace prefix.
    pub block_name: String,
    /// Maximum matching positions retained in canonical scan order.
    pub limit: u16,
}

impl LoadedResourceQuery {
    pub(crate) fn validate(&self) -> crate::Result<()> {
        if self.block_name.is_empty()
            || self.block_name.len() > 128
            || self.block_name.chars().any(char::is_control)
            || self.limit == 0
            || usize::from(self.limit) > 4096
            || !matches!(
                self.expected_dimension.as_str(),
                "minecraft:overworld" | "minecraft:the_nether" | "minecraft:the_end"
            )
        {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidInput,
                anyhow::anyhow!("loaded resource query shape is invalid"),
            ));
        }
        let axis = |min: i32, max: i32, bound: i64| -> crate::Result<i64> {
            let length = i64::from(max) - i64::from(min) + 1;
            if length <= 0 || length > bound {
                return Err(crate::Error::new(
                    crate::ErrorKind::InvalidInput,
                    anyhow::anyhow!("loaded resource query region exceeds its bound"),
                ));
            }
            Ok(length)
        };
        let x = axis(self.region.min.x, self.region.max.x, 256)?;
        let y = axis(self.region.min.y, self.region.max.y, 256)?;
        let z = axis(self.region.min.z, self.region.max.z, 256)?;
        if self.region.min.y < 0 || self.region.max.y > 255 {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidInput,
                anyhow::anyhow!("loaded resource query vertical region is outside protocol 736"),
            ));
        }
        if x.checked_mul(y).and_then(|value| value.checked_mul(z)) > Some(4_194_304) {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidInput,
                anyhow::anyhow!("loaded resource query volume exceeds its bound"),
            ));
        }
        Ok(())
    }
}

/// Coverage of one coherent loaded-resource query.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadedResourceCoverage {
    /// Every chunk intersecting the requested horizontal region was loaded.
    Complete,
    /// At least one intersecting chunk was absent from the packet cache.
    Partial {
        /// Number of horizontally intersecting chunks absent from the cache.
        missing_chunks: u32,
    },
}

/// Raw coherent result of a [`LoadedResourceQuery`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedResourceQuerySnapshot {
    /// Exact packet-domain capture shared with other purpose-specific sensors.
    pub capture: SensorCaptureIdentity,
    /// Exact loaded-chunk coverage classification.
    pub coverage: LoadedResourceCoverage,
    /// Canonical matching positions retained up to the request limit.
    pub candidates: Vec<BlockPos>,
    /// Matching positions omitted after the request limit.
    pub omitted_candidates: u32,
}

impl TraversalMovementFactsRequest {
    const MAX_AXIS: i64 = 80;
    const MAX_VOLUME: i64 = 262_144;
    const MAX_ENTITY_RADIUS: u16 = 64;
    const MAX_ENTITIES: u16 = 512;

    pub(crate) fn validate(self) -> crate::Result<Self> {
        let axis = |min: i32, max: i32| -> crate::Result<i64> {
            let length = i64::from(max) - i64::from(min) + 1;
            if length <= 0 || length > Self::MAX_AXIS {
                return Err(crate::Error::new(
                    crate::ErrorKind::InvalidInput,
                    anyhow::anyhow!("movement facts region axis exceeds its bound"),
                ));
            }
            Ok(length)
        };
        let x = axis(self.region.min.x, self.region.max.x)?;
        let y = axis(self.region.min.y, self.region.max.y)?;
        let z = axis(self.region.min.z, self.region.max.z)?;
        if x.checked_mul(y).and_then(|value| value.checked_mul(z)) > Some(Self::MAX_VOLUME) {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidInput,
                anyhow::anyhow!("movement facts region volume exceeds its bound"),
            ));
        }
        if self.entity_radius > Self::MAX_ENTITY_RADIUS {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidInput,
                anyhow::anyhow!("movement facts entity radius exceeds its bound"),
            ));
        }
        if self.max_entities > Self::MAX_ENTITIES {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidInput,
                anyhow::anyhow!("movement facts entity limit exceeds its bound"),
            ));
        }
        Ok(self)
    }
}

/// One raw block state in a traversal provider snapshot.
#[derive(Clone, Debug, PartialEq)]
pub enum TraversalBlockFact {
    /// A loaded state with exact registry geometry and raw registry data.
    Loaded {
        /// Integer world position.
        position: BlockPos,
        /// Protocol block state ID.
        state_id: i32,
        /// Canonical registry block name.
        name: String,
        /// Exact bundled collision boxes.
        shapes: Vec<BlockCollisionShape>,
        /// Raw registry mining metadata.
        registry: RawBlockMovementRegistryFact,
        /// Canonical state properties.
        properties: Vec<(String, String)>,
    },
    /// The containing chunk was unavailable at capture time.
    Unloaded {
        /// Integer world position.
        position: BlockPos,
    },
    /// A loaded state ID was not present in the bundled registry.
    Unknown {
        /// Integer world position.
        position: BlockPos,
        /// Unrecognized protocol state ID.
        state_id: i32,
    },
}

/// One position-bearing block fact returned by the geometry-only sensor.
/// Mining and movement-registry metadata intentionally remain outside this
/// boundary.
#[derive(Clone, Debug, PartialEq)]
pub enum TraversalGeometryBlockFact {
    /// A loaded state with the geometry fields requested by the Body.
    Loaded {
        /// Integer world position.
        position: BlockPos,
        /// Protocol block state ID.
        state_id: i32,
        /// Canonical registry block name.
        name: String,
        /// Coarse collision fact derived from the version-pinned registry.
        collision: crate::BlockCollision,
        /// Full-top support geometry derived from the same registry state.
        support_surface: crate::BlockSupportSurface,
        /// Canonical state properties.
        properties: Vec<(String, String)>,
    },
    /// The containing chunk was unavailable at capture time.
    Unloaded {
        /// Integer world position.
        position: BlockPos,
    },
    /// A loaded state ID was not present in the bundled registry.
    Unknown {
        /// Integer world position.
        position: BlockPos,
        /// Unrecognized protocol state ID.
        state_id: i32,
    },
}

/// Exact raw dimensions for one known entity registry entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TraversalEntityDimensions {
    /// Registry width in blocks.
    pub width: f64,
    /// Registry height in blocks.
    pub height: f64,
}

/// One bounded entity retained with its raw registry dimensions.
#[derive(Clone, Debug, PartialEq)]
pub struct TraversalEntityFact {
    /// Packet-backed entity state.
    pub entity: EntityState,
    /// Exact registry dimensions, or `None` when the type is unknown.
    /// Consumers must fail closed when dimensions are unavailable.
    pub dimensions: Option<TraversalEntityDimensions>,
}

/// One bounded inventory slot retained as a raw tool/NBT fact.
#[derive(Clone, Debug, PartialEq)]
pub struct TraversalInventorySlotFact {
    /// Protocol window-0 slot number.
    pub slot: i16,
    /// Raw item stack, including its optional NBT payload.
    pub item: Option<ItemStack>,
}

/// Raw player inventory facts used by the provider's tool-cost port.
#[derive(Clone, Debug, PartialEq)]
pub struct TraversalInventoryFact {
    /// Selected hotbar index from the protocol cache.
    pub selected_hotbar: u8,
    /// Window-0 slots in deterministic slot order.
    pub slots: Vec<TraversalInventorySlotFact>,
}

/// One coherent bounded raw movement-facts snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct TraversalMovementFactsSnapshot {
    /// Client connection generation that produced the snapshot.
    pub generation: ClientConnectionGeneration,
    /// Generation-local capture sequence.
    pub sequence: ObservationSequence,
    /// Player state captured in the same actor turn.
    pub player: Player,
    /// Physics state captured in the same actor turn.
    pub motion: MotionState,
    /// Full packet-backed survival state, including active effects.
    pub survival: SurvivalState,
    /// Bounded region facts in lexicographic x/y/z order.
    pub blocks: Vec<TraversalBlockFact>,
    /// Entity facts retained in entity-id order.
    pub entities: Vec<TraversalEntityFact>,
    /// Number of in-radius entities omitted by the explicit bound.
    pub entities_omitted: u32,
    /// Raw window-0 inventory/tool NBT facts.
    pub inventory: TraversalInventoryFact,
}

/// Coherent packet-backed traversal geometry for exactly one bounded query.
#[derive(Clone, Debug, PartialEq)]
pub struct TraversalGeometrySnapshot {
    /// Exact packet-domain identity captured under the coherent state gate.
    pub capture: SensorCaptureIdentity,
    /// Actual player origin captured under the same coherent state gate.
    pub evaluated_origin: crate::Vec3,
    /// Lexicographically ordered facts for the exact requested region.
    pub blocks: Vec<TraversalGeometryBlockFact>,
}

/// One immutable block-state section in an all-loaded geometry snapshot.
/// State storage is shared with the live client cache through copy-on-write
/// `Arc` buffers; retaining a snapshot never makes it the live world owner.
#[derive(Clone)]
pub struct LoadedGeometrySection {
    /// Loaded chunk containing this section.
    pub(crate) chunk: crate::ChunkPos,
    /// Vertical section coordinate (`block_y.div_euclid(16)`).
    pub(crate) section_y: i32,
    /// Canonical Minecraft section order, containing exactly 4096 state IDs.
    pub(crate) state_ids: std::sync::Arc<[i32; 4096]>,
}

/// Immutable identity of the bundled registry used to interpret snapshot
/// state IDs. This is physical/catalog provenance, not Production meaning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockRegistryIdentity {
    pub(crate) protocol_version: i32,
    pub(crate) minecraft_version: &'static str,
    pub(crate) descriptor_revision: u32,
}

impl BlockRegistryIdentity {
    /// Java protocol number used by the bundled registry.
    #[must_use]
    pub const fn protocol_version(self) -> i32 {
        self.protocol_version
    }
    /// Minecraft release represented by the bundled registry.
    #[must_use]
    pub const fn minecraft_version(self) -> &'static str {
        self.minecraft_version
    }
    /// Revision of this descriptor projection shape.
    #[must_use]
    pub const fn descriptor_revision(self) -> u32 {
        self.descriptor_revision
    }
}

/// Raw catalog identity of a known block state. The name is provenance for
/// Body-owned catalog lookup and does not classify a resource or facility.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockCatalogIdentity {
    pub(crate) canonical_name: String,
    pub(crate) type_min_state_id: i32,
}

impl BlockCatalogIdentity {
    /// Canonical version-pinned registry name.
    #[must_use]
    pub fn canonical_name(&self) -> &str {
        &self.canonical_name
    }
    /// Minimum state ID of the owning block type.
    #[must_use]
    pub const fn type_min_state_id(&self) -> i32 {
        self.type_min_state_id
    }
}

/// Bounded physical and raw-registry description of one known state ID.
/// Route, hazard, resource, fuel and facility semantics remain Body-owned.
#[derive(Clone)]
pub struct BlockPhysicalDescriptor {
    pub(crate) state_id: i32,
    pub(crate) catalog: BlockCatalogIdentity,
    pub(crate) properties: Vec<(String, String)>,
    pub(crate) collision_shapes: Vec<BlockCollisionShape>,
    pub(crate) collision: crate::BlockCollision,
    pub(crate) support_surface: crate::BlockSupportSurface,
    pub(crate) movement_registry: RawBlockMovementRegistryFact,
}

impl BlockPhysicalDescriptor {
    /// Exact protocol state ID described by this value.
    #[must_use]
    pub const fn state_id(&self) -> i32 {
        self.state_id
    }
    /// Raw catalog identity for Body-side semantic lookup.
    #[must_use]
    pub fn catalog(&self) -> &BlockCatalogIdentity {
        &self.catalog
    }
    /// Canonically ordered raw state properties.
    #[must_use]
    pub fn properties(&self) -> &[(String, String)] {
        &self.properties
    }
    /// Exact finite registry collision boxes.
    #[must_use]
    pub fn collision_shapes(&self) -> &[BlockCollisionShape] {
        &self.collision_shapes
    }
    /// Coarse collision projection from the same shapes.
    #[must_use]
    pub const fn collision(&self) -> crate::BlockCollision {
        self.collision
    }
    /// Full-top support projection from the same shapes.
    #[must_use]
    pub const fn support_surface(&self) -> crate::BlockSupportSurface {
        self.support_surface
    }
    /// Raw mining and material registry facts.
    #[must_use]
    pub fn movement_registry(&self) -> &RawBlockMovementRegistryFact {
        &self.movement_registry
    }
}

impl std::fmt::Debug for BlockPhysicalDescriptor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BlockPhysicalDescriptor")
            .field("state_id", &self.state_id)
            .field("catalog", &self.catalog)
            .field("property_count", &self.properties.len())
            .field("collision_shape_count", &self.collision_shapes.len())
            .field("collision", &self.collision)
            .field("support_surface", &self.support_surface)
            .finish()
    }
}

/// Closed lookup result. An unknown bundled-registry state is never weakened
/// to air, empty collision, or a semantic block class.
#[derive(Clone, Debug)]
pub enum BlockPhysicalDescriptorLookup {
    /// The bundled registry fully described this state ID.
    Known(std::sync::Arc<BlockPhysicalDescriptor>),
    /// The state ID was absent or one required raw fact was invalid.
    UnknownStateId,
}

/// Closed lookup for the physical descriptor of a newly placed block type.
#[derive(Clone, Debug)]
pub enum PlacedBlockPhysicalDescriptorLookup {
    /// All placement-state variants share the returned planning facts.
    Known(Arc<BlockPhysicalDescriptor>),
    /// The catalog key is absent from this version-pinned registry.
    UnknownCatalog,
    /// The registry entry or default descriptor is malformed or missing.
    UnknownRegistry,
    /// Placement-state variants differ on a physical fact used by planning.
    UnsupportedVariant,
}

impl LoadedGeometrySection {
    /// Loaded chunk containing this section.
    #[must_use]
    pub const fn chunk(&self) -> crate::ChunkPos {
        self.chunk
    }

    /// Vertical section coordinate (`block_y.div_euclid(16)`).
    #[must_use]
    pub const fn section_y(&self) -> i32 {
        self.section_y
    }

    /// Immutable canonical section state IDs.
    #[must_use]
    pub fn state_ids(&self) -> &[i32; 4096] {
        &self.state_ids
    }

    /// Shares the immutable copy-on-write state buffer with an in-process
    /// consumer without materializing the section.
    #[must_use]
    pub fn state_ids_arc(&self) -> std::sync::Arc<[i32; 4096]> {
        std::sync::Arc::clone(&self.state_ids)
    }
}

impl std::fmt::Debug for LoadedGeometrySection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LoadedGeometrySection")
            .field("chunk", &self.chunk)
            .field("section_y", &self.section_y)
            .field("state_count", &self.state_ids.len())
            .finish()
    }
}

/// Immutable low-copy view of every chunk currently loaded by one client.
///
/// This is a Rust integration object, not a serialized observation contract.
/// The client remains the sole owner of live world state; consumers use this
/// value only as evidence from the captured instant. Cloning is intentionally
/// low-copy: section state arrays remain shared, while bounded manifest
/// metadata is copied for in-process handoff.
#[derive(Clone)]
pub struct LoadedGeometrySnapshot {
    /// Coherent packet-domain identity of the captured world and inventory.
    pub(crate) capture: SensorCaptureIdentity,
    /// Exact protocol dimension captured with the chunk manifest.
    pub(crate) dimension: String,
    /// Canonically sorted complete set of chunks loaded at capture time.
    pub(crate) loaded_chunks: Vec<crate::ChunkPos>,
    /// Canonically sorted present sections. Missing sections in a loaded
    /// chunk represent state ID zero (air), matching `World::block`.
    pub(crate) sections: Vec<LoadedGeometrySection>,
    /// Number of logical cells represented by `loaded_chunks`, including
    /// implicit-air sections.
    pub(crate) logical_cell_count: usize,
}

impl LoadedGeometrySnapshot {
    /// Immutable registry identity used by [`Self::descriptor`].
    #[must_use]
    pub const fn registry_identity(&self) -> BlockRegistryIdentity {
        crate::registry::BLOCK_REGISTRY_IDENTITY
    }

    /// Resolves one state ID through the exact bundled registry associated
    /// with this snapshot. Consumers must preserve `UnknownStateId`.
    #[must_use]
    pub fn descriptor(&self, state_id: i32) -> BlockPhysicalDescriptorLookup {
        crate::registry::block_physical_descriptor(state_id)
    }

    /// Resolves a version-pinned default placement descriptor only when all
    /// state variants share the physical facts consumed by planning.
    #[must_use]
    pub fn placed_descriptor(&self, canonical_name: &str) -> PlacedBlockPhysicalDescriptorLookup {
        crate::registry::placed_block_physical_descriptor(canonical_name)
    }
    /// Item capacity from the same bundled Minecraft version as this snapshot.
    /// This is definition data: the item need not be observed or held.
    #[must_use]
    pub fn item_max_stack_size(&self, canonical_name: &str) -> Option<u8> {
        let id = crate::registry::item_id(canonical_name)?;
        crate::registry::item_max_stack_size(id)
    }

    /// Coherent packet-domain identity of this evidence.
    #[must_use]
    pub const fn capture(&self) -> SensorCaptureIdentity {
        self.capture
    }

    /// Exact protocol dimension of this evidence.
    #[must_use]
    pub fn dimension(&self) -> &str {
        &self.dimension
    }

    /// Canonically sorted complete loaded-chunk manifest.
    #[must_use]
    pub fn loaded_chunks(&self) -> &[crate::ChunkPos] {
        &self.loaded_chunks
    }

    /// Canonically sorted present block-state sections.
    #[must_use]
    pub fn sections(&self) -> &[LoadedGeometrySection] {
        &self.sections
    }

    /// Number of logical cells, including implicit-air sections.
    #[must_use]
    pub const fn logical_cell_count(&self) -> usize {
        self.logical_cell_count
    }

    /// Reads one raw state ID from the immutable captured universe. `None`
    /// means its chunk was not loaded; a loaded missing section returns zero.
    #[must_use]
    pub fn state_id(&self, position: crate::BlockPos) -> Option<i32> {
        if !(0..=255).contains(&position.y) {
            return None;
        }
        let chunk = crate::ChunkPos {
            x: position.x.div_euclid(16),
            z: position.z.div_euclid(16),
        };
        self.loaded_chunks.binary_search(&chunk).ok()?;
        let section_y = position.y.div_euclid(16);
        let section = self
            .sections
            .binary_search_by_key(&(chunk, section_y), |section| {
                (section.chunk, section.section_y)
            })
            .ok()
            .map(|index| &self.sections[index]);
        let Some(section) = section else {
            return Some(0);
        };
        let local_x = usize::try_from(position.x.rem_euclid(16)).ok()?;
        let local_y = usize::try_from(position.y.rem_euclid(16)).ok()?;
        let local_z = usize::try_from(position.z.rem_euclid(16)).ok()?;
        Some(section.state_ids[local_y * 256 + local_z * 16 + local_x])
    }
}

impl std::fmt::Debug for LoadedGeometrySnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LoadedGeometrySnapshot")
            .field("capture", &self.capture)
            .field("dimension", &self.dimension)
            .field("loaded_chunk_count", &self.loaded_chunks.len())
            .field("present_section_count", &self.sections.len())
            .field("logical_cell_count", &self.logical_cell_count)
            .finish()
    }
}

/// Stable packet-domain identity shared by a coherent observation and
/// purpose-specific sensor reads. Repeated observations may have different
/// observation sequences while retaining this identity when none of the
/// relevant packet domains changed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SensorCaptureIdentity {
    /// Connection generation that owns every revision below.
    pub generation: ClientConnectionGeneration,
    /// Block/chunk geometry revision. Light and block-entity-only updates do
    /// not advance this traversal dependency.
    pub block_geometry_revision: u64,
    /// Inventory/window-domain revision.
    pub inventory_revision: u64,
}

impl Default for CoherentObservationRequest {
    fn default() -> Self {
        Self {
            entity_radius: 32.0,
            max_entities: 512,
            max_events: 256,
            body_interest_generation: None,
            observation_interest: Vec::new(),
        }
    }
}

impl CoherentObservationRequest {
    pub(crate) fn validate(self) -> crate::Result<Self> {
        if !self.entity_radius.is_finite() || !(0.0..=1024.0).contains(&self.entity_radius) {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidInput,
                anyhow::anyhow!(
                    "coherent observation entity radius must be finite and within 0..=1024"
                ),
            ));
        }
        if self.max_entities > 512 {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidInput,
                anyhow::anyhow!("coherent observation entity limit must be at most 512"),
            ));
        }
        if self.max_events > 256 {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidInput,
                anyhow::anyhow!("coherent observation event limit must be at most 256"),
            ));
        }
        let mut positions =
            std::collections::HashSet::with_capacity(self.observation_interest.len());
        for position in &self.observation_interest {
            if !positions.insert((position.x, position.y, position.z)) {
                return Err(crate::Error::new(
                    crate::ErrorKind::InvalidInput,
                    anyhow::anyhow!("coherent observation interest positions must be unique"),
                ));
            }
        }
        if self.body_interest_generation.is_none() && !self.observation_interest.is_empty() {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidInput,
                anyhow::anyhow!("coherent observation interest cells require a generation"),
            ));
        }
        Ok(self)
    }
}

/// One sparse Body-requested world cell captured without client-side selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoherentInterestCell {
    /// Exact requested position.
    pub position: BlockPos,
    /// Raw block-state ID, or `None` when its chunk was unavailable.
    pub state_id: Option<i32>,
    /// Packet-completeness-backed light at the same position.
    pub light: CoherentLightState,
}

/// Body-selected sparse observation returned in full or rejected before capture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoherentObservationInterest {
    /// Body-owned interest generation copied without reinterpretation.
    pub body_generation: Option<u64>,
    /// Cells in the exact request order.
    pub cells: Vec<CoherentInterestCell>,
    /// Requested cells whose chunks were unavailable.
    pub unloaded_cells: u32,
    /// Requested cells without a complete block/sky light pair.
    pub light_unknown_cells: u32,
}

/// A furnace window correlated by the connection actor with the preceding
/// exact block interaction. The correlation is raw packet context only; it
/// does not establish a semantic furnace transition result.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenFurnaceObservation {
    /// Block position supplied by the Body's block interaction.
    pub position: BlockPos,
    /// Raw open-window header.
    pub window: OpenWindow,
    /// Window slots in their protocol order.
    pub slots: Vec<Option<ItemStack>>,
    /// Raw furnace properties in deterministic property-key order.
    pub properties: Vec<WindowProperty>,
}

/// Packet-backed world clock captured in the same actor turn as the other
/// observation domains.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoherentWorldTime {
    /// Server world age in ticks.
    pub world_age: i64,
    /// Server time of day in ticks.
    pub time_of_day: i64,
    /// Gamerule daylight-cycle evidence, when supplied by the server.
    pub daylight_cycle: Option<bool>,
}

impl CoherentWorldTime {
    /// Converts the signed Java 1.16.1 time-update value to the Body clock.
    ///
    /// The protocol uses a negative time-of-day to freeze the daylight cycle;
    /// the magnitude is still the displayed time. The conversion therefore
    /// uses the absolute value before reducing it to one Minecraft day.
    #[must_use]
    pub fn contract_time_of_day(self) -> Option<(u16, bool)> {
        let magnitude = self.time_of_day.checked_abs()?;
        let normalized = u16::try_from(magnitude % 24_000).ok()?;
        Some((normalized, self.time_of_day >= 0))
    }
}

/// Packet-completeness-backed light evidence for one world cell.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoherentLightState {
    /// At least one required light section/value has not been observed.
    Unknown,
    /// Both vanilla light nibbles were observed.
    Observed {
        /// Block-emitted light in `0..=15`.
        block: u8,
        /// Sky light in `0..=15`.
        sky: u8,
    },
}

/// One atomic raw client observation.
///
/// This is an internal Rust integration contract, not the public Zen Body or
/// JSON wire contract. The Body assigns its own `StateRevision` only after
/// validating and accepting one value of this type.
#[derive(Clone, Debug, PartialEq)]
pub struct CoherentObservation {
    /// Exact transport connection that produced this snapshot.
    pub generation: ClientConnectionGeneration,
    /// Monotonic sequence scoped to `generation`.
    pub sequence: ObservationSequence,
    /// Packet-domain identity used to correlate purpose-specific sensors.
    pub sensor_capture: SensorCaptureIdentity,
    /// Monotonic capture time in this process.
    pub received_at: Instant,
    /// Local player state.
    pub player: Player,
    /// Client physics state.
    pub motion: MotionState,
    /// Packet-backed survival state.
    pub survival: SurvivalState,
    /// Raw air-supply level when a valid entity metadata value was observed.
    pub oxygen_level: Option<u8>,
    /// Local pose and eye-fluid facts captured with this world/player snapshot.
    pub mining_environment: crate::mining_environment::MiningEnvironment,
    /// Packet-backed world clock, or `None` before a time-update packet.
    pub world_time: Option<CoherentWorldTime>,
    /// Packet-backed inventory and window state.
    pub inventory: InventoryState,
    /// Convenience clone of the active window from `inventory`.
    pub open_window: Option<OpenWindow>,
    /// Furnace window correlated with the preceding exact block interaction.
    pub open_furnace: Option<OpenFurnaceObservation>,
    /// Bounded block observation.
    /// Exact sparse observation interest selected by the Body.
    pub observation_interest: CoherentObservationInterest,
    /// Deterministically ordered bounded entities.
    pub entities: Vec<EntityState>,
    /// Valid in-radius entities omitted by `max_entities`.
    pub entities_omitted: u32,
    /// Events drained at this actor capture boundary.
    pub events: Vec<Event>,
    /// Events omitted because the bounded connection queue overflowed or the
    /// request asked for fewer events than were available.
    pub events_omitted: u32,
    /// Events lost when the connection-owned bounded queue overflowed.
    pub events_queue_omitted: u32,
    /// Events intentionally omitted because this request selected a smaller bound.
    pub events_request_omitted: u32,
}

pub(crate) struct CaptureCommand {
    pub(crate) request: CoherentObservationRequest,
    pub(crate) reply: oneshot::Sender<crate::Result<CoherentObservation>>,
}

pub(crate) struct TraversalMovementFactsCommand {
    pub(crate) request: TraversalMovementFactsRequest,
    pub(crate) reply: oneshot::Sender<crate::Result<TraversalMovementFactsSnapshot>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(region: BlockRegion) -> TraversalMovementFactsRequest {
        TraversalMovementFactsRequest {
            expected_generation: ClientConnectionGeneration::allocate(),
            region,
            entity_radius: 32,
            max_entities: 512,
        }
    }

    #[test]
    fn movement_facts_request_rejects_overlarge_axes_volume_and_entities() {
        let generation = ClientConnectionGeneration::allocate();
        let valid = TraversalMovementFactsRequest {
            expected_generation: generation,
            region: BlockRegion::new(
                BlockPos {
                    x: -39,
                    y: 0,
                    z: -39,
                },
                BlockPos {
                    x: 40,
                    y: 39,
                    z: 40,
                },
            ),
            entity_radius: 64,
            max_entities: 512,
        };
        assert!(valid.validate().is_ok());

        assert!(
            request(BlockRegion::new(
                BlockPos { x: 0, y: 0, z: 0 },
                BlockPos { x: 80, y: 0, z: 0 },
            ))
            .validate()
            .is_err()
        );
        assert!(
            request(BlockRegion::new(
                BlockPos { x: 0, y: 0, z: 0 },
                BlockPos {
                    x: 79,
                    y: 40,
                    z: 79
                },
            ))
            .validate()
            .is_err()
        );
        let mut invalid = request(BlockRegion::new(
            BlockPos { x: 0, y: 0, z: 0 },
            BlockPos { x: 0, y: 0, z: 0 },
        ));
        invalid.entity_radius = 65;
        assert!(invalid.validate().is_err());
        invalid.entity_radius = 32;
        invalid.max_entities = 513;
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn movement_facts_snapshot_variants_keep_unknown_and_unloaded_distinct() {
        let position = BlockPos { x: 1, y: 2, z: 3 };
        let unloaded = TraversalBlockFact::Unloaded { position };
        let unknown = TraversalBlockFact::Unknown {
            position,
            state_id: i32::MAX,
        };
        assert_ne!(unloaded, unknown);
    }

    fn geometry_query(region: BlockRegion) -> TraversalGeometryQuery {
        TraversalGeometryQuery {
            expected_capture: SensorCaptureIdentity {
                generation: ClientConnectionGeneration::allocate(),
                block_geometry_revision: 0,
                inventory_revision: 0,
            },
            expected_dimension: "minecraft:overworld".to_owned(),
            region,
        }
    }

    #[test]
    fn geometry_query_rejects_invalid_dimension_and_region_values() {
        let valid = geometry_query(BlockRegion::new(
            BlockPos {
                x: -1,
                y: 63,
                z: -1,
            },
            BlockPos { x: 1, y: 65, z: 1 },
        ));
        assert!(valid.validate().is_ok());

        let mut invalid = valid.clone();
        invalid.expected_dimension.clear();
        assert!(invalid.validate().is_err());
        invalid = geometry_query(BlockRegion::new(
            BlockPos { x: 0, y: 0, z: 0 },
            BlockPos { x: 16, y: 0, z: 0 },
        ));
        assert!(invalid.validate().is_err());
        invalid = geometry_query(BlockRegion::new(
            BlockPos { x: 0, y: 0, z: 0 },
            BlockPos {
                x: 15,
                y: 16,
                z: 15,
            },
        ));
        assert!(invalid.validate().is_err());
        invalid = geometry_query(BlockRegion::new(
            BlockPos { x: 0, y: -1, z: 0 },
            BlockPos { x: 0, y: 0, z: 0 },
        ));
        assert!(invalid.validate().is_err());
        invalid = geometry_query(BlockRegion::new(
            BlockPos { x: 0, y: 255, z: 0 },
            BlockPos { x: 0, y: 256, z: 0 },
        ));
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn geometry_fact_surface_has_only_position_state_collision_and_properties() {
        let loaded = TraversalGeometryBlockFact::Loaded {
            position: BlockPos { x: 1, y: 2, z: 3 },
            state_id: 1,
            name: "stone".to_owned(),
            collision: crate::BlockCollision::NonEmpty,
            support_surface: crate::BlockSupportSurface::FullTop,
            properties: Vec::new(),
        };
        let unloaded = TraversalGeometryBlockFact::Unloaded {
            position: BlockPos { x: 1, y: 2, z: 3 },
        };
        let unknown = TraversalGeometryBlockFact::Unknown {
            position: BlockPos { x: 1, y: 2, z: 3 },
            state_id: i32::MAX,
        };
        assert_ne!(loaded, unloaded);
        assert_ne!(unloaded, unknown);
    }
}
