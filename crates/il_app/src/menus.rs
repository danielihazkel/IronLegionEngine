//! The main menu's screens and the settings screen (T2-091, REQ-UI-004,
//! REQ-UI-007, REQ-INP-005): the catalog the custom battle builder offers,
//! the conversions between the settings file and the screen's draft, the
//! chord capture, the load screen's entries, and what each click does.
//! `app.rs` owns the frame; this module owns the menus.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use il_data::{Registries, UnitCategory};
use il_sim_battle::{BattleSetup, Scenario};
use il_ui::{
    BindingRow, BuilderAction, BuilderCatalog, BuilderState, Chord, FactionChoice, Gesture,
    InputState, LoadAction, MapChoice, MenuChoice, MenuModel, SaveEntry, SettingsAction,
    SettingsDraft, SettingsState, Trigger, UnitChoice, custom_battle, load_screen, main_menu,
    scenario_list, settings_screen,
};
use winit::window::Fullscreen;

use crate::app::App;
use crate::settings::{self, BindingOverride, Settings};
use crate::state::{AppState, MenuScreen, Transition};

/// What the builder offers, from the registries.
pub fn builder_catalog(regs: &Registries) -> BuilderCatalog {
    let l = &regs.locale;
    let unit = |h| {
        let u = regs.units.get(h);
        UnitChoice {
            id: u.id.clone(),
            name: l.get(&u.name_key).to_string(),
            category: u.category,
        }
    };
    let all_generals: Vec<UnitChoice> = regs
        .units
        .iter()
        .filter(|(_, u)| u.category == UnitCategory::General)
        .map(|(h, _)| unit(h))
        .collect();
    let factions = regs
        .factions
        .iter()
        .map(|(_, f)| {
            let (generals, units): (Vec<UnitChoice>, Vec<UnitChoice>) = f
                .units
                .iter()
                .map(|h| unit(*h))
                .partition(|u| u.category == UnitCategory::General);
            FactionChoice {
                id: f.id.clone(),
                name: l.get(&f.name_key).to_string(),
                units,
                generals: if generals.is_empty() {
                    all_generals.clone()
                } else {
                    generals
                },
            }
        })
        .collect();
    let maps = regs
        .maps
        .iter()
        .map(|(_, m)| MapChoice {
            id: m.id.clone(),
            name: l.get(&m.name_key).to_string(),
            zones: m.deployment.len(),
            weather: m.weather_allowed.iter().map(|w| w.to_lowercase()).collect(),
        })
        .collect();
    BuilderCatalog { maps, factions }
}

/// A seed from the clock (the builder's Random button).
pub fn random_seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
        | 1
}

/// The settings screen's draft for the current settings and content.
pub fn draft_from(settings: &Settings, regs: &Registries) -> SettingsDraft {
    let bindings = regs
        .input
        .bindings
        .iter()
        .map(|b| BindingRow {
            action: b.action.clone(),
            keys: settings
                .bindings
                .iter()
                .find(|o| o.action == b.action)
                .map_or_else(|| b.keys.clone(), |o| o.keys.clone()),
            default_keys: b.keys.clone(),
        })
        .collect();
    SettingsDraft {
        ui_scale: settings.ui_scale,
        vsync: settings.vsync,
        fullscreen: settings.fullscreen,
        threads: settings.threads,
        master: settings.volume.master,
        effects: settings.volume.effects,
        music: settings.volume.music,
        bindings,
    }
}

/// The settings a draft means: overrides only for rows that differ from the
/// mods' defaults.
pub fn settings_from(draft: &SettingsDraft, base: &Settings) -> Settings {
    Settings {
        ui_scale: draft
            .ui_scale
            .clamp(settings::UI_SCALE_RANGE.0, settings::UI_SCALE_RANGE.1),
        vsync: draft.vsync,
        fullscreen: draft.fullscreen,
        threads: draft.threads.max(1),
        volume: settings::Volume {
            master: draft.master,
            effects: draft.effects,
            music: draft.music,
        },
        bindings: draft
            .bindings
            .iter()
            .filter(|r| r.keys != r.default_keys)
            .map(|r| BindingOverride {
                action: r.action.clone(),
                keys: r.keys.clone(),
            })
            .collect(),
        replays_dir: base.replays_dir.clone(),
        saves_dir: base.saves_dir.clone(),
    }
}

/// The chord the frame's input means, if any: the first key press, else the
/// first click (plan I10). Wheel and drag chords are not capturable.
pub fn captured_chord(input: &InputState) -> Option<String> {
    if let Some((code, mods)) = input.key_presses().first() {
        return Some(
            Chord {
                mods: *mods,
                trigger: Trigger::Key(*code),
            }
            .to_text(),
        );
    }
    input.gestures().iter().find_map(|g| match g {
        Gesture::Click {
            button,
            mods,
            double,
            ..
        } => Some(
            Chord {
                mods: *mods,
                trigger: if *double {
                    Trigger::DoubleClick(*button)
                } else {
                    Trigger::Click(*button)
                },
            }
            .to_text(),
        ),
        _ => None,
    })
}

/// The load screen's entries: every `.ilsv` under `dir`, newest name last.
pub fn save_entries(dir: &Path, regs: &Registries) -> Vec<SaveEntry> {
    let l = &regs.locale;
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == il_save::SAVE_EXTENSION))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            match il_save::read_header(&path) {
                Ok(h) => {
                    let problem = if h.kind != il_save::SaveKind::Battle
                        || h.schema_version != il_sim_battle::SNAPSHOT_VERSION
                        || !il_save::content_matches(&h, regs)
                    {
                        Some(l.get("il.load.wrong_content").to_string())
                    } else {
                        None
                    };
                    SaveEntry {
                        path,
                        name,
                        created: h.created,
                        tick: h.tick,
                        summary: h.summary,
                        problem,
                    }
                }
                Err(_) => SaveEntry {
                    path,
                    name,
                    created: String::new(),
                    tick: None,
                    summary: String::new(),
                    problem: Some(l.get("il.load.unreadable").to_string()),
                },
            }
        })
        .collect()
}

/// Writes a builder's setup as a scenario file (SDK §4.13; pretty JSON is
/// valid JSON5).
pub fn write_scenario(path: &Path, setup: &BattleSetup) -> anyhow::Result<()> {
    let scenario = Scenario {
        setup: setup.clone(),
        commands: Vec::new(),
        determinism: None,
    };
    let text = serde_json::to_string_pretty(&scenario)?;
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

impl App {
    /// Applies a settings draft at once (decision 13): the zoom factor, vsync,
    /// fullscreen, the bindings, the thread count for the next battle.
    pub(crate) fn apply_settings(&mut self, draft: &SettingsDraft) {
        let new = settings_from(draft, &self.launch.settings);
        self.launch.settings = new;
        self.ui_scale_user = self.launch.settings.ui_scale;
        self.launch.threads = self.launch.settings.threads;
        let (bindings, errors) = settings::effective_bindings(&self.regs, &self.launch.settings);
        for e in errors {
            eprintln!("bindings: {e}");
        }
        self.bindings = bindings;
        self.audio.set_volumes(&self.launch.settings.volume);
        self.apply_window_settings();
    }

    /// Vsync and fullscreen from the settings, once the window exists.
    pub(crate) fn apply_window_settings(&mut self) {
        if let Some(r) = self.renderer.as_mut() {
            r.set_vsync(self.launch.settings.vsync && !self.launch.bench_sprites);
        }
        if let Some(w) = self.window.as_ref() {
            let want = self.launch.settings.fullscreen;
            if w.fullscreen().is_some() != want {
                w.set_fullscreen(want.then_some(Fullscreen::Borderless(None)));
            }
        }
    }

    /// Feeds a captured chord to whichever settings screen is capturing.
    pub(crate) fn capture_chord(&mut self) {
        let Some(chord) = captured_chord(&self.input) else {
            return;
        };
        if let Some(s) = self.battle_ui.settings.as_mut()
            && s.capturing.is_some()
        {
            s.captured(chord);
            return;
        }
        if let AppState::MainMenu(menu) = &mut self.state
            && let MenuScreen::Settings(s) = &mut menu.screen
            && s.capturing.is_some()
        {
            s.captured(chord);
        }
    }

    /// Whether a settings screen is waiting for a chord (game input pauses).
    pub(crate) fn capturing_chord(&self) -> bool {
        if self
            .battle_ui
            .settings
            .as_ref()
            .is_some_and(|s| s.capturing.is_some())
        {
            return true;
        }
        matches!(&self.state, AppState::MainMenu(m) if matches!(&m.screen, MenuScreen::Settings(s) if s.capturing.is_some()))
    }

    /// A settings screen's click; returns true when the screen closes.
    pub(crate) fn settings_action(
        &mut self,
        state: &mut SettingsState,
        action: SettingsAction,
    ) -> bool {
        let regs = self.regs.clone();
        let l = &regs.locale;
        match action {
            SettingsAction::Apply => {
                let draft = state.draft.clone();
                self.apply_settings(&draft);
                state.dirty = false;
                state.note = None;
                false
            }
            SettingsAction::Save => {
                let draft = state.draft.clone();
                self.apply_settings(&draft);
                state.dirty = false;
                let path = self.launch.settings_path.clone();
                state.note = Some(match settings::save(&path, &self.launch.settings) {
                    Ok(()) => l.fmt("il.settings.saved", &[("path", &path.display())]),
                    Err(e) => l.fmt("il.settings.save_failed", &[("error", &format!("{e:#}"))]),
                });
                false
            }
            SettingsAction::Back => true,
        }
    }

    /// Draws the main menu's current screen and applies its click. Returns
    /// true when the player asked to exit.
    pub(crate) fn menu_frame(
        &mut self,
        ui: &mut il_ui::UiContext,
        window: &winit::window::Window,
    ) -> (il_ui::UiOutput, bool) {
        let regs = self.regs.clone();
        let l = &regs.locale;
        let scenarios_dir = self.launch.scenarios_dir.clone();
        let saves_dir = self.launch.saves_dir.clone();
        let mut exit = false;
        let mut transition = None;
        let mut next_screen: Option<MenuScreen> = None;
        let mut settings_click: Option<SettingsAction> = None;
        let mut builder_click: Option<BuilderAction> = None;
        let AppState::MainMenu(menu) = &mut self.state else {
            unreachable!("menu_frame runs in the menu state");
        };
        let scenarios: Vec<String> = menu.scenarios.iter().map(|p| file_name(p)).collect();
        let mods: Vec<String> = menu.mods.iter().map(|p| file_name(p)).collect();
        let error = menu.error.clone();
        let model = MenuModel {
            scenarios: &scenarios,
            mods: &mods,
            error: error.as_deref(),
            locale: l,
        };
        let catalog = builder_catalog(&regs);
        let out = ui.run(window, |ctx| match &mut menu.screen {
            MenuScreen::Root => match main_menu(ctx, &model) {
                Some(MenuChoice::CustomBattle) => {
                    next_screen = Some(MenuScreen::CustomBattle(Box::new(
                        BuilderState::default_for(&catalog, random_seed()),
                    )));
                }
                Some(MenuChoice::Scenarios) => next_screen = Some(MenuScreen::Scenarios),
                Some(MenuChoice::Load) => {
                    next_screen = Some(MenuScreen::Load(save_entries(&saves_dir, &regs)));
                }
                Some(MenuChoice::Settings) => {
                    next_screen = Some(MenuScreen::Settings(Box::new(SettingsState::new(
                        draft_from(&self.launch.settings, &regs),
                    ))));
                }
                Some(MenuChoice::Exit) => exit = true,
                Some(MenuChoice::Start(_) | MenuChoice::Back) | None => {}
            },
            MenuScreen::Scenarios => match scenario_list(ctx, &model) {
                Some(MenuChoice::Start(i)) => {
                    transition = Some(Transition::StartBattle(menu.scenarios[i].clone()));
                }
                Some(MenuChoice::Back) => next_screen = Some(MenuScreen::Root),
                _ => {}
            },
            MenuScreen::CustomBattle(state) => {
                builder_click = custom_battle(ctx, state, &catalog, l);
            }
            MenuScreen::Settings(state) => {
                settings_click = settings_screen(ctx, state, l, false);
            }
            MenuScreen::Load(entries) => match load_screen(ctx, entries, l) {
                Some(LoadAction::Load(path)) => transition = Some(Transition::LoadSave(path)),
                Some(LoadAction::Back) => next_screen = Some(MenuScreen::Root),
                None => {}
            },
        });
        // The clicks that need `self` beyond the menu.
        if let Some(action) = settings_click {
            let AppState::MainMenu(menu) = &mut self.state else {
                unreachable!();
            };
            let MenuScreen::Settings(state) = &mut menu.screen else {
                unreachable!();
            };
            let mut state = std::mem::replace(
                state,
                Box::new(SettingsState::new(draft_from(&Settings::default(), &regs))),
            );
            let close = self.settings_action(&mut state, action);
            if close {
                next_screen = Some(MenuScreen::Root);
            } else if let AppState::MainMenu(menu) = &mut self.state {
                menu.screen = MenuScreen::Settings(state);
            }
        }
        if let Some(action) = builder_click {
            let AppState::MainMenu(menu) = &mut self.state else {
                unreachable!();
            };
            let MenuScreen::CustomBattle(state) = &mut menu.screen else {
                unreachable!();
            };
            match action {
                BuilderAction::Back => next_screen = Some(MenuScreen::Root),
                BuilderAction::RandomSeed => {
                    state.seed = random_seed();
                    state.name = format!("custom_{}", state.seed);
                    state.confirm_overwrite = false;
                }
                BuilderAction::Start => match state.to_setup(&catalog) {
                    Ok(setup) => {
                        transition = Some(Transition::StartSetup {
                            setup: Box::new(setup),
                            stem: state.name.clone(),
                            ai: Vec::new(),
                        });
                    }
                    Err(e) => state.error = Some(e.text(l)),
                },
                BuilderAction::Save => match state.to_setup(&catalog) {
                    Ok(setup) => {
                        let stem: String = state
                            .name
                            .chars()
                            .map(|c| {
                                if c.is_alphanumeric() || c == '_' || c == '-' {
                                    c
                                } else {
                                    '_'
                                }
                            })
                            .collect();
                        let path = scenarios_dir.join(format!("{stem}.json5"));
                        if path.exists() && !state.confirm_overwrite {
                            state.confirm_overwrite = true;
                            state.error = Some(l.get("il.custom.overwrite").to_string());
                        } else {
                            state.confirm_overwrite = false;
                            state.error = Some(match write_scenario(&path, &setup) {
                                Ok(()) => {
                                    let mods = menu.mods.clone();
                                    let fresh = crate::state::MenuState::scan(&scenarios_dir, mods);
                                    menu.scenarios = fresh.scenarios;
                                    l.fmt("il.custom.saved", &[("path", &path.display())])
                                }
                                Err(e) => {
                                    l.fmt("il.custom.save_failed", &[("error", &format!("{e:#}"))])
                                }
                            });
                        }
                    }
                    Err(e) => state.error = Some(e.text(l)),
                },
            }
        }
        if let Some(screen) = next_screen
            && let AppState::MainMenu(menu) = &mut self.state
        {
            menu.screen = screen;
            menu.error = None;
        }
        if transition.is_some() {
            self.transition = transition;
        }
        (out, exit)
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use il_ui::Mods;
    use winit::keyboard::KeyCode;

    fn regs() -> Registries {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game");
        il_data::load_roots(&[root]).unwrap_or_else(|d| panic!("{d}"))
    }

    /// T2-091 (decision 13): the draft carries the mods' bindings with the
    /// overrides applied; only rows that differ come back as overrides.
    #[test]
    fn draft_and_settings_round_trip_through_the_registry_bindings() {
        let regs = regs();
        let mut base = Settings::default();
        base.bindings.push(BindingOverride {
            action: "order_halt".into(),
            keys: vec!["J".into()],
        });
        let draft = draft_from(&base, &regs);
        let halt = draft
            .bindings
            .iter()
            .find(|r| r.action == "order_halt")
            .unwrap();
        assert_eq!(halt.keys, ["J"]);
        assert_eq!(halt.default_keys, ["H"]);
        assert_eq!(draft.bindings.len(), regs.input.bindings.len());
        let back = settings_from(&draft, &base);
        assert_eq!(back.bindings, base.bindings);
        let mut reset = draft.clone();
        reset
            .bindings
            .iter_mut()
            .for_each(|r| r.keys = r.default_keys.clone());
        assert!(settings_from(&reset, &base).bindings.is_empty());
        let catalog = builder_catalog(&regs);
        assert!(catalog.factions.iter().all(|f| !f.generals.is_empty()));
        assert!(catalog.factions.iter().all(|f| !f.units.is_empty()));
        assert_eq!(catalog.maps[0].zones, 2);
    }

    /// T2-091 (plan I10): the frame's first key press, else its first
    /// click, becomes the captured chord's text.
    #[test]
    fn captured_chords_come_from_keys_then_clicks() {
        let mut input = InputState::new();
        input.begin_frame(0.0);
        assert!(captured_chord(&input).is_none());
        input.set_modifiers(Mods::CTRL);
        input.key(KeyCode::KeyK, true, false);
        assert_eq!(captured_chord(&input).as_deref(), Some("Ctrl+K"));
        input.end_frame();
        input.begin_frame(0.1);
        input.set_modifiers(Mods::NONE);
        input.button(il_ui::Button::Right, true);
        input.button(il_ui::Button::Right, false);
        assert_eq!(captured_chord(&input).as_deref(), Some("RightClick"));
    }
}
