//! T0-050: determinism over every scenario (REQ-TEST-002, TDD §17).
//!
//! For each file in `tests/scenarios/`: run the file's determinism budget
//! (`determinism: { ticks, snapshot_at }`, default 10,000 / 5,000; T2-111)
//! with 1 thread and with 8 threads, feeding the scenario's scripted
//! commands (T1-081), and compare the per-tick hash vectors; snapshot the
//! 1-thread run at `snapshot_at`, restore into a fresh world, run to the
//! end and compare the tail. Failures name the first divergent tick.

use il_core::StateHash;
use il_sim_battle::{BattleWorld, ScriptedCommands, Snapshot};
use il_tests::{game_regs, load_scenario, scenario_files};

const THREADS: usize = 8;

fn first_divergence(a: &[StateHash], b: &[StateHash], offset: u32) -> Option<u32> {
    a.iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .map(|i| offset + i as u32 + 1)
        .or_else(|| (a.len() != b.len()).then_some(offset + a.len().min(b.len()) as u32 + 1))
}

/// Runs `world` up to `until` completed ticks feeding `script`, returning
/// one hash per tick.
fn run_to(world: &mut BattleWorld, script: &mut ScriptedCommands, until: u32) -> Vec<StateHash> {
    let mut hashes = Vec::with_capacity((until - world.tick().0) as usize);
    while world.tick().0 < until {
        let commands = script.take_for(world.tick().next());
        let out = world.step(&commands);
        assert!(
            out.rejected.is_empty(),
            "tick {}: rejected {:?}",
            world.tick().0,
            out.rejected
        );
        hashes.push(out.hash);
    }
    hashes
}

#[test]
fn every_scenario_is_deterministic_across_threads_and_restore() {
    let regs = game_regs();
    for path in scenario_files() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let scenario = load_scenario(&path);
        let setup = &scenario.setup;
        let budget = scenario.determinism_budget();
        let (ticks, snapshot_at) = (budget.ticks, budget.snapshot_at);
        assert!(
            snapshot_at > 0 && snapshot_at < ticks,
            "{name}: determinism budget {budget:?}"
        );

        // Reference run: one thread, snapshot at the midpoint.
        let mut reference = BattleWorld::new(setup, regs.clone()).unwrap();
        reference.set_threads(1);
        let mut script = scenario.script();
        let mut ref_hashes = run_to(&mut reference, &mut script, snapshot_at);
        let snapshot_bytes = reference.snapshot().to_bytes();
        ref_hashes.extend(run_to(&mut reference, &mut script, ticks));
        assert_eq!(ref_hashes.len(), ticks as usize);
        assert_eq!(script.remaining(), 0, "{name}: commands left unfed");

        // Same again on one thread: the run must reproduce itself.
        let mut again = BattleWorld::new(setup, regs.clone()).unwrap();
        again.set_threads(1);
        let again_hashes = run_to(&mut again, &mut scenario.script(), ticks);
        if let Some(t) = first_divergence(&ref_hashes, &again_hashes, 0) {
            panic!("{name}: two 1-thread runs diverge at tick {t}");
        }

        // Multi-threaded executor.
        let mut threaded = BattleWorld::new(setup, regs.clone()).unwrap();
        threaded.set_threads(THREADS);
        assert_eq!(threaded.threads(), THREADS);
        let threaded_hashes = run_to(&mut threaded, &mut scenario.script(), ticks);
        if let Some(t) = first_divergence(&ref_hashes, &threaded_hashes, 0) {
            panic!("{name}: 1-thread and {THREADS}-thread runs diverge at tick {t}");
        }

        // Snapshot, restore into a fresh world, continue to the end.
        let snapshot = Snapshot::from_bytes(&snapshot_bytes).unwrap();
        let mut restored = BattleWorld::restore(&snapshot, regs.clone()).unwrap();
        assert_eq!(restored.tick().0, snapshot_at);
        assert_eq!(
            restored.hash(),
            ref_hashes[snapshot_at as usize - 1],
            "{name}: hash(restore(snapshot)) differs at tick {snapshot_at}"
        );
        restored.set_threads(THREADS);
        let mut script = scenario.script();
        script.take_for(restored.tick());
        let tail = run_to(&mut restored, &mut script, ticks);
        if let Some(t) = first_divergence(&ref_hashes[snapshot_at as usize..], &tail, snapshot_at) {
            panic!("{name}: restored run diverges from the uninterrupted run at tick {t}");
        }
    }
}

#[test]
fn cli_run_prints_the_same_hashes_as_the_library() {
    // Mirrors the exit checklist: `il_cli run idle_1000 --ticks 10000
    // --hash-every 1000` twice gives identical output.
    let path = il_tests::scenario_dir().join("idle_1000.json5");
    let mut opts = il_cli::RunOptions::new(&path, 2_000);
    opts.hash_every = 500;
    opts.content_root = il_tests::game_root();
    let mut out_a = Vec::new();
    let a = il_cli::run(&opts, &mut out_a).unwrap();
    let mut out_b = Vec::new();
    let b = il_cli::run(&opts, &mut out_b).unwrap();
    assert_eq!(a, b);
    assert_eq!(out_a, out_b);
    assert_eq!(a.len(), 4);
    let text = String::from_utf8(out_a).unwrap();
    assert_eq!(text.lines().count(), 4);
    assert!(text.starts_with("500,"));
    assert_eq!(text.lines().next().unwrap().len(), "500,".len() + 16);
}
