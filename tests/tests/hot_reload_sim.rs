//! T1-025 done-when (sim half): editing `speed_walk` during a battle
//! changes regiment speed on the next tick, through `HotReload` and
//! `BattleWorld::replace_registries` exactly as `il_app` wires them.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use il_core::{PlayerId, RegimentId, Scalar, Tick, V2};
use il_data::hot_reload::HotReload;
use il_data::{ModSet, discover};
use il_sim_battle::{BattleWorld, Command, CommandKind, SpeedMode};

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A private copy of `game/` to edit.
fn game_copy() -> PathBuf {
    let root = il_tests::workspace_root().join("target/il_tests/hot_reload_sim/game");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let game = il_tests::game_root();
    std::fs::copy(game.join("mod.json5"), root.join("mod.json5")).unwrap();
    for dir in ["content", "locale", "assets"] {
        copy_dir(&game.join(dir), &root.join(dir));
    }
    root
}

fn walk(world: &mut BattleWorld, ticks: u32) -> f32 {
    let start = world.view().regiment(RegimentId(0)).unwrap().anchor_pos;
    for _ in 0..ticks {
        world.step(&[]);
    }
    let end = world.view().regiment(RegimentId(0)).unwrap().anchor_pos;
    start.distance(end).to_f32_render()
}

#[test]
fn a_hot_reloaded_speed_walk_changes_the_pace_on_the_next_tick() {
    let root = game_copy();
    let found = discover(std::slice::from_ref(&root)).unwrap();
    let set = ModSet::all(&found).unwrap();
    let regs = Arc::new(il_data::load(&set).unwrap_or_else(|e| panic!("{e}")));
    let mut hot = HotReload::new(set, regs.clone()).expect("watcher starts");

    let scenario = il_tests::load_scenario(&il_tests::scenario_dir().join("idle_1000.json5"));
    let mut world = BattleWorld::new(&scenario.setup, regs).unwrap();
    let order = Command {
        tick: Tick(1),
        player: PlayerId(0),
        seq: 0,
        kind: CommandKind::Move {
            regiments: vec![RegimentId(0)],
            target: V2::from_f32_data(300.0, 260.0),
            facing: None,
            speed: SpeedMode::Walk,
        },
    };
    assert!(world.step(&[order]).rejected.is_empty());
    // Let the wheel finish, then measure 10 s of walking.
    walk(&mut world, 60);
    let before = walk(&mut world, 200);

    // Edit the unit file in place and rebuild, as the app does on a
    // watcher event, then swap the registries into the running world.
    let unit = root.join("content/units/hastati.json5");
    let text = std::fs::read_to_string(&unit).unwrap();
    assert!(text.contains("speed_walk: 1.6,"));
    std::fs::write(&unit, text.replace("speed_walk: 1.6,", "speed_walk: 3.2,")).unwrap();
    let reloaded = hot.rebuild_now().expect("rebuild succeeds");
    world.replace_registries(reloaded);

    let after = walk(&mut world, 200);
    assert!(
        after > before * 1.6,
        "{after} m after the reload vs {before} m before"
    );
}
