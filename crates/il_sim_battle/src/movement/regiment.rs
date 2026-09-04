//! Regiment path following (T1-042; SIM-MOVE-010..013, SIM-MOVE-020
//! regiment part, SIM-MOVE-030, SIM-MOVE-004, SIM-FORM-032, TDD §6.2).
//!
//! Stage 3, after `serve_path_requests`, parallel per regiment: the anchor
//! moves toward the current waypoint at the regiment speed, wheels toward
//! the waypoint at `wheel_rate`, slows while stragglers exceed
//! `straggler_fraction`, morphs to a column for corridors narrower than the
//! formation and back afterwards, and takes the ordered facing on arrival.
//! Every write is to the regiment's own components.

use bevy_ecs::prelude::*;
use il_core::{Angle, S, Scalar, TICKS_PER_SECOND, Tick, V2};
use il_data::{FormationTemplate, Handle, Layout, MovementRules, Registries, UnitType};

use crate::command::SpeedMode;
use crate::components::{
    Anchor, Combat, FormationState, Order, OrderKind, Path, Pos, Regiment, SlotRef,
};
use crate::formation::{slot_world, spacing};
use crate::map::LoadedMap;
use crate::resources::{Clock, Ids, MapRes, Regs};

/// Seconds per tick as a sim scalar (TDD §2.2 `time.rs`).
pub fn tick_dt() -> S {
    S::ONE / S::from_i32(TICKS_PER_SECOND as i32)
}

/// Degrees to radians for rule values expressed in degrees.
pub fn deg_to_rad(deg: S) -> S {
    deg * S::PI / S::from_i32(180)
}

/// SIM-MOVE-030: the speed multiplier of the ground `at` for a move in
/// direction `dir` (unit vector): rise over run one metre ahead, penalised
/// uphill, rewarded downhill, clamped to `[slope_min_mult, slope_max_mult]`.
pub fn slope_mult(map: &LoadedMap, rules: &MovementRules, at: V2, dir: V2) -> S {
    let ahead = at + dir;
    let g = map.height_at(ahead) - map.height_at(at);
    let up = g.max(S::ZERO);
    let down = (-g).max(S::ZERO);
    (S::ONE - rules.slope_penalty * up + rules.slope_bonus * down)
        .clamp(rules.slope_min_mult, rules.slope_max_mult)
}

/// The unit's base speed for a mode (SIM-MOVE-020).
pub fn mode_speed(unit: &UnitType, mode: SpeedMode) -> S {
    match mode {
        SpeedMode::Walk => unit.speed_walk,
        SpeedMode::Run => unit.speed_run,
        SpeedMode::March => unit.speed_march,
    }
}

/// Zone speed multiplier at `p` (`1` off the placeholder map).
pub fn zone_move_mult(map: &LoadedMap, regs: &Registries, p: V2) -> S {
    map.zone_at(p)
        .map_or(S::ONE, |h| regs.zones.get(h).move_mult)
}

/// Width of a regiment in metres: `files × sf` (SIM-MOVE-004).
pub fn formation_width(template: &FormationTemplate, files: u16, radius: S) -> S {
    let (sf, _) = spacing(template, radius);
    let sf = if template.layout == Layout::Loose {
        sf * template.loose_mult
    } else {
        sf
    };
    S::from_i32(i32::from(files)) * sf
}

/// The column template a unit can morph into, if it has one.
fn column_template(unit: &UnitType, regs: &Registries) -> Option<Handle<FormationTemplate>> {
    unit.formations
        .iter()
        .copied()
        .find(|h| regs.formations.get(*h).layout == Layout::Column)
}

type SoldierRead<'w, 's> = Query<'w, 's, (&'static Pos, &'static SlotRef)>;

/// One regiment as `regiment_follow_path` sees it.
type FollowItem<'a> = (
    &'a Regiment,
    &'a Combat,
    Mut<'a, Anchor>,
    Mut<'a, Order>,
    Mut<'a, Path>,
    Mut<'a, FormationState>,
);

/// SIM-MOVE-012: the fraction of soldiers farther than `straggler_radius
/// × sf` from their slot.
fn straggler_fraction(
    regiment: &Regiment,
    anchor: &Anchor,
    state: &FormationState,
    soldiers: &SoldierRead,
    ids: &Ids,
    radius_m: S,
) -> S {
    if regiment.soldiers.is_empty() {
        return S::ZERO;
    }
    let r_sq = radius_m * radius_m;
    let mut stragglers = 0;
    for &sid in &regiment.soldiers {
        let Some(entity) = ids.soldier_entity(sid) else {
            continue;
        };
        let Ok((pos, slot)) = soldiers.get(entity) else {
            continue;
        };
        let Some(slot) = slot.slot.and_then(|s| state.slots.get(usize::from(s))) else {
            stragglers += 1;
            continue;
        };
        if slot_world(anchor, slot).distance_sq(pos.p) > r_sq {
            stragglers += 1;
        }
    }
    S::from_i32(stragglers) / S::from_i32(regiment.soldiers.len() as i32)
}

/// Whether any waypoint from `from` on is narrower than `width`.
fn narrow_ahead(path: &Path, from: usize, width: S) -> bool {
    path.waypoints[from.min(path.waypoints.len())..]
        .iter()
        .any(|w| w.corridor < width)
}

#[allow(clippy::too_many_arguments)]
fn follow_one(
    regiment: &Regiment,
    anchor: &mut Anchor,
    order: &mut Order,
    path: &mut Path,
    state: &mut FormationState,
    soldiers: &SoldierRead,
    ids: &Ids,
    regs: &Registries,
    map: &LoadedMap,
    tick: Tick,
) {
    let rules = &regs.rules.movement;
    let dt = tick_dt();
    let unit = regs.units.get(regiment.unit);
    let radius = unit.soldier_radius;

    // Waypoint bookkeeping: skip every waypoint already within reach.
    while let Some(wp) = path.current() {
        if wp.p.distance(anchor.pos) <= rules.waypoint_radius {
            path.next += 1;
        } else {
            break;
        }
    }
    let Some(wp) = path.current().copied() else {
        // SIM-MOVE-013: arrival. An attack order that still has a target
        // keeps chasing it (`pursue_update` re-paths on its next tick).
        if let Some(facing) = order.facing {
            anchor.facing = facing;
        }
        if !(order.kind.is_attack() && order.target_regiment.is_some()) {
            order.kind = OrderKind::Idle;
        }
        if let Some(prior) = state.prior_template.take() {
            state.template = prior;
        }
        state.needs_reform = true;
        return;
    };

    // SIM-MOVE-004 (Phase 1 plan S11): morph to a column for a corridor the
    // current formation cannot fit, restore the prior template afterwards.
    let template = regs.formations.get(state.template);
    let width = formation_width(template, state.files, radius);
    let next = usize::from(path.next);
    if state.prior_template.is_none() {
        if wp.corridor < width
            && template.layout != Layout::Column
            && let Some(column) = column_template(unit, regs)
        {
            state.prior_template = Some(state.template);
            state.template = column;
            state.needs_reform = true;
        }
    } else if let Some(prior) = state.prior_template {
        let prior_width = formation_width(regs.formations.get(prior), state.files.max(1), radius);
        let prior_files =
            crate::formation::files_for(regiment.soldiers.len() as u16, state.ranks.max(1));
        let prior_width = prior_width.max(formation_width(
            regs.formations.get(prior),
            prior_files,
            radius,
        ));
        if !narrow_ahead(path, next, prior_width) {
            state.template = prior;
            state.prior_template = None;
            state.needs_reform = true;
        }
    }

    // SIM-MOVE-010: wheel toward the waypoint, move at regiment speed.
    let to_wp = wp.p - anchor.pos;
    let dist = to_wp.length();
    let dir = to_wp * (S::ONE / dist);
    let desired = Angle::from_direction(dir);
    let max_turn = deg_to_rad(rules.wheel_rate) * dt;
    anchor.facing = anchor.facing.turn_toward(desired, max_turn);

    // SIM-MOVE-011 / SIM-MOVE-020 (regiment): unit speed × formation ×
    // zone × slope, × morph and straggler factors.
    let template = regs.formations.get(state.template);
    let mut v = mode_speed(unit, order.speed)
        * template.speed_mult
        * zone_move_mult(map, regs, anchor.pos)
        * slope_mult(map, rules, anchor.pos, dir);
    if tick < state.morph_until {
        v = v * regs.rules.formation.morph_speed_mult;
    }
    let (sf, _) = spacing(template, radius);
    let straggler_radius = rules.straggler_radius * sf;
    if straggler_fraction(regiment, anchor, state, soldiers, ids, straggler_radius)
        > rules.straggler_fraction
    {
        v = v * rules.straggler_slowdown;
    }
    let step = (v * dt).min(dist);
    anchor.pos = map.clamp(anchor.pos + dir * step);
}

/// Stage 3 `regiment_follow_path`.
pub fn regiment_follow_path(
    mut regiments: Query<(
        &Regiment,
        &Combat,
        &mut Anchor,
        &mut Order,
        &mut Path,
        &mut FormationState,
    )>,
    soldiers: SoldierRead,
    ids: Res<Ids>,
    regs: Res<Regs>,
    map: Res<MapRes>,
    clock: Res<Clock>,
) {
    let tick = clock.tick;
    let parallel = bevy_tasks::ComputeTaskPool::try_get().is_some_and(|p| p.thread_num() > 1);
    let soldiers = &soldiers;
    let ids = &ids;
    let regs = &regs.0;
    let map = &map.0;
    let wheel_per_tick = deg_to_rad(regs.rules.movement.wheel_rate) * tick_dt();
    let run = |(r, combat, mut anchor, mut order, mut path, mut state): FollowItem<'_>| {
        // SIM-CMBT-003 (plan decision 7): the anchor of an engaged attacker
        // holds while its soldiers fight; the path is kept.
        if order.kind.is_attack() && combat.engaged {
            return;
        }
        if !order.kind.moves() {
            // SIM-FORM-024: a halted regiment wheels toward its ordered
            // facing while its soldiers track the moving slots.
            if let Some(target) = order.facing
                && anchor.facing != target
            {
                anchor.facing = anchor.facing.turn_toward(target, wheel_per_tick);
            }
            return;
        }
        if path.requested || !path.is_active() {
            return;
        }
        follow_one(
            r,
            &mut anchor,
            &mut order,
            &mut path,
            &mut state,
            soldiers,
            ids,
            regs,
            map,
            tick,
        );
    };
    if parallel {
        regiments.par_iter_mut().for_each(run);
    } else {
        regiments.iter_mut().for_each(run);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slope_multiplier_follows_sim_move_030() {
        let mut rules = il_data::Rules::zeroed().movement;
        rules.slope_penalty = S::from_i32(2);
        rules.slope_bonus = S::HALF;
        rules.slope_min_mult = S::from_f32_data(0.4);
        rules.slope_max_mult = S::from_f32_data(1.2);
        // A plane rising 0.1 m per metre along +x on a 2 x 2 sample map.
        let mut map = LoadedMap::flat(S::from_i32(10), S::from_i32(10));
        map.heights = vec![S::ZERO, S::ONE, S::ZERO, S::ONE];
        let at = V2::new(S::from_i32(4), S::from_i32(5));
        let up = slope_mult(&map, &rules, at, V2::new(S::ONE, S::ZERO));
        let down = slope_mult(&map, &rules, at, V2::new(-S::ONE, S::ZERO));
        let flat = slope_mult(&map, &rules, at, V2::new(S::ZERO, S::ONE));
        assert!(
            (up - S::from_f32_data(0.8)).abs() < S::from_f32_data(1e-5),
            "{up:?}"
        );
        assert!(
            (down - S::from_f32_data(1.05)).abs() < S::from_f32_data(1e-5),
            "{down:?}"
        );
        assert_eq!(flat, S::ONE);
        // Clamped on a cliff.
        map.heights = vec![S::ZERO, S::from_i32(100), S::ZERO, S::from_i32(100)];
        assert_eq!(
            slope_mult(&map, &rules, at, V2::new(S::ONE, S::ZERO)),
            rules.slope_min_mult
        );
        assert_eq!(
            slope_mult(&map, &rules, at, V2::new(-S::ONE, S::ZERO)),
            rules.slope_max_mult
        );
        assert_eq!(tick_dt(), S::from_f32_data(0.05));
    }
}
