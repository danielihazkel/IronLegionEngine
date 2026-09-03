//! Debug overlays (T1-054, TDD §10.1 debug, REQ-RNDR-008): nav grid, slots,
//! paths, anchors and facings, spatial cells, drawn as lines from a
//! `BattleView` (never `&mut`) into the frame's `LineScene`.

use glam::Vec2;
use il_core::{Scalar, V2};
use il_sim_battle::components::Anchor;
use il_sim_battle::{BattleView, slot_world};

use crate::camera::Camera;
use crate::lines::LineScene;
use crate::scene::side_tint;
use crate::terrain::project;

/// Which overlays to draw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DebugFlags {
    pub nav_grid: bool,
    pub slots: bool,
    pub paths: bool,
    pub anchors: bool,
    pub spatial_cells: bool,
}

impl DebugFlags {
    pub fn any(self) -> bool {
        self.nav_grid || self.slots || self.paths || self.anchors || self.spatial_cells
    }
}

const IMPASSABLE: [u8; 4] = [230, 60, 50, 200];
const COSTLY: [u8; 4] = [240, 170, 40, 140];
const GRID: [u8; 4] = [255, 255, 255, 40];
const PATH: [u8; 4] = [255, 230, 80, 220];
const NARROW: [u8; 4] = [255, 90, 200, 240];
const ANCHOR: [u8; 4] = [255, 255, 255, 230];

/// Cells beyond this count are not drawn (a zoomed-out view would
/// otherwise draw the whole map's grid).
const MAX_CELLS: u32 = 40_000;

fn v2(p: V2) -> Vec2 {
    Vec2::new(p.x.to_f32_render(), p.y.to_f32_render())
}

/// Appends every enabled overlay to `lines`.
pub fn build_debug_lines(
    view: &BattleView,
    flags: DebugFlags,
    camera: &Camera,
    screen: Vec2,
    lines: &mut LineScene,
) {
    if !flags.any() {
        return;
    }
    let map = view.map();
    let proj = |p: Vec2| project(map, camera, screen, p);
    let (min, max) = camera.visible_bounds(screen, 0.0);

    if flags.nav_grid {
        let nav = view.nav_grid();
        let cell = nav.cell().to_f32_render();
        let (x0, y0) = nav.cell_of(V2::from_f32_data(min.x, min.y));
        let (x1, y1) = nav.cell_of(V2::from_f32_data(max.x, max.y));
        if (x1 - x0 + 1) * (y1 - y0 + 1) <= MAX_CELLS {
            for cy in y0..=y1 {
                for cx in x0..=x1 {
                    let cost = nav.cost(cx, cy);
                    let colour = if cost == 0 {
                        IMPASSABLE
                    } else if cost > 100 {
                        COSTLY
                    } else {
                        continue;
                    };
                    let a = Vec2::new(cx as f32 * cell, cy as f32 * cell);
                    let b = a + Vec2::splat(cell);
                    let corners = [a, Vec2::new(b.x, a.y), b, Vec2::new(a.x, b.y)].map(proj);
                    lines.polyline(&corners, colour, true);
                    if cost == 0 {
                        lines.segment(corners[0], corners[2], colour);
                    }
                }
            }
        }
    }

    if flags.spatial_cells {
        let grid = view.spatial_grid();
        let cell = grid.cell().to_f32_render();
        let (x0, y0) = grid.cell_of(V2::from_f32_data(min.x, min.y));
        let (x1, y1) = grid.cell_of(V2::from_f32_data(max.x, max.y));
        if (x1 - x0 + 1) * (y1 - y0 + 1) <= MAX_CELLS {
            for cx in x0..=x1 + 1 {
                let x = cx as f32 * cell;
                lines.segment(
                    proj(Vec2::new(x, y0 as f32 * cell)),
                    proj(Vec2::new(x, (y1 + 1) as f32 * cell)),
                    GRID,
                );
            }
            for cy in y0..=y1 + 1 {
                let y = cy as f32 * cell;
                lines.segment(
                    proj(Vec2::new(x0 as f32 * cell, y)),
                    proj(Vec2::new((x1 + 1) as f32 * cell, y)),
                    GRID,
                );
            }
        }
    }

    for r in view.regiments() {
        let tint = side_tint(r.side);
        let anchor = Anchor {
            pos: r.anchor_pos,
            facing: r.anchor_facing,
        };
        let a = v2(r.anchor_pos);
        let on_screen = a.x >= min.x && a.x <= max.x && a.y >= min.y && a.y <= max.y;
        if flags.anchors && on_screen {
            lines.circle(proj(a), 12.0, 12, ANCHOR);
            let dir = v2(r.anchor_facing.direction());
            lines.segment(proj(a), proj(a + dir * 4.0), ANCHOR);
            lines.circle(proj(a), 6.0, 8, tint);
        }
        if flags.slots
            && on_screen
            && let Some(state) = view.formation_state(r.id)
        {
            let half = camera.zoom * 0.15;
            for slot in &state.slots {
                let s = proj(v2(slot_world(&anchor, slot)));
                lines.segment(s - Vec2::new(half, 0.0), s + Vec2::new(half, 0.0), tint);
                lines.segment(s - Vec2::new(0.0, half), s + Vec2::new(0.0, half), tint);
            }
        }
        if flags.paths
            && let Some(path) = view.path(r.id)
            && path.is_active()
        {
            let mut prev = proj(a);
            for wp in &path.waypoints[usize::from(path.next)..] {
                let p = proj(v2(wp.p));
                lines.segment(prev, p, PATH);
                lines.circle(p, 4.0, 6, PATH);
                prev = p;
            }
            // Corridors narrower than the formation: the morph triggers.
            if let Some(state) = view.formation_state(r.id) {
                let regs = view.regs();
                let template = regs.formations.get(state.template);
                let radius = regs.units.get(r.unit).soldier_radius;
                let width = il_sim_battle::movement::formation_width(template, state.files, radius);
                for wp in &path.waypoints[usize::from(path.next)..] {
                    if wp.corridor < width {
                        lines.circle(proj(v2(wp.p)), 8.0, 4, NARROW);
                    }
                }
            }
        }
    }
}
