//! T3-020: the HPA* graph on the hand-written tiny map and on the flagship
//! test map (golden gate counts), and the same graph at 1 and 8 threads.

mod common;

use std::path::Path;

use il_data::{ContentId, Registries};
use il_sim_battle::{HpaGraph, LoadedMap, NavGrid};

fn regs_with_tiny() -> Registries {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    Registries::load_roots(&[root.join("game"), root.join("tests/maps")])
        .unwrap_or_else(|d| panic!("{d}"))
}

fn nav_of(regs: &Registries, id: &str) -> NavGrid {
    let h = regs
        .maps
        .lookup(&ContentId::new(id).unwrap())
        .unwrap_or_else(|| panic!("{id} registered"));
    let map = LoadedMap::from_def(regs.maps.get(h), regs.rules.movement.zone_cell).unwrap();
    NavGrid::from_map(&map, regs, &regs.rules.movement)
}

#[test]
fn tiny_map_gates_are_golden() {
    // 2 x 2 nav cells; the river row is impassable except where the ford
    // covers it, and the rock triangle blocks the south-east cell, so the
    // only gate is between the two northern cells. One-cell clusters make
    // every cell its own cluster.
    let regs = regs_with_tiny();
    let nav = nav_of(&regs, "tiny:tiny");
    assert_eq!((nav.cols(), nav.rows()), (2, 2));
    let g = HpaGraph::build(&nav, 1, 6);
    assert_eq!(g.cluster_count(), 4);
    let cells: Vec<(u32, u32)> = g
        .nodes()
        .iter()
        .map(|n| nav.coords(n.cell as usize))
        .collect();
    assert_eq!(cells, vec![(0, 0), (1, 0)], "{cells:?}");
    assert_eq!(g.edges(0), &[(1, 100)]);
    assert_eq!(g.edges(1), &[(0, 150)], "back into the forest cell");
}

#[test]
fn test_field_gates_are_golden_and_the_banks_connect() {
    let regs = common::regs();
    let w = common::world(10);
    let nav = w.nav_grid();
    let rules = &regs.rules.movement;
    assert_eq!((rules.hpa_cluster, rules.hpa_gate_split), (16, 6));
    let g = HpaGraph::build(&nav.clone(), 16, 6);
    assert_eq!((g.clusters_x(), g.clusters_y()), (13, 10));
    // Golden: pinned from the first build on 2026-09-10; a change here means
    // the map, the nav grid or the gate rule changed.
    assert_eq!(g.node_count(), GOLDEN_TEST_FIELD_NODES, "gate nodes");
    // The north bank reaches the south bank through the abstract graph
    // (the bridge and the ford gates), and nothing reaches the rock.
    let start = g
        .nodes()
        .iter()
        .position(|n| nav.coords(n.cell as usize).1 < 60)
        .expect("a gate north of the river") as u32;
    let mut seen = vec![false; g.node_count()];
    let mut stack = vec![start];
    while let Some(u) = stack.pop() {
        if std::mem::replace(&mut seen[u as usize], true) {
            continue;
        }
        for &(v, _) in g.edges(u) {
            stack.push(v);
        }
    }
    let south = g
        .nodes()
        .iter()
        .enumerate()
        .any(|(id, n)| seen[id] && nav.coords(n.cell as usize).1 > 90);
    assert!(south, "the abstract graph crosses the river");
    for n in g.nodes() {
        let (cx, cy) = nav.coords(n.cell as usize);
        assert!(nav.is_passable(cx, cy));
    }
}

const GOLDEN_TEST_FIELD_NODES: usize = 1382;

#[test]
fn the_graph_is_identical_at_one_and_eight_threads() {
    let mut w = common::world(10);
    let serial = HpaGraph::build_with(w.nav_grid(), 16, 6, false);
    w.set_threads(8);
    let parallel = HpaGraph::build_with(w.nav_grid(), 16, 6, true);
    assert_eq!(serial, parallel);
    // The world's own graph (built by `new` through `rebuild_derived`).
    let from_world = w
        .ecs()
        .resource::<il_sim_battle::PathfinderRes>()
        .0
        .graph()
        .clone();
    assert_eq!(from_world, serial);
}
