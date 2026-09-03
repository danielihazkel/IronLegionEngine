//! Uniform spatial grid with linked cells (T1-031, TDD §5, ADR-013,
//! REQ-PATH-009).
//!
//! Rebuilt from scratch every tick at Stage 6 (32k inserts are cheaper than
//! keeping an incremental structure deterministic). Entries are sorted by
//! stable id before insertion and each cell's chain is ascending, so every
//! traversal order is a function of ids alone, never of ECS storage order
//! (SIM-DET-003). Two instances exist: soldiers at `movement.spatial_cell`
//! and regiment anchors at `movement.anchor_cell`.

use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::*;
use il_core::{RegimentId, S, Scalar, SoldierId, V2};

use crate::components::{Anchor, Pos, Regiment, Soldier};
use crate::resources::{AnchorGridRes, MapRes, Regs, SpatialGridRes};

const NONE: u32 = u32::MAX;

/// Stage 6 (`SpatialGrid`): rebuilds the soldier grid at
/// `movement.spatial_cell` and the anchor grid at `movement.anchor_cell`
/// from this tick's positions. Also run by `rebuild_derived` so the first
/// step after `new` or `restore` sees a grid of the starting positions.
pub fn rebuild_spatial_grids(
    soldiers: Query<(Entity, &Soldier, &Pos)>,
    regiments: Query<(Entity, &Regiment, &Anchor)>,
    map: Res<MapRes>,
    regs: Res<Regs>,
    mut grid: ResMut<SpatialGridRes>,
    mut anchors: ResMut<AnchorGridRes>,
) {
    let (w, h) = (map.0.width, map.0.height);
    let rules = &regs.0.rules.movement;
    grid.0.ensure(w, h, rules.spatial_cell);
    grid.0
        .rebuild(soldiers.iter().map(|(entity, s, pos)| Entry {
            id: s.id,
            entity,
            pos: pos.p,
        }));
    anchors.0.ensure(w, h, rules.anchor_cell);
    anchors
        .0
        .rebuild(regiments.iter().map(|(entity, r, anchor)| Entry {
            id: r.id,
            entity,
            pos: anchor.pos,
        }));
}

/// Marker so the type aliases below read at the use sites.
pub type SoldierGrid = SpatialGrid<SoldierId>;
pub type AnchorGrid = SpatialGrid<RegimentId>;

/// One indexed object: its stable id, ECS entity and position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Entry<Id> {
    pub id: Id,
    pub entity: Entity,
    pub pos: V2,
}

#[derive(Clone, Debug)]
pub struct SpatialGrid<Id> {
    cell: S,
    /// `1 / cell`, so bucketing multiplies instead of dividing.
    inv_cell: S,
    cols: u32,
    rows: u32,
    /// Per cell: index of the first entry, or `NONE`.
    heads: Vec<u32>,
    /// Per entry: index of the next entry in the same cell, or `NONE`.
    next: Vec<u32>,
    /// Ascending by id; indices into this are what the pair and cell
    /// iterators hand out.
    entries: Vec<Entry<Id>>,
    /// Scratch: cell slot per entry during `rebuild`.
    slots: Vec<u32>,
}

impl<Id: Copy + Ord> SpatialGrid<Id> {
    /// An empty grid covering `[0, width] × [0, height]` in cells of
    /// `cell` metres (a non-positive cell means one cell for the whole map).
    pub fn new(width: S, height: S, cell: S) -> Self {
        let cell = if cell > S::ZERO {
            cell
        } else {
            width.max(height).max(S::ONE)
        };
        let cols = ceil_cells(width, cell);
        let rows = ceil_cells(height, cell);
        Self {
            cell,
            inv_cell: S::ONE / cell,
            cols,
            rows,
            heads: vec![NONE; cols as usize * rows as usize],
            next: Vec::new(),
            entries: Vec::new(),
            slots: Vec::new(),
        }
    }

    /// Re-dimensions the grid if the map size or cell changed (rules hot
    /// reload); keeps entries otherwise. Returns whether it changed.
    pub fn ensure(&mut self, width: S, height: S, cell: S) -> bool {
        let fresh = Self::new(width, height, cell);
        if fresh.cell == self.cell && fresh.cols == self.cols && fresh.rows == self.rows {
            return false;
        }
        *self = fresh;
        true
    }

    pub fn cell(&self) -> S {
        self.cell
    }

    pub fn cols(&self) -> u32 {
        self.cols
    }

    pub fn rows(&self) -> u32 {
        self.rows
    }

    /// Entries in ascending id order.
    pub fn entries(&self) -> &[Entry<Id>] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The cell holding `p`; positions outside the grid clamp to the edge.
    pub fn cell_of(&self, p: V2) -> (u32, u32) {
        let cx = (p.x * self.inv_cell)
            .floor_i32()
            .clamp(0, self.cols as i32 - 1);
        let cy = (p.y * self.inv_cell)
            .floor_i32()
            .clamp(0, self.rows as i32 - 1);
        (cx as u32, cy as u32)
    }

    #[inline]
    fn cell_index(&self, cx: u32, cy: u32) -> usize {
        cy as usize * self.cols as usize + cx as usize
    }

    /// Replaces every entry. The input may arrive in any order; it is sorted
    /// by id and inserted back to front so each cell's chain ascends.
    pub fn rebuild(&mut self, iter: impl IntoIterator<Item = Entry<Id>>) {
        self.entries.clear();
        self.entries.extend(iter);
        self.entries.sort_unstable_by_key(|e| e.id);
        let cells = self.cols as usize * self.rows as usize;
        if self.heads.len() == cells {
            self.heads.fill(NONE);
        } else {
            self.heads.clear();
            self.heads.resize(cells, NONE);
        }
        self.next.clear();
        self.next.resize(self.entries.len(), NONE);
        // Bucket first (a streaming pass the compiler can vectorise), then
        // link back to front so each chain ascends.
        let cols = self.cols;
        let inv = self.inv_cell;
        let (max_x, max_y) = (self.cols as i32 - 1, self.rows as i32 - 1);
        self.slots.clear();
        self.slots.extend(self.entries.iter().map(|e| {
            let cx = (e.pos.x * inv).floor_i32().clamp(0, max_x) as u32;
            let cy = (e.pos.y * inv).floor_i32().clamp(0, max_y) as u32;
            cy * cols + cx
        }));
        for i in (0..self.entries.len()).rev() {
            let slot = self.slots[i] as usize;
            self.next[i] = self.heads[slot];
            self.heads[slot] = i as u32;
        }
    }

    /// Indices of the entries in cell `(cx, cy)`, ascending id.
    pub fn cell_entries(&self, cx: u32, cy: u32) -> CellIter<'_, Id> {
        CellIter {
            grid: self,
            current: self.heads[self.cell_index(cx, cy)],
        }
    }

    /// Indices of every entry within `r` of `c`, ascending id.
    pub fn query_circle_indices(&self, c: V2, r: S, out: &mut Vec<usize>) {
        out.clear();
        let r = r.max(S::ZERO);
        let (x0, y0) = self.cell_of(V2::new(c.x - r, c.y - r));
        let (x1, y1) = self.cell_of(V2::new(c.x + r, c.y + r));
        let r_sq = r * r;
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                for i in self.cell_entries(cx, cy) {
                    if self.entries[i].pos.distance_sq(c) <= r_sq {
                        out.push(i);
                    }
                }
            }
        }
        out.sort_unstable();
    }

    /// Every entry within `r` of `c`, ascending id (TDD §5 `query_circle`).
    pub fn query_circle(&self, c: V2, r: S, out: &mut Vec<Entry<Id>>) {
        let mut indices = Vec::new();
        self.query_circle_indices(c, r, &mut indices);
        out.clear();
        out.extend(indices.into_iter().map(|i| self.entries[i]));
    }

    /// Calls `f(i, j)` with `i < j` for every pair of entries in the same or
    /// neighbouring cells, each pair exactly once (half-neighbourhood: self,
    /// east, north-east, north, north-west), rows in ascending order.
    pub fn for_each_pair(&self, mut f: impl FnMut(usize, usize)) {
        for cy in 0..self.rows {
            self.for_each_pair_in_row(cy, &mut f);
        }
    }

    /// The pairs [`for_each_pair`](Self::for_each_pair) emits for the cells of
    /// row `cy` (with their east neighbour and the three cells above), so
    /// rows can be processed in parallel into per-row buffers (SAD §8).
    pub fn for_each_pair_in_row(&self, cy: u32, mut f: impl FnMut(usize, usize)) {
        for cx in 0..self.cols {
            let head = self.heads[self.cell_index(cx, cy)];
            if head == NONE {
                continue;
            }
            // Within the cell.
            let mut a = head;
            while a != NONE {
                let mut b = self.next[a as usize];
                while b != NONE {
                    f(a as usize, b as usize);
                    b = self.next[b as usize];
                }
                a = self.next[a as usize];
            }
            // Against the four forward neighbours.
            let neighbours = [
                (cx as i64 + 1, cy as i64),
                (cx as i64 + 1, cy as i64 + 1),
                (cx as i64, cy as i64 + 1),
                (cx as i64 - 1, cy as i64 + 1),
            ];
            for (nx, ny) in neighbours {
                if nx < 0 || ny < 0 || nx >= i64::from(self.cols) || ny >= i64::from(self.rows) {
                    continue;
                }
                let mut a = self.heads[self.cell_index(cx, cy)];
                while a != NONE {
                    let mut b = self.heads[self.cell_index(nx as u32, ny as u32)];
                    while b != NONE {
                        let (i, j) = if a < b { (a, b) } else { (b, a) };
                        f(i as usize, j as usize);
                        b = self.next[b as usize];
                    }
                    a = self.next[a as usize];
                }
            }
        }
    }
}

/// Iterator over one cell's chain.
pub struct CellIter<'a, Id> {
    grid: &'a SpatialGrid<Id>,
    current: u32,
}

impl<Id> Iterator for CellIter<'_, Id> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        if self.current == NONE {
            return None;
        }
        let i = self.current as usize;
        self.current = self.grid.next[i];
        Some(i)
    }
}

fn ceil_cells(extent: S, cell: S) -> u32 {
    let q = extent / cell;
    let f = q.floor_i32();
    let n = if S::from_i32(f) < q { f + 1 } else { f };
    n.max(1) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Small deterministic generator (no `rand` in sim crates).
    struct Lcg(u64);
    impl Lcg {
        fn next_unit(&mut self) -> S {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            S::from_i32(((self.0 >> 40) & 0xffff) as i32) / S::from_i32(0x1_0000)
        }
    }

    fn layout(n: u32, w: i32, h: i32, seed: u64) -> Vec<Entry<u32>> {
        let mut g = Lcg(seed);
        let mut v: Vec<Entry<u32>> = (0..n)
            .map(|id| Entry {
                id,
                entity: Entity::PLACEHOLDER,
                pos: V2::new(
                    g.next_unit() * S::from_i32(w),
                    g.next_unit() * S::from_i32(h),
                ),
            })
            .collect();
        // Shuffle the input order: the grid must not care.
        v.reverse();
        v.swap(0, n as usize / 2);
        v
    }

    #[test]
    fn dimensions_and_cells() {
        let g: SpatialGrid<u32> =
            SpatialGrid::new(S::from_i32(100), S::from_i32(50), S::from_i32(4));
        assert_eq!((g.cols(), g.rows()), (25, 13));
        assert_eq!(g.cell_of(V2::new(S::from_i32(7), S::from_i32(49))), (1, 12));
        assert_eq!(
            g.cell_of(V2::new(S::from_i32(-5), S::from_i32(500))),
            (0, 12)
        );
        let flat: SpatialGrid<u32> = SpatialGrid::new(S::from_i32(100), S::from_i32(50), S::ZERO);
        assert_eq!((flat.cols(), flat.rows()), (1, 1));
        assert_eq!(flat.cell(), S::from_i32(100));
    }

    #[test]
    fn query_circle_matches_brute_force_and_is_ascending() {
        let entries = layout(2000, 200, 150, 7);
        let mut g = SpatialGrid::new(S::from_i32(200), S::from_i32(150), S::from_i32(4));
        g.rebuild(entries.iter().copied());
        assert!(g.entries().windows(2).all(|w| w[0].id < w[1].id));
        let mut out = Vec::new();
        let mut lcg = Lcg(99);
        for _ in 0..50 {
            let c = V2::new(
                lcg.next_unit() * S::from_i32(220) - S::from_i32(10),
                lcg.next_unit() * S::from_i32(170) - S::from_i32(10),
            );
            let r = lcg.next_unit() * S::from_i32(15);
            g.query_circle(c, r, &mut out);
            let got: Vec<u32> = out.iter().map(|e| e.id).collect();
            let mut expected: Vec<u32> = entries
                .iter()
                .filter(|e| e.pos.distance_sq(c) <= r * r)
                .map(|e| e.id)
                .collect();
            expected.sort_unstable();
            assert_eq!(got, expected, "c={c:?} r={r:?}");
        }
    }

    #[test]
    fn pair_enumeration_visits_each_neighbouring_pair_once() {
        let entries = layout(1500, 120, 80, 3);
        let mut g = SpatialGrid::new(S::from_i32(120), S::from_i32(80), S::from_i32(4));
        g.rebuild(entries.iter().copied());
        let mut seen: Vec<(usize, usize)> = Vec::new();
        g.for_each_pair(|i, j| {
            assert!(i < j);
            seen.push((i, j));
        });
        let set: BTreeSet<(usize, usize)> = seen.iter().copied().collect();
        assert_eq!(set.len(), seen.len(), "a pair was emitted twice");
        // Exactly the pairs whose cells are within one step in both axes.
        let e = g.entries();
        let mut expected = BTreeSet::new();
        for i in 0..e.len() {
            for j in i + 1..e.len() {
                let (ax, ay) = g.cell_of(e[i].pos);
                let (bx, by) = g.cell_of(e[j].pos);
                if ax.abs_diff(bx) <= 1 && ay.abs_diff(by) <= 1 {
                    expected.insert((i, j));
                }
            }
        }
        assert_eq!(set, expected);
        // Every pair closer than one cell is covered.
        for i in 0..e.len() {
            for j in i + 1..e.len() {
                if e[i].pos.distance(e[j].pos) < g.cell() {
                    assert!(set.contains(&(i, j)));
                }
            }
        }
        // Per-row enumeration partitions the same set.
        let mut by_rows = BTreeSet::new();
        for cy in 0..g.rows() {
            g.for_each_pair_in_row(cy, |i, j| {
                assert!(by_rows.insert((i, j)));
            });
        }
        assert_eq!(by_rows, set);
    }

    #[test]
    fn rebuild_is_order_independent_and_ensure_redimensions() {
        let entries = layout(300, 50, 50, 11);
        let mut a = SpatialGrid::new(S::from_i32(50), S::from_i32(50), S::from_i32(5));
        let mut b = a.clone();
        a.rebuild(entries.iter().copied());
        let mut shuffled = entries.clone();
        shuffled.rotate_left(77);
        b.rebuild(shuffled);
        assert_eq!(a.entries(), b.entries());
        let mut pa = Vec::new();
        let mut pb = Vec::new();
        a.for_each_pair(|i, j| pa.push((i, j)));
        b.for_each_pair(|i, j| pb.push((i, j)));
        assert_eq!(pa, pb);
        assert!(!a.ensure(S::from_i32(50), S::from_i32(50), S::from_i32(5)));
        assert!(a.ensure(S::from_i32(50), S::from_i32(50), S::from_i32(10)));
        assert_eq!(a.cols(), 5);
        assert!(a.is_empty());
    }
}
