//! Nav grid, A\* and string pulling (T1-032; SIM-MOVE-001, SIM-MOVE-002,
//! SIM-MOVE-005, TDD §6.1, REQ-PATH-002).
//!
//! Costs are integers (zone `move_cost × 100`, `0` = impassable) so the heap
//! order never depends on the scalar type; ties break on the node index.
//! Slope does not enter the cost (Phase 1 decision): it scales speed only.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use bevy_ecs::prelude::*;
use il_core::{RegimentId, S, Scalar, V2};
use il_data::{MovementRules, Registries};

use crate::components::{Anchor, Order, OrderKind, Path, Waypoint};
use crate::events::BattleEvent;
use crate::map::LoadedMap;
use crate::resources::{Clock, Events, Ids, NavGridRes, PathRequests, PathfinderRes, Regs};

/// Stage 3 `serve_path_requests` (SIM-MOVE-002, SIM-MOVE-005): serves at
/// most `movement.paths_per_tick` queued regiments per tick in ascending
/// id, writing each one's `Path` from its anchor to its order target. A
/// regiment whose order is no longer a move is dropped from the queue; a
/// blocked or unreachable target ends the order with `PathNotFound`.
pub fn serve_path_requests(world: &mut World) {
    let per_tick = usize::from(world.resource::<Regs>().0.rules.movement.paths_per_tick);
    let tick = world.resource::<Clock>().tick;
    let mut served: Vec<RegimentId> = Vec::with_capacity(per_tick);
    {
        let mut queue = world.resource_mut::<PathRequests>();
        while served.len() < per_tick {
            match queue.0.pop_first() {
                Some(id) => served.push(id),
                None => break,
            }
        }
    }
    let mut out = Vec::new();
    for rid in served {
        let Some(entity) = world.resource::<Ids>().regiment_entity(rid) else {
            continue;
        };
        let (Some(anchor), Some(order)) = (world.get::<Anchor>(entity), world.get::<Order>(entity))
        else {
            continue;
        };
        let (from, to) = (anchor.pos, order.target);
        if !order.kind.moves() {
            if let Some(mut path) = world.get_mut::<Path>(entity) {
                path.requested = false;
            }
            continue;
        }
        let result = world.resource_scope(|world, mut pf: Mut<PathfinderRes>| {
            let nav = &world.resource::<NavGridRes>().0;
            pf.0.find(nav, from, to, &mut out)
        });
        let nav = &world.resource::<NavGridRes>().0;
        let path = match result {
            PathResult::Found => Path {
                waypoints: out
                    .iter()
                    .map(|&p| Waypoint {
                        p,
                        corridor: nav.corridor_width_at(p),
                    })
                    .collect(),
                next: u16::from(out.len() > 1),
                requested: false,
            },
            _ => Path::default(),
        };
        if result != PathResult::Found {
            if let Some(mut order) = world.get_mut::<Order>(entity) {
                order.kind = OrderKind::Idle;
            }
            world
                .resource_mut::<Events>()
                .0
                .push(tick, BattleEvent::PathNotFound { regiment: rid });
        }
        if let Some(mut slot) = world.get_mut::<Path>(entity) {
            *slot = path;
        }
    }
}

/// Cost of an impassable cell.
pub const IMPASSABLE: u16 = 0;
/// Cost unit: a zone with `move_cost` 1 costs this per cardinal step.
pub const COST_SCALE: u32 = 100;
/// `sqrt(2) × COST_SCALE`, the diagonal step multiplier.
const DIAGONAL: u32 = 141;
/// How far a blocked start or goal cell is searched for a passable one.
const SNAP_RADIUS: i64 = 8;

/// The eight neighbour offsets in a fixed order (cardinals first).
const NEIGHBOURS: [(i64, i64); 8] = [
    (1, 0),
    (0, 1),
    (-1, 0),
    (0, -1),
    (1, 1),
    (-1, 1),
    (-1, -1),
    (1, -1),
];

#[derive(Clone, Debug, PartialEq)]
pub struct NavGrid {
    cell: S,
    inv_cell: S,
    cols: u32,
    rows: u32,
    /// Per cell: `IMPASSABLE` or `move_cost × COST_SCALE`.
    cost: Vec<u16>,
    /// Per cell: length in cells of the maximal passable run along x that
    /// contains it (capped at 255; 0 for impassable cells).
    passable_run_x: Vec<u8>,
    passable_run_y: Vec<u8>,
}

fn ceil_cells(extent: S, cell: S) -> u32 {
    let q = extent / cell;
    let f = q.floor_i32();
    let n = if S::from_i32(f) < q { f + 1 } else { f };
    n.max(1) as u32
}

impl NavGrid {
    /// Rasterises the map at `rules.nav_cell`: a cell is impassable if any
    /// zone cell whose centre lies inside it is impassable (`passable:
    /// false`, or a river cell without a `crossing` zone); its cost is the
    /// largest `move_cost` of those zone cells.
    pub fn from_map(map: &LoadedMap, regs: &Registries, rules: &MovementRules) -> Self {
        let cell = if rules.nav_cell > S::ZERO {
            rules.nav_cell
        } else {
            map.width.max(map.height).max(S::ONE)
        };
        let cols = ceil_cells(map.width, cell);
        let rows = ceil_cells(map.height, cell);
        let mut cost = vec![COST_SCALE as u16; cols as usize * rows as usize];
        let ratio = cell / map.zone_cell;
        for cy in 0..rows {
            for cx in 0..cols {
                // Zone cells whose centres lie in [cx·cell, (cx+1)·cell).
                let zx0 = (S::from_i32(cx as i32) * ratio).floor_i32().max(0);
                let zx1 =
                    ((S::from_i32(cx as i32 + 1) * ratio).floor_i32()).min(map.zone_cols as i32);
                let zy0 = (S::from_i32(cy as i32) * ratio).floor_i32().max(0);
                let zy1 =
                    ((S::from_i32(cy as i32 + 1) * ratio).floor_i32()).min(map.zone_rows as i32);
                let mut passable = true;
                let mut max_cost = S::ONE;
                // Always include at least the zone cell under the centre.
                let (cx0, cy0) = map.zone_cell_of(V2::new(
                    (S::from_i32(cx as i32) + S::HALF) * cell,
                    (S::from_i32(cy as i32) + S::HALF) * cell,
                ));
                let zx_range = zx0.min(cx0 as i32)..zx1.max(cx0 as i32 + 1);
                let zy_range = zy0.min(cy0 as i32)..zy1.max(cy0 as i32 + 1);
                for zy in zy_range {
                    for zx in zx_range.clone() {
                        let slot = zy as usize * map.zone_cols as usize + zx as usize;
                        let Some(handle) = map.zone_handles.get(usize::from(map.zones[slot]))
                        else {
                            continue; // flat placeholder map: open ground
                        };
                        let zone = regs.zones.get(*handle);
                        if !zone.passable || (map.river[slot] && !zone.crossing) {
                            passable = false;
                        }
                        max_cost = max_cost.max(zone.move_cost);
                    }
                }
                let idx = cy as usize * cols as usize + cx as usize;
                cost[idx] = if passable {
                    let scaled = (max_cost * S::from_i32(COST_SCALE as i32) + S::HALF).floor_i32();
                    scaled.clamp(COST_SCALE as i32, i32::from(u16::MAX)) as u16
                } else {
                    IMPASSABLE
                };
            }
        }
        Self::from_costs(cell, cols, rows, cost)
    }

    /// A grid from explicit per-cell costs (tests, tools).
    pub fn from_costs(cell: S, cols: u32, rows: u32, cost: Vec<u16>) -> Self {
        assert_eq!(cost.len(), cols as usize * rows as usize);
        let mut grid = Self {
            cell,
            inv_cell: S::ONE / cell,
            cols,
            rows,
            cost,
            passable_run_x: Vec::new(),
            passable_run_y: Vec::new(),
        };
        grid.compute_runs();
        grid
    }

    fn compute_runs(&mut self) {
        let (cols, rows) = (self.cols as usize, self.rows as usize);
        self.passable_run_x = vec![0; cols * rows];
        self.passable_run_y = vec![0; cols * rows];
        for y in 0..rows {
            let mut x = 0;
            while x < cols {
                if self.cost[y * cols + x] == IMPASSABLE {
                    x += 1;
                    continue;
                }
                let start = x;
                while x < cols && self.cost[y * cols + x] != IMPASSABLE {
                    x += 1;
                }
                let run = (x - start).min(255) as u8;
                for i in start..x {
                    self.passable_run_x[y * cols + i] = run;
                }
            }
        }
        for x in 0..cols {
            let mut y = 0;
            while y < rows {
                if self.cost[y * cols + x] == IMPASSABLE {
                    y += 1;
                    continue;
                }
                let start = y;
                while y < rows && self.cost[y * cols + x] != IMPASSABLE {
                    y += 1;
                }
                let run = (y - start).min(255) as u8;
                for j in start..y {
                    self.passable_run_y[j * cols + x] = run;
                }
            }
        }
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

    pub fn cell_count(&self) -> usize {
        self.cost.len()
    }

    #[inline]
    pub fn index(&self, cx: u32, cy: u32) -> usize {
        cy as usize * self.cols as usize + cx as usize
    }

    #[inline]
    pub fn coords(&self, index: usize) -> (u32, u32) {
        (
            (index % self.cols as usize) as u32,
            (index / self.cols as usize) as u32,
        )
    }

    /// The cell holding `p`, clamped to the grid.
    pub fn cell_of(&self, p: V2) -> (u32, u32) {
        let cx = (p.x * self.inv_cell)
            .floor_i32()
            .clamp(0, self.cols as i32 - 1);
        let cy = (p.y * self.inv_cell)
            .floor_i32()
            .clamp(0, self.rows as i32 - 1);
        (cx as u32, cy as u32)
    }

    /// The cell holding `p`, or `None` outside the grid.
    fn cell_of_unclamped(&self, p: V2) -> Option<(i64, i64)> {
        let cx = i64::from((p.x * self.inv_cell).floor_i32());
        let cy = i64::from((p.y * self.inv_cell).floor_i32());
        self.in_bounds(cx, cy).then_some((cx, cy))
    }

    pub fn in_bounds(&self, cx: i64, cy: i64) -> bool {
        cx >= 0 && cy >= 0 && cx < i64::from(self.cols) && cy < i64::from(self.rows)
    }

    pub fn cell_center(&self, cx: u32, cy: u32) -> V2 {
        V2::new(
            (S::from_i32(cx as i32) + S::HALF) * self.cell,
            (S::from_i32(cy as i32) + S::HALF) * self.cell,
        )
    }

    /// `IMPASSABLE` or the scaled move cost.
    #[inline]
    pub fn cost(&self, cx: u32, cy: u32) -> u16 {
        self.cost[self.index(cx, cy)]
    }

    #[inline]
    pub fn is_passable(&self, cx: u32, cy: u32) -> bool {
        self.cost(cx, cy) != IMPASSABLE
    }

    /// Whether the cell under `p` is passable (positions outside the grid
    /// are not).
    pub fn is_passable_at(&self, p: V2) -> bool {
        self.cell_of_unclamped(p)
            .is_some_and(|(cx, cy)| self.is_passable(cx as u32, cy as u32))
    }

    /// Width of the passable corridor through the cell under `p`: the
    /// shorter of its x and y runs, in metres (SIM-MOVE-004).
    pub fn corridor_width_at(&self, p: V2) -> S {
        let (cx, cy) = self.cell_of(p);
        let i = self.index(cx, cy);
        let run = self.passable_run_x[i].min(self.passable_run_y[i]);
        S::from_i32(i32::from(run)) * self.cell
    }

    pub fn passable_run_x(&self, cx: u32, cy: u32) -> u8 {
        self.passable_run_x[self.index(cx, cy)]
    }

    pub fn passable_run_y(&self, cx: u32, cy: u32) -> u8 {
        self.passable_run_y[self.index(cx, cy)]
    }

    /// The nearest passable cell to `(cx, cy)` within `SNAP_RADIUS` rings
    /// (itself if passable), ties by smaller `(cy, cx)`.
    pub fn nearest_passable(&self, cx: u32, cy: u32) -> Option<(u32, u32)> {
        if self.is_passable(cx, cy) {
            return Some((cx, cy));
        }
        let (ox, oy) = (i64::from(cx), i64::from(cy));
        for r in 1..=SNAP_RADIUS {
            let mut best: Option<(i64, i64, i64)> = None;
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue;
                    }
                    let (x, y) = (ox + dx, oy + dy);
                    if !self.in_bounds(x, y) || !self.is_passable(x as u32, y as u32) {
                        continue;
                    }
                    let d = dx * dx + dy * dy;
                    let better = match best {
                        None => true,
                        Some((bd, bx, by)) => d < bd || (d == bd && (y, x) < (by, bx)),
                    };
                    if better {
                        best = Some((d, x, y));
                    }
                }
            }
            if let Some((_, x, y)) = best {
                return Some((x as u32, y as u32));
            }
        }
        None
    }

    /// Supercover line test: every cell the segment `a → b` touches is
    /// passable, including both cells at an exact corner crossing.
    pub fn segment_clear(&self, a: V2, b: V2) -> bool {
        let (Some((mut cx, mut cy)), Some((ex, ey))) =
            (self.cell_of_unclamped(a), self.cell_of_unclamped(b))
        else {
            return false;
        };
        let d = b - a;
        let step_x: i64 = if d.x > S::ZERO { 1 } else { -1 };
        let step_y: i64 = if d.y > S::ZERO { 1 } else { -1 };
        // Parametric distance to the next cell boundary along each axis, and
        // per-cell increment; `None` when the segment does not move along it.
        let axis = |start: S, delta: S, cell: i64, step: i64| -> Option<(S, S)> {
            if delta == S::ZERO {
                return None;
            }
            let boundary = if step > 0 {
                S::from_i32(cell as i32 + 1) * self.cell
            } else {
                S::from_i32(cell as i32) * self.cell
            };
            Some(((boundary - start) / delta, self.cell / delta.abs()))
        };
        let mut tx = axis(a.x, d.x, cx, step_x);
        let mut ty = axis(a.y, d.y, cy, step_y);
        let max_steps = (self.cols + self.rows) as usize * 2 + 4;
        for _ in 0..max_steps {
            if !self.is_passable(cx as u32, cy as u32) {
                return false;
            }
            if cx == ex && cy == ey {
                return true;
            }
            match (tx, ty) {
                (Some((mx, dx)), Some((my, dy))) if mx == my => {
                    // Exact corner: the segment touches both side cells.
                    for (sx, sy) in [(cx + step_x, cy), (cx, cy + step_y)] {
                        if !self.in_bounds(sx, sy) || !self.is_passable(sx as u32, sy as u32) {
                            return false;
                        }
                    }
                    cx += step_x;
                    cy += step_y;
                    tx = Some((mx + dx, dx));
                    ty = Some((my + dy, dy));
                }
                (Some((mx, dx)), Some((my, _))) if mx < my => {
                    cx += step_x;
                    tx = Some((mx + dx, dx));
                }
                (Some(_), Some((my, dy))) => {
                    cy += step_y;
                    ty = Some((my + dy, dy));
                }
                (Some((mx, dx)), None) => {
                    cx += step_x;
                    tx = Some((mx + dx, dx));
                }
                (None, Some((my, dy))) => {
                    cy += step_y;
                    ty = Some((my + dy, dy));
                }
                (None, None) => return true,
            }
            if !self.in_bounds(cx, cy) {
                return false;
            }
        }
        false
    }
}

/// Octile heuristic in cost units; admissible because the cheapest cell
/// costs `COST_SCALE`.
fn octile(from: (u32, u32), to: (u32, u32)) -> u32 {
    let dx = from.0.abs_diff(to.0);
    let dy = from.1.abs_diff(to.1);
    COST_SCALE * dx.max(dy) + (DIAGONAL - COST_SCALE) * dx.min(dy)
}

/// Outcome of a path request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathResult {
    /// `out` holds the waypoints from `from` to `to`.
    Found,
    /// No passable route exists between the (snapped) endpoints.
    NoPath,
    /// The start lies in impassable terrain with no passable cell nearby.
    StartBlocked,
    /// The goal lies in impassable terrain with no passable cell nearby.
    GoalBlocked,
}

/// A path search over the nav grid; `AStar` now, HPA\* from Phase 3.
pub trait Pathfinder {
    /// Writes the string-pulled waypoints from `from` to `to` into `out`
    /// (cleared first). `out[0] == from` and the last point is `to`, or the
    /// centre of the nearest passable cell when `to` is blocked.
    fn find(&mut self, nav: &NavGrid, from: V2, to: V2, out: &mut Vec<V2>) -> PathResult;
}

/// A\* with 8-connectivity, integer costs, an epoch-stamped closed set and
/// node-index tie-breaks (TDD §6.1).
#[derive(Clone, Debug, Default)]
pub struct AStar {
    open: BinaryHeap<Reverse<(u32, u32)>>,
    g: Vec<u32>,
    came: Vec<u32>,
    g_epoch: Vec<u32>,
    closed_epoch: Vec<u32>,
    epoch: u32,
}

impl AStar {
    pub fn new() -> Self {
        Self::default()
    }

    fn begin(&mut self, cells: usize) {
        if self.g.len() != cells {
            self.g = vec![0; cells];
            self.came = vec![u32::MAX; cells];
            self.g_epoch = vec![0; cells];
            self.closed_epoch = vec![0; cells];
            self.epoch = 0;
        }
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            // Wrapped: stale stamps could collide, so reset them.
            self.g_epoch.fill(0);
            self.closed_epoch.fill(0);
            self.epoch = 1;
        }
        self.open.clear();
    }

    /// Cheapest cell path from `start` to `goal` (inclusive, in order) and
    /// its cost, or `None`. Diagonal steps never cut an impassable corner.
    pub fn search_cells(
        &mut self,
        nav: &NavGrid,
        start: (u32, u32),
        goal: (u32, u32),
        out: &mut Vec<(u32, u32)>,
    ) -> Option<u32> {
        out.clear();
        if !nav.is_passable(start.0, start.1) || !nav.is_passable(goal.0, goal.1) {
            return None;
        }
        self.begin(nav.cell_count());
        let s = nav.index(start.0, start.1) as u32;
        let t = nav.index(goal.0, goal.1) as u32;
        self.g[s as usize] = 0;
        self.g_epoch[s as usize] = self.epoch;
        self.came[s as usize] = u32::MAX;
        self.open.push(Reverse((octile(start, goal), s)));

        while let Some(Reverse((_, node))) = self.open.pop() {
            if self.closed_epoch[node as usize] == self.epoch {
                continue;
            }
            self.closed_epoch[node as usize] = self.epoch;
            if node == t {
                let mut cur = t;
                while cur != u32::MAX {
                    out.push(nav.coords(cur as usize));
                    cur = self.came[cur as usize];
                }
                out.reverse();
                return Some(self.g[t as usize]);
            }
            let (cx, cy) = nav.coords(node as usize);
            let g = self.g[node as usize];
            for (k, (dx, dy)) in NEIGHBOURS.iter().enumerate() {
                let (nx, ny) = (i64::from(cx) + dx, i64::from(cy) + dy);
                if !nav.in_bounds(nx, ny) {
                    continue;
                }
                let (nx, ny) = (nx as u32, ny as u32);
                let cost = nav.cost(nx, ny);
                if cost == IMPASSABLE {
                    continue;
                }
                let diagonal = k >= 4;
                if diagonal && (!nav.is_passable(nx, cy) || !nav.is_passable(cx, ny)) {
                    continue;
                }
                let step = if diagonal {
                    (u32::from(cost) * DIAGONAL).div_ceil(COST_SCALE)
                } else {
                    u32::from(cost)
                };
                let ng = g + step;
                let n = nav.index(nx, ny) as u32;
                if self.closed_epoch[n as usize] == self.epoch {
                    continue;
                }
                if self.g_epoch[n as usize] == self.epoch && self.g[n as usize] <= ng {
                    continue;
                }
                self.g[n as usize] = ng;
                self.g_epoch[n as usize] = self.epoch;
                self.came[n as usize] = node;
                self.open.push(Reverse((ng + octile((nx, ny), goal), n)));
            }
        }
        None
    }
}

impl Pathfinder for AStar {
    fn find(&mut self, nav: &NavGrid, from: V2, to: V2, out: &mut Vec<V2>) -> PathResult {
        out.clear();
        let (sx, sy) = nav.cell_of(from);
        let Some(start) = nav.nearest_passable(sx, sy) else {
            return PathResult::StartBlocked;
        };
        let (gx, gy) = nav.cell_of(to);
        let Some(goal) = nav.nearest_passable(gx, gy) else {
            return PathResult::GoalBlocked;
        };
        // The goal point itself unless it had to be snapped.
        let end = if goal == (gx, gy) && nav.is_passable_at(to) {
            to
        } else {
            nav.cell_center(goal.0, goal.1)
        };
        if start == goal {
            out.push(from);
            out.push(end);
            return PathResult::Found;
        }
        let mut cells = Vec::new();
        if self.search_cells(nav, start, goal, &mut cells).is_none() {
            return PathResult::NoPath;
        }
        out.push(from);
        out.extend(
            cells[1..cells.len() - 1]
                .iter()
                .map(|&(cx, cy)| nav.cell_center(cx, cy)),
        );
        out.push(end);
        string_pull(nav, out);
        PathResult::Found
    }
}

/// Removes every waypoint that a straight, fully passable segment can skip
/// (greedy farthest-visible). The first and last points are kept.
pub fn string_pull(nav: &NavGrid, path: &mut Vec<V2>) {
    if path.len() < 3 {
        return;
    }
    let mut pulled = Vec::with_capacity(path.len());
    let mut i = 0;
    pulled.push(path[0]);
    while i + 1 < path.len() {
        let mut j = path.len() - 1;
        while j > i + 1 && !nav.segment_clear(path[i], path[j]) {
            j -= 1;
        }
        pulled.push(path[j]);
        i = j;
    }
    *path = pulled;
}

/// Dijkstra over the same graph, the optimality oracle for A\* (tests and
/// the flow fields of Phase 2).
pub fn dijkstra_cost(nav: &NavGrid, start: (u32, u32), goal: (u32, u32)) -> Option<u32> {
    if !nav.is_passable(start.0, start.1) || !nav.is_passable(goal.0, goal.1) {
        return None;
    }
    let n = nav.cell_count();
    let mut dist = vec![u32::MAX; n];
    let mut heap = BinaryHeap::new();
    let s = nav.index(start.0, start.1);
    dist[s] = 0;
    heap.push(Reverse((0u32, s as u32)));
    while let Some(Reverse((d, node))) = heap.pop() {
        if d > dist[node as usize] {
            continue;
        }
        let (cx, cy) = nav.coords(node as usize);
        if (cx, cy) == goal {
            return Some(d);
        }
        for (k, (dx, dy)) in NEIGHBOURS.iter().enumerate() {
            let (nx, ny) = (i64::from(cx) + dx, i64::from(cy) + dy);
            if !nav.in_bounds(nx, ny) {
                continue;
            }
            let (nx, ny) = (nx as u32, ny as u32);
            let cost = nav.cost(nx, ny);
            if cost == IMPASSABLE {
                continue;
            }
            let diagonal = k >= 4;
            if diagonal && (!nav.is_passable(nx, cy) || !nav.is_passable(cx, ny)) {
                continue;
            }
            let step = if diagonal {
                (u32::from(cost) * DIAGONAL).div_ceil(COST_SCALE)
            } else {
                u32::from(cost)
            };
            let nd = d + step;
            let ni = nav.index(nx, ny);
            if nd < dist[ni] {
                dist[ni] = nd;
                heap.push(Reverse((nd, ni as u32)));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) as u32
        }
    }

    /// A random grid: ~25 % rock, the rest costs 100, 150 or 250.
    fn random_grid(cols: u32, rows: u32, seed: u64) -> NavGrid {
        let mut g = Lcg(seed);
        let cost = (0..cols * rows)
            .map(|_| match g.next() % 8 {
                0 | 1 => IMPASSABLE,
                2 => 250,
                3 => 150,
                _ => 100,
            })
            .collect();
        NavGrid::from_costs(S::from_i32(4), cols, rows, cost)
    }

    fn v(x: f32, y: f32) -> V2 {
        V2::from_f32_data(x, y)
    }

    #[test]
    fn astar_matches_dijkstra_on_random_grids() {
        let mut astar = AStar::new();
        let mut cells = Vec::new();
        let mut found = 0;
        for seed in 0..40u64 {
            let nav = random_grid(24, 18, seed);
            let mut g = Lcg(seed * 7 + 1);
            for _ in 0..6 {
                let start = (g.next() % 24, g.next() % 18);
                let goal = (g.next() % 24, g.next() % 18);
                let a = astar.search_cells(&nav, start, goal, &mut cells);
                let d = dijkstra_cost(&nav, start, goal);
                assert_eq!(a, d, "seed {seed} {start:?} -> {goal:?}");
                if let Some(cost) = a {
                    found += 1;
                    assert_eq!(cells.first(), Some(&start));
                    assert_eq!(cells.last(), Some(&goal));
                    assert!(cells.iter().all(|&(x, y)| nav.is_passable(x, y)));
                    // Consecutive cells are 8-neighbours and the cost adds up.
                    let mut total = 0;
                    for w in cells.windows(2) {
                        let dx = w[0].0.abs_diff(w[1].0);
                        let dy = w[0].1.abs_diff(w[1].1);
                        assert!(dx <= 1 && dy <= 1 && (dx, dy) != (0, 0));
                        let c = u32::from(nav.cost(w[1].0, w[1].1));
                        total += if dx == 1 && dy == 1 {
                            (c * DIAGONAL).div_ceil(COST_SCALE)
                        } else {
                            c
                        };
                    }
                    assert_eq!(total, cost);
                }
            }
        }
        assert!(found > 100, "only {found} solvable pairs");
    }

    #[test]
    fn pulled_paths_never_cross_impassable_cells_and_are_shorter() {
        let mut astar = AStar::new();
        let mut out = Vec::new();
        let mut checked = 0;
        for seed in 100..130u64 {
            let nav = random_grid(20, 20, seed);
            let mut g = Lcg(seed);
            for _ in 0..5 {
                let from = v((g.next() % 80) as f32 + 0.5, (g.next() % 80) as f32 + 0.5);
                let to = v((g.next() % 80) as f32 + 0.5, (g.next() % 80) as f32 + 0.5);
                if !nav.is_passable_at(from) || !nav.is_passable_at(to) {
                    continue;
                }
                if astar.find(&nav, from, to, &mut out) != PathResult::Found {
                    continue;
                }
                checked += 1;
                assert_eq!(out[0], from);
                assert_eq!(*out.last().unwrap(), to);
                for w in out.windows(2) {
                    assert!(nav.segment_clear(w[0], w[1]), "seed {seed}: {w:?}");
                    // Dense sampling along the segment agrees.
                    for k in 0..=64 {
                        let t = S::from_i32(k) / S::from_i32(64);
                        assert!(nav.is_passable_at(w[0].lerp(w[1], t)));
                    }
                }
            }
        }
        assert!(checked > 40);
    }

    #[test]
    fn string_pull_skips_collinear_and_visible_points() {
        let nav = NavGrid::from_costs(S::from_i32(1), 10, 10, vec![100; 100]);
        let mut path = vec![
            v(0.5, 0.5),
            v(2.5, 0.5),
            v(4.5, 0.5),
            v(6.5, 2.5),
            v(8.5, 8.5),
        ];
        string_pull(&nav, &mut path);
        assert_eq!(path, vec![v(0.5, 0.5), v(8.5, 8.5)]);
        // A wall forces a corner.
        let mut cost = vec![100u16; 100];
        for y in 0..8 {
            cost[y * 10 + 5] = IMPASSABLE;
        }
        let nav = NavGrid::from_costs(S::from_i32(1), 10, 10, cost);
        let mut astar = AStar::new();
        let mut out = Vec::new();
        assert_eq!(
            astar.find(&nav, v(1.5, 1.5), v(8.5, 1.5), &mut out),
            PathResult::Found
        );
        assert!(out.len() >= 3, "{out:?}");
        assert!(out.iter().all(|p| nav.is_passable_at(*p)));
        assert!(
            out.iter().any(|p| p.y > S::from_i32(8)),
            "goes round the wall: {out:?}"
        );
        assert_eq!(nav.passable_run_x(0, 0), 5);
        assert_eq!(nav.passable_run_x(6, 0), 4);
        assert_eq!(nav.passable_run_y(5, 9), 2);
        assert_eq!(nav.corridor_width_at(v(6.5, 0.5)), S::from_i32(4));
    }

    #[test]
    fn segment_clear_handles_corners_and_axes() {
        // 4 x 4 grid, cell 1, with (1,1) and (2,2) blocked: the diagonal
        // from (0.5,0.5) to (3.5,3.5) runs through the blocked corners.
        let mut cost = vec![100u16; 16];
        cost[5] = IMPASSABLE;
        cost[10] = IMPASSABLE;
        let nav = NavGrid::from_costs(S::from_i32(1), 4, 4, cost);
        assert!(!nav.segment_clear(v(0.5, 0.5), v(3.5, 3.5)));
        assert!(nav.segment_clear(v(0.5, 0.5), v(3.5, 0.5)));
        assert!(nav.segment_clear(v(0.5, 3.5), v(0.5, 0.5)));
        assert!(nav.segment_clear(v(0.5, 2.5), v(1.5, 2.5)));
        assert!(!nav.segment_clear(v(0.5, 2.5), v(2.5, 2.5)));
        // Exactly through the corner between (0,1) and (1,0) which are open,
        // but (1,1) is blocked and touched.
        assert!(!nav.segment_clear(v(0.5, 0.5), v(1.5, 1.5)));
        // Outside the grid is never clear.
        assert!(!nav.segment_clear(v(-1.0, 0.5), v(0.5, 0.5)));
        assert_eq!(nav.nearest_passable(1, 1), Some((1, 0)));
        assert_eq!(nav.nearest_passable(0, 0), Some((0, 0)));
    }

    #[test]
    fn blocked_endpoints_snap_or_fail() {
        let mut cost = vec![100u16; 25];
        cost[12] = IMPASSABLE; // centre of a 5 x 5
        let nav = NavGrid::from_costs(S::from_i32(2), 5, 5, cost);
        let mut astar = AStar::new();
        let mut out = Vec::new();
        assert_eq!(
            astar.find(&nav, v(1.0, 1.0), v(5.0, 5.0), &mut out),
            PathResult::Found
        );
        assert_ne!(
            *out.last().unwrap(),
            v(5.0, 5.0),
            "goal snapped off the rock"
        );
        assert!(nav.is_passable_at(*out.last().unwrap()));
        let solid = NavGrid::from_costs(S::from_i32(2), 5, 5, vec![IMPASSABLE; 25]);
        assert_eq!(
            astar.find(&solid, v(1.0, 1.0), v(5.0, 5.0), &mut out),
            PathResult::StartBlocked
        );
        // An island: start and goal passable but disconnected.
        let mut cost = vec![100u16; 25];
        for i in 0..5 {
            cost[2 * 5 + i] = IMPASSABLE;
        }
        let split = NavGrid::from_costs(S::from_i32(2), 5, 5, cost);
        assert_eq!(
            astar.find(&split, v(1.0, 1.0), v(1.0, 9.0), &mut out),
            PathResult::NoPath
        );
    }
}
