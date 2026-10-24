//! Finite exclusion at the declared receipt/model origin; never a server clock.
use super::{
    EntityDataField, EntityDataValue, EntityHistoryCursor, EntityHistoryKind as K,
    EntityHistoryRecord, EntityHistoryValue, EntityId, EntityPose, LivingInterpolationPose,
    LivingPacketGuards, LivingPacketModel, MAX_ENTITY_HISTORY_READ, SpawnLedger,
    contact_capabilities::hull,
    contact_policy::{self, selectors},
};
use crate::{
    MovementRefusal, Result,
    client::{Aabb, SessionStamp, physics::Body},
};
use std::collections::{BTreeMap, BTreeSet};

// Separate from the numerical dispatch table: qualifying a new ordinary type
// must not silently qualify its ride/attachment hooks for exclusion.
fn audited_type(version: crate::MinecraftVersion, name: &str) -> bool {
    matches!(
        version,
        crate::MinecraftVersion::Java1_16_1 | crate::MinecraftVersion::Java1_21_11
    ) && matches!(
        name,
        "minecraft:chicken"
            | "minecraft:cow"
            | "minecraft:creeper"
            | "minecraft:giant"
            | "minecraft:mooshroom"
            | "minecraft:ocelot"
            | "minecraft:pig"
            | "minecraft:rabbit"
            | "minecraft:sheep"
            | "minecraft:silverfish"
            | "minecraft:villager"
            | "minecraft:wandering_trader"
            | "minecraft:zombie"
            | "minecraft:player"
    ) && selectors::dispatch(version, name).is_some()
}

fn default_spider(version: crate::MinecraftVersion, name: &str) -> bool {
    matches!(name, "minecraft:spider" | "minecraft:cave_spider")
        && contact_policy::locality::DeferredPolicy::new(version, name).envelope(false)
}
fn no_contact(version: crate::MinecraftVersion, name: &str) -> bool {
    use super::contact_capabilities::{Classification, capabilities};
    let c = capabilities(version, name);
    // Geometry may remain unknown only because every way it could matter to
    // this contact lane, including callbacks, has positive irrelevance evidence.
    [
        c.walking,
        c.incoming,
        c.solid,
        c.metadata,
        c.status,
        c.animation,
    ]
    .into_iter()
    .all(|c| c == Classification::ProvenIrrelevant)
}

#[derive(Clone, Debug)]
struct Bound {
    id: EntityId,
    name: Option<String>,
    position: Option<[f64; 3]>,
    motion: Option<LivingPacketModel>,
    envelope: Option<Aabb>,
    // Maximum ordinary shape, including constructor and metadata placements.
    dimensions: [f64; 2],
    irrelevant: bool,
    fault: Option<String>,
    ordinal: u64,
    sequence: u64,
    trigger: crate::MovementPacketTrigger,
}
impl Bound {
    fn error(&self, reason: &str) -> crate::Error {
        let mut value = MovementRefusal::permanent(reason.into());
        let c = value.context_mut();
        c.entity = Some(self.id);
        c.native_entity_id = Some(self.id.native_id());
        c.entity_type = self.name.clone();
        c.record_ordinal = Some(self.ordinal);
        c.receive_sequence = Some(self.sequence);
        c.packet = Some(self.trigger.clone());
        let mut value = MovementRefusal::Unsupported { context: c.clone() };
        let g = value.geometry_mut();
        g.entity_position = self.position;
        if let Some(p) = self.position.filter(|p| p.iter().all(|x| x.is_finite())) {
            let p = p.map(|x| x.floor() as i32);
            g.block_position = Some(p);
            g.chunk_position = Some([p[0].div_euclid(16), p[2].div_euclid(16)]);
        }
        value.into_error()
    }

    fn include(&mut self, p: [f64; 3]) {
        if p.iter().any(|x| !x.is_finite())
            || p[0].abs() > 32_000_000.
            || p[2].abs() > 32_000_000.
            || p[1].abs() > 8_000_000.
        {
            self.fault = Some("contact scope coordinates unavailable".into());
            return;
        }
        let [w, h] = self.dimensions;
        // Enclose unclamped codec anchors and original clamped placement,
        // native float dimensions, pair selector (0.2) and rounding.
        let b = Aabb {
            min_x: p[0].min(p[0].clamp(-30_000_000., 30_000_000.)) - w - 1.,
            min_y: p[1] - 1.,
            min_z: p[2].min(p[2].clamp(-30_000_000., 30_000_000.)) - w - 1.,
            max_x: p[0].max(p[0].clamp(-30_000_000., 30_000_000.)) + w + 1.,
            max_y: p[1] + h + 1.,
            max_z: p[2].max(p[2].clamp(-30_000_000., 30_000_000.)) + w + 1.,
        };
        self.envelope = Some(self.envelope.map_or(b, |old| hull(old, b)));
    }
    fn extend(&mut self) {
        if let Some(m) = &self.motion {
            let f = m.frame();
            let anchors = [
                Some(f.codec_base),
                Some(f.interpolation.pose.position),
                f.interpolation.target.map(|p| p.position),
                m.collision_query_position(),
            ];
            for p in anchors.into_iter().flatten() {
                self.include(p);
            }
        }
    }
}

type RiderMembership = (EntityId, Vec<(i32, Option<EntityId>)>);

#[derive(Clone, Debug, Default)]
pub(super) struct ScopeReceipts {
    members: BTreeMap<i32, Bound>,
    // Exact lifetimes, rather than numeric IDs which can be reused.
    riders: BTreeMap<i32, RiderMembership>,
    global: Option<MovementRefusal>,
}
#[derive(Clone, Debug)]
struct Group {
    ids: Vec<i32>,
    // None is a positive class proof that every member has no contact
    // effects. Unknown geometry never creates this alternative.
    envelope: Option<Aabb>,
    mounted: bool,
}

fn trigger(kind: &K) -> crate::MovementPacketTrigger {
    let (packet_type, opcode) = match kind {
        K::Spawn(_) => ("Spawn", 0),
        K::Motion(_) => ("Motion", 0),
        K::Metadata { .. } => ("Metadata", 0),
        K::Scale { .. } => ("Scale", 0),
        K::Equipment { .. } => ("Equipment", 0),
        K::Passengers { .. } => ("Passengers", 0),
        K::Status { status, .. } => ("Status", i32::from(*status)),
        K::Animation { animation, .. } => ("Animation", i32::from(*animation)),
        K::Removed { .. } => ("Removed", 0),
        K::WorldChanged { .. } => ("WorldChanged", 0),
        K::OwnPositionCorrection { .. } => ("OwnPositionCorrection", 0),
    };
    crate::MovementPacketTrigger {
        packet_type: packet_type.into(),
        opcode,
    }
}

fn intersects(a: Aabb, b: Aabb) -> bool {
    a.min_x <= b.max_x
        && a.max_x >= b.min_x
        && a.min_y <= b.max_y
        && a.max_y >= b.min_y
        && a.min_z <= b.max_z
        && a.max_z >= b.min_z
}
fn expand(a: Aabb, amount: f64) -> Aabb {
    Aabb {
        min_x: a.min_x - amount,
        min_y: a.min_y - amount,
        min_z: a.min_z - amount,
        max_x: a.max_x + amount,
        max_y: a.max_y + amount,
        max_z: a.max_z + amount,
    }
}
fn permanent(reason: &str) -> crate::Error {
    MovementRefusal::permanent(reason.into()).into_error()
}
impl ScopeReceipts {
    pub(super) fn apply(&mut self, r: &EntityHistoryRecord, own: i32) {
        match &r.kind {
            K::WorldChanged { .. } => {
                *self = Self::default();
            }
            K::Spawn(m) => {
                let id = m.entity.id;
                let name = m.entity.type_name.clone();
                let irrelevant = name
                    .as_deref()
                    .is_some_and(|n| no_contact(r.session.version, n));
                let qualified = name.as_deref().is_some_and(|n| {
                    audited_type(r.session.version, n) || default_spider(r.session.version, n)
                });
                let d = name
                    .as_deref()
                    .and_then(|n| r.session.version.table().entity_dimensions(n));
                let pose = m
                    .position
                    .as_ref()
                    .zip(m.rotation.as_ref())
                    .and_then(|(p, rot)| {
                        let source = crate::client::ValueSource::Received {
                            sequence: r.receive_sequence,
                        };
                        (p.source == source && rot.source == source).then_some(
                            LivingInterpolationPose {
                                position: p.value.position,
                                rotation: rot.value,
                            },
                        )
                    });
                let mut b = Bound {
                    id,
                    name,
                    position: m.position.as_ref().map(|p| p.value.position),
                    motion: pose
                        .and_then(|p| LivingPacketModel::new(r.session.version, p, r.ordinal).ok()),
                    envelope: None,
                    dimensions: d.map_or([0.; 2], |d| {
                        [f64::from(d.width as f32), f64::from(d.height as f32)]
                    }),
                    irrelevant,
                    fault: None,
                    ordinal: r.ordinal,
                    sequence: r.receive_sequence,
                    trigger: trigger(&r.kind),
                };
                if (!irrelevant && (!qualified || d.is_none() || b.motion.is_none()))
                    || id.spawn_sequence() != r.receive_sequence
                {
                    b.fault = Some("no audited finite contact scope for this spawn".into());
                }
                // Velocity is part of A's declared input, even though the remote
                // nonauthoritative lane never travels using it.
                if !irrelevant
                    && m.velocity
                        .as_ref()
                        .is_none_or(|v| v.value.iter().any(|x| !x.is_finite()))
                {
                    b.fault = Some("contact scope velocity unavailable".into());
                }
                let source = crate::client::ValueSource::Received {
                    sequence: r.receive_sequence,
                };
                if id.session() != r.session
                    || (!irrelevant
                        && (m.velocity.as_ref().is_some_and(|v| v.source != source)
                            || m.head_yaw.as_ref().is_some_and(|v| !v.value.is_finite())))
                {
                    b.fault = Some("contact scope spawn provenance/numeric input invalid".into());
                }
                if !irrelevant {
                    b.extend();
                }
                self.members.insert(id.native_id(), b);
            }
            K::Removed { native_id, .. } => {
                // Preserve possible attached placements on surviving children
                // before removing relationships. A reuse is a fresh lifetime.
                self.preserve_groups(&[*native_id]);
                self.members.remove(native_id);
                self.riders.remove(native_id);
                for (_, children) in self.riders.values_mut() {
                    children.retain(|(id, _)| id != native_id);
                }
            }
            K::Passengers {
                native_id,
                entity,
                passengers,
            } => {
                for (&id, b) in &mut self.members {
                    let involved = *native_id == id
                        || match passengers {
                            EntityHistoryValue::Retained { value } => {
                                value.iter().any(|(child, _)| *child == id)
                            }
                            EntityHistoryValue::RetentionLimit => true,
                        };
                    if involved
                        && b.name
                            .as_deref()
                            .is_some_and(|n| default_spider(r.session.version, n))
                        && b.fault.is_none()
                    {
                        b.fault =
                            Some("default spider mount involvement revokes finite proof".into());
                        b.ordinal = r.ordinal;
                        b.sequence = r.receive_sequence;
                        b.trigger = trigger(&r.kind);
                    }
                }

                if let (Some(entity), EntityHistoryValue::Retained { value }) = (entity, passengers)
                {
                    if self
                        .riders
                        .get(native_id)
                        .is_some_and(|old| old == &(*entity, value.clone()))
                        || (value.is_empty() && !self.riders.contains_key(native_id))
                    {
                        return;
                    }
                }
                let mut affected = vec![*native_id];
                if let EntityHistoryValue::Retained { value } = passengers {
                    affected.extend(value.iter().map(|(id, _)| *id));
                }
                self.preserve_groups(&affected);
                for id in &affected {
                    if let Some(b) = self.members.get_mut(id).filter(|b| b.fault.is_none()) {
                        b.ordinal = r.ordinal;
                        b.sequence = r.receive_sequence;
                        b.trigger = trigger(&r.kind);
                    }
                }
                match (entity, passengers) {
                    (Some(entity), EntityHistoryValue::Retained { value }) => {
                        if value.is_empty() {
                            self.riders.remove(native_id);
                        } else {
                            self.riders.insert(*native_id, (*entity, value.clone()));
                        }
                    }
                    _ => {
                        self.global = Some(self.record_error(
                            r,
                            *native_id,
                            *entity,
                            "contact scope passenger history unavailable",
                        ));
                    }
                }
            }
            _ => {
                let (native, entity) = match &r.kind {
                    K::Motion(m) => (m.entity.id.native_id(), Some(m.entity.id)),
                    K::Metadata {
                        native_id, entity, ..
                    }
                    | K::Scale {
                        native_id, entity, ..
                    }
                    | K::Equipment {
                        native_id, entity, ..
                    }
                    | K::Status {
                        native_id, entity, ..
                    }
                    | K::Animation {
                        native_id, entity, ..
                    } => (*native_id, *entity),
                    _ => return,
                };
                if native == own {
                    return;
                }
                let Some(b) = self.members.get_mut(&native) else {
                    self.global = Some(self.record_error(
                        r,
                        native,
                        entity,
                        "contact scope missing spawn history",
                    ));
                    return;
                };
                if b.fault.is_some() {
                    return;
                }
                b.ordinal = r.ordinal;
                b.sequence = r.receive_sequence;
                b.trigger = trigger(&r.kind);
                if entity != Some(b.id) {
                    b.fault = Some("contact scope lifetime changed".into());
                    return;
                }
                let result = apply_body(b, r);
                if let Err(error) = result {
                    b.fault.get_or_insert_with(|| error.to_string());
                }
            }
        }
    }
    fn record_error(
        &self,
        record: &EntityHistoryRecord,
        native: i32,
        entity: Option<EntityId>,
        reason: &str,
    ) -> MovementRefusal {
        let mut value = self
            .members
            .get(&native)
            .and_then(|b| b.error(reason).movement_refusal().cloned())
            .unwrap_or_else(|| MovementRefusal::permanent(reason.into()));
        let c = value.context_mut();
        c.native_entity_id = Some(native);
        c.entity = entity;
        c.receive_sequence = Some(record.receive_sequence);
        c.record_ordinal = Some(record.ordinal);
        c.packet = Some(trigger(&record.kind));
        value
    }
    fn preserve_groups(&mut self, affected: &[i32]) {
        if !self.riders.iter().any(|(id, (_, children))| {
            affected.contains(id) || children.iter().any(|(id, _)| affected.contains(id))
        }) {
            return;
        }
        match self.groups() {
            Ok(groups) => {
                for (ids, envelope) in groups
                    .into_iter()
                    .filter(|g| g.mounted && g.ids.iter().any(|id| affected.contains(id)))
                    .filter_map(|g| g.envelope.map(|envelope| (g.ids, envelope)))
                {
                    for id in ids {
                        if let Some(b) = self.members.get_mut(&id) {
                            b.envelope = Some(b.envelope.map_or(envelope, |v| hull(v, envelope)));
                        }
                    }
                }
            }
            Err(error) => {
                // Keep the original entity/packet/coordinates as well as the
                // irrecoverable relationship failure; a distance test cannot
                // reconstruct a lost attachment placement.
                let mut value = error
                    .movement_refusal()
                    .cloned()
                    .unwrap_or_else(|| MovementRefusal::permanent(error.to_string()));
                let c = value.context_mut();
                c.reason = format!(
                    "contact scope lost relationship placement history (fresh connection required): {}",
                    c.reason
                );
                self.global = Some(value);
            }
        }
    }
    fn groups(&self) -> Result<Vec<Group>> {
        if let Some(error) = &self.global {
            return Err(error.clone().into_error());
        }
        let mut neighbors: BTreeMap<i32, Vec<i32>> = BTreeMap::new();
        let mut parents = BTreeMap::new();
        for (&parent, (entity, children)) in &self.riders {
            let b = self
                .members
                .get(&parent)
                .ok_or_else(|| permanent("contact carrier spawn missing"))?;
            if b.id != *entity {
                return Err(b.error("contact carrier numeric ID reused"));
            }
            for (id, lifetime) in children {
                let child = self
                    .members
                    .get(id)
                    .ok_or_else(|| b.error("contact rider spawn unresolved"))?;
                if Some(child.id) != *lifetime
                    || *id == parent
                    || parents.insert(*id, parent).is_some()
                {
                    return Err(child.error("contact rider lifetime/relationship unavailable"));
                }
                neighbors.entry(parent).or_default().push(*id);
                neighbors.entry(*id).or_default().push(parent);
            }
        }
        for &start in parents.keys() {
            let mut path = BTreeSet::new();
            let mut id = start;
            while let Some(&parent) = parents.get(&id) {
                if !path.insert(id) {
                    return Err(self.members[&id].error("cyclic contact riders"));
                }
                id = parent;
            }
        }
        let mut seen = BTreeSet::new();
        let mut groups = Vec::new();
        for &id in self.members.keys() {
            if !seen.insert(id) {
                continue;
            }
            let mut ids = vec![id];
            let mut at = 0;
            while at < ids.len() {
                if let Some(peers) = neighbors.get(&ids[at]) {
                    for &peer in peers {
                        if seen.insert(peer) {
                            ids.push(peer);
                        }
                    }
                }
                at += 1;
            }
            for id in &ids {
                let body = &self.members[id];
                if let Some(reason) = &body.fault {
                    return Err(body.error(reason));
                }
            }
            if ids.iter().all(|id| self.members[id].irrelevant) {
                groups.push(Group {
                    mounted: ids.len() > 1,
                    ids,
                    envelope: None,
                });
                continue;
            }
            let mut envelope = None;
            let mut offsets = 0.;
            for id in &ids {
                let body = &self.members[id];
                let bounds = body.envelope.ok_or_else(|| {
                    body.error("carrier/passenger group lacks a finite member envelope")
                })?;
                envelope = Some(envelope.map_or(bounds, |old| hull(old, bounds)));
                if neighbors.contains_key(id) {
                    offsets += 4. * (body.dimensions[0] + body.dimensions[1] + 1.);
                }
            }
            let envelope = envelope.expect("nonempty validated group");
            let envelope = if offsets == 0. {
                envelope
            } else {
                let mut b = expand(envelope, offsets);
                let dx = envelope.max_x - envelope.min_x;
                let dy = envelope.max_y - envelope.min_y;
                let dz = envelope.max_z - envelope.min_z;
                b.min_x -= dx;
                b.max_x += dx;
                b.min_y -= dy;
                b.max_y += dy;
                b.min_z -= dz;
                b.max_z += dz;
                b
            };
            groups.push(Group {
                mounted: ids.len() > 1,
                ids,
                envelope: Some(envelope),
            });
        }
        Ok(groups)
    }
    fn capture(
        session: SessionStamp,
        own: i32,
        ledger: &SpawnLedger,
        sequence: u64,
    ) -> Result<(Self, EntityHistoryCursor)> {
        if ledger.contact_checkpoint_required() {
            return Err(permanent("contact scope checkpoint unavailable"));
        }
        let (checkpoint, mut cursor) = ledger.contact_checkpoint(session)?;
        let mut proof = checkpoint.map_or_else(Self::default, |c| c.proof);
        // Bind checkpoint lifetimes to the capturing connection, just as the
        // numerical checkpoint does. Version/world remain exact.
        for b in proof.members.values_mut() {
            b.id.session.connection_id = session.connection_id;
        }
        if let Some(error) = &mut proof.global {
            let context = error.context_mut();
            context.session = Some(session);
            if let Some(id) = &mut context.entity {
                id.session.connection_id = session.connection_id;
            }
        }
        for (id, children) in proof.riders.values_mut() {
            id.session.connection_id = session.connection_id;
            for (_, child) in children {
                if let Some(id) = child {
                    id.session.connection_id = session.connection_id;
                }
            }
        }
        loop {
            let page =
                ledger.history_after(session, sequence, Some(cursor), MAX_ENTITY_HISTORY_READ)?;
            if page.gap.is_some() || page.session != session {
                return Err(permanent("contact scope history gap/world changed"));
            }
            for record in &page.records {
                proof.apply(record, own);
            }
            cursor = page.next_cursor;
            if !page.has_more {
                break;
            }
        }
        let surviving = ledger.capture(session, sequence).entities;
        if surviving
            .iter()
            .any(|e| !proof.members.values().any(|b| b.id == e.id))
        {
            return Err(permanent(
                "contact scope retained lifetime lacks complete history",
            ));
        }
        Ok((proof, cursor))
    }
}

fn apply_body(b: &mut Bound, r: &EntityHistoryRecord) -> Result<()> {
    let version = r.session.version;
    if b.irrelevant {
        if let K::Motion(m) = &r.kind {
            b.position = m.position.as_ref().map(|p| p.value.position).or(b.position);
        }
        return Ok(());
    }
    if b.name
        .as_deref()
        .is_some_and(|n| default_spider(version, n))
        && !matches!(r.kind, K::Motion(_))
    {
        return Err(permanent(
            "default spider callback/metadata revokes finite proof",
        ));
    }
    match &r.kind {
        K::Motion(m) => {
            b.position = m.position.as_ref().map(|p| p.value.position).or(b.position);
            let packet = m
                .packet
                .as_ref()
                .filter(|p| {
                    p.source
                        == crate::client::ValueSource::Received {
                            sequence: r.receive_sequence,
                        }
                })
                .ok_or_else(|| permanent("contact scope motion provenance missing"))?
                .value
                .living_packet(version)
                .map_err(|_| permanent("contact scope motion unsupported"))?;
            b.motion
                .as_mut()
                .ok_or_else(|| permanent("contact scope motion unavailable"))?
                .apply(
                    r.ordinal,
                    packet,
                    LivingPacketGuards {
                        local_authoritative: false,
                        ticking: true,
                    },
                )
                .map_err(|_| permanent("contact scope motion invalid"))?;
            match packet {
                super::LivingMotionPacket::LegacyTeleport { position, .. }
                | super::LivingMotionPacket::ModernPositionSync { position, .. } => {
                    b.include(position)
                }
                _ => {}
            }
            b.extend();
        }
        K::Metadata {
            entries: EntityHistoryValue::Retained { value },
            complete: true,
            ..
        } => {
            let name = b
                .name
                .as_deref()
                .ok_or_else(|| permanent("contact scope type missing"))?;
            let rows = version.table().entity_data(name);
            let field = |index, f: EntityDataField| {
                rows.iter().any(|row| {
                    row.index == index
                        && f.official_names()
                            .iter()
                            .any(|&(owner, name)| row.owner == owner && row.field == name)
                })
            };
            let mut anchors = Vec::new();
            for entry in value {
                if !rows.iter().any(|row| row.index == entry.index) {
                    return Err(permanent("contact scope metadata unknown"));
                }
                if matches!(entry.value, EntityDataValue::Float(v) if !v.is_finite()) {
                    return Err(permanent("contact scope nonfinite metadata"));
                }
                if entry.index == 6 {
                    let EntityDataValue::Pose(pose) = entry.value else {
                        return Err(permanent("contact scope pose serializer unknown"));
                    };
                    let pose = EntityPose::from_native(version, pose)
                        .ok_or_else(|| permanent("contact scope pose unknown"))?;
                    let shape = selectors::shape(version, name, pose, 1., 1.);
                    if f64::from(shape.0) > b.dimensions[0] || f64::from(shape.1) > b.dimensions[1]
                    {
                        return Err(permanent("contact scope shape exceeds audited dimensions"));
                    }
                } else if field(entry.index, EntityDataField::Health) {
                    if !matches!(entry.value, EntityDataValue::Float(health) if health.is_finite())
                    {
                        return Err(permanent(
                            "contact scope health serializer/numeric input unknown",
                        ));
                    }
                } else if field(entry.index, EntityDataField::Baby) {
                    if !matches!(entry.value, EntityDataValue::Bool(_)) {
                        return Err(permanent("contact scope age unknown"));
                    }
                } else if field(entry.index, EntityDataField::SleepingPosition) {
                    let EntityDataValue::OptionalBlockPosition(position) = entry.value else {
                        return Err(permanent("contact scope sleeping serializer unknown"));
                    };
                    if let Some(p) = position {
                        anchors.push([
                            f64::from(p.x) + 0.5,
                            f64::from(p.y) + 0.6875,
                            f64::from(p.z) + 0.5,
                        ]);
                    }
                } else if field(entry.index, EntityDataField::LivingFlags)
                    && entry.value.as_i64().is_none_or(|f| f & 4 != 0)
                {
                    return Err(permanent("contact scope spin dispatch unknown"));
                }
            }
            for p in anchors {
                // A bed callback can move feet after modern interpolation's
                // previous pose was retained. Enclose both collision choices
                // for target + new_pose - previous_pose, including a previous
                // pose reached during omitted ticks rather than our codec pose.
                if let Some(old) = b.envelope {
                    let span = [
                        old.max_x - old.min_x,
                        old.max_y - old.min_y,
                        old.max_z - old.min_z,
                    ];
                    b.include(std::array::from_fn(|i| p[i] - span[i]));
                    b.include(std::array::from_fn(|i| p[i] + span[i]));
                }
                b.include(p);
                if let Some(motion) = &mut b.motion {
                    let mut pose = motion.frame().interpolation.pose;
                    pose.position = p;
                    motion
                        .set_model_pose(pose)
                        .map_err(|_| permanent("contact scope bed placement unavailable"))?;
                }
                b.extend();
            }
        }
        K::Equipment {
            entries: EntityHistoryValue::Retained { .. },
            complete: true,
            ..
        } => {}
        K::Scale {
            attribute: EntityHistoryValue::Retained { value },
            ..
        } => {
            if !value.base.is_finite()
                || value.modifiers.len() > 16
                || value.modifiers.iter().any(|m| !m.amount.is_finite())
            {
                return Err(permanent("contact scope scale unknown"));
            }
            let mods = value.modifiers.iter().collect::<Vec<_>>();
            if crate::client::physics::attribute_value(version, value.base, &mods)
                .clamp(0.0625, 16.) as f32
                != 1.
            {
                return Err(permanent("contact scope requires audited unit scale"));
            }
        }
        K::Status { .. } | K::Animation { .. }
            if b.name
                .as_deref()
                .is_some_and(|n| contact_policy::harmless_packet(version, n, &r.kind)) => {}
        _ => return Err(permanent("contact scope callback/history unavailable")),
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub(super) struct PlayerScope {
    proof: ScopeReceipts,
    session: SessionStamp,
    own: i32,
    cursor: EntityHistoryCursor,
    excluded: Vec<EntityId>,
    omitted: Vec<EntityId>,
    groups: Vec<Group>,
    query: Aabb,
    own_position: [f64; 3],
    cap: usize,
}
impl PlayerScope {
    pub(super) fn new(
        session: SessionStamp,
        own: i32,
        ledger: &SpawnLedger,
        sequence: u64,
        body: &Body,
        environment: &crate::client::physics::Environment,
        cap: usize,
    ) -> Result<Self> {
        let (proof, cursor) = ScopeReceipts::capture(session, own, ledger, sequence)?;
        let mut value = Self {
            proof,
            session,
            own,
            cursor,
            excluded: Vec::new(),
            omitted: Vec::new(),
            groups: Vec::new(),
            query: body_query(session.version, body, environment)?,
            own_position: body.position,
            cap,
        };
        value.select(0)?;
        Ok(value)
    }
    pub(super) fn excludes(&self, id: EntityId) -> bool {
        self.excluded.contains(&id)
    }
    pub(super) fn refresh(&mut self, ledger: &SpawnLedger, sequence: u64, tick: u64) -> Result<()> {
        loop {
            let page = ledger.history_after(
                self.session,
                sequence,
                Some(self.cursor),
                MAX_ENTITY_HISTORY_READ,
            )?;
            if page.gap.is_some() || page.session != self.session {
                return Err(permanent("contact scope history gap/world changed"));
            }
            for record in &page.records {
                self.proof.apply(record, self.own);
            }
            self.cursor = page.next_cursor;
            if !page.has_more {
                break;
            }
        }
        self.omitted
            .retain(|id| self.proof.members.values().any(|b| b.id == *id));
        self.select(tick)
    }
    pub(super) fn prepare(
        &mut self,
        body: &Body,
        environment: &crate::client::physics::Environment,
    ) -> Result<()> {
        self.query = body_query(self.session.version, body, environment)?;
        self.own_position = body.position;
        Ok(())
    }
    pub(super) fn stage(
        &mut self,
        body: &Body,
        environment: &crate::client::physics::Environment,
        tick: u64,
    ) -> Result<()> {
        self.prepare(body, environment)?;
        self.select(tick)
    }
    fn select(&mut self, tick: u64) -> Result<()> {
        self.groups = self.proof.groups()?;
        let mut selected = BTreeSet::new();
        // Fixed-point graph closure: retain whole rider groups and every first
        // contact edge, independent of native order or propagation chain length.
        loop {
            let mut changed = false;
            let count: usize = selected
                .iter()
                .map(|&i: &usize| self.groups[i].ids.len())
                .sum();
            let query = expand(self.query, count as f64 * 0.1);
            for (i, g) in self.groups.iter().enumerate() {
                if selected.contains(&i) {
                    continue;
                }
                let Some(envelope) = g.envelope else {
                    continue;
                };
                if intersects(query, envelope)
                    || selected.iter().any(|&j| {
                        self.groups[j]
                            .envelope
                            .is_some_and(|peer| intersects(envelope, peer))
                    })
                {
                    selected.insert(i);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let mut count = 0;
        self.excluded.clear();
        for (i, group) in self.groups.iter().enumerate() {
            for id in &group.ids {
                let body = &self.proof.members[id];
                if selected.contains(&i) {
                    if group.mounted {
                        return Err(body.error("living passenger group reaches player contact scope; separate contact origin required"));
                    }
                    if !body
                        .name
                        .as_deref()
                        .is_some_and(|n| audited_type(self.session.version, n))
                    {
                        return Err(body.error(
                            "finite exclusion proof does not qualify nearby numerical contact",
                        ));
                    }
                    if tick != 0 && self.omitted.contains(&body.id) {
                        return Err(body.error("contact scope re-entry after omitted model ticks; start a new control owner"));
                    }
                    count += 1;
                    if count > self.cap {
                        return Err(body
                            .error("TooManyBodies in player contact scope (max_contact_bodies)"));
                    }
                } else {
                    self.excluded.push(body.id);
                    if !self.omitted.contains(&body.id) {
                        self.omitted.push(body.id);
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn annotate(&self, error: &mut MovementRefusal) {
        if error.context().native_entity_id.is_none()
            && error.geometry().is_some_and(|g| g.block_position.is_some())
        {
            let c = error.context_mut();
            c.native_entity_id = Some(self.own);
            c.entity_type = Some("minecraft:player".into());
            error.geometry_mut().entity_position = Some(self.own_position);
        }
        let native = error.context().native_entity_id;
        if let Some(b) = native.and_then(|id| self.proof.members.get(&id)) {
            let c = error.context_mut();
            c.entity_type = c.entity_type.clone().or_else(|| b.name.clone());
            let g = error.geometry_mut();
            g.entity_position = g.entity_position.or(b.position);
            if g.block_position.is_none() {
                if let Some(p) = b.position.filter(|p| p.iter().all(|x| x.is_finite())) {
                    let p = p.map(|x| x.floor() as i32);
                    g.block_position = Some(p);
                    g.chunk_position = Some([p[0].div_euclid(16), p[2].div_euclid(16)]);
                }
            }
        }
    }
    pub(super) fn check(&self, query: Aabb) -> Result<()> {
        for group in &self.groups {
            let Some(envelope) = group.envelope else {
                continue;
            };
            if group
                .ids
                .iter()
                .any(|id| self.excludes(self.proof.members[id].id))
                && intersects(query, envelope)
            {
                return Err(self.proof.members[&group.ids[0]].error(
                    "staged sweep reaches excluded contact envelope; start a new control owner",
                ));
            }
        }
        Ok(())
    }
}
fn body_query(
    version: crate::MinecraftVersion,
    body: &Body,
    environment: &crate::client::physics::Environment,
) -> Result<Aabb> {
    if body
        .bounds
        .iter()
        .chain(body.velocity.iter())
        .any(|x| !x.is_finite())
    {
        return Err(permanent("player contact scope numeric input invalid"));
    }
    let b = Aabb {
        min_x: body.bounds[0],
        min_y: body.bounds[1],
        min_z: body.bounds[2],
        max_x: body.bounds[3],
        max_y: body.bounds[4],
        max_z: body.bounds[5],
    };
    // Bound both sprint choices and every legal input direction using the same
    // received attribute arithmetic as physics. The ample coefficients include
    // fluid acceleration, flight, jump, step alternatives and glide redirection.
    // Actual attempted geometry and the completed sweep remain final guards.
    let sprint = crate::client::physics::Modifier {
        id: version.table().physics_rules.sprint_modifier.into(),
        operation: crate::client::physics::ModifierOperation::MultiplyTotal,
        amount: f64::from(0.3_f32),
    };
    let mut modifiers = environment
        .movement_speed_modifiers
        .iter()
        .collect::<Vec<_>>();
    let speed = crate::client::physics::attribute_value(
        version,
        environment.movement_speed_base,
        &modifiers,
    )
    .clamp(0., 1024.);
    modifiers.push(&sprint);
    let speed = speed.max(
        crate::client::physics::attribute_value(
            version,
            environment.movement_speed_base,
            &modifiers,
        )
        .clamp(0., 1024.),
    );
    let facts = [
        environment.movement_speed_base,
        environment.jump_strength,
        environment.step_height,
        environment.gravity,
        environment.sneaking_speed,
        environment.movement_efficiency,
        environment.water_movement_efficiency,
        f64::from(environment.ability_flying_speed.unwrap_or(0.)),
    ];
    if facts.iter().any(|v| !v.is_finite())
        || environment
            .movement_speed_modifiers
            .iter()
            .any(|m| !m.amount.is_finite())
    {
        return Err(permanent("player contact scope acceleration input invalid"));
    }
    let reach = 4.
        + body.velocity.iter().map(|v| v.abs()).sum::<f64>() * 4.
        + speed * 16.
        + environment.jump_strength.abs() * 4.
        + environment.step_height.abs() * 4.
        + environment.gravity.abs() * 4.
        + f64::from(environment.ability_flying_speed.unwrap_or(0.)).abs() * 16.
        + environment
            .jump_boost
            .map_or(0., |boost| f64::from(boost) * 0.4)
        + environment
            .levitation_amplifier
            .map_or(0., |amp| f64::from(amp).abs() * 0.2);
    if !reach.is_finite() {
        return Err(permanent("player contact scope reach overflow"));
    }
    Ok(expand(b, reach))
}
pub(super) fn block_error(error: crate::Error, p: [i32; 3]) -> crate::Error {
    MovementRefusal::terrain(error, p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::entity::{NativeSpawn, NativeSpawnMotion, owned_contacts::OwnedContacts};
    use crate::{
        MinecraftVersion as V,
        client::{control::Controls, physics::Environment},
    };
    const VERSIONS: [V; 2] = [V::Java1_16_1, V::Java1_21_11];
    fn stamp(version: V) -> SessionStamp {
        SessionStamp {
            version,
            connection_id: 9,
            world_generation: 3,
        }
    }
    fn spawn(
        ledger: &mut SpawnLedger,
        version: V,
        id: i32,
        name: &'static str,
        x: f64,
        sequence: u64,
    ) {
        ledger.contact_local_player(99);
        ledger.history_context(version, 3, sequence);
        ledger
            .insert(
                version,
                NativeSpawn {
                    id,
                    uuid: Some([id as u8; 16]),
                    type_id: None,
                    dedicated_type_name: Some(name),
                    position: [x, 65., 0.5],
                    living: Some(true),
                },
                sequence,
                4096,
            )
            .unwrap();
        ledger.initialize_motion(
            id,
            NativeSpawnMotion {
                position: Some([x, 65., 0.5]),
                rotation: Some([0.; 2]),
                head_yaw: Some(0.),
                velocity: Some([0.; 3]),
            },
            sequence,
        );
    }
    fn passengers(ledger: &mut SpawnLedger, version: V, sequence: u64, children: Vec<i32>) {
        ledger.history_context(version, 3, sequence);
        ledger.history_passengers(&crate::client::vehicle::NativePassengers {
            vehicle: 7,
            passengers: children,
        });
    }
    fn body() -> Body {
        let mut b = Body::new([0.5, 65., 0.5]);
        b.on_ground = true;
        b
    }
    fn owner(version: V, ledger: &SpawnLedger, sequence: u64, cap: usize) -> Result<OwnedContacts> {
        OwnedContacts::for_player(
            (stamp(version), 10, 99),
            ledger,
            sequence,
            (&body(), &Environment::defaults(version)),
            cap,
            false,
        )
    }
    fn state(_version: V, name: &str) -> crate::NativeBlockState {
        crate::NativeBlockState {
            name: name.into(),
            properties: Default::default(),
        }
    }
    fn tick(version: V, model: &mut OwnedContacts, b: &mut Body) -> Result<()> {
        model
            .tick_player(
                b,
                &Environment::defaults(version),
                Controls {
                    forward: 1,
                    ..Controls::default()
                },
                false,
                None,
                &mut |p| {
                    assert!(
                        p[0].abs() < 16 && p[2].abs() < 16,
                        "far terrain queried: {p:?}"
                    );
                    Ok(state(
                        version,
                        if p[1] < 65 {
                            "minecraft:stone"
                        } else {
                            "minecraft:air"
                        },
                    ))
                },
            )
            .map(|_| ())
    }
    #[test]
    fn proven_no_contact_classes_need_no_geometry_and_cannot_hide_a_rider() {
        for version in VERSIONS {
            let mut ledger = SpawnLedger::default();
            spawn(&mut ledger, version, 7, "bat", 0.9, 1);
            spawn(&mut ledger, version, 8, "armor_stand", 0.9, 2);
            ledger.history_context(version, 3, 3);
            ledger.receive_metadata(7, [], false, 3);
            passengers(&mut ledger, version, 4, vec![8]);
            let mut model = owner(version, &ledger, 4, 0).unwrap();
            tick(version, &mut model, &mut body()).unwrap();
            assert!(model.world.bodies.is_empty());
            passengers(&mut ledger, version, 5, vec![]);
            spawn(&mut ledger, version, 8, "cow", 128., 6);
            passengers(&mut ledger, version, 7, vec![8]);
            assert!(owner(version, &ledger, 7, 256).is_err());
        }
    }
    #[test]
    fn existing_default_spider_proof_only_excludes_and_revokes_on_metadata() {
        for version in VERSIONS {
            for name in ["spider", "cave_spider"] {
                let mut ledger = SpawnLedger::default();
                spawn(&mut ledger, version, 7, name, 128., 1);
                let mut model = owner(version, &ledger, 1, 0).unwrap();
                tick(version, &mut model, &mut body()).unwrap();
                let mut near = body();
                near.teleport(version, [128., 65., 0.5]);
                assert!(
                    OwnedContacts::for_player(
                        (stamp(version), 11, 99),
                        &ledger,
                        1,
                        (&near, &Environment::defaults(version)),
                        256,
                        false
                    )
                    .is_err()
                );
                ledger.history_context(version, 3, 2);
                ledger.receive_metadata(7, [], true, 2);
                assert!(owner(version, &ledger, 2, 256).is_err());
            }
        }
    }
    #[test]
    fn far_unloaded_entity_is_not_ticked_or_queried() {
        for version in VERSIONS {
            let mut ledger = SpawnLedger::default();
            spawn(&mut ledger, version, 7, "cow", 128., 1);
            let mut model = owner(version, &ledger, 1, 0).unwrap();
            let mut b = body();
            tick(version, &mut model, &mut b).unwrap();
            assert!(b.position[2] > 0.5);
            assert!(model.world.bodies.is_empty());
        }
    }
    #[test]
    fn far_group_is_excluded_atomically_and_near_group_has_typed_cause() {
        for version in VERSIONS {
            for x in [0.9, 64.] {
                let mut ledger = SpawnLedger::default();
                spawn(&mut ledger, version, 7, "chicken", x, 1);
                spawn(&mut ledger, version, 8, "zombie", x, 2);
                passengers(&mut ledger, version, 3, vec![8]);
                if x > 1. {
                    let mut model = owner(version, &ledger, 3, 0).unwrap();
                    tick(version, &mut model, &mut body()).unwrap();
                    assert!(model.world.bodies.is_empty());
                } else {
                    let e = owner(version, &ledger, 3, 256).unwrap_err();
                    let refusal = e.movement_refusal().unwrap();
                    let c = refusal.context();
                    let g = refusal.geometry().unwrap();
                    assert_eq!(c.native_entity_id, Some(7));
                    assert_eq!(c.entity_type.as_deref(), Some("minecraft:chicken"));
                    assert_eq!(g.entity_position, Some([x, 65., 0.5]));
                    assert_eq!(g.block_position, Some([0, 65, 0]));
                    assert_eq!(g.chunk_position, Some([0, 0]));
                }
            }
        }
    }
    #[test]
    fn transitive_contact_propagation_selects_the_entire_chain() {
        for version in VERSIONS {
            let mut ledger = SpawnLedger::default();
            // Touching packet envelopes form a path from own broad phase to
            // the distant group; the group's direct own test alone would miss.
            for id in 7..40 {
                spawn(
                    &mut ledger,
                    version,
                    id,
                    "cow",
                    f64::from(id - 7) * 3.,
                    id as u64,
                );
            }
            let model = owner(version, &ledger, 39, 64).unwrap();
            assert_eq!(model.world.bodies.len(), 33);
            assert!(
                owner(version, &ledger, 39, 8)
                    .unwrap_err()
                    .to_string()
                    .contains("TooManyBodies")
            );
        }
    }
    #[test]
    fn an_indirect_far_group_is_not_a_distance_exception() {
        for version in VERSIONS {
            let mut ledger = SpawnLedger::default();
            spawn(&mut ledger, version, 7, "chicken", 64., 1);
            spawn(&mut ledger, version, 8, "zombie", 64., 2);
            passengers(&mut ledger, version, 3, vec![8]);
            for id in 10..30 {
                spawn(
                    &mut ledger,
                    version,
                    id,
                    "cow",
                    f64::from(id - 10) * 3.,
                    id as u64,
                );
            }
            assert!(
                owner(version, &ledger, 29, 256)
                    .unwrap_err()
                    .to_string()
                    .contains("passenger")
            );
        }
    }
    #[test]
    fn cap_counts_only_scoped_bodies_and_is_configurable_above_256() {
        for version in VERSIONS {
            let mut ledger = SpawnLedger::default();
            for id in 1..=300 {
                if id != 99 {
                    spawn(&mut ledger, version, id, "cow", 128., id as u64);
                }
            }
            spawn(&mut ledger, version, 301, "cow", 0.9, 301);
            assert_eq!(
                owner(version, &ledger, 301, 1).unwrap().world.bodies.len(),
                1
            );
            assert!(
                owner(version, &ledger, 301, 0)
                    .unwrap_err()
                    .to_string()
                    .contains("TooManyBodies")
            );
            for id in 302..=560 {
                spawn(&mut ledger, version, id, "cow", 0.9, id as u64);
            }
            assert_eq!(
                owner(version, &ledger, 560, 300)
                    .unwrap()
                    .world
                    .bodies
                    .len(),
                260
            );
            assert!(owner(version, &ledger, 560, 256).is_err());
        }
    }
    #[test]
    fn dismount_and_id_reuse_recompute_before_a_write() {
        for version in VERSIONS {
            let mut ledger = SpawnLedger::default();
            spawn(&mut ledger, version, 7, "chicken", 64., 1);
            spawn(&mut ledger, version, 8, "zombie", 64., 2);
            passengers(&mut ledger, version, 3, vec![8]);
            let mut model = owner(version, &ledger, 3, 256).unwrap();
            tick(version, &mut model, &mut body()).unwrap();
            passengers(&mut ledger, version, 4, vec![]);
            model.refresh(10, &ledger, 4).unwrap();
            tick(version, &mut model, &mut body()).unwrap();
            // Reusing the carrier number close to us cannot inherit its old
            // exclusion, even without an explicit Removed receipt.
            spawn(&mut ledger, version, 7, "chicken", 0.9, 5);
            model.refresh(10, &ledger, 5).unwrap();
            assert_eq!(model.world.bodies.len(), 1);
            passengers(&mut ledger, version, 6, vec![8]);
            assert!(
                model
                    .refresh(10, &ledger, 6)
                    .unwrap_err()
                    .to_string()
                    .contains("passenger")
            );
            passengers(&mut ledger, version, 7, vec![]);
            // A fresh origin is usable on the same connection after dismount.
            assert!(owner(version, &ledger, 7, 256).is_ok());
        }
    }
    #[test]
    fn skipped_ticks_reentry_refuses_and_new_owner_can_restart() {
        for version in VERSIONS {
            let mut ledger = SpawnLedger::default();
            spawn(&mut ledger, version, 7, "cow", 32., 1);
            let mut model = owner(version, &ledger, 1, 256).unwrap();
            tick(version, &mut model, &mut body()).unwrap();
            let mut moved = body();
            moved.teleport(version, [32., 65., 0.5]);
            assert!(
                model
                    .tick_player(
                        &mut moved,
                        &Environment::defaults(version),
                        Controls::default(),
                        false,
                        None,
                        &mut |_| panic!("must refuse before terrain")
                    )
                    .is_err()
            );
            assert!(
                OwnedContacts::for_player(
                    (stamp(version), 11, 99),
                    &ledger,
                    1,
                    (&moved, &Environment::defaults(version)),
                    256,
                    false
                )
                .is_ok()
            );
        }
    }
    #[test]
    fn checkpoint_eviction_preserves_scope_and_relationships() {
        for version in VERSIONS {
            let mut ledger = SpawnLedger::default();
            spawn(&mut ledger, version, 7, "chicken", 64., 1);
            spawn(&mut ledger, version, 8, "zombie", 64., 2);
            passengers(&mut ledger, version, 3, vec![8]);
            for sequence in 4..=16400 {
                ledger.history_context(version, 3, sequence);
                ledger.receive_metadata(7, [(0, EntityDataValue::Byte(0))], true, sequence);
            }
            let mut model = owner(version, &ledger, 16400, 0).unwrap();
            tick(version, &mut model, &mut body()).unwrap();
            passengers(&mut ledger, version, 16401, vec![]);
            assert!(owner(version, &ledger, 16401, 0).is_ok());
        }
    }
    #[test]
    fn unknown_inputs_and_actual_motion_changes_never_gain_distance_exemptions() {
        use crate::client::entity::NativeMotion;
        for version in VERSIONS {
            for bad in [
                "velocity", "age", "health", "callback", "history", "type", "rider",
            ] {
                let mut ledger = SpawnLedger::default();
                spawn(
                    &mut ledger,
                    version,
                    7,
                    if bad == "type" { "horse" } else { "cow" },
                    128.,
                    1,
                );
                ledger.history_context(version, 3, 2);
                match bad {
                    "velocity" => ledger.receive_motion(
                        version,
                        7,
                        NativeMotion::Velocity([f64::NAN, 0., 0.]),
                        2,
                    ),
                    "age" => {
                        let field = version
                            .table()
                            .entity_data("minecraft:cow")
                            .iter()
                            .find(|r| {
                                EntityDataField::Baby
                                    .official_names()
                                    .iter()
                                    .any(|&(o, n)| r.owner == o && r.field == n)
                            })
                            .unwrap()
                            .index;
                        ledger.receive_metadata(7, [(field, EntityDataValue::Int(1))], true, 2);
                    }
                    "health" => {
                        let field = version
                            .table()
                            .entity_data("minecraft:cow")
                            .iter()
                            .find(|r| {
                                EntityDataField::Health
                                    .official_names()
                                    .iter()
                                    .any(|&(o, n)| r.owner == o && r.field == n)
                            })
                            .unwrap()
                            .index;
                        ledger.receive_metadata(7, [(field, EntityDataValue::Int(1))], true, 2);
                    }
                    "callback" => ledger.history_signal(7, Some(127), None),
                    "history" => {
                        ledger.receive_metadata(7, [(6, EntityDataValue::Pose(0))], false, 2)
                    }
                    "rider" => passengers(&mut ledger, version, 2, vec![1234]),
                    _ => {}
                }
                assert!(
                    owner(version, &ledger, 2, 256).is_err(),
                    "{version:?}/{bad}"
                );
            }
            let mut ledger = SpawnLedger::default();
            spawn(&mut ledger, version, 7, "cow", 64., 1);
            let mut model = owner(version, &ledger, 1, 256).unwrap();
            tick(version, &mut model, &mut body()).unwrap();
            ledger.history_context(version, 3, 2);
            ledger.receive_motion(
                version,
                7,
                NativeMotion::Absolute {
                    position: [0.9, 65., 0.5],
                    rotation: [0.; 2],
                    velocity: (version == V::Java1_21_11).then_some([0.; 3]),
                    ground: Some(true),
                    reset_base: true,
                },
                2,
            );
            assert!(
                model
                    .refresh(10, &ledger, 2)
                    .unwrap_err()
                    .to_string()
                    .contains("re-entry")
            );
            // The original owner was invalidated even though its staged proof
            // failed before applying the numerical receipt suffix.
            assert!(model.refresh(10, &ledger, 2).is_err());
        }
    }
    #[test]
    fn active_history_eviction_and_world_change_refuse_before_write() {
        for version in VERSIONS {
            let mut ledger = SpawnLedger::default();
            spawn(&mut ledger, version, 7, "cow", 128., 1);
            let mut model = owner(version, &ledger, 1, 0).unwrap();
            for sequence in 2..=8200 {
                ledger.history_context(version, 3, sequence);
                ledger.receive_metadata(7, [(0, EntityDataValue::Byte(0))], true, sequence);
            }
            assert!(model.refresh(10, &ledger, 8200).is_err());
            let mut fresh = owner(version, &ledger, 8200, 0).unwrap();
            ledger.history_context(version, 4, 8201);
            assert!(fresh.refresh(10, &ledger, 8201).is_err());
        }
    }
    #[test]
    fn sleeping_displacement_encloses_the_shifted_interpolation_target() {
        use crate::client::entity::NativeMotion;
        for version in VERSIONS {
            let mut ledger = SpawnLedger::default();
            spawn(&mut ledger, version, 7, "cow", 64., 1);
            ledger.history_context(version, 3, 2);
            ledger.receive_motion(
                version,
                7,
                NativeMotion::Absolute {
                    position: [128., 65., 0.5],
                    rotation: [0.; 2],
                    velocity: (version == V::Java1_21_11).then_some([0.; 3]),
                    ground: Some(true),
                    reset_base: true,
                },
                2,
            );
            let index = version
                .table()
                .entity_data("minecraft:cow")
                .iter()
                .find(|r| {
                    EntityDataField::SleepingPosition
                        .official_names()
                        .iter()
                        .any(|&(o, n)| r.owner == o && r.field == n)
                })
                .unwrap()
                .index;
            ledger.history_context(version, 3, 3);
            ledger.receive_metadata(
                7,
                [(
                    index,
                    EntityDataValue::OptionalBlockPosition(Some(crate::client::BlockPos {
                        x: 96,
                        y: 65,
                        z: 0,
                    })),
                )],
                true,
                3,
            );
            let mut local = body();
            local.teleport(version, [160., 65., 0.5]);
            let model = OwnedContacts::for_player(
                (stamp(version), 11, 99),
                &ledger,
                3,
                (&local, &Environment::defaults(version)),
                256,
                false,
            )
            .unwrap();
            assert_eq!(
                model.world.bodies.len(),
                1,
                "bed-shifted target must not be excluded"
            );
        }
    }
    #[test]
    fn missing_near_terrain_identifies_entity_and_cell() {
        for version in VERSIONS {
            let mut ledger = SpawnLedger::default();
            spawn(&mut ledger, version, 7, "cow", 6.9, 1);
            let mut model = owner(version, &ledger, 1, 256).unwrap();
            let e = model
                .tick_player(
                    &mut body(),
                    &Environment::defaults(version),
                    Controls::default(),
                    false,
                    None,
                    &mut |p| {
                        if p[0] >= 6 {
                            Err(permanent("motion geometry is not loaded"))
                        } else {
                            Ok(state(version, "minecraft:air"))
                        }
                    },
                )
                .unwrap_err();
            let refusal = e.movement_refusal().unwrap();
            let c = refusal.context();
            let g = refusal.geometry().unwrap();
            assert_eq!(c.native_entity_id, Some(7));
            assert_eq!(g.entity_position, Some([6.9, 65., 0.5]));
            assert!(g.block_position.unwrap()[0] >= 6);
            assert_eq!(g.chunk_position, Some([0, 0]));
        }
    }
}
