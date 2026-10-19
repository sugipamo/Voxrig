//! Immutable received player facts at one coherent packet boundary.
//!
//! Receipts record local application, not server currentness, liveness, action
//! acceptance or permission. Copies preserve historical evidence. Missing and
//! invalidated facts never count as received coverage. This passive API has no
//! freshness policy and does not advance the connection.
use super::{
    GameMode, Hand, Health, PlayerAbilities, PlayerEffect, ReceivedSlot, SessionStamp, WorldTime,
    container::{ScreenId, ScreenLayout, ScreenTitle},
    control::Modifier,
    registry::{ServerRegistryObservation, ServerRegistryStamp},
};
use std::{collections::BTreeMap, sync::Arc, time::Instant};
/// Original live packet application metadata. Reads and local predictions never refresh it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Receipt {
    pub(crate) session: SessionStamp, // original apply session, not the capture's session
    pub(crate) ordinal: u64,          // original connection packet ordinal
    pub(crate) applied_at: Instant,   // original live receipt-state commit
}
impl Receipt {
    /// Session.
    pub fn session(&self) -> SessionStamp {
        self.session
    }
    /// Ordinal.
    pub fn ordinal(&self) -> u64 {
        self.ordinal
    }
    /// Applied at.
    pub fn applied_at(&self) -> Instant {
        self.applied_at
    }
}

/// One supported packet value paired with its original receipt.
#[derive(Clone, Debug)]
pub struct FieldReceipt<T> {
    pub(crate) value: T,
    pub(crate) receipt: Receipt,
}
impl<T> FieldReceipt<T> {
    /// Value.
    pub fn value(&self) -> &T {
        &self.value
    }
    /// Receipt.
    pub fn receipt(&self) -> Receipt {
        self.receipt
    }
}

/// Connection receive high-water mark at capture; not per-field freshness.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReceiveBoundary {
    pub(crate) session: SessionStamp, // current world at the coherent capture boundary
    pub(crate) ordinal: u64,          // connection receive high-water mark, not field freshness
}
impl ReceiveBoundary {
    /// Session.
    pub fn session(&self) -> SessionStamp {
        self.session
    }
    /// Ordinal.
    pub fn ordinal(&self) -> u64 {
        self.ordinal
    }
}

/// Eligible received coverage of the accessor's explicitly scoped domain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Coverage {
    /// No eligible received constituents.
    Unknown,
    /// Some eligible knowledge, without a complete domain fence.
    Partial,
    /// Every constituent in this accessor's documented finite scope qualifies.
    Complete,
}

/// Supplied attribute base and complete modifier list, without a computed effective value.
#[derive(Clone, Debug)]
pub struct ReceivedAttribute {
    pub(crate) base: f64,
    pub(crate) modifiers: Vec<Modifier>, // complete supported modifier list from this update
}
impl ReceivedAttribute {
    /// Base.
    pub fn base(&self) -> f64 {
        self.base
    }
    /// Modifiers.
    pub fn modifiers(&self) -> &[Modifier] {
        &self.modifiers
    }
}

/// Literal correction operand, before baseline arithmetic or angle normalization.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum CorrectionComponent<T> {
    /// Literal absolute component.
    Absolute(T), // literal packet component, before model normalization
    /// Literal relative delta; no absolute receipt is implied.
    Relative(T), // literal packet delta; not an absolute position/angle receipt
}

/// Original correction packet operands. Relative components do not establish absolute samples.
#[derive(Clone, Debug)]
pub struct ReceivedCorrectionFields {
    pub(crate) teleport_id: i32,
    pub(crate) position: [CorrectionComponent<f64>; 3], // x, y, z
    pub(crate) rotation: [CorrectionComponent<f32>; 2], // yaw, pitch in packet degrees
    pub(crate) delta_movement: Option<[f64; 3]>,        // literal modern fields; absent in legacy
    pub(crate) native_flags: u32, // unmodified flags, interpreted for boundary.session.version
}
impl ReceivedCorrectionFields {
    /// Teleport id.
    pub fn teleport_id(&self) -> i32 {
        self.teleport_id
    }
    /// Position.
    pub fn position(&self) -> &[CorrectionComponent<f64>; 3] {
        &self.position
    }
    /// Rotation.
    pub fn rotation(&self) -> &[CorrectionComponent<f32>; 2] {
        &self.rotation
    }
    /// Delta movement.
    pub fn delta_movement(&self) -> Option<[f64; 3]> {
        self.delta_movement
    }
    /// Native flags.
    pub fn native_flags(&self) -> u32 {
        self.native_flags
    }
}

/// World and dimension type explicitly selected by an entry packet.
#[derive(Clone, Debug)]
pub struct ReceivedDimensionSelection {
    pub(crate) world_name: String,
    pub(crate) dimension_type_name: Option<String>, // Some only for legacy selection
    pub(crate) dimension_type_id: Option<i32>,      // Some only for modern selection
}
impl ReceivedDimensionSelection {
    /// World name.
    pub fn world_name(&self) -> &str {
        &self.world_name
    }
    /// Dimension type name.
    pub fn dimension_type_name(&self) -> Option<&str> {
        self.dimension_type_name.as_deref()
    }
    /// Dimension type id.
    pub fn dimension_type_id(&self) -> Option<i32> {
        self.dimension_type_id
    }
}

/// Received selected type geometry with its original configuration registry owner.
#[derive(Clone, Debug)]
pub struct ReceivedDimensionGeometry {
    pub(crate) min_y: i32,
    pub(crate) height: i32,
    pub(crate) registry_owner: ServerRegistryStamp,
}
impl ReceivedDimensionGeometry {
    /// Min y.
    pub fn min_y(&self) -> i32 {
        self.min_y
    }
    /// Height.
    pub fn height(&self) -> i32 {
        self.height
    }
    /// Registry owner.
    pub fn registry_owner(&self) -> ServerRegistryStamp {
        self.registry_owner
    }
}
/// Knowledge in the current evidence scope. Only Received contributes to coverage.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum FieldState<T> {
    /// No qualifying receipt and no recorded invalidation in this scope.
    Missing,
    /// Supported value with its original packet receipt.
    Received(FieldReceipt<T>),
    /// Eligibility was withdrawn by the recorded event.
    Invalidated(Invalidation),
}

/// Event that withdrew eligibility. Future reasons require conservative handling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum InvalidationReason {
    /// Replacement entry or other world evidence boundary.
    SessionBoundary, // replacement Join/LOGIN, or another world boundary
    /// Every respawn, including same-world and copied native metadata.
    RespawnReset, // every respawn, including same-world / copy-metadata
    /// Modern reconfiguration or replacement configuration ownership.
    Reconfiguration, // START_CONFIGURATION and replacement configuration owner
    /// Local irreversible connection revocation.
    Revoke,
    /// Local or observed transport end.
    Disconnect,
    /// Packet application or transport reader failure.
    ReaderFailure,
    /// Opening supersession or local close withdrew screen knowledge.
    ScreenChanged, // opening/local close/supersession loses domain knowledge
    /// A supplied replacement cannot be represented completely.
    UnsupportedPayload,
    /// A required registry configuration owner was replaced.
    RegistryOwnerChanged,
    /// Literal relative operands cannot establish the requested absolute projection.
    ProjectionUnavailable, // e.g. relative operands cannot establish an absolute sample
    /// Evidence withdrawn without a supported negative-value mapping.
    ExplicitRemoval, // evidence withdrawn without a supported absence-value mapping
}

/// Original loss-of-evidence event. Local events have neither packet ordinal nor apply time.
#[derive(Clone, Debug)]
pub struct Invalidation {
    pub(crate) reason: InvalidationReason,
    pub(crate) invalidated_session: SessionStamp,
    pub(crate) event_session: SessionStamp,
    pub(crate) event_ordinal: Option<u64>,
    pub(crate) event_applied_at: Option<Instant>,
}
impl Invalidation {
    /// Reason.
    pub fn reason(&self) -> InvalidationReason {
        self.reason
    }
    /// Invalidated session.
    pub fn invalidated_session(&self) -> SessionStamp {
        self.invalidated_session
    }
    /// Event session.
    pub fn event_session(&self) -> SessionStamp {
        self.event_session
    }
    /// Event ordinal.
    pub fn event_ordinal(&self) -> Option<u64> {
        self.event_ordinal
    }
    /// Event applied at.
    pub fn event_applied_at(&self) -> Option<Instant> {
        self.event_applied_at
    }
}
/// Explicit received presence or removal, never absence inferred from map omission.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Presence<T> {
    /// Explicit received positive sample.
    Present(T),
    /// Explicit supported removal, including removal of an unseen key.
    Absent, // an explicit supported removal, never inferred from map omission
}

/// Received container status, with an exact opening identity when observed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ContainerStatus {
    /// A received Java 1.16.1 Join Game or Respawn packet reset the client to its player inventory; no foreign container has been observed open since that receipt.
    NoOpening,
    /// This exact opening was received.
    Open(ScreenId),
    /// A server packet closed this exact observed opening.
    Closed(ScreenId), // this exact observed opening was closed by a server packet
}
/// Owned coherent player evidence. Copies are historical and cannot prove connection liveness.
#[derive(Clone, Debug)]
pub struct ReceivedPlayerSnapshot {
    last_open: Option<ScreenId>,
    pub(crate) boundary: ReceiveBoundary,
    pub(crate) captured_at: Instant,
    pub(crate) health: FieldState<Health>,
    pub(crate) correction: FieldState<ReceivedCorrectionFields>,
    pub(crate) absolute_position: FieldState<[f64; 3]>,
    pub(crate) absolute_rotation: FieldState<[f32; 2]>,
    pub(crate) selected_hotbar: FieldState<u8>,
    pub(crate) game_mode: FieldState<GameMode>,
    pub(crate) abilities: FieldState<PlayerAbilities>,
    pub(crate) using_item: FieldState<Option<Hand>>,
    pub(crate) air_supply: FieldState<i32>,
    pub(crate) fall_flying: FieldState<bool>,
    pub(crate) world_time: FieldState<WorldTime>,
    pub(crate) entity_id: FieldState<i32>,
    pub(crate) dimension_selection: FieldState<ReceivedDimensionSelection>,
    pub(crate) dimension_geometry: FieldState<ReceivedDimensionGeometry>,
    pub(crate) attributes: BTreeMap<String, FieldState<Presence<ReceivedAttribute>>>,
    pub(crate) attribute_fallback: FieldState<Presence<ReceivedAttribute>>,
    pub(crate) effects: BTreeMap<String, FieldState<Presence<PlayerEffect>>>, // bounded K_v
    pub(crate) attributes_coverage: Coverage,
    pub(crate) effects_coverage: Coverage,
    pub(crate) registries: Arc<ServerRegistryObservation>,
    pub(crate) inventory: ReceivedInventorySnapshot,
    pub(crate) container_state: FieldState<ContainerStatus>,
    pub(crate) container: Option<ReceivedContainerSnapshot>, // details only for matching Open
}
impl ReceivedPlayerSnapshot {
    /// Boundary.
    pub fn boundary(&self) -> ReceiveBoundary {
        self.boundary
    }
    /// Local capture time, independent of every original receipt apply time.
    pub fn captured_at(&self) -> Instant {
        self.captured_at
    }
    /// Health.
    pub fn health(&self) -> &FieldState<Health> {
        &self.health
    }
    /// Correction.
    pub fn correction(&self) -> &FieldState<ReceivedCorrectionFields> {
        &self.correction
    }
    /// Literal position only when all three components of the latest correction are absolute.
    pub fn absolute_position(&self) -> &FieldState<[f64; 3]> {
        &self.absolute_position
    }
    /// Literal angles only when both latest correction angles are absolute; no normalization is applied.
    pub fn absolute_rotation(&self) -> &FieldState<[f32; 2]> {
        &self.absolute_rotation
    }
    /// Selected hotbar.
    pub fn selected_hotbar(&self) -> &FieldState<u8> {
        &self.selected_hotbar
    }
    /// Game mode.
    pub fn game_mode(&self) -> &FieldState<GameMode> {
        &self.game_mode
    }
    /// Abilities.
    pub fn abilities(&self) -> &FieldState<PlayerAbilities> {
        &self.abilities
    }
    /// Using item.
    pub fn using_item(&self) -> &FieldState<Option<Hand>> {
        &self.using_item
    }
    /// Air supply.
    pub fn air_supply(&self) -> &FieldState<i32> {
        &self.air_supply
    }
    /// Fall flying.
    pub fn fall_flying(&self) -> &FieldState<bool> {
        &self.fall_flying
    }
    /// World time.
    pub fn world_time(&self) -> &FieldState<WorldTime> {
        &self.world_time
    }
    /// Entity id.
    pub fn entity_id(&self) -> &FieldState<i32> {
        &self.entity_id
    }
    /// Dimension selection.
    pub fn dimension_selection(&self) -> &FieldState<ReceivedDimensionSelection> {
        &self.dimension_selection
    }
    /// Dimension geometry.
    pub fn dimension_geometry(&self) -> &FieldState<ReceivedDimensionGeometry> {
        &self.dimension_geometry
    }
    /// Supplied key state, or the shared Missing/Invalidated fallback. Omission never proves absence.
    pub fn attribute(&self, name: &str) -> &FieldState<Presence<ReceivedAttribute>> {
        self.attributes
            .get(name)
            .unwrap_or(&self.attribute_fallback)
    }
    /// Attributes.
    pub fn attributes(
        &self,
    ) -> impl Iterator<Item = (&str, &FieldState<Presence<ReceivedAttribute>>)> + '_ {
        self.attributes
            .iter()
            .map(|(name, state)| (name.as_str(), state))
    }
    /// State for a supported canonical effect name; unsupported names return InvalidInput.
    pub fn effect(&self, name: &str) -> crate::Result<&FieldState<Presence<PlayerEffect>>> {
        self.effects
            .get(name)
            .ok_or_else(|| super::registry::invalid("unsupported effect name"))
    }
    /// Finite supported catalog in key order, including Missing, Invalidated and removal tombstones.
    pub fn effects(
        &self,
    ) -> impl Iterator<Item = (&str, &FieldState<Presence<PlayerEffect>>)> + '_ {
        self.effects
            .iter()
            .map(|(name, state)| (name.as_str(), state))
    }
    /// Container state.
    pub fn container_state(&self) -> &FieldState<ContainerStatus> {
        &self.container_state
    }
    /// Container.
    pub fn container(&self) -> Option<&ReceivedContainerSnapshot> {
        self.container.as_ref()
    }
    /// Inventory.
    pub fn inventory(&self) -> &ReceivedInventorySnapshot {
        &self.inventory
    }
    /// Registry state.
    pub fn registry_state(&self) -> &ServerRegistryObservation {
        &self.registries
    }
    /// Whole server attribute set: Unknown before an eligible delta, then Partial; never Complete.
    pub fn attributes_coverage(&self) -> Coverage {
        self.attributes_coverage
    }
    /// Whole server effect set: Unknown before an eligible addition or removal, then Partial; never Complete.
    pub fn effects_coverage(&self) -> Coverage {
        self.effects_coverage
    }
    /// Coverage of selection plus valid configuration-owned selected geometry.
    pub fn dimension_descriptor_coverage(&self) -> Coverage {
        if !is_received(&self.dimension_selection) {
            Coverage::Unknown
        } else if is_received(&self.dimension_geometry) {
            Coverage::Complete
        } else {
            Coverage::Partial
        }
    }
}

/// Canonical player slots 0..45, independent cursor, full-packet marker and player revision.
#[derive(Clone, Debug)]
pub struct ReceivedInventorySnapshot {
    pub(crate) slots: [FieldState<ReceivedSlot>; 46],
    pub(crate) cursor: FieldState<ReceivedSlot>,
    pub(crate) last_full_contents: FieldState<()>,
    pub(crate) player_revision: FieldState<i32>,
}
impl ReceivedInventorySnapshot {
    /// One slot; out-of-bounds indices return InvalidInput. Empty, Missing and Invalidated stay distinct.
    pub fn slot(&self, index: usize) -> crate::Result<&FieldState<ReceivedSlot>> {
        self.slots
            .get(index)
            .ok_or_else(|| super::registry::invalid("slot index outside snapshot"))
    }
    /// Cursor.
    pub fn cursor(&self) -> &FieldState<ReceivedSlot> {
        &self.cursor
    }
    /// Latest validated supported full-packet marker. Single-slot updates never establish it.
    pub fn last_full_contents(&self) -> &FieldState<()> {
        &self.last_full_contents
    }
    /// Player revision.
    pub fn player_revision(&self) -> &FieldState<i32> {
        &self.player_revision
    }
    /// Coverage of exactly canonical player slots 0..45, counting received Empty values.
    pub fn slots_coverage(&self) -> Coverage {
        coverage(self.slots.iter().map(is_received))
    }
    /// Coverage of exactly the 46 canonical player slots plus cursor.
    pub fn bundle_coverage(&self) -> Coverage {
        coverage(
            self.slots
                .iter()
                .chain(std::iter::once(&self.cursor))
                .map(is_received),
        )
    }
}

/// Details for the matching received Open status, with constructor-scoped contents.
#[derive(Clone, Debug)]
pub struct ReceivedContainerSnapshot {
    pub(crate) id: ScreenId,
    pub(crate) opened: Receipt,
    pub(crate) title: FieldReceipt<ScreenTitle>,
    pub(crate) layout: Option<ScreenLayout>,
    pub(crate) slots: Vec<FieldState<ReceivedSlot>>,
    pub(crate) last_full_contents: FieldState<()>,
    pub(crate) revision: FieldState<i32>,
}
impl ReceivedContainerSnapshot {
    /// Id.
    pub fn id(&self) -> ScreenId {
        self.id
    }
    /// Opened.
    pub fn opened(&self) -> Receipt {
        self.opened
    }
    /// Title.
    pub fn title(&self) -> &FieldReceipt<ScreenTitle> {
        &self.title
    }
    /// Layout.
    pub fn layout(&self) -> Option<&ScreenLayout> {
        self.layout.as_ref()
    }
    /// One slot; out-of-bounds indices return InvalidInput. Empty, Missing and Invalidated stay distinct.
    pub fn slot(&self, index: usize) -> crate::Result<&FieldState<ReceivedSlot>> {
        self.slots
            .get(index)
            .ok_or_else(|| super::registry::invalid("slot index outside snapshot"))
    }
    /// Latest validated supported full-packet marker. Single-slot updates never establish it.
    pub fn last_full_contents(&self) -> &FieldState<()> {
        &self.last_full_contents
    }
    /// Revision.
    pub fn revision(&self) -> &FieldState<i32> {
        &self.revision
    }
    /// Coverage of this opening's constructor-verified slots; unknown layouts never yield Complete.
    pub fn contents_coverage(&self) -> Coverage {
        if self.layout.is_some() {
            coverage(self.slots.iter().map(is_received))
        } else {
            Coverage::Partial
        }
    }
}

pub(crate) fn is_received<T>(state: &FieldState<T>) -> bool {
    matches!(state, FieldState::Received(_))
}
fn coverage(values: impl Iterator<Item = bool>) -> Coverage {
    let (mut supplied, mut total) = (0, 0);
    for value in values {
        total += 1;
        supplied += usize::from(value);
    }
    if supplied == 0 {
        Coverage::Unknown
    } else if supplied == total {
        Coverage::Complete
    } else {
        Coverage::Partial
    }
}
impl Receipt {
    pub(crate) fn new(session: SessionStamp, ordinal: u64) -> Self {
        Self {
            session,
            ordinal,
            applied_at: Instant::now(),
        }
    }
}
impl Invalidation {
    pub(crate) fn packet(reason: InvalidationReason, old: SessionStamp, event: Receipt) -> Self {
        Self {
            reason,
            invalidated_session: old,
            event_session: event.session,
            event_ordinal: Some(event.ordinal),
            event_applied_at: Some(event.applied_at),
        }
    }
}
impl<T> FieldState<T> {
    pub(crate) fn set(&mut self, value: T, receipt: Receipt) {
        *self = Self::Received(FieldReceipt { value, receipt });
    }
    pub(crate) fn invalidate(&mut self, invalidation: &Invalidation) {
        *self = Self::Invalidated(invalidation.clone());
    }
    // Called only at live ordered packet commit. A retained sequence cannot be
    // backfilled: only constituents supplied by this exact event can enter.
    pub(crate) fn apply(&mut self, value: Option<&super::ObservedValue<T>>, event: Receipt)
    where
        T: Clone,
    {
        if let Some(value) = value {
            if value.source
                == (super::ValueSource::Received {
                    sequence: event.ordinal,
                })
            {
                self.set(value.value.clone(), event);
            }
        }
    }
}
impl ReceivedCorrectionFields {
    pub(crate) fn from_packet(
        teleport_id: i32,
        position: [f64; 3],
        rotation: [f32; 2],
        delta_movement: Option<[f64; 3]>,
        native_flags: u32,
    ) -> Self {
        Self {
            teleport_id,
            position: std::array::from_fn(|i| {
                if native_flags & (1 << i) == 0 {
                    CorrectionComponent::Absolute(position[i])
                } else {
                    CorrectionComponent::Relative(position[i])
                }
            }),
            rotation: std::array::from_fn(|i| {
                if native_flags & (8 << i) == 0 {
                    CorrectionComponent::Absolute(rotation[i])
                } else {
                    CorrectionComponent::Relative(rotation[i])
                }
            }),
            delta_movement,
            native_flags,
        }
    }
}
impl ReceivedPlayerSnapshot {
    fn empty(session: SessionStamp, ordinal: u64, registries: ServerRegistryObservation) -> Self {
        Self {
            last_open: None,
            boundary: ReceiveBoundary { session, ordinal },
            captured_at: Instant::now(),
            health: FieldState::Missing,
            correction: FieldState::Missing,
            absolute_position: FieldState::Missing,
            absolute_rotation: FieldState::Missing,
            selected_hotbar: FieldState::Missing,
            game_mode: FieldState::Missing,
            abilities: FieldState::Missing,
            using_item: FieldState::Missing,
            air_supply: FieldState::Missing,
            fall_flying: FieldState::Missing,
            world_time: FieldState::Missing,
            entity_id: FieldState::Missing,
            dimension_selection: FieldState::Missing,
            dimension_geometry: FieldState::Missing,
            attributes: BTreeMap::new(),
            attribute_fallback: FieldState::Missing,
            effects: super::physics::blocks::table(session.version)
                .effect_ids
                .keys()
                .map(|key| (key.clone(), FieldState::Missing))
                .collect(),
            attributes_coverage: Coverage::Unknown,
            effects_coverage: Coverage::Unknown,
            registries: Arc::new(registries),
            inventory: ReceivedInventorySnapshot {
                slots: std::array::from_fn(|_| FieldState::Missing),
                cursor: FieldState::Missing,
                last_full_contents: FieldState::Missing,
                player_revision: FieldState::Missing,
            },
            container_state: FieldState::Missing,
            container: None,
        }
    }
    pub(crate) fn reset(&mut self, reason: InvalidationReason, event: Receipt) {
        let invalidation = Invalidation::packet(reason, self.boundary.session, event);
        self.invalidate_all(&invalidation);
    }
    fn invalidate_all(&mut self, invalidation: &Invalidation) {
        self.health.invalidate(invalidation);
        self.correction.invalidate(invalidation);
        self.absolute_position.invalidate(invalidation);
        self.absolute_rotation.invalidate(invalidation);
        self.selected_hotbar.invalidate(invalidation);
        self.game_mode.invalidate(invalidation);
        self.abilities.invalidate(invalidation);
        self.using_item.invalidate(invalidation);
        self.air_supply.invalidate(invalidation);
        self.fall_flying.invalidate(invalidation);
        self.world_time.invalidate(invalidation);
        self.entity_id.invalidate(invalidation);
        self.dimension_selection.invalidate(invalidation);
        self.attribute_fallback.invalidate(invalidation);
        self.container_state.invalidate(invalidation);
        if matches!(
            invalidation.reason,
            InvalidationReason::Reconfiguration
                | InvalidationReason::Revoke
                | InvalidationReason::Disconnect
                | InvalidationReason::ReaderFailure
        ) {
            self.dimension_geometry.invalidate(invalidation);
        }
        self.inventory.invalidate(invalidation);
        self.attributes.clear();
        for effect in self.effects.values_mut() {
            effect.invalidate(invalidation);
        }
        self.attributes_coverage = Coverage::Unknown;
        self.effects_coverage = Coverage::Unknown;
        self.container = None;
        self.last_open = None;
    }
    pub(crate) fn correction_received(&mut self, value: ReceivedCorrectionFields, event: Receipt) {
        let invalidation = Invalidation::packet(
            InvalidationReason::ProjectionUnavailable,
            event.session,
            event,
        );
        let mut position = [0.0; 3];
        let mut rotation = [0.0; 2];
        let p = value
            .position
            .iter()
            .zip(&mut position)
            .all(|(input, out)| {
                if let CorrectionComponent::Absolute(v) = input {
                    *out = *v;
                    true
                } else {
                    false
                }
            });
        let r = value
            .rotation
            .iter()
            .zip(&mut rotation)
            .all(|(input, out)| {
                if let CorrectionComponent::Absolute(v) = input {
                    *out = *v;
                    true
                } else {
                    false
                }
            });
        if p {
            self.absolute_position.set(position, event);
        } else {
            self.absolute_position.invalidate(&invalidation);
        }
        if r {
            self.absolute_rotation.set(rotation, event);
        } else {
            self.absolute_rotation.invalidate(&invalidation);
        }
        self.correction.set(value, event);
    }
    pub(crate) fn effect_received(
        &mut self,
        name: &str,
        value: Presence<PlayerEffect>,
        event: Receipt,
    ) {
        if let Some(field) = self.effects.get_mut(name) {
            field.set(value, event);
            self.effects_coverage = Coverage::Partial;
        }
    }
    pub(crate) fn attribute_received(
        &mut self,
        name: String,
        base: f64,
        modifiers: Vec<Modifier>,
        event: Receipt,
    ) {
        self.attributes.insert(
            name,
            FieldState::Received(FieldReceipt {
                value: Presence::Present(ReceivedAttribute { base, modifiers }),
                receipt: event,
            }),
        );
        self.attributes_coverage = Coverage::Partial;
    }
    pub(crate) fn entry(
        &mut self,
        entry: &super::WorldEntryContext,
        entity_id: Option<i32>,
        event: Receipt,
    ) -> crate::Result<()> {
        self.dimension_selection.set(
            ReceivedDimensionSelection {
                world_name: entry.world_name.clone(),
                dimension_type_name: entry.dimension_type_name.clone(),
                dimension_type_id: entry.dimension_type_id,
            },
            event,
        );
        self.game_mode
            .set(GameMode::decode(entry.game_mode & 7)?, event);
        if let Some(id) = entity_id {
            self.entity_id.set(id, event);
        }
        Ok(())
    }
    pub(crate) fn no_opening(&mut self, event: Receipt) {
        self.container_state.set(ContainerStatus::NoOpening, event);
        self.container = None;
        self.last_open = None;
    }
    pub(crate) fn close(&mut self, window: i32, event: Receipt) {
        let id = match &self.container_state {
            FieldState::Received(FieldReceipt {
                value: ContainerStatus::Open(id),
                ..
            }) => Some(*id),
            // A local close preserves the bounded original identity but no
            // eligibility. A later matching server close can still establish it.
            _ => self.last_open,
        };
        if let Some(id) = id.filter(|id| id.session() == event.session && id.window_id() == window)
        {
            self.close_opening(id, event);
        }
    }
    pub(crate) fn close_opening(&mut self, id: ScreenId, event: Receipt) {
        if self.last_open == Some(id) && id.session() == event.session {
            self.container_state.set(ContainerStatus::Closed(id), event);
            self.container = None;
            self.inventory.cursor.invalidate(&Invalidation::packet(
                InvalidationReason::ScreenChanged,
                event.session,
                event,
            ));
        }
    }
    pub(crate) fn screen(
        &mut self,
        screen: Option<super::container::ContainerScreen>,
        event: Receipt,
    ) -> crate::Result<()> {
        use super::received_items::ReceiptLocation;
        let Some(screen) = screen else {
            return Ok(());
        };
        if screen.id.opened_sequence() == event.ordinal {
            let invalidation =
                Invalidation::packet(InvalidationReason::ScreenChanged, event.session, event);
            self.inventory.cursor.invalidate(&invalidation);
            self.last_open = Some(screen.id);
            self.container_state
                .set(ContainerStatus::Open(screen.id), event);
            self.container = Some(ReceivedContainerSnapshot {
                id: screen.id,
                opened: event,
                title: FieldReceipt {
                    value: screen.title.value.clone(),
                    receipt: event,
                },
                layout: screen.layout.clone(),
                slots: vec![FieldState::Invalidated(invalidation); screen.slots.len()],
                last_full_contents: FieldState::Missing,
                revision: FieldState::Missing,
            });
        }
        if let Some(out) = self.container.as_mut().filter(|out| out.id == screen.id) {
            out.slots
                .resize_with(screen.slots.len(), || FieldState::Missing);
            for (index, value) in screen.slots.iter().enumerate() {
                if value.is_none() && is_received(&out.slots[index]) {
                    out.slots[index].invalidate(&Invalidation::packet(
                        InvalidationReason::UnsupportedPayload,
                        event.session,
                        event,
                    ));
                }
                apply_slot(
                    &mut out.slots[index],
                    value.as_ref(),
                    &self.registries,
                    ReceiptLocation::ContainerSlot {
                        screen: out.id,
                        slot: index,
                    },
                    event,
                )?;
            }
            if screen.full_contents_sequence.is_none() && is_received(&out.last_full_contents) {
                let invalidation = Invalidation::packet(
                    InvalidationReason::UnsupportedPayload,
                    event.session,
                    event,
                );
                for slot in &mut out.slots {
                    slot.invalidate(&invalidation);
                }
                out.last_full_contents.invalidate(&invalidation);
                out.revision.invalidate(&invalidation);
            }
            if screen.revision.is_none() && is_received(&out.revision) {
                out.revision.invalidate(&Invalidation::packet(
                    InvalidationReason::UnsupportedPayload,
                    event.session,
                    event,
                ));
            }
            out.revision.apply(screen.revision.as_ref(), event);
            if screen.full_contents_sequence == Some(event.ordinal) {
                out.last_full_contents.set((), event);
            }
        }
        Ok(())
    }
    pub(crate) fn inventory_received(
        &mut self,
        inventory: &super::InventoryObservation,
        event: Receipt,
    ) -> crate::Result<()> {
        use super::received_items::ReceiptLocation;
        for (index, out) in self.inventory.slots.iter_mut().enumerate() {
            apply_slot(
                out,
                inventory.slots.get(index).and_then(Option::as_ref),
                &self.registries,
                ReceiptLocation::PlayerSlot(index),
                event,
            )?;
        }
        apply_slot(
            &mut self.inventory.cursor,
            inventory.cursor.as_ref(),
            &self.registries,
            ReceiptLocation::Cursor,
            event,
        )?;
        self.inventory
            .player_revision
            .apply(inventory.player_screen_revision.as_ref(), event);
        Ok(())
    }
}
fn apply_slot(
    out: &mut FieldState<ReceivedSlot>,
    input: Option<&super::ObservedValue<super::SlotKnowledge>>,
    registries: &Arc<ServerRegistryObservation>,
    location: super::received_items::ReceiptLocation,
    event: Receipt,
) -> crate::Result<()> {
    if let Some(input) = input.filter(|v| {
        v.source
            == (super::ValueSource::Received {
                sequence: event.ordinal,
            })
    }) {
        if matches!(input.value, super::SlotKnowledge::Unavailable) {
            out.invalidate(&Invalidation::packet(
                InvalidationReason::UnsupportedPayload,
                event.session,
                event,
            ));
        } else {
            out.set(
                ReceivedSlot::capture(registries, input.clone(), location)?,
                event,
            );
        }
    }
    Ok(())
}
impl ReceivedInventorySnapshot {
    pub(crate) fn invalidate(&mut self, invalidation: &Invalidation) {
        for slot in &mut self.slots {
            slot.invalidate(invalidation);
        }
        self.cursor.invalidate(invalidation);
        self.last_full_contents.invalidate(invalidation);
        self.player_revision.invalidate(invalidation);
    }
}
#[derive(Default)]
pub(crate) struct Ledger {
    data: Option<ReceivedPlayerSnapshot>,
}
impl Ledger {
    // All mutation happens under the reader's existing ordered state boundary.
    pub(crate) fn commit(
        &mut self,
        event: Receipt,
        registries: ServerRegistryObservation,
        boundary_reason: InvalidationReason,
    ) -> &mut ReceivedPlayerSnapshot {
        let data = self.data.get_or_insert_with(|| {
            ReceivedPlayerSnapshot::empty(event.session, event.ordinal, registries.clone())
        });
        if data.boundary.session.world_generation != event.session.world_generation
            && (data.boundary.session.world_generation != 0
                || boundary_reason == InvalidationReason::Reconfiguration)
        {
            data.reset(boundary_reason, event);
        }
        data.boundary = ReceiveBoundary {
            session: event.session,
            ordinal: event.ordinal,
        };
        data.registries = Arc::new(registries);
        data
    }
    pub(crate) fn capture(
        &self,
        session: SessionStamp,
        ordinal: u64,
        registries: ServerRegistryObservation,
    ) -> ReceivedPlayerSnapshot {
        let mut data = self
            .data
            .clone()
            .unwrap_or_else(|| ReceivedPlayerSnapshot::empty(session, ordinal, registries.clone()));
        data.boundary = ReceiveBoundary { session, ordinal };
        data.registries = Arc::new(registries);
        data.captured_at = Instant::now();
        data
    }
}

impl Ledger {
    pub(crate) fn local_close(&mut self, session: SessionStamp) {
        if let Some(data) = &mut self.data {
            if data.last_open.is_some() {
                let invalidation = Invalidation {
                    reason: InvalidationReason::ScreenChanged,
                    invalidated_session: session,
                    event_session: session,
                    event_ordinal: None,
                    event_applied_at: None,
                };
                data.container_state.invalidate(&invalidation);
                data.container = None;
                data.inventory.cursor.invalidate(&invalidation);
            }
        }
    }
    pub(crate) fn terminate(
        &mut self,
        reason: InvalidationReason,
        session: SessionStamp,
        packet: Option<Receipt>,
    ) {
        if let Some(data) = &mut self.data {
            let invalidation = Invalidation {
                reason,
                invalidated_session: data.boundary.session,
                event_session: session,
                event_ordinal: packet.map(|p| p.ordinal),
                event_applied_at: packet.map(|p| p.applied_at),
            };
            data.invalidate_all(&invalidation);
        }
    }
}

impl ReceivedPlayerSnapshot {
    pub(crate) fn inventory_unavailable(
        &mut self,
        unavailable: impl Iterator<Item = bool>,
        cursor: bool,
        reason: InvalidationReason,
        event: Receipt,
    ) {
        let invalidation = Invalidation::packet(reason, event.session, event);
        for (out, unavailable) in self.inventory.slots.iter_mut().zip(unavailable) {
            if unavailable {
                out.invalidate(&invalidation);
            }
        }
        if cursor {
            self.inventory.cursor.invalidate(&invalidation);
        }
        if reason == InvalidationReason::UnsupportedPayload {
            self.inventory.last_full_contents.invalidate(&invalidation);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> SessionStamp {
        SessionStamp {
            version: crate::MinecraftVersion::Java1_16_1,
            connection_id: 57,
            world_generation: 1,
        }
    }
    #[test]
    fn received_snapshot_old_opening_close_cannot_certify_reused_window() {
        let session = session();
        let registries =
            super::super::registry::received::ReceivedRegistries::default().capture(session, 20);
        let mut snapshot = ReceivedPlayerSnapshot::empty(session, 20, registries);
        let screen = |sequence| {
            super::super::container::ScreenReceipts::open(
                session.version,
                5,
                Some(0),
                ScreenTitle::Unavailable,
                sequence,
            )
            .capture(session)
        };
        let first = screen(10);
        let first_id = first.id;
        snapshot
            .screen(Some(first), Receipt::new(session, 10))
            .unwrap();
        let second = screen(12);
        let second_id = second.id;
        snapshot
            .screen(Some(second), Receipt::new(session, 12))
            .unwrap();
        snapshot.close_opening(first_id, Receipt::new(session, 13));
        assert!(
            matches!(&snapshot.container_state, FieldState::Received(value) if matches!(value.value(), ContainerStatus::Open(id) if *id == second_id))
        );
        snapshot.close_opening(second_id, Receipt::new(session, 14));
        assert!(
            matches!(&snapshot.container_state, FieldState::Received(value) if matches!(value.value(), ContainerStatus::Closed(id) if *id == second_id))
        );
        assert!(snapshot.container().is_none());
    }
    #[test]
    fn received_snapshot_local_close_has_no_packet_metadata_and_copies_are_owned() {
        fn owned<T: Clone + std::fmt::Debug + Send + Sync + 'static>() {}
        owned::<ReceivedPlayerSnapshot>();
        owned::<ReceivedInventorySnapshot>();
        owned::<ReceivedContainerSnapshot>();
        let session = session();
        let registries =
            super::super::registry::received::ReceivedRegistries::default().capture(session, 1);
        let mut ledger = Ledger::default();
        let out = ledger.commit(
            Receipt::new(session, 1),
            registries.clone(),
            InvalidationReason::SessionBoundary,
        );
        let screen = super::super::container::ScreenReceipts::open(
            session.version,
            7,
            Some(0),
            ScreenTitle::Unavailable,
            1,
        )
        .capture(session);
        out.screen(Some(screen), Receipt::new(session, 1)).unwrap();
        let old = ledger.capture(session, 1, registries.clone());
        ledger.local_close(session);
        let local = ledger.capture(session, 1, registries);
        let FieldState::Invalidated(event) = local.container_state() else {
            panic!("local close")
        };
        assert_eq!(event.reason(), InvalidationReason::ScreenChanged);
        assert_eq!(event.event_ordinal(), None);
        assert_eq!(event.event_applied_at(), None);
        assert!(local.container().is_none());
        assert!(old.container().is_some());
        let out = ledger.data.as_mut().unwrap();
        out.close(7, Receipt::new(session, 2));
        assert!(
            matches!(out.container_state(), FieldState::Received(value) if matches!(value.value(), ContainerStatus::Closed(_)))
        );
    }
}
