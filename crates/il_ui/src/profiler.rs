//! Profiler overlay (T1-060, REQ-TOOL-003, SAD §9.3): per-stage tick time,
//! frame time, entity counts. The numbers come from the app, which owns the
//! clock; this module only draws them.

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
pub fn profiler_overlay(ctx: &egui::Context, stats: &ProfilerStats) {
    egui::Window::new("Profiler")
        .default_pos(egui::pos2(8.0, 8.0))
        .default_width(360.0)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label(format!(
                "frame {:.2} ms ({:.0} FPS) · tick {:.2} ms last / {:.2} mean / {:.2} max over {} ticks",
                stats.frame_ms,
                stats.fps,
                stats.tick_last_ms,
                stats.tick_mean_ms,
                stats.tick_max_ms,
                stats.ticks_sampled
            ));
            ui.label(format!(
                "{} soldiers ({} drawn) · {} regiments · {} ticks this frame · alpha {:.2}",
                stats.soldiers,
                stats.visible_soldiers,
                stats.regiments,
                stats.ticks_last_frame,
                stats.accumulator_alpha
            ));
            ui.separator();
            egui::Grid::new("stages")
                .num_columns(4)
                .striped(true)
                .min_col_width(60.0)
                .show(ui, |ui| {
                    ui.strong("stage");
                    ui.strong("last ms");
                    ui.strong("mean ms");
                    ui.strong("max ms");
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
