//! T1-082 (Phase 1 exit criterion, REQ-MOD-001): a mod folder that overrides
//! a unit's speed takes effect with no code change.

use il_core::{S, Scalar};
use il_data::{ContentId, Registries};

fn hastati_speed(roots: &[std::path::PathBuf]) -> (S, S, usize) {
    let regs = Registries::load_roots(roots).unwrap_or_else(|e| panic!("{e}"));
    let h = regs
        .units
        .lookup(&ContentId::new("rome:hastati").unwrap())
        .unwrap();
    let u = regs.units.get(h);
    (u.speed_walk, u.speed_run, regs.mods.len())
}

#[test]
fn speed_override_mod_changes_only_speed_walk() {
    let game = il_tests::game_root();
    let mod_dir = il_tests::workspace_root().join("tests/mods/speed_override");

    let (walk, run, mods) = hastati_speed(std::slice::from_ref(&game));
    assert_eq!(walk, S::from_f32_data(1.6));
    assert_eq!(mods, 1);

    let (walk2, run2, mods2) = hastati_speed(&[game, mod_dir]);
    assert_eq!(walk2, S::from_f32_data(3.2), "the mod's speed_walk wins");
    assert_eq!(run2, run, "untouched fields survive the merge");
    assert_eq!(mods2, 2);
}

#[test]
fn il_cli_run_accepts_the_same_mod_folder() {
    let mut opts = il_cli::RunOptions::new(il_tests::scenario_dir().join("idle_1000.json5"), 20);
    opts.content_root = il_tests::game_root();
    opts.mods = vec![il_tests::workspace_root().join("tests/mods/speed_override")];
    opts.hash_every = 10;
    let mut out = Vec::new();
    let hashes = il_cli::run(&opts, &mut out).expect("runs with the mod loaded");
    assert_eq!(hashes.len(), 2);
}
