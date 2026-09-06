//! The minimap (T2-090, REQ-UI-001, plan decisions 4 and 11, I6): a small
//! terrain texture (the zone palette, water for open river cells) darkened
//! outside the union of the observer side's line-of-sight discs, rebuilt
//! every `REFRESH_FRAMES`, with a block per regiment the observer sees
//! (own always, remembered enemies as grey ghosts) and the camera's
//! viewport quadrilateral. A left click or drag pans the camera there, a
//! right click orders the selection there; both come back in world metres.
//!
//! World `y` points up on the map (the camera's rotation 0 has +y toward
//! the top of the screen), so row 0 of the texture is the map's north edge.

use egui::{Color32, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions};
use glam::Vec2;
use il_core::{Scalar, V2};
use il_data::Locale;
use il_sim_battle::LoadedMap;

/// Frames between texture rebuilds (TDD §11 budget).
pub const REFRESH_FRAMES: u32 = 10;
/// The texture's long side in pixels.
pub const TEXTURE_LONG_SIDE: usize = 256;
/// The widget's width in points.
pub const WIDGET_WIDTH: f32 = 220.0;
/// Brightness of fogged terrain.
pub const FOG_BRIGHTNESS: f32 = 0.45;
/// Open water (river outside a crossing), the terrain renderer's colour.
pub const WATER: [u8; 3] = [0x3a, 0x6e, 0xa5];
/// A regiment block's half size in points.
const BLOCK_HALF: f32 = 2.5;
const GHOST: [u8; 4] = [170, 170, 170, 200];

/// One regiment block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MiniBlock {
    /// Anchor in world metres.
    pub pos: Vec2,
    pub tint: [u8; 4],
    /// Drawn grey: a remembered enemy the observer no longer sees.
    pub ghost: bool,
    /// Drawn with a highlight outline.
    pub selected: bool,
}

pub struct MinimapInput<'a> {
    pub map: &'a LoadedMap,
    /// Colour per zone index (`LoadedMap::zone_handles` order).
    pub zone_colours: &'a [[u8; 3]],
    /// Per zone index: a crossing (a river cell under it is not water).
    pub zone_crossing: &'a [bool],
    /// The observer side's line-of-sight discs: `(anchor, radius)` metres.
    pub discs: &'a [(Vec2, f32)],
    pub blocks: &'a [MiniBlock],
    /// The camera's viewport corners on the ground plane, world metres.
    pub viewport: [Vec2; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MinimapAction {
    /// Centre the camera here (world metres).
    Pan(Vec2),
    /// Order the selection here (world metres).
    Order(Vec2),
}

/// The fog texture's geometry: metres per pixel and the pixel dimensions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapScale {
    pub metres_per_px: f32,
    pub width: usize,
    pub height: usize,
}

impl MapScale {
    pub fn for_map(map: &LoadedMap) -> Self {
        let w = map.width.to_f32_render().max(1.0);
        let h = map.height.to_f32_render().max(1.0);
        let metres_per_px = w.max(h) / TEXTURE_LONG_SIDE as f32;
        Self {
            metres_per_px,
            width: ((w / metres_per_px).ceil() as usize).max(1),
            height: ((h / metres_per_px).ceil() as usize).max(1),
        }
    }

    /// Pixel column and row (may be outside the texture) of a world point.
    pub fn to_px(&self, p: Vec2) -> (f32, f32) {
        (
            p.x / self.metres_per_px,
            self.height as f32 - p.y / self.metres_per_px,
        )
    }

    /// World metres at the centre of pixel `(x, y)`.
    pub fn px_centre(&self, x: usize, y: usize) -> Vec2 {
        Vec2::new(
            (x as f32 + 0.5) * self.metres_per_px,
            (self.height as f32 - (y as f32 + 0.5)) * self.metres_per_px,
        )
    }
}

/// Builds the fogged terrain image (pure; the tests cover it).
pub fn render_fog(input: &MinimapInput<'_>) -> egui::ColorImage {
    let scale = MapScale::for_map(input.map);
    let (w, h) = (scale.width, scale.height);
    let mut lit = vec![false; w * h];
    for (centre, radius) in input.discs {
        let (cx, cy) = scale.to_px(*centre);
        let r = radius / scale.metres_per_px;
        let x0 = (cx - r).floor().max(0.0) as usize;
        let y0 = (cy - r).floor().max(0.0) as usize;
        let x1 = ((cx + r).ceil().max(0.0) as usize).min(w);
        let y1 = ((cy + r).ceil().max(0.0) as usize).min(h);
        for y in y0..y1 {
            for x in x0..x1 {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                if dx * dx + dy * dy <= r * r {
                    lit[y * w + x] = true;
                }
            }
        }
    }
    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let p = scale.px_centre(x, y);
            let wp = V2::from_f32_data(p.x, p.y);
            let zone = usize::from(input.map.zone_index_at(wp));
            let crossing = input.zone_crossing.get(zone).copied().unwrap_or(false);
            let base = if input.map.river_at(wp) && !crossing {
                WATER
            } else {
                input
                    .zone_colours
                    .get(zone)
                    .copied()
                    .unwrap_or([90, 120, 60])
            };
            let k = if lit[y * w + x] { 1.0 } else { FOG_BRIGHTNESS };
            pixels.push(Color32::from_rgb(
                (base[0] as f32 * k) as u8,
                (base[1] as f32 * k) as u8,
                (base[2] as f32 * k) as u8,
            ));
        }
    }
    egui::ColorImage::new([w, h], pixels)
}

/// The widget: owns the texture and the refresh counter.
#[derive(Default)]
pub struct Minimap {
    texture: Option<TextureHandle>,
    frames_since: u32,
}

impl Minimap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drops the texture (a new battle).
    pub fn reset(&mut self) {
        self.texture = None;
        self.frames_since = 0;
    }

    /// Draws the minimap; returns the click, if any.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        input: &MinimapInput<'_>,
        locale: &Locale,
    ) -> Option<MinimapAction> {
        if self.texture.is_none() || self.frames_since >= REFRESH_FRAMES {
            let image = render_fog(input);
            match self.texture.as_mut() {
                Some(t) => t.set(image, TextureOptions::NEAREST),
                None => {
                    self.texture =
                        Some(ctx.load_texture("il_minimap", image, TextureOptions::NEAREST));
                }
            }
            self.frames_since = 0;
        }
        self.frames_since += 1;
        let texture = self.texture.as_ref()?;
        let map_w = input.map.width.to_f32_render().max(1.0);
        let map_h = input.map.height.to_f32_render().max(1.0);
        let size = egui::vec2(WIDGET_WIDTH, WIDGET_WIDTH * map_h / map_w);
        let mut action = None;
        egui::Window::new(locale.get("il.minimap.title"))
            .id(egui::Id::new("il_minimap"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-8.0, -8.0))
            .title_bar(false)
            .resizable(false)
            .show(ctx, |ui| {
                let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
                let to_widget = |p: Vec2| -> Pos2 {
                    Pos2::new(
                        rect.min.x + (p.x / map_w).clamp(0.0, 1.0) * rect.width(),
                        rect.min.y + (1.0 - p.y / map_h).clamp(0.0, 1.0) * rect.height(),
                    )
                };
                let to_world = |s: Pos2| -> Vec2 {
                    Vec2::new(
                        ((s.x - rect.min.x) / rect.width()).clamp(0.0, 1.0) * map_w,
                        (1.0 - (s.y - rect.min.y) / rect.height()).clamp(0.0, 1.0) * map_h,
                    )
                };
                let painter = ui.painter_at(rect);
                painter.image(
                    texture.id(),
                    rect,
                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
                for b in input.blocks {
                    let c = to_widget(b.pos);
                    let r =
                        Rect::from_center_size(c, egui::vec2(BLOCK_HALF * 2.0, BLOCK_HALF * 2.0));
                    let tint = if b.ghost { GHOST } else { b.tint };
                    painter.rect_filled(
                        r,
                        0.0,
                        Color32::from_rgba_unmultiplied(tint[0], tint[1], tint[2], tint[3]),
                    );
                    if b.selected {
                        painter.rect_stroke(
                            r.expand(1.0),
                            0.0,
                            Stroke::new(1.0, Color32::from_rgb(255, 230, 120)),
                            egui::StrokeKind::Outside,
                        );
                    }
                }
                let corners: Vec<Pos2> = input.viewport.iter().map(|p| to_widget(*p)).collect();
                painter.add(egui::Shape::closed_line(
                    corners,
                    Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 200)),
                ));
                if let Some(pos) = response.interact_pointer_pos() {
                    if response.secondary_clicked() {
                        action = Some(MinimapAction::Order(to_world(pos)));
                    } else if response.clicked() || response.dragged() {
                        action = Some(MinimapAction::Pan(to_world(pos)));
                    }
                }
            });
        action
    }
}
