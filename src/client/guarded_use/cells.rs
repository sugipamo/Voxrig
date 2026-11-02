//! Bounded receive ordinals of world cells for guarded-use read sets.
//!
//! Owned by each version's world store and mutated only where terrain is
//! applied, so loss of a column (unload, eviction, reset) also loses its
//! ordinals. Ordinals may be over-estimated (column reload, section floor) but
//! are never older than the last applied packet that set the cell's block state.
use std::collections::HashMap;

/// Touched cells kept per section; more collapse into a conservative floor.
const SECTION_CELLS: usize = 64;

#[derive(Clone, Default)]
pub(crate) struct CellRevisions {
    columns: HashMap<(i32, i32), Column>,
}
#[derive(Clone, Default)]
struct Column {
    /// Chunk data ordinal; 0 while the replacement is not yet stamped.
    loaded: u64,
    sections: HashMap<i32, Section>,
}
#[derive(Clone, Default)]
struct Section {
    floor: u64,
    cells: HashMap<u16, u64>,
}
fn column(p: [i32; 3]) -> (i32, i32) {
    (p[0].div_euclid(16), p[2].div_euclid(16))
}
fn index(p: [i32; 3]) -> u16 {
    (p[1].rem_euclid(16) * 256 + p[2].rem_euclid(16) * 16 + p[0].rem_euclid(16)) as u16
}
impl CellRevisions {
    /// Chunk data replaced all or part of a column. Earlier cell ordinals are
    /// dropped; the column reads as unknown until `stamp`.
    pub(crate) fn replaced(&mut self, position: [i32; 2]) {
        self.columns
            .insert((position[0], position[1]), Column::default());
    }
    /// Bind a pending replacement to the ordinal of the packet that carried it.
    pub(crate) fn stamp(&mut self, position: [i32; 2], sequence: u64) {
        if let Some(column) = self.columns.get_mut(&(position[0], position[1])) {
            if column.loaded == 0 {
                column.loaded = sequence;
            }
        }
    }
    pub(crate) fn forget(&mut self, position: [i32; 2]) {
        self.columns.remove(&(position[0], position[1]));
    }
    pub(crate) fn clear(&mut self) {
        self.columns.clear();
    }
    /// A received block state for this cell, changed or not. Unknown columns
    /// are ignored: they read as unknown until chunk data arrives.
    pub(crate) fn touch(&mut self, p: [i32; 3], sequence: u64) {
        let Some(column) = self.columns.get_mut(&column(p)) else {
            return;
        };
        let section = column.sections.entry(p[1].div_euclid(16)).or_default();
        let index = index(p);
        if section.cells.len() >= SECTION_CELLS && !section.cells.contains_key(&index) {
            section.floor = section
                .cells
                .drain()
                .map(|(_, s)| s)
                .fold(section.floor.max(sequence), u64::max);
            return;
        }
        let cell = section.cells.entry(index).or_default();
        *cell = (*cell).max(sequence);
    }
    /// Latest ordinal over every cell of the inclusive box `min..=max`.
    ///
    /// `None` if any overlapping column is not `loaded` (per the world store)
    /// or not yet stamped. Exact for kept cells; a column load and a section
    /// floor count for every cell they cover, so the result is never earlier
    /// than the real last touch. A single cell is the box `p..=p`. The caller
    /// checks Y against the dimension and bounds the box size: the cost is one
    /// visit per overlapping column plus its kept cells in overlapping sections.
    pub(crate) fn latest(
        &self,
        min: [i32; 3],
        max: [i32; 3],
        loaded: impl Fn((i32, i32)) -> bool,
    ) -> Option<u64> {
        let sections = min[1].div_euclid(16)..=max[1].div_euclid(16);
        let mut latest = 0;
        for x in min[0].div_euclid(16)..=max[0].div_euclid(16) {
            for z in min[2].div_euclid(16)..=max[2].div_euclid(16) {
                if !loaded((x, z)) {
                    return None;
                }
                let column = self.columns.get(&(x, z)).filter(|c| c.loaded != 0)?;
                latest = latest.max(column.loaded);
                for (&y, section) in &column.sections {
                    if !sections.contains(&y) {
                        continue;
                    }
                    latest = latest.max(section.floor);
                    for (&index, &sequence) in &section.cells {
                        let index = i32::from(index);
                        let p = [
                            x * 16 + index % 16,
                            y * 16 + index / 256,
                            z * 16 + index / 16 % 16,
                        ];
                        if (0..3).all(|a| (min[a]..=max[a]).contains(&p[a])) {
                            latest = latest.max(sequence);
                        }
                    }
                }
            }
        }
        Some(latest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl CellRevisions {
        fn revision(&self, p: [i32; 3]) -> Option<u64> {
            self.latest(p, p, |_| true)
        }
    }

    #[test]
    fn ordinals_follow_touches_reloads_and_loss() {
        let mut r = CellRevisions::default();
        r.touch([1, 2, 3], 4);
        assert_eq!(r.revision([1, 2, 3]), None, "no column, no ordinal");
        r.replaced([0, 0]);
        assert_eq!(r.revision([1, 2, 3]), None, "unstamped load is unknown");
        r.stamp([0, 0], 5);
        r.stamp([0, 0], 9);
        assert_eq!(r.revision([1, 2, 3]), Some(5));
        r.touch([1, 2, 3], 7);
        assert_eq!(r.revision([1, 2, 3]), Some(7));
        assert_eq!(r.revision([2, 2, 3]), Some(5), "neighbour cell untouched");
        assert_eq!(r.revision([1, 40, 3]), Some(5), "other section untouched");
        r.replaced([0, 0]);
        r.stamp([0, 0], 8);
        assert_eq!(r.revision([1, 2, 3]), Some(8), "reload supersedes cells");
        r.forget([0, 0]);
        assert_eq!(r.revision([1, 2, 3]), None);
        r.replaced([-1, -1]);
        r.stamp([-1, -1], 2);
        r.touch([-1, -1, -1], 3);
        assert_eq!(r.revision([-1, -1, -1]), Some(3));
        assert_eq!(r.revision([-16, -1, -1]), Some(2));
        r.clear();
        assert_eq!(r.revision([-1, -1, -1]), None);
    }

    #[test]
    fn full_section_collapses_into_a_conservative_floor() {
        let mut r = CellRevisions::default();
        r.replaced([0, 0]);
        r.stamp([0, 0], 1);
        for i in 0..SECTION_CELLS as i32 {
            r.touch([i % 16, 0, i / 16], 10 + i as u64);
        }
        r.touch([0, 20, 0], 500);
        let last = 10 + SECTION_CELLS as u64 - 1;
        assert_eq!(r.revision([0, 0, 0]), Some(10));
        r.touch([15, 15, 15], last + 1);
        for p in [[0, 0, 0], [15, 15, 15], [3, 9, 3]] {
            assert_eq!(r.revision(p), Some(last + 1), "{p:?} never under-reported");
        }
        assert_eq!(r.revision([0, 20, 0]), Some(500));
        assert_eq!(r.revision([0, 16, 0]), Some(1));
    }

    #[test]
    fn box_is_the_latest_of_its_cells_and_fails_closed() {
        let mut r = CellRevisions::default();
        for (i, c) in [[-1, -1], [-1, 0], [0, -1], [0, 0]].into_iter().enumerate() {
            r.replaced(c);
            r.stamp(c, 1 + i as u64);
        }
        // Deterministic spread of touches, including a collapsed section.
        let mut seed = 7u32;
        for sequence in 10..400u64 {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let v = (seed >> 8) as i32;
            let p = [v % 24 - 12, (v >> 5) % 40 - 4, (v >> 11) % 24 - 12];
            r.touch(p, sequence);
        }
        for i in 0..SECTION_CELLS as i32 + 1 {
            r.touch([i % 16 - 16, 20, i / 16 - 16], 400 + i as u64);
        }
        let boxes = [
            ([-12, -4, -12], [11, 35, 11]),
            ([-3, 0, -3], [3, 6, 3]),
            ([-1, 15, -1], [0, 16, 0]),
            ([-16, 16, -16], [-15, 31, -15]),
            ([5, -4, 5], [5, -4, 5]),
            ([-12, 30, 4], [-2, 33, 9]),
        ];
        for (min, max) in boxes {
            let mut cells = 0;
            for x in min[0]..=max[0] {
                for y in min[1]..=max[1] {
                    for z in min[2]..=max[2] {
                        cells = cells.max(r.revision([x, y, z]).unwrap());
                    }
                }
            }
            assert_eq!(
                r.latest(min, max, |_| true),
                Some(cells),
                "{min:?}..={max:?}"
            );
        }
        for i in 0..SECTION_CELLS as i32 + 1 {
            let p = [i % 16 - 16, 20, i / 16 - 16];
            assert!(
                r.revision(p).unwrap() >= 400 + i as u64,
                "never under-reported"
            );
        }
        assert_eq!(
            r.latest([-1, 0, -1], [16, 0, 0], |_| true),
            None,
            "x=16 unloaded"
        );
        assert_eq!(r.latest([0, 0, 0], [1, 1, 1], |c| c != (0, 0)), None);
        r.replaced([0, 0]);
        assert_eq!(
            r.latest([-1, 0, -1], [0, 0, 0], |_| true),
            None,
            "unstamped"
        );
    }
}
