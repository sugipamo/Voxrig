//! Once-only receipt application, separate from explicit model tick selection.
use super::{
    EntityHistory, EntityHistoryKind, EntityHistoryRecord, EntityId, EntityMotionObservation,
    LivingInterpolationPose, LivingPacketFrame, LivingPacketGuards, LivingPacketModel,
    LivingPacketUnavailable, MAX_ENTITY_HISTORY_READ,
};

/// Packet replay refusal; lifetime/input failures irreversibly stop continuation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum LivingReplayUnavailable {
    /// Replay was already invalidated or cancelled.
    Invalidated,
    /// Initial input is not a complete original spawn motion record.
    MissingSpawn,
    /// Entity class has no qualified inherited-living dispatch in this lane.
    UnsupportedEntity,
    /// Connection, world or same-numeric-ID spawn lifetime changed.
    LifetimeChanged,
    /// Complete ordered history is unavailable or discontinuous.
    HistoryGap,
    /// A page exceeds the fixed native history read budget.
    PageTooLarge,
    /// Original operation/component provenance is missing or inconsistent.
    MissingPacket,
    /// Own-player correction requires an explicit new model fence.
    CorrectionFence,
    /// Selected entity status/animation semantics are outside this isolated lane.
    UnsupportedEntityEvent,
    /// Incomplete retained geometry/equipment inputs must stop consumers before travel.
    IncompleteEntityInput,
    /// Original motion dispatch or numerical input could not be admitted.
    Packet(LivingPacketUnavailable),
}

/// Fixed-size read-only replay diagnostics. None of these fields is a live target.
#[derive(Clone, Debug, serde::Serialize)]
pub struct LivingReplayFrame {
    /// Original received entity lifetime, readable after invalidation.
    pub entity: EntityId,
    /// Last applied record boundary, including unrelated records that were visited.
    pub applied_record: u64,
    /// Irreversible continuation status.
    pub valid: bool,
    /// Separate numerical packet/interpolation model.
    pub model: LivingPacketFrame,
}

/// Fixed-size replay bound to an actual original spawn. Every complete history
/// record is visited once, but only this lifetime's original motion operations
/// affect its model. Pages do not choose native ticks, fill missing predicates,
/// compute collision queries, simulate travel, or confer send authority.
/// The caller must invalidate on client revocation/closure and discarded owner
/// fences; packet history remains readable after those terminal boundaries.
#[derive(Clone, Debug)]
pub struct LivingEntityPacketReplay {
    entity: EntityId,
    applied: u64,
    valid: bool,
    model: LivingPacketModel,
}
impl LivingEntityPacketReplay {
    /// Admit a complete retained spawn record for cow/pig/zombie. Original spawn
    /// coordinates initialize numerical feet/base; later targets cannot replace
    /// this baseline. Local/remote player overrides require a separate lane.
    pub fn from_spawn(record: &EntityHistoryRecord) -> Result<Self, LivingReplayUnavailable> {
        use LivingReplayUnavailable as U;
        let EntityHistoryKind::Spawn(m) = &record.kind else {
            return Err(U::MissingSpawn);
        };
        if m.entity.id.session() != record.session
            || record.ordinal == 0
            || record.receive_sequence != m.entity.id.spawn_sequence()
            || m.packet.is_some()
        {
            return Err(U::MissingSpawn);
        }
        if !matches!(
            m.entity.type_name.as_deref(),
            Some("minecraft:cow" | "minecraft:pig" | "minecraft:zombie")
        ) {
            return Err(U::UnsupportedEntity);
        }
        let p = m.position.as_ref().ok_or(U::MissingSpawn)?;
        let r = m.rotation.as_ref().ok_or(U::MissingSpawn)?;
        if sequence(p.source) != Some(record.receive_sequence)
            || sequence(r.source) != Some(record.receive_sequence)
            || m.velocity
                .as_ref()
                .is_some_and(|v| sequence(v.source) != Some(record.receive_sequence))
            || m.head_yaw
                .as_ref()
                .is_some_and(|v| sequence(v.source) != Some(record.receive_sequence))
        {
            return Err(U::MissingSpawn);
        }
        let mut model = LivingPacketModel::new(
            record.session.version,
            LivingInterpolationPose {
                position: p.value.position,
                rotation: r.value,
            },
            record.ordinal,
        )
        .map_err(U::Packet)?;
        // Initial velocity/head are received samples, kept independently of feet.
        model
            .initialize_samples(
                m.velocity.as_ref().map(|v| v.value),
                m.head_yaw.as_ref().map(|v| v.value),
            )
            .map_err(U::Packet)?;
        Ok(Self {
            entity: m.entity.id,
            applied: record.ordinal,
            valid: true,
            model,
        })
    }
    /// Read numerical state and the original lifetime/record fence.
    pub fn frame(&self) -> LivingReplayFrame {
        LivingReplayFrame {
            entity: self.entity,
            applied_record: self.applied,
            valid: self.valid,
            model: self.model.frame(),
        }
    }
    /// Irreversibly stop on cancellation, closure, revocation or owner loss.
    pub fn invalidate(&mut self) {
        self.valid = false;
        self.model.invalidate();
    }
    fn ready(&self) -> Result<(), LivingReplayUnavailable> {
        if self.valid {
            Ok(())
        } else {
            Err(LivingReplayUnavailable::Invalidated)
        }
    }
    /// Replay a complete bounded page transactionally. Retransmitted old pages
    /// are no-ops; discontinuities, world changes, replacement spawns and
    /// unqualified dispatch stop the replay. No partially applied page escapes.
    /// The caller explicitly supplies original packet-application predicates;
    /// one page must not span a change in those predicates.
    pub fn apply_page(
        &mut self,
        page: &EntityHistory,
        guards: LivingPacketGuards,
    ) -> Result<usize, LivingReplayUnavailable> {
        self.ready()?;
        let mut next = self.clone();
        match next.apply_inner(page, guards) {
            Ok(n) => {
                *self = next;
                Ok(n)
            }
            Err(e) => {
                self.invalidate();
                Err(e)
            }
        }
    }
    fn apply_inner(
        &mut self,
        page: &EntityHistory,
        guards: LivingPacketGuards,
    ) -> Result<usize, LivingReplayUnavailable> {
        use LivingReplayUnavailable as U;
        if page.session != self.entity.session() {
            return Err(U::LifetimeChanged);
        }
        if page.gap.is_some() {
            return Err(U::HistoryGap);
        }
        if page.records.len() > MAX_ENTITY_HISTORY_READ {
            return Err(U::PageTooLarge);
        }
        let mut count = 0;
        let mut previous = 0;
        for record in &page.records {
            if record.ordinal <= previous {
                return Err(U::HistoryGap);
            }
            previous = record.ordinal;
            if record.ordinal <= self.applied {
                continue;
            }
            if record.ordinal != self.applied.checked_add(1).ok_or(U::HistoryGap)? {
                return Err(U::HistoryGap);
            }
            if record.session != self.entity.session() {
                return Err(U::LifetimeChanged);
            }
            match &record.kind {
                EntityHistoryKind::WorldChanged { .. } => return Err(U::LifetimeChanged),
                EntityHistoryKind::OwnPositionCorrection { .. } => return Err(U::CorrectionFence),
                EntityHistoryKind::Spawn(m)
                    if m.entity.id.native_id() == self.entity.native_id() =>
                {
                    return Err(U::LifetimeChanged);
                }
                EntityHistoryKind::Removed { native_id, .. }
                    if *native_id == self.entity.native_id() =>
                {
                    return Err(U::LifetimeChanged);
                }
                EntityHistoryKind::Motion(m)
                    if m.entity.id.native_id() == self.entity.native_id() =>
                {
                    self.motion(record, m, guards)?;
                    count += 1;
                }
                EntityHistoryKind::Metadata {
                    entity: Some(entity),
                    complete,
                    entries,
                    ..
                } if *entity == self.entity => {
                    if !complete || matches!(entries, super::EntityHistoryValue::RetentionLimit) {
                        return Err(U::IncompleteEntityInput);
                    }
                }
                EntityHistoryKind::Equipment {
                    entity: Some(entity),
                    complete,
                    entries,
                    ..
                } if *entity == self.entity => {
                    if !complete || matches!(entries, super::EntityHistoryValue::RetentionLimit) {
                        return Err(U::IncompleteEntityInput);
                    }
                }
                EntityHistoryKind::Scale {
                    entity: Some(entity),
                    attribute: super::EntityHistoryValue::RetentionLimit,
                    ..
                } if *entity == self.entity => return Err(U::IncompleteEntityInput),
                EntityHistoryKind::Status {
                    entity: Some(entity),
                    ..
                }
                | EntityHistoryKind::Animation {
                    entity: Some(entity),
                    ..
                } if *entity == self.entity => return Err(U::UnsupportedEntityEvent),
                _ => {}
            }
            self.applied = record.ordinal;
        }
        Ok(count)
    }
    fn motion(
        &mut self,
        record: &EntityHistoryRecord,
        m: &EntityMotionObservation,
        guards: LivingPacketGuards,
    ) -> Result<(), LivingReplayUnavailable> {
        use LivingReplayUnavailable as U;
        if m.entity.id != self.entity {
            return Err(U::LifetimeChanged);
        }
        let packet = m.packet.as_ref().ok_or(U::MissingPacket)?;
        if sequence(packet.source) != Some(record.receive_sequence)
            || m.receive_sequence != record.receive_sequence
        {
            return Err(U::MissingPacket);
        }
        self.model
            .apply(
                record.ordinal,
                packet
                    .value
                    .living_packet(self.entity.session().version)
                    .map_err(U::Packet)?,
                guards,
            )
            .map_err(U::Packet)?;
        Ok(())
    }
    /// Explicitly advance an already admitted interpolation stage. An actual
    /// original-style modern collision result is required while active. The
    /// stream fence does not advance and no original clock is inferred.
    pub fn step(
        &mut self,
        collision_free: Option<bool>,
    ) -> Result<LivingReplayFrame, LivingReplayUnavailable> {
        self.ready()?;
        self.model
            .step(collision_free)
            .map_err(LivingReplayUnavailable::Packet)?;
        Ok(self.frame())
    }
    /// Cancel an active model continuation without synthesizing a receipt.
    pub fn cancel(&mut self) -> Result<LivingReplayFrame, LivingReplayUnavailable> {
        self.ready()?;
        self.model
            .cancel()
            .map_err(LivingReplayUnavailable::Packet)?;
        Ok(self.frame())
    }
}

fn sequence(source: crate::client::ValueSource) -> Option<u64> {
    match source {
        crate::client::ValueSource::Received { sequence } => Some(sequence),
        _ => None,
    }
}
