//! `il_cli bench` end to end (T1-080): the generated setup steps, every
//! stage is reported, the baseline round-trips through `--record-baseline`
//! and `--baseline`, and `--strict` sees a doctored regression.

use std::path::{Path, PathBuf};

use il_cli::bench::{Baseline, BenchOptions, BenchReport, bench, measure};
use il_sim_battle::Stage;

fn game_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game")
}

fn options(dir: &Path) -> BenchOptions {
    let mut o = BenchOptions::new(2_000, 5);
    o.threads = 1;
    o.content_root = game_root();
    o.json = Some(dir.join("report.json"));
    o
}

#[test]
fn measure_reports_every_stage_of_every_tick() {
    let mut o = BenchOptions::new(2_000, 5);
    o.threads = 1;
    o.content_root = game_root();
    let r = measure(&o).unwrap();
    assert_eq!(r.soldiers, 2_000);
    assert_eq!(r.regiments, 10);
    assert_eq!(r.ticks, 5);
    assert_eq!(r.threads, 1);
    assert_eq!(r.stages.len(), Stage::COUNT);
    assert!(r.stages.iter().enumerate().all(|(i, s)| s.index == i));
    assert!(r.tick.mean_ms > 0.0);
    assert!(r.tick.max_ms >= r.tick.p95_ms && r.tick.p95_ms >= r.tick.mean_ms * 0.0);
    assert!(r.phase1_stages_mean_ms > 0.0);
    assert!(r.phase1_stages_mean_ms <= r.tick.mean_ms);
}

#[test]
fn baseline_records_compares_and_strict_flags_a_regression() {
    let dir = std::env::temp_dir().join(format!("il_cli_bench_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let baseline = dir.join("baseline.json");

    // Record.
    let mut o = options(&dir);
    o.record_baseline = Some(baseline.clone());
    o.machine = Some("test machine".to_owned());
    o.recorded = Some("2026-09-03".to_owned());
    let mut out = Vec::new();
    let (report, regressions) = bench(&o, &mut out).unwrap();
    assert!(regressions.is_empty());
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("bench: 2000 soldiers in 10 regiments, 5 ticks, 1 thread(s)"));
    assert!(text.contains(" 7 Collision"));
    let json: BenchReport =
        serde_json::from_str(&std::fs::read_to_string(dir.join("report.json")).unwrap()).unwrap();
    assert_eq!(json, report);
    let mut b = Baseline::load(&baseline).unwrap();
    assert_eq!(b.machine, "test machine");
    assert_eq!(b.run_for(2_000), Some(&report));

    // Doctor the baseline: Collision and the tick ten times faster than
    // measured, every other stage absurdly slow.
    let run = b.runs.get_mut("2000").unwrap();
    for s in &mut run.stages {
        s.summary.mean_ms = if s.name == "Collision" {
            s.summary.mean_ms * 0.1
        } else {
            1_000.0
        };
    }
    run.tick.mean_ms *= 0.1;
    b.save(&baseline).unwrap();

    let mut o = options(&dir);
    o.baseline = Some(baseline.clone());
    o.strict = true;
    let mut out = Vec::new();
    let (_, regressions) = bench(&o, &mut out).unwrap();
    let names: Vec<&str> = regressions.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["Collision", "tick"]);
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("regression: Collision is +"));
    assert!(text.contains("base ms"));

    // A baseline without this soldier count compares nothing.
    let mut o = options(&dir);
    o.soldiers = 10_000;
    o.ticks = 1;
    o.baseline = Some(baseline.clone());
    let mut out = Vec::new();
    let (r, regressions) = bench(&o, &mut out).unwrap();
    assert_eq!(r.regiments, 50);
    assert!(regressions.is_empty());
    assert!(
        String::from_utf8(out)
            .unwrap()
            .contains("no run for 10000 soldiers")
    );

    std::fs::remove_dir_all(&dir).unwrap();
}
