//! The settings screen (T2-091, REQ-UI-007, REQ-INP-005; plan decision 13,
//! I10): Video (UI scale, vsync, fullscreen, sim threads), Audio (the
//! volumes T2-100 will read) and Bindings (every action with its chords;
//! click a chord to capture the next key or mouse chord, Reset restores the
//! mods' default, a chord bound twice is flagged). Apply takes effect at
//! once, Save writes the file. The app owns the capture (it has the input
//! state) and the file (it has the settings); this panel edits a draft.

use il_data::Locale;

/// The remove-chord button (a minus sign, not a word).
const MINUS: &str = "\u{2212}";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Video,
    Audio,
    Bindings,
}

/// One action's chords, editable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingRow {
    pub action: String,
    pub keys: Vec<String>,
    /// The mods' chords, for Reset.
    pub default_keys: Vec<String>,
}

/// The editable copy of the settings.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsDraft {
    pub ui_scale: f32,
    pub vsync: bool,
    pub fullscreen: bool,
    pub threads: usize,
    pub master: f32,
    pub effects: f32,
    pub music: f32,
    pub bindings: Vec<BindingRow>,
}

/// A chord slot being captured: the row and the slot (`keys.len()` = a new
/// chord).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capture {
    pub row: usize,
    pub slot: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SettingsState {
    pub draft: SettingsDraft,
    pub tab: Tab,
    pub capturing: Option<Capture>,
    /// Edited since the last Apply.
    pub dirty: bool,
    /// The last Save's outcome (localised).
    pub note: Option<String>,
}

impl SettingsState {
    pub fn new(draft: SettingsDraft) -> Self {
        Self {
            draft,
            tab: Tab::Video,
            capturing: None,
            dirty: false,
            note: None,
        }
    }

    /// Stores a captured chord into the slot being captured.
    pub fn captured(&mut self, chord: String) {
        if let Some(c) = self.capturing.take()
            && let Some(row) = self.draft.bindings.get_mut(c.row)
        {
            if c.slot < row.keys.len() {
                row.keys[c.slot] = chord;
            } else {
                row.keys.push(chord);
            }
            self.dirty = true;
        }
    }

    /// Actions bound to the same chord as `row`'s slot `k`, other than the row itself.
    pub fn conflicts(&self, row: usize, k: usize) -> Vec<String> {
        let Some(chord) = self.draft.bindings.get(row).and_then(|r| r.keys.get(k)) else {
            return Vec::new();
        };
        self.draft
            .bindings
            .iter()
            .enumerate()
            .filter(|(i, r)| *i != row && r.keys.iter().any(|c| c == chord))
            .map(|(_, r)| r.action.clone())
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsAction {
    Apply,
    Save,
    Back,
}

/// Draws the screen (a window over a battle, a panel from the menu).
pub fn settings_screen(
    ctx: &egui::Context,
    state: &mut SettingsState,
    locale: &Locale,
    over_battle: bool,
) -> Option<SettingsAction> {
    let mut action = None;
    let body = |ui: &mut egui::Ui| {
        ui.heading(locale.get("il.settings.title"));
        ui.horizontal(|ui| {
            for (tab, key) in [
                (Tab::Video, "il.settings.video"),
                (Tab::Audio, "il.settings.audio"),
                (Tab::Bindings, "il.settings.bindings"),
            ] {
                if ui
                    .selectable_label(state.tab == tab, locale.get(key))
                    .clicked()
                {
                    state.tab = tab;
                }
            }
        });
        ui.separator();
        match state.tab {
            Tab::Video => video_tab(ui, state, locale),
            Tab::Audio => audio_tab(ui, state, locale),
            Tab::Bindings => bindings_tab(ui, state, locale),
        }
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button(locale.get("il.settings.apply")).clicked() {
                action = Some(SettingsAction::Apply);
            }
            if ui.button(locale.get("il.settings.save")).clicked() {
                action = Some(SettingsAction::Save);
            }
            if ui.button(locale.get("il.menu.back")).clicked() {
                action = Some(SettingsAction::Back);
            }
            if state.dirty {
                ui.weak(locale.get("il.settings.unapplied"));
            }
        });
        if let Some(n) = &state.note {
            ui.label(n);
        }
    };
    if over_battle {
        egui::Window::new(locale.get("il.settings.title"))
            .id(egui::Id::new("il_settings"))
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(560.0)
                    .show(ui, body);
            });
    } else {
        egui::Window::new("il_settings_menu")
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 24.0))
            .title_bar(false)
            .resizable(true)
            .default_width(640.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(700.0)
                    .show(ui, body);
            });
    }
    action
}

fn video_tab(ui: &mut egui::Ui, state: &mut SettingsState, l: &Locale) {
    let d = &mut state.draft;
    egui::Grid::new("il_settings_video")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label(l.get("il.settings.ui_scale"));
            if ui
                .add(egui::Slider::new(&mut d.ui_scale, 0.5..=2.0).step_by(0.05))
                .changed()
            {
                state.dirty = true;
            }
            ui.end_row();
            ui.label(l.get("il.settings.vsync"));
            if ui.checkbox(&mut d.vsync, "").changed() {
                state.dirty = true;
            }
            ui.end_row();
            ui.label(l.get("il.settings.fullscreen"));
            if ui.checkbox(&mut d.fullscreen, "").changed() {
                state.dirty = true;
            }
            ui.end_row();
            ui.label(l.get("il.settings.threads"));
            if ui.add(egui::Slider::new(&mut d.threads, 1..=32)).changed() {
                state.dirty = true;
            }
            ui.end_row();
        });
    ui.weak(l.get("il.settings.threads_note"));
}

fn audio_tab(ui: &mut egui::Ui, state: &mut SettingsState, l: &Locale) {
    let d = &mut state.draft;
    egui::Grid::new("il_settings_audio")
        .num_columns(2)
        .show(ui, |ui| {
            for (key, v) in [
                ("il.settings.master", &mut d.master),
                ("il.settings.effects", &mut d.effects),
                ("il.settings.music", &mut d.music),
            ] {
                ui.label(l.get(key));
                if ui.add(egui::Slider::new(v, 0.0..=1.0)).changed() {
                    state.dirty = true;
                }
                ui.end_row();
            }
        });
    ui.weak(l.get("il.settings.audio_note"));
}

fn bindings_tab(ui: &mut egui::Ui, state: &mut SettingsState, l: &Locale) {
    ui.weak(l.get("il.settings.bindings_hint"));
    let capturing = state.capturing;
    let mut next_capture = None;
    let mut cancel = false;
    let mut edits: Vec<(usize, Edit)> = Vec::new();
    egui::Grid::new("il_settings_bindings")
        .num_columns(3)
        .striped(true)
        .show(ui, |ui| {
            ui.strong(l.get("il.settings.action"));
            ui.strong(l.get("il.settings.chords"));
            ui.label("");
            ui.end_row();
            for (i, row) in state.draft.bindings.iter().enumerate() {
                ui.monospace(&row.action);
                ui.horizontal_wrapped(|ui| {
                    for (k, chord) in row.keys.iter().enumerate() {
                        let here = capturing == Some(Capture { row: i, slot: k });
                        let conflicts = state.conflicts(i, k);
                        let text = if here {
                            l.get("il.settings.press").to_string()
                        } else if conflicts.is_empty() {
                            chord.clone()
                        } else {
                            l.fmt(
                                "il.settings.conflict",
                                &[("chord", chord), ("actions", &conflicts.join(", "))],
                            )
                        };
                        let button = if conflicts.is_empty() && !here {
                            egui::Button::new(text)
                        } else {
                            egui::Button::new(text).fill(egui::Color32::from_rgb(120, 60, 30))
                        };
                        if ui.add(button).clicked() {
                            next_capture = Some(Capture { row: i, slot: k });
                        }
                        if ui.small_button(MINUS).clicked() {
                            edits.push((i, Edit::Remove(k)));
                        }
                    }
                    let adding = capturing
                        == Some(Capture {
                            row: i,
                            slot: row.keys.len(),
                        });
                    let plus = if adding {
                        l.get("il.settings.press").to_string()
                    } else {
                        "+".to_string()
                    };
                    if ui.small_button(plus).clicked() {
                        next_capture = Some(Capture {
                            row: i,
                            slot: row.keys.len(),
                        });
                    }
                });
                if row.keys != row.default_keys
                    && ui.small_button(l.get("il.settings.reset")).clicked()
                {
                    edits.push((i, Edit::Reset));
                }
                ui.end_row();
            }
        });
    if capturing.is_some() && ui.button(l.get("il.settings.cancel_capture")).clicked() {
        cancel = true;
    }
    for (i, e) in edits {
        if let Some(row) = state.draft.bindings.get_mut(i) {
            match e {
                Edit::Remove(k) => {
                    if k < row.keys.len() {
                        row.keys.remove(k);
                    }
                }
                Edit::Reset => row.keys = row.default_keys.clone(),
            }
            state.dirty = true;
        }
    }
    if let Some(c) = next_capture {
        state.capturing = Some(c);
    }
    if cancel {
        state.capturing = None;
    }
}

enum Edit {
    Remove(usize),
    Reset,
}
