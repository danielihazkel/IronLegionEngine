//! `RenderSnapshot`: everything a frame needs, copied out of the sim
//! (T1-052, TDD §10.1 `build_snapshot`, SAD §12 T-5).
//!
//! The snapshot owns plain data only, so in Phase 3 it can be filled on the
//! sim thread and sent over a channel unchanged. Positions are world space,
//! already interpolated; projection happens in [`crate::scene`].

use std::collections::BTreeSet;

use glam::Vec2;
use il_core::{RegimentId, Scalar, Tick};
use il_sim_battle::BattleView;

use crate::camera::Camera;

/// Metres of padding around the viewport when culling, so sprites whose
/// ground point is just off screen still draw their upper half.
pub const CULL_PAD_METRES: f32 = 4.0;

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
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EntityCounts {
    pub soldiers: u32,
    pub regiments: u32,
    pub visible_soldiers: u32,
    pub projectiles: u32,
}

#[derive(Clone, Debug)]
pub struct RenderSnapshot {
    pub tick: Tick,
    pub alpha: f32,
    pub camera: Camera,
    pub soldiers: Vec<SoldierInst>,
    pub regiments: Vec<RegimentBlock>,
    /// Projectiles inside the view (T2-031).
    pub projectiles: Vec<ProjectileInst>,
    pub counts: EntityCounts,
}

impl Default for RenderSnapshot {
    fn default() -> Self {
        Self {
            tick: Tick::ZERO,
            alpha: 0.0,
            camera: Camera::new(Vec2::ZERO),
            soldiers: Vec::new(),
            regiments: Vec::new(),
            projectiles: Vec::new(),
            counts: EntityCounts::default(),
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
}

fn v2(p: il_core::V2) -> Vec2 {
    Vec2::new(p.x.to_f32_render(), p.y.to_f32_render())
}

/// Clears and refills `out` from `view`: lerps positions, snaps facings,
/// culls to the camera bounds.
pub fn build_snapshot(view: &BattleView, input: &SnapshotInput, out: &mut RenderSnapshot) {
    out.tick = view.tick();
    out.alpha = input.alpha;
    out.camera = input.camera;
    out.soldiers.clear();
    out.regiments.clear();
    out.projectiles.clear();

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
        });
    }

    let (min, max) = input.camera.visible_bounds(input.screen, CULL_PAD_METRES);
    let units = &view.regs().units;
    let map = view.map();
    let alpha = input.alpha.clamp(0.0, 1.0);
    let mut total = 0u32;
    for s in view.soldiers_unordered() {
        total += 1;
        let prev = v2(s.prev_pos);
        let cur = v2(s.pos);
        let p = prev + (cur - prev) * alpha;
        if p.x < min.x || p.y < min.y || p.x > max.x || p.y > max.y {
            continue;
        }
        let (side, selected) = out
            .regiments
            .binary_search_by_key(&s.regiment, |b| b.id)
            .map(|i| (out.regiments[i].side, out.regiments[i].selected))
            .unwrap_or((u8::MAX, false));
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
    for c in input.corpses {
        let p = Vec2::from(c.pos);
        if p.x < min.x || p.y < min.y || p.x > max.x || p.y > max.y {
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
    // Projectiles: the arc is closed-form from the launch data, so the
    // renderer evaluates it at the interpolated time `tick − 1 + alpha`.
    let mut projectiles = 0u32;
    let now = (view.tick().0 as f32 - 1.0 + alpha).max(0.0);
    for p in view.projectiles() {
        projectiles += 1;
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
        if pos.x < min.x || pos.y < min.y || pos.x > max.x || pos.y > max.y {
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
    };
}
