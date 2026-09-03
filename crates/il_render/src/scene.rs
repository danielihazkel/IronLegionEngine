//! Turns a `RenderSnapshot` into the sprite scene for one frame (T1-052):
//! projection, depth from projected y, facing remap, animation column, tint.

use glam::Vec2;

use crate::atlas::{AtlasId, SpriteSheet};
use crate::snapshot::RenderSnapshot;
use crate::sprite::{SpriteInstance, SpriteScene};

/// Sheet pixels per world metre at `scale = 1` (a 0.4 m soldier radius is a
/// 12 px disc in the placeholder art).
pub const SHEET_PIXELS_PER_METRE: f32 = 30.0;

/// Placeholder faction tints until `Faction.colour_primary` arrives (T1-023).
pub fn side_tint(side: u8) -> [u8; 4] {
    match side {
        0 => [214, 66, 52, 255],
        1 => [64, 96, 214, 255],
        2 => [222, 190, 70, 255],
        3 => [80, 190, 120, 255],
        _ => [200, 200, 200, 255],
    }
}

/// One entry per unit category: the atlas to draw with and its frame table.
pub struct CategoryAtlas<'a> {
    pub atlas: AtlasId,
    pub sheet: &'a SpriteSheet,
}

/// Clears and refills `out` with one batch per category that has visible
/// soldiers. `time` drives the walk animation.
pub fn scene_from_snapshot(
    snap: &RenderSnapshot,
    screen: Vec2,
    time: f32,
    categories: &[CategoryAtlas<'_>],
    out: &mut SpriteScene,
) {
    out.clear();
    let cam = &snap.camera;
    let scale = cam.zoom / SHEET_PIXELS_PER_METRE;
    let mut buckets: Vec<Vec<SpriteInstance>> = (0..categories.len()).map(|_| Vec::new()).collect();
    for s in &snap.soldiers {
        let Some(bucket) = buckets.get_mut(usize::from(s.category)) else {
            continue;
        };
        let sheet = categories[usize::from(s.category)].sheet;
        let p = cam.world_to_screen(Vec2::from(s.pos), s.height, screen);
        // Ground point drives the depth so a sprite lower on screen draws in front.
        let ground_y = cam.world_to_screen(Vec2::from(s.pos), 0.0, screen).y;
        let depth = (1.0 - ground_y / screen.y).clamp(0.0, 1.0);
        let column = sheet.column(if s.moving { "walk" } else { "idle" }, time);
        bucket.push(SpriteInstance {
            pos: p.to_array(),
            depth,
            frame_facing: SpriteInstance::pack_frame_facing(column, cam.facing_index(s.facing8)),
            tint: side_tint(s.side),
            scale,
            flags: u32::from(s.selected),
            _reserved: 0,
        });
    }
    for (i, bucket) in buckets.into_iter().enumerate() {
        out.push_batch(categories[i].atlas, bucket);
    }
}
