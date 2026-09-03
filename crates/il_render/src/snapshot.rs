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
}

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
}

#[derive(Clone, Debug)]
pub struct RenderSnapshot {
    pub tick: Tick,
    pub alpha: f32,
    pub camera: Camera,
    pub soldiers: Vec<SoldierInst>,
    pub regiments: Vec<RegimentBlock>,
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
            height: 0.0, // heightmap sampling arrives with the map (T1-030/T1-053)
            facing8: s.facing.to_facing8(),
            sprite_set: units.get(s.unit).sprite_set().index() as u16,
            side,
            moving: (cur - prev).length_squared() > 1e-8,
            selected,
        });
    }
    out.counts = EntityCounts {
        soldiers: total,
        regiments: out.regiments.len() as u32,
        visible_soldiers: out.soldiers.len() as u32,
    };
}
