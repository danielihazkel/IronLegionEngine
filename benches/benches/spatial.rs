//! T1-031 done-when: rebuilding the soldier grid from 32k entries takes
//! under 0.5 ms (TDD §5 budget: 32k inserts ≈ 0.3 ms).

use criterion::{Criterion, criterion_group, criterion_main};
use il_benches::scattered_soldiers;
use il_core::{S, Scalar, SoldierId};
use il_sim_battle::SpatialGrid;

fn spatial_grid(c: &mut Criterion) {
    let entries = scattered_soldiers(32_768, 2_000, 2_000, 7);
    let mut grid: SpatialGrid<SoldierId> =
        SpatialGrid::new(S::from_i32(2_000), S::from_i32(2_000), S::from_i32(4));
    c.bench_function("spatial_grid_rebuild_32k", |b| {
        b.iter(|| grid.rebuild(entries.iter().copied()));
    });
    grid.rebuild(entries.iter().copied());
    c.bench_function("spatial_grid_for_each_pair_32k", |b| {
        b.iter(|| {
            let mut n = 0u64;
            grid.for_each_pair(|_, _| n += 1);
            n
        });
    });
    let mut out = Vec::new();
    c.bench_function("spatial_grid_query_circle_r8", |b| {
        b.iter(|| {
            grid.query_circle(
                il_core::V2::new(S::from_i32(1_000), S::from_i32(1_000)),
                S::from_i32(8),
                &mut out,
            );
            out.len()
        });
    });
}

criterion_group!(benches, spatial_grid);
criterion_main!(benches);
