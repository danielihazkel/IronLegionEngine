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

/// Draws the drag-formation preview: the line the regiments will stand on
/// between `from` and `to`, and an arrow from its midpoint to `tip`, the
/// projected point one step ahead along the facing (T1-062).
pub fn drag_formation_preview(ctx: &egui::Context, from: Vec2, to: Vec2, tip: Vec2) {
    let ppp = ctx.pixels_per_point();
    let painter = ctx.layer_painter(egui::LayerId::background());
    let colour = Color32::from_rgb(255, 220, 120);
    let stroke = Stroke::new(2.0, colour);
    let (a, b, t) = (pos(from, ppp), pos(to, ppp), pos(tip, ppp));
    painter.line_segment([a, b], stroke);
    let mid = Pos2::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5);
    painter.line_segment([mid, t], stroke);
    let dir = egui::Vec2::new(t.x - mid.x, t.y - mid.y);
    let len = dir.length();
    if len > 1.0 {
        let d = dir / len;
        let n = egui::Vec2::new(-d.y, d.x);
        let head = 8.0;
        painter.line_segment([t, t - d * head + n * head * 0.5], stroke);
        painter.line_segment([t, t - d * head - n * head * 0.5], stroke);
    }
}
