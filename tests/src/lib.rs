//! Workspace integration tests: determinism, dependency rules, content
//! validation (TDD §17). This library holds the shared helpers; the tests
//! live in `tests/tests/` and the scenario files in `tests/scenarios/`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use il_data::Registries;
use il_sim_battle::BattleSetup;

pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("tests/ lives directly under the workspace root")
        .to_path_buf()
}

pub fn game_root() -> PathBuf {
    workspace_root().join("game")
}

pub fn scenario_dir() -> PathBuf {
    workspace_root().join("tests/scenarios")
}

/// Every `*.json5` scenario, sorted by path.
pub fn scenario_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(scenario_dir())
        .expect("tests/scenarios exists")
        .map(|e| e.expect("readable entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "json5"))
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "no scenarios under {}",
        scenario_dir().display()
    );
    files
}

pub fn load_scenario(path: &Path) -> BattleSetup {
    il_cli::load_setup(path).unwrap_or_else(|e| panic!("{e:#}"))
}

pub fn game_regs() -> Arc<Registries> {
    il_cli::load_registries(&game_root()).unwrap_or_else(|e| panic!("{e:#}"))
}
