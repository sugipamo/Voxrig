//! One-shot checked placement with independent world/material receipts.
mod slab;
mod stair;
#[derive(Clone, Copy, PartialEq)]
enum Family {
    Cube,
    Slab,
    Stair,
}
use super::geometry::GeometryView;
use super::*;
use crate::diagnostic_projection::diagnostic_record;
use std::time::Duration;

diagnostic_record! {
    /// A single main-hand placement, retained before any packet write.
    #[derive(Clone, Debug, PartialEq, Serialize)]
    pub struct PlacementIntent => RecordedPlacementIntent {
        /// Owning live connection; serialized history cannot create an operation.
        pub connection_id: u64,
        /// Current loading/world generation.
        pub generation: u64,
        /// Receive boundary before submission.
        pub after_sequence: u64,
        /// Connection-global one-shot block-use sequence.
        pub sequence: i32,
        /// Observed dimension.
        pub dimension: String,
        /// Stationary feet position.
        pub position: [f64; 3],
        /// Original clicked block; air-only methods require a passive cube.
        pub support: [i32; 3],
        /// Exact received predecessor at the clicked support.
        pub support_state: crate::NativeBlockState,
        /// Native face ID, 0..5.
        pub face_id: u8,
        /// Derived native face-hit cursor, local to support.
        pub cursor: [f32; 3],
        /// Destination; air-only methods require the adjacent air cell.
        pub target: [i32; 3],
        /// Received target predecessor; never inferred air.
        pub before: crate::NativeBlockState,
        /// Expected complete block state; air-only cube methods have no properties.
        pub expected: crate::NativeBlockState,
        /// Ordered main-hand selection used by this action.
        pub selection: HotbarSelection,
        /// Received component-free selected stack before placement.
        pub held_before: PlainItem,
        /// Per-slot receipt boundary for that predecessor.
        pub held_receive_sequence: u64,
    }
    diagnostic_serde {}
}
diagnostic_record! {
    /// Both received outcomes, not attribution to a particular actor.
    #[derive(Clone, Debug, Serialize)]
    pub struct PlacementObservation => RecordedPlacementObservation {
        /// Original operation and predecessors.
        pub intent: PlacementIntent,
        /// Fresh target-specific block/section update.
        pub target_receive_sequence: u64,
        /// Fresh selected-slot update showing exactly one material consumed.
        pub inventory_receive_sequence: u64,
        /// Exact remaining selected stack (possibly empty).
        pub held_after: InventorySlot,
        /// Native one-shot interaction sequence processed; never success by itself.
        pub acknowledged_sequence: i32,
    }
    diagnostic_serde {}
}
diagnostic_record! {
    /// Retained stage; timeout/cancellation never permits a blind second use.
    #[derive(Clone, Debug, Serialize)]
    pub struct PlacementRecord => RecordedPlacementRecord {
        /// Stored before I/O.
        pub intent: PlacementIntent,
        /// Complete frame submitted, not server acceptance.
        pub dispatched: bool,
        /// Latest exact target receipt.
        pub target_receive_sequence: Option<u64>,
        /// Latest receipt showing exactly one selected material consumed.
        pub inventory_receive_sequence: Option<u64>,
        /// Latched unexpected state/context.
        pub requires_inspection: Option<String>,
        /// Historical completed result. Each next operation revalidates its own site.
        pub observation: Option<PlacementObservation>,
    }
    diagnostic_serde {}
}
diagnostic_record! {
    /// Acknowledgement, target change and material consumption are distinct stages.
    #[derive(Clone, Debug, Serialize)]
    #[serde(tag = "status", rename_all = "snake_case")]
    pub enum PlacementStatus => RecordedPlacementStatus {
        /// One or more required receipts have not arrived. Do not resend.
        Pending {
            /// Retained unresolved attempt.
            record: PlacementRecord,
        },
        /// Both expected outcomes and processed sequence have been received.
        ObservedPlaced {
            /// Received placement and material outcome.
            observation: PlacementObservation,
        },
        /// A conflict requires fresh inspection, not automatic retry.
        RequiresInspection {
            /// Attempt and latched conflict.
            record: PlacementRecord,
        },
    }
    diagnostic_serde { #[serde(tag = "status", rename_all = "snake_case")] }
}
fn unavailable(message: &str) -> Error {
    crate::error::state_error(message)
}
fn air(block: &crate::NativeBlockState) -> bool {
    matches!(
        block.name.as_str(),
        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
    )
}
fn remaining(intent: &PlacementIntent) -> InventorySlot {
    if intent.held_before.count == 1 {
        InventorySlot::Empty
    } else {
        let mut item = intent.held_before.clone();
        item.count -= 1;
        InventorySlot::Item { item }
    }
}
#[cfg(test)]
fn offset(face: u8) -> [i32; 3] {
    [
        [0, -1, 0],
        [0, 1, 0],
        [0, 0, -1],
        [0, 0, 1],
        [-1, 0, 0],
        [1, 0, 0],
    ][face as usize]
}
// Qualified through original canBeReplaced/getStateForPlacement fixtures.
// Complete property-free states only; no generic name/tag replaceability fallback.
const REPLACEABLE_PLANTS: &[(&str, [f64; 6])] = &[
    (
        "minecraft:short_grass",
        [0.125, 0., 0.125, 0.875, 0.8125, 0.875],
    ),
    ("minecraft:fern", [0.125, 0., 0.125, 0.875, 0.8125, 0.875]),
    (
        "minecraft:dead_bush",
        [0.125, 0., 0.125, 0.875, 0.8125, 0.875],
    ),
];
pub(in crate::versions::java_1_21_11::client) fn plant_outline(
    block: &crate::NativeBlockState,
) -> Option<&'static [[f64; 6]]> {
    block.properties.is_empty().then_some(())?;
    REPLACEABLE_PLANTS
        .iter()
        .find(|(name, _)| *name == block.name)
        .map(|(_, bounds)| std::slice::from_ref(bounds))
}
pub(super) fn replaceable_plant(block: &crate::NativeBlockState) -> bool {
    plant_outline(block).is_some()
}
#[derive(Clone)]
struct ReplacementCapture {
    first_outline: crate::client::survival::BlockTargetHit,
    replaces_clicked: bool,
}
#[cfg(test)]
fn prepare(
    state: &mut State,
    connection_id: u64,
    tick: u64,
    support: [i32; 3],
    face: u8,
    common_player_screen: bool,
) -> Result<PlacementIntent> {
    prepare_in(
        state,
        connection_id,
        tick,
        support,
        face,
        common_player_screen,
        None,
        Family::Cube,
    )
    .map(|(intent, _)| intent)
}
#[allow(clippy::too_many_arguments)]
fn prepare_in(
    state: &mut State,
    connection_id: u64,
    tick: u64,
    support: [i32; 3],
    face: u8,
    common_player_screen: bool,
    replacement: Option<([i32; 3], &crate::NativeBlockState)>,
    family: Family,
) -> Result<(PlacementIntent, Option<ReplacementCapture>)> {
    if state.operations.game_mode != Some(GameMode::Survival) {
        return Err(unavailable("placement requires received survival mode"));
    }
    let standing = if replacement.is_some() && family == Family::Cube {
        survival::replacement_context(state, connection_id, tick)?
    } else {
        survival::context(state, connection_id, tick)?
    };
    if !standing.on_ground
        || standing.submerged
        || !standing
            .player
            .health
            .as_ref()
            .is_some_and(|h| h.health > 0.0)
    {
        return Err(unavailable(
            "placement requires healthy dry standing contact",
        ));
    }
    let selection = state
        .operations
        .selected_hotbar
        .clone()
        .filter(|s| s.dispatched)
        .ok_or_else(|| unavailable("selected hand unavailable or incomplete"))?;
    let inventory = &state.operations.inventory;
    let slot = 36 + usize::from(selection.slot);
    if (!common_player_screen && inventory.window_id != Some(0))
        || inventory.cursor != InventorySlot::Empty
        || inventory.unsupported_components
        || inventory.pending_swap.is_some()
        || !inventory.pending_creative.is_empty()
    {
        return Err(unavailable(
            "placement requires supported player inventory with no pending transfer",
        ));
    }
    let InventorySlot::Item { item } = &inventory.slots[slot] else {
        return Err(unavailable(
            "selected building material unavailable or empty",
        ));
    };
    // This is a separate placement admission: stateful grass is valid ground,
    // but is not an exact state-free placement material.
    if if family == Family::Stair {
        item.name != "minecraft:oak_stairs"
    } else if family == Family::Slab {
        item.name != "minecraft:oak_slab"
    } else {
        !survival::DRY_CUBES.contains(&item.name.as_str()) || item.name == "minecraft:grass_block"
    } {
        return Err(Error::new(
            ErrorKind::Unsupported,
            anyhow::anyhow!(if family == Family::Stair {
                "stair placement requires oak stair material"
            } else if family == Family::Slab {
                "slab placement requires oak slab material"
            } else {
                "placement requires an admitted passive cube material"
            }),
        ));
    }
    let count = u8::try_from(item.count).map_err(|_| invalid("invalid material count"))?;
    if default_item(&item.name, count)? != *item {
        return Err(invalid(
            "material identity/count differs from native registry",
        ));
    }
    let held_before = item.clone();
    let held_receive_sequence = inventory.slot_sequences[slot]
        .ok_or_else(|| unavailable("held material receipt unavailable"))?;
    let geometry = if family == Family::Stair {
        stair::geometry(
            state,
            &standing,
            support,
            face,
            replacement.expect("stair predecessor"),
            state.rotation,
        )?
    } else if family == Family::Slab {
        slab::geometry(
            state,
            &standing,
            support,
            face,
            replacement.expect("slab predecessor"),
            state.rotation,
        )?
    } else {
        placement_geometry_in(
            state,
            standing.position,
            standing.bounds,
            standing.position_basis.geometry_reserve(),
            state.rotation,
            support,
            face,
            replacement,
        )?
    };
    let PlacementGeometry {
        target,
        before,
        hit,
        cursor,
    } = geometry;
    let expected = if family == Family::Stair {
        let expected = stair::expected(state, target, state.rotation[0], face, cursor[1])?;
        stair::unobstructed(state, connection_id, standing.bounds, target, &expected)?;
        expected
    } else if family == Family::Slab {
        let expected = slab::expected(&before, face, cursor[1]);
        slab::unobstructed(state, connection_id, standing.bounds, target, &expected)?;
        expected
    } else {
        crate::NativeBlockState {
            name: item.name.clone(),
            properties: Default::default(),
        }
    };
    super::super::super::state_id(&expected)?;
    let eye = [
        standing.position[0],
        standing.position[1] + f64::from(1.62f32),
        standing.position[2],
    ];
    let vector = crate::client::survival::target::direction(
        crate::MinecraftVersion::Java1_21_11,
        state.rotation,
    );
    let length = vector.iter().map(|v| v * v).sum::<f64>().sqrt();
    let replacement_capture = replacement.map(|_| ReplacementCapture {
        first_outline: crate::client::survival::BlockTargetHit {
            position: hit.position,
            state: hit.state.clone(),
            face: [
                crate::BlockFace::Down,
                crate::BlockFace::Up,
                crate::BlockFace::North,
                crate::BlockFace::South,
                crate::BlockFace::West,
                crate::BlockFace::East,
            ][face as usize],
            point: std::array::from_fn(|i| eye[i] + vector[i] * (hit.distance / length)),
            distance: hit.distance,
        },
        replaces_clicked: target == support,
    });
    Ok((
        PlacementIntent {
            connection_id,
            generation: state.loading.generation,
            after_sequence: state.sequence,
            sequence: 0,
            dimension: standing.dimension,
            position: standing.position,
            support,
            support_state: hit.state,
            face_id: face,
            cursor,
            target,
            before,
            expected,
            selection,
            held_before,
            held_receive_sequence,
        },
        replacement_capture,
    ))
}
pub(super) struct PlacementGeometry {
    pub target: [i32; 3],
    pub before: crate::NativeBlockState,
    pub hit: super::super::raycast::BlockHit,
    pub cursor: [f32; 3],
}
// Shared native targeting and uncertainty checks; no inventory or authority.
pub(super) fn placement_geometry(
    view: &impl GeometryView,
    position: [f64; 3],
    bounds: [f64; 6],
    error: [f64; 3],
    rotation: [f32; 2],
    support: [i32; 3],
    face: u8,
) -> Result<PlacementGeometry> {
    placement_geometry_in(view, position, bounds, error, rotation, support, face, None)
}
#[allow(clippy::too_many_arguments)]
fn placement_geometry_in(
    view: &impl GeometryView,
    position: [f64; 3],
    bounds: [f64; 6],
    error: [f64; 3],
    rotation: [f32; 2],
    support: [i32; 3],
    face: u8,
    replacement: Option<([i32; 3], &crate::NativeBlockState)>,
) -> Result<PlacementGeometry> {
    validate_pose(position, rotation)?;
    if face > 5 {
        return Err(invalid("invalid placement face"));
    }
    let eye = [position[0], position[1] + f64::from(1.62f32), position[2]];
    let hit = if replacement.is_some() {
        let direction = crate::client::survival::target::direction(
            crate::MinecraftVersion::Java1_21_11,
            rotation,
        );
        let end = std::array::from_fn(|i| eye[i] + direction[i] * 4.5);
        super::super::raycast::replacement_cast_in(eye, end, |p| {
            view.block(p).map_err(anyhow::Error::from)
        })?
        .map(|hit| super::super::raycast::BlockHit {
            position: hit.position,
            state: hit.state,
            distance: hit.distance,
            face: Some(
                super::super::super::reconstruction::Direction::from_id(hit.face as u8)
                    .expect("native face"),
            ),
        })
    } else {
        super::super::raycast::outline_hit_in(eye, rotation, 4.5, |p| {
            view.block(p).map_err(anyhow::Error::from)
        })?
    }
    .ok_or_else(|| unavailable("no first native outline hit for placement"))?;
    survival::uncertain_target_in(view, eye, error, rotation, &hit)?;
    if hit.position != support
        || hit.face.map(|f| f as u8) != Some(face)
        || !(survival::DRY_CUBES.contains(&hit.state.name.as_str())
            || (replacement.is_some() && replaceable_plant(&hit.state)))
    {
        return Err(unavailable(
            "placement support must be the first hit on an admitted passive cube face",
        ));
    }
    let target = if replacement.is_some() && replaceable_plant(&hit.state) {
        support
    } else {
        crate::client::survival::placement::adjacent(
            support,
            [
                crate::BlockFace::Down,
                crate::BlockFace::Up,
                crate::BlockFace::North,
                crate::BlockFace::South,
                crate::BlockFace::West,
                crate::BlockFace::East,
            ][face as usize],
        )?
    };
    let before = view.block(target)?;
    if let Some((requested, predecessor)) = replacement {
        if requested != target || predecessor != &before || !replaceable_plant(&before) {
            return Err(unavailable(
                "replacement requires the exact audited plant destination state",
            ));
        }
    }
    if (replacement.is_none() && !air(&before))
        || (0..3)
            .all(|i| bounds[i] < f64::from(target[i] + 1) && bounds[i + 3] > f64::from(target[i]))
    {
        return Err(unavailable(
            "placement requires known empty air outside the standing body",
        ));
    }
    let cursor = super::super::raycast::hit_cursor_in(rotation, eye, &hit);
    Ok(PlacementGeometry {
        target,
        before,
        hit,
        cursor,
    })
}
fn packet(intent: &PlacementIntent) -> Vec<u8> {
    let mut p = vec![0]; // Main hand.
    p.extend(pack_position(intent.support).to_be_bytes());
    put_varint(&mut p, i32::from(intent.face_id));
    for v in intent.cursor {
        p.extend(v.to_be_bytes());
    }
    p.extend([0, 0]); // Outside shape, not against world border.
    put_varint(&mut p, intent.sequence);
    p
}
fn owning(state: &State, intent: &PlacementIntent, id: u64) -> Result<()> {
    if intent.connection_id != id || state.placement.as_ref().is_none_or(|p| p.intent != *intent) {
        return Err(unavailable(
            "placement intent belongs to another connection or attempt",
        ));
    }
    Ok(())
}
impl Operations {
    /// Submit one ordinary stationary placement into adjacent observed air.
    /// Caller owns site/edit permissions; this API grants no Blueprint authority.
    /// No automatic retry, local prediction, item creation or mode change.
    pub async fn place_survival_cube(
        &self,
        support: [i32; 3],
        face: crate::BlockFace,
    ) -> Result<PlacementIntent> {
        self.place_cube_in(support, face, false, None, Family::Cube)
            .await
    }
    async fn place_cube_in(
        &self,
        support: [i32; 3],
        face: crate::BlockFace,
        capture_common: bool,
        replacement: Option<([i32; 3], crate::NativeBlockState)>,
        family: Family,
    ) -> Result<PlacementIntent> {
        self.bot
            .session
            .ledger
            .try_work()?
            .run_scoped(async {
                let mut state = self.bot.session.state.lock().await;
                self.mutable(&state)?;
                let initial = capture_common
                    .then(|| self.common_player_unlocked(&state))
                    .transpose()?;
                if let Some(p) = &initial {
                    if family == Family::Stair {
                        crate::client::survival::placement::selected_stair(p)?;
                    } else if family == Family::Slab {
                        crate::client::survival::placement::selected_slab(p)?;
                    } else {
                        crate::client::survival::placement::selected_material(p)?;
                    }
                }
                let (mut intent, replacement_capture) = prepare_in(
                    &mut state,
                    self.bot.session.id,
                    self.bot.session.started.elapsed().as_millis() as u64 / 50,
                    support,
                    face as u8,
                    initial
                        .as_ref()
                        .is_some_and(|p| p.inventory.player_screen.is_some()),
                    replacement.as_ref().map(|(p, s)| (*p, s)),
                    family,
                )?;
                let stairs = if family == Family::Stair {
                    Some(stair::capture(&*state, &intent)?)
                } else {
                    None
                };
                intent.sequence = self.next_sequence()?;
                if let Some(previous) = state.common_placement.as_ref() {
                    if previous.stairs.is_some() {
                        state.retired_stair_placement = stair::record_in(&state)?;
                    } else if previous.slab {
                        state.retired_slab_placement = slab::record_in(&state)?;
                    } else if previous.replacement.is_some() {
                        state.retired_replaceable_placement = replacement_record_in(&state)?;
                    } else if !capture_common
                        || replacement_capture.is_some()
                        || family != Family::Cube
                    {
                        state.retired_common_placement = common_record_in(&state)?;
                    }
                }
                state.common_placement = initial.map(|initial| CommonPlacementCapture {
                    initial,
                    sequence: intent.sequence,
                    replacement: replacement_capture,
                    processing: None,
                    slab: family == Family::Slab,
                    stairs,
                });
                let work = state.finite_work.begin(
                    &self.bot.session.ledger,
                    crate::client::ledger::finite::Family::Placement,
                    intent.sequence as u64,
                );
                state.placement = Some(PlacementRecord {
                    intent: intent.clone(),
                    dispatched: false,
                    target_receive_sequence: None,
                    inventory_receive_sequence: None,
                    requires_inspection: None,
                    observation: None,
                });
                work.run(
                    self.bot
                        .session
                        .send(ids::play_serverbound::BLOCK_PLACE, &packet(&intent)),
                )
                .await?;
                state.placement.as_mut().expect("intent stored").dispatched = true;
                Ok(intent)
            })
            .await
    }
    /// Read received progress. Successful observation releases the one-shot gate;
    /// pending/conflicting observations never do. Closure preserves history.
    pub async fn observe_survival_placement(
        &self,
        intent: &PlacementIntent,
    ) -> Result<PlacementStatus> {
        let mut state = self.bot.session.state.lock().await;
        self.ready(&state)?;
        owning(&state, intent, self.bot.session.id)?;
        observe_in(
            &mut state,
            self.bot.session.id,
            self.bot.session.started.elapsed().as_millis() as u64 / 50,
            intent,
        )
    }
    /// Bounded read-only wait; timeout/cancellation retains the original intent.
    pub async fn wait_survival_placement(
        &self,
        intent: &PlacementIntent,
        maximum_wait: Duration,
    ) -> Result<PlacementStatus> {
        if maximum_wait.is_zero() || maximum_wait > Duration::from_secs(30) {
            return Err(invalid(
                "placement wait must be greater than zero and at most 30 seconds",
            ));
        }
        match timeout(maximum_wait, async {
            loop {
                let notified = self.bot.session.changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                let result = self.observe_survival_placement(intent).await?;
                if !matches!(result, PlacementStatus::Pending { .. }) {
                    return Ok(result);
                }
                notified.await;
            }
        })
        .await
        {
            Ok(result) => result,
            Err(_) => {
                self.bot.session.ledger.fail(Failure::TransactionExpired);
                self.observe_survival_placement(intent).await
            }
        }
    }
}
fn observe_in(
    state: &mut State,
    connection_id: u64,
    tick: u64,
    intent: &PlacementIntent,
) -> Result<PlacementStatus> {
    if let Some(observation) = &state.placement.as_ref().expect("owner").observation {
        return Ok(PlacementStatus::ObservedPlaced {
            observation: observation.clone(),
        });
    }
    placement_context_received(state);
    let context = if state
        .common_placement
        .as_ref()
        .is_some_and(|c| c.replacement.is_some() && !c.slab && c.stairs.is_none())
    {
        survival::replacement_context(state, connection_id, tick)
    } else {
        survival::context(state, connection_id, tick)
    };
    if let Err(e) = context {
        state
            .placement
            .as_mut()
            .expect("owner")
            .requires_inspection
            .get_or_insert_with(|| e.to_string());
    }
    let cell = state.reconstruction.cell(&state.world, intent.target);
    let require_fresh_ack = state.common_placement.is_some();
    let neighbors_received = stair::confirmed(state);
    let State {
        placement,
        operations,
        ..
    } = state;
    let record = placement.as_mut().expect("owner");
    if cell.moving.is_some() || cell.state.is_none() {
        record
            .requires_inspection
            .get_or_insert_with(|| "placement target unavailable or moving".into());
    }
    if record.requires_inspection.is_some() {
        return Ok(PlacementStatus::RequiresInspection {
            record: record.clone(),
        });
    }
    let inventory = &operations.inventory;
    let slot = 36 + usize::from(intent.selection.slot);
    if record.dispatched
        && neighbors_received
        && record
            .target_receive_sequence
            .is_some_and(|s| s > intent.after_sequence)
        && cell.state.as_ref() == Some(&intent.expected)
        && inventory.slot_sequences[slot].is_some_and(|s| s > intent.after_sequence)
        && inventory.slots[slot] == remaining(intent)
        && operations.ack.is_some_and(|s| s >= intent.sequence)
        && (!require_fresh_ack
            || operations
                .ack_receive_sequence
                .is_some_and(|s| s > intent.after_sequence))
    {
        let observation = PlacementObservation {
            intent: intent.clone(),
            target_receive_sequence: record.target_receive_sequence.unwrap(),
            inventory_receive_sequence: inventory.slot_sequences[slot].unwrap(),
            held_after: inventory.slots[slot].clone(),
            acknowledged_sequence: operations.ack.unwrap(),
        };
        record.observation = Some(observation.clone());
        if let Some(c) = &mut state.common_placement {
            c.processing = operations.ack.zip(operations.ack_receive_sequence).map(
                |(sequence, receive_sequence)| crate::client::survival::PlacementProcessing {
                    sequence,
                    receive_sequence,
                },
            );
        }
        return Ok(PlacementStatus::ObservedPlaced { observation });
    }
    Ok(PlacementStatus::Pending {
        record: record.clone(),
    })
}
/// Called after every successfully applied receive packet, so a conflicting
/// intermediate inventory/world state cannot be erased by a later matching one.
pub(in crate::versions::java_1_21_11::client) fn placement_context_received(state: &mut State) {
    stair::context_received(state);
    let screen_matches = state.common_placement.as_ref().map_or(
        state.operations.inventory.window_id == Some(0),
        |capture| {
            common_player_screen_access(state, capture.initial.session)
                .is_some_and(|basis| Some(basis) == capture.initial.inventory.player_screen)
        },
    );
    let Some(r) = &mut state.placement else {
        return;
    };
    if r.observation.is_some() {
        return;
    }
    let i = &r.intent;
    let inventory = &state.operations.inventory;
    let slot = 36 + usize::from(i.selection.slot);
    let before = InventorySlot::Item {
        item: i.held_before.clone(),
    };
    let after = remaining(i);
    if inventory.slot_sequences[slot].is_some_and(|seq| seq > i.after_sequence) {
        if inventory.slots[slot] == after {
            r.inventory_receive_sequence = inventory.slot_sequences[slot];
        } else if r.inventory_receive_sequence.is_some() {
            r.requires_inspection.get_or_insert_with(|| {
                "consumed material changed again before placement confirmation".into()
            });
        }
    }
    if state.loading.generation != i.generation
        || !state.ready
        || state.position != Some(i.position)
        || state.world.dimension.as_ref().map(|d| &d.0) != Some(&i.dimension)
        || state.operations.game_mode != Some(GameMode::Survival)
        || state.operations.selected_hotbar.as_ref().map(|s| s.slot) != Some(i.selection.slot)
        || !screen_matches
        || inventory.cursor != InventorySlot::Empty
        || inventory.unsupported_components
        || inventory.pending_swap.is_some()
        || (inventory.slots[slot] != before && inventory.slots[slot] != remaining(i))
    {
        r.requires_inspection
            .get_or_insert_with(|| "placement player/world/inventory context changed".into());
    }
}
pub(in crate::versions::java_1_21_11::client) fn placement_chunk_changed(
    state: &mut State,
    chunk: [i32; 2],
) {
    stair::chunk_changed(state, chunk);
    if let Some(r) = state.placement.as_mut().filter(|r| {
        r.observation.is_none()
            && [r.intent.target, r.intent.support]
                .iter()
                .any(|p| [p[0].div_euclid(16), p[2].div_euclid(16)] == chunk)
    }) {
        r.requires_inspection
            .get_or_insert_with(|| "placement chunk unloaded or replaced".into());
    }
}
pub(in crate::versions::java_1_21_11::client) fn placement_received(
    state: &mut State,
    changes: &[([i32; 3], i32)],
) -> anyhow::Result<()> {
    stair::received(state, changes)?;
    let Some(r) = &mut state.placement else {
        return Ok(());
    };
    if r.observation.is_some() {
        return Ok(());
    }
    for (p, id) in changes {
        if *p != r.intent.target && *p != r.intent.support {
            continue;
        }
        let block = super::super::super::native_state(*id)?;
        if state.world.block(*p) != Some(*id)
            || (*p == r.intent.support && *p != r.intent.target && block != r.intent.support_state)
            || (*p == r.intent.target && block != r.intent.before && block != r.intent.expected)
        {
            r.requires_inspection.get_or_insert_with(|| {
                "placement target/support conflict or unavailable baseline".into()
            });
        }
        if *p == r.intent.target {
            // Once expected placement has arrived, its disappearance is a conflict.
            if r.target_receive_sequence.is_some() && block != r.intent.expected {
                r.requires_inspection.get_or_insert_with(|| {
                    "placed target changed before material confirmation".into()
                });
            }
            if block == r.intent.expected {
                r.target_receive_sequence = Some(state.sequence);
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests;

/// Original common capture retained under the same native intent lock.
#[derive(Clone)]
pub(in crate::versions::java_1_21_11::client) struct CommonPlacementCapture {
    initial: crate::client::PlayerObservation,
    sequence: i32,
    replacement: Option<ReplacementCapture>,
    slab: bool,
    stairs: Option<stair::Capture>,
    processing: Option<crate::client::survival::PlacementProcessing>,
}
fn common_player_screen_access(
    state: &State,
    mut session: crate::client::SessionStamp,
) -> Option<crate::client::container::PlayerScreenAccess> {
    session.world_generation = state.loading.generation;
    let inventory = &state.operations.inventory;
    crate::client::container::player_screen_access(
        session,
        inventory.window_id,
        inventory.container.as_ref().map(|s| s.capture(session).id),
        state.common_container_close.as_ref(),
    )
}
fn common_record_in(state: &State) -> Result<Option<crate::client::survival::PlacementRecord>> {
    use crate::client::{self as common, survival as api};
    let Some(capture) = state.common_placement.as_ref() else {
        return Ok(state.retired_common_placement.clone());
    };
    let native = state
        .placement
        .as_ref()
        .filter(|p| p.intent.sequence == capture.sequence)
        .ok_or_else(|| unavailable("common placement no longer owns native intent"))?;
    let i = &native.intent;
    let (held_before, held_receive_sequence) = if capture.stairs.is_some() {
        api::placement::selected_stair(&capture.initial)?
    } else if capture.slab {
        api::placement::selected_slab(&capture.initial)?
    } else {
        api::placement::selected_material(&capture.initial)?
    };
    let processing = capture.processing.clone().or_else(|| {
        state
            .operations
            .ack
            .zip(state.operations.ack_receive_sequence)
            .filter(|(sequence, receive)| *sequence >= i.sequence && *receive > i.after_sequence)
            .map(|(sequence, receive_sequence)| api::PlacementProcessing {
                sequence,
                receive_sequence,
            })
    });
    // Completed receipts are frozen in observation, independent of current inventory.
    let material_receipt = if let Some(o) = &native.observation {
        Some(common::received(
            super::common_slot(&o.held_after)?,
            o.inventory_receive_sequence,
        ))
    } else {
        native
            .inventory_receive_sequence
            .map(|s| common::received(api::placement::remaining(&held_before), s))
    };
    Ok(Some(api::PlacementRecord {
        id: api::PlacementId::new(capture.initial.session, i.sequence as u64 + 1),
        initial: capture.initial.clone(),
        support: i.support,
        support_state: i.support_state.clone(),
        face: [
            crate::BlockFace::Down,
            crate::BlockFace::Up,
            crate::BlockFace::North,
            crate::BlockFace::South,
            crate::BlockFace::West,
            crate::BlockFace::East,
        ][i.face_id as usize],
        cursor: i.cursor,
        target: i.target,
        before: i.before.clone(),
        expected: i.expected.clone(),
        held_before,
        held_receive_sequence,
        send: api::PlacementSend {
            after_sequence: i.after_sequence,
            interaction_sequence: Some(i.sequence),
            dispatched: native.dispatched,
        },
        target_receipt: native
            .target_receive_sequence
            .map(|s| common::received(i.expected.clone(), s)),
        material_receipt,
        processing,
        requires_inspection: native.requires_inspection.clone(),
        stage: if native.observation.is_some() {
            api::PlacementStage::ObservedPlaced
        } else if native.requires_inspection.is_some() {
            api::PlacementStage::RequiresInspection
        } else {
            api::PlacementStage::Pending
        },
    }))
}
pub(in crate::versions::java_1_21_11::client) fn common_placement_context_received(
    state: &mut State,
) {
    let Some(capture) = state.common_placement.clone() else {
        return;
    };
    if state
        .placement
        .as_ref()
        .is_none_or(|p| p.observation.is_some())
    {
        return;
    }
    let check = (|| -> Result<()> {
        let p = &capture.initial;
        if state.loading.generation != p.session.world_generation
            || common_player_screen_access(state, p.session) != p.inventory.player_screen
            || state.position != p.position.as_ref().map(|p| p.value)
            || state.rotation != p.rotation
            || state.operations.selected_hotbar.as_ref()
                != Some(&state.placement.as_ref().expect("retained").intent.selection)
            || state
                .motion
                .received_pose
                .as_ref()
                .map(|r| r.receive_sequence)
                != p.received_pose.as_ref().map(|r| r.receive_sequence)
            || state.operations.inventory.cursor_sequence.is_none()
        {
            return Err(unavailable(
                "common placement pose/selection/cursor context changed",
            ));
        }
        let standing = if capture.replacement.is_some() && !capture.slab && capture.stairs.is_none()
        {
            survival::replacement_context(
                state,
                p.session.connection_id,
                state.reconstruction.tick,
            )?
        } else {
            survival::context(state, p.session.connection_id, state.reconstruction.tick)?
        };
        if capture.stairs.is_some() {
            let intent = &state.placement.as_ref().expect("retained").intent;
            stair::unobstructed(
                state,
                p.session.connection_id,
                standing.bounds,
                intent.target,
                &intent.expected,
            )?;
        }
        if capture.slab {
            let intent = &state.placement.as_ref().expect("retained").intent;
            slab::unobstructed(
                state,
                p.session.connection_id,
                standing.bounds,
                intent.target,
                &intent.expected,
            )?;
        }
        if !standing.on_ground
            || !standing
                .player
                .health
                .as_ref()
                .is_some_and(|h| h.health > 0.0)
        {
            return Err(unavailable(
                "common placement healthy dry standing prerequisites changed",
            ));
        }
        Ok(())
    })();
    if let Err(error) = check {
        state
            .placement
            .as_mut()
            .expect("retained")
            .requires_inspection
            .get_or_insert_with(|| error.to_string());
    }
}

impl crate::client::adapter::PlacementOps for Operations {
    async fn place_stair(
        &self,
        clicked: [i32; 3],
        face: crate::BlockFace,
        target: [i32; 3],
        before: crate::NativeBlockState,
    ) -> Result<crate::client::survival::StairPlacementRecord> {
        self.place_cube_in(clicked, face, true, Some((target, before)), Family::Stair)
            .await?;
        self.stair_placement_record()
            .await?
            .ok_or_else(|| unavailable("stair placement capture missing"))
    }
    async fn stair_placement_record(
        &self,
    ) -> Result<Option<crate::client::survival::StairPlacementRecord>> {
        self.refresh_placement().await?;
        stair::record_in(&*self.bot.session.state.lock().await)
    }
    async fn place_slab(
        &self,
        clicked: [i32; 3],
        face: crate::BlockFace,
        target: [i32; 3],
        before: crate::NativeBlockState,
    ) -> Result<crate::client::survival::SlabPlacementRecord> {
        self.place_cube_in(clicked, face, true, Some((target, before)), Family::Slab)
            .await?;
        self.slab_placement_record()
            .await?
            .ok_or_else(|| unavailable("slab placement capture missing"))
    }
    async fn slab_placement_record(
        &self,
    ) -> Result<Option<crate::client::survival::SlabPlacementRecord>> {
        self.refresh_placement().await?;
        slab::record_in(&*self.bot.session.state.lock().await)
    }
    async fn place_cube(
        &self,
        support: [i32; 3],
        face: crate::BlockFace,
    ) -> Result<crate::client::survival::PlacementRecord> {
        self.place_cube_in(support, face, true, None, Family::Cube)
            .await?;
        self.placement_record()
            .await?
            .ok_or_else(|| unavailable("common placement capture missing"))
    }
    async fn placement_record(&self) -> Result<Option<crate::client::survival::PlacementRecord>> {
        self.refresh_placement().await?;
        let state = self.bot.session.state.lock().await;
        if state
            .common_placement
            .as_ref()
            .is_some_and(|c| c.replacement.is_some() || c.slab || c.stairs.is_some())
        {
            Ok(state.retired_common_placement.clone())
        } else {
            common_record_in(&state)
        }
    }
    async fn place_cube_replacing(
        &self,
        clicked: [i32; 3],
        face: crate::BlockFace,
        target: [i32; 3],
        before: crate::NativeBlockState,
    ) -> Result<crate::client::survival::ReplaceableCubePlacementRecord> {
        self.place_cube_in(clicked, face, true, Some((target, before)), Family::Cube)
            .await?;
        self.replaceable_cube_placement_record()
            .await?
            .ok_or_else(|| unavailable("replacement placement capture missing"))
    }
    async fn replaceable_cube_placement_record(
        &self,
    ) -> Result<Option<crate::client::survival::ReplaceableCubePlacementRecord>> {
        self.refresh_placement().await?;
        replacement_record_in(&*self.bot.session.state.lock().await)
    }
}
impl Operations {
    async fn refresh_placement(&self) -> Result<Option<crate::client::survival::PlacementRecord>> {
        let mut state = self.bot.session.state.lock().await;
        let Some(intent) = state
            .common_placement
            .as_ref()
            .and_then(|c| {
                state
                    .placement
                    .as_ref()
                    .filter(|p| p.intent.sequence == c.sequence)
            })
            .map(|p| p.intent.clone())
        else {
            return common_record_in(&state);
        };
        if self.bot.session.stopped.load(Ordering::Acquire)
            || state.failure.is_some()
            || !state.ready
            || !matches!(state.phase, Phase::Play)
        {
            if let Some(p) = state.placement.as_mut().filter(|p| p.observation.is_none()) {
                p.requires_inspection
                    .get_or_insert_with(|| "placement connection closed or uncertain".into());
            }
        }
        common_placement_context_received(&mut state);
        observe_in(
            &mut state,
            self.bot.session.id,
            self.bot.session.started.elapsed().as_millis() as u64 / 50,
            &intent,
        )?;
        common_record_in(&state)
    }
}

pub(in crate::versions::java_1_21_11::client) fn reconcile_ledger(state: &mut State) {
    if let Some(intent) = state
        .placement
        .as_ref()
        .filter(|r| r.dispatched)
        .map(|r| r.intent.clone())
    {
        let _ = observe_in(
            state,
            intent.connection_id,
            state.reconstruction.tick,
            &intent,
        );
    }
}

fn replacement_record_in(
    state: &State,
) -> Result<Option<crate::client::survival::ReplaceableCubePlacementRecord>> {
    let Some(capture) = state
        .common_placement
        .as_ref()
        .filter(|c| !c.slab && c.stairs.is_none())
        .and_then(|c| c.replacement.as_ref())
    else {
        return Ok(state.retired_replaceable_placement.clone());
    };
    let r = common_record_in(state)?
        .ok_or_else(|| unavailable("replacement placement capture missing"))?;
    Ok(Some(
        crate::client::survival::ReplaceableCubePlacementRecord {
            first_outline: capture.first_outline.clone(),
            replaces_clicked: capture.replaces_clicked,
            id: r.id,
            initial: r.initial,
            support: r.support,
            support_state: r.support_state,
            face: r.face,
            cursor: r.cursor,
            target: r.target,
            before: r.before,
            expected: r.expected,
            held_before: r.held_before,
            held_receive_sequence: r.held_receive_sequence,
            send: r.send,
            target_receipt: r.target_receipt,
            material_receipt: r.material_receipt,
            processing: r.processing,
            requires_inspection: r.requires_inspection,
            stage: r.stage,
        },
    ))
}
