//! Turns a `RenderSnapshot` into the sprite scene for one frame (T1-052):
//! projection, depth from projected y, facing remap, animation column, tint.

use glam::Vec2;

use il_data::SpriteSet;

use crate::atlas::{AtlasId, anim_column};
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

/// Depth pushed behind the living for a corpse on the same ground line.
const CORPSE_DEPTH_BIAS: f32 = 0.002;

/// A corpse keeps its side's hue at half brightness (T2-022).
pub fn corpse_tint(tint: [u8; 4]) -> [u8; 4] {
    [tint[0] / 2, tint[1] / 2, tint[2] / 2, 230]
}

/// One entry per sprite set, in registry order: the atlas to draw with and
/// its frame table.
pub struct SetAtlas<'a> {
    pub atlas: AtlasId,
    pub set: &'a SpriteSet,
}

/// Clears and refills `out` with one batch per sprite set that has visible
/// soldiers. `time` drives the walk animation.
pub fn scene_from_snapshot(
    snap: &RenderSnapshot,
    screen: Vec2,
    time: f32,
    sets: &[SetAtlas<'_>],
    out: &mut SpriteScene,
) {
    out.clear();
    let cam = &snap.camera;
    let scale = cam.zoom / SHEET_PIXELS_PER_METRE;
    let mut buckets: Vec<Vec<SpriteInstance>> = (0..sets.len()).map(|_| Vec::new()).collect();
    for s in &snap.soldiers {
        let Some(bucket) = buckets.get_mut(usize::from(s.sprite_set)) else {
            continue;
        };
        let sheet = sets[usize::from(s.sprite_set)].set;
        let p = cam.world_to_screen(Vec2::from(s.pos), s.height, screen);
        // Ground point drives the depth so a sprite lower on screen draws in front.
        let ground_y = cam.world_to_screen(Vec2::from(s.pos), 0.0, screen).y;
        // Corpses sit just behind anything living on the same ground line.
        let depth = (1.0 - ground_y / screen.y + if s.corpse { CORPSE_DEPTH_BIAS } else { 0.0 })
            .clamp(0.0, 1.0);
        let column = anim_column(sheet, if s.moving { "walk" } else { "idle" }, time);
        let tint = if s.corpse {
            corpse_tint(side_tint(s.side))
        } else {
            side_tint(s.side)
        };
        bucket.push(SpriteInstance {
            pos: p.to_array(),
            depth,
            frame_facing: SpriteInstance::pack_frame_facing(column, cam.facing_index(s.facing8)),
            tint,
            scale,
            flags: u32::from(s.selected),
            _reserved: 0,
        });
    }
    for (i, bucket) in buckets.into_iter().enumerate() {
        out.push_batch(sets[i].atlas, bucket);
    }
}
