//! `il_cli autoresolve` end to end (T2-071): the four-phase scenario ends
//! with a winner and prints a result that parses back; a tick cap that cuts
//! the battle short reports no winner and `ended == false`.

use std::path::{Path, PathBuf};

use il_cli::autoresolve::{AutoresolveOptions, autoresolve};
use il_sim_battle::BattleResult;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn options(max_ticks: Option<u32>) -> AutoresolveOptions {
    AutoresolveOptions {
        scenario: root().join("tests/scenarios/phases_all_four.json5"),
        max_ticks,
        threads: 1,
        json: None,
        content_root: root().join("game"),
        mods: Vec::new(),
    }
}

#[test]
fn the_four_phase_scenario_resolves_to_a_winner() {
    let mut out = Vec::new();
    let (result, ended) = autoresolve(&options(None), &mut out).unwrap();
    assert!(ended);
    assert_eq!(result.winner, Some(0));
    let parsed: BattleResult = serde_json::from_slice(&out).unwrap();
    assert_eq!(parsed, result);
    assert!(result.duration_ticks > 0);
}

#[test]
fn a_tick_cap_cuts_the_battle_short_without_a_winner() {
    let mut out = Vec::new();
    let (result, ended) = autoresolve(&options(Some(5)), &mut out).unwrap();
    assert!(!ended);
    assert_eq!(result.winner, None);
    assert_eq!(result.duration_ticks, 0, "still deploying at tick 5");
}
