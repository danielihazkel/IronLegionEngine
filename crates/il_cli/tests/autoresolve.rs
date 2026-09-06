//! `il_cli autoresolve` end to end (T2-071): the four-phase scenario ends
//! with a winner and prints a result that parses back; a tick cap that cuts
//! the battle short reports no winner and `ended == false`.

use std::path::{Path, PathBuf};

use il_cli::autoresolve::{AiPlayers, AutoresolveOptions, autoresolve};
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
        ai: AiPlayers::None,
    }
}

/// T2-082 (plan decision 17): by default every side goes to the engine and
/// the script is dropped; the AI-versus-AI skirmish ends with a winner.
/// (The four-phase scenario's blind armies aim at each other's zone
/// centres, which on the test map never brings them within sight.)
#[test]
fn the_default_hands_every_side_to_the_engine() {
    let mut out = Vec::new();
    let opts = AutoresolveOptions {
        ai: AiPlayers::All,
        scenario: root().join("tests/scenarios/ai_skirmish_300.json5"),
        max_ticks: Some(12_000),
        ..options(None)
    };
    let (result, ended) = autoresolve(&opts, &mut out).unwrap();
    assert!(ended);
    assert!(result.winner.is_some(), "{result:?}");
    assert!(result.duration_ticks > 0);
    let total_killed = result.summary.total_killed;
    assert!(total_killed > 0, "the armies fought: {result:?}");
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
