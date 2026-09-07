//! T0-050: determinism over every scenario (REQ-TEST-002, TDD §17).
//!
//! For each file in `tests/scenarios/` (the classic corpus), each band file
//! under `tests/scenarios/bands/` (T2-112, stepped through the band
//! harness's `SeedDriver` so its morale pins and general kills apply) and
//! the 10k fight `perf_10k.json5`: run the file's determinism budget
//! (`determinism: { ticks, snapshot_at }`, default 10,000 / 5,000) with 1
//! thread and with 8 threads, feeding the scenario's scripted commands
//! (T1-081), and compare the per-tick hash vectors; snapshot the 1-thread
//! run at `snapshot_at`, restore into a fresh world, run to the end and
//! compare the tail. Failures name the first divergent tick. The three
//! corpora are separate tests so cargo runs them side by side.

use std::path::{Path, PathBuf};

use il_cli::bands::{HarnessEvent, SeedDriver, load_band_file};
use il_core::{PlayerId, StateHash};
use il_sim_battle::{Scenario, Snapshot};
use il_tests::{band_scenario_files, game_regs, load_scenario, scenario_files};

const THREADS: usize = 8;

fn first_divergence(a: &[StateHash], b: &[StateHash], offset: u32) -> Option<u32> {
    a.iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .map(|i| offset + i as u32 + 1)
        .or_else(|| (a.len() != b.len()).then_some(offset + a.len().min(b.len()) as u32 + 1))
}

/// How a file is driven: plain scenarios reject nothing; band and perf
/// files (AI-driven, with the documented one-tick race) must reject the
/// same count every tick instead (plan I13).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Strictness {
    NoRejections,
    SameRejections,
}

/// One run's record: the hash after every tick and the rejections per tick.
struct Trace {
    hashes: Vec<StateHash>,
    rejected: Vec<u32>,
}

/// Steps `driver` to `until` completed ticks.
fn run_to(driver: &mut SeedDriver, until: u32, name: &str, strict: Strictness) -> Trace {
    let mut trace = Trace {
        hashes: Vec::with_capacity((until - driver.tick().0) as usize),
        rejected: Vec::new(),
    };
    while driver.tick().0 < until {
        let (out, hash) = driver.step();
        if strict == Strictness::NoRejections {
            assert!(
                out.rejected.is_empty(),
                "{name}: tick {}: rejected {:?}",
                driver.tick().0,
                out.rejected
            );
        }
        trace.rejected.push(out.rejected.len() as u32);
        trace.hashes.push(hash);
    }
    trace
}

fn check_corpus(files: &[(PathBuf, Scenario, Vec<u8>, Vec<HarnessEvent>)], strict: Strictness) {
    let regs = game_regs();
    for (path, scenario, pins, harness) in files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let budget = scenario.determinism_budget();
        let (ticks, snapshot_at) = (budget.ticks, budget.snapshot_at);
        assert!(
            snapshot_at > 0 && snapshot_at < ticks,
            "{name}: determinism budget {budget:?}"
        );
        let new = |threads: usize| {
            SeedDriver::new(scenario, None, pins, harness, regs.clone(), threads)
                .unwrap_or_else(|e| panic!("{name}: {e:#}"))
        };

        // Reference run: one thread, snapshot at the midpoint.
        let mut reference = new(1);
        let mut trace = run_to(&mut reference, snapshot_at, &name, strict);
        let snapshot_bytes = reference.snapshot().to_bytes();
        let tail = run_to(&mut reference, ticks, &name, strict);
        trace.hashes.extend(tail.hashes);
        trace.rejected.extend(tail.rejected);
        assert_eq!(trace.hashes.len(), ticks as usize);

        // Same again on one thread: the run must reproduce itself.
        let again = run_to(&mut new(1), ticks, &name, strict);
        if let Some(t) = first_divergence(&trace.hashes, &again.hashes, 0) {
            panic!("{name}: two 1-thread runs diverge at tick {t}");
        }
        assert_eq!(
            trace.rejected, again.rejected,
            "{name}: rejections differ between 1-thread runs"
        );

        // Multi-threaded executor.
        let mut threaded = new(THREADS);
        assert_eq!(threaded.world.threads(), THREADS);
        let threaded = run_to(&mut threaded, ticks, &name, strict);
        if let Some(t) = first_divergence(&trace.hashes, &threaded.hashes, 0) {
            panic!("{name}: 1-thread and {THREADS}-thread runs diverge at tick {t}");
        }
        assert_eq!(
            trace.rejected, threaded.rejected,
            "{name}: rejections differ across threads"
        );

        // Snapshot, restore into a fresh world, continue to the end.
        let snapshot = Snapshot::from_bytes(&snapshot_bytes).unwrap();
        let mut restored =
            SeedDriver::restored(&snapshot, scenario, pins, harness, regs.clone(), THREADS)
                .unwrap_or_else(|e| panic!("{name}: {e:#}"));
        assert_eq!(restored.tick().0, snapshot_at);
        assert_eq!(
            restored.world.hash(),
            trace.hashes[snapshot_at as usize - 1],
            "{name}: hash(restore(snapshot)) differs at tick {snapshot_at}"
        );
        let tail = run_to(&mut restored, ticks, &name, strict);
        if let Some(t) = first_divergence(
            &trace.hashes[snapshot_at as usize..],
            &tail.hashes,
            snapshot_at,
        ) {
            panic!("{name}: restored run diverges from the uninterrupted run at tick {t}");
        }
        assert_eq!(
            trace.rejected[snapshot_at as usize..],
            tail.rejected[..],
            "{name}: rejections differ after the restore"
        );
    }
}

fn plain(path: &Path) -> (PathBuf, Scenario, Vec<u8>, Vec<HarnessEvent>) {
    (
        path.to_path_buf(),
        load_scenario(path),
        Vec::new(),
        Vec::new(),
    )
}

/// A scenario with an engine-owned side (plan I13): its AI may hit the
/// documented one-tick race, so its rejections are compared, not banned.
fn ai_driven(scenario: &Scenario) -> bool {
    scenario
        .setup
        .sides
        .iter()
        .any(|s| s.player == PlayerId::ENGINE_AI)
}

/// The classic corpus: every top-level scenario but the 10k fight.
#[test]
fn every_scenario_is_deterministic_across_threads_and_restore() {
    let files: Vec<_> = scenario_files()
        .iter()
        .filter(|p| p.file_stem().is_none_or(|s| s != "perf_10k"))
        .map(|p| plain(p))
        .collect();
    assert!(!files.is_empty());
    let (ai, scripted): (Vec<_>, Vec<_>) = files.into_iter().partition(|f| ai_driven(&f.1));
    check_corpus(&scripted, Strictness::NoRejections);
    check_corpus(&ai, Strictness::SameRejections);
}

/// The band files (T2-112): pins and harness applied, the file's own seed.
#[test]
fn every_band_scenario_is_deterministic_across_threads_and_restore() {
    let files: Vec<_> = band_scenario_files()
        .iter()
        .map(|p| {
            let (scenario, bands) = load_band_file(p).unwrap_or_else(|e| panic!("{e:#}"));
            (
                p.clone(),
                scenario,
                bands.pin_morale.clone(),
                bands.harness.clone(),
            )
        })
        .collect();
    assert!(!files.is_empty());
    check_corpus(&files, Strictness::SameRejections);
}

/// The 10k fight (T2-111/T2-112) for its short budget.
#[test]
fn the_ten_thousand_fight_is_deterministic_across_threads_and_restore() {
    let path = il_tests::scenario_dir().join("perf_10k.json5");
    check_corpus(&[plain(&path)], Strictness::SameRejections);
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
