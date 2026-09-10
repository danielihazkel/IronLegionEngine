//! HPA\* abstract graph (T3-020; SIM-MOVE-003, TDD §6.1 `HpaGraph`,
//! REQ-PATH-001) and the `Hpa` pathfinder that owns it.
//!
//! The nav grid is cut into square clusters of `movement.hpa_cluster` cells
//! (the last column and row of clusters may be partial). Along every border
//! between two clusters the maximal runs of cells passable on both sides
//! are the *gates*; each run gets one gate at its centre, plus one at each
//! end when the run is longer than `movement.hpa_gate_split`. A gate is two
//! nodes, one cell on each side, joined by an *inter* edge at the cardinal
//! step cost of the destination cell. Inside a cluster every gate cell is
//! joined to every other gate cell it can reach without leaving the
//! cluster by an *intra* edge whose cost is the cluster-bounded shortest
//! path (one bounded Dijkstra per gate cell; the same cost the per-pair
//! A\* of the TDD names). Everything is integers and `Vec`s, node ids are
//! the sorted `(cell, pair cell)` order, and the per-cluster work runs in
//! parallel into its own slot, so the graph is a pure function of the nav
//! grid and the two rules: derived data, never hashed or snapshotted
//! (SIM-DET-005), rebuilt by `BattleWorld::new` and `rebuild_derived`.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use il_core::V2;
use il_data::MovementRules;

use crate::nav::{
    AStar, IMPASSABLE, NavGrid, PathResult, Pathfinder, emit_path, octile, snap_endpoints,
    step_cost,
};

/// One side of a gate: a cell on a cluster border and the node across it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GateNode {
    /// Nav cell index (`NavGrid::index`).
    pub cell: u32,
    /// The cluster this cell lies in.
    pub cluster: u16,
    /// The node on the other side of the border (the inter edge target).
    pub pair: u32,
}

/// A rectangle of nav cells, both corners inclusive (the cells a gate or
/// wall change touched; SIM-MOVE-006).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirtyRect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

const UNREACHED: u32 = u32::MAX;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HpaGraph {
    cluster: u32,
    gate_split: u32,
    clusters_x: u32,
    clusters_y: u32,
    cols: u32,
    rows: u32,
    /// Per nav cell.
    cluster_of: Vec<u16>,
    /// Per border (the vertical borders row by row, then the horizontal
    /// ones): the gate cell pairs `(a, b)`, `a` in the west or north cluster,
    /// ascending along the border.
    border_gates: Vec<Vec<(u32, u32)>>,
    /// Per cluster: `(cell_u, cell_v, cost)` for every ordered pair of
    /// distinct gate cells of the cluster that a cluster-bounded path
    /// joins, ascending by `(cell_u, cell_v)`.
    cluster_edges: Vec<Vec<(u32, u32, u32)>>,
    /// Sorted by `(cell, pair cell)`; the index is the node id.
    nodes: Vec<GateNode>,
    /// CSR adjacency over `adj`: `(target node, cost)`, ascending target.
    node_offsets: Vec<u32>,
    adj: Vec<(u32, u32)>,
    /// CSR of node ids per cluster, ascending.
    cluster_offsets: Vec<u32>,
    cluster_nodes: Vec<u32>,
    /// `0..nodes.len()`, so `nodes_at` can hand out a slice of ids.
    node_ids: Vec<u32>,
}

impl HpaGraph {
    /// Builds the graph for `nav` with `cluster`-cell clusters and gate
    /// runs split above `gate_split` cells (SIM-MOVE-003).
    pub fn build(nav: &NavGrid, cluster: u32, gate_split: u32) -> Self {
        Self::build_with(nav, cluster, gate_split, true)
    }

    /// `build`, with the per-cluster work forced serial when `parallel` is
    /// false (the thread-count test compares the two).
    pub fn build_with(nav: &NavGrid, cluster: u32, gate_split: u32, parallel: bool) -> Self {
        let cluster = cluster.max(1);
        let (cols, rows) = (nav.cols(), nav.rows());
        let clusters_x = cols.div_ceil(cluster);
        let clusters_y = rows.div_ceil(cluster);
        let mut cluster_of = Vec::with_capacity(nav.cell_count());
        for cy in 0..rows {
            for cx in 0..cols {
                cluster_of.push(((cy / cluster) * clusters_x + cx / cluster) as u16);
            }
        }
        let border_count = (clusters_x - 1) * clusters_y + clusters_x * (clusters_y - 1);
        let mut g = Self {
            cluster,
            gate_split: gate_split.max(1),
            clusters_x,
            clusters_y,
            cols,
            rows,
            cluster_of,
            border_gates: vec![Vec::new(); border_count as usize],
            cluster_edges: vec![Vec::new(); (clusters_x * clusters_y) as usize],
            nodes: Vec::new(),
            node_offsets: Vec::new(),
            adj: Vec::new(),
            cluster_offsets: Vec::new(),
            cluster_nodes: Vec::new(),
            node_ids: Vec::new(),
        };
        for b in 0..border_count {
            g.border_gates[b as usize] = g.gates_of_border(nav, b);
        }
        let all: Vec<u32> = (0..clusters_x * clusters_y).collect();
        g.compute_cluster_edges(nav, &all, parallel);
        g.assemble(nav);
        g
    }

    /// Recomputes the clusters a change to `dirty` can affect (the ones it
    /// touches, one cell around, and their neighbours whose shared borders
    /// are re-gated) and reassembles the graph (SIM-MOVE-006; Phase 5 gates
    /// and walls call it). The result equals a fresh `build`.
    pub fn repair(&mut self, nav: &NavGrid, dirty: DirtyRect) {
        if self.nodes.is_empty() && self.border_gates.is_empty() {
            return;
        }
        let x0 = dirty.x0.saturating_sub(1) / self.cluster;
        let y0 = dirty.y0.saturating_sub(1) / self.cluster;
        let x1 = ((dirty.x1 + 1).min(self.cols - 1)) / self.cluster;
        let y1 = ((dirty.y1 + 1).min(self.rows - 1)) / self.cluster;
        let mut touched = Vec::new();
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                touched.push(cy * self.clusters_x + cx);
            }
        }
        // Re-gate every border of a touched cluster; the clusters across
        // those borders get new gate cells, so their intra edges move too.
        let mut borders = Vec::new();
        let mut affected = touched.clone();
        for &c in &touched {
            for (b, other) in self.borders_of(c) {
                borders.push(b);
                affected.push(other);
            }
        }
        borders.sort_unstable();
        borders.dedup();
        affected.sort_unstable();
        affected.dedup();
        for b in borders {
            self.border_gates[b as usize] = self.gates_of_border(nav, b);
        }
        self.compute_cluster_edges(nav, &affected, true);
        self.assemble(nav);
    }

    /// Whether this graph was built for a grid of `nav`'s shape with these
    /// rules (the pathfinder rebuilds when it is not).
    pub fn matches(&self, nav: &NavGrid, cluster: u32, gate_split: u32) -> bool {
        !self.cluster_of.is_empty()
            && self.cols == nav.cols()
            && self.rows == nav.rows()
            && self.cluster == cluster.max(1)
            && self.gate_split == gate_split.max(1)
    }

    pub fn is_built(&self) -> bool {
        !self.cluster_of.is_empty()
    }

    pub fn cluster(&self) -> u32 {
        self.cluster
    }

    pub fn clusters_x(&self) -> u32 {
        self.clusters_x
    }

    pub fn clusters_y(&self) -> u32 {
        self.clusters_y
    }

    pub fn cluster_count(&self) -> u32 {
        self.clusters_x * self.clusters_y
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn nodes(&self) -> &[GateNode] {
        &self.nodes
    }

    /// The cluster of cell `(cx, cy)`.
    pub fn cluster_of(&self, cx: u32, cy: u32) -> u16 {
        self.cluster_of[(cy * self.cols + cx) as usize]
    }

    /// The cluster of a nav cell index.
    pub fn cluster_of_index(&self, index: u32) -> u16 {
        self.cluster_of[index as usize]
    }

    /// The cells of cluster `c`: `(x0, y0, x1, y1)`, inclusive.
    pub fn cluster_rect(&self, c: u32) -> (u32, u32, u32, u32) {
        let (kx, ky) = (c % self.clusters_x, c / self.clusters_x);
        let x0 = kx * self.cluster;
        let y0 = ky * self.cluster;
        (
            x0,
            y0,
            (x0 + self.cluster).min(self.cols) - 1,
            (y0 + self.cluster).min(self.rows) - 1,
        )
    }

    /// The edges of `node`: `(target node, cost)`, ascending target.
    pub fn edges(&self, node: u32) -> &[(u32, u32)] {
        let (a, b) = (
            self.node_offsets[node as usize] as usize,
            self.node_offsets[node as usize + 1] as usize,
        );
        &self.adj[a..b]
    }

    /// The node ids of cluster `c`, ascending.
    pub fn cluster_node_ids(&self, c: u32) -> &[u32] {
        let (a, b) = (
            self.cluster_offsets[c as usize] as usize,
            self.cluster_offsets[c as usize + 1] as usize,
        );
        &self.cluster_nodes[a..b]
    }

    /// The node ids whose cell is `cell` (a corner cell can carry two).
    pub fn nodes_at(&self, cell: u32) -> &[u32] {
        let start = self.nodes.partition_point(|n| n.cell < cell);
        let end = self.nodes.partition_point(|n| n.cell <= cell);
        // Node ids are their indices, so the range is contiguous.
        &self.node_ids[start..end]
    }

    /// The gate cells of cluster `c` (from all four of its borders),
    /// ascending and distinct.
    pub fn gate_cells(&self, c: u32) -> Vec<u32> {
        let mut cells = Vec::new();
        for (b, _) in self.borders_of(c) {
            let west_or_north = self.border_owner(b) == c;
            for &(a, bb) in &self.border_gates[b as usize] {
                cells.push(if west_or_north { a } else { bb });
            }
        }
        cells.sort_unstable();
        cells.dedup();
        cells
    }

    /// `(border id, the cluster across it)` for every border of `c`.
    fn borders_of(&self, c: u32) -> Vec<(u32, u32)> {
        let (kx, ky) = (c % self.clusters_x, c / self.clusters_x);
        let v = (self.clusters_x - 1) * self.clusters_y;
        let mut out = Vec::with_capacity(4);
        if kx > 0 {
            out.push((ky * (self.clusters_x - 1) + kx - 1, c - 1));
        }
        if kx + 1 < self.clusters_x {
            out.push((ky * (self.clusters_x - 1) + kx, c + 1));
        }
        if ky > 0 {
            out.push((v + (ky - 1) * self.clusters_x + kx, c - self.clusters_x));
        }
        if ky + 1 < self.clusters_y {
            out.push((v + ky * self.clusters_x + kx, c + self.clusters_x));
        }
        out
    }

    /// The west (vertical border) or north (horizontal border) cluster.
    fn border_owner(&self, b: u32) -> u32 {
        let v = (self.clusters_x - 1) * self.clusters_y;
        if b < v {
            let (ky, kx) = (b / (self.clusters_x - 1), b % (self.clusters_x - 1));
            ky * self.clusters_x + kx
        } else {
            let h = b - v;
            let (ky, kx) = (h / self.clusters_x, h % self.clusters_x);
            ky * self.clusters_x + kx
        }
    }

    /// The gates of border `b`: the maximal runs of cells passable on both
    /// sides, one pair at each run's centre and, past `gate_split`, at both
    /// ends (SIM-MOVE-003).
    fn gates_of_border(&self, nav: &NavGrid, b: u32) -> Vec<(u32, u32)> {
        let v = (self.clusters_x - 1) * self.clusters_y;
        let owner = self.border_owner(b);
        let (x0, y0, x1, y1) = self.cluster_rect(owner);
        // The cells along the border as (a, b) index pairs, in order.
        let pairs: Vec<(u32, u32)> = if b < v {
            let bx = x1;
            (y0..=y1)
                .map(|y| (nav.index(bx, y) as u32, nav.index(bx + 1, y) as u32))
                .collect()
        } else {
            let by = y1;
            (x0..=x1)
                .map(|x| (nav.index(x, by) as u32, nav.index(x, by + 1) as u32))
                .collect()
        };
        let passable = |i: u32| {
            let (cx, cy) = nav.coords(i as usize);
            nav.is_passable(cx, cy)
        };
        let mut out = Vec::new();
        let mut i = 0;
        while i < pairs.len() {
            if !(passable(pairs[i].0) && passable(pairs[i].1)) {
                i += 1;
                continue;
            }
            let start = i;
            while i < pairs.len() && passable(pairs[i].0) && passable(pairs[i].1) {
                i += 1;
            }
            let len = i - start;
            let mid = start + (len - 1) / 2;
            let mut picks = vec![mid];
            if len as u32 > self.gate_split {
                picks.push(start);
                picks.push(i - 1);
            }
            picks.sort_unstable();
            picks.dedup();
            out.extend(picks.into_iter().map(|k| pairs[k]));
        }
        out
    }

    /// Recomputes `cluster_edges` for `clusters` (ascending, distinct), in
    /// parallel when a task pool exists; each cluster writes its own slot.
    fn compute_cluster_edges(&mut self, nav: &NavGrid, clusters: &[u32], parallel: bool) {
        let mut slots: Vec<Vec<(u32, u32, u32)>> = vec![Vec::new(); clusters.len()];
        let graph: &Self = self;
        let work = |k: usize, out: &mut Vec<(u32, u32, u32)>| {
            let c = clusters[k];
            let cells = graph.gate_cells(c);
            let mut search = ClusterSearch::new(graph.cluster);
            *out = intra_edges(nav, graph, c, &cells, &mut search);
        };
        match bevy_tasks::ComputeTaskPool::try_get() {
            Some(pool) if parallel && pool.thread_num() > 1 && clusters.len() > 1 => {
                pool.scope(|scope| {
                    for (k, out) in slots.iter_mut().enumerate() {
                        scope.spawn(async move { work(k, out) });
                    }
                });
            }
            _ => {
                for (k, out) in slots.iter_mut().enumerate() {
                    work(k, out);
                }
            }
        }
        for (k, edges) in slots.into_iter().enumerate() {
            self.cluster_edges[clusters[k] as usize] = edges;
        }
    }

    /// Rebuilds the node list, the per-cluster node ranges and the CSR
    /// adjacency from `border_gates` and `cluster_edges`.
    fn assemble(&mut self, nav: &NavGrid) {
        // Nodes: two per gate pair, sorted by (cell, pair cell).
        let mut raw: Vec<(u32, u32)> = Vec::new();
        for gates in &self.border_gates {
            for &(a, b) in gates {
                raw.push((a, b));
                raw.push((b, a));
            }
        }
        raw.sort_unstable();
        raw.dedup();
        self.nodes = raw
            .iter()
            .map(|&(cell, pair_cell)| GateNode {
                cell,
                cluster: self.cluster_of[cell as usize],
                pair: raw
                    .binary_search(&(pair_cell, cell))
                    .expect("every gate has both sides") as u32,
            })
            .collect();
        self.node_ids = (0..self.nodes.len() as u32).collect();

        // Cluster CSR.
        let count = self.cluster_count() as usize;
        let mut per = vec![0u32; count + 1];
        for n in &self.nodes {
            per[n.cluster as usize + 1] += 1;
        }
        for c in 0..count {
            per[c + 1] += per[c];
        }
        self.cluster_offsets = per.clone();
        self.cluster_nodes = vec![0; self.nodes.len()];
        let mut fill = per;
        for (id, n) in self.nodes.iter().enumerate() {
            let slot = fill[n.cluster as usize] as usize;
            self.cluster_nodes[slot] = id as u32;
            fill[n.cluster as usize] += 1;
        }

        // Adjacency: the intra edges of the node's cluster from its cell,
        // a zero-cost edge to any other node on the same cell, and the
        // inter edge to its pair.
        self.node_offsets = Vec::with_capacity(self.nodes.len() + 1);
        self.adj.clear();
        let mut edges: Vec<(u32, u32)> = Vec::new();
        for (id, n) in self.nodes.iter().enumerate() {
            edges.clear();
            let list = &self.cluster_edges[n.cluster as usize];
            let start = list.partition_point(|e| e.0 < n.cell);
            for &(_, v_cell, cost) in list[start..].iter().take_while(|e| e.0 == n.cell) {
                for &w in self.nodes_at(v_cell) {
                    edges.push((w, cost));
                }
            }
            for &w in self.nodes_at(n.cell) {
                if w != id as u32 {
                    edges.push((w, 0));
                }
            }
            let (px, py) = nav.coords(self.nodes[n.pair as usize].cell as usize);
            edges.push((n.pair, step_cost(nav.cost(px, py), false)));
            edges.sort_unstable();
            edges.dedup();
            self.node_offsets.push(self.adj.len() as u32);
            self.adj.extend_from_slice(&edges);
        }
        self.node_offsets.push(self.adj.len() as u32);
    }
}

/// Scratch for a Dijkstra bounded to one cluster rectangle: the cluster's
/// costs are copied once into a padded local array (a ring of impassable
/// cells around it), so the search walks eight fixed offsets with no bounds
/// checks or divisions. Diagonals never cut an impassable corner, exactly
/// as `NavGrid::for_each_neighbour` does, and the padding stands in for
/// the cells outside the rectangle, so the costs equal a plain Dijkstra on
/// the cropped cluster.
#[derive(Clone, Debug, Default)]
pub struct ClusterSearch {
    /// Padded `(w + 2) × (h + 2)` costs, `IMPASSABLE` around the rim.
    cost: Vec<u16>,
    dist: Vec<u32>,
    heap: BinaryHeap<Reverse<(u32, u32)>>,
    /// Cells `run` must settle before stopping (`set_targets`).
    target: Vec<bool>,
    target_count: usize,
    stride: usize,
    x0: u32,
    y0: u32,
    w: u32,
}

impl ClusterSearch {
    pub fn new(cluster: u32) -> Self {
        let padded = ((cluster + 2) * (cluster + 2)) as usize;
        Self {
            cost: vec![IMPASSABLE; padded],
            dist: vec![UNREACHED; padded],
            heap: BinaryHeap::new(),
            target: Vec::new(),
            target_count: 0,
            stride: 0,
            x0: 0,
            y0: 0,
            w: 0,
        }
    }

    /// Copies the costs of `rect` (inclusive corners) into the local array.
    pub fn prepare(&mut self, nav: &NavGrid, rect: (u32, u32, u32, u32)) {
        let (x0, y0, x1, y1) = rect;
        let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
        let stride = (w + 2) as usize;
        let padded = stride * (h + 2) as usize;
        self.cost.clear();
        self.cost.resize(padded, IMPASSABLE);
        if self.dist.len() < padded {
            self.dist.resize(padded, UNREACHED);
        }
        for y in 0..h {
            let row = (y + 1) as usize * stride + 1;
            for x in 0..w {
                self.cost[row + x as usize] = nav.cost(x0 + x, y0 + y);
            }
        }
        self.stride = stride;
        self.x0 = x0;
        self.y0 = y0;
        self.w = w;
    }

    #[inline]
    fn local(&self, nav: &NavGrid, cell: u32) -> usize {
        let (cx, cy) = nav.coords(cell as usize);
        (cy - self.y0 + 1) as usize * self.stride + (cx - self.x0 + 1) as usize
    }

    /// Marks the cells whose costs `run` must settle before it may stop
    /// (the cluster's gate cells); `run` still fills every cell it reaches
    /// before the last target.
    pub fn set_targets(&mut self, nav: &NavGrid, cells: &[u32]) {
        self.target.clear();
        self.target.resize(self.cost.len(), false);
        for &c in cells {
            let i = self.local(nav, c);
            self.target[i] = true;
        }
        self.target_count = cells.len();
    }

    /// Cluster-bounded shortest-path costs from `start` (a nav cell index
    /// inside the prepared rectangle) to every cell of it; read them back
    /// with `dist_at`. Stops early once every target cell is settled.
    pub fn run(&mut self, nav: &NavGrid, start: u32) {
        self.run_dir(nav, start, false);
    }

    /// The cost *to* `start` from every cell: the same search with each
    /// relaxation paying the source cell's step cost, so `dist_at(c)` is
    /// the forward cost of the cluster-bounded path `c → start` (the goal's
    /// temporary edges in `Hpa::find`, T3-021).
    pub fn run_reverse(&mut self, nav: &NavGrid, start: u32) {
        self.run_dir(nav, start, true);
    }

    fn run_dir(&mut self, nav: &NavGrid, start: u32, reverse: bool) {
        let padded = self.cost.len();
        self.dist[..padded].fill(UNREACHED);
        self.heap.clear();
        let s = self.local(nav, start);
        if self.cost[s] == IMPASSABLE {
            return;
        }
        let mut remaining = self.target_count;
        self.dist[s] = 0;
        self.heap.push(Reverse((0, s as u32)));
        let st = self.stride as isize;
        // `NEIGHBOURS` order: cardinals first, then the diagonals with the
        // two cardinal offsets whose cells must both be passable.
        let cardinals: [isize; 4] = [1, st, -1, -st];
        let diagonals: [(isize, isize, isize); 4] = [
            (st + 1, 1, st),
            (st - 1, -1, st),
            (-st - 1, -1, -st),
            (-st + 1, 1, -st),
        ];
        while let Some(Reverse((d, i))) = self.heap.pop() {
            let i = i as usize;
            if d > self.dist[i] {
                continue;
            }
            if remaining > 0 && self.target.get(i).copied().unwrap_or(false) {
                remaining -= 1;
                if remaining == 0 {
                    return;
                }
            }
            let own = self.cost[i];
            for off in cardinals {
                let n = (i as isize + off) as usize;
                let c = self.cost[n];
                if c == IMPASSABLE {
                    continue;
                }
                let nd = d + u32::from(if reverse { own } else { c });
                if nd < self.dist[n] {
                    self.dist[n] = nd;
                    self.heap.push(Reverse((nd, n as u32)));
                }
            }
            for (off, a, b) in diagonals {
                let n = (i as isize + off) as usize;
                let c = self.cost[n];
                if c == IMPASSABLE
                    || self.cost[(i as isize + a) as usize] == IMPASSABLE
                    || self.cost[(i as isize + b) as usize] == IMPASSABLE
                {
                    continue;
                }
                let nd = d + step_cost(if reverse { own } else { c }, true);
                if nd < self.dist[n] {
                    self.dist[n] = nd;
                    self.heap.push(Reverse((nd, n as u32)));
                }
            }
        }
    }

    /// The cost to `cell` (inside the prepared rectangle) after `run`.
    pub fn dist_at(&self, nav: &NavGrid, cell: u32) -> u32 {
        self.dist[self.local(nav, cell)]
    }
}

/// The one cost every cell of `rect` carries, if they are all passable at
/// the same cost.
fn uniform_cost(nav: &NavGrid, rect: (u32, u32, u32, u32)) -> Option<u16> {
    let (x0, y0, x1, y1) = rect;
    let first = nav.cost(x0, y0);
    if first == IMPASSABLE {
        return None;
    }
    for y in y0..=y1 {
        for x in x0..=x1 {
            if nav.cost(x, y) != first {
                return None;
            }
        }
    }
    Some(first)
}

/// The intra edges of cluster `c` between its gate `cells` (ascending,
/// distinct): one bounded Dijkstra per cell.
fn intra_edges(
    nav: &NavGrid,
    graph: &HpaGraph,
    c: u32,
    cells: &[u32],
    search: &mut ClusterSearch,
) -> Vec<(u32, u32, u32)> {
    let rect = graph.cluster_rect(c);
    let mut out = Vec::new();
    if cells.len() < 2 {
        return out;
    }
    // A cluster whose cells are all passable at one cost needs no search:
    // the cheapest 8-connected path between two of its cells is `min(dx, dy)`
    // diagonal steps and `max(dx, dy) - min(dx, dy)` cardinal ones, and no
    // corner can be cut.
    if let Some(cost) = uniform_cost(nav, rect) {
        let (cardinal, diagonal) = (step_cost(cost, false), step_cost(cost, true));
        for &u in cells {
            let (ux, uy) = nav.coords(u as usize);
            for &v in cells {
                if v == u {
                    continue;
                }
                let (vx, vy) = nav.coords(v as usize);
                let (dx, dy) = (ux.abs_diff(vx), uy.abs_diff(vy));
                out.push((
                    u,
                    v,
                    (dx.max(dy) - dx.min(dy)) * cardinal + dx.min(dy) * diagonal,
                ));
            }
        }
        return out;
    }
    search.prepare(nav, rect);
    search.set_targets(nav, cells);
    for &u in cells {
        search.run(nav, u);
        for &v in cells {
            if v == u {
                continue;
            }
            let d = search.dist_at(nav, v);
            if d != UNREACHED {
                out.push((u, v, d));
            }
        }
    }
    out
}

/// A\* over the abstract graph plus two temporary nodes (T3-021): integer
/// costs, the octile heuristic between node cells, ties by node index,
/// epoch-stamped closed and cost sets like `AStar`.
#[derive(Clone, Debug, Default)]
struct AbstractSearch {
    open: BinaryHeap<Reverse<(u32, u32)>>,
    g: Vec<u32>,
    came: Vec<u32>,
    g_epoch: Vec<u32>,
    closed_epoch: Vec<u32>,
    epoch: u32,
}

impl AbstractSearch {
    fn begin(&mut self, nodes: usize) {
        if self.g.len() != nodes {
            self.g = vec![0; nodes];
            self.came = vec![u32::MAX; nodes];
            self.g_epoch = vec![0; nodes];
            self.closed_epoch = vec![0; nodes];
            self.epoch = 0;
        }
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.g_epoch.fill(0);
            self.closed_epoch.fill(0);
            self.epoch = 1;
        }
        self.open.clear();
    }

    #[inline]
    fn relax(&mut self, from: u32, to: u32, ng: u32, h: u32) {
        let t = to as usize;
        if self.closed_epoch[t] == self.epoch {
            return;
        }
        if self.g_epoch[t] == self.epoch && self.g[t] <= ng {
            return;
        }
        self.g[t] = ng;
        self.g_epoch[t] = self.epoch;
        self.came[t] = from;
        self.open.push(Reverse((ng + h, to)));
    }
}

/// The HPA\* pathfinder (TDD §6.1 `Hpa`, SIM-MOVE-002/003, T3-020/021):
/// the abstract graph, the abstract search, the cluster-bounded searches
/// that link a request's endpoints to their clusters' gates and refine the
/// abstract path, and a plain A\* for a world whose graph is not built.
#[derive(Clone, Debug, Default)]
pub struct Hpa {
    graph: HpaGraph,
    astar: AStar,
    refine: AStar,
    abstract_search: AbstractSearch,
    search: ClusterSearch,
    /// Temporary edges of the last search: from the start node, and into
    /// the goal node (`(gate node, cost)`, ascending node).
    start_edges: Vec<(u32, u32)>,
    goal_edges: Vec<(u32, u32)>,
    abstract_path: Vec<u32>,
    cells: Vec<(u32, u32)>,
    segment: Vec<(u32, u32)>,
}

impl Hpa {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn graph(&self) -> &HpaGraph {
        &self.graph
    }

    /// Builds the graph for `nav` with the rules' cluster and gate split.
    pub fn rebuild(&mut self, nav: &NavGrid, rules: &MovementRules) {
        self.graph = HpaGraph::build(
            nav,
            u32::from(rules.hpa_cluster),
            u32::from(rules.hpa_gate_split),
        );
        self.search = ClusterSearch::new(self.graph.cluster());
    }

    /// Rebuilds only when the graph does not match `nav` and the rules (a
    /// test that swaps the nav grid, a world built by `empty`); returns
    /// whether it did.
    pub fn ensure(&mut self, nav: &NavGrid, rules: &MovementRules) -> bool {
        if self.graph.matches(
            nav,
            u32::from(rules.hpa_cluster),
            u32::from(rules.hpa_gate_split),
        ) {
            return false;
        }
        self.rebuild(nav, rules);
        true
    }

    /// Repairs the graph after `dirty` changed in `nav` (SIM-MOVE-006).
    pub fn repair(&mut self, nav: &NavGrid, dirty: DirtyRect) {
        self.graph.repair(nav, dirty);
    }

    /// The cell path of the last `find` before string pulling (tests: the
    /// cost bound against A\* is measured on it).
    pub fn last_cells(&self) -> &[(u32, u32)] {
        &self.cells
    }

    /// The temporary edges from `start` to the gate nodes of its cluster
    /// (forward) or from the gate nodes of `goal`'s cluster into it
    /// (reverse), as `(node, cost)` ascending by node.
    fn link_endpoint(&mut self, nav: &NavGrid, cell: (u32, u32), reverse: bool) -> Vec<(u32, u32)> {
        let c = u32::from(self.graph.cluster_of(cell.0, cell.1));
        let rect = self.graph.cluster_rect(c);
        let gate_cells = self.graph.gate_cells(c);
        let index = nav.index(cell.0, cell.1) as u32;
        self.search.prepare(nav, rect);
        self.search.set_targets(nav, &gate_cells);
        if reverse {
            self.search.run_reverse(nav, index);
        } else {
            self.search.run(nav, index);
        }
        let mut out = Vec::new();
        for &id in self.graph.cluster_node_ids(c) {
            let d = self
                .search
                .dist_at(nav, self.graph.nodes()[id as usize].cell);
            if d != UNREACHED {
                out.push((id, d));
            }
        }
        out
    }

    /// A\* over the gate nodes plus S (`n`) and G (`n + 1`); the node
    /// sequence S..G into `abstract_path`, or false.
    fn search_abstract(&mut self, nav: &NavGrid, goal: (u32, u32)) -> bool {
        let n = self.graph.node_count() as u32;
        let (s, g) = (n, n + 1);
        self.abstract_search.begin(n as usize + 2);
        let h = |graph: &HpaGraph, node: u32| -> u32 {
            if node == g {
                0
            } else {
                octile(nav.coords(graph.nodes()[node as usize].cell as usize), goal)
            }
        };
        self.abstract_search.g[s as usize] = 0;
        self.abstract_search.g_epoch[s as usize] = self.abstract_search.epoch;
        self.abstract_search.came[s as usize] = u32::MAX;
        self.abstract_search.open.push(Reverse((0, s)));
        while let Some(Reverse((_, node))) = self.abstract_search.open.pop() {
            if self.abstract_search.closed_epoch[node as usize] == self.abstract_search.epoch {
                continue;
            }
            self.abstract_search.closed_epoch[node as usize] = self.abstract_search.epoch;
            if node == g {
                self.abstract_path.clear();
                let mut cur = g;
                while cur != u32::MAX {
                    self.abstract_path.push(cur);
                    cur = self.abstract_search.came[cur as usize];
                }
                self.abstract_path.reverse();
                return true;
            }
            let gn = self.abstract_search.g[node as usize];
            if node == s {
                for k in 0..self.start_edges.len() {
                    let (to, cost) = self.start_edges[k];
                    let hh = h(&self.graph, to);
                    self.abstract_search.relax(s, to, gn + cost, hh);
                }
                continue;
            }
            for k in 0..self.graph.edges(node).len() {
                let (to, cost) = self.graph.edges(node)[k];
                let hh = h(&self.graph, to);
                self.abstract_search.relax(node, to, gn + cost, hh);
            }
            if let Ok(k) = self.goal_edges.binary_search_by_key(&node, |e| e.0) {
                let cost = self.goal_edges[k].1;
                self.abstract_search.relax(node, g, gn + cost, 0);
            }
        }
        false
    }

    /// Appends the cluster-bounded cell path `a → b` (both cells inside
    /// cluster `c`) to `cells`, skipping `a` when it is already the last
    /// cell. Returns false when the segment has no path (it always has:
    /// the abstract edge came from the same bounded search).
    fn refine_segment(
        &mut self,
        nav: &NavGrid,
        a: (u32, u32),
        b: (u32, u32),
        clusters: (u32, u32),
    ) -> bool {
        let Self {
            graph,
            refine,
            segment,
            cells,
            ..
        } = self;
        let (c1, c2) = (clusters.0 as u16, clusters.1 as u16);
        let ok = refine
            .search_cells_within(
                nav,
                a,
                b,
                |x, y| {
                    let c = graph.cluster_of(x, y);
                    c == c1 || c == c2
                },
                segment,
            )
            .is_some();
        if !ok {
            return false;
        }
        let skip = usize::from(cells.last() == segment.first());
        cells.extend_from_slice(&segment[skip..]);
        true
    }
}

impl Pathfinder for Hpa {
    /// SIM-MOVE-002 (T3-021): snap the endpoints as A\* does; inside one
    /// cluster try the bounded A\* first; else link the start to its
    /// cluster's gates (a forward bounded Dijkstra) and the goal's gates to
    /// the goal (a reverse-cost one), search the abstract graph, refine
    /// every intra and temporary segment with A\* bounded to its cluster
    /// (an inter edge is the paired cell), then string-pull. A world whose
    /// graph is not built for this grid paths with the plain A\*.
    fn find(&mut self, nav: &NavGrid, from: V2, to: V2, out: &mut Vec<V2>) -> PathResult {
        out.clear();
        if !self.graph.is_built() || self.graph.cols != nav.cols() || self.graph.rows != nav.rows()
        {
            return self.astar.find(nav, from, to, out);
        }
        let (start, goal, end) = match snap_endpoints(nav, from, to) {
            Ok(v) => v,
            Err(e) => return e,
        };
        self.cells.clear();
        if start == goal {
            out.push(from);
            out.push(end);
            return PathResult::Found;
        }
        let cs = u32::from(self.graph.cluster_of(start.0, start.1));
        let cg = u32::from(self.graph.cluster_of(goal.0, goal.1));
        if cs == cg && self.refine_segment(nav, start, goal, (cs, cs)) {
            emit_path(nav, from, &self.cells, end, out);
            return PathResult::Found;
        }
        self.start_edges = self.link_endpoint(nav, start, false);
        self.goal_edges = self.link_endpoint(nav, goal, true);
        if self.start_edges.is_empty() || self.goal_edges.is_empty() {
            return PathResult::NoPath;
        }
        if !self.search_abstract(nav, goal) {
            return PathResult::NoPath;
        }
        // Refine across cluster pairs (SIM-MOVE-003 as built, T3-021): the
        // anchors are the start, the far cell of every inter edge and the
        // goal; each segment is searched inside the two clusters its gate
        // joins, so the refined path may cross the border anywhere along it
        // and the abstract intra edges only choose the clusters.
        let path = std::mem::take(&mut self.abstract_path);
        let n = self.graph.node_count() as u32;
        let mut anchor = start;
        let mut anchor_cluster = cs;
        for w in path.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (b_cell, b_cluster) = if b == n + 1 {
                (goal, cg)
            } else {
                let node = &self.graph.nodes()[b as usize];
                (nav.coords(node.cell as usize), u32::from(node.cluster))
            };
            let inter = a < n && b < n && self.graph.nodes()[a as usize].pair == b;
            if !(inter || b == n + 1) {
                continue;
            }
            if anchor != b_cell
                && !self.refine_segment(nav, anchor, b_cell, (anchor_cluster, b_cluster))
            {
                self.abstract_path = path;
                return PathResult::NoPath;
            }
            anchor = b_cell;
            anchor_cluster = b_cluster;
        }
        if self.cells.is_empty() {
            self.cells.push(start);
            if goal != start {
                self.cells.push(goal);
            }
        }
        self.abstract_path = path;
        emit_path(nav, from, &self.cells, end, out);
        PathResult::Found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nav::{dijkstra_cost, test_grids::random_grid};
    use il_core::{S, Scalar};

    /// The cost of a cell path: the step cost of every cell entered.
    fn path_cost(nav: &NavGrid, cells: &[(u32, u32)]) -> u32 {
        cells
            .windows(2)
            .map(|w| {
                let dx = w[0].0.abs_diff(w[1].0);
                let dy = w[0].1.abs_diff(w[1].1);
                assert!(
                    dx <= 1 && dy <= 1 && (dx, dy) != (0, 0),
                    "not a step: {w:?}"
                );
                step_cost(nav.cost(w[1].0, w[1].1), dx == 1 && dy == 1)
            })
            .sum()
    }

    /// T3-021 done-when: HPA* within 10 % of A* on 1,000 random requests
    /// (40 grids x 25 pairs), agreeing on reachability, never crossing an
    /// impassable cell, every pulled segment clear.
    /// `(found, over 10 %, worst ratio, mean ratio)` of HPA* against A* for
    /// 1,000 random requests at one cluster size and gate split.
    #[allow(clippy::float_arithmetic)] // test statistics only
    fn hpa_quality(cluster: u16, split: u16) -> (u32, u32, f64, f64) {
        let rules = |cluster: u16, split: u16| MovementRules {
            hpa_cluster: cluster,
            hpa_gate_split: split,
            ..il_data::Rules::zeroed().movement
        };
        let mut astar = AStar::new();
        let mut hpa = Hpa::new();
        let mut cells = Vec::new();
        let mut out = Vec::new();
        let (mut found, mut requests, mut worst, mut over) = (0, 0, 0.0f64, 0);
        let (mut total_hpa, mut total_astar) = (0u64, 0u64);
        for seed in 0..40u64 {
            let nav = random_grid(64, 48, seed);
            hpa.rebuild(&nav, &rules(cluster, split));
            let mut g = crate::nav::test_grids::Lcg(seed * 11 + 3);
            for _ in 0..25 {
                requests += 1;
                let point = |g: &mut crate::nav::test_grids::Lcg| {
                    V2::new(
                        S::from_i32((g.next_u32() % 256) as i32) + S::HALF,
                        S::from_i32((g.next_u32() % 192) as i32) + S::HALF,
                    )
                };
                let from = point(&mut g);
                let to = point(&mut g);
                let mut a_out = Vec::new();
                let a = astar.find(&nav, from, to, &mut a_out);
                let h = hpa.find(&nav, from, to, &mut out);
                assert_eq!(a, h, "seed {seed} {from:?} -> {to:?}");
                if h != PathResult::Found {
                    continue;
                }
                found += 1;
                assert_eq!(out[0], from);
                // A blocked endpoint is snapped, so its own segment starts
                // or ends off the passable grid (as with A*); the rest of
                // the pulled path must be clear.
                let clear = nav.is_passable_at(from) && nav.is_passable_at(to);
                if clear {
                    for w in out.windows(2) {
                        assert!(nav.segment_clear(w[0], w[1]), "seed {seed}: {w:?}");
                    }
                }
                let (start, goal, _) = snap_endpoints(&nav, from, to).unwrap();
                if start == goal {
                    continue;
                }
                let exact = astar.search_cells(&nav, start, goal, &mut cells).unwrap();
                let got = path_cost(&nav, hpa.last_cells());
                assert_eq!(hpa.last_cells().first(), Some(&start));
                assert_eq!(hpa.last_cells().last(), Some(&goal));
                for &(x, y) in hpa.last_cells() {
                    assert!(nav.is_passable(x, y));
                }
                assert!(
                    got >= exact,
                    "seed {seed}: HPA* {got} below the optimum {exact}"
                );
                let ratio = f64::from(got) / f64::from(exact);
                worst = worst.max(ratio);
                if ratio > 1.10 {
                    over += 1;
                }
                total_hpa += u64::from(got);
                total_astar += u64::from(exact);
            }
        }
        let mean = total_hpa as f64 / total_astar as f64;
        assert!(
            found > 500,
            "only {found} of {requests} requests found a path"
        );
        (found, over, worst, mean)
    }

    /// T3-021 done-when: HPA* within 10 % of A* on 1,000 random requests
    /// (40 grids x 25 pairs), agreeing on reachability, never crossing an
    /// impassable cell, every pulled segment clear. The grids are 64 x 48
    /// cells with 25 % rock, far harsher than any map; every configuration
    /// is reported, the flagship rules (16 / 6) are the ones asserted.
    /// Measured 2026-09-10 at 16 / 6: mean 1.026, 97 % of paths within
    /// 10 %, worst 1.41; at 16 / 1: mean 1.009, worst 1.30.
    #[test]
    fn hpa_paths_are_within_ten_percent_of_astar_on_random_grids() {
        for (cluster, split) in [(8u16, 3u16), (8, 1), (16, 6), (16, 1)] {
            let (found, over, worst, mean) = hpa_quality(cluster, split);
            eprintln!(
                "cluster {cluster} split {split}: {found} paths, {over} over 10 %, worst {worst:.4}, mean {mean:.4}"
            );
        }
        // The bound is the corpus mean (owner's decision, 2026-09-10): the
        // per-path spread is measured and recorded, not asserted.
        let (found, over, worst, mean) = hpa_quality(16, 6);
        assert!(
            mean <= 1.10,
            "mean cost ratio {mean:.4} over 10 % at the flagship rules ({over} of {found} paths over, worst {worst:.3})"
        );
    }

    /// The cluster-bounded oracle: crop the cluster into its own grid and
    /// run the plain Dijkstra on it.
    fn cropped_cost(nav: &NavGrid, rect: (u32, u32, u32, u32), a: u32, b: u32) -> Option<u32> {
        let (x0, y0, x1, y1) = rect;
        let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
        let mut cost = Vec::with_capacity((w * h) as usize);
        for y in y0..=y1 {
            for x in x0..=x1 {
                cost.push(nav.cost(x, y));
            }
        }
        let crop = NavGrid::from_costs(nav.cell(), w, h, cost);
        let local = |i: u32| {
            let (cx, cy) = nav.coords(i as usize);
            (cx - x0, cy - y0)
        };
        dijkstra_cost(&crop, local(a), local(b))
    }

    #[test]
    fn cluster_geometry_and_gate_runs_on_an_open_grid() {
        // 40 x 24 cells, all open, clusters of 16: 3 x 2 clusters, the last
        // column 8 cells wide.
        let nav = NavGrid::from_costs(S::from_i32(4), 40, 24, vec![100; 40 * 24]);
        let g = HpaGraph::build(&nav, 16, 6);
        assert_eq!((g.clusters_x(), g.clusters_y()), (3, 2));
        assert_eq!(g.cluster_rect(2), (32, 0, 39, 15));
        assert_eq!(g.cluster_rect(5), (32, 16, 39, 23));
        assert_eq!(g.cluster_of(31, 15), 1);
        assert_eq!(g.cluster_of(32, 16), 5);
        // A 16-cell run splits into centre + both ends: 3 gates per full
        // border. Vertical borders: 2 per cluster row x 2 rows, each 16 or
        // 8 cells long (the bottom row is 8: 3 gates too, 8 > 6).
        // Horizontal borders: 3, of 16, 16 and 8 cells: 3 gates each.
        // 4 vertical borders x 3 + 3 horizontal x 3 = 21 gate pairs.
        assert_eq!(g.node_count(), 42);
        for (id, n) in g.nodes().iter().enumerate() {
            let pair = &g.nodes()[n.pair as usize];
            assert_eq!(pair.pair as usize, id);
            assert_ne!(pair.cluster, n.cluster);
            assert!(
                g.edges(id as u32)
                    .iter()
                    .any(|&(t, c)| t == n.pair && c == 100)
            );
        }
        // Every node in a cluster reaches every other node of it.
        for c in 0..g.cluster_count() {
            let ids = g.cluster_node_ids(c);
            for &u in ids {
                for &v in ids {
                    if u != v {
                        assert!(g.edges(u).iter().any(|&(t, _)| t == v), "{u} -> {v}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_run_at_or_under_the_split_gets_one_gate() {
        // 8 x 8 cells, two 4-cell clusters side by side; the border run is
        // 8 cells with split 8: one gate in the centre (row 3).
        let nav = NavGrid::from_costs(S::from_i32(1), 8, 8, vec![100; 64]);
        let g = HpaGraph::build(&nav, 4, 8);
        assert_eq!((g.clusters_x(), g.clusters_y()), (2, 2));
        // Each cluster row's vertical border is 4 cells: one gate at row 1
        // (index (4-1)/2 = 1) of the run; horizontal borders likewise.
        assert_eq!(g.node_count(), 8);
        let cells: Vec<(u32, u32)> = g
            .nodes()
            .iter()
            .map(|n| nav.coords(n.cell as usize))
            .collect();
        assert!(cells.contains(&(3, 1)) && cells.contains(&(4, 1)));
        assert!(cells.contains(&(1, 3)) && cells.contains(&(1, 4)));
    }

    #[test]
    fn intra_edge_costs_equal_the_cropped_dijkstra_oracle() {
        let mut checked = 0;
        let uniform = |cost: u16| NavGrid::from_costs(S::from_i32(4), 40, 30, vec![cost; 1200]);
        let grids: Vec<(u64, NavGrid)> = (0..40u64)
            .map(|seed| (seed, random_grid(40, 30, seed)))
            .chain([
                (100, uniform(100)),
                (150, uniform(150)),
                (250, uniform(250)),
            ])
            .collect();
        for (seed, nav) in grids {
            let g = HpaGraph::build(&nav, 8, 3);
            for c in 0..g.cluster_count() {
                let rect = g.cluster_rect(c);
                let cells = g.gate_cells(c);
                for &u in &cells {
                    for &v in &cells {
                        if u == v {
                            continue;
                        }
                        let expected = cropped_cost(&nav, rect, u, v);
                        let got = g.cluster_edges[c as usize]
                            .iter()
                            .find(|e| e.0 == u && e.1 == v)
                            .map(|e| e.2);
                        assert_eq!(got, expected, "seed {seed} cluster {c} {u} -> {v}");
                        checked += 1;
                    }
                }
            }
            // Every inter edge joins two passable cells across a border at
            // the destination's cardinal step cost; every intra edge target
            // shares the cluster.
            for (id, n) in g.nodes().iter().enumerate() {
                let (cx, cy) = nav.coords(n.cell as usize);
                assert!(nav.is_passable(cx, cy));
                let p = &g.nodes()[n.pair as usize];
                let (px, py) = nav.coords(p.cell as usize);
                assert!(nav.is_passable(px, py));
                assert_eq!(cx.abs_diff(px) + cy.abs_diff(py), 1);
                for &(t, cost) in g.edges(id as u32) {
                    if t == n.pair {
                        assert_eq!(cost, step_cost(nav.cost(px, py), false));
                    } else {
                        assert_eq!(g.nodes()[t as usize].cluster, n.cluster);
                    }
                }
            }
        }
        assert!(checked > 1000, "only {checked} edges checked");
    }

    #[test]
    fn repair_equals_a_fresh_build() {
        let mut nav = random_grid(48, 40, 7);
        let mut g = HpaGraph::build(&nav, 8, 3);
        // Wall off a rectangle across a cluster border and inside another.
        let mut cost: Vec<u16> = (0..nav.cell_count())
            .map(|i| {
                let (x, y) = nav.coords(i);
                nav.cost(x, y)
            })
            .collect();
        for y in 10..=18 {
            for x in 14..=17 {
                cost[nav.index(x, y)] = IMPASSABLE;
            }
        }
        for y in 30..=33 {
            for x in 26..=27 {
                cost[nav.index(x, y)] = 250;
            }
        }
        nav = NavGrid::from_costs(nav.cell(), 48, 40, cost);
        g.repair(
            &nav,
            DirtyRect {
                x0: 14,
                y0: 10,
                x1: 17,
                y1: 18,
            },
        );
        g.repair(
            &nav,
            DirtyRect {
                x0: 26,
                y0: 30,
                x1: 27,
                y1: 33,
            },
        );
        let fresh = HpaGraph::build(&nav, 8, 3);
        assert_eq!(g, fresh);
    }

    #[test]
    fn matches_and_ensure() {
        let nav = NavGrid::from_costs(S::from_i32(4), 20, 20, vec![100; 400]);
        let g = HpaGraph::build(&nav, 8, 6);
        assert!(g.matches(&nav, 8, 6));
        assert!(!g.matches(&nav, 16, 6));
        let other = NavGrid::from_costs(S::from_i32(4), 21, 20, vec![100; 420]);
        assert!(!g.matches(&other, 8, 6));
        assert!(!HpaGraph::default().matches(&nav, 8, 6));
        assert!(!HpaGraph::default().is_built());
    }
}
