//! The casualties line (T2-090, REQ-UI-001, plan decision 11): one row per
//! side at the top of the screen with the soldiers alive, killed and fled
//! (withdrawn soldiers count with the fled here; the result screen keeps
//! them apart). The app tallies the sim's death, flight and withdrawal
//! events (plan I8), so no sim pass happens per frame.

use egui::Color32;
use il_data::Locale;

/// One side's running totals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SideTally {
    /// Localised faction name (or `Side n`).
    pub name: String,
    /// The side's tint (`il_render::side_tint`, handed in by the app).
    pub tint: [u8; 4],
    pub alive: u32,
    pub killed: u32,
    pub fled: u32,
}

/// Draws the line; nothing when there are no sides.
pub fn casualties_line(ctx: &egui::Context, tallies: &[SideTally], locale: &Locale) {
    if tallies.is_empty() {
        return;
    }
    egui::Window::new("il_casualties")
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 8.0))
        .title_bar(false)
        .resizable(false)
        .show(ctx, |ui| {
            egui::Grid::new("il_casualties_grid").show(ui, |ui| {
                for t in tallies {
                    ui.colored_label(
                        Color32::from_rgba_unmultiplied(t.tint[0], t.tint[1], t.tint[2], t.tint[3]),
                        &t.name,
                    );
                    ui.label(locale.fmt(
                        "il.casualties.line",
                        &[
                            ("alive", &t.alive),
                            ("killed", &t.killed),
                            ("fled", &t.fled),
                        ],
                    ));
                    ui.end_row();
                }
            });
        });
}
