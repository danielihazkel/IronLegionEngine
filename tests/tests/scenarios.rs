//! T2-110: scenario outcome bands (REQ-TEST-004, Simulation Spec §15.3,
//! TDD §17).
//!
//! The band files under `tests/scenarios/bands/` are parsed on every push;
//! the 50-seed runs are `#[ignore]`d and the nightly workflow runs them
//! with `cargo test -- --ignored`. Locally:
//! `cargo test --release -p il_tests --test scenarios -- --ignored --nocapture`.
//!
//! Rejected commands (T3-011): a file whose commands are all scripted must
//! reject nothing. A file with an engine-owned side may reject a few
//! through the one-tick race of SIM-CMD-005 (the AI decides at Stage 1 of
//! `t` for `t + 1`; the target can rout or die in between), so those files
//! run a second time with the same seeds and every seed must reject the
//! same count on the same ticks and end on the same hash.

use il_cli::bands::{BandOptions, BandReport, ai_driven, load_band_file, run_bands};
use il_tests::{band_scenario_dir, band_scenario_files, game_root};

fn options() -> BandOptions {
    let mut o = BandOptions::new(band_scenario_dir());
    o.content_root = game_root();
    o.jobs = std::thread::available_parallelism().map_or(4, |n| n.get());
    o
}

#[test]
fn every_band_file_parses_as_a_scenario_with_a_bands_block() {
    for path in band_scenario_files() {
        let (scenario, bands) = load_band_file(&path).unwrap_or_else(|e| panic!("{e:#}"));
        assert!(!bands.assertions.is_empty(), "{}", path.display());
        assert!(
            bands.seeds >= 1 && bands.tick_limit >= 1,
            "{}",
            path.display()
        );
        assert!(scenario.setup.sides.len() >= 2, "{}", path.display());
        // The plain scenario loader must accept the file too (the `bands`
        // key is ignored), so `il_cli run` and the app can open it.
        let plain = il_tests::load_scenario(&path);
        assert_eq!(plain.setup, scenario.setup);
    }
}

/// The scripted files rejected nothing, and the AI-driven files, run a
/// second time under the same options, rejected the same count on the
/// same ticks and ended on the same hash in every seed (T3-011).
fn check_rejections(first: &BandReport, opts: &BandOptions) {
    assert_eq!(
        first.rejected_scripted,
        0,
        "rejected commands in a scripted band file: {:?}",
        first
            .files
            .iter()
            .filter(|f| !f.ai_driven)
            .map(|f| (&f.file, f.outcomes.iter().map(|o| o.rejected).sum::<u32>()))
            .collect::<Vec<_>>()
    );
    let ai_files: Vec<_> = band_scenario_files()
        .into_iter()
        .filter(|p| {
            let (scenario, _) = load_band_file(p).unwrap_or_else(|e| panic!("{e:#}"));
            ai_driven(&scenario)
        })
        .collect();
    assert!(!ai_files.is_empty(), "no AI-driven band file");
    for path in ai_files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let a = first
            .files
            .iter()
            .find(|f| f.file == name)
            .unwrap_or_else(|| panic!("{name}: missing from the first run"));
        assert!(a.ai_driven, "{name}");
        let mut again = opts.clone();
        again.dir = path.clone();
        again.json = None;
        let second = run_bands(&again, &mut std::io::sink()).unwrap_or_else(|e| panic!("{e:#}"));
        let b = &second.files[0];
        assert_eq!(a.outcomes.len(), b.outcomes.len(), "{name}: seed count");
        for (x, y) in a.outcomes.iter().zip(&b.outcomes) {
            assert_eq!(x.seed, y.seed, "{name}");
            assert_eq!(
                x.rejected_by_tick, y.rejected_by_tick,
                "{name}: seed {}: rejections differ between the two runs",
                x.seed
            );
            assert_eq!(
                (x.end_tick, &x.hash),
                (y.end_tick, &y.hash),
                "{name}: seed {}: the two runs end differently",
                x.seed
            );
        }
    }
}

/// Every band file runs one seed for a few ticks: the scripted files
/// without a rejected command (every command they script exists since
/// T2-020), the AI-driven files with the same rejections twice.
#[test]
fn band_files_run_with_only_the_documented_rejections() {
    let mut opts = options();
    opts.seeds = Some(1);
    opts.max_ticks = Some(200);
    let report = run_bands(&opts, &mut std::io::sink()).unwrap_or_else(|e| panic!("{e:#}"));
    check_rejections(&report, &opts);
}

/// The §15.3 bands over their full seed counts (nightly).
#[test]
#[ignore = "50 seeds per band, minutes in release; nightly"]
fn melee_bands_hold() {
    let opts = options();
    let mut out = Vec::new();
    let report = run_bands(&opts, &mut out).unwrap_or_else(|e| panic!("{e:#}"));
    println!("{}", String::from_utf8_lossy(&out));
    check_rejections(&report, &opts);
    assert_eq!(report.failed, 0, "{} band assertions failed", report.failed);
}
