//! egui panels (T1-070, TDD §11 "Panels"): the Phase 1 main menu (custom
//! battle from a scenario file), the battle HUD (clock, speed, pause, the
//! selection card) and the developer event panel. Panels draw a model the
//! app fills and return what the player clicked; they never touch the sim.

use il_core::{RegimentId, TICK_SECONDS, Tick};

/// What the main menu shows.
pub struct MenuModel<'a> {
    /// Scenario files, display names.
    pub scenarios: &'a [String],
    /// Mod roots in load order, display names.
    pub mods: &'a [String],
    /// The last failure to start a battle.
    pub error: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuChoice {
    /// Start the custom battle in `scenarios[index]`.
    Start(usize),
    Exit,
}

/// Draws the main menu; returns the click, if any.
pub fn main_menu(ctx: &egui::Context, model: &MenuModel<'_>) -> Option<MenuChoice> {
    let mut choice = None;
    egui::Window::new("Main menu")
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .title_bar(false)
        .resizable(false)
        .default_width(360.0)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(24.0);
                ui.heading("Iron Legion");
                ui.label("Custom battle from a scenario file");
                ui.add_space(16.0);
                if model.scenarios.is_empty() {
                    ui.label("No scenario files found (pass --scenarios-dir).");
                }
                for (i, name) in model.scenarios.iter().enumerate() {
                    if ui.button(name).clicked() {
                        choice = Some(MenuChoice::Start(i));
                    }
                }
                ui.add_space(16.0);
                if !model.mods.is_empty() {
                    ui.label(format!("Mods: {}", model.mods.join(", ")));
                }
                if let Some(e) = model.error {
                    ui.add_space(8.0);
                    ui.colored_label(egui::Color32::from_rgb(255, 120, 120), e);
                }
                ui.add_space(24.0);
                if ui.button("Exit").clicked() {
                    choice = Some(MenuChoice::Exit);
                }
            });
        });
    choice
}

/// One selected regiment on the selection card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedRegiment {
    pub id: RegimentId,
    /// Localised unit name.
    pub unit: String,
    pub soldiers: u32,
    /// Localised formation name and current ranks.
    pub formation: String,
    pub ranks: u8,
    pub order: &'static str,
}

pub struct HudModel<'a> {
    pub tick: Tick,
    pub paused: bool,
    pub speed: f32,
    /// The run toggle for new orders.
    pub run: bool,
    pub selection: &'a [SelectedRegiment],
    /// Commands recorded so far (the replay-to-be).
    pub commands: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HudAction {
    TogglePause,
    SpeedUp,
    SpeedDown,
    QuitToMenu,
}

/// `mm:ss` of battle time.
pub fn clock(tick: Tick) -> String {
    let seconds = (tick.0 as f64 * f64::from(TICK_SECONDS)).floor() as u64;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

/// Draws the battle HUD; returns the click, if any.
pub fn battle_hud(ctx: &egui::Context, model: &HudModel<'_>) -> Option<HudAction> {
    let mut action = None;
    egui::Window::new("Battle")
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-8.0, 8.0))
        .title_bar(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.monospace(clock(model.tick));
                ui.label(format!("x{:.2}", model.speed));
                if ui.small_button("-").clicked() {
                    action = Some(HudAction::SpeedDown);
                }
                if ui.small_button("+").clicked() {
                    action = Some(HudAction::SpeedUp);
                }
                let pause = if model.paused { "Resume" } else { "Pause" };
                if ui.small_button(pause).clicked() {
                    action = Some(HudAction::TogglePause);
                }
                if ui.small_button("Menu").clicked() {
                    action = Some(HudAction::QuitToMenu);
                }
            });
            ui.label(format!(
                "{} commands · {}",
                model.commands,
                if model.run { "running" } else { "walking" }
            ));
        });
    if !model.selection.is_empty() {
        egui::Window::new("Selection")
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -8.0))
            .title_bar(false)
            .resizable(false)
            .show(ctx, |ui| {
                egui::Grid::new("selection").striped(true).show(ui, |ui| {
                    for r in model.selection {
                        ui.monospace(format!("#{}", r.id.0));
                        ui.label(&r.unit);
                        ui.label(format!("{} soldiers", r.soldiers));
                        ui.label(format!("{} · {} ranks", r.formation, r.ranks));
                        ui.label(r.order);
                        ui.end_row();
                    }
                });
            });
    }
    action
}

/// One routed event or rejected command, for the developer panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventLine {
    pub tick: Tick,
    pub text: String,
}

/// Draws the most recent events, newest last.
pub fn event_panel(ctx: &egui::Context, lines: &[EventLine]) {
    egui::Window::new("Events")
        .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(8.0, -8.0))
        .default_width(420.0)
        .resizable(true)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .max_height(160.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if lines.is_empty() {
                        ui.label("no events yet");
                    }
                    for l in lines {
                        ui.monospace(format!("{:>6} {}", l.tick.0, l.text));
                    }
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_formats_ticks_as_minutes_and_seconds() {
        assert_eq!(clock(Tick(0)), "00:00");
        assert_eq!(clock(Tick(20)), "00:01");
        assert_eq!(clock(Tick(1_219)), "01:00");
        assert_eq!(clock(Tick(1_220)), "01:01");
    }
}
