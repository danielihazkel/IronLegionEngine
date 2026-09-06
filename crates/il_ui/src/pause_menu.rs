//! The pause menu (T2-090, plan decision 10): Escape or the HUD's Menu
//! button pauses the battle and opens it; Resume restores the previous
//! pause state, Surrender sends the `Surrender` command, Settings opens
//! the settings screen (T2-091) and Quit returns to the main menu after the
//! replay is written (T2-101).

use il_data::Locale;

pub struct PauseModel<'a> {
    /// A spectator or an ended battle cannot surrender.
    pub can_surrender: bool,
    /// The settings screen exists (T2-091); hidden until then.
    pub has_settings: bool,
    pub locale: &'a Locale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseAction {
    Resume,
    Surrender,
    Settings,
    Quit,
}

/// Draws the menu; returns the click, if any.
pub fn pause_menu(ctx: &egui::Context, model: &PauseModel<'_>) -> Option<PauseAction> {
    let l = model.locale;
    let mut action = None;
    egui::Window::new(l.get("il.pause.title"))
        .id(egui::Id::new("il_pause_menu"))
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .collapsible(false)
        .resizable(false)
        .default_width(240.0)
        .show(ctx, |ui| {
            ui.vertical_centered_justified(|ui| {
                if ui.button(l.get("il.pause.resume")).clicked() {
                    action = Some(PauseAction::Resume);
                }
                if ui
                    .add_enabled(
                        model.can_surrender,
                        egui::Button::new(l.get("il.pause.surrender")),
                    )
                    .clicked()
                {
                    action = Some(PauseAction::Surrender);
                }
                if model.has_settings && ui.button(l.get("il.pause.settings")).clicked() {
                    action = Some(PauseAction::Settings);
                }
                if ui.button(l.get("il.pause.quit")).clicked() {
                    action = Some(PauseAction::Quit);
                }
            });
        });
    action
}
