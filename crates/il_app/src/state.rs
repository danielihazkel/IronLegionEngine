//! The app state machine (T1-070, SAD §6.1, TDD §15): `MainMenu` and
//! `Battle` in Phase 1 (`Campaign` and `Editor` arrive with their phases).
//! Transitions are pure so they can be tested without a window: the caller
//! supplies the function that turns a scenario path into a session.

use std::path::{Path, PathBuf};

use crate::session::BattleSession;

pub enum AppState {
    MainMenu(MenuState),
    Battle(Box<BattleSession>),
}

/// What the main menu shows: the scenario files it found, the mod roots in
/// load order, and the last failure to start a battle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuState {
    pub scenarios: Vec<PathBuf>,
    pub mods: Vec<PathBuf>,
    pub error: Option<String>,
}

impl MenuState {
    /// Lists `*.json5` under `scenarios_dir`, sorted by name (a missing
    /// directory is an empty list, not an error).
    pub fn scan(scenarios_dir: &Path, mods: Vec<PathBuf>) -> Self {
        let mut scenarios: Vec<PathBuf> = std::fs::read_dir(scenarios_dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json5"))
            .collect();
        scenarios.sort();
        Self {
            scenarios,
            mods,
            error: None,
        }
    }
}

/// What the UI asked for this frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transition {
    /// Main menu: start the custom battle in this scenario file.
    StartBattle(PathBuf),
    /// Battle: back to the main menu (the session is dropped).
    QuitToMenu,
}

impl AppState {
    pub fn is_battle(&self) -> bool {
        matches!(self, AppState::Battle(_))
    }

    pub fn session(&self) -> Option<&BattleSession> {
        match self {
            AppState::Battle(s) => Some(s),
            AppState::MainMenu(_) => None,
        }
    }

    pub fn session_mut(&mut self) -> Option<&mut BattleSession> {
        match self {
            AppState::Battle(s) => Some(s),
            AppState::MainMenu(_) => None,
        }
    }

    /// Applies a transition. `start` builds the session for a scenario; on
    /// failure the menu stays up and shows the error. `menu` rebuilds the
    /// menu when a battle quits.
    pub fn apply(
        self,
        transition: Transition,
        start: impl FnOnce(&Path) -> anyhow::Result<BattleSession>,
        menu: impl FnOnce() -> MenuState,
    ) -> Self {
        match (self, transition) {
            (AppState::MainMenu(mut m), Transition::StartBattle(path)) => match start(&path) {
                Ok(session) => AppState::Battle(Box::new(session)),
                Err(e) => {
                    m.error = Some(format!("{}: {e:#}", path.display()));
                    AppState::MainMenu(m)
                }
            },
            (AppState::Battle(_), Transition::QuitToMenu) => AppState::MainMenu(menu()),
            // Starting from a battle or quitting from the menu is a no-op.
            (state, _) => state,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use il_core::PlayerId;
    use il_data::Registries;
    use il_sim_battle::{BattlePhase, BattleWorld, ScriptedCommands};
    use std::sync::Arc;

    fn session(_: &Path) -> anyhow::Result<BattleSession> {
        let world = BattleWorld::empty(1, Arc::new(Registries::default()), BattlePhase::Battle);
        Ok(BattleSession::new(
            world,
            PlayerId(0),
            ScriptedCommands::default(),
        ))
    }

    fn failing(p: &Path) -> anyhow::Result<BattleSession> {
        anyhow::bail!("no such scenario {}", p.display())
    }

    fn menu() -> MenuState {
        MenuState {
            scenarios: vec![PathBuf::from("a.json5")],
            ..MenuState::default()
        }
    }

    #[test]
    fn menu_starts_a_battle_and_a_battle_quits_to_the_menu() {
        let state = AppState::MainMenu(menu());
        let state = state.apply(
            Transition::StartBattle(PathBuf::from("a.json5")),
            session,
            menu,
        );
        assert!(state.is_battle());
        assert!(state.session().is_some());
        let state = state.apply(Transition::QuitToMenu, session, menu);
        assert!(!state.is_battle());
        match state {
            AppState::MainMenu(m) => assert_eq!(m, menu()),
            AppState::Battle(_) => unreachable!(),
        }
    }

    #[test]
    fn a_failed_start_stays_in_the_menu_with_the_error() {
        let state = AppState::MainMenu(menu()).apply(
            Transition::StartBattle(PathBuf::from("missing.json5")),
            failing,
            menu,
        );
        match state {
            AppState::MainMenu(m) => {
                let e = m.error.expect("error shown");
                assert!(
                    e.contains("missing.json5") && e.contains("no such scenario"),
                    "{e}"
                );
                assert_eq!(m.scenarios, menu().scenarios, "the list survives");
            }
            AppState::Battle(_) => panic!("must not enter a battle"),
        }
    }

    #[test]
    fn mismatched_transitions_are_ignored() {
        let state = AppState::MainMenu(menu()).apply(Transition::QuitToMenu, session, menu);
        assert!(!state.is_battle());
        let battle = AppState::Battle(Box::new(session(Path::new("x")).unwrap()));
        let tick = battle.session().unwrap().world.tick();
        let battle = battle.apply(
            Transition::StartBattle(PathBuf::from("b.json5")),
            failing,
            menu,
        );
        assert!(battle.is_battle());
        assert_eq!(battle.session().unwrap().world.tick(), tick);
    }

    #[test]
    fn scan_lists_json5_files_sorted_and_tolerates_a_missing_dir() {
        let dir = std::env::temp_dir().join(format!("il_app_scan_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.json5"), "{}").unwrap();
        std::fs::write(dir.join("a.json5"), "{}").unwrap();
        std::fs::write(dir.join("notes.txt"), "").unwrap();
        let m = MenuState::scan(&dir, vec![PathBuf::from("game")]);
        let names: Vec<_> = m
            .scenarios
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.json5", "b.json5"]);
        assert_eq!(m.mods, [PathBuf::from("game")]);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(MenuState::scan(&dir, Vec::new()).scenarios.is_empty());
    }
}
