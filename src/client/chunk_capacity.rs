//! Received-view column retention and explicit loss of cached evidence.
use super::{SessionStamp, WorldViewObservation};

/// Per-connection column budget. A column includes terrain, context and light;
/// a legacy light-only column consumes one unit too. Units are columns, not bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ChunkCapacity {
    /// Size from the received view, with a hard maximum number of retained columns.
    Auto {
        /// Maximum column budget, including light-only columns and teleport overlap.
        hard_memory_cap: usize,
    },
    /// Explicit maximum number of retained columns. Must fit the received view.
    Fixed(usize),
}
impl From<usize> for ChunkCapacity {
    fn from(columns: usize) -> Self {
        Self::Fixed(columns)
    }
}
impl Default for ChunkCapacity {
    fn default() -> Self {
        Self::Auto {
            hard_memory_cap: 16_384,
        }
    }
}
impl ChunkCapacity {
    pub(crate) fn maximum(self) -> usize {
        match self {
            Self::Auto { hard_memory_cap } => hard_memory_cap,
            Self::Fixed(n) => n,
        }
    }
    pub(crate) fn resolve(self, view: Option<i32>) -> Result<usize, ChunkCapacityError> {
        // One square plus a one-column light halo, twice for old/new teleport
        // views. Bootstrap without a received view reserves the minimum radius 2.
        let radius = usize::try_from(view.unwrap_or(2).max(2)).unwrap_or(usize::MAX);
        let needed = radius
            .checked_mul(2)
            .and_then(|n| n.checked_add(3))
            .and_then(|n| n.checked_mul(n))
            .and_then(|n| n.checked_mul(2))
            .unwrap_or(usize::MAX);
        let maximum = self.maximum();
        if needed > maximum || maximum == 0 {
            return Err(ChunkCapacityError {
                required: needed,
                capacity: maximum,
                view_distance: view,
            });
        }
        Ok(match self {
            Self::Auto { .. } => needed,
            Self::Fixed(n) => n,
        })
    }
}

/// The configured budget cannot preserve required received-view/body columns.
/// Refused before accepting the new column or evicting any protected data.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ChunkCapacityError {
    /// Minimum column count required by the received view and protection.
    pub required: usize,
    /// Available column budget.
    pub capacity: usize,
    /// Actual received view distance; None before the first view receipt.
    pub view_distance: Option<i32>,
}
impl std::fmt::Display for ChunkCapacityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "chunk capacity {} below required {} (received view {:?})",
            self.capacity, self.required, self.view_distance
        )
    }
}
impl std::error::Error for ChunkCapacityError {}

/// Last locally evicted column, with the policy at the time of loss.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ChunkEviction {
    /// Column coordinates.
    pub column: [i32; 2],
    /// Received view at eviction time.
    pub view_distance: Option<i32>,
    /// Effective column budget at eviction time.
    pub capacity: usize,
}
/// Coherent cache availability metrics. Compare both session and epoch before
/// reusing terrain evidence. Any epoch change invalidates previously held proofs;
/// immutable snapshots may still hold bytes, but those bytes are historical.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ChunkCacheObservation {
    /// Fences connection/world changes, including epoch reset.
    pub session: SessionStamp,
    /// Advances on unload, eviction and cache clear. Never a server world version.
    pub epoch: u64,
    /// Cumulative locally evicted columns in this world cache.
    pub evicted_columns: u64,
    /// Last eviction and the view/cap at that time.
    pub last_eviction: Option<ChunkEviction>,
    /// First capacity refusal, retained even if the transport reports a string error.
    pub refusal: Option<ChunkCapacityError>,
    /// Current number of retained columns, including light-only columns.
    pub columns: usize,
    /// Current effective column budget.
    pub capacity: usize,
    /// Latest received distance used by the policy.
    pub view_distance: Option<i32>,
}
#[derive(Default)]
pub(crate) struct Cache {
    epoch: u64,
    evicted: u64,
    last: Option<ChunkEviction>,
    capacity: usize,
    view: Option<i32>,
    refusal: Option<ChunkCapacityError>,
}
impl Cache {
    pub(crate) fn capture(&self, session: SessionStamp, columns: usize) -> ChunkCacheObservation {
        ChunkCacheObservation {
            session,
            epoch: self.epoch,
            evicted_columns: self.evicted,
            last_eviction: self.last.clone(),
            refusal: self.refusal.clone(),
            columns,
            capacity: self.capacity,
            view_distance: self.view,
        }
    }
    pub(crate) fn lost(&mut self) {
        self.epoch = self
            .epoch
            .checked_add(1)
            .expect("chunk cache epoch exhausted");
    }
    pub(crate) fn evicted(&mut self, column: [i32; 2]) {
        self.lost();
        self.evicted = self
            .evicted
            .checked_add(1)
            .expect("chunk eviction count exhausted");
        self.last = Some(ChunkEviction {
            column,
            view_distance: self.view,
            capacity: self.capacity,
        });
    }
    pub(crate) fn configure(
        &mut self,
        policy: ChunkCapacity,
        view: Option<i32>,
    ) -> Result<usize, ChunkCapacityError> {
        policy.resolve(view).inspect_err(|e| {
            self.refusal.get_or_insert_with(|| e.clone());
        })
    }
    pub(crate) fn plan(
        &mut self,
        policy: ChunkCapacity,
        view: &WorldViewObservation,
        body: Option<[i32; 2]>,
        columns: impl Iterator<Item = [i32; 2]>,
        incoming: Option<[i32; 2]>,
    ) -> Result<Vec<[i32; 2]>, ChunkCapacityError> {
        let distance = view.distance.as_ref().map(|r| r.value);
        let capacity = self.configure(policy, distance)?;
        let center = view.center.as_ref().map(|r| r.value);
        let radius = i64::from(distance.unwrap_or(2).max(2)) + 1;
        let keys: Vec<_> = columns.collect();
        let extra = usize::from(incoming.is_some_and(|p| !keys.contains(&p)));
        let excess = keys.len().saturating_add(extra).saturating_sub(capacity);
        let distance_to = |p: [i32; 2], q: [i32; 2]| {
            (i64::from(p[0]) - i64::from(q[0]))
                .abs()
                .max((i64::from(p[1]) - i64::from(q[1])).abs())
        };
        let mut candidates: Vec<_> = if excess == 0 {
            Vec::new()
        } else {
            keys.into_iter()
                .filter(|p| {
                    Some(*p) != incoming
                        && center.is_none_or(|c| distance_to(*p, c) > radius)
                        && body.is_none_or(|c| distance_to(*p, c) > 2)
                })
                .collect()
        };
        if candidates.len() < excess {
            let error = ChunkCapacityError {
                required: capacity.saturating_add(excess - candidates.len()),
                capacity,
                view_distance: distance,
            };
            self.refusal.get_or_insert_with(|| error.clone());
            return Err(error);
        }
        candidates.sort_unstable_by_key(|p| {
            let d = center
                .into_iter()
                .chain(body)
                .map(|c| distance_to(*p, c))
                .min()
                .unwrap_or(0);
            (std::cmp::Reverse(d), *p)
        });
        candidates.truncate(excess);
        self.capacity = capacity;
        self.view = distance;
        Ok(candidates)
    }
}
impl super::Client {
    /// Observe retained-column availability and loss metrics without sending.
    pub async fn chunk_cache(&self) -> crate::Result<ChunkCacheObservation> {
        super::dispatch!(&self.adapter, a => super::adapter::CoreOps::chunk_cache(a).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn view_budget_includes_light_halo_and_two_teleport_views() {
        assert_eq!(ChunkCapacity::default().resolve(Some(10)).unwrap(), 1058);
        assert_eq!(ChunkCapacity::default().resolve(Some(32)).unwrap(), 8978);
        let e = ChunkCapacity::Fixed(512).resolve(Some(10)).unwrap_err();
        assert_eq!((e.required, e.capacity), (1058, 512));
        assert!(ChunkCapacity::default().resolve(Some(i32::MAX)).is_err());
    }
    #[test]
    fn eviction_protects_both_received_view_and_body_before_mutating() {
        let mut cache = Cache::default();
        let view = WorldViewObservation {
            center: Some(super::super::received([0, 0], 1)),
            distance: Some(super::super::received(2, 1)),
            ..Default::default()
        };
        let mut columns: Vec<_> = (-3..=3)
            .flat_map(|x| (-3..=3).map(move |z| [x, z]))
            .collect();
        columns.extend((98..=102).flat_map(|x| (-2..=2).map(move |z| [x, z])));
        columns.extend((200..224).map(|x| [x, 0]));
        assert_eq!(columns.len(), 98);
        let removed = cache
            .plan(
                ChunkCapacity::Fixed(98),
                &view,
                Some([100, 0]),
                columns.iter().copied(),
                Some([500, 0]),
            )
            .unwrap();
        assert_eq!(removed, vec![[223, 0]]);
        cache.evicted(removed[0]);
        assert_eq!(cache.epoch, 1);
        assert_eq!(cache.last.as_ref().unwrap().capacity, 98);
        let e = cache
            .plan(
                ChunkCapacity::Fixed(97),
                &view,
                Some([100, 0]),
                columns.into_iter(),
                None,
            )
            .unwrap_err();
        assert_eq!(e.required, 98);
        assert_eq!(cache.epoch, 1);
    }
}
