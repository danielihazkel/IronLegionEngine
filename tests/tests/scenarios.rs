//! T2-110: scenario outcome bands (REQ-TEST-004, Simulation Spec §15.3,
//! TDD §17).
//!
//! The band files under `tests/scenarios/bands/` are parsed on every push;
//! the 50-seed runs are `#[ignore]`d and the nightly workflow runs them
//! with `cargo test -- --ignored`. Locally:
//! `cargo test --release -p il_tests --test scenarios -- --ignored --nocapture`.

use il_cli::bands::{BandOptions, load_band_file, run_bands};
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

/// Every band file runs one seed for a few ticks without a rejected
/// command. `AttackRegiment` returns `NotImplemented` until T2-020.
#[test]
#[ignore = "AttackRegiment is NotImplemented until T2-020"]
fn band_files_run_without_rejected_commands() {
    let mut opts = options();
    opts.seeds = Some(1);
    opts.max_ticks = Some(200);
    let report = run_bands(&opts, &mut std::io::sink()).unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(report.rejected, 0, "rejected commands: {report:?}");
}

/// The §15.3 bands over their full seed counts (nightly).
#[test]
#[ignore = "50 seeds per band, minutes in release; nightly"]
fn melee_bands_hold() {
    let opts = options();
    let mut out = Vec::new();
    let report = run_bands(&opts, &mut out).unwrap_or_else(|e| panic!("{e:#}"));
    println!("{}", String::from_utf8_lossy(&out));
    assert_eq!(report.rejected, 0, "rejected commands");
    assert_eq!(report.failed, 0, "{} band assertions failed", report.failed);
}
