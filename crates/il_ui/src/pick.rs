//! Hit testing (T1-061, REQ-INP-002): which of the local player's regiments
//! is under the cursor or inside a box, decided on `BattleView` soldier
//! positions projected through a closure the app builds from its camera.
//! Only the local player's regiments are ever returned (TDD §11 "own
//! faction only"); there is no fog in Phase 1, so "visible" means on screen.

use std::collections::BTreeSet;

use glam::Vec2;
use il_core::{PlayerId, RegimentId, Scalar, V2};
use il_data::{Handle, UnitType};
use il_sim_battle::BattleView;

/// World point (metres, at ground height) to screen pixels.
pub type Project<'a> = dyn Fn(V2) -> Vec2 + 'a;

/// A soldier is never harder to hit than a circle of this radius.
pub const MIN_HIT_RADIUS_PX: f32 = 6.0;
/// Hit radius as a multiple of the soldier's drawn radius.
pub const HIT_RADIUS_SCALE: f32 = 1.5;

/// One regiment the picker may consider.
#[derive(Clone, Copy, Debug)]
struct Candidate {
    id: RegimentId,
    unit: Handle<UnitType>,
}

/// The local player's regiments, ascending by id.
fn candidates(view: &BattleView, player: PlayerId) -> Vec<Candidate> {
    let sides = view.sides();
    view.regiments()
        .filter(|r| {
            sides
                .get(usize::from(r.side))
                .is_some_and(|s| s.player == player)
        })
        .map(|r| Candidate {
            id: r.id,
            unit: r.unit,
        })
        .collect()
}

fn candidate(cands: &[Candidate], id: RegimentId) -> Option<Candidate> {
    cands
        .binary_search_by_key(&id, |c| c.id)
        .ok()
        .map(|i| cands[i])
}

/// Whether `id` belongs to `player`.
pub fn owned(view: &BattleView, id: RegimentId, player: PlayerId) -> bool {
    view.regiment(id).is_some_and(|r| {
        view.sides()
            .get(usize::from(r.side))
            .is_some_and(|s| s.player == player)
    })
}

/// Every regiment `player` commands.
pub fn own_regiments(view: &BattleView, player: PlayerId) -> BTreeSet<RegimentId> {
    candidates(view, player).iter().map(|c| c.id).collect()
}

/// Screen-space hit circle of a soldier drawn at ground point `p` with
/// `radius_m`: centred half a body up (sprites stand on their ground point).
fn hit_circle(p: Vec2, radius_m: f32, pixels_per_metre: f32) -> (Vec2, f32) {
    let r_px = radius_m * pixels_per_metre;
    (
        Vec2::new(p.x, p.y - r_px),
        (r_px * HIT_RADIUS_SCALE).max(MIN_HIT_RADIUS_PX),
    )
}

/// The own regiment whose soldier is nearest to `cursor` within its hit
/// circle.
pub fn pick_regiment(
    view: &BattleView,
    project: &Project<'_>,
    pixels_per_metre: f32,
    player: PlayerId,
    cursor: Vec2,
) -> Option<RegimentId> {
    let cands = candidates(view, player);
    if cands.is_empty() {
        return None;
    }
    let units = &view.regs().units;
    let mut best: Option<(f32, RegimentId)> = None;
    for s in view.soldiers_unordered() {
        let Some(c) = candidate(&cands, s.regiment) else {
            continue;
        };
        let radius_m = units.get(c.unit).soldier_radius.to_f32_render();
        let (centre, r) = hit_circle(project(s.pos), radius_m, pixels_per_metre);
        let d = (cursor - centre).length();
        if d <= r && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, c.id));
        }
    }
    best.map(|(_, id)| id)
}

/// Own regiments with at least one soldier whose ground point projects
/// inside the rectangle spanned by `a` and `b` (any corner order).
pub fn regiments_in_box(
    view: &BattleView,
    project: &Project<'_>,
    player: PlayerId,
    a: Vec2,
    b: Vec2,
) -> BTreeSet<RegimentId> {
    let (min, max) = (a.min(b), a.max(b));
    let cands = candidates(view, player);
    let mut out = BTreeSet::new();
    if cands.is_empty() {
        return out;
    }
    for s in view.soldiers_unordered() {
        if out.contains(&s.regiment) || candidate(&cands, s.regiment).is_none() {
            continue;
        }
        let p = project(s.pos);
        if p.x >= min.x && p.x <= max.x && p.y >= min.y && p.y <= max.y {
            out.insert(s.regiment);
        }
    }
    out
}

/// Own regiments of the same unit type as `like` with any soldier on a
/// `screen`-sized viewport (double-click by type).
pub fn regiments_of_type_on_screen(
    view: &BattleView,
    project: &Project<'_>,
    player: PlayerId,
    like: RegimentId,
    screen: Vec2,
) -> BTreeSet<RegimentId> {
    let cands = candidates(view, player);
    let Some(unit) = candidate(&cands, like).map(|c| c.unit) else {
        return BTreeSet::new();
    };
    let same: Vec<Candidate> = cands.into_iter().filter(|c| c.unit == unit).collect();
    let mut out = BTreeSet::new();
    for s in view.soldiers_unordered() {
        if out.contains(&s.regiment) || candidate(&same, s.regiment).is_none() {
            continue;
        }
        let p = project(s.pos);
        if p.x >= 0.0 && p.x <= screen.x && p.y >= 0.0 && p.y <= screen.y {
            out.insert(s.regiment);
        }
    }
    out
}
