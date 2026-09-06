//! egui panels (T1-070, TDD §11 "Panels"): the Phase 1 main menu (custom
//! battle from a scenario file), the battle HUD (clock, speed, pause, the
//! selection card) and the developer event panel. Panels draw a model the
//! app fills and return what the player clicked; they never touch the sim.
//! Every label comes from the locale under `il.*` (REQ-LOC-001), so a mod
//! can translate or reword the engine UI.

use std::fmt::Display;

use il_core::{RegimentId, TICK_SECONDS, Tick};
use il_data::Locale;

/// What the main menu shows.
pub struct MenuModel<'a> {
    /// Scenario files, display names.
    pub scenarios: &'a [String],
    /// Mod roots in load order, display names.
    pub mods: &'a [String],
    /// The last failure to start a battle.
    pub error: Option<&'a str>,
    pub locale: &'a Locale,
}

/// The root menu's buttons (T2-091: custom battle, scenario file, load,
/// settings) and the scenario list's picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuChoice {
    CustomBattle,
    /// Open the scenario file list.
    Scenarios,
    Load,
    Settings,
    /// Start the scenario `scenarios[index]` (the list screen).
    Start(usize),
    /// Back to the root (the list screen).
    Back,
    Exit,
}

/// Draws the root of the main menu; returns the click, if any.
pub fn main_menu(ctx: &egui::Context, model: &MenuModel<'_>) -> Option<MenuChoice> {
    let mut choice = None;
    let l = model.locale;
    egui::Window::new("il_main_menu")
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .title_bar(false)
        .resizable(false)
        .default_width(360.0)
        .show(ctx, |ui| {
            ui.vertical_centered_justified(|ui| {
                ui.add_space(24.0);
                ui.heading(l.get("il.app.title"));
                ui.add_space(16.0);
                for (key, c) in [
                    ("il.menu.custom_battle", MenuChoice::CustomBattle),
                    ("il.menu.scenarios", MenuChoice::Scenarios),
                    ("il.menu.load", MenuChoice::Load),
                    ("il.menu.settings", MenuChoice::Settings),
                ] {
                    if ui.button(l.get(key)).clicked() {
                        choice = Some(c);
                    }
                }
                ui.add_space(16.0);
                if !model.mods.is_empty() {
                    ui.label(l.fmt("il.menu.mods", &[("list", &model.mods.join(", "))]));
                }
                if let Some(e) = model.error {
                    ui.add_space(8.0);
                    ui.colored_label(egui::Color32::from_rgb(255, 120, 120), e);
                }
                ui.add_space(24.0);
                if ui.button(l.get("il.menu.exit")).clicked() {
                    choice = Some(MenuChoice::Exit);
                }
            });
        });
    choice
}

/// Draws the scenario file list (the T1-070 menu); returns the click.
pub fn scenario_list(ctx: &egui::Context, model: &MenuModel<'_>) -> Option<MenuChoice> {
    let mut choice = None;
    let l = model.locale;
    egui::Window::new("il_scenarios")
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .title_bar(false)
        .resizable(false)
        .default_width(360.0)
        .show(ctx, |ui| {
            ui.vertical_centered_justified(|ui| {
                ui.heading(l.get("il.menu.scenarios_title"));
                ui.add_space(8.0);
                if model.scenarios.is_empty() {
                    ui.label(l.get("il.menu.no_scenarios"));
                }
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .show(ui, |ui| {
                        for (i, name) in model.scenarios.iter().enumerate() {
                            if ui.button(name).clicked() {
                                choice = Some(MenuChoice::Start(i));
                            }
                        }
                    });
                if let Some(e) = model.error {
                    ui.add_space(8.0);
                    ui.colored_label(egui::Color32::from_rgb(255, 120, 120), e);
                }
                ui.add_space(16.0);
                if ui.button(l.get("il.menu.back")).clicked() {
                    choice = Some(MenuChoice::Back);
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
    /// Localised order label (`il.order.*`).
    pub order: String,
    /// Morale value and localised state (`il.battle.morale`, T2-040).
    pub morale: String,
    /// Localised fatigue state (`il.fatigue.*`, T2-040).
    pub fatigue: String,
    /// One line per ability slot (`il.battle.ability`, T2-050).
    pub abilities: Vec<String>,
    /// One line per active status (`il.battle.status`, T2-050).
    pub statuses: Vec<String>,
}

pub struct HudModel<'a> {
    pub tick: Tick,
    /// Localised phase (`il.battle.phase.*`, T2-070) and, during the
    /// deployment, the hint line.
    pub phase: String,
    pub deploying: bool,
    pub paused: bool,
    pub speed: f32,
    /// The run toggle for new orders.
    pub run: bool,
    /// Commands recorded so far (the replay-to-be).
    pub commands: usize,
    pub locale: &'a Locale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HudAction {
    TogglePause,
    SpeedUp,
    SpeedDown,
    /// The Menu button: opens the pause menu (T2-090).
    OpenMenu,
    /// The deployment's Confirm button (T2-090, decision 6).
    ConfirmDeployment,
}

/// `mm:ss` of battle time.
pub fn clock(tick: Tick) -> String {
    let seconds = (tick.0 as f64 * f64::from(TICK_SECONDS)).floor() as u64;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

/// Draws the battle HUD; returns the click, if any.
pub fn battle_hud(ctx: &egui::Context, model: &HudModel<'_>) -> Option<HudAction> {
    let mut action = None;
    let l = model.locale;
    egui::Window::new("il_battle_hud")
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-8.0, 8.0))
        .title_bar(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.monospace(clock(model.tick));
                ui.label(l.fmt(
                    "il.battle.speed",
                    &[("mult", &format!("{:.2}", model.speed))],
                ));
                if ui.small_button("-").clicked() {
                    action = Some(HudAction::SpeedDown);
                }
                if ui.small_button("+").clicked() {
                    action = Some(HudAction::SpeedUp);
                }
                let pause = if model.paused {
                    l.get("il.battle.resume")
                } else {
                    l.get("il.battle.pause")
                };
                if ui.small_button(pause).clicked() {
                    action = Some(HudAction::TogglePause);
                }
                if ui.small_button(l.get("il.battle.menu")).clicked() {
                    action = Some(HudAction::OpenMenu);
                }
            });
            let mode = if model.run {
                l.get("il.battle.running")
            } else {
                l.get("il.battle.walking")
            };
            ui.label(l.fmt(
                "il.battle.commands",
                &[("count", &model.commands as &dyn Display), ("mode", &mode)],
            ));
            ui.horizontal(|ui| {
                ui.label(&model.phase);
                if model.deploying && ui.button(l.get("il.battle.confirm")).clicked() {
                    action = Some(HudAction::ConfirmDeployment);
                }
            });
            if model.deploying {
                ui.label(l.get("il.battle.deploy_hint"));
            }
        });
    action
}

/// One routed event or rejected command, for the developer panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventLine {
    pub tick: Tick,
    pub text: String,
}

/// Draws the most recent events, newest last.
pub fn event_panel(ctx: &egui::Context, locale: &Locale, lines: &[EventLine]) {
    egui::Window::new(locale.get("il.events.title"))
        .id(egui::Id::new("il_events"))
        .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(8.0, -8.0))
        .default_width(420.0)
        .resizable(true)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .max_height(160.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if lines.is_empty() {
                        ui.label(locale.get("il.events.none"));
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
