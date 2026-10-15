//! Contact models fenced by one selected movement owner and complete receipts.
use super::{
    EntityDataField, EntityDataValue, EntityHistoryCursor, EntityHistoryKind, EntityHistoryValue,
    EntityId, EntityPose, LivingInterpolationPose, LivingTickBody, MAX_ENTITY_HISTORY_READ,
    MAX_ENTITY_HISTORY_RECORDS, SpawnLedger,
    contact_capabilities::{self, DeferredContact},
    contact_tick::{ContactDispatch, ContactTickBody, ContactTickWorld},
};
use crate::{
    MovementOwnerState, MovementPacketTrigger, MovementRefusal, Result,
    client::{
        SessionStamp,
        ui::{PlayerListObservation, TeamsObservation},
    },
};
use std::collections::BTreeMap;

/// One remote body's numerical state in an owned contact frame.
/// No field is an actual current position, received velocity or live target factory.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct EntityContactSample {
    /// Original received spawn lifetime.
    pub entity: EntityId,
    /// Numerical feet after this owner's explicit model tick.
    pub model_position: [f64; 3],
    /// Numerical delta movement of the qualified remote AI-step lane.
    /// Full Entity.baseTick fluid/other callbacks are not included; this is
    /// separate from packet velocity samples and does not drive remote travel.
    pub model_velocity: [f64; 3],
}

/// Numerical origin and progress of automatic contact discovery for one owner.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct EntityContactFrame {
    /// Complete receipt drain at owner admission; zero model ticks precede it.
    /// This is a declared local origin, not the age of a graphical native client.
    pub origin_after_record: u64,
    /// Latest complete history record consumed before this frame.
    pub applied_record: u64,
    /// Original receive boundary at this model fence.
    pub receive_sequence: u64,
    /// Explicit ticks advanced by this selected owner.
    pub model_tick: u64,
    /// Ordered inherited pair calls, including original early returns.
    pub pair_calls: usize,
    /// Bounded surviving supported remote bodies in numeric ID order.
    pub bodies: Vec<EntityContactSample>,
}

#[derive(Clone, Debug)]
struct Inputs {
    entity: EntityId,
    name: String,
    uuid: Option<[u8; 16]>,
    alive: bool,
    pose: EntityPose,
    spectator: bool,
    baby: f32,
    tame: bool,
    owner_uuid: Option<[u8; 16]>,
    owner_known: bool,
}

#[derive(Clone, Debug)]
struct BoatScope {
    entity: EntityId,
    passengers: Vec<i32>,
    ignored: Vec<EntityId>,
}

#[derive(Clone, Debug)]
pub(crate) struct OwnedContacts {
    pub world: ContactTickWorld,
    session: SessionStamp,
    owner: u64,
    cursor: Option<EntityHistoryCursor>,
    fresh: u64,
    inputs: BTreeMap<u64, Inputs>,
    deferred: BTreeMap<i32, DeferredContact>,
    origin: u64,
    tick: u64,
    receive_sequence: u64,
    valid: bool,
    boat: Option<BoatScope>,
    native_keys: BTreeMap<i32, u64>,
    own_glide: bool,
    own_identity: Option<([u8; 16], String)>,
    // Living bodies excluded from geometry can still be modern wolf owners.
    other_team_owners: BTreeMap<i32, (EntityId, [u8; 16])>,
    checkpoint_errors: Vec<(Option<EntityId>, MovementRefusal)>,
}
/// Unticked numerical receipt reducer at the public eviction boundary. Storage
/// follows active lifetimes and native map capacity, rather than receipt count.
#[derive(Clone, Debug)]
pub(super) struct ContactCheckpoint {
    model: OwnedContacts,
    own_metadata: BTreeMap<u8, (EntityDataValue, super::EntityHistoryRecord)>,
    errors: BTreeMap<i32, (Option<EntityId>, MovementRefusal)>,
    global_error: Option<MovementRefusal>,
    own_id: i32,
}
impl ContactCheckpoint {
    pub(super) fn new(session: SessionStamp, own_id: i32) -> Result<Self> {
        Ok(Self {
            model: OwnedContacts::empty(session, own_id)?,
            own_metadata: BTreeMap::new(),
            errors: BTreeMap::new(),
            global_error: None,
            own_id,
        })
    }

    pub(super) fn advance(
        &mut self,
        record: super::EntityHistoryRecord,
        owner: Option<Option<[u8; 16]>>,
    ) {
        // World/order failures cannot be repaired by later receipt deltas.
        // Keep the bounded refusal until the world checkpoint is superseded.
        if self.global_error.is_some() {
            return;
        }
        match &record.kind {
            EntityHistoryKind::Spawn(m) => {
                self.errors.remove(&m.entity.id.native_id());
            }
            EntityHistoryKind::Removed { native_id, .. } => {
                self.errors.remove(native_id);
            }
            _ => {}
        }
        let fresh = self.model.fresh;
        if let Err(error) = self
            .model
            .reduce_record(&record, true, None, &mut self.own_metadata)
        {
            // Even an unavailable numerical spawn contributes native map order.
            // A later boat scope may legitimately ignore this body's geometry.
            if let EntityHistoryKind::Spawn(m) = &record.kind {
                if self.model.world.order.body(self.model.fresh).is_none()
                    || self.model.fresh == fresh
                {
                    if self.model.fresh == fresh {
                        let Some(next) = fresh.checked_add(1) else {
                            self.global_error.get_or_insert_with(|| refusal(&error));
                            return;
                        };
                        self.model.fresh = next;
                    }
                    let key = self.model.fresh;
                    if let Err(order_error) = self
                        .model
                        .world
                        .admit(tick_body(key, m.entity.id.native_id(), false), None)
                    {
                        self.global_error
                            .get_or_insert_with(|| refusal(&order_error));
                    }
                }
                self.model
                    .native_keys
                    .insert(m.entity.id.native_id(), self.model.fresh);
                if m.entity.type_name.is_none()
                    || m.entity.id.session() != record.session
                    || m.entity.id.spawn_sequence() != record.receive_sequence
                {
                    self.global_error.get_or_insert_with(|| refusal(&error));
                }
            }
            if matches!(
                record.kind,
                EntityHistoryKind::Passengers {
                    passengers: EntityHistoryValue::RetentionLimit,
                    ..
                }
            ) {
                self.global_error.get_or_insert_with(|| refusal(&error));
            }
            let affected = match &record.kind {
                EntityHistoryKind::Spawn(m) | EntityHistoryKind::Motion(m) => {
                    vec![(m.entity.id.native_id(), Some(m.entity.id))]
                }
                EntityHistoryKind::Metadata {
                    native_id, entity, ..
                }
                | EntityHistoryKind::Equipment {
                    native_id, entity, ..
                }
                | EntityHistoryKind::Scale {
                    native_id, entity, ..
                }
                | EntityHistoryKind::Status {
                    native_id, entity, ..
                }
                | EntityHistoryKind::Animation {
                    native_id, entity, ..
                } => vec![(*native_id, *entity)],
                EntityHistoryKind::Passengers {
                    entity, passengers, ..
                } => self
                    .model
                    .inputs
                    .values()
                    .filter(|input| {
                        Some(input.entity) == *entity
                            || match passengers {
                                EntityHistoryValue::Retained { value } => {
                                    value.iter().any(|(_, id)| *id == Some(input.entity))
                                }
                                EntityHistoryValue::RetentionLimit => true,
                            }
                    })
                    .map(|input| (input.entity.native_id(), Some(input.entity)))
                    .collect(),
                _ => vec![(self.own_id, None)],
            };
            for (native_id, entity) in affected {
                self.errors
                    .entry(native_id)
                    .or_insert((entity, refusal(&error)));
            }
        }
        self.model.apply_owner(&record, owner);
    }

    fn seed(
        &self,
        target: &mut OwnedContacts,
        surviving: &[EntityId],
        own_metadata: &mut BTreeMap<u8, (EntityDataValue, super::EntityHistoryRecord)>,
    ) -> Result<()> {
        let own = target.world.order.body(1).unwrap().native_id;
        if self.model.world.order.body(1).map(|b| b.native_id) != Some(own) {
            return Err(failure("contact checkpoint local player changed"));
        }
        target.world = self.model.world.clone();
        target.inputs = self.model.inputs.clone();
        target.other_team_owners = self.model.other_team_owners.clone();
        for (id, _) in target.other_team_owners.values_mut() {
            id.session.connection_id = target.session.connection_id;
        }
        target
            .other_team_owners
            .retain(|_, (id, _)| surviving.contains(id));
        target.deferred = self.model.deferred.clone();
        for body in target.deferred.values_mut() {
            body.entity.session.connection_id = target.session.connection_id;
        }
        target.deferred.retain(|_, body| {
            surviving.contains(&body.entity)
                && !target
                    .boat
                    .as_ref()
                    .is_some_and(|b| b.ignored.contains(&body.entity))
        });
        for input in target.inputs.values_mut() {
            input.entity.session.connection_id = target.session.connection_id;
        }
        target.native_keys = self.model.native_keys.clone();
        target.fresh = self.model.fresh;
        if let Some(scope) = &target.boat {
            for id in &scope.ignored {
                if let Some(&key) = target.native_keys.get(&id.native_id()) {
                    target
                        .world
                        .order
                        .bootstrap_placeholder(key)
                        .map_err(failure)?;
                }
            }
        }
        let discard = target
            .inputs
            .iter()
            .filter(|(_, input)| {
                !surviving.contains(&input.entity)
                    || target
                        .boat
                        .as_ref()
                        .is_some_and(|scope| scope.ignored.contains(&input.entity))
            })
            .map(|(&key, _)| key)
            .collect::<Vec<_>>();
        for key in discard {
            target
                .world
                .order
                .bootstrap_placeholder(key)
                .map_err(failure)?;
            target.inputs.remove(&key);
            target.world.bodies.remove(&key);
            target.world.spatial.remove(key).map_err(failure)?;
        }
        target.checkpoint_errors = self
            .errors
            .values()
            .map(|(entity, error)| {
                (
                    entity.map(|mut id| {
                        id.session.connection_id = target.session.connection_id;
                        id
                    }),
                    {
                        let mut error = error.clone();
                        if let Some(id) = &mut error.context_mut().entity {
                            id.session.connection_id = target.session.connection_id;
                        }
                        error
                    },
                )
            })
            .collect();
        if let Some(error) = &self.global_error {
            target.checkpoint_errors.push((None, error.clone()));
        }
        *own_metadata = self.own_metadata.clone();
        Ok(())
    }
}

// Local failures are classified at their construction site, never from Display text.
fn refusal(error: &crate::Error) -> MovementRefusal {
    error.movement_refusal().cloned().unwrap_or_else(|| {
        let mut value = MovementRefusal::permanent(error.to_string());
        if error.kind() == crate::ErrorKind::Unsupported {
            value = MovementRefusal::Unsupported {
                context: value.context().clone(),
            };
        }
        value
    })
}
fn failure(reason: impl std::fmt::Debug) -> crate::Error {
    MovementRefusal::permanent(format!("entity contact model unavailable: {reason:?}")).into_error()
}
fn unsupported(reason: impl std::fmt::Debug) -> crate::Error {
    let context = refusal(&failure(reason)).context().clone();
    MovementRefusal::Unsupported { context }.into_error()
}
fn transient(reason: impl std::fmt::Debug) -> crate::Error {
    let context = refusal(&failure(reason)).context().clone();
    MovementRefusal::Transient { context }.into_error()
}
fn entity_failure(error: crate::Error, input: &Inputs) -> crate::Error {
    let mut value = refusal(&error);
    let c = value.context_mut();
    c.entity = Some(input.entity);
    c.native_entity_id = Some(input.entity.native_id());
    c.entity_type = Some(input.name.clone());
    value.into_error()
}

impl OwnedContacts {
    fn empty(session: SessionStamp, own_native_id: i32) -> Result<Self> {
        let mut model = Self {
            world: ContactTickWorld::new(session.version, 1),
            session,
            owner: 0,
            cursor: None,
            fresh: 1,
            inputs: BTreeMap::new(),
            deferred: BTreeMap::new(),
            origin: 0,
            tick: 0,
            receive_sequence: 0,
            valid: true,
            boat: None,
            native_keys: BTreeMap::from([(own_native_id, 1)]),
            own_glide: false,
            own_identity: None,
            other_team_owners: BTreeMap::new(),
            checkpoint_errors: Vec::new(),
        };
        model.world.admit(tick_body(1, own_native_id, true), None)?;
        Ok(model)
    }

    pub(crate) fn has_bodies(&self) -> bool {
        !self.world.bodies.is_empty() || self.deferred.values().any(|b| b.relevant(false))
    }

    pub(crate) fn tick_boat(
        &mut self,
        record: &crate::client::VehicleControlRecord,
        vehicle: &crate::client::VehicleObservation,
        ledger: &SpawnLedger,
        relations: &crate::client::vehicle::PassengerLedger,
        input: crate::client::VehicleInput,
        block_at: &mut impl FnMut([i32; 3]) -> Result<crate::NativeBlockState>,
    ) -> Result<(crate::client::BoatCollisionSample, crate::client::BoatFrame)> {
        self.tick_boat_inner(record, vehicle, ledger, relations, input, block_at)
            .map_err(|error| self.annotate(error, MovementOwnerState::Invalidated))
    }

    fn tick_boat_inner(
        &mut self,
        record: &crate::client::VehicleControlRecord,
        vehicle: &crate::client::VehicleObservation,
        ledger: &SpawnLedger,
        relations: &crate::client::vehicle::PassengerLedger,
        input: crate::client::VehicleInput,
        block_at: &mut impl FnMut([i32; 3]) -> Result<crate::NativeBlockState>,
    ) -> Result<(crate::client::BoatCollisionSample, crate::client::BoatFrame)> {
        use crate::client::{
            physics,
            vehicle::collision::{self, BoatCollisionGeometry, LivingBoatBody},
        };
        let scope = self
            .boat
            .as_ref()
            .ok_or_else(|| failure("missing boat contact scope"))?;
        let root = self.native_keys[&scope.entity.native_id()];
        let boat = record
            .boat_motion
            .as_ref()
            .ok_or_else(|| failure("missing boat prediction"))?;
        let mut seed = boat.frames.last().unwrap_or(&boat.initial_frame).clone();
        if let Some(velocity) = boat.pending_velocity {
            seed.velocity = velocity;
        }
        let deferred = &self.deferred;
        contact_capabilities::check(deferred.values(), boat_query(seed.bounds), true)?;
        contact_capabilities::check_peers(deferred.values(), &self.world, true)?;
        let own_bounds = std::cell::RefCell::new(seed.bounds);
        let fixed = ledger
            .capture(self.session, self.receive_sequence)
            .entities
            .into_iter()
            .filter(|e| {
                e.id != scope.entity && e.type_name.as_deref().is_some_and(collision::rigid)
            })
            .map(|e| {
                let p = ledger
                    .capture_motion(self.session, e.id, self.receive_sequence)?
                    .position
                    .ok_or_else(|| failure("missing rigid position"))?
                    .value
                    .position;
                let d = self
                    .session
                    .version
                    .table()
                    .entity_dimensions(e.type_name.as_deref().unwrap())
                    .ok_or_else(|| failure("missing rigid dimensions"))?;
                let h = f64::from(d.width as f32) / 2.;
                Ok([
                    p[0] - h,
                    p[1],
                    p[2] - h,
                    p[0] + h,
                    p[1] + f64::from(d.height as f32),
                    p[2] + h,
                ])
            })
            .collect::<Result<Vec<_>>>()?;
        for bounds in &fixed {
            contact_capabilities::check(deferred.values(), boat_query(*bounds), true)?;
        }
        let version = self.session.version;
        let inputs = &self.inputs;
        let mut guarded_blocks = |p: [i32; 3]| {
            contact_capabilities::check(deferred.values(), contact_capabilities::cell(p), true)?;
            block_at(p)
        };
        let read = std::cell::RefCell::new(&mut guarded_blocks);
        let origin = self.origin;
        let applied = self.cursor.map_or(0, EntityHistoryCursor::ordinal);
        let tick = self.tick + 1;
        let result = self.world.tick_geometry(
            root,
            |bodies, index| {
                let living = bodies
                    .iter()
                    .map(|(&key, b)| {
                        let p = b.motion.frame().interpolation.pose.position;
                        let bounds = b.contact().bounds;
                        LivingBoatBody {
                            entity: inputs[&key].entity,
                            bounds: array(bounds),
                            pushable: b.pushable,
                            spectator: inputs[&key].spectator,
                            geometry: BoatCollisionGeometry::LivingInterpolation {
                                origin_after_record: origin,
                                applied_record: applied,
                                model_tick: tick,
                                position: p,
                                dimensions: [b.width, b.height],
                            },
                        }
                    })
                    .collect::<Vec<_>>();
                let mut sample = collision::sample_with_living(
                    record,
                    vehicle,
                    ledger,
                    relations,
                    self.receive_sequence,
                    Some(&living),
                    &deferred.values().map(|b| b.entity).collect::<Vec<_>>(),
                )?
                .ok_or_else(|| failure("missing boat collision sample"))?;
                // The complete retained model owns entity membership. Terrain is
                // checked independently before marking each queried chunk known.
                let query = boat_query(seed.bounds);
                let min = [
                    ((query.min_x - 2.) / 16.).floor() as i32,
                    ((query.min_z - 2.) / 16.).floor() as i32,
                ];
                let max = [
                    ((query.max_x + 2.) / 16.).floor() as i32,
                    ((query.max_z + 2.) / 16.).floor() as i32,
                ];
                for x in min[0]..=max[0] {
                    for z in min[1]..=max[1] {
                        read.borrow_mut()([x * 16, query.min_y.floor() as i32, z * 16])?;
                        index.set_chunk_known([x, z], true).map_err(failure)?;
                    }
                }
                let keys = index.geometry_keys(query, &[root]).map_err(failure)?;
                sample
                    .bodies
                    .retain(|b| keys.contains(&self.native_keys[&b.entity.native_id()]));
                sample.bodies.sort_by_key(|b| {
                    keys.iter()
                        .position(|key| *key == self.native_keys[&b.entity.native_id()])
                        .unwrap()
                });
                let frame = crate::client::vehicle::control::boat_step(
                    record,
                    input,
                    Some(&sample),
                    &mut *read.borrow_mut(),
                )?
                .ok_or_else(|| failure("missing boat frame"))?;
                contact_capabilities::check(
                    deferred.values(),
                    contact_capabilities::hull(boat_query(seed.bounds), boat_query(frame.bounds)),
                    true,
                )?;
                *own_bounds.borrow_mut() = frame.bounds;
                Ok((sample, frame))
            },
            |remote| {
                let interpolation = remote.motion.frame().interpolation;
                if version == crate::MinecraftVersion::Java1_16_1
                    || interpolation.remaining_steps == 0
                {
                    return Ok(None);
                }
                let Some(query) = remote.motion.collision_query_position() else {
                    return Ok(Some(true));
                };
                let half = f64::from(remote.width / 2.);
                let bounds = [
                    query[0] - half,
                    query[1],
                    query[2] - half,
                    query[0] + half,
                    query[1] + f64::from(remote.height),
                    query[2] + half,
                ];
                let clear = physics::contact_block_collision_free(
                    version,
                    bounds,
                    interpolation.pose.position[1],
                    &mut *read.borrow_mut(),
                )? && !overlap(bounds, *own_bounds.borrow())
                    && !fixed.iter().any(|b| overlap(bounds, *b));
                Ok(Some(clear))
            },
            |remote| {
                let input = &inputs[&remote.key];
                remote.pushable = input.alive
                    && (input.spectator
                        || !physics::contact_climbable(
                            version,
                            remote.motion.frame().interpolation.pose.position,
                            &mut *read.borrow_mut(),
                        )?);
                Ok(())
            },
        )?;
        self.tick = tick;
        Ok(result)
    }

    pub(crate) fn tick_player(
        &mut self,
        body: &mut crate::client::physics::Body,
        environment: &crate::client::physics::Environment,
        controls: crate::client::control::Controls,
        ground_requested: bool,
        own_team: Option<&(String, crate::client::ui::TeamCollision)>,
        block_at: &mut impl FnMut([i32; 3]) -> Result<crate::NativeBlockState>,
    ) -> Result<(
        Option<crate::client::control::GroundJumpOutcome>,
        EntityContactFrame,
    )> {
        self.tick_player_inner(
            body,
            environment,
            controls,
            ground_requested,
            own_team,
            block_at,
        )
        .map_err(|error| self.annotate(error, MovementOwnerState::Invalidated))
    }

    fn tick_player_inner(
        &mut self,
        body: &mut crate::client::physics::Body,
        environment: &crate::client::physics::Environment,
        controls: crate::client::control::Controls,
        ground_requested: bool,
        own_team: Option<&(String, crate::client::ui::TeamCollision)>,
        block_at: &mut impl FnMut([i32; 3]) -> Result<crate::NativeBlockState>,
    ) -> Result<(
        Option<crate::client::control::GroundJumpOutcome>,
        EntityContactFrame,
    )> {
        use crate::client::{LivingContactBody, LivingPushBody, physics};
        let version = self.session.version;
        let deferred = &self.deferred;
        contact_capabilities::check(deferred.values(), aabb(body.bounds), false)?;
        contact_capabilities::check_peers(deferred.values(), &self.world, false)?;
        let mut guarded_blocks = |p: [i32; 3]| {
            contact_capabilities::check(deferred.values(), contact_capabilities::cell(p), false)?;
            block_at(p)
        };
        let read = std::cell::RefCell::new(&mut guarded_blocks);
        let mut local = LivingContactBody {
            push: LivingPushBody {
                position: body.position,
                velocity: body.velocity,
                no_physics: environment.spectator,
                vehicle: false,
                pushable: true,
            },
            bounds: aabb(body.bounds),
            sleeping: false,
            legacy_pushthrough: 0.,
            team: own_team.map(|t| t.1),
        };
        if !self.world.bodies.is_empty() && !environment.spectator {
            local.push.pushable =
                !physics::contact_climbable(version, body.position, &mut *read.borrow_mut())?;
        }
        let mut outcome = None;
        let inputs = &self.inputs;
        let nonempty = !self.world.bodies.is_empty();
        let pairs = self.world.tick(
            &mut local,
            (own_team.map(|t| t.0.as_str()), environment.spectator),
            false,
            |local| {
                let before = local.bounds;
                body.velocity = local.push.velocity;
                outcome = physics::tick_with_ground_jump(
                    version,
                    body,
                    environment,
                    controls,
                    ground_requested,
                    &mut *read.borrow_mut(),
                )?;
                local.push.position = body.position;
                local.push.velocity = body.velocity;
                local.bounds = aabb(body.bounds);
                contact_capabilities::check(
                    deferred.values(),
                    contact_capabilities::hull(before, local.bounds),
                    false,
                )?;
                local.push.pushable = environment.spectator
                    || (!nonempty
                        || !physics::contact_climbable(
                            version,
                            body.position,
                            &mut *read.borrow_mut(),
                        )?);
                Ok(())
            },
            |remote| {
                let interpolation = remote.motion.frame().interpolation;
                if version == crate::MinecraftVersion::Java1_16_1
                    || interpolation.remaining_steps == 0
                {
                    return Ok(None);
                }
                let Some(query) = remote.motion.collision_query_position() else {
                    return Ok(Some(true));
                };
                let half = f64::from(remote.width / 2.);
                let bounds = [
                    query[0] - half,
                    query[1],
                    query[2] - half,
                    query[0] + half,
                    query[1] + f64::from(remote.height),
                    query[2] + half,
                ];
                Ok(Some(physics::contact_block_collision_free(
                    version,
                    bounds,
                    interpolation.pose.position[1],
                    &mut *read.borrow_mut(),
                )?))
            },
            |remote| {
                let input = &inputs[&remote.key];
                remote.pushable = input.alive
                    && (input.spectator
                        || !physics::contact_climbable(
                            version,
                            remote.motion.frame().interpolation.pose.position,
                            &mut *read.borrow_mut(),
                        )?);
                Ok(())
            },
        )?;
        body.velocity = local.push.velocity;
        Ok((outcome, self.frame(pairs)))
    }

    pub(crate) fn new(
        session: SessionStamp,
        owner: u64,
        own_native_id: i32,
        ledger: &SpawnLedger,
        receive_sequence: u64,
    ) -> Result<Self> {
        Self::new_scoped(
            session,
            owner,
            own_native_id,
            ledger,
            receive_sequence,
            None,
            false,
        )
    }

    pub(crate) fn new_glide(
        session: SessionStamp,
        owner: u64,
        own_native_id: i32,
        ledger: &SpawnLedger,
        receive_sequence: u64,
    ) -> Result<Self> {
        Self::new_scoped(
            session,
            owner,
            own_native_id,
            ledger,
            receive_sequence,
            None,
            true,
        )
    }

    pub(crate) fn for_boat(
        record: &crate::client::VehicleControlRecord,
        ledger: &SpawnLedger,
    ) -> Result<Self> {
        let result = (|| {
            let mount = record.id.mount();
            let entity = mount
                .vehicle()
                .ok_or_else(|| failure("boat contact spawn missing"))?;
            let own = record
                .vehicle
                .player_native_id
                .ok_or_else(|| failure("boat contact player missing"))?;
            let passengers = record
                .vehicle
                .passengers
                .as_ref()
                .ok_or_else(|| failure("boat contact passengers missing"))?
                .value
                .clone();
            let ignored = passengers
                .iter()
                .filter(|&&id| id != own)
                .map(|&id| {
                    ledger
                        .identity(mount.session(), id)
                        .ok_or_else(|| failure("boat contact passenger lifetime missing"))
                })
                .collect::<Result<Vec<_>>>()?;
            Self::new_scoped(
                mount.session(),
                record.id.attempt(),
                own,
                ledger,
                record.initial.receive_sequence,
                Some(BoatScope {
                    entity,
                    passengers,
                    ignored,
                }),
                false,
            )
        })();
        result.map_err(|error| {
            let mut value = refusal(&error);
            let c = value.context_mut();
            c.session = Some(record.id.mount().session());
            c.owner_id = Some(record.id.attempt());
            c.owner_state = MovementOwnerState::NotAdmitted;
            value.into_error()
        })
    }

    fn new_scoped(
        session: SessionStamp,
        owner: u64,
        own_native_id: i32,
        ledger: &SpawnLedger,
        receive_sequence: u64,
        boat: Option<BoatScope>,
        own_glide: bool,
    ) -> Result<Self> {
        let result = (|| {
            let mut model = Self::empty(session, own_native_id)?;
            model.owner = owner;
            model.boat = boat;
            model.own_glide = own_glide;
            model.drain(ledger, receive_sequence, true)?;
            model.origin = model.cursor.map_or(0, EntityHistoryCursor::ordinal);
            if let Some(scope) = &model.boat {
                let root = *model
                    .native_keys
                    .get(&scope.entity.native_id())
                    .ok_or_else(|| failure("boat contact root history missing"))?;
                let children = scope
                    .passengers
                    .iter()
                    .map(|id| {
                        model
                            .native_keys
                            .get(id)
                            .copied()
                            .ok_or_else(|| failure("boat contact child history missing"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let mut parent = model
                    .world
                    .order
                    .body(root)
                    .ok_or_else(|| failure("boat contact root retired"))?
                    .clone();
                parent.passengers = children.clone();
                parent.player_passengers = 1;
                model.world.order.update(parent).map_err(failure)?;
                for key in children {
                    let mut child = model
                        .world
                        .order
                        .body(key)
                        .ok_or_else(|| failure("boat contact child retired"))?
                        .clone();
                    child.passenger_of = Some(root);
                    model.world.order.update(child).map_err(failure)?;
                }
            }
            Ok(model)
        })();
        result.map_err(|error| {
            let mut value = refusal(&error);
            let c = value.context_mut();
            c.session = Some(session);
            c.owner_id = Some(owner);
            c.owner_state = MovementOwnerState::NotAdmitted;
            value.into_error()
        })
    }

    pub(crate) fn refresh(
        &mut self,
        owner: u64,
        ledger: &SpawnLedger,
        sequence: u64,
    ) -> Result<()> {
        let result = (|| {
            if !self.valid || owner != self.owner {
                self.valid = false;
                self.world.order.invalidate();
                return Err(failure("discarded contact owner"));
            }
            let mut next = self.clone();
            match next.drain(ledger, sequence, false) {
                Ok(()) => {
                    *self = next;
                    Ok(())
                }
                Err(error) => {
                    self.valid = false;
                    self.world.order.invalidate();
                    Err(error)
                }
            }
        })();
        result.map_err(|error| self.annotate(error, MovementOwnerState::Invalidated))
    }

    fn drain(&mut self, ledger: &SpawnLedger, sequence: u64, bootstrap: bool) -> Result<()> {
        self.drain_inner(ledger, sequence, bootstrap)
            .map_err(|error| {
                self.annotate(
                    error,
                    if bootstrap {
                        MovementOwnerState::NotAdmitted
                    } else {
                        MovementOwnerState::Invalidated
                    },
                )
            })
    }

    fn drain_inner(&mut self, ledger: &SpawnLedger, sequence: u64, bootstrap: bool) -> Result<()> {
        self.own_identity = ledger.1.contact_identity.clone();
        let mut records = 0;
        let mut own_metadata = BTreeMap::new();
        let surviving = ledger
            .capture(self.session, sequence)
            .entities
            .into_iter()
            .map(|e| e.id)
            .collect::<Vec<_>>();
        if bootstrap {
            let (checkpoint, boundary) = ledger.contact_checkpoint(self.session)?;
            if ledger.contact_checkpoint_required() {
                return Err(failure("contact checkpoint local player missing"));
            }
            if let Some(checkpoint) = checkpoint {
                checkpoint.seed(self, &surviving, &mut own_metadata)?;
            }
            self.cursor = Some(boundary);
        }
        loop {
            let page = ledger.history_after(
                self.session,
                sequence,
                self.cursor,
                MAX_ENTITY_HISTORY_READ,
            )?;
            if page.gap.is_some() {
                return Err(failure("contact history gap"));
            }
            if page.session != self.session {
                return Err(failure("contact world changed"));
            }
            records += page.records.len();
            if records > MAX_ENTITY_HISTORY_RECORDS {
                return Err(failure("contact history drain budget"));
            }
            for record in &page.records {
                self.reduce_record(record, bootstrap, Some(&surviving), &mut own_metadata)?;
                self.apply_owner(
                    record,
                    ledger.1.contact_owners.get(&record.ordinal).copied(),
                );
            }
            self.cursor = Some(page.next_cursor);
            if !page.has_more {
                break;
            }
        }
        for (index, (value, source)) in own_metadata {
            self.own_metadata(index, &value).map_err(|error| {
                let mut value = refusal(&error);
                let c = value.context_mut();
                c.packet = Some(MovementPacketTrigger {
                    packet_type: "Metadata".into(),
                    opcode: 0,
                });
                c.receive_sequence = Some(source.receive_sequence);
                c.record_ordinal = Some(source.ordinal);
                if let EntityHistoryKind::Metadata {
                    native_id, entity, ..
                } = source.kind
                {
                    c.native_entity_id = Some(native_id);
                    c.entity = entity;
                }
                c.entity_type = Some("minecraft:player".into());
                value.into_error()
            })?;
        }
        for (entity, error) in &self.checkpoint_errors {
            if entity.is_none()
                || entity.is_some_and(|id| {
                    surviving.contains(&id)
                        && !self
                            .boat
                            .as_ref()
                            .is_some_and(|scope| scope.ignored.contains(&id))
                })
            {
                return Err(error.clone().into_error());
            }
        }
        self.checkpoint_errors.clear();
        // Deferred bodies retain native order; only separately proven envelopes
        // can establish irrelevance at the movement boundary.
        for spawn in ledger.capture(self.session, sequence).entities {
            if !self.inputs.values().any(|i| i.entity == spawn.id)
                && !self.deferred.values().any(|i| i.entity == spawn.id)
                && !self.boat.as_ref().is_some_and(|scope| {
                    scope.ignored.contains(&spawn.id)
                        || spawn
                            .type_name
                            .as_deref()
                            .is_some_and(crate::client::vehicle::collision::rigid)
                })
            {
                let mut value = refusal(&unsupported(format!(
                    "contact lifetime lacks capability state: {:?}",
                    spawn.type_name
                )));
                let context = value.context_mut();
                context.entity = Some(spawn.id);
                context.native_entity_id = Some(spawn.id.native_id());
                context.entity_type = spawn.type_name;
                context.packet = Some(MovementPacketTrigger {
                    packet_type: "Spawn".into(),
                    opcode: 0,
                });
                context.receive_sequence = Some(spawn.id.spawn_sequence());
                return Err(value.into_error());
            }
        }
        self.receive_sequence = sequence;
        if self.boat.is_some() {
            for spawn in ledger.capture(self.session, sequence).entities {
                if !spawn
                    .type_name
                    .as_deref()
                    .is_some_and(crate::client::vehicle::collision::rigid)
                {
                    continue;
                }
                if self
                    .boat
                    .as_ref()
                    .is_some_and(|scope| scope.entity == spawn.id)
                {
                    continue;
                }
                let motion = ledger.capture_motion(self.session, spawn.id, sequence)?;
                let position = motion
                    .position
                    .ok_or_else(|| failure("rigid contact position missing"))?
                    .value
                    .position;
                let dimensions = self
                    .session
                    .version
                    .table()
                    .entity_dimensions(spawn.type_name.as_deref().unwrap())
                    .ok_or_else(|| failure("rigid contact dimension missing"))?;
                let half = f64::from(dimensions.width as f32) / 2.;
                let bounds = crate::client::Aabb {
                    min_x: position[0] - half,
                    min_y: position[1],
                    min_z: position[2] - half,
                    max_x: position[0] + half,
                    max_y: position[1] + f64::from(dimensions.height as f32),
                    max_z: position[2] + half,
                };
                super::contact_tick::index_geometry(
                    &mut self.world.spatial,
                    self.native_keys[&spawn.id.native_id()],
                    position,
                    bounds,
                )?;
            }
        }
        Ok(())
    }

    fn annotate(&self, error: crate::Error, state: MovementOwnerState) -> crate::Error {
        let mut value = refusal(&error);
        let context = value.context_mut();
        context.session = Some(self.session);
        context.owner_id = (self.owner != 0).then_some(self.owner);
        context.owner_state = state;
        context.model_tick = Some(self.tick);
        value.into_error()
    }

    fn reduce_record(
        &mut self,
        record: &super::EntityHistoryRecord,
        bootstrap: bool,
        surviving: Option<&[EntityId]>,
        own_metadata: &mut BTreeMap<u8, (EntityDataValue, super::EntityHistoryRecord)>,
    ) -> Result<()> {
        self.reduce_record_inner(record, bootstrap, surviving, own_metadata)
            .map_err(|error| {
                let mut value = refusal(&error);
                let c = value.context_mut();
                let (entity, native, packet, opcode) = match &record.kind {
                    EntityHistoryKind::Spawn(m) => {
                        (Some(m.entity.id), Some(m.entity.id.native_id()), "Spawn", 0)
                    }
                    EntityHistoryKind::Motion(m) => (
                        Some(m.entity.id),
                        Some(m.entity.id.native_id()),
                        "Motion",
                        0,
                    ),
                    EntityHistoryKind::Status {
                        entity,
                        native_id,
                        status,
                    } => (*entity, Some(*native_id), "Status", i32::from(*status)),
                    EntityHistoryKind::Animation {
                        entity,
                        native_id,
                        animation,
                    } => (
                        *entity,
                        Some(*native_id),
                        "Animation",
                        i32::from(*animation),
                    ),
                    EntityHistoryKind::Metadata {
                        entity, native_id, ..
                    } => (*entity, Some(*native_id), "Metadata", 0),
                    EntityHistoryKind::Equipment {
                        entity, native_id, ..
                    } => (*entity, Some(*native_id), "Equipment", 0),
                    EntityHistoryKind::Scale {
                        entity, native_id, ..
                    } => (*entity, Some(*native_id), "Scale", 0),
                    EntityHistoryKind::Passengers {
                        entity, native_id, ..
                    } => (*entity, Some(*native_id), "Passengers", 0),
                    EntityHistoryKind::Removed { entity, native_id } => {
                        (*entity, Some(*native_id), "Removed", 0)
                    }
                    EntityHistoryKind::WorldChanged { .. } => (None, None, "WorldChanged", 0),
                    EntityHistoryKind::OwnPositionCorrection { .. } => {
                        (None, None, "OwnPositionCorrection", 0)
                    }
                };
                c.session = Some(record.session);
                c.entity = entity;
                c.native_entity_id = native;
                c.entity_type = match &record.kind {
                    EntityHistoryKind::Metadata { native_id, .. }
                        if self.native_keys.get(native_id) == Some(&1) =>
                    {
                        Some("minecraft:player".into())
                    }
                    EntityHistoryKind::Spawn(m) | EntityHistoryKind::Motion(m) => {
                        m.entity.type_name.clone()
                    }
                    _ => self
                        .inputs
                        .values()
                        .find(|i| Some(i.entity) == entity)
                        .map(|i| i.name.clone()),
                };
                c.packet = Some(MovementPacketTrigger {
                    packet_type: packet.into(),
                    opcode,
                });
                c.receive_sequence = Some(record.receive_sequence);
                c.record_ordinal = Some(record.ordinal);
                value.into_error()
            })
    }

    fn reduce_record_inner(
        &mut self,
        record: &super::EntityHistoryRecord,
        bootstrap: bool,
        surviving: Option<&[EntityId]>,
        own_metadata: &mut BTreeMap<u8, (EntityDataValue, super::EntityHistoryRecord)>,
    ) -> Result<()> {
        let own_id = self
            .world
            .order
            .body(self.world.own_key)
            .ok_or_else(|| failure("own contact lifetime missing"))?
            .native_id;
        // A new owner's numerical constructor origin belongs to its
        // current world. Earlier worlds never supply current bodies.
        if record.session != self.session {
            if bootstrap {
                return Ok(());
            }
            return Err(failure("contact record lifetime changed"));
        }
        let ordinal = record.ordinal;
        match &record.kind {
            EntityHistoryKind::Metadata {
                native_id, entity, ..
            }
            | EntityHistoryKind::Equipment {
                native_id, entity, ..
            }
            | EntityHistoryKind::Scale {
                native_id, entity, ..
            }
            | EntityHistoryKind::Status {
                native_id, entity, ..
            }
            | EntityHistoryKind::Animation {
                native_id, entity, ..
            } => {
                let (packet, opcode) = match &record.kind {
                    EntityHistoryKind::Metadata { .. } => ("Metadata", 0),
                    EntityHistoryKind::Equipment { .. } => ("Equipment", 0),
                    EntityHistoryKind::Scale { .. } => ("Scale", 0),
                    EntityHistoryKind::Status { status, .. } => ("Status", i32::from(*status)),
                    EntityHistoryKind::Animation { animation, .. } => {
                        ("Animation", i32::from(*animation))
                    }
                    _ => unreachable!(),
                };
                if let Some(body) = self.deferred.get_mut(native_id) {
                    if Some(body.entity) == *entity {
                        body.invalidate(record, packet, opcode);
                    }
                }
            }
            EntityHistoryKind::Passengers {
                native_id,
                passengers,
                ..
            } => {
                // Both the vehicle and its children can acquire passenger travel.
                // Unknown/retention-limited membership removes all spatial proofs.
                for (&id, body) in &mut self.deferred {
                    let affected = match passengers {
                        EntityHistoryValue::Retained { value } => {
                            *native_id == id || value.iter().any(|(child, _)| *child == id)
                        }
                        EntityHistoryValue::RetentionLimit => true,
                    };
                    if affected {
                        body.invalidate(record, "Passengers", 0);
                    }
                }
            }
            _ => {}
        }
        match &record.kind {
            EntityHistoryKind::Spawn(m) => {
                let id = m.entity.id;
                self.deferred.remove(&id.native_id());
                self.other_team_owners.remove(&id.native_id());
                let old = self
                    .world
                    .bodies
                    .iter()
                    .filter(|(_, body)| body.native_id == id.native_id())
                    .map(|(&key, _)| key)
                    .collect::<Vec<_>>();
                for key in old {
                    self.world.bodies.remove(&key);
                    self.inputs.remove(&key);
                    self.world.spatial.remove(key).map_err(failure)?;
                }
                if id.session() != self.session || id.spawn_sequence() != record.receive_sequence {
                    return Err(failure("contact spawn provenance"));
                }
                let Some(name) = m.entity.type_name.as_deref() else {
                    return Err(unsupported("unknown contact entity type"));
                };
                if matches!(
                    name,
                    "minecraft:bat"
                        | "minecraft:armor_stand"
                        | "minecraft:spider"
                        | "minecraft:cave_spider"
                ) && surviving.is_none_or(|ids| ids.contains(&id))
                {
                    if let Some(uuid) = m.entity.uuid {
                        self.other_team_owners.insert(id.native_id(), (id, uuid));
                    }
                }
                if (bootstrap && surviving.is_some_and(|ids| !ids.contains(&id)))
                    || !contact_capabilities::capabilities(self.session.version, name).modeled()
                    || self.boat.as_ref().is_some_and(|b| b.ignored.contains(&id))
                {
                    // Harmless and unresolved bodies keep their original slot.
                    // No received target is claimed as a current model box.
                    if surviving.is_none_or(|ids| ids.contains(&id))
                        && !self.boat.as_ref().is_some_and(|scope| {
                            scope.ignored.contains(&id)
                                || crate::client::vehicle::collision::rigid(name)
                        })
                    {
                        self.deferred.insert(
                            id.native_id(),
                            DeferredContact::new(
                                self.session.version,
                                id,
                                name,
                                ordinal,
                                m.position
                                    .as_ref()
                                    .zip(m.rotation.as_ref())
                                    .and_then(|(p, r)| {
                                        let source = crate::client::ValueSource::Received {
                                            sequence: record.receive_sequence,
                                        };
                                        (p.source == source && r.source == source).then_some(
                                            LivingInterpolationPose {
                                                position: p.value.position,
                                                rotation: r.value,
                                            },
                                        )
                                    }),
                            ),
                        );
                    }
                    self.fresh = self
                        .fresh
                        .checked_add(1)
                        .ok_or_else(|| failure("contact lifetime exhausted"))?;
                    self.world
                        .admit(tick_body(self.fresh, id.native_id(), false), None)?;
                    self.native_keys.insert(id.native_id(), self.fresh);
                    return Ok(());
                }
                let p = m
                    .position
                    .as_ref()
                    .ok_or_else(|| failure("contact spawn position missing"))?;
                let r = m
                    .rotation
                    .as_ref()
                    .ok_or_else(|| failure("contact spawn rotation missing"))?;
                for source in [p.source, r.source] {
                    if source
                        != (crate::client::ValueSource::Received {
                            sequence: record.receive_sequence,
                        })
                    {
                        return Err(failure("contact spawn receipt source"));
                    }
                }
                self.fresh = self
                    .fresh
                    .checked_add(1)
                    .ok_or_else(|| failure("contact lifetime exhausted"))?;
                let dimensions = super::geometry::shape(
                    self.session.version,
                    name,
                    EntityPose::Standing,
                    1.,
                    1.,
                );
                let body = ContactTickBody::new(
                    self.session.version,
                    (self.fresh, id.native_id()),
                    if name == "minecraft:player" {
                        ContactDispatch::RemotePlayer
                    } else if name == "minecraft:wolf" {
                        ContactDispatch::Wolf
                    } else if name == "minecraft:chicken" {
                        ContactDispatch::Chicken
                    } else {
                        ContactDispatch::InheritedLiving
                    },
                    LivingInterpolationPose {
                        position: p.value.position,
                        rotation: r.value,
                    },
                    m.velocity.as_ref().map_or([0.; 3], |v| v.value),
                    ordinal,
                    [dimensions.0, dimensions.1],
                )?;
                self.world.admit(
                    tick_body(self.fresh, id.native_id(), name == "minecraft:player"),
                    Some(body),
                )?;
                self.native_keys.insert(id.native_id(), self.fresh);
                self.inputs.insert(
                    self.fresh,
                    Inputs {
                        entity: id,
                        name: name.to_owned(),
                        uuid: m.entity.uuid,
                        alive: true,
                        pose: EntityPose::Standing,
                        spectator: false,
                        baby: 1.,
                        tame: false,
                        owner_uuid: None,
                        owner_known: true,
                    },
                );
            }
            EntityHistoryKind::Motion(m) => {
                if let Some(body) = self.deferred.get_mut(&m.entity.id.native_id()) {
                    if body.entity != m.entity.id {
                        return Err(failure("deferred contact motion lifetime changed"));
                    }
                    body.apply(record, m);
                }
                if let Some(body) = self
                    .world
                    .bodies
                    .values_mut()
                    .find(|b| b.native_id == m.entity.id.native_id())
                {
                    if self.inputs[&body.key].entity != m.entity.id {
                        return Err(failure("contact motion spawn changed"));
                    }
                    let packet = m
                        .packet
                        .as_ref()
                        .ok_or_else(|| failure("contact operation missing"))?;
                    if packet.source
                        != (crate::client::ValueSource::Received {
                            sequence: record.receive_sequence,
                        })
                    {
                        return Err(failure("contact motion provenance"));
                    }
                    body.apply(
                        self.session.version,
                        ordinal,
                        packet
                            .value
                            .living_packet(self.session.version)
                            .map_err(failure)?,
                    )?;
                    let contact = body.contact();
                    if self.session.version == crate::MinecraftVersion::Java1_16_1 {
                        self.world
                            .spatial
                            .set_bounds_in_listed_section(body.key, contact.bounds)
                            .map_err(failure)?;
                    } else {
                        super::contact_tick::index_geometry(
                            &mut self.world.spatial,
                            body.key,
                            contact.push.position,
                            contact.bounds,
                        )?;
                    }
                }
            }
            EntityHistoryKind::Removed { native_id, .. } => {
                self.deferred.remove(native_id);
                self.other_team_owners.remove(native_id);
                self.world
                    .order
                    .remove_membership(*native_id)
                    .map_err(failure)?;
                if let Some(key) = self.native_keys.remove(native_id) {
                    self.world.spatial.remove(key).map_err(failure)?;
                }
                let keys = self
                    .world
                    .bodies
                    .iter()
                    .filter(|(_, b)| b.native_id == *native_id)
                    .map(|(&k, _)| k)
                    .collect::<Vec<_>>();
                for key in keys {
                    self.world.bodies.remove(&key);
                    self.inputs.remove(&key);
                }
            }
            EntityHistoryKind::Metadata {
                native_id,
                entity,
                entries,
                complete,
            } => {
                if *native_id == own_id {
                    let EntityHistoryValue::Retained { value } = entries else {
                        return Err(failure("own contact metadata gap"));
                    };
                    if !complete {
                        return Err(failure("own contact metadata partial decode"));
                    }
                    for entry in value {
                        if bootstrap {
                            if self.own_metadata_index(entry.index) {
                                let source = super::EntityHistoryRecord {
                                    session: record.session,
                                    ordinal: record.ordinal,
                                    receive_sequence: record.receive_sequence,
                                    applied_after: record.applied_after,
                                    kind: EntityHistoryKind::Metadata {
                                        native_id: *native_id,
                                        entity: *entity,
                                        entries: EntityHistoryValue::Retained { value: Vec::new() },
                                        complete: true,
                                    },
                                };
                                own_metadata.insert(entry.index, (entry.value.clone(), source));
                            }
                        } else {
                            self.own_metadata(entry.index, &entry.value)?;
                        }
                    }
                    return Ok(());
                }
                let Some(entity) = entity else {
                    return Ok(());
                };
                if self
                    .deferred
                    .get(native_id)
                    .is_some_and(|b| b.entity == *entity)
                {
                    return Ok(());
                }
                let Some(key) = self
                    .inputs
                    .iter()
                    .find(|(_, i)| i.entity == *entity)
                    .map(|(&k, _)| k)
                else {
                    return Ok(());
                };
                let EntityHistoryValue::Retained { value } = entries else {
                    return Err(failure("contact metadata retention gap"));
                };
                if !complete {
                    return Err(failure("contact metadata partial decode"));
                }
                for entry in value {
                    self.metadata(key, entry.index, &entry.value)?;
                }
            }
            EntityHistoryKind::Scale {
                entity: Some(entity),
                attribute,
                ..
            } if self.inputs.values().any(|i| i.entity == *entity) => {
                let EntityHistoryValue::Retained { value } = attribute else {
                    return Err(failure("contact scale retention gap"));
                };
                if value.modifiers.len() > 16
                    || !value.base.is_finite()
                    || value.modifiers.iter().any(|m| !m.amount.is_finite())
                {
                    return Err(failure("contact scale unavailable"));
                }
                let modifiers = value.modifiers.iter().collect::<Vec<_>>();
                let scale = crate::client::physics::attribute_value(
                    self.session.version,
                    value.base,
                    &modifiers,
                )
                .clamp(0.0625, 16.) as f32;
                if scale != 1. {
                    return Err(unsupported("contact entity requires unit scale"));
                }
            }
            EntityHistoryKind::Equipment {
                entity: Some(entity),
                entries,
                complete,
                ..
            } if self.inputs.values().any(|i| i.entity == *entity) => {
                if !complete || matches!(entries, EntityHistoryValue::RetentionLimit) {
                    return Err(failure("contact equipment input unavailable"));
                }
            }
            EntityHistoryKind::Status {
                entity: Some(entity),
                ..
            }
            | EntityHistoryKind::Animation {
                entity: Some(entity),
                ..
            } if self.inputs.values().any(|i| i.entity == *entity) => {
                let name = &self
                    .inputs
                    .values()
                    .find(|i| i.entity == *entity)
                    .unwrap()
                    .name;
                if !contact_capabilities::harmless_packet(self.session.version, name, &record.kind)
                {
                    return Err(unsupported("unqualified contact status/animation dispatch"));
                }
            }
            EntityHistoryKind::Passengers {
                entity, passengers, ..
            } => {
                let EntityHistoryValue::Retained { value } = passengers else {
                    return Err(failure("contact passenger history unavailable"));
                };
                if let Some(scope) = &self.boat {
                    if *entity == Some(scope.entity) {
                        if !bootstrap
                            && value.iter().map(|(id, _)| *id).collect::<Vec<_>>()
                                != scope.passengers
                        {
                            return Err(failure("boat contact passenger fence changed"));
                        }
                        if value.iter().all(|(id, entity)| {
                            scope.passengers.contains(id)
                                && (self.native_keys.get(id) == Some(&1)
                                    || entity.is_some_and(|id| scope.ignored.contains(&id)))
                        }) {
                            return Ok(());
                        }
                    }
                }
                if self.inputs.values().any(|i| {
                    Some(i.entity) == *entity || value.iter().any(|(_, id)| *id == Some(i.entity))
                }) {
                    return Err(unsupported(
                        "living passenger travel requires a separate contact origin",
                    ));
                }
            }
            EntityHistoryKind::OwnPositionCorrection { .. } if !bootstrap => {
                return Err(failure("own correction discarded contact origin"));
            }
            EntityHistoryKind::WorldChanged { .. } if !bootstrap => {
                return Err(failure("world change discarded contact origin"));
            }
            _ => {}
        }
        Ok(())
    }

    fn own_metadata_index(&self, index: u8) -> bool {
        index == 6
            || self
                .session
                .version
                .table()
                .entity_data("minecraft:player")
                .iter()
                .any(|row| {
                    row.index == index
                        && [
                            EntityDataField::SleepingPosition,
                            EntityDataField::LivingFlags,
                        ]
                        .iter()
                        .any(|field| {
                            field
                                .official_names()
                                .iter()
                                .any(|&(owner, name)| row.owner == owner && row.field == name)
                        })
                })
    }

    fn own_metadata(&self, index: u8, value: &EntityDataValue) -> Result<()> {
        let table = self.session.version.table();
        let field = |field: EntityDataField| {
            table
                .entity_data("minecraft:player")
                .iter()
                .find(|r| {
                    field
                        .official_names()
                        .iter()
                        .any(|&(owner, name)| r.owner == owner && r.field == name)
                })
                .map(|r| r.index)
        };
        if index == 6 {
            let EntityDataValue::Pose(id) = value else {
                return Err(unsupported("own contact pose unavailable"));
            };
            let pose = EntityPose::from_native(self.session.version, *id);
            if !matches!(
                pose,
                Some(EntityPose::Standing | EntityPose::Crouching | EntityPose::Swimming)
            ) && !(self.own_glide && pose == Some(EntityPose::FallFlying))
            {
                return Err(unsupported(
                    "own posture outside ground/held flight contact model",
                ));
            }
        } else if Some(index) == field(EntityDataField::SleepingPosition) {
            if !matches!(value, EntityDataValue::OptionalBlockPosition(None)) {
                return Err(unsupported("own sleep requires separate movement owner"));
            }
        } else if Some(index) == field(EntityDataField::LivingFlags)
            && value.as_i64().is_none_or(|flags| flags & 4 != 0)
        {
            return Err(unsupported(
                "own spin attack requires separate movement owner",
            ));
        }
        Ok(())
    }

    fn metadata(&mut self, key: u64, index: u8, value: &EntityDataValue) -> Result<()> {
        let input = self.inputs.get_mut(&key).unwrap();
        let body = self.world.bodies.get_mut(&key).unwrap();
        let table = self.session.version.table();
        if !table
            .entity_data(&input.name)
            .iter()
            .any(|row| row.index == index)
        {
            return Err(unsupported("unclassified contact metadata accessor"));
        }
        let field = |field: EntityDataField| {
            table
                .entity_data(&input.name)
                .iter()
                .find(|r| {
                    field
                        .official_names()
                        .iter()
                        .any(|&(owner, name)| r.owner == owner && r.field == name)
                })
                .map(|r| r.index)
        };
        let species_field = |owner: &str, name: &str| {
            table
                .entity_data(&input.name)
                .iter()
                .find(|r| r.owner == owner && r.field == name)
                .map(|r| r.index)
        };
        if input.name == "minecraft:wolf"
            && Some(index) == species_field("TamableAnimal", "DATA_FLAGS_ID")
        {
            let EntityDataValue::Byte(flags) = value else {
                return Err(unsupported("unknown wolf tame/sitting flags"));
            };
            input.tame = flags & 4 != 0;
        } else if input.name == "minecraft:wolf"
            && Some(index) == species_field("Wolf", "DATA_INTERESTED_ID")
        {
            let EntityDataValue::Bool(interested) = value else {
                return Err(unsupported("unknown wolf interest"));
            };
            body.wolf_interest(*interested);
        } else if input.name == "minecraft:wolf"
            && Some(index) == species_field("TamableAnimal", "DATA_OWNERUUID_ID")
        {
            input.owner_known = false;
        } else if index == 6 {
            let EntityDataValue::Pose(id) = value else {
                return Err(unsupported("unknown contact pose serializer"));
            };
            input.pose = EntityPose::from_native(self.session.version, *id)
                .ok_or_else(|| unsupported("unknown contact pose"))?;
            let dimensions = super::geometry::shape(
                self.session.version,
                &input.name,
                input.pose,
                input.baby,
                1.,
            );
            body.refresh_dimensions(self.session.version, [dimensions.0, dimensions.1]);
        } else if Some(index) == field(EntityDataField::Health) {
            let EntityDataValue::Float(health) = value else {
                return Err(unsupported("unknown contact health"));
            };
            if !health.is_finite() {
                return Err(unsupported("nonfinite contact health"));
            }
            input.alive = *health > 0.;
            body.alive = input.alive;
        } else if Some(index) == field(EntityDataField::Baby) {
            let EntityDataValue::Bool(baby) = value else {
                return Err(unsupported("unknown contact baby data"));
            };
            input.baby = if *baby { 0.5 } else { 1. };
            let dimensions = super::geometry::shape(
                self.session.version,
                &input.name,
                input.pose,
                input.baby,
                1.,
            );
            body.refresh_dimensions(self.session.version, [dimensions.0, dimensions.1]);
        } else if Some(index) == field(EntityDataField::SleepingPosition) {
            let EntityDataValue::OptionalBlockPosition(position) = value else {
                return Err(unsupported("unknown contact sleeping position"));
            };
            body.sleeping = position.is_some();
            if let Some(position) = position {
                let mut pose = body.motion.frame().interpolation.pose;
                pose.position = [
                    f64::from(position.x) + 0.5,
                    f64::from(position.y) + 0.6875,
                    f64::from(position.z) + 0.5,
                ];
                body.set_model_pose(pose)?;
            }
        } else if Some(index) == field(EntityDataField::LivingFlags) {
            let Some(flags) = value.as_i64() else {
                return Err(unsupported("unknown contact living flags"));
            };
            if flags & 4 != 0 {
                return Err(unsupported("spin attack contact dispatch unqualified"));
            }
        }
        let contact = body.contact();
        if self.session.version == crate::MinecraftVersion::Java1_16_1 {
            self.world
                .spatial
                .set_bounds_in_listed_section(key, contact.bounds)
                .map_err(failure)?;
        } else {
            super::contact_tick::index_geometry(
                &mut self.world.spatial,
                key,
                contact.push.position,
                contact.bounds,
            )?;
        }

        Ok(())
    }

    fn apply_owner(
        &mut self,
        record: &super::EntityHistoryRecord,
        owner: Option<Option<[u8; 16]>>,
    ) {
        if let (
            EntityHistoryKind::Metadata {
                entity: Some(id), ..
            },
            Some(owner),
        ) = (&record.kind, owner)
        {
            if let Some(input) = self
                .inputs
                .values_mut()
                .find(|i| i.entity == *id && i.name == "minecraft:wolf")
            {
                input.owner_uuid = owner;
                input.owner_known = true;
            }
        }
    }

    fn wolf_team(
        &self,
        key: u64,
        teams: &TeamsObservation,
        base: &BTreeMap<u64, Option<(String, crate::client::ui::TeamCollision)>>,
    ) -> Result<Option<(String, crate::client::ui::TeamCollision)>> {
        let input = &self.inputs[&key];
        let own = base[&key].clone();
        let legacy = self.session.version == crate::MinecraftVersion::Java1_16_1;
        if !input.tame || (!legacy && own.is_some()) {
            return Ok(own);
        }
        let mut current = input;
        let mut seen = vec![key];
        loop {
            if !current.owner_known {
                return Err(entity_failure(
                    unsupported("wolf owner metadata unavailable"),
                    current,
                ));
            }
            let Some(uuid) = current.owner_uuid else {
                return Ok(if legacy { own } else { None });
            };
            if let Some((local_uuid, name)) = &self.own_identity {
                if *local_uuid == uuid {
                    return Ok(team(teams, name));
                }
            }
            let Some((&next, owner)) = self
                .inputs
                .iter()
                .find(|(_, i)| i.uuid == Some(uuid) && (!legacy || i.name == "minecraft:player"))
            else {
                if !legacy && self.other_team_owners.values().any(|(_, u)| *u == uuid) {
                    return Ok(team(teams, &uuid_holder(uuid)));
                }
                return Ok(if legacy { own } else { None });
            };
            if legacy || owner.name != "minecraft:wolf" {
                return Ok(base[&next].clone());
            }
            if seen.contains(&next) {
                return Ok(None);
            }
            seen.push(next);
            // OwnableEntity follows owner references regardless of tame flags;
            // intermediate own teams are not consulted by getRootOwner.
            current = owner;
        }
    }

    pub(crate) fn teams(
        &mut self,
        teams: &TeamsObservation,
        profiles: &PlayerListObservation,
        owner_state: MovementOwnerState,
    ) -> Result<()> {
        let result = (|| {
            if teams.session != self.session || profiles.session != self.session {
                return Err(failure("contact team/profile world"));
            }
            for (&key, input) in &mut self.inputs {
                let uuid = input.uuid.ok_or_else(|| {
                    entity_failure(failure("contact scoreboard UUID missing"), input)
                })?;
                let holder = if input.name == "minecraft:player" {
                    // Original RemotePlayer requires a registered profile. Use the
                    // actual retained UUID/name, never the configured bot name.
                    let profile = profiles
                        .entries
                        .iter()
                        .find(|p| p.uuid == uuid)
                        .ok_or_else(|| {
                            entity_failure(
                                transient("remote player contact profile missing"),
                                input,
                            )
                        })?;
                    let mode = profile
                        .game_mode
                        .as_ref()
                        .and_then(|mode| mode.value)
                        .ok_or_else(|| {
                            entity_failure(
                                transient("remote player contact game mode missing"),
                                input,
                            )
                        })?;
                    input.spectator = mode == crate::client::GameMode::Spectator;
                    profile.profile.value.name.clone()
                } else {
                    uuid_holder(uuid)
                };
                self.world.bodies.get_mut(&key).unwrap().team = team(teams, &holder);
            }
            let base = self
                .world
                .bodies
                .iter()
                .map(|(&k, b)| (k, b.team.clone()))
                .collect::<BTreeMap<_, _>>();
            for (&key, input) in &self.inputs {
                if input.name == "minecraft:wolf" {
                    self.world.bodies.get_mut(&key).unwrap().team =
                        self.wolf_team(key, teams, &base)?;
                }
            }
            Ok(())
        })();
        result.map_err(|error| self.annotate(error, owner_state))
    }

    pub(crate) fn frame(&mut self, pair_calls: usize) -> EntityContactFrame {
        self.tick += 1;
        let mut bodies = self
            .inputs
            .iter()
            .map(|(&key, input)| {
                let body = &self.world.bodies[&key];
                EntityContactSample {
                    entity: input.entity,
                    model_position: body.motion.frame().interpolation.pose.position,
                    model_velocity: body.velocity,
                }
            })
            .collect::<Vec<_>>();
        bodies.sort_by_key(|b| b.entity.native_id());
        EntityContactFrame {
            origin_after_record: self.origin,
            applied_record: self.cursor.map_or(0, EntityHistoryCursor::ordinal),
            receive_sequence: self.receive_sequence,
            model_tick: self.tick,
            pair_calls,
            bodies,
        }
    }
}

pub(crate) fn team(
    teams: &TeamsObservation,
    holder: &str,
) -> Option<(String, crate::client::ui::TeamCollision)> {
    teams
        .teams
        .iter()
        .find(|t| t.members.iter().any(|m| m.value == holder))
        .map(|t| (t.name.clone(), t.parameters.value.collision))
}
fn uuid_holder(uuid: [u8; 16]) -> String {
    let h = uuid.iter().map(|b| format!("{b:02x}")).collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}
fn tick_body(key: u64, native_id: i32, player: bool) -> LivingTickBody {
    LivingTickBody {
        key,
        native_id,
        player,
        removed: false,
        passenger_of: None,
        passengers: Vec::new(),
        legacy_ticking_chunk: Some(true),
        legacy_in_chunk: Some(true),
        player_passengers: 0,
    }
}
fn aabb(b: [f64; 6]) -> crate::client::Aabb {
    crate::client::Aabb {
        min_x: b[0],
        min_y: b[1],
        min_z: b[2],
        max_x: b[3],
        max_y: b[4],
        max_z: b[5],
    }
}
fn array(b: crate::client::Aabb) -> [f64; 6] {
    [b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z]
}
pub(crate) fn boat_query(b: [f64; 6]) -> crate::client::Aabb {
    aabb(std::array::from_fn(|i| {
        b[i] + if i < 3 {
            -1.0000001 - if i == 1 { 4. } else { 0. }
        } else {
            1.0000001 + if i == 4 { 4. } else { 0. }
        }
    }))
}
fn overlap(a: [f64; 6], b: [f64; 6]) -> bool {
    (0..3).all(|i| a[i] < b[i + 3] && a[i + 3] > b[i])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        MinecraftVersion as V,
        client::entity::{NativeMotion, NativeSpawn, NativeSpawnMotion},
    };
    fn receipts(version: V) -> (SpawnLedger, SessionStamp) {
        let session = SessionStamp {
            version,
            connection_id: 9,
            world_generation: 3,
        };
        let mut ledger = SpawnLedger::default();
        ledger.contact_local_player(99);
        ledger.history_context(version, 3, 1);
        ledger
            .insert(
                version,
                NativeSpawn {
                    id: 7,
                    uuid: Some([1; 16]),
                    type_id: None,
                    dedicated_type_name: Some("cow"),
                    position: [0.9, 65., 0.5],
                    living: Some(true),
                },
                1,
                16,
            )
            .unwrap();
        ledger.initialize_motion(
            7,
            NativeSpawnMotion {
                position: Some([0.9, 65., 0.5]),
                rotation: Some([0.; 2]),
                head_yaw: Some(0.),
                velocity: Some([0.; 3]),
            },
            1,
        );
        (ledger, session)
    }
    fn numerical_state(model: &OwnedContacts) -> serde_json::Value {
        serde_json::json!({
            "bodies": model.world.bodies.iter().map(|(key, body)|
                (key, body.motion.frame(), body.velocity, body.width, body.height)
            ).collect::<Vec<_>>(),
            "keys": model.native_keys,
            "order": format!("{:?}", model.world.order),
            "spatial": format!("{:?}", model.world.spatial),
        })
    }

    fn extra(
        ledger: &mut SpawnLedger,
        version: V,
        name: &'static str,
        position: [f64; 3],
        motion: bool,
        sequence: u64,
    ) {
        ledger.history_context(version, 3, sequence);
        ledger
            .insert(
                version,
                NativeSpawn {
                    id: 8,
                    uuid: None,
                    type_id: None,
                    dedicated_type_name: Some(name),
                    position,
                    living: None,
                },
                sequence,
                32,
            )
            .unwrap();
        ledger.initialize_motion(
            8,
            if motion {
                NativeSpawnMotion {
                    position: Some(position),
                    rotation: Some([0.; 2]),
                    ..Default::default()
                }
            } else {
                NativeSpawnMotion::default()
            },
            sequence,
        );
    }

    fn player_step(
        model: &mut OwnedContacts,
        body: &mut crate::client::physics::Body,
    ) -> Result<()> {
        model
            .tick_player(
                body,
                &crate::client::physics::Environment::defaults(model.session.version),
                crate::client::control::Controls::default(),
                false,
                None,
                &mut |_| {
                    Ok(crate::NativeBlockState {
                        name: "minecraft:air".into(),
                        properties: Default::default(),
                    })
                },
            )
            .map(|_| ())
    }
    #[test]
    fn envelope_far_allow_and_near_refusal_preserve_slots_without_fabricated_samples() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            for name in ["spider", "cave_spider"] {
                for own_id in [1, 99, 1000] {
                    let (mut ledger, session) = receipts(version);
                    ledger.remove(7);
                    extra(&mut ledger, version, name, [600., 65., 0.], true, 2);
                    let mut model = OwnedContacts::new(session, 10, own_id, &ledger, 2).unwrap();
                    let key = model.native_keys[&8];
                    // Unknown geometry/predicate capabilities stay private and
                    // unqualified; this separate proof only establishes absence.
                    assert_eq!(
                        model.deferred[&8].capabilities.geometry,
                        contact_capabilities::Classification::Unknown
                    );
                    let high = crate::client::physics::Body::new([600., 70., 0.]);
                    contact_capabilities::check(model.deferred.values(), aabb(high.bounds), false)
                        .unwrap();
                    assert!(
                        contact_capabilities::check(
                            model.deferred.values(),
                            boat_query(high.bounds),
                            true
                        )
                        .is_err()
                    );
                    let mut own = crate::client::physics::Body::new([0., 65., 0.]);
                    for _ in 0..8 {
                        player_step(&mut model, &mut own).unwrap();
                    }
                    assert!(model.world.order.body(key).is_some());
                    assert!(model.frame(0).bodies.is_empty());
                    contact_capabilities::check(
                        model.deferred.values(),
                        boat_query(own.bounds),
                        true,
                    )
                    .unwrap();
                    let error = player_step(
                        &mut model,
                        &mut crate::client::physics::Body::new([600., 65., 0.]),
                    )
                    .unwrap_err();
                    assert!(matches!(
                        error.movement_refusal(),
                        Some(MovementRefusal::Unsupported { .. })
                    ));
                    assert_eq!(
                        error.movement_refusal().unwrap().context().native_entity_id,
                        Some(8)
                    );
                }
            }
        }
    }

    #[test]
    fn envelope_retains_interpolation_path_and_old_anchors_after_far_teleport() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            let (mut ledger, session) = receipts(version);
            ledger.remove(7);
            extra(&mut ledger, version, "spider", [-100., 65., 0.], true, 2);
            let mut model = OwnedContacts::new(session, 10, 99, &ledger, 2).unwrap();
            player_step(
                &mut model,
                &mut crate::client::physics::Body::new([0., 65., 0.]),
            )
            .unwrap();
            ledger.history_context(version, 3, 3);
            let packet = NativeMotion::Absolute {
                position: [100., 65., 0.],
                rotation: [0.; 2],
                velocity: (version == V::Java1_21_11).then_some([0.; 3]),
                ground: Some(false),
                reset_base: true,
            };
            ledger.receive_motion(version, 8, packet, 3);
            model.refresh(10, &ledger, 3).unwrap();
            // Both endpoints are far; the complete path crosses the player.
            assert!(
                player_step(
                    &mut model.clone(),
                    &mut crate::client::physics::Body::new([0., 65., 0.])
                )
                .is_err()
            );
            assert!(
                player_step(
                    &mut model.clone(),
                    &mut crate::client::physics::Body::new([-100., 65., 0.])
                )
                .is_err()
            );
            assert!(
                player_step(
                    &mut model,
                    &mut crate::client::physics::Body::new([0., 65., 100.])
                )
                .is_ok()
            );
        }
    }

    #[test]
    fn envelope_checks_attempted_sweep_even_when_wall_clips_the_result() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            let (mut ledger, session) = receipts(version);
            ledger.remove(7);
            extra(&mut ledger, version, "spider", [8., 65., 0.], true, 2);
            let model = OwnedContacts::new(session, 10, 99, &ledger, 2).unwrap();
            let before = numerical_state(&model);
            let mut own = crate::client::physics::Body::new([0., 65., 0.]);
            own.velocity = [10., 0., 0.];
            let error = model
                .clone()
                .tick_player(
                    &mut own,
                    &crate::client::physics::Environment::defaults(version),
                    crate::client::control::Controls::default(),
                    false,
                    None,
                    &mut |p| {
                        Ok(crate::NativeBlockState {
                            name: if p[0] == 2 {
                                "minecraft:stone"
                            } else {
                                "minecraft:air"
                            }
                            .into(),
                            properties: Default::default(),
                        })
                    },
                )
                .unwrap_err();
            assert!(matches!(
                error.movement_refusal(),
                Some(MovementRefusal::Unsupported { .. })
            ));
            // The retained model is never replaced by this failed staged clone.
            assert_eq!(numerical_state(&model), before);
        }
    }

    #[test]
    fn envelope_refuses_indirect_remote_overlap_and_shifted_interpolation_target() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            let (mut ledger, session) = receipts(version);
            extra(&mut ledger, version, "spider", [600., 65., 0.], true, 2);
            let mut model = OwnedContacts::new(session, 10, 99, &ledger, 2).unwrap();
            player_step(
                &mut model.clone(),
                &mut crate::client::physics::Body::new([0., 65., 0.]),
            )
            .unwrap();
            let remote = model.world.bodies.values_mut().next().unwrap();
            let mut pose = remote.motion.frame().interpolation.pose;
            pose.position = [600., 65., 0.];
            remote.set_model_pose(pose).unwrap();
            let error = player_step(
                &mut model.clone(),
                &mut crate::client::physics::Body::new([0., 65., 0.]),
            )
            .unwrap_err();
            assert_eq!(
                error.movement_refusal().unwrap().context().native_entity_id,
                Some(8)
            );
            if version == V::Java1_21_11 {
                let remote = model.world.bodies.values_mut().next().unwrap();
                pose.position = [0., 65., 0.];
                remote.motion = super::super::LivingPacketModel::new(version, pose, 1).unwrap();
                remote
                    .motion
                    .apply(
                        100,
                        super::super::LivingMotionPacket::Relative {
                            delta: [32767, 0, 0],
                            rotation: None,
                            ground: false,
                        },
                        super::super::LivingPacketGuards {
                            local_authoritative: false,
                            ticking: true,
                        },
                    )
                    .unwrap();
                // Sleeping-position metadata can separately displace model feet.
                pose.position = [450., 65., 0.];
                remote.set_model_pose(pose).unwrap();
                // target~=8, previous=0, current=450 -> shifted target~=458.
                // This body is outside the ordinary [8,450] interpolation hull.
                extra(&mut ledger, version, "spider", [456., 65., 0.], true, 3);
                model.refresh(10, &ledger, 3).unwrap();
                assert!(
                    player_step(
                        &mut model,
                        &mut crate::client::physics::Body::new([0., 65., 0.])
                    )
                    .is_err()
                );
            }
        }
    }

    #[test]
    fn envelope_unproven_callbacks_revoke_locality_with_typed_packet_context() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            for kind in [
                "Metadata",
                "Status",
                "Animation",
                "Passengers",
                "Equipment",
                "Scale",
            ] {
                let (mut ledger, session) = receipts(version);
                ledger.remove(7);
                extra(&mut ledger, version, "spider", [600., 65., 0.], true, 2);
                let mut model = OwnedContacts::new(session, 10, 99, &ledger, 2).unwrap();
                player_step(
                    &mut model,
                    &mut crate::client::physics::Body::new([0., 65., 0.]),
                )
                .unwrap();
                ledger.history_context(version, 3, 3);
                let entity = Some(model.deferred[&8].entity);
                let opcode = match kind {
                    "Status" => 127,
                    "Animation" => 255,
                    _ => 0,
                };
                let record_kind = match kind {
                    "Metadata" => EntityHistoryKind::Metadata {
                        native_id: 8,
                        entity,
                        entries: EntityHistoryValue::Retained { value: vec![] },
                        complete: true,
                    },
                    "Status" => EntityHistoryKind::Status {
                        native_id: 8,
                        entity,
                        status: 127,
                    },
                    "Animation" => EntityHistoryKind::Animation {
                        native_id: 8,
                        entity,
                        animation: 255,
                    },
                    "Passengers" => EntityHistoryKind::Passengers {
                        native_id: 99,
                        entity: None,
                        passengers: EntityHistoryValue::Retained {
                            value: vec![(8, entity)],
                        },
                    },
                    "Equipment" => EntityHistoryKind::Equipment {
                        native_id: 8,
                        entity,
                        entries: EntityHistoryValue::Retained { value: vec![] },
                        complete: true,
                    },
                    "Scale" => EntityHistoryKind::Scale {
                        native_id: 8,
                        entity,
                        attribute: EntityHistoryValue::RetentionLimit,
                    },
                    _ => unreachable!(),
                };
                let record = super::super::EntityHistoryRecord {
                    session,
                    ordinal: 100,
                    receive_sequence: 3,
                    applied_after: std::time::Duration::ZERO,
                    kind: record_kind,
                };
                model
                    .reduce_record(&record, false, None, &mut BTreeMap::new())
                    .unwrap();
                let error = player_step(
                    &mut model,
                    &mut crate::client::physics::Body::new([0., 65., 0.]),
                )
                .unwrap_err();
                let detail = error.movement_refusal().unwrap();
                assert!(matches!(detail, MovementRefusal::Unsupported { .. }));
                let c = detail.context();
                assert_eq!(c.entity, entity);
                assert_eq!(c.entity_type.as_deref(), Some("minecraft:spider"));
                assert_eq!(c.record_ordinal, Some(100));
                assert_eq!(c.receive_sequence, Some(3));
                assert_eq!(c.packet.as_ref().unwrap().packet_type, kind);
                assert_eq!(c.packet.as_ref().unwrap().opcode, opcode);
            }
        }
    }

    #[test]
    fn envelope_revocation_survives_checkpoint_and_unsupported_motion_stays_global() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            for signal in [true, false] {
                let (mut ledger, session) = receipts(version);
                ledger.remove(7);
                extra(&mut ledger, version, "spider", [600., 65., 0.], true, 2);
                let mut model = OwnedContacts::new(session, 10, 99, &ledger, 2).unwrap();
                ledger.history_context(version, 3, 3);
                if signal {
                    ledger.history_signal(8, Some(127), None);
                } else {
                    ledger.receive_motion(
                        version,
                        8,
                        NativeMotion::Absolute {
                            position: [600., 65., 0.],
                            rotation: [0.; 2],
                            velocity: None,
                            ground: None,
                            reset_base: false,
                        },
                        3,
                    );
                }
                model.refresh(10, &ledger, 3).unwrap();
                let expected = if signal { "Status" } else { "Motion" };
                let error = player_step(
                    &mut model,
                    &mut crate::client::physics::Body::new([0., 65., 0.]),
                )
                .unwrap_err();
                assert_eq!(
                    error
                        .movement_refusal()
                        .unwrap()
                        .context()
                        .packet
                        .as_ref()
                        .unwrap()
                        .packet_type,
                    expected
                );
                for sequence in 4..MAX_ENTITY_HISTORY_RECORDS as u64 + 8 {
                    ledger.history_context(version, 3, sequence);
                    ledger.history_signal(500, None, Some(0));
                }
                let mut fresh = OwnedContacts::new(
                    session,
                    11,
                    99,
                    &ledger,
                    MAX_ENTITY_HISTORY_RECORDS as u64 + 8,
                )
                .unwrap();
                let error = player_step(
                    &mut fresh,
                    &mut crate::client::physics::Body::new([0., 65., 0.]),
                )
                .unwrap_err();
                let detail = error.movement_refusal().unwrap();
                assert!(matches!(detail, MovementRefusal::Unsupported { .. }));
                assert_eq!(
                    detail.context().packet.as_ref().unwrap().packet_type,
                    expected
                );
                assert_eq!(detail.context().receive_sequence, Some(3));
            }
        }
    }

    #[test]
    fn envelope_includes_world_boundary_clamped_placement_and_refuses_out_of_domain() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            for sign in [-1., 1.] {
                let (mut ledger, session) = receipts(version);
                ledger.remove(7);
                extra(
                    &mut ledger,
                    version,
                    "spider",
                    [sign * 31_000_000., 65., 0.],
                    true,
                    2,
                );
                let mut model = OwnedContacts::new(session, 10, 99, &ledger, 2).unwrap();
                for invalid_x in [f64::NAN, f64::INFINITY, 10.] {
                    let mut query = aabb(crate::client::physics::Body::new([0., 65., 0.]).bounds);
                    query.min_x = invalid_x; // 10 also makes min > max.
                    let error = contact_capabilities::check(model.deferred.values(), query, false)
                        .unwrap_err();
                    assert!(matches!(
                        error.movement_refusal(),
                        Some(MovementRefusal::Unsupported { .. })
                    ));
                }

                player_step(
                    &mut model,
                    &mut crate::client::physics::Body::new([0., 65., 0.]),
                )
                .unwrap();
                let error = player_step(
                    &mut model,
                    &mut crate::client::physics::Body::new([sign * 30_000_000., 65., 0.]),
                )
                .unwrap_err();
                assert!(matches!(
                    error.movement_refusal(),
                    Some(MovementRefusal::Unsupported { .. })
                ));
                extra(
                    &mut ledger,
                    version,
                    "spider",
                    [sign * 32_000_001., 65., 0.],
                    true,
                    3,
                );
                model.refresh(10, &ledger, 3).unwrap();
                assert!(
                    player_step(
                        &mut model,
                        &mut crate::client::physics::Body::new([0., 65., 0.])
                    )
                    .is_err()
                );
            }
        }
    }

    #[test]
    fn envelope_replays_checkpoint_and_rechecks_removal_id_reuse_and_world() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            let (mut ledger, session) = receipts(version);
            ledger.remove(7);
            extra(&mut ledger, version, "spider", [600., 65., 0.], true, 2);
            let mut model = OwnedContacts::new(session, 10, 99, &ledger, 2).unwrap();
            let old = model.deferred[&8].entity;
            for sequence in 3..MAX_ENTITY_HISTORY_RECORDS as u64 + 8 {
                ledger.history_context(version, 3, sequence);
                ledger.history_signal(500, None, Some(0));
                if sequence % 64 == 0 {
                    model.refresh(10, &ledger, sequence).unwrap();
                }
            }
            let end = MAX_ENTITY_HISTORY_RECORDS as u64 + 8;
            let mut fresh = OwnedContacts::new(session, 11, 99, &ledger, end).unwrap();
            player_step(
                &mut fresh,
                &mut crate::client::physics::Body::new([0., 65., 0.]),
            )
            .unwrap();
            // Live owners and new owners use the same checkpoint/suffix reducer.
            model.refresh(10, &ledger, end).unwrap();
            player_step(
                &mut model,
                &mut crate::client::physics::Body::new([0., 65., 0.]),
            )
            .unwrap();
            ledger.remove(8);
            model.refresh(10, &ledger, end).unwrap();
            assert!(model.deferred.is_empty());
            extra(&mut ledger, version, "spider", [0., 65., 0.], true, end + 1);
            model.refresh(10, &ledger, end + 1).unwrap();
            assert_ne!(model.deferred[&8].entity, old);
            assert!(
                player_step(
                    &mut model,
                    &mut crate::client::physics::Body::new([0., 65., 0.])
                )
                .is_err()
            );
            let mut wrong = session;
            wrong.world_generation += 1;
            assert!(OwnedContacts::new(wrong, 12, 99, &ledger, end + 1).is_err());
        }
    }

    #[test]
    fn harmless_overlap_keeps_native_slots_without_motion_metadata_profiles_or_teams() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            for name in ["bat", "armor_stand"] {
                let (mut ledger, session) = receipts(version);
                ledger.remove(7);
                extra(&mut ledger, version, name, [0., 65., 0.], false, 2);
                // Even incomplete metadata has no living/boat contact relevance.
                ledger.receive_metadata(8, [], false, 3);
                let mut model = OwnedContacts::new(session, 10, 99, &ledger, 3).unwrap();
                assert!(!model.has_bodies(), "{version:?} {name}");
                assert!(model.world.order.body(model.native_keys[&8]).is_some());
                let mut own = crate::client::physics::Body::new([0., 65., 0.]);
                player_step(&mut model, &mut own).unwrap();
                assert_eq!(own.velocity[0], 0.);
                assert_eq!(own.velocity[2], 0.);
                assert!(model.frame(0).bodies.is_empty());
                contact_capabilities::check(model.deferred.values(), boat_query(own.bounds), true)
                    .unwrap();
            }
        }
    }
    #[test]
    fn unproven_unknown_and_bounded_unknown_refuse_with_spawn_lifetime_context() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            for name in [
                "horse",
                "slime",
                "shulker",
                "firework_rocket",
                "item",
                "arrow",
                "cat",
                "llama",
                "trader_llama",
                "fox",
                "iron_golem",
                "donkey",
                "mule",
                "skeleton_horse",
                "zombie_horse",
                "panda",
                "polar_bear",
                "dolphin",
                "turtle",
                "bee",
                "endermite",
                "enderman",
                "witch",
                "vindicator",
                "pillager",
                "evoker",
                "ravager",
                "drowned",
                "husk",
                "zombified_piglin",
                "zombie_villager",
                "snow_golem",
                "skeleton",
                "stray",
                "wither_skeleton",
            ] {
                for position in [[0., 65., 0.], [600., 65., 0.]] {
                    let (mut ledger, session) = receipts(version);
                    ledger.remove(7);
                    extra(&mut ledger, version, name, position, true, 2);
                    let mut model = OwnedContacts::new(session, 10, 99, &ledger, 2).unwrap();
                    assert!(model.world.bodies.is_empty());
                    let own = crate::client::physics::Body::new([0., 65., 0.]);
                    let before = numerical_state(&model);
                    let error = player_step(&mut model.clone(), &mut own.clone()).unwrap_err();
                    assert_eq!(before, numerical_state(&model));
                    let detail = error.movement_refusal().unwrap();
                    assert!(matches!(detail, MovementRefusal::Unsupported { .. }));
                    let context = detail.context();
                    assert_eq!(context.entity, Some(model.deferred[&8].entity));
                    assert_eq!(context.native_entity_id, Some(8));
                    assert_eq!(
                        context.entity_type.as_deref(),
                        Some(format!("minecraft:{name}").as_str())
                    );
                    assert_eq!(context.session, Some(session));
                    assert_eq!(context.owner_id, Some(10));
                    assert_eq!(context.packet.as_ref().unwrap().packet_type, "Spawn");
                    assert_eq!(context.receive_sequence, Some(2));
                    assert!(context.record_ordinal.is_some());
                    let boat_error = contact_capabilities::check(
                        model.deferred.values(),
                        boat_query(own.bounds),
                        true,
                    )
                    .unwrap_err();
                    assert!(matches!(
                        boat_error.movement_refusal(),
                        Some(MovementRefusal::Unsupported { .. })
                    ));
                    // A distant target or a new lifetime never qualifies the species.
                    ledger.history_context(version, 3, 3);
                    ledger.receive_motion(
                        version,
                        8,
                        NativeMotion::Relative {
                            delta: [24576, 0, 0],
                            rotation: None,
                            ground: false,
                        },
                        3,
                    );
                    model.refresh(10, &ledger, 3).unwrap();
                    assert!(player_step(&mut model.clone(), &mut own.clone()).is_err());
                    let old = model.deferred[&8].entity;
                    ledger.remove(8);
                    extra(&mut ledger, version, name, [600., 65., 0.], true, 4);
                    model.refresh(10, &ledger, 4).unwrap();
                    assert_ne!(model.deferred[&8].entity, old);
                    let error = player_step(&mut model.clone(), &mut own.clone()).unwrap_err();
                    assert_eq!(
                        error.movement_refusal().unwrap().context().receive_sequence,
                        Some(4)
                    );
                    // Reuse as a proven exclusion admits independently of the old refusal.
                    ledger.remove(8);
                    extra(&mut ledger, version, "bat", [0., 65., 0.], false, 5);
                    model.refresh(10, &ledger, 5).unwrap();
                    player_step(&mut model, &mut own.clone()).unwrap();
                }
            }
        }
    }

    #[test]
    fn unknown_geometry_metadata_and_passengers_never_authorize_deferral() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            for (name, motion) in [
                ("spider", false),
                ("horse", false),
                ("slime", true),
                ("shulker", true),
                ("boat", true),
            ] {
                let (mut ledger, session) = receipts(version);
                ledger.remove(7);
                extra(
                    &mut ledger,
                    version,
                    if name == "boat" && version == V::Java1_21_11 {
                        "oak_boat"
                    } else {
                        name
                    },
                    [600., 65., 0.],
                    motion,
                    2,
                );
                let mut model = OwnedContacts::new(session, 10, 99, &ledger, 2).unwrap();
                assert!(
                    player_step(
                        &mut model,
                        &mut crate::client::physics::Body::new([0., 65., 0.])
                    )
                    .is_err(),
                    "{version:?} {name}"
                );
            }
            for complete in [false, true] {
                let (mut ledger, session) = receipts(version);
                ledger.remove(7);
                extra(&mut ledger, version, "horse", [6., 65., 0.], true, 2);
                let mut model = OwnedContacts::new(session, 10, 99, &ledger, 2).unwrap();
                ledger.history_context(version, 3, 3);
                ledger.receive_metadata(8, [(6, EntityDataValue::Int(99))], complete, 3);
                model.refresh(10, &ledger, 3).unwrap();
                assert!(
                    player_step(
                        &mut model,
                        &mut crate::client::physics::Body::new([0., 65., 0.])
                    )
                    .is_err()
                );
            }
            let (mut ledger, session) = receipts(version);
            ledger.remove(7);
            extra(&mut ledger, version, "horse", [6., 65., 0.], true, 2);
            let mut model = OwnedContacts::new(session, 10, 99, &ledger, 2).unwrap();
            ledger.history_context(version, 3, 3);
            ledger.history_passengers(&crate::client::vehicle::NativePassengers {
                vehicle: 8,
                passengers: vec![99],
            });
            model.refresh(10, &ledger, 3).unwrap();
            assert!(
                player_step(
                    &mut model,
                    &mut crate::client::physics::Body::new([0., 65., 0.])
                )
                .is_err()
            );
        }
    }
    #[test]
    fn unknown_contacts_remain_unsupported_across_checkpoint_and_world_lifetime() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            let (mut ledger, session) = receipts(version);
            ledger.remove(7);
            extra(&mut ledger, version, "horse", [6., 65., 0.], true, 2);
            let model = OwnedContacts::new(session, 10, 99, &ledger, 2).unwrap();
            let mut own = crate::client::physics::Body::new([0., 65., 0.]);
            own.velocity = [8., 0., 0.];
            assert!(player_step(&mut model.clone(), &mut own).is_err());
            for sequence in 3..MAX_ENTITY_HISTORY_RECORDS as u64 + 8 {
                ledger.history_context(version, 3, sequence);
                ledger.history_signal(500, None, Some(0));
            }
            let fresh = OwnedContacts::new(
                session,
                11,
                99,
                &ledger,
                MAX_ENTITY_HISTORY_RECORDS as u64 + 8,
            )
            .unwrap();
            assert_eq!(fresh.deferred[&8].entity, model.deferred[&8].entity);
            let error = player_step(
                &mut fresh.clone(),
                &mut crate::client::physics::Body::new([0., 65., 0.]),
            )
            .unwrap_err();
            assert!(matches!(
                error.movement_refusal(),
                Some(MovementRefusal::Unsupported { .. })
            ));
            let mut wrong = session;
            wrong.world_generation += 1;
            assert!(
                OwnedContacts::new(
                    wrong,
                    12,
                    99,
                    &ledger,
                    MAX_ENTITY_HISTORY_RECORDS as u64 + 8
                )
                .is_err()
            );
        }
    }

    #[test]
    fn unclassified_metadata_accessor_is_typed_unsupported_at_the_packet() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            let (mut ledger, session) = receipts(version);
            let mut model = OwnedContacts::new(session, 10, 99, &ledger, 1).unwrap();
            ledger.history_context(version, 3, 2);
            ledger.receive_metadata(7, [(254, EntityDataValue::Bool(true))], true, 2);
            for error in [
                model.refresh(10, &ledger, 2).unwrap_err(),
                OwnedContacts::new(session, 11, 99, &ledger, 2)
                    .err()
                    .unwrap(),
            ] {
                let detail = error.movement_refusal().unwrap();
                assert!(matches!(detail, MovementRefusal::Unsupported { .. }));
                let c = detail.context();
                assert_eq!(c.native_entity_id, Some(7));
                assert_eq!(c.entity_type.as_deref(), Some("minecraft:cow"));
                assert_eq!(c.entity.unwrap().spawn_sequence(), 1);
                assert_eq!(c.packet.as_ref().unwrap().packet_type, "Metadata");
                assert_eq!(c.receive_sequence, Some(2));
            }
        }
    }

    #[test]
    fn audited_signals_preserve_contacts_and_history_at_every_replay_boundary() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            let (mut ledger, session) = receipts(version);
            // Use the same original spawn and numeric receipt state, changing only species.
            // Reinsert to establish a real sheep lifetime.
            ledger.remove(7);
            ledger.history_context(version, 3, 2);
            ledger
                .insert(
                    version,
                    NativeSpawn {
                        id: 7,
                        uuid: Some([2; 16]),
                        type_id: None,
                        dedicated_type_name: Some("sheep"),
                        position: [0.9, 65., 0.5],
                        living: Some(true),
                    },
                    2,
                    16,
                )
                .unwrap();
            ledger.initialize_motion(
                7,
                NativeSpawnMotion {
                    position: Some([0.9, 65., 0.5]),
                    rotation: Some([0.; 2]),
                    head_yaw: Some(0.),
                    velocity: Some([0.; 3]),
                },
                2,
            );
            let mut model = OwnedContacts::new(session, 10, 99, &ledger, 2).unwrap();
            let baseline = numerical_state(&model);
            for (sequence, status, animation) in [
                (3, Some(10), None),
                (4, Some(35), None),
                (5, None, Some(4)),
                (6, None, Some(5)),
            ] {
                ledger.history_context(version, 3, sequence);
                ledger.history_signal(7, status, animation);
                model.refresh(10, &ledger, sequence).unwrap();
                assert_eq!(numerical_state(&model), baseline);
                let fresh = OwnedContacts::new(session, 11, 99, &ledger, sequence).unwrap();
                assert_eq!(numerical_state(&fresh), baseline);
            }
            let history = ledger
                .history_after(session, 6, None, MAX_ENTITY_HISTORY_READ)
                .unwrap();
            assert_eq!(
                history
                    .records
                    .iter()
                    .filter(|r| matches!(
                        r.kind,
                        EntityHistoryKind::Status { .. } | EntityHistoryKind::Animation { .. }
                    ))
                    .count(),
                4
            );
            // Exercise the exact eviction checkpoint reducer, not just the retained suffix.
            let mut checkpoint = ContactCheckpoint::new(session, 99).unwrap();
            for record in history.records {
                checkpoint.advance(record, None);
            }
            assert!(checkpoint.errors.is_empty());
            assert!(checkpoint.global_error.is_none());
            assert_eq!(numerical_state(&checkpoint.model), baseline);
        }
    }

    #[test]
    fn unaudited_signals_are_typed_and_survive_checkpoint_replay() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            // Includes death, wake-up, unknown values, and sheep-only status on a cow.
            for (status, animation, kind, opcode) in [
                (Some(3), None, "Status", 3),
                (Some(10), None, "Status", 10),
                (Some(127), None, "Status", 127),
                (None, Some(2), "Animation", 2),
                (None, Some(255), "Animation", 255),
            ] {
                let (mut ledger, session) = receipts(version);
                let mut model = OwnedContacts::new(session, 10, 99, &ledger, 1).unwrap();
                let entity = model.inputs.values().next().unwrap().entity;
                ledger.history_context(version, 3, 2);
                ledger.history_signal(7, status, animation);
                let error = model.refresh(10, &ledger, 2).unwrap_err();
                let detail = error.movement_refusal().unwrap();
                assert!(matches!(detail, MovementRefusal::Unsupported { .. }));
                let c = detail.context();
                assert_eq!(c.owner_state, MovementOwnerState::Invalidated);
                assert_eq!(c.owner_id, Some(10));
                assert_eq!(c.entity, Some(entity));
                assert_eq!(c.native_entity_id, Some(7));
                assert_eq!(c.entity_type.as_deref(), Some("minecraft:cow"));
                assert_eq!(
                    c.packet,
                    Some(MovementPacketTrigger {
                        packet_type: kind.into(),
                        opcode
                    })
                );
                assert_eq!(c.receive_sequence, Some(2));
                assert!(!model.valid);
                let admission = OwnedContacts::new(session, 11, 99, &ledger, 2).unwrap_err();
                assert_eq!(
                    admission.movement_refusal().unwrap().context().owner_state,
                    MovementOwnerState::NotAdmitted
                );
                let mut checkpoint = ContactCheckpoint::new(session, 99).unwrap();
                for record in ledger
                    .history_after(session, 2, None, MAX_ENTITY_HISTORY_READ)
                    .unwrap()
                    .records
                {
                    checkpoint.advance(record, None);
                }
                let saved = &checkpoint.errors[&7].1;
                assert_eq!(saved.context().packet, c.packet);
                assert_eq!(saved.context().entity, Some(entity));
                let mut target = OwnedContacts::empty(session, 99).unwrap();
                checkpoint
                    .seed(&mut target, &[entity], &mut BTreeMap::new())
                    .unwrap();
                assert_eq!(target.checkpoint_errors[0].1.context().packet, c.packet);
            }
        }
    }

    #[test]
    fn refusal_classes_do_not_depend_on_reason_text() {
        assert!(matches!(
            failure("unsupported packet timed out").movement_refusal(),
            Some(MovementRefusal::Permanent { .. })
        ));
        assert!(matches!(
            transient("missing profile").movement_refusal(),
            Some(MovementRefusal::Transient { .. })
        ));
        assert!(matches!(
            unsupported("unknown hook").movement_refusal(),
            Some(MovementRefusal::Unsupported { .. })
        ));
    }

    #[test]
    fn sheep_and_creeper_admission_and_sheep_age_receipts() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            for name in ["sheep", "creeper"] {
                let (mut ledger, session) = receipts(version);
                ledger.remove(7);
                ledger.history_context(version, 3, 2);
                ledger
                    .insert(
                        version,
                        NativeSpawn {
                            id: 7,
                            uuid: Some([2; 16]),
                            type_id: None,
                            dedicated_type_name: Some(name),
                            position: [0.9, 65., 0.5],
                            living: Some(true),
                        },
                        2,
                        16,
                    )
                    .unwrap();
                ledger.initialize_motion(
                    7,
                    NativeSpawnMotion {
                        position: Some([0.9, 65., 0.5]),
                        rotation: Some([0.; 2]),
                        head_yaw: Some(0.),
                        velocity: Some([0.; 3]),
                    },
                    2,
                );
                let mut model = OwnedContacts::new(session, 10, 99, &ledger, 2).unwrap();
                let body = model.world.bodies.values().next().unwrap();
                assert_eq!(body.dispatch, ContactDispatch::InheritedLiving);
                let adult = (body.width, body.height);
                if name == "sheep" {
                    assert_eq!(adult, (0.9, 1.3));
                    let index = version
                        .table()
                        .entity_data("minecraft:sheep")
                        .iter()
                        .find(|r| {
                            EntityDataField::Baby
                                .official_names()
                                .iter()
                                .any(|&(owner, field)| r.owner == owner && r.field == field)
                        })
                        .unwrap()
                        .index;
                    for (sequence, baby) in [(3, true), (4, false)] {
                        ledger.history_context(version, 3, sequence);
                        ledger.receive_metadata(
                            7,
                            [(index, EntityDataValue::Bool(baby))],
                            true,
                            sequence,
                        );
                        model.refresh(10, &ledger, sequence).unwrap();
                        let body = model.world.bodies.values().next().unwrap();
                        let factor = if baby { 0.5 } else { 1. };
                        assert_eq!(
                            (body.width, body.height),
                            (adult.0 * factor, adult.1 * factor)
                        );
                    }
                } else {
                    assert_eq!(adult, (0.6, 1.7));
                }
            }
        }
    }

    #[test]
    fn original_wolf_owner_team_precedence_and_cycles() {
        let fixture: serde_json::Value = serde_json::from_reader(flate2::read::GzDecoder::new(
            &include_bytes!("../../../data/client_api/entity_owner_oracle.json.gz")[..],
        ))
        .unwrap();
        let uuid = |s: &str| {
            u128::from_str_radix(&s.replace('-', ""), 16)
                .unwrap()
                .to_be_bytes()
        };
        for (version, name) in [(V::Java1_16_1, "1.16.1"), (V::Java1_21_11, "1.21.11")] {
            let rows = fixture["versions"][name]["wolf_teams"].as_array().unwrap();
            assert_eq!(rows.len(), 12);
            for row in rows {
                let scenario = row["scenario"].as_u64().unwrap();
                let (mut ledger, session) = receipts(version);
                ledger.remove(7);
                for (id, kind, entity_uuid) in [
                    (8, "wolf", 10_u128.to_be_bytes()),
                    (9, "wolf", 11_u128.to_be_bytes()),
                    (501, "player", ((1_u128 << 64) + 501).to_be_bytes()),
                ] {
                    ledger.history_context(version, 3, id as u64);
                    ledger
                        .insert(
                            version,
                            NativeSpawn {
                                id,
                                uuid: Some(entity_uuid),
                                type_id: None,
                                dedicated_type_name: Some(kind),
                                position: [0., 65., 0.],
                                living: Some(true),
                            },
                            id as u64,
                            32,
                        )
                        .unwrap();
                    ledger.initialize_motion(
                        id,
                        NativeSpawnMotion {
                            position: Some([0., 65., 0.]),
                            rotation: Some([0.; 2]),
                            ..Default::default()
                        },
                        id as u64,
                    );
                }
                let mut model = OwnedContacts::new(session, 10, 99, &ledger, 501).unwrap();
                let a = model.native_keys[&8];
                let b = model.native_keys[&9];
                let p = model.native_keys[&501];
                for (key, field, tame) in [
                    (a, "owner", row["tame"].as_bool().unwrap()),
                    (b, "parent_owner", row["parent_tame"].as_bool().unwrap()),
                ] {
                    let input = model.inputs.get_mut(&key).unwrap();
                    input.tame = tame;
                    input.owner_uuid = row[field].as_str().map(uuid);
                }
                let value = |prefix: &str| {
                    Some((
                        format!("{prefix}Wolf{scenario}"),
                        crate::client::ui::TeamCollision::Always,
                    ))
                };
                let base = BTreeMap::from([
                    (
                        a,
                        if row["own_team"].as_bool().unwrap() {
                            value("A")
                        } else {
                            None
                        },
                    ),
                    (b, value("B")),
                    (p, if scenario == 5 { None } else { value("P") }),
                ]);
                let teams = TeamsObservation {
                    session,
                    receive_sequence: 501,
                    context_reset_sequence: None,
                    last_update_sequence: None,
                    teams: vec![],
                };
                let actual = model.wolf_team(a, &teams, &base).unwrap().map(|t| t.0);
                assert_eq!(
                    actual.as_deref(),
                    row["result"].as_str(),
                    "{name} {scenario}"
                );
            }
        }
    }

    #[test]
    fn wolf_private_owner_receipts_survive_checkpoint_and_replacement() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            let (mut ledger, session) = receipts(version);
            ledger.remove(7);
            extra(&mut ledger, version, "wolf", [0., 65., 0.], true, 2);
            let flags = if version == V::Java1_16_1 { 16 } else { 17 };
            let owner = flags + 1;
            let mut model = OwnedContacts::new(session, 10, 99, &ledger, 2).unwrap();
            for (sequence, reference) in [(3, Some([2; 16])), (4, None), (5, Some([3; 16]))] {
                ledger.history_context(version, 3, sequence);
                ledger.contact_owner_receipt(reference);
                ledger.receive_metadata(
                    8,
                    [
                        (flags, EntityDataValue::Byte(5)),
                        (
                            owner,
                            EntityDataValue::Other(if version == V::Java1_16_1 { 12 } else { 13 }),
                        ),
                    ],
                    true,
                    sequence,
                );
                model.refresh(10, &ledger, sequence).unwrap();
                let input = model.inputs.values().next().unwrap();
                assert!(input.tame && input.owner_known);
                assert_eq!(input.owner_uuid, reference);
            }
            // Force public suffix eviction; the private reducer consumes each
            // owner sidecar with its original record before releasing it.
            for sequence in 6..(MAX_ENTITY_HISTORY_RECORDS as u64 + 12) {
                ledger.history_context(version, 3, sequence);
                ledger.receive_metadata(8, [], true, sequence);
            }
            let fresh = OwnedContacts::new(
                session,
                11,
                99,
                &ledger,
                MAX_ENTITY_HISTORY_RECORDS as u64 + 11,
            )
            .unwrap();
            let input = fresh.inputs.values().next().unwrap();
            assert!(input.tame && input.owner_known);
            assert_eq!(input.owner_uuid, Some([3; 16]));
            assert!(ledger.1.contact_owners.is_empty());
        }
    }

    #[test]
    fn ordinary_mob_admission_and_age_receipts() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            for name in [
                "wolf",
                "chicken",
                "rabbit",
                "ocelot",
                "villager",
                "wandering_trader",
                "mooshroom",
                "silverfish",
                "giant",
            ] {
                let (mut ledger, session) = receipts(version);
                ledger.remove(7);
                ledger.history_context(version, 3, 2);
                ledger
                    .insert(
                        version,
                        NativeSpawn {
                            id: 7,
                            uuid: Some([2; 16]),
                            type_id: None,
                            dedicated_type_name: Some(name),
                            position: [0.9, 65., 0.5],
                            living: Some(true),
                        },
                        2,
                        16,
                    )
                    .unwrap();
                ledger.initialize_motion(
                    7,
                    NativeSpawnMotion {
                        position: Some([0.9, 65., 0.5]),
                        rotation: Some([0.; 2]),
                        head_yaw: Some(0.),
                        velocity: Some([0.; 3]),
                    },
                    2,
                );
                let mut model = OwnedContacts::new(session, 10, 99, &ledger, 2).unwrap();
                let body = model.world.bodies.values().next().unwrap();
                assert_eq!(
                    body.dispatch,
                    if name == "wolf" {
                        ContactDispatch::Wolf
                    } else if name == "chicken" {
                        ContactDispatch::Chicken
                    } else {
                        ContactDispatch::InheritedLiving
                    }
                );
                let adult = (body.width, body.height);
                if !matches!(name, "silverfish" | "giant") {
                    let index = version
                        .table()
                        .entity_data(&format!("minecraft:{name}"))
                        .iter()
                        .find(|r| {
                            EntityDataField::Baby
                                .official_names()
                                .iter()
                                .any(|&(owner, field)| r.owner == owner && r.field == field)
                        })
                        .unwrap()
                        .index;
                    for (sequence, baby) in [(3, true), (4, false)] {
                        ledger.history_context(version, 3, sequence);
                        ledger.receive_metadata(
                            7,
                            [(index, EntityDataValue::Bool(baby))],
                            true,
                            sequence,
                        );
                        model.refresh(10, &ledger, sequence).unwrap();
                        let body = model.world.bodies.values().next().unwrap();
                        let factor = if baby { 0.5 } else { 1. };
                        assert_eq!(
                            (body.width, body.height),
                            (adult.0 * factor, adult.1 * factor)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn ordinary_mob_species_metadata_and_packet_guards() {
        let fixture: serde_json::Value = serde_json::from_reader(flate2::read::GzDecoder::new(
            &include_bytes!("../../../data/client_api/entity_owner_oracle.json.gz")[..],
        ))
        .unwrap();
        for (version, key) in [(V::Java1_16_1, "1.16.1"), (V::Java1_21_11, "1.21.11")] {
            for name in [
                "wolf",
                "chicken",
                "rabbit",
                "ocelot",
                "villager",
                "wandering_trader",
                "mooshroom",
                "silverfish",
                "giant",
            ] {
                let (mut ledger, session) = receipts(version);
                ledger.remove(7);
                extra(&mut ledger, version, name, [0.9, 65., 0.5], true, 2);
                let mut model = OwnedContacts::new(session, 10, 99, &ledger, 2).unwrap();
                let baseline = numerical_state(&model);
                let mut sequence = 2;
                let mut variants = 0;
                for action in fixture["versions"][key]["results"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .flat_map(|row| row["steps"].as_array().unwrap())
                    .flat_map(|step| step["actions"].as_array().unwrap())
                    .filter(|a| {
                        a["op"] == "variant" && a["type_name"] == format!("minecraft:{name}")
                    })
                {
                    let alternate = action["alternate"].as_bool().unwrap();
                    let value = match name {
                        "rabbit" => EntityDataValue::Int(if alternate { 99 } else { 0 }),
                        "ocelot" => EntityDataValue::Bool(alternate),
                        "mooshroom" if version == V::Java1_16_1 => {
                            EntityDataValue::String(if alternate { "brown" } else { "red" }.into())
                        }
                        "mooshroom" => EntityDataValue::Int(i32::from(alternate)),
                        "chicken" | "villager" => {
                            EntityDataValue::Other(action["serializer"].as_i64().unwrap() as i32)
                        }
                        _ => unreachable!(),
                    };
                    sequence += 1;
                    ledger.history_context(version, 3, sequence);
                    ledger.receive_metadata(
                        8,
                        [(action["index"].as_u64().unwrap() as u8, value)],
                        true,
                        sequence,
                    );
                    model.refresh(10, &ledger, sequence).unwrap();
                    assert_eq!(numerical_state(&model), baseline, "{key} {name}");
                    variants += 1;
                }
                assert_eq!(
                    variants,
                    if matches!(name, "rabbit" | "ocelot" | "mooshroom" | "villager")
                        || (name == "chicken" && version == V::Java1_21_11)
                    {
                        16
                    } else {
                        0
                    }
                );
                for (status, animation) in [(Some(35), None), (None, Some(4)), (None, Some(5))] {
                    sequence += 1;
                    ledger.history_context(version, 3, sequence);
                    ledger.history_signal(8, status, animation);
                    model.refresh(10, &ledger, sequence).unwrap();
                    assert_eq!(numerical_state(&model), baseline);
                    let fresh = OwnedContacts::new(session, 11, 99, &ledger, sequence).unwrap();
                    assert_eq!(numerical_state(&fresh), baseline);
                }
                for (status, animation, packet, opcode) in [
                    (Some(127), None, "Status", 127),
                    (None, Some(255), "Animation", 255),
                ] {
                    let (mut ledger, session) = receipts(version);
                    ledger.remove(7);
                    extra(&mut ledger, version, name, [0.9, 65., 0.5], true, 2);
                    let mut model = OwnedContacts::new(session, 10, 99, &ledger, 2).unwrap();
                    ledger.history_context(version, 3, sequence + 1);
                    ledger.history_signal(8, status, animation);
                    let error = model.refresh(10, &ledger, sequence + 1).unwrap_err();
                    let refusal = error.movement_refusal().unwrap();
                    assert!(matches!(refusal, MovementRefusal::Unsupported { .. }));
                    let context = refusal.context();
                    assert_eq!(
                        context.entity,
                        Some(model.inputs.values().next().unwrap().entity)
                    );
                    assert_eq!(context.native_entity_id, Some(8));
                    assert_eq!(
                        context.entity_type.as_deref(),
                        Some(format!("minecraft:{name}").as_str())
                    );
                    assert_eq!(context.receive_sequence, Some(sequence + 1));
                    assert_eq!(context.packet.as_ref().unwrap().packet_type, packet);
                    assert_eq!(context.packet.as_ref().unwrap().opcode, opcode);
                }
            }
        }
    }

    #[test]
    fn ordinary_dimension_refresh_survives_receipts_checkpoint_and_no_lerp_ticks() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            for name in [
                "wolf",
                "chicken",
                "rabbit",
                "ocelot",
                "villager",
                "wandering_trader",
                "mooshroom",
                "silverfish",
                "giant",
            ] {
                let (mut ledger, session) = receipts(version);
                ledger.remove(7);
                extra(&mut ledger, version, name, [0.9, 65., 0.5], true, 2);
                let mut model = OwnedContacts::new(session, 10, 99, &ledger, 2).unwrap();
                for (sequence, pose) in [(3, 2), (4, 0)] {
                    ledger.history_context(version, 3, sequence);
                    ledger.receive_metadata(8, [(6, EntityDataValue::Pose(pose))], true, sequence);
                    model.refresh(10, &ledger, sequence).unwrap();
                }
                let bounds = model.world.bodies.values().next().unwrap().contact().bounds;
                let centered = model
                    .world
                    .bodies
                    .values()
                    .next()
                    .unwrap()
                    .centered_bounds();
                if version == V::Java1_16_1 {
                    assert_ne!(bounds, centered, "{name}");
                } else {
                    assert_eq!(bounds, centered, "{name}");
                }
                ledger.history_context(version, 3, 5);
                ledger.receive_motion(version, 8, NativeMotion::Velocity([0.125, -0.25, 0.375]), 5);
                model.refresh(10, &ledger, 5).unwrap();
                assert_eq!(
                    model.world.bodies.values().next().unwrap().contact().bounds,
                    bounds
                );
                let fresh = OwnedContacts::new(session, 11, 99, &ledger, 5).unwrap();
                assert_eq!(
                    fresh.world.bodies.values().next().unwrap().contact().bounds,
                    bounds
                );
                let mut checkpoint = ContactCheckpoint::new(session, 99).unwrap();
                for record in ledger
                    .history_after(session, 5, None, MAX_ENTITY_HISTORY_READ)
                    .unwrap()
                    .records
                {
                    checkpoint.advance(record, None);
                }
                assert!(checkpoint.errors.is_empty());
                assert!(checkpoint.global_error.is_none());
                assert_eq!(
                    checkpoint
                        .model
                        .world
                        .bodies
                        .values()
                        .next()
                        .unwrap()
                        .contact()
                        .bounds,
                    bounds
                );
                player_step(
                    &mut model,
                    &mut crate::client::physics::Body::new([600., 65., 0.]),
                )
                .unwrap();
                assert_eq!(
                    model.world.bodies.values().next().unwrap().contact().bounds,
                    bounds
                );
                ledger.history_context(version, 3, 6);
                ledger.receive_motion(
                    version,
                    8,
                    NativeMotion::Relative {
                        delta: [512, 0, 0],
                        rotation: None,
                        ground: false,
                    },
                    6,
                );
                model.refresh(10, &ledger, 6).unwrap();
                player_step(
                    &mut model,
                    &mut crate::client::physics::Body::new([600., 65., 0.]),
                )
                .unwrap();
                let body = model.world.bodies.values().next().unwrap();
                assert_eq!(body.contact().bounds, body.centered_bounds());
            }
        }
    }

    #[test]
    fn sustained_meaningful_traffic_compacts_numerically_before_fresh_admission() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            let (mut ledger, session) = receipts(version);
            let mut complete = OwnedContacts::new(session, 10, 99, &ledger, 1).unwrap();
            let end = 8 * MAX_ENTITY_HISTORY_RECORDS as u64;
            for sequence in 2..=end {
                ledger.history_context(version, 3, sequence);
                ledger.receive_motion(
                    version,
                    7,
                    NativeMotion::Relative {
                        delta: [if sequence % 2 == 0 { 128 } else { -64 }, 0, 32],
                        rotation: Some([(sequence % 360) as f32, 12.]),
                        ground: true,
                    },
                    sequence,
                );
                ledger.receive_metadata(
                    7,
                    [(6, EntityDataValue::Pose((sequence % 2) as i32))],
                    true,
                    sequence,
                );
                complete.refresh(10, &ledger, sequence).unwrap();
            }
            let checkpoint = ledger.contact_checkpoint(session).unwrap().0.unwrap();
            assert_eq!(checkpoint.model.inputs.len(), 1);
            assert_eq!(checkpoint.model.world.bodies.len(), 1);
            assert!(checkpoint.errors.is_empty());
            assert!(checkpoint.global_error.is_none());
            let public = ledger.history_after(session, end, None, 1).unwrap();
            assert!(public.gap.unwrap().dropped_through > 8 * MAX_ENTITY_HISTORY_RECORDS as u64);
            let mut models = Vec::new();
            for glide in [false, true] {
                let fresh =
                    OwnedContacts::new_scoped(session, 11, 99, &ledger, end, None, glide).unwrap();
                assert_eq!(numerical_state(&fresh), numerical_state(&complete));
                assert_eq!(fresh.origin, public.latest_cursor.ordinal());
                assert_eq!(fresh.cursor, Some(public.latest_cursor));
                assert_eq!(fresh.tick, 0);
                models.push(fresh);
            }
            ledger.history_context(version, 3, end + 1);
            ledger.receive_motion(version, 7, NativeMotion::HeadYaw(45.), end + 1);
            complete.refresh(10, &ledger, end + 1).unwrap();
            for mut fresh in models {
                fresh.refresh(11, &ledger, end + 1).unwrap();
                assert_eq!(numerical_state(&fresh), numerical_state(&complete));
            }
        }
    }

    #[test]
    fn evicted_partial_metadata_refuses_survivor_but_not_retired_lifetime() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            let (mut ledger, session) = receipts(version);
            ledger.receive_metadata(7, [(6, EntityDataValue::Pose(0))], false, 1);
            for sequence in 2..=2 * MAX_ENTITY_HISTORY_RECORDS as u64 {
                ledger.history_context(version, 3, sequence);
                ledger.receive_motion(version, 7, NativeMotion::HeadYaw(0.), sequence);
            }
            let error = OwnedContacts::new(session, 11, 99, &ledger, 16384).unwrap_err();
            assert!(error.to_string().contains("partial decode"));
            let detail = error.movement_refusal().unwrap();
            assert!(matches!(detail, MovementRefusal::Permanent { .. }));
            assert_eq!(
                detail.context().entity_type.as_deref(),
                Some("minecraft:cow")
            );
            assert_eq!(detail.context().native_entity_id, Some(7));
            assert_eq!(
                detail.context().owner_state,
                MovementOwnerState::NotAdmitted
            );
            assert_eq!(
                detail.context().packet.as_ref().unwrap().packet_type,
                "Metadata"
            );
            ledger.remove(7);
            assert!(OwnedContacts::new(session, 12, 99, &ledger, 16384).is_ok());
        }
    }

    #[test]
    fn fresh_ground_and_glide_bootstrap_survive_count_and_byte_eviction() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            for payload in [0, 60_000] {
                let (mut ledger, session) = receipts(version);
                ledger.history_context(version, 3, 2);
                ledger.receive_motion(
                    version,
                    7,
                    NativeMotion::Relative {
                        delta: [512, 0, -128],
                        rotation: Some([8., 16.]),
                        ground: true,
                    },
                    2,
                );
                ledger.receive_metadata(7, [(6, EntityDataValue::Pose(0))], true, 2);
                let complete = OwnedContacts::new(session, 10, 99, &ledger, 2).unwrap();
                let expected = numerical_state(&complete);
                let count = if payload == 0 {
                    MAX_ENTITY_HISTORY_RECORDS + 4
                } else {
                    180
                };
                for sequence in 3..count as u64 + 3 {
                    ledger.history_context(version, 3, sequence);
                    if payload == 0 {
                        ledger.history_signal(500, None, Some(0));
                    } else {
                        ledger.receive_metadata(
                            500,
                            [(250, EntityDataValue::String("x".repeat(payload)))],
                            true,
                            sequence,
                        );
                    }
                }
                let page = ledger
                    .history_after(session, count as u64 + 2, None, 1)
                    .unwrap();
                assert!(page.gap.is_some());
                for glide in [false, true] {
                    let fresh = OwnedContacts::new_scoped(
                        session,
                        11,
                        99,
                        &ledger,
                        count as u64 + 2,
                        None,
                        glide,
                    )
                    .unwrap();
                    assert_eq!(numerical_state(&fresh), expected);
                    assert_eq!(fresh.tick, 0);
                    assert_eq!(fresh.origin, page.latest_cursor.ordinal());
                    assert_eq!(fresh.cursor, Some(page.latest_cursor));
                }
            }
        }
    }

    #[test]
    fn checkpoint_replay_preserves_boat_scope_and_retired_map_history() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            let (mut ledger, session) = receipts(version);
            // Grow and shrink the legacy map, then reuse native IDs. The fresh
            // reducer must preserve these mutations rather than sort survivors.
            for sequence in 2..=45 {
                ledger.history_context(version, 3, sequence);
                let id = 100 + sequence as i32;
                ledger
                    .insert(
                        version,
                        NativeSpawn {
                            id,
                            uuid: Some([2; 16]),
                            type_id: None,
                            dedicated_type_name: Some("cow"),
                            position: [2., 65., 2.],
                            living: Some(true),
                        },
                        sequence,
                        128,
                    )
                    .unwrap();
                ledger.initialize_motion(
                    id,
                    NativeSpawnMotion {
                        position: Some([2., 65., 2.]),
                        rotation: Some([0.; 2]),
                        head_yaw: Some(0.),
                        velocity: Some([0.; 3]),
                    },
                    sequence,
                );
            }
            for id in 102..=145 {
                ledger.remove(id);
            }
            ledger.history_context(version, 3, 46);
            ledger
                .insert(
                    version,
                    NativeSpawn {
                        id: 102,
                        uuid: Some([3; 16]),
                        type_id: None,
                        dedicated_type_name: Some(if version == V::Java1_16_1 {
                            "boat"
                        } else {
                            "oak_boat"
                        }),
                        position: [2., 65., 2.],
                        living: Some(false),
                    },
                    46,
                    128,
                )
                .unwrap();
            ledger.initialize_motion(
                102,
                NativeSpawnMotion {
                    position: Some([2., 65., 2.]),
                    rotation: Some([0.; 2]),
                    head_yaw: Some(0.),
                    velocity: Some([0.; 3]),
                },
                46,
            );
            ledger
                .insert(
                    version,
                    NativeSpawn {
                        id: 8,
                        uuid: Some([4; 16]),
                        type_id: None,
                        dedicated_type_name: Some("player"),
                        position: [2., 65., 2.],
                        living: Some(true),
                    },
                    46,
                    128,
                )
                .unwrap();
            ledger.initialize_motion(
                8,
                NativeSpawnMotion {
                    position: Some([2., 65., 2.]),
                    rotation: Some([0.; 2]),
                    head_yaw: Some(0.),
                    velocity: Some([0.; 3]),
                },
                46,
            );
            let scope = BoatScope {
                entity: ledger.identity(session, 102).unwrap(),
                passengers: vec![99, 8],
                ignored: vec![ledger.identity(session, 8).unwrap()],
            };
            let complete =
                OwnedContacts::new_scoped(session, 10, 99, &ledger, 46, Some(scope.clone()), false)
                    .unwrap();
            for sequence in 47..=MAX_ENTITY_HISTORY_RECORDS as u64 + 150 {
                ledger.history_context(version, 3, sequence);
                ledger.history_signal(500, Some(0), None);
            }
            let fresh = OwnedContacts::new_scoped(
                session,
                11,
                99,
                &ledger,
                MAX_ENTITY_HISTORY_RECORDS as u64 + 150,
                Some(scope),
                false,
            )
            .unwrap();
            assert_eq!(numerical_state(&fresh), numerical_state(&complete));
        }
    }

    #[test]
    fn incomplete_receipt_rolls_back_motion_and_irreversibly_discards_the_owner() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            let (mut ledger, session) = receipts(version);
            let mut model = OwnedContacts::new(session, 11, 99, &ledger, 1).unwrap();
            let before = model.world.bodies.values().next().unwrap().motion.frame();
            ledger.history_context(version, 3, 2);
            ledger.receive_motion(
                version,
                7,
                NativeMotion::Relative {
                    delta: [512, 0, 0],
                    rotation: None,
                    ground: true,
                },
                2,
            );
            ledger.receive_metadata(7, [(6, EntityDataValue::Pose(0))], false, 2);
            assert!(model.refresh(11, &ledger, 2).is_err());
            assert_eq!(
                serde_json::to_value(model.world.bodies.values().next().unwrap().motion.frame())
                    .unwrap(),
                serde_json::to_value(before).unwrap()
            );
            assert!(!model.valid);
            ledger.receive_metadata(7, [(6, EntityDataValue::Pose(0))], true, 3);
            assert!(model.refresh(11, &ledger, 3).is_err());
        }
    }
    #[test]
    fn history_eviction_and_wrong_owner_never_resume_a_contact_model() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            let (mut ledger, session) = receipts(version);
            let mut model = OwnedContacts::new(session, 11, 99, &ledger, 1).unwrap();
            assert!(model.refresh(12, &ledger, 1).is_err());
            assert!(model.refresh(11, &ledger, 1).is_err());
            let mut model = OwnedContacts::new(session, 13, 99, &ledger, 1).unwrap();
            for sequence in 2..=(MAX_ENTITY_HISTORY_RECORDS as u64 + 3) {
                ledger.history_context(version, 3, sequence);
                ledger.history_signal(500, None, Some(0));
            }
            assert!(
                model
                    .refresh(13, &ledger, MAX_ENTITY_HISTORY_RECORDS as u64 + 3)
                    .is_err()
            );
            assert!(!model.valid);
            assert!(
                OwnedContacts::new(
                    session,
                    14,
                    99,
                    &ledger,
                    MAX_ENTITY_HISTORY_RECORDS as u64 + 3
                )
                .is_ok()
            );
        }
    }
    #[test]
    fn own_sleep_and_spin_require_a_new_qualified_movement_kind() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            let (mut ledger, session) = receipts(version);
            let mut model = OwnedContacts::new(session, 11, 99, &ledger, 1).unwrap();
            ledger.history_context(version, 3, 2);
            ledger.receive_metadata(99, [(6, EntityDataValue::Pose(2))], true, 2);
            let error = model.refresh(11, &ledger, 2).unwrap_err();
            assert!(matches!(
                error.movement_refusal(),
                Some(MovementRefusal::Unsupported { .. })
            ));
            assert_eq!(
                error.movement_refusal().unwrap().context().native_entity_id,
                Some(99)
            );
            let admission = OwnedContacts::new(session, 12, 99, &ledger, 2).unwrap_err();
            let c = admission.movement_refusal().unwrap().context();
            assert_eq!(c.packet.as_ref().unwrap().packet_type, "Metadata");
            assert_eq!(c.native_entity_id, Some(99));
            assert_eq!(c.entity_type.as_deref(), Some("minecraft:player"));
            assert_eq!(c.owner_state, MovementOwnerState::NotAdmitted);
            ledger.receive_metadata(99, [(6, EntityDataValue::Pose(0))], true, 3);
            // A new explicit model sees the final own pose, not the older sleep.
            let mut fresh = OwnedContacts::new(session, 12, 99, &ledger, 3).unwrap();
            let index = version.table().entities.living_flags_metadata_index;
            ledger.receive_metadata(99, [(index, EntityDataValue::Byte(4))], true, 4);
            assert!(fresh.refresh(12, &ledger, 4).is_err());
        }
    }
}
