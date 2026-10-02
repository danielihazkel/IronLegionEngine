//! The app state machine (T1-070, SAD §6.1, TDD §15): `MainMenu`, `Battle`
//! and, from T3-060, `Editor` (`Campaign` arrives with its phase).
//! Transitions are pure so they can be tested without a window: the caller
//! supplies the functions that turn a scenario path, a setup, a save or a
//! map id into a session.

use std::path::{Path, PathBuf};

use il_core::PlayerId;
use il_data::ContentId;
use il_editor::{BlankMap, EditorSession, PickerState};
use il_sim_battle::BattleSetup;
use il_ui::{BuilderState, SaveEntry, SettingsState};

use crate::sim_thread::BattleHandle;

pub enum AppState {
    MainMenu(MenuState),
    /// A battle: the handle on its session, stepped on the sim thread or
    /// inline (T3-032).
    Battle(Box<BattleHandle>),
    /// The map editor (T3-060, TDD §16).
    Editor(Box<EditorSession>),
}

/// Which menu screen is up (T2-091, REQ-UI-007).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum MenuScreen {
    #[default]
    Root,
    Scenarios,
    CustomBattle(Box<BuilderState>),
    Settings(Box<SettingsState>),
    Load(Vec<SaveEntry>),
    /// The map editor's picker: open a registry map or a blank one (T3-060).
    Editor(Box<PickerState>),
}

/// What the main menu shows: the scenario files it found, the mod roots in
/// load order, the last failure to start a battle, and the screen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuState {
    pub scenarios: Vec<PathBuf>,
    pub mods: Vec<PathBuf>,
    pub error: Option<String>,
    pub screen: MenuScreen,
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
            screen: MenuScreen::Root,
        }
    }
}

/// What the UI asked for this frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Transition {
    /// Main menu: start the custom battle in this scenario file.
    StartBattle(PathBuf),
    /// Start a battle from a setup built in memory: the custom battle
    /// builder or a rematch (T2-091); from a battle the session is replaced.
    StartSetup {
        setup: Box<BattleSetup>,
        stem: String,
        ai: Vec<PlayerId>,
    },
    /// Continue the battle save at this path (quick load or the load
    /// screen, T2-101); from a battle the session is replaced.
    LoadSave(PathBuf),
    /// Main menu: open the map editor on a registry map, or on the blank
    /// map the picker's form described (T3-060).
    OpenEditor {
        map: Option<ContentId>,
        blank: Option<BlankMap>,
    },
    /// Battle or editor: back to the main menu (the session is dropped).
    QuitToMenu,
}

impl AppState {
    pub fn is_battle(&self) -> bool {
        matches!(self, AppState::Battle(_))
    }

    pub fn is_editor(&self) -> bool {
        matches!(self, AppState::Editor(_))
    }

    pub fn session(&self) -> Option<&BattleHandle> {
        match self {
            AppState::Battle(s) => Some(s),
            _ => None,
        }
    }

    pub fn session_mut(&mut self) -> Option<&mut BattleHandle> {
        match self {
            AppState::Battle(s) => Some(s),
            _ => None,
        }
    }

    pub fn editor(&self) -> Option<&EditorSession> {
        match self {
            AppState::Editor(s) => Some(s),
            _ => None,
        }
    }

    pub fn editor_mut(&mut self) -> Option<&mut EditorSession> {
        match self {
            AppState::Editor(s) => Some(s),
            _ => None,
        }
    }

    /// Applies a transition. `start` builds the session for a scenario,
    /// `build` one for a setup in memory, `load` one for a battle save and
    /// `open_editor` the editor session for a map or a blank; on failure the
    /// menu comes up with the error. `menu` rebuilds the menu when a battle
    /// or the editor quits.
    pub fn apply(
        self,
        transition: Transition,
        start: impl FnOnce(&Path) -> anyhow::Result<BattleHandle>,
        build: impl FnOnce(BattleSetup, String, Vec<PlayerId>) -> anyhow::Result<BattleHandle>,
        load: impl FnOnce(&Path) -> anyhow::Result<BattleHandle>,
        open_editor: impl FnOnce(Option<ContentId>, Option<BlankMap>) -> anyhow::Result<EditorSession>,
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
            (
                state @ (AppState::MainMenu(_) | AppState::Battle(_)),
                Transition::StartSetup { setup, stem, ai },
            ) => match build(*setup, stem.clone(), ai) {
                Ok(session) => AppState::Battle(Box::new(session)),
                Err(e) => {
                    let mut m = match state {
                        AppState::MainMenu(m) => m,
                        _ => menu(),
                    };
                    m.error = Some(format!("{stem}: {e:#}"));
                    AppState::MainMenu(m)
                }
            },
            (state @ (AppState::MainMenu(_) | AppState::Battle(_)), Transition::LoadSave(path)) => {
                match load(&path) {
                    Ok(session) => AppState::Battle(Box::new(session)),
                    Err(e) => {
                        let mut m = match state {
                            AppState::MainMenu(m) => m,
                            _ => menu(),
                        };
                        m.error = Some(format!("{}: {e:#}", path.display()));
                        AppState::MainMenu(m)
                    }
                }
            }
            (AppState::MainMenu(mut m), Transition::OpenEditor { map, blank }) => {
                let what = map
                    .as_ref()
                    .map_or_else(|| "new map".to_string(), |id| id.as_str().to_string());
                match open_editor(map, blank) {
                    Ok(session) => AppState::Editor(Box::new(session)),
                    Err(e) => {
                        m.error = Some(format!("editor {what}: {e:#}"));
                        AppState::MainMenu(m)
                    }
                }
            }
            (AppState::Battle(_) | AppState::Editor(_), Transition::QuitToMenu) => {
                AppState::MainMenu(menu())
            }
            // Starting from a battle, quitting from the menu, or anything
            // but Quit from the editor is a no-op.
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

    fn session(_: &Path) -> anyhow::Result<BattleHandle> {
        let world = BattleWorld::empty(1, Arc::new(Registries::default()), BattlePhase::Battle);
        Ok(BattleHandle::new(
            crate::session::BattleSession::new(
                world,
                PlayerId(0),
                ScriptedCommands::default(),
                Vec::new(),
            ),
            false,
        ))
    }

    fn no_build(_: BattleSetup, stem: String, _: Vec<PlayerId>) -> anyhow::Result<BattleHandle> {
        anyhow::bail!("no builder for {stem}")
    }

    fn no_editor(_: Option<ContentId>, _: Option<BlankMap>) -> anyhow::Result<EditorSession> {
        anyhow::bail!("no editor")
    }

    fn failing(p: &Path) -> anyhow::Result<BattleHandle> {
        anyhow::bail!("no such scenario {}", p.display())
    }

    fn menu() -> MenuState {
        MenuState {
            scenarios: vec![PathBuf::from("a.json5")],
            ..MenuState::default()
        }
    }

    fn game_regs() -> Arc<Registries> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game");
        Arc::new(il_data::load_roots(&[root]).unwrap_or_else(|d| panic!("{d}")))
    }

    #[test]
    fn menu_starts_a_battle_and_a_battle_quits_to_the_menu() {
        let state = AppState::MainMenu(menu());
        let state = state.apply(
            Transition::StartBattle(PathBuf::from("a.json5")),
            session,
            no_build,
            failing,
            no_editor,
            menu,
        );
        assert!(state.is_battle());
        assert!(state.session().is_some());
        let state = state.apply(
            Transition::QuitToMenu,
            session,
            no_build,
            session,
            no_editor,
            menu,
        );
        assert!(!state.is_battle());
        match state {
            AppState::MainMenu(m) => assert_eq!(m, menu()),
            _ => unreachable!(),
        }
    }

    #[test]
    fn a_failed_start_stays_in_the_menu_with_the_error() {
        let state = AppState::MainMenu(menu()).apply(
            Transition::StartBattle(PathBuf::from("missing.json5")),
            failing,
            no_build,
            failing,
            no_editor,
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
            _ => panic!("must not enter a battle"),
        }
    }

    #[test]
    fn mismatched_transitions_are_ignored() {
        let state = AppState::MainMenu(menu()).apply(
            Transition::QuitToMenu,
            session,
            no_build,
            session,
            no_editor,
            menu,
        );
        assert!(!state.is_battle());
        let battle = AppState::Battle(Box::new(session(Path::new("x")).unwrap()));
        let tick = battle.session().unwrap().view().tick();
        let battle = battle.apply(
            Transition::StartBattle(PathBuf::from("b.json5")),
            failing,
            no_build,
            failing,
            no_editor,
            menu,
        );
        assert!(battle.is_battle());
        assert_eq!(battle.session().unwrap().view().tick(), tick);
    }

    /// T2-101: a load replaces the battle or leaves the menu; a failed load
    /// lands in the menu with the error either way.
    #[test]
    fn a_load_replaces_the_session_and_a_failed_load_reports_in_the_menu() {
        let from_menu = AppState::MainMenu(menu()).apply(
            Transition::LoadSave(PathBuf::from("quick.ilsv")),
            failing,
            no_build,
            session,
            no_editor,
            menu,
        );
        assert!(from_menu.is_battle());
        let battle = AppState::Battle(Box::new(session(Path::new("x")).unwrap()));
        let replaced = battle.apply(
            Transition::LoadSave(PathBuf::from("quick.ilsv")),
            failing,
            no_build,
            session,
            no_editor,
            menu,
        );
        assert!(replaced.is_battle());
        let failed = replaced.apply(
            Transition::LoadSave(PathBuf::from("gone.ilsv")),
            failing,
            no_build,
            failing,
            no_editor,
            menu,
        );
        match failed {
            AppState::MainMenu(m) => assert!(m.error.unwrap().contains("gone.ilsv")),
            _ => panic!("a failed load must not keep a battle"),
        }
    }

    /// T3-060: the menu opens the editor on a registry map or a blank; a
    /// failure reports in the menu; the editor's Quit returns to the menu
    /// and a load or a setup start is ignored while it is up.
    #[test]
    fn the_menu_opens_the_editor_and_the_editor_quits_to_the_menu() {
        let regs = game_regs();
        let open = |map: Option<ContentId>, blank: Option<BlankMap>| {
            let doc = match (map, blank) {
                (Some(id), _) => il_editor::MapDocument::from_registry(&regs, &id, &[])?,
                (None, Some(b)) => il_editor::MapDocument::blank(&b, &regs)?,
                (None, None) => anyhow::bail!("nothing"),
            };
            Ok(EditorSession::open(doc, regs.clone(), Vec::new(), None)?)
        };
        let state = AppState::MainMenu(menu()).apply(
            Transition::OpenEditor {
                map: Some(ContentId::new("rome:test_field").unwrap()),
                blank: None,
            },
            failing,
            no_build,
            failing,
            open,
            menu,
        );
        assert!(state.is_editor());
        assert_eq!(
            state.editor().unwrap().doc.def.id.as_str(),
            "rome:test_field"
        );
        let still = state.apply(
            Transition::LoadSave(PathBuf::from("quick.ilsv")),
            failing,
            no_build,
            session,
            no_editor,
            menu,
        );
        assert!(still.is_editor(), "a load is ignored in the editor");
        let back = still.apply(
            Transition::QuitToMenu,
            failing,
            no_build,
            failing,
            no_editor,
            menu,
        );
        assert!(matches!(back, AppState::MainMenu(_)));

        let blank = BlankMap {
            id: ContentId::new("mymod:fresh").unwrap(),
            size: [200.0, 100.0],
            height_cell: 4.0,
            base_zone: ContentId::new("rome:open").unwrap(),
        };
        let fresh = AppState::MainMenu(menu()).apply(
            Transition::OpenEditor {
                map: None,
                blank: Some(blank),
            },
            failing,
            no_build,
            failing,
            open,
            menu,
        );
        assert!(fresh.editor().unwrap().doc.dirty);

        let failed = AppState::MainMenu(menu()).apply(
            Transition::OpenEditor {
                map: Some(ContentId::new("rome:nowhere").unwrap()),
                blank: None,
            },
            failing,
            no_build,
            failing,
            open,
            menu,
        );
        match failed {
            AppState::MainMenu(m) => {
                let e = m.error.unwrap();
                assert!(e.contains("rome:nowhere"), "{e}");
            }
            _ => panic!("an unknown map must not open the editor"),
        }
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
