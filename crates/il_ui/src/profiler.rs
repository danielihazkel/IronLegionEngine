//! Profiler overlay (T1-060, REQ-TOOL-003, SAD §9.3): per-stage tick time,
//! frame time, entity counts. The numbers come from the app, which owns the
//! clock; this module only draws them.

use std::fmt::Display;

use il_data::Locale;

/// One stage's timings in milliseconds over the recent window.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StageStat {
    pub name: &'static str,
    pub last_ms: f32,
    pub mean_ms: f32,
    pub max_ms: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProfilerStats {
    /// One entry per sim stage, in schedule order.
    pub stages: Vec<StageStat>,
    pub tick_last_ms: f32,
    pub tick_mean_ms: f32,
    pub tick_max_ms: f32,
    /// Ticks that contributed to the means.
    pub ticks_sampled: u32,
    pub frame_ms: f32,
    pub fps: f32,
    pub soldiers: u32,
    pub regiments: u32,
    pub visible_soldiers: u32,
    /// Ticks stepped in the last frame (0 while paused).
    pub ticks_last_frame: u32,
    /// Wall seconds the accumulator is behind, as a fraction of a tick.
    pub accumulator_alpha: f32,
}

/// Draws the overlay window. Returns nothing; the caller decides visibility.
pub fn profiler_overlay(ctx: &egui::Context, locale: &Locale, stats: &ProfilerStats) {
    egui::Window::new(locale.get("il.profiler.title"))
        .id(egui::Id::new("il_profiler"))
        .default_pos(egui::pos2(8.0, 8.0))
        .default_width(360.0)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label(locale.fmt(
                "il.profiler.frame",
                &[
                    ("ms", &format!("{:.2}", stats.frame_ms) as &dyn Display),
                    ("fps", &format!("{:.0}", stats.fps)),
                    ("last", &format!("{:.2}", stats.tick_last_ms)),
                    ("mean", &format!("{:.2}", stats.tick_mean_ms)),
                    ("max", &format!("{:.2}", stats.tick_max_ms)),
                    ("ticks", &stats.ticks_sampled),
                ],
            ));
            ui.label(locale.fmt(
                "il.profiler.counts",
                &[
                    ("soldiers", &stats.soldiers as &dyn Display),
                    ("drawn", &stats.visible_soldiers),
                    ("regiments", &stats.regiments),
                    ("ticks", &stats.ticks_last_frame),
                    ("alpha", &format!("{:.2}", stats.accumulator_alpha)),
                ],
            ));
            ui.separator();
            egui::Grid::new("stages")
                .num_columns(4)
                .striped(true)
                .min_col_width(60.0)
                .show(ui, |ui| {
                    ui.strong(locale.get("il.profiler.stage"));
                    ui.strong(locale.get("il.profiler.last"));
                    ui.strong(locale.get("il.profiler.mean"));
                    ui.strong(locale.get("il.profiler.max"));
                    ui.end_row();
                    for (i, s) in stats.stages.iter().enumerate() {
                        ui.monospace(format!("{i:>2} {}", s.name));
                        ui.monospace(format!("{:.3}", s.last_ms));
                        ui.monospace(format!("{:.3}", s.mean_ms));
                        ui.monospace(format!("{:.3}", s.max_ms));
                        ui.end_row();
                    }
                });
        });
}
