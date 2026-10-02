//! `RenderSnapshot`: everything a frame needs, copied out of the sim
//! (T1-052, TDD §10.1 `build_snapshot`, SAD §12 T-5).
//!
//! The snapshot owns plain data only, so it can be filled on the sim
//! thread and handed on unchanged. Positions are world space, already
//! interpolated; projection happens in [`crate::scene`]. T3-031 adds the
//! level-of-detail tier (REQ-RNDR-004): the tier is chosen once per frame
//! from the camera zoom and the two thresholds, and the aggregation tier
//! replaces the soldiers of a regiment in formation by one block.

use std::collections::BTreeSet;

use glam::Vec2;
use il_core::{RegimentId, Scalar, SoldierId, Tick};
use il_data::Registries;
use il_sim_battle::BattleFrame;
use il_sim_battle::components::SoldierState;
use il_sim_battle::formation::layout::spacing;

use crate::camera::Camera;

/// Metres of padding around the viewport when culling, so sprites whose
/// ground point is just off screen still draw their upper half.
pub const CULL_PAD_METRES: f32 = 4.0;

/// Level-of-detail tier (REQ-RNDR-004, TDD §10.1 LOD, T3-031).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DetailTier {
    /// Every visible soldier with its full frame and animation.
    #[default]
    Detailed,
    /// Every visible soldier, one frame per state, no animation; corpses
    /// thinned to every fourth.
    Reduced,
    /// One block per regiment rank block; only routing and withdrawing
    /// soldiers still draw as sprites; no projectiles, no corpses.
    Aggregation,
}

impl DetailTier {
    pub fn label(self) -> &'static str {
        match self {
            Self::Detailed => "detailed",
            Self::Reduced => "reduced",
            Self::Aggregation => "aggregation",
        }
    }
}

/// Starting thresholds in camera pixels per metre (Video settings, §15).
pub const DETAIL_Z1: f32 = 24.0;
pub const DETAIL_Z2: f32 = 8.0;

/// `zoom ≥ z1` Detailed, `z2 ≤ zoom < z1` Reduced, below Aggregation
/// (zoom is pixels per metre, so far away is small).
pub fn detail_tier(zoom: f32, z1: f32, z2: f32) -> DetailTier {
    if zoom >= z1 {
        DetailTier::Detailed
    } else if zoom >= z2 {
        DetailTier::Reduced
    } else {
        DetailTier::Aggregation
    }
}

/// The item part of the sprite set drawn per regiment at the aggregation
/// tier (`<namespace>:sprites_blocks`; `il_cli genart` writes the
/// placeholder). Without one the tier draws reduced sprites instead.
pub const BLOCK_SET_NAME: &str = "sprites_blocks";

/// Registry index of the block sheet, if the content has one.
pub fn block_set_index(regs: &Registries) -> Option<u16> {
    regs.sprite_sets
        .iter()
        .find(|(_, s)| s.id.item() == BLOCK_SET_NAME)
        .map(|(h, _)| h.index() as u16)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoldierInst {
    /// Interpolated world position.
    pub pos: [f32; 2],
    pub height: f32,
    /// World facing in eighths of a turn (not interpolated: facing snaps).
    pub facing8: u8,
    /// Registry index of the unit's sprite set.
    pub sprite_set: u16,
    pub side: u8,
    pub moving: bool,
    pub selected: bool,
    /// A corpse (T2-022): drawn dark at ground depth, never animated.
    pub corpse: bool,
}

/// A dead soldier the app remembers for `combat.corpse_ticks` after its
/// `SoldierDied` event (SIM-CORE-008: render-only; the sim forgot it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Corpse {
    /// The soldier that fell (the reduced tier thins corpses by it, T3-031).
    pub id: SoldierId,
    pub pos: [f32; 2],
    pub side: u8,
    pub sprite_set: u16,
    pub facing8: u8,
    pub died: Tick,
}

/// A projectile in flight (T2-031): a short segment along its direction of
/// travel at its interpolated position, lifted by the arc height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectileInst {
    /// World-space ends of the segment.
    pub a: [f32; 2],
    pub b: [f32; 2],
    /// Ground height plus the arc height at the midpoint.
    pub height: f32,
    pub side: u8,
}

/// Half-length of a drawn projectile, metres.
pub const PROJECTILE_HALF_LENGTH: f32 = 0.4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegimentBlock {
    pub id: RegimentId,
    pub side: u8,
    pub anchor: [f32; 2],
    pub facing8: u8,
    pub count: u32,
    pub selected: bool,
    /// Seen by the observer side (T2-060); hidden regiments draw no soldiers.
    pub visible: bool,
}

/// One regiment's rank block at the aggregation tier (T3-031): the
/// geometry the block quad is built from in [`crate::scene::block_axes`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockInst {
    /// The anchor: the centre of the front rank, world metres.
    pub anchor: [f32; 2],
    /// Ground height at the anchor.
    pub height: f32,
    /// World facing in radians (the geometry rotates the block, not the art).
    pub facing: f32,
    pub ranks: u8,
    pub files: u16,
    /// Living soldiers in formation (routing and withdrawing ones excluded:
    /// they draw as sprites).
    pub count: u16,
    /// `ranks × files`: the density is `count / capacity`.
    pub capacity: u16,
    /// File and rank spacing in metres (the template's multipliers of the
    /// widest group's soldier diameter).
    pub spacing: [f32; 2],
    pub side: u8,
    pub selected: bool,
}

/// A remembered enemy regiment the observer no longer sees (SIM-VIS-005,
/// T2-060): drawn as a grey marker at its last known anchor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GhostInst {
    pub id: RegimentId,
    pub side: u8,
    pub pos: [f32; 2],
    pub facing8: u8,
    pub count: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EntityCounts {
    pub soldiers: u32,
    pub regiments: u32,
    pub visible_soldiers: u32,
    pub projectiles: u32,
    /// Blocks drawn at the aggregation tier (T3-031).
    pub blocks: u32,
}

#[derive(Clone, Debug)]
pub struct RenderSnapshot {
    pub tick: Tick,
    pub alpha: f32,
    pub camera: Camera,
    /// The tier this frame was built at (T3-031).
    pub tier: DetailTier,
    pub soldiers: Vec<SoldierInst>,
    pub regiments: Vec<RegimentBlock>,
    /// One per regiment in formation at the aggregation tier, else empty.
    pub blocks: Vec<BlockInst>,
    /// Projectiles inside the view (T2-031).
    pub projectiles: Vec<ProjectileInst>,
    /// Remembered enemy regiments inside the view (T2-060).
    pub ghosts: Vec<GhostInst>,
    pub counts: EntityCounts,
    /// Scratch: soldiers in formation per regiment (parallel to
    /// `regiments`), kept so the aggregation tier allocates nothing.
    pub tally: Vec<u16>,
}

impl Default for RenderSnapshot {
    fn default() -> Self {
        Self {
            tick: Tick::ZERO,
            alpha: 0.0,
            camera: Camera::new(Vec2::ZERO),
            tier: DetailTier::Detailed,
            soldiers: Vec::new(),
            regiments: Vec::new(),
            blocks: Vec::new(),
            projectiles: Vec::new(),
            ghosts: Vec::new(),
            counts: EntityCounts::default(),
            tally: Vec::new(),
        }
    }
}

pub struct SnapshotInput<'a> {
    /// Interpolation factor in `[0, 1)`.
    pub alpha: f32,
    pub camera: Camera,
    /// Viewport size in pixels.
    pub screen: Vec2,
    pub selected: &'a BTreeSet<RegimentId>,
    /// Corpses to draw (T2-022).
    pub corpses: &'a [Corpse],
    /// The side whose fog of war applies (T2-060); `None` draws everything.
    pub observer_side: Option<u8>,
    /// The LOD thresholds (T3-031; `DETAIL_Z1` / `DETAIL_Z2` by default).
    pub detail_z1: f32,
    pub detail_z2: f32,
    /// Registry index of the block sheet (`block_set_index`); `None` makes
    /// the aggregation tier draw reduced sprites instead.
    pub block_set: Option<u16>,
}

fn v2(p: il_core::V2) -> Vec2 {
    Vec2::new(p.x.to_f32_render(), p.y.to_f32_render())
}

fn outside(p: Vec2, min: Vec2, max: Vec2) -> bool {
    p.x < min.x || p.y < min.y || p.x > max.x || p.y > max.y
}

/// Clears and refills `out` from `view`: picks the tier, lerps positions,
/// snaps facings, culls to the camera bounds.
pub fn build_snapshot(view: &BattleFrame, input: &SnapshotInput, out: &mut RenderSnapshot) {
    out.tick = view.tick();
    out.alpha = input.alpha;
    out.camera = input.camera;
    out.tier = detail_tier(input.camera.zoom, input.detail_z1, input.detail_z2);
    out.soldiers.clear();
    out.regiments.clear();
    out.blocks.clear();
    out.projectiles.clear();
    out.ghosts.clear();
    let tier = out.tier;
    // Blocks need a sheet; without one the far tier draws reduced sprites.
    let aggregate = tier == DetailTier::Aggregation && input.block_set.is_some();

    // Regiment table first: side and selection per regiment, ascending id so
    // soldiers can binary-search it.
    for r in view.regiments() {
        out.regiments.push(RegimentBlock {
            id: r.id,
            side: r.side,
            anchor: v2(r.anchor_pos).to_array(),
            facing8: r.anchor_facing.to_facing8(),
            count: r.soldier_count,
            selected: input.selected.contains(&r.id),
            visible: input.observer_side.is_none_or(|o| view.visible(o, r.id)),
        });
    }
    out.tally.clear();
    out.tally.resize(out.regiments.len(), 0);

    let (min, max) = input.camera.visible_bounds(input.screen, CULL_PAD_METRES);
    let units = &view.regs().units;
    let map = view.map();
    let alpha = input.alpha.clamp(0.0, 1.0);
    let mut total = 0u32;
    for s in view.soldiers_unordered() {
        total += 1;
        if aggregate && !matches!(s.state, SoldierState::Routing | SoldierState::Withdrawing) {
            // In formation: counted into its regiment's block, never drawn.
            if let Ok(i) = out.regiments.binary_search_by_key(&s.regiment, |b| b.id) {
                out.tally[i] = out.tally[i].saturating_add(1);
            }
            continue;
        }
        let prev = v2(s.prev_pos);
        let cur = v2(s.pos);
        let p = prev + (cur - prev) * alpha;
        if outside(p, min, max) {
            continue;
        }
        let (side, selected, visible) = out
            .regiments
            .binary_search_by_key(&s.regiment, |b| b.id)
            .map(|i| {
                let b = &out.regiments[i];
                (b.side, b.selected, b.visible)
            })
            .unwrap_or((u8::MAX, false, true));
        if !visible {
            continue;
        }
        out.soldiers.push(SoldierInst {
            pos: p.to_array(),
            height: crate::terrain::ground_height(map, p),
            facing8: s.facing.to_facing8(),
            sprite_set: units.get(s.unit).sprite_set().index() as u16,
            side,
            moving: (cur - prev).length_squared() > 1e-8,
            selected,
            corpse: false,
        });
    }
    // Corpses: all of them close up, every fourth at the reduced tier, none
    // at the aggregation tier (REQ-RNDR-009 far-zoom culling).
    if tier != DetailTier::Aggregation {
        for c in input.corpses {
            if tier == DetailTier::Reduced && !c.id.0.is_multiple_of(4) {
                continue;
            }
            let p = Vec2::from(c.pos);
            if outside(p, min, max) {
                continue;
            }
            out.soldiers.push(SoldierInst {
                pos: c.pos,
                height: crate::terrain::ground_height(map, p),
                facing8: c.facing8,
                sprite_set: c.sprite_set,
                side: c.side,
                moving: false,
                selected: false,
                corpse: true,
            });
        }
    }
    // Ghosts (SIM-VIS-005): the observer's memory of hidden regiments.
    if let Some(o) = input.observer_side {
        for b in &out.regiments {
            if b.visible {
                continue;
            }
            let Some(seen) = view.seen(o, b.id) else {
                continue;
            };
            let p = v2(seen.pos);
            if outside(p, min, max) {
                continue;
            }
            out.ghosts.push(GhostInst {
                id: b.id,
                side: b.side,
                pos: p.to_array(),
                facing8: seen.facing.to_facing8(),
                count: seen.count,
            });
        }
    }
    // Blocks (T3-031): one per visible regiment with soldiers in formation,
    // culled by the anchor padded with the block's half-diagonal.
    if aggregate {
        let formations = &view.regs().formations;
        for (i, b) in out.regiments.iter().enumerate() {
            let count = out.tally[i];
            if count == 0 || !b.visible {
                continue;
            }
            let Some(r) = view.regiment(b.id) else {
                continue;
            };
            let (sf, sr) = spacing(formations.get(r.formation), r.radius);
            let spacing = [sf.to_f32_render(), sr.to_f32_render()];
            let w = f32::from(r.files) * spacing[0];
            let d = f32::from(r.ranks) * spacing[1];
            let half_diagonal = 0.5 * (w * w + d * d).sqrt();
            let anchor = Vec2::from(b.anchor);
            if outside(
                anchor,
                min - Vec2::splat(half_diagonal),
                max + Vec2::splat(half_diagonal),
            ) {
                continue;
            }
            out.blocks.push(BlockInst {
                anchor: b.anchor,
                height: crate::terrain::ground_height(map, anchor),
                facing: r.anchor_facing.radians().to_f32_render(),
                ranks: r.ranks,
                files: r.files,
                count,
                capacity: u16::from(r.ranks).saturating_mul(r.files).max(1),
                spacing,
                side: b.side,
                selected: b.selected,
            });
        }
    }
    // Projectiles: the arc is closed-form from the launch data, so the
    // renderer evaluates it at the interpolated time `tick − 1 + alpha`.
    // Culled entirely at the aggregation tier (REQ-RNDR-009).
    let mut projectiles = 0u32;
    let now = (view.tick().0 as f32 - 1.0 + alpha).max(0.0);
    for p in view.projectiles() {
        projectiles += 1;
        if tier == DetailTier::Aggregation {
            continue;
        }
        let launch = p.launch_tick.0 as f32;
        let land = p.land_tick.0 as f32;
        let u = if land > launch {
            ((now - launch) / (land - launch)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let start = v2(p.start);
        let end = v2(p.end);
        let pos = start + (end - start) * u;
        if outside(pos, min, max) {
            continue;
        }
        let dir = (end - start).normalize_or_zero() * PROJECTILE_HALF_LENGTH;
        let z = p.apex.to_f32_render() * 4.0 * u * (1.0 - u);
        out.projectiles.push(ProjectileInst {
            a: (pos - dir).to_array(),
            b: (pos + dir).to_array(),
            height: crate::terrain::ground_height(map, pos) + z,
            side: p.side,
        });
    }
    out.counts = EntityCounts {
        soldiers: total,
        regiments: out.regiments.len() as u32,
        visible_soldiers: out.soldiers.len() as u32,
        projectiles,
        blocks: out.blocks.len() as u32,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_boundaries_follow_the_thresholds() {
        let (z1, z2) = (DETAIL_Z1, DETAIL_Z2);
        assert_eq!(detail_tier(z1, z1, z2), DetailTier::Detailed);
        assert_eq!(detail_tier(z1 - 1e-3, z1, z2), DetailTier::Reduced);
        assert_eq!(detail_tier(z2, z1, z2), DetailTier::Reduced);
        assert_eq!(detail_tier(z2 - 1e-3, z1, z2), DetailTier::Aggregation);
        assert_eq!(
            detail_tier(Camera::MIN_ZOOM, z1, z2),
            DetailTier::Aggregation
        );
        assert_eq!(detail_tier(Camera::MAX_ZOOM, z1, z2), DetailTier::Detailed);
        assert_eq!(
            detail_tier(Camera::DEFAULT_ZOOM, z1, z2),
            DetailTier::Reduced,
            "the default zoom 12 starts reduced with the starting thresholds"
        );
        assert_eq!(DetailTier::Aggregation.label(), "aggregation");
    }
}
