//! The result screen (T2-091, REQ-UI-004; plan decision 15): the winner,
//! the duration, per side the general's fate and the loot, a table per side
//! with a row per regiment (unit, initial, survivors, killed, fled,
//! experience gained, ammo left), the replay file, Back and Rematch. The
//! app fills the rows from `BattleResult` and the setup's rosters so the
//! panel needs no registry.

use il_data::Locale;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultRow {
    /// Localised unit name.
    pub unit: String,
    pub initial: u16,
    pub survivors: u16,
    pub killed: u16,
    pub fled: u16,
    pub experience: u16,
    pub ammo: u16,
    /// A reinforcement group that never entered the field.
    pub arrived: bool,
    /// The unit groups of a mixed regiment (SIM-FORM-015, T3-041); empty
    /// for a single-unit regiment.
    pub parts: Vec<ResultPart>,
}

/// One unit group's counts under a mixed regiment's row (T3-041).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultPart {
    /// Localised unit name.
    pub unit: String,
    pub initial: u16,
    pub survivors: u16,
    pub killed: u16,
    pub fled: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultSide {
    /// Localised faction name.
    pub name: String,
    pub tint: [u8; 4],
    /// Localised general's fate.
    pub fate: String,
    pub loot: i64,
    pub rows: Vec<ResultRow>,
}

pub struct ResultScreenModel<'a> {
    pub winner: Option<u8>,
    /// `mm:ss`.
    pub duration: String,
    pub sides: &'a [ResultSide],
    pub replay_path: Option<&'a str>,
    /// A playback has no rematch.
    pub can_rematch: bool,
    pub locale: &'a Locale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultAction {
    Menu,
    /// The same setup with the next seed (decision 12).
    Rematch,
}

/// Draws the screen; returns the click, if any.
pub fn result_screen(ctx: &egui::Context, model: &ResultScreenModel<'_>) -> Option<ResultAction> {
    let l = model.locale;
    let mut action = None;
    egui::Window::new(l.get("il.result.title"))
        .id(egui::Id::new("il_result"))
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .collapsible(false)
        .resizable(true)
        .default_width(640.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .max_height(600.0)
                .show(ui, |ui| {
                    match model.winner {
                        Some(side) => {
                            let name = model
                                .sides
                                .get(usize::from(side))
                                .map_or_else(|| side.to_string(), |s| s.name.clone());
                            ui.heading(l.fmt("il.result.winner", &[("side", &name)]))
                        }
                        None => ui.heading(l.get("il.result.draw")),
                    };
                    ui.label(l.fmt("il.result.duration", &[("time", &model.duration)]));
                    for (i, side) in model.sides.iter().enumerate() {
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.colored_label(
                                egui::Color32::from_rgba_unmultiplied(
                                    side.tint[0],
                                    side.tint[1],
                                    side.tint[2],
                                    side.tint[3],
                                ),
                                "\u{25a0}",
                            );
                            ui.strong(&side.name);
                            ui.label(l.fmt(
                                "il.result.side_line",
                                &[("fate", &side.fate), ("loot", &side.loot)],
                            ));
                        });
                        egui::Grid::new(("il_result_side", i))
                            .striped(true)
                            .num_columns(7)
                            .show(ui, |ui| {
                                for key in [
                                    "il.result.col.unit",
                                    "il.result.col.initial",
                                    "il.result.col.survivors",
                                    "il.result.col.killed",
                                    "il.result.col.fled",
                                    "il.result.col.experience",
                                    "il.result.col.ammo",
                                ] {
                                    ui.strong(l.get(key));
                                }
                                ui.end_row();
                                for r in &side.rows {
                                    if r.arrived {
                                        ui.label(&r.unit);
                                    } else {
                                        ui.weak(
                                            l.fmt("il.result.never_arrived", &[("unit", &r.unit)]),
                                        );
                                    }
                                    ui.label(r.initial.to_string());
                                    ui.label(r.survivors.to_string());
                                    ui.label(r.killed.to_string());
                                    ui.label(r.fled.to_string());
                                    ui.label(r.experience.to_string());
                                    ui.label(r.ammo.to_string());
                                    ui.end_row();
                                    // T3-041: a mixed regiment's groups, one
                                    // sub-row each.
                                    for p in &r.parts {
                                        ui.weak(l.fmt("il.result.part", &[("unit", &p.unit)]));
                                        ui.weak(p.initial.to_string());
                                        ui.weak(p.survivors.to_string());
                                        ui.weak(p.killed.to_string());
                                        ui.weak(p.fled.to_string());
                                        ui.label("");
                                        ui.label("");
                                        ui.end_row();
                                    }
                                }
                            });
                    }
                    ui.add_space(8.0);
                    if let Some(path) = model.replay_path {
                        ui.label(l.fmt("il.result.replay", &[("path", &path)]));
                    }
                    ui.horizontal(|ui| {
                        if ui.button(l.get("il.result.back")).clicked() {
                            action = Some(ResultAction::Menu);
                        }
                        if model.can_rematch && ui.button(l.get("il.result.rematch")).clicked() {
                            action = Some(ResultAction::Rematch);
                        }
                    });
                });
        });
    action
}
