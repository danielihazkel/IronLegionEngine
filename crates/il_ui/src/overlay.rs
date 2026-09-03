//! Screen-space overlays drawn with egui's background painter (T1-061):
//! the box-select rectangle and, from T1-062, the drag-formation preview.

use egui::{Color32, Pos2, Rect, Stroke, StrokeKind};
use glam::Vec2;

fn pos(v: Vec2, ppp: f32) -> Pos2 {
    Pos2::new(v.x / ppp, v.y / ppp)
}

/// Draws the box-select rectangle between two physical-pixel corners.
pub fn selection_box(ctx: &egui::Context, from: Vec2, to: Vec2) {
    let ppp = ctx.pixels_per_point();
    let rect = Rect::from_two_pos(pos(from, ppp), pos(to, ppp));
    let painter = ctx.layer_painter(egui::LayerId::background());
    painter.rect_filled(
        rect,
        0.0,
        Color32::from_rgba_unmultiplied(120, 200, 255, 30),
    );
    painter.rect_stroke(
        rect,
        0.0,
        Stroke::new(1.0, Color32::from_rgb(140, 220, 255)),
        StrokeKind::Outside,
    );
}
