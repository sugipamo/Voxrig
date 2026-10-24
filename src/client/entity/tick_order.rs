//! Original client entity iteration at an explicitly chosen model tick.
use crate::MinecraftVersion;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Fixed active-body limit. A modern snapshot can additionally retain that many
/// retired bodies until its iteration finishes; no historical tombstones grow.
pub const MAX_LIVING_TICK_BODIES: usize = 256;

/// Explicit numerical predicates. These are not inferred from receive order.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct LivingTickBody {
    /// Monotonically fresh numerical lifetime key, never a live EntityId.
    pub key: u64,
    /// Original numeric map/list key, including zero and negative identifiers.
    pub native_id: i32,
    /// Original player class predicate.
    pub player: bool,
    /// Original removal state. Removed bodies cannot be revived in place.
    pub removed: bool,
    /// Original vehicle key, separate from ordered passenger membership.
    pub passenger_of: Option<u64>,
    /// Explicit original passenger order.
    pub passengers: Vec<u64>,
    /// Original legacy chunk ticking predicate. None refuses required dispatch.
    pub legacy_ticking_chunk: Option<bool>,
    /// Original legacy inChunk predicate. False requires a separate unqualified
    /// chunk-update stage and is refused when it could affect dispatch.
    pub legacy_in_chunk: Option<bool>,
    /// Original countPlayerPassengers result for the modern frozen guard.
    pub player_passengers: u32,
}

/// One native dispatch stage; ride dispatch does not mean passenger travel is implemented.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum LivingTickDispatch {
    /// Ordinary non-passenger callback.
    Tick,
    /// Passenger callback; travel remains a separate physics stage.
    Ride,
}
/// Fixed-size numerical dispatch with original lifetime key.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct LivingTickInstruction {
    /// Original numerical body lifetime.
    pub key: u64,
    /// Native map/list identifier.
    pub native_id: i32,
    /// Distinct ordinary tick or ride callback.
    pub dispatch: LivingTickDispatch,
}
/// Refusal of an incomplete or unqualified model schedule.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum LivingTickUnavailable {
    /// Explicit lifetime, cancellation or history gap invalidation.
    Invalidated,
    /// A new tick cannot nest in a still-active original iterator.
    TickAlreadyActive,
    /// Finish/next was requested without an active tick.
    NoActiveTick,
    /// Earlier instructions remain; a tick cannot silently skip them.
    TickNotFinished,
    /// Original legacy iterator mutation is outside this admitted lane.
    UnsupportedLegacyMutation,
    /// Missing original chunk/visibility input.
    MissingPredicate,
    /// Original chunk update could alter inChunk/passenger dispatch.
    UnsupportedChunkUpdate,
    /// Required passenger body or native parent/membership relation is missing.
    IncompletePassengers,
    /// Cyclic, excessive, duplicate or inconsistent membership inputs.
    InconsistentIdentity,
    /// Active or retained-body/dispatch budgets are exhausted.
    TooManyBodies,
}

/// Pure numerical original-order iterator. The caller admits complete original
/// membership and chooses a boundary after explicit packet application. Modern
/// mutations preserve snapshot body references; numerical IDs never substitute
/// a replacement lifetime into the old iterator. No clock or movement I/O runs.
#[derive(Clone, Debug)]
pub struct LivingEntityTickOrder {
    version: MinecraftVersion,
    valid: bool,
    fresh: u64,
    active: BTreeMap<i32, u64>,
    bodies: BTreeMap<u64, LivingTickBody>,
    legacy: LegacyMap,
    linked: Vec<i32>,
    iterator: Option<VecDeque<u64>>,
    legacy_iterator: Option<LegacyIteration>,
    current: Option<u64>,
    pending: VecDeque<(u64, Option<u64>, usize)>,
    frozen: bool,
    dispatched: usize,
    limit: usize,
}
impl LivingEntityTickOrder {
    /// Create an empty original-default map/list, preserving its capacity history.
    pub fn new(version: MinecraftVersion) -> Self {
        Self {
            version,
            valid: true,
            fresh: 0,
            active: BTreeMap::new(),
            bodies: BTreeMap::new(),
            legacy: LegacyMap::new(),
            linked: Vec::new(),
            iterator: None,
            legacy_iterator: None,
            current: None,
            pending: VecDeque::new(),
            frozen: false,
            dispatched: 0,
            limit: MAX_LIVING_TICK_BODIES,
        }
    }
    // Receipt checkpoints and scoped owners retain native slots separately from
    // the configurable numerical contact-body budget. Public models stay fixed.
    pub(crate) fn receipt_slots(version: MinecraftVersion) -> Self {
        let mut value = Self::new(version);
        value.limit = usize::MAX / 2;
        value
    }
    pub(crate) fn restore_default_budget(&mut self) -> Result<(), LivingTickUnavailable> {
        self.limit = MAX_LIVING_TICK_BODIES;
        if self.active.len() > self.limit || self.bodies.len() > self.limit * 2 {
            return Err(LivingTickUnavailable::TooManyBodies);
        }
        Ok(())
    }
    fn ready(&self) -> Result<(), LivingTickUnavailable> {
        if self.valid {
            Ok(())
        } else {
            Err(LivingTickUnavailable::Invalidated)
        }
    }
    fn can_mutate(&self) -> Result<(), LivingTickUnavailable> {
        self.ready()?;
        if self.version == MinecraftVersion::Java1_16_1 && self.iterator.is_some() {
            Err(LivingTickUnavailable::UnsupportedLegacyMutation)
        } else {
            Ok(())
        }
    }
    /// Admit a fresh model body. Replacing an occupied numeric ID keeps its
    /// original map/list slot; explicit removal then admission changes order.
    pub fn admit(&mut self, body: LivingTickBody) -> Result<(), LivingTickUnavailable> {
        use LivingTickUnavailable as U;
        self.can_mutate()?;
        if body.key <= self.fresh
            || body.passengers.len() > self.limit
            || body
                .passengers
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len()
                != body.passengers.len()
            || body.passengers.contains(&body.key)
            || body.passenger_of == Some(body.key)
        {
            return Err(U::InconsistentIdentity);
        }
        if (self.active.len() >= self.limit && !self.active.contains_key(&body.native_id))
            || self.bodies.len() >= 2 * self.limit
        {
            return Err(U::TooManyBodies);
        }
        if !self.active.contains_key(&body.native_id) {
            self.legacy.insert(body.native_id);
            self.linked.push(body.native_id);
        }
        self.fresh = body.key;
        self.active.insert(body.native_id, body.key);
        self.bodies.insert(body.key, body);
        self.collect();
        Ok(())
    }
    /// Remove tick membership. Modern old snapshot references remain alive
    /// unless their removal predicate is explicitly set; no receipt is invented.
    pub fn remove_membership(&mut self, native_id: i32) -> Result<bool, LivingTickUnavailable> {
        self.can_mutate()?;
        if self.active.remove(&native_id).is_none() {
            return Ok(false);
        }
        self.legacy.remove(native_id, true);
        self.linked.retain(|id| *id != native_id);
        self.collect();
        Ok(true)
    }
    /// Project an unticked checkpoint body into a scope that ignores its
    /// living dispatch. Membership, probe slots and lifetime keys stay intact.
    pub(super) fn bootstrap_placeholder(&mut self, key: u64) -> Result<(), LivingTickUnavailable> {
        self.can_mutate()?;
        if self.iterator.is_some() || self.current.is_some() || self.frozen {
            return Err(LivingTickUnavailable::InconsistentIdentity);
        }
        let body = self
            .bodies
            .get_mut(&key)
            .ok_or(LivingTickUnavailable::InconsistentIdentity)?;
        body.player = false;
        Ok(())
    }

    /// Read a retained numerical body (including modern snapshot retirees).
    pub fn body(&self, key: u64) -> Option<&LivingTickBody> {
        self.bodies.get(&key)
    }
    /// Replace predicates for the same lifetime. Original removed lifetimes
    /// cannot be revived; changing an ID or key requires fresh admission.
    pub fn update(&mut self, body: LivingTickBody) -> Result<(), LivingTickUnavailable> {
        use LivingTickUnavailable as U;
        self.can_mutate()?;
        let old = self.bodies.get(&body.key).ok_or(U::InconsistentIdentity)?;
        if old.native_id != body.native_id
            || old.player != body.player
            || (old.removed && !body.removed)
            || body.passengers.len() > self.limit
            || body
                .passengers
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len()
                != body.passengers.len()
            || body.passengers.contains(&body.key)
            || body.passenger_of == Some(body.key)
        {
            return Err(U::InconsistentIdentity);
        }
        self.bodies.insert(body.key, body);
        Ok(())
    }
    /// Irreversibly stop a partial iteration on lifetime/history loss or cancel.
    pub fn invalidate(&mut self) {
        self.valid = false;
        self.iterator = None;
        self.legacy_iterator = None;
        self.current = None;
        self.pending.clear();
        self.bodies.clear();
        self.active.clear();
        self.linked.clear();
    }
    /// Capture an explicit model tick boundary. Freeze is original modern
    /// runGameElements state, never inferred from wall time or packet ordinal.
    pub fn begin_tick(&mut self, frozen: bool) -> Result<(), LivingTickUnavailable> {
        use LivingTickUnavailable as U;
        self.ready()?;
        if self.iterator.is_some() {
            return Err(U::TickAlreadyActive);
        }
        if frozen && self.version == MinecraftVersion::Java1_16_1 {
            return Err(U::MissingPredicate);
        }
        let ids = if self.version == MinecraftVersion::Java1_16_1 {
            self.legacy.order()
        } else {
            self.linked.clone()
        };
        self.iterator = Some(ids.into_iter().map(|id| self.active[&id]).collect());
        if self.version == MinecraftVersion::Java1_16_1 {
            self.legacy_iterator = Some(LegacyIteration::new(&self.legacy));
        }
        self.frozen = frozen;
        self.dispatched = 0;
        Ok(())
    }
    /// Return the next guarded original dispatch. Unknown/unsupported predicates
    /// halt the iteration; previously returned diagnostics are not rolled back.
    pub fn next_tick(&mut self) -> Result<Option<LivingTickInstruction>, LivingTickUnavailable> {
        self.ready()?;
        if self.iterator.is_none() {
            return Err(LivingTickUnavailable::NoActiveTick);
        }
        match self.next_inner() {
            Ok(v) => Ok(v),
            Err(e) => {
                self.valid = false;
                Err(e)
            }
        }
    }
    fn next_inner(&mut self) -> Result<Option<LivingTickInstruction>, LivingTickUnavailable> {
        use LivingTickUnavailable as U;
        let legacy = self.version == MinecraftVersion::Java1_16_1;
        loop {
            let (key, parent, depth) = if let Some(v) = self.pending.pop_front() {
                v
            } else {
                let key = if legacy {
                    let Some(id) = self.legacy_iterator.as_mut().unwrap().next(&self.legacy) else {
                        return Ok(None);
                    };
                    let key = *self.active.get(&id).ok_or(U::InconsistentIdentity)?;
                    self.iterator.as_mut().unwrap().retain(|v| *v != key);
                    key
                } else {
                    let Some(key) = self.iterator.as_mut().unwrap().pop_front() else {
                        return Ok(None);
                    };
                    key
                };
                (key, None, 0)
            };
            let b = self
                .bodies
                .get(&key)
                .ok_or(U::IncompletePassengers)?
                .clone();
            self.current = Some(key);
            if depth > 32 {
                return Err(U::InconsistentIdentity);
            }
            let ride = parent.is_some();
            if let Some(parent) = parent {
                if b.removed || b.passenger_of != Some(parent) {
                    continue;
                }
                let member = self.active.contains_key(&b.native_id);
                if !b.player
                    && if legacy {
                        !b.legacy_ticking_chunk.ok_or(U::MissingPredicate)?
                    } else {
                        !member
                    }
                {
                    continue;
                }
            } else {
                if b.passenger_of.is_some() {
                    continue;
                }
                if b.removed {
                    if legacy {
                        self.active.remove(&b.native_id);
                        self.legacy_iterator
                            .as_mut()
                            .unwrap()
                            .remove_current(&mut self.legacy);
                        self.linked.retain(|id| *id != b.native_id);
                    }
                    continue;
                }
                if legacy && !b.player && !b.legacy_ticking_chunk.ok_or(U::MissingPredicate)? {
                    continue;
                }
                if !legacy && self.frozen && !b.player && b.player_passengers == 0 {
                    continue;
                }
            }
            if legacy && !b.legacy_in_chunk.ok_or(U::MissingPredicate)? {
                return Err(U::UnsupportedChunkUpdate);
            }
            self.dispatched += 1;
            if self.dispatched > self.limit {
                return Err(U::TooManyBodies);
            }
            // Children run immediately after their original parent callback,
            // before the next root, in the explicitly supplied native list order.
            if self.pending.len().saturating_add(b.passengers.len()) > self.limit {
                return Err(U::TooManyBodies);
            }
            for child in b.passengers.iter().rev() {
                self.pending.push_front((*child, Some(key), depth + 1));
            }
            return Ok(Some(LivingTickInstruction {
                key,
                native_id: b.native_id,
                dispatch: if ride {
                    LivingTickDispatch::Ride
                } else {
                    LivingTickDispatch::Tick
                },
            }));
        }
    }
    /// Finish only after the iterator has drained. Retired snapshot references
    /// are released; original map capacity/order history remains for the next tick.
    pub fn finish_tick(&mut self) -> Result<(), LivingTickUnavailable> {
        self.ready()?;
        let it = self
            .iterator
            .as_ref()
            .ok_or(LivingTickUnavailable::NoActiveTick)?;
        if !it.is_empty() || !self.pending.is_empty() {
            return Err(LivingTickUnavailable::TickNotFinished);
        }
        self.iterator = None;
        self.legacy_iterator = None;
        self.current = None;
        self.collect();
        Ok(())
    }
    fn collect(&mut self) {
        let keep: BTreeMap<u64, ()> = self
            .active
            .values()
            .chain(self.iterator.iter().flatten())
            .chain(self.current.iter())
            .chain(self.pending.iter().map(|v| &v.0))
            .map(|key| (*key, ()))
            .collect();
        self.bodies.retain(|key, _| keep.contains_key(key));
    }
}

/// Own bounded implementation of original integer-map allocation/probe order.
/// This uses numerical algorithm facts, not original source or class artifacts.
#[derive(Clone, Debug)]
struct LegacyMap {
    slots: Vec<Option<i32>>,
    zero: bool,
    size: usize,
}

/// The legacy iterator scans the live probe table, including keys shifted
/// across its wrap by iterator removal. A copied order would dispatch those
/// keys too early and miss the original iterator's deferred wrapped entries.
#[derive(Clone, Debug)]
struct LegacyIteration {
    remaining: usize,
    position: usize,
    zero_pending: bool,
    wrapped: VecDeque<i32>,
    last: Option<LegacyLast>,
}
#[derive(Clone, Copy, Debug)]
enum LegacyLast {
    Zero,
    Slot(usize),
    Wrapped(i32),
}
impl LegacyIteration {
    fn new(map: &LegacyMap) -> Self {
        Self {
            remaining: map.size,
            position: map.slots.len(),
            zero_pending: map.zero,
            wrapped: VecDeque::new(),
            last: None,
        }
    }
    fn next(&mut self, map: &LegacyMap) -> Option<i32> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        if self.zero_pending {
            self.zero_pending = false;
            self.last = Some(LegacyLast::Zero);
            return Some(0);
        }
        while self.position > 0 {
            self.position -= 1;
            if let Some(id) = map.slots[self.position] {
                self.last = Some(LegacyLast::Slot(self.position));
                return Some(id);
            }
        }
        let id = self.wrapped.pop_front()?;
        self.last = Some(LegacyLast::Wrapped(id));
        Some(id)
    }
    fn remove_current(&mut self, map: &mut LegacyMap) {
        match self
            .last
            .take()
            .expect("root removal follows iterator next")
        {
            LegacyLast::Zero => {
                map.zero = false;
                map.size -= 1;
            }
            LegacyLast::Wrapped(id) => map.remove(id, true),
            LegacyLast::Slot(mut hole) => {
                let n = map.slots.len();
                map.slots[hole] = None;
                let mut scan = (hole + 1) & (n - 1);
                while let Some(candidate) = map.slots[scan] {
                    let home = LegacyMap::home(candidate, n);
                    if (scan + n - home) % n > (hole + n - home) % n {
                        if scan < hole {
                            self.wrapped.push_back(candidate);
                        }
                        map.slots[hole] = Some(candidate);
                        map.slots[scan] = None;
                        hole = scan;
                    }
                    scan = (scan + 1) & (n - 1);
                }
                map.size -= 1;
            }
        }
    }
}
impl LegacyMap {
    fn new() -> Self {
        Self {
            slots: vec![None; 32],
            zero: false,
            size: 0,
        }
    }
    fn home(key: i32, n: usize) -> usize {
        let h = (key as u32).wrapping_mul(0x9e3779b9);
        ((h ^ (h >> 16)) as usize) & (n - 1)
    }
    fn place(&mut self, key: i32) {
        let n = self.slots.len();
        let mut i = Self::home(key, n);
        while self.slots[i].is_some() {
            i = (i + 1) & (n - 1)
        }
        self.slots[i] = Some(key);
    }
    fn insert(&mut self, key: i32) {
        if key == 0 {
            if self.zero {
                return;
            }
            self.zero = true;
        } else {
            self.place(key);
        }
        let before = self.size;
        self.size += 1;
        if before >= self.slots.len() * 3 / 4 {
            let mut n = 32;
            while n * 3 / 4 < self.size + 1 {
                n *= 2
            }
            self.rehash(n);
        }
    }
    fn rehash(&mut self, n: usize) {
        let old = std::mem::replace(&mut self.slots, vec![None; n]);
        for key in old.into_iter().rev().flatten() {
            self.place(key);
        }
    }
    fn remove(&mut self, key: i32, shrink: bool) {
        let n = self.slots.len();
        if key == 0 {
            if !self.zero {
                return;
            }
            self.zero = false;
        } else {
            let mut hole = Self::home(key, n);
            loop {
                match self.slots[hole] {
                    Some(found) if found == key => break,
                    Some(_) => hole = (hole + 1) & (n - 1),
                    None => return,
                }
            }
            self.slots[hole] = None;
            let mut scan = (hole + 1) & (n - 1);
            while let Some(candidate) = self.slots[scan] {
                let home = Self::home(candidate, n);
                if (scan + n - home) % n > (hole + n - home) % n {
                    self.slots[hole] = Some(candidate);
                    self.slots[scan] = None;
                    hole = scan;
                }
                scan = (scan + 1) & (n - 1);
            }
        }
        self.size -= 1;
        if shrink && n > 32 && self.size < (n * 3 / 4) / 4 {
            self.rehash(n / 2);
        }
    }
    fn order(&self) -> Vec<i32> {
        let mut out = Vec::with_capacity(self.size);
        if self.zero {
            out.push(0)
        }
        out.extend(self.slots.iter().rev().flatten().copied());
        out
    }
}
