//! The load screen (T2-091, REQ-UI-007, decision 14): the battle saves the
//! app found, each with its header's time, tick and summary; a save the
//! loaded content cannot read is listed greyed with the reason.

use std::path::PathBuf;

use il_data::Locale;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveEntry {
    pub path: PathBuf,
    /// The file name.
    pub name: String,
    /// The header's `created`.
    pub created: String,
    pub tick: Option<u32>,
    pub summary: String,
    /// `None` when the file can be loaded, else why not (localised).
    pub problem: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadAction {
    Load(PathBuf),
    Back,
}

/// Draws the screen; returns the click, if any.
pub fn load_screen(
    ctx: &egui::Context,
    entries: &[SaveEntry],
    locale: &Locale,
) -> Option<LoadAction> {
    let l = locale;
    let mut action = None;
    egui::Window::new("il_load")
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 24.0))
        .title_bar(false)
        .resizable(true)
        .default_width(640.0)
        .show(ctx, |ui| {
            ui.heading(l.get("il.load.title"));
            if entries.is_empty() {
                ui.label(l.get("il.load.none"));
            }
            egui::Grid::new("il_load").striped(true).show(ui, |ui| {
                for e in entries {
                    ui.monospace(&e.name);
                    ui.label(&e.created);
                    ui.label(match e.tick {
                        Some(t) => crate::panels::clock(il_core::Tick(t)),
                        None => String::new(),
                    });
                    ui.label(&e.summary);
                    match &e.problem {
                        None => {
                            if ui.button(l.get("il.load.load")).clicked() {
                                action = Some(LoadAction::Load(e.path.clone()));
                            }
                        }
                        Some(p) => {
                            ui.weak(p);
                        }
                    }
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            if ui.button(l.get("il.menu.back")).clicked() {
                action = Some(LoadAction::Back);
            }
        });
    action
}
