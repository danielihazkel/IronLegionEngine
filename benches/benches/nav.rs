//! T1-080: A\* and string pulling on the Phase 1 test map's nav grid
//! (TDD §6.1). The path runs from the north-west corner to the south-east
//! corner, so it must cross the river at the bridge or the ford and skirt
//! the forest and the rock.

use std::path::Path;

use criterion::{Criterion, criterion_group, criterion_main};
use il_core::V2;
use il_sim_battle::{AStar, BattleWorld, string_pull};

fn world() -> BattleWorld {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../game");
    let regs = il_cli::load_registries(&root).unwrap_or_else(|e| panic!("{e}"));
    let scenario = il_cli::bench::generate_scenario(2_000).unwrap();
    BattleWorld::new(&scenario.setup, regs).unwrap_or_else(|e| panic!("{e}"))
}

fn nav(c: &mut Criterion) {
    let world = world();
    let nav = world.nav_grid();
    let from = nav.cell_of(V2::from_f32_data(100.0, 100.0));
    let to = nav.cell_of(V2::from_f32_data(700.0, 500.0));
    let mut astar = AStar::new();
    let mut cells = Vec::new();
    c.bench_function("astar_test_field_corner_to_corner", |b| {
        b.iter(|| astar.search_cells(nav, from, to, &mut cells))
    });
    assert!(!cells.is_empty(), "the corners must be connected");
    let points: Vec<V2> = cells.iter().map(|&(x, y)| nav.cell_center(x, y)).collect();
    let mut pulled = Vec::with_capacity(points.len());
    c.bench_function("string_pull_corner_to_corner", |b| {
        b.iter(|| {
            pulled.clear();
            pulled.extend_from_slice(&points);
            string_pull(nav, &mut pulled);
            pulled.len()
        })
    });
}

criterion_group!(benches, nav);
criterion_main!(benches);
