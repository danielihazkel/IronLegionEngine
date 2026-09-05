//! Escape flow fields (T2-042; SIM-FLOW-001..003, REQ-PATH-004, TDD §6.2).
//!
//! One field per side over the nav grid: a multi-source Dijkstra from
//! every passable cell of the side's escape edge (the map edge nearest its
//! deployment polygon, plan decision 9), storing per cell the direction to
//! the lowest-cost neighbour. Routing and withdrawing soldiers follow it
//! (`movement::steer`). Derived data: rebuilt by `rebuild_derived` and
//! whenever the nav grid changes, never hashed or snapshotted. The build is
//! single-threaded over a total order, so it cannot differ by thread count.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use bevy_ecs::prelude::*;
use il_core::{S, Scalar, V2};
use il_data::MapEdge;

use crate::map::LoadedMap;
use crate::nav::{NEIGHBOURS, NavGrid};
use crate::resources::{FlowFields, MapRes, NavGridRes, Sides};

/// Direction code of a cell without a direction: impassable, unreachable
/// or off the field.
pub const NO_DIRECTION: u8 = 8;

/// The escape field of one side.
#[derive(Clone, Debug, PartialEq)]
pub struct FlowField {
    cols: u32,
    rows: u32,
    edge: MapEdge,
    /// Per cell: index into `nav::NEIGHBOURS` or `NO_DIRECTION`.
    dir: Vec<u8>,
    /// Per cell: Dijkstra cost from the nearest seed (`u32::MAX` when
    /// unreachable); kept for tests and the overlay.
    dist: Vec<u32>,
    /// Unit vectors per direction code (`NO_DIRECTION` is zero).
    vec: [V2; 9],
}

/// SIM-FLOW-001: the outward normal's direction code of an edge.
fn outward(edge: MapEdge) -> u8 {
    match edge {
        MapEdge::East => 0,
        MapEdge::North => 1,
        MapEdge::West => 2,
        MapEdge::South => 3,
    }
}

/// The unit vectors of the nine direction codes, from `NEIGHBOURS`
/// (diagonals scaled by `1/√2`) through `Scalar` arithmetic only.
fn direction_vectors() -> [V2; 9] {
    let inv_sqrt2 = S::ONE / S::from_i32(2).sqrt();
    let mut out = [V2::ZERO; 9];
    for (k, (dx, dy)) in NEIGHBOURS.iter().enumerate() {
        let v = V2::new(S::from_i32(*dx as i32), S::from_i32(*dy as i32));
        out[k] = if k >= 4 { v * inv_sqrt2 } else { v };
    }
    out
}

/// SIM-FLOW-001 (plan decision 9): the map edge nearest the mean of the
/// side's deployment polygon vertices; ties West, East, South, North;
/// West for a map without that polygon (the flat placeholder).
pub fn escape_edge(map: &LoadedMap, zone: u8) -> MapEdge {
    let Some(polygon) = map.deployment_polygon(zone).filter(|p| !p.is_empty()) else {
        return MapEdge::West;
    };
    let mut sum = V2::ZERO;
    for p in polygon {
        sum += *p;
    }
    let c = sum * (S::ONE / S::from_i32(polygon.len() as i32));
    let candidates = [
        (MapEdge::West, c.x),
        (MapEdge::East, map.width - c.x),
        (MapEdge::South, c.y),
        (MapEdge::North, map.height - c.y),
    ];
    let mut best = candidates[0];
    for cand in &candidates[1..] {
        if cand.1 < best.1 {
            best = *cand;
        }
    }
    best.0
}

impl FlowField {
    /// SIM-FLOW-001: the field toward `edge` over `nav`.
    pub fn build(nav: &NavGrid, edge: MapEdge) -> Self {
        let (cols, rows) = (nav.cols(), nav.rows());
        let n = nav.cell_count();
        let mut dist = vec![u32::MAX; n];
        let mut heap: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new();
        for (cx, cy) in Self::edge_cells(cols, rows, edge) {
            if nav.is_passable(cx, cy) {
                let i = nav.index(cx, cy);
                dist[i] = 0;
                heap.push(Reverse((0, i as u32)));
            }
        }
        while let Some(Reverse((d, node))) = heap.pop() {
            if d > dist[node as usize] {
                continue;
            }
            let (cx, cy) = nav.coords(node as usize);
            nav.for_each_neighbour(cx, cy, |_, nx, ny, step| {
                let ni = nav.index(nx, ny);
                let nd = d + step;
                if nd < dist[ni] {
                    dist[ni] = nd;
                    heap.push(Reverse((nd, ni as u32)));
                }
            });
        }
        // Directions from the finished distances: the passable neighbour
        // with the strictly lowest cost, first in `NEIGHBOURS` order on a
        // tie; seeds point off the map.
        let mut dir = vec![NO_DIRECTION; n];
        for i in 0..n {
            if dist[i] == u32::MAX {
                continue;
            }
            if dist[i] == 0 {
                dir[i] = outward(edge);
                continue;
            }
            let (cx, cy) = nav.coords(i);
            let mut best: Option<(u32, u8)> = None;
            nav.for_each_neighbour(cx, cy, |k, nx, ny, _| {
                let nd = dist[nav.index(nx, ny)];
                if nd != u32::MAX && best.is_none_or(|(bd, _)| nd < bd) {
                    best = Some((nd, k as u8));
                }
            });
            dir[i] = best.map_or(NO_DIRECTION, |(_, k)| k);
        }
        Self {
            cols,
            rows,
            edge,
            dir,
            dist,
            vec: direction_vectors(),
        }
    }

    /// The cells of a map edge in ascending index order.
    fn edge_cells(cols: u32, rows: u32, edge: MapEdge) -> Vec<(u32, u32)> {
        match edge {
            MapEdge::West => (0..rows).map(|cy| (0, cy)).collect(),
            MapEdge::East => (0..rows).map(|cy| (cols - 1, cy)).collect(),
            MapEdge::South => (0..cols).map(|cx| (cx, 0)).collect(),
            MapEdge::North => (0..cols).map(|cx| (cx, rows - 1)).collect(),
        }
    }

    pub fn edge(&self) -> MapEdge {
        self.edge
    }

    /// The direction code of a cell.
    pub fn code(&self, cx: u32, cy: u32) -> u8 {
        self.dir[(cy * self.cols + cx) as usize]
    }

    /// The Dijkstra cost of a cell (`u32::MAX` when unreachable).
    pub fn cost(&self, cx: u32, cy: u32) -> u32 {
        self.dist[(cy * self.cols + cx) as usize]
    }

    /// SIM-FLOW-002: the unit direction of the cell under `p` (nearest
    /// cell; zero where the field has no direction).
    pub fn direction_at(&self, nav: &NavGrid, p: V2) -> V2 {
        debug_assert_eq!((nav.cols(), nav.rows()), (self.cols, self.rows));
        let (cx, cy) = nav.cell_of(p);
        self.vec[usize::from(self.code(cx, cy))]
    }

    /// SIM-FLOW-002: `p` lies in a cell of the escape edge (`cell_of`
    /// clamps, so a soldier pressed against the map boundary counts).
    pub fn is_exit(&self, nav: &NavGrid, p: V2) -> bool {
        let (cx, cy) = nav.cell_of(p);
        match self.edge {
            MapEdge::West => cx == 0,
            MapEdge::East => cx == self.cols - 1,
            MapEdge::South => cy == 0,
            MapEdge::North => cy == self.rows - 1,
        }
    }
}

/// Builds one field per side from the current nav grid (`rebuild_derived`
/// and any nav-grid change, SIM-FLOW-003).
pub fn rebuild_flow_fields(world: &mut World) {
    let fields: Vec<FlowField> = {
        let nav = &world.resource::<NavGridRes>().0;
        let _map = &world.resource::<MapRes>().0;
        world
            .resource::<Sides>()
            .0
            .iter()
            .map(|s| FlowField::build(nav, s.escape_edge))
            .collect()
    };
    world.resource_mut::<FlowFields>().fields = fields;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nav::IMPASSABLE;

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

    fn on_edge(cx: u32, cy: u32, cols: u32, rows: u32, edge: MapEdge) -> bool {
        match edge {
            MapEdge::West => cx == 0,
            MapEdge::East => cx == cols - 1,
            MapEdge::South => cy == 0,
            MapEdge::North => cy == rows - 1,
        }
    }

    /// Every directed cell walks a strictly descending chain of passable
    /// cells (no corner cut) to a seed on the edge.
    fn check_chains(nav: &NavGrid, field: &FlowField) {
        let (cols, rows) = (nav.cols(), nav.rows());
        for cy in 0..rows {
            for cx in 0..cols {
                let code = field.code(cx, cy);
                let cost = field.cost(cx, cy);
                if !nav.is_passable(cx, cy) {
                    assert_eq!(code, NO_DIRECTION);
                    assert_eq!(cost, u32::MAX);
                    continue;
                }
                assert_eq!(code == NO_DIRECTION, cost == u32::MAX, "({cx},{cy})");
                if code == NO_DIRECTION {
                    continue;
                }
                let (mut x, mut y, mut d) = (cx, cy, cost);
                let mut steps = 0;
                while d > 0 {
                    let (dx, dy) = NEIGHBOURS[usize::from(field.code(x, y))];
                    let (nx, ny) = ((i64::from(x) + dx) as u32, (i64::from(y) + dy) as u32);
                    assert!(nav.is_passable(nx, ny), "into rock at ({nx},{ny})");
                    if dx != 0 && dy != 0 {
                        assert!(
                            nav.is_passable(nx, y) && nav.is_passable(x, ny),
                            "corner cut"
                        );
                    }
                    let nd = field.cost(nx, ny);
                    assert!(nd < d, "not descending at ({x},{y})");
                    (x, y, d) = (nx, ny, nd);
                    steps += 1;
                    assert!(steps < cols * rows, "cycle");
                }
                assert!(
                    on_edge(x, y, cols, rows, field.edge()),
                    "chain ends off the edge"
                );
            }
        }
    }

    #[test]
    fn chains_reach_the_edge_on_random_grids() {
        for seed in 0..40u64 {
            let nav = random_grid(24, 18, seed);
            for edge in [MapEdge::West, MapEdge::East, MapEdge::South, MapEdge::North] {
                check_chains(&nav, &FlowField::build(&nav, edge));
            }
        }
    }

    #[test]
    fn flat_grid_points_straight_off_the_edge() {
        let nav = NavGrid::from_costs(S::from_i32(4), 10, 10, vec![100; 100]);
        let west = FlowField::build(&nav, MapEdge::West);
        assert_eq!(west.code(0, 5), 2, "seeds point outward");
        assert_eq!(west.code(1, 5), 2, "a straight step beats a diagonal");
        let minus_x = V2::new(-S::ONE, S::ZERO);
        assert_eq!(
            west.direction_at(&nav, V2::from_f32_data(0.0, 20.0)),
            minus_x
        );
        assert_eq!(
            west.direction_at(&nav, V2::from_f32_data(5.9, 20.0)),
            minus_x
        );
        assert!(west.is_exit(&nav, V2::from_f32_data(0.0, 20.0)));
        assert!(west.is_exit(&nav, V2::from_f32_data(3.99, 20.0)));
        assert!(!west.is_exit(&nav, V2::from_f32_data(4.0, 20.0)));
        let north = FlowField::build(&nav, MapEdge::North);
        assert_eq!(
            north.direction_at(&nav, V2::from_f32_data(20.0, 30.0)),
            V2::new(S::ZERO, S::ONE)
        );
        assert!(north.is_exit(&nav, V2::from_f32_data(20.0, 39.0)));
        let east = FlowField::build(&nav, MapEdge::East);
        assert_eq!(east.code(9, 0), 0);
        let south = FlowField::build(&nav, MapEdge::South);
        assert_eq!(south.code(4, 0), 3);
        assert_eq!(south.cost(4, 3), 300);
    }

    #[test]
    fn a_walled_pocket_has_no_direction_and_two_builds_agree() {
        let mut cost = vec![100u16; 100];
        // Ring of rock around cell (5, 5).
        for (x, y) in [
            (4, 4),
            (5, 4),
            (6, 4),
            (4, 5),
            (6, 5),
            (4, 6),
            (5, 6),
            (6, 6),
        ] {
            cost[y * 10 + x] = IMPASSABLE;
        }
        let nav = NavGrid::from_costs(S::from_i32(4), 10, 10, cost);
        let a = FlowField::build(&nav, MapEdge::West);
        assert_eq!(a.code(5, 5), NO_DIRECTION);
        assert_eq!(
            a.direction_at(&nav, V2::from_f32_data(22.0, 22.0)),
            V2::ZERO
        );
        assert!(!a.is_exit(&nav, V2::from_f32_data(22.0, 22.0)));
        check_chains(&nav, &a);
        assert_eq!(a, FlowField::build(&nav, MapEdge::West));
    }
}
