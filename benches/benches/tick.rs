//! T1-080: one full `BattleWorld::step` at 2k, 10k and 20k soldiers on the
//! generated bench setup, 8 threads. Each iteration advances the same world
//! by one tick, feeding the scripted move/reform stream while it lasts, so
//! the sample mixes moving and idle ticks; `il_cli bench` is the per-stage
//! view of the same run.

use std::path::Path;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use il_sim_battle::BattleWorld;

fn tick(c: &mut Criterion) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../game");
    let regs = il_cli::load_registries(&root).unwrap_or_else(|e| panic!("{e}"));
    let mut group = c.benchmark_group("tick_move_reform");
    group.sample_size(20);
    for soldiers in [2_000u32, 10_000, 20_000] {
        let scenario = il_cli::bench::generate_scenario(soldiers).unwrap();
        let mut script = scenario.script();
        let mut world =
            BattleWorld::new(&scenario.setup, regs.clone()).unwrap_or_else(|e| panic!("{e}"));
        world.set_threads(8);
        group.bench_with_input(BenchmarkId::from_parameter(soldiers), &soldiers, |b, _| {
            b.iter(|| {
                let commands = script.take_for(world.tick().next());
                world.step(&commands).hash
            })
        });
    }
    group.finish();
}

criterion_group!(benches, tick);
criterion_main!(benches);
