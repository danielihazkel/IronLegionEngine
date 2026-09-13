//! Turns a `RenderSnapshot` into the sprite scene for one frame (T1-052):
//! projection, depth from projected y, facing remap, animation column, tint.
//! T3-031: the reduced tier picks one frame per state, the aggregation tier
//! adds one block quad per regiment through the block sheet.

use glam::Vec2;

use il_data::SpriteSet;

use crate::atlas::{AtlasId, anim_column};
use crate::camera::Camera;
use crate::snapshot::{BlockInst, DetailTier, RenderSnapshot};
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

/// A block's brightness at zero density; full density is the plain tint.
const BLOCK_MIN_BRIGHTNESS: f32 = 0.45;

/// A corpse keeps its side's hue at half brightness (T2-022).
pub fn corpse_tint(tint: [u8; 4]) -> [u8; 4] {
    [tint[0] / 2, tint[1] / 2, tint[2] / 2, 230]
}

/// The side tint shaded by the block's density (`count / capacity`).
pub fn block_tint(side: u8, density: f32) -> [u8; 4] {
    let k = BLOCK_MIN_BRIGHTNESS + (1.0 - BLOCK_MIN_BRIGHTNESS) * density.clamp(0.0, 1.0);
    let t = side_tint(side);
    let shade = |c: u8| (f32::from(c) * k).round().clamp(0.0, 255.0) as u8;
    [shade(t[0]), shade(t[1]), shade(t[2]), 255]
}

/// One entry per sprite set, in registry order: the atlas to draw with and
/// its frame table.
pub struct SetAtlas<'a> {
    pub atlas: AtlasId,
    pub set: &'a SpriteSet,
}

/// The quad of a rank block on screen: its centre, its two axes (the full
/// file extent and the full rank extent, front edge at `−ay / 2`) and its
/// depth from the centre's ground point. The isometric projection is
/// affine on the ground plane, so the rotated rectangle is exactly this
/// parallelogram.
pub fn block_axes(cam: &Camera, screen: Vec2, b: &BlockInst) -> (Vec2, Vec2, Vec2, f32) {
    let forward = Vec2::new(b.facing.cos(), b.facing.sin());
    // The formation's local frame: x right, y forward (`formation::frame`).
    let right = Vec2::new(forward.y, -forward.x);
    let w = f32::from(b.files) * b.spacing[0];
    let d = f32::from(b.ranks) * b.spacing[1];
    let anchor = Vec2::from(b.anchor);
    // The anchor is the front rank's centre; the ranks stand behind it.
    let centre = anchor - forward * ((f32::from(b.ranks) - 1.0) * b.spacing[1] * 0.5);
    let p = |q: Vec2| cam.world_to_screen(q, b.height, screen);
    let ax = p(centre + right * (w * 0.5)) - p(centre - right * (w * 0.5));
    let ay = p(centre - forward * (d * 0.5)) - p(centre + forward * (d * 0.5));
    let ground_y = cam.world_to_screen(centre, 0.0, screen).y;
    let depth = (1.0 - ground_y / screen.y).clamp(0.0, 1.0);
    (p(centre), ax, ay, depth)
}

/// Clears and refills `out` with one batch per sprite set that has visible
/// soldiers, then the blocks (T3-031). `time` drives the walk animation at
/// the detailed tier; `block_set` is the block sheet's atlas.
pub fn scene_from_snapshot(
    snap: &RenderSnapshot,
    screen: Vec2,
    time: f32,
    sets: &[SetAtlas<'_>],
    block_set: Option<AtlasId>,
    out: &mut SpriteScene,
) {
    out.clear();
    let cam = &snap.camera;
    let scale = cam.zoom / SHEET_PIXELS_PER_METRE;
    // Reduced and aggregation tiers: the first frame of the state's
    // animation, never advanced.
    let anim_time = if snap.tier == DetailTier::Detailed {
        time
    } else {
        0.0
    };
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
        let column = anim_column(sheet, if s.moving { "walk" } else { "idle" }, anim_time);
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
    if let Some(atlas) = block_set
        && snap.tier == DetailTier::Aggregation
    {
        let blocks = snap.blocks.iter().map(|b| {
            let (centre, ax, ay, depth) = block_axes(cam, screen, b);
            let density = f32::from(b.count) / f32::from(b.capacity.max(1));
            SpriteInstance::block(
                centre,
                depth,
                ax,
                ay,
                block_tint(b.side, density),
                b.selected,
            )
        });
        out.push_batch(atlas, blocks);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unrotated_block_projects_to_its_extents() {
        let screen = Vec2::new(1280.0, 800.0);
        let mut cam = Camera::new(Vec2::new(100.0, 100.0));
        cam.zoom = 4.0;
        // Facing +y (north): files run along x, ranks behind along −y.
        let b = BlockInst {
            anchor: [100.0, 100.0],
            height: 0.0,
            facing: std::f32::consts::FRAC_PI_2,
            ranks: 4,
            files: 10,
            count: 40,
            capacity: 40,
            spacing: [1.0, 2.0],
            side: 0,
            selected: false,
        };
        let (centre, ax, ay, depth) = block_axes(&cam, screen, &b);
        // 10 files × 1 m × 4 px = 40 px wide, along +x on screen.
        assert!((ax.x - 40.0).abs() < 1e-3 && ax.y.abs() < 1e-3, "{ax}");
        // 4 ranks × 2 m = 8 m deep × 4 px × pitch 0.5 = 16 px, downward on
        // screen (the back edge is south, so lower).
        assert!(ay.x.abs() < 1e-3 && (ay.y - 16.0).abs() < 1e-3, "{ay}");
        // The centre sits 3 m behind the anchor: 6 px below the screen centre.
        assert!((centre.x - 640.0).abs() < 1e-3 && (centre.y - 406.0).abs() < 1e-3);
        assert!(depth > 0.0 && depth < 1.0);
        assert_eq!(block_tint(0, 1.0), side_tint(0));
        let dim = block_tint(0, 0.0);
        assert!(dim[0] < side_tint(0)[0] && dim[3] == 255);
    }
}
