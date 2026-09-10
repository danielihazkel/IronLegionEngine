//! T1-080: A\* and string pulling on the Phase 1 test map's nav grid
//! (TDD §6.1). The path runs from the north-west corner to the south-east
//! corner, so it must cross the river at the bridge or the ford and skirt
//! the forest and the rock.

use std::path::Path;

use criterion::{Criterion, criterion_group, criterion_main};
use il_core::V2;
use il_sim_battle::nav::test_grids::random_grid;
use il_sim_battle::{AStar, BattleWorld, HpaGraph, Pathfinder, string_pull};

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
    // Once outside the timed loop, so a criterion filter that skips this
    // function still leaves `cells` filled for the string-pull bench.
    astar.search_cells(nav, from, to, &mut cells);
    assert!(!cells.is_empty(), "the corners must be connected");
    c.bench_function("astar_test_field_corner_to_corner", |b| {
        b.iter(|| astar.search_cells(nav, from, to, &mut cells))
    });
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

/// T3-020: `HpaGraph::build` on the test map (13 x 10 clusters) and on a
/// synthetic 1600 x 1200 m random grid (400 x 300 cells, ~25 % rock), the
/// shape of T3-024's map; the done-when is under 50 ms for the latter.
fn hpa_build(c: &mut Criterion) {
    let world = world();
    let nav = world.nav_grid();
    c.bench_function("hpa_build_test_field", |b| {
        b.iter(|| HpaGraph::build(nav, 16, 6).node_count())
    });
    let wide = random_grid(400, 300, 1);
    c.bench_function("hpa_build_1600x1200_random_serial", |b| {
        b.iter(|| HpaGraph::build_with(&wide, 16, 6, false).node_count())
    });
    // The per-cluster work on the process-global pool, as the app and
    // `il_cli bench --threads 8` run it.
    let mut pooled = world;
    pooled.set_threads(8);
    c.bench_function("hpa_build_1600x1200_random_8_threads", |b| {
        b.iter(|| HpaGraph::build(&wide, 16, 6).node_count())
    });
    let open = il_sim_battle::NavGrid::from_costs(
        <il_core::S as il_core::Scalar>::from_i32(4),
        400,
        300,
        vec![100; 400 * 300],
    );
    c.bench_function("hpa_build_1600x1200_open_8_threads", |b| {
        b.iter(|| HpaGraph::build(&open, 16, 6).node_count())
    });
}

/// T3-021: one `Hpa::find` corner to corner on the test map (Stage 3
/// serves at most `paths_per_tick` = 8 of these per tick), against the
/// plain A* on the same request.
fn hpa_find(c: &mut Criterion) {
    let world = world();
    let nav = world.nav_grid();
    let mut hpa = world
        .ecs()
        .resource::<il_sim_battle::PathfinderRes>()
        .0
        .clone();
    let (from, to) = (
        V2::from_f32_data(100.0, 100.0),
        V2::from_f32_data(700.0, 500.0),
    );
    let mut out = Vec::new();
    c.bench_function("hpa_find_corner_to_corner", |b| {
        b.iter(|| hpa.find(nav, from, to, &mut out))
    });
    assert!(!out.is_empty());
    let mut astar = AStar::new();
    c.bench_function("astar_find_corner_to_corner", |b| {
        b.iter(|| astar.find(nav, from, to, &mut out))
    });
}

criterion_group!(benches, nav, hpa_build, hpa_find);
criterion_main!(benches);
