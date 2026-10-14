//! Contact models fenced by one selected movement owner and complete receipts.
use super::{
    EntityDataField, EntityDataValue, EntityHistoryCursor, EntityHistoryKind, EntityHistoryValue,
    EntityId, EntityPose, LivingInterpolationPose, LivingTickBody, MAX_ENTITY_HISTORY_READ,
    MAX_ENTITY_HISTORY_RECORDS, SpawnLedger,
    contact_tick::{ContactDispatch, ContactTickBody, ContactTickWorld, failure},
};
use crate::{
    Result,
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
    origin: u64,
    tick: u64,
    receive_sequence: u64,
    valid: bool,
    boat: Option<BoatScope>,
    native_keys: BTreeMap<i32, u64>,
    own_glide: bool,
}
impl OwnedContacts {
    pub(crate) fn has_bodies(&self) -> bool {
        !self.world.bodies.is_empty()
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
        let version = self.session.version;
        let inputs = &self.inputs;
        let read = std::cell::RefCell::new(block_at);
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
        use crate::client::{LivingContactBody, LivingPushBody, physics};
        let version = self.session.version;
        let read = std::cell::RefCell::new(block_at);
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
        if self.has_bodies() && !environment.spectator {
            local.push.pushable =
                !physics::contact_climbable(version, body.position, &mut *read.borrow_mut())?;
        }
        let mut outcome = None;
        let inputs = &self.inputs;
        let nonempty = self.has_bodies();
        let pairs = self.world.tick(
            &mut local,
            (own_team.map(|t| t.0.as_str()), environment.spectator),
            false,
            |local| {
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
        let mut model = Self {
            world: ContactTickWorld::new(session.version, 1),
            session,
            owner,
            cursor: None,
            fresh: 1,
            inputs: BTreeMap::new(),
            origin: 0,
            tick: 0,
            receive_sequence,
            valid: true,
            boat,
            native_keys: BTreeMap::from([(own_native_id, 1)]),
            own_glide,
        };
        model.world.admit(tick_body(1, own_native_id, true), None)?;
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
    }

    pub(crate) fn refresh(
        &mut self,
        owner: u64,
        ledger: &SpawnLedger,
        sequence: u64,
    ) -> Result<()> {
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
    }

    fn drain(&mut self, ledger: &SpawnLedger, sequence: u64, bootstrap: bool) -> Result<()> {
        let mut records = 0;
        let own_id = self
            .world
            .order
            .body(self.world.own_key)
            .ok_or_else(|| failure("own contact lifetime missing"))?
            .native_id;
        let mut own_metadata = BTreeMap::new();
        let surviving = ledger
            .capture(self.session, sequence)
            .entities
            .into_iter()
            .map(|e| e.id)
            .collect::<Vec<_>>();
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
                // A new owner's numerical constructor origin belongs to its
                // current world. Earlier worlds never supply current bodies.
                if record.session != self.session {
                    if bootstrap {
                        continue;
                    }
                    return Err(failure("contact record lifetime changed"));
                }
                let ordinal = record.ordinal;
                match &record.kind {
                    EntityHistoryKind::Spawn(m) => {
                        let id = m.entity.id;
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
                        if id.session() != self.session
                            || id.spawn_sequence() != record.receive_sequence
                        {
                            return Err(failure("contact spawn provenance"));
                        }
                        let Some(name) = m.entity.type_name.as_deref() else {
                            return Err(failure("unknown contact entity type"));
                        };
                        if (bootstrap && !surviving.contains(&id))
                            || !matches!(
                                name,
                                "minecraft:player"
                                    | "minecraft:cow"
                                    | "minecraft:pig"
                                    | "minecraft:zombie"
                            )
                            || self.boat.as_ref().is_some_and(|b| b.ignored.contains(&id))
                        {
                            // Current unqualified dispatch is refused below. A
                            // removed old body still contributes map capacity.
                            self.fresh = self
                                .fresh
                                .checked_add(1)
                                .ok_or_else(|| failure("contact lifetime exhausted"))?;
                            self.world
                                .admit(tick_body(self.fresh, id.native_id(), false), None)?;
                            self.native_keys.insert(id.native_id(), self.fresh);
                            continue;
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
                            },
                        );
                    }
                    EntityHistoryKind::Motion(m) => {
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
                                    own_metadata.insert(entry.index, entry.value.clone());
                                } else {
                                    self.own_metadata(entry.index, &entry.value)?;
                                }
                            }
                            continue;
                        }
                        let Some(entity) = entity else {
                            continue;
                        };
                        let Some(key) = self
                            .inputs
                            .iter()
                            .find(|(_, i)| i.entity == *entity)
                            .map(|(&k, _)| k)
                        else {
                            continue;
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
                            return Err(failure("contact entity requires unit scale"));
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
                        return Err(failure("unqualified contact status/animation dispatch"));
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
                                    continue;
                                }
                            }
                        }
                        if self.inputs.values().any(|i| {
                            Some(i.entity) == *entity
                                || value.iter().any(|(_, id)| *id == Some(i.entity))
                        }) {
                            return Err(failure(
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
            }
            self.cursor = Some(page.next_cursor);
            if !page.has_more {
                break;
            }
        }
        for (index, value) in own_metadata {
            self.own_metadata(index, &value)?;
        }
        // Every surviving body must have a qualified runtime dispatch. No
        // default box or received target is promoted to current contact state.
        let surviving = ledger.capture(self.session, sequence);
        for spawn in surviving.entities {
            if !self.inputs.values().any(|i| i.entity == spawn.id)
                && !self.boat.as_ref().is_some_and(|scope| {
                    scope.ignored.contains(&spawn.id)
                        || spawn
                            .type_name
                            .as_deref()
                            .is_some_and(crate::client::vehicle::collision::rigid)
                })
            {
                return Err(failure(format!(
                    "unqualified contact dispatch: {:?}",
                    spawn.type_name
                )));
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
                return Err(failure("own contact pose unavailable"));
            };
            let pose = EntityPose::from_native(self.session.version, *id);
            if !matches!(
                pose,
                Some(EntityPose::Standing | EntityPose::Crouching | EntityPose::Swimming)
            ) && !(self.own_glide && pose == Some(EntityPose::FallFlying))
            {
                return Err(failure(
                    "own posture outside ground/held flight contact model",
                ));
            }
        } else if Some(index) == field(EntityDataField::SleepingPosition) {
            if !matches!(value, EntityDataValue::OptionalBlockPosition(None)) {
                return Err(failure("own sleep requires separate movement owner"));
            }
        } else if Some(index) == field(EntityDataField::LivingFlags)
            && value.as_i64().is_none_or(|flags| flags & 4 != 0)
        {
            return Err(failure("own spin attack requires separate movement owner"));
        }
        Ok(())
    }

    fn metadata(&mut self, key: u64, index: u8, value: &EntityDataValue) -> Result<()> {
        let input = self.inputs.get_mut(&key).unwrap();
        let body = self.world.bodies.get_mut(&key).unwrap();
        let table = self.session.version.table();
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
        if index == 6 {
            let EntityDataValue::Pose(id) = value else {
                return Err(failure("unknown contact pose serializer"));
            };
            input.pose = EntityPose::from_native(self.session.version, *id)
                .ok_or_else(|| failure("unknown contact pose"))?;
            let dimensions = super::geometry::shape(
                self.session.version,
                &input.name,
                input.pose,
                input.baby,
                1.,
            );
            body.width = dimensions.0;
            body.height = dimensions.1;
        } else if Some(index) == field(EntityDataField::Health) {
            let EntityDataValue::Float(health) = value else {
                return Err(failure("unknown contact health"));
            };
            if !health.is_finite() {
                return Err(failure("nonfinite contact health"));
            }
            input.alive = *health > 0.;
        } else if Some(index) == field(EntityDataField::Baby) {
            let EntityDataValue::Bool(baby) = value else {
                return Err(failure("unknown contact baby data"));
            };
            input.baby = if *baby { 0.5 } else { 1. };
            let dimensions = super::geometry::shape(
                self.session.version,
                &input.name,
                input.pose,
                input.baby,
                1.,
            );
            body.width = dimensions.0;
            body.height = dimensions.1;
        } else if Some(index) == field(EntityDataField::SleepingPosition) {
            let EntityDataValue::OptionalBlockPosition(position) = value else {
                return Err(failure("unknown contact sleeping position"));
            };
            body.sleeping = position.is_some();
            if let Some(position) = position {
                let mut pose = body.motion.frame().interpolation.pose;
                pose.position = [
                    f64::from(position.x) + 0.5,
                    f64::from(position.y) + 0.6875,
                    f64::from(position.z) + 0.5,
                ];
                body.motion.set_model_pose(pose).map_err(failure)?;
            }
        } else if Some(index) == field(EntityDataField::LivingFlags) {
            let Some(flags) = value.as_i64() else {
                return Err(failure("unknown contact living flags"));
            };
            if flags & 4 != 0 {
                return Err(failure("spin attack contact dispatch unqualified"));
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

    pub(crate) fn teams(
        &mut self,
        teams: &TeamsObservation,
        profiles: &PlayerListObservation,
    ) -> Result<()> {
        if teams.session != self.session || profiles.session != self.session {
            return Err(failure("contact team/profile world"));
        }
        for (&key, input) in &mut self.inputs {
            let uuid = input
                .uuid
                .ok_or_else(|| failure("contact scoreboard UUID missing"))?;
            let holder = if input.name == "minecraft:player" {
                // Original RemotePlayer requires a registered profile. Use the
                // actual retained UUID/name, never the configured bot name.
                let profile = profiles
                    .entries
                    .iter()
                    .find(|p| p.uuid == uuid)
                    .ok_or_else(|| failure("remote player contact profile missing"))?;
                let mode = profile
                    .game_mode
                    .as_ref()
                    .and_then(|mode| mode.value)
                    .ok_or_else(|| failure("remote player contact game mode missing"))?;
                input.spectator = mode == crate::client::GameMode::Spectator;
                profile.profile.value.name.clone()
            } else {
                uuid_holder(uuid)
            };
            self.world.bodies.get_mut(&key).unwrap().team = team(teams, &holder);
        }
        Ok(())
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
        }
    }
    #[test]
    fn own_sleep_and_spin_require_a_new_qualified_movement_kind() {
        for version in [V::Java1_16_1, V::Java1_21_11] {
            let (mut ledger, session) = receipts(version);
            let mut model = OwnedContacts::new(session, 11, 99, &ledger, 1).unwrap();
            ledger.history_context(version, 3, 2);
            ledger.receive_metadata(99, [(6, EntityDataValue::Pose(2))], true, 2);
            assert!(model.refresh(11, &ledger, 2).is_err());
            ledger.receive_metadata(99, [(6, EntityDataValue::Pose(0))], true, 3);
            // A new explicit model sees the final own pose, not the older sleep.
            let mut fresh = OwnedContacts::new(session, 12, 99, &ledger, 3).unwrap();
            let index = version.table().entities.living_flags_metadata_index;
            ledger.receive_metadata(99, [(index, EntityDataValue::Byte(4))], true, 4);
            assert!(fresh.refresh(12, &ledger, 4).is_err());
        }
    }
}
