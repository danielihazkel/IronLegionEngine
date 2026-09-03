//! Soldier steering (T1-043; SIM-CORE-010..011, SIM-MOVE-020..024,
//! TDD §6.2 `soldier_steer`).
//!
//! Stage 4, parallel over soldiers: each soldier reads its regiment's anchor,
//! formation and order, the previous tick's spatial grid and the nav grid,
//! and writes only its own `Vel`, `Facing` and `Fsm`. Neighbour sums run in
//! a fixed order (nearest first, ties by id), so the result never depends on
//! thread count.

use bevy_ecs::prelude::*;
use il_core::{Angle, S, Scalar, Tick, V2};
use il_data::Registries;

use crate::components::{
    Anchor, Body, Facing, FormationState, Fsm, Order, Pos, SlotRef, Soldier, SoldierState, Vel,
};
use crate::formation::slot_world;
use crate::map::LoadedMap;
use crate::movement::regiment::{mode_speed, slope_mult, tick_dt, zone_move_mult};
use crate::nav::NavGrid;
use crate::resources::{Clock, Ids, MapRes, NavGridRes, Regs, SpatialGridRes};
use crate::spatial::SpatialGrid;

/// SIM-MOVE-023: the rotations tried, in order, when the look-ahead segment
/// crosses an impassable cell (degrees).
const AVOID_DEGREES: [i32; 10] = [15, -15, 30, -30, 45, -45, 60, -60, 90, -90];

type RegimentRead<'w, 's> =
    Query<'w, 's, (&'static Anchor, &'static FormationState, &'static Order)>;

/// One soldier's steering inputs and outputs, so the closure stays small.
struct Steer<'a, 'w, 's> {
    regs: &'a Registries,
    map: &'a LoadedMap,
    nav: &'a NavGrid,
    grid: &'a SpatialGrid<il_core::SoldierId>,
    /// Neighbour radii (grid entries carry the entity).
    bodies: &'a Query<'w, 's, &'static Body>,
    tick: Tick,
    dt: S,
    /// Largest soldier radius in the battle, for the neighbour query.
    max_radius: S,
}

/// SIM-MOVE-021: seek with arrive damping.
pub fn seek_velocity(seek: V2, v_max: S, dt: S, arrive_damping: S) -> V2 {
    let dist = seek.length();
    if dist <= S::ZERO {
        return V2::ZERO;
    }
    let speed = v_max.min(dist / dt * arrive_damping);
    seek * (speed / dist)
}

impl Steer<'_, '_, '_> {
    /// SIM-MOVE-022: separation from the nearest `sep_max_neighbours`
    /// within touching distance plus `sep_margin`, from the previous tick's
    /// grid, summed nearest-first (ties by id).
    fn separation(&self, id: il_core::SoldierId, p: V2, r: S, scratch: &mut Vec<usize>) -> V2 {
        let rules = &self.regs.rules.movement;
        let reach = r + r + self.max_radius + self.max_radius + rules.sep_margin;
        self.grid.query_circle_indices(p, reach, scratch);
        // Nearest first; the query returned ascending ids, and the sort is
        // stable, so equal distances keep id order.
        let entries = self.grid.entries();
        scratch.retain(|&i| entries[i].id != id);
        scratch.sort_by(|&a, &b| {
            entries[a]
                .pos
                .distance_sq(p)
                .partial_cmp(&entries[b].pos.distance_sq(p))
                .expect("finite distances")
        });
        let mut sep = V2::ZERO;
        for &i in scratch.iter().take(usize::from(rules.sep_max_neighbours)) {
            let e = &entries[i];
            let r_j = self.bodies.get(e.entity).map_or(self.max_radius, |b| b.r);
            let touch = r + r + r_j + r_j + rules.sep_margin;
            let d = e.pos.distance(p);
            if d >= touch || d <= S::ZERO {
                continue;
            }
            let away = (p - e.pos) * (S::ONE / d);
            sep += away * (rules.sep_weight * (S::ONE - d / touch));
        }
        sep
    }

    /// SIM-MOVE-023: rotate `v_des` off impassable cells ahead.
    fn avoid(&self, p: V2, v_des: V2) -> V2 {
        if v_des == V2::ZERO {
            return v_des;
        }
        let rules = &self.regs.rules.movement;
        let ahead = self.dt * S::from_i32(i32::from(rules.lookahead_ticks));
        if self.nav.segment_clear(p, p + v_des * ahead) {
            return v_des;
        }
        for deg in AVOID_DEGREES {
            let rotated = v_des.rotate(S::from_i32(deg) * S::PI / S::from_i32(180));
            if self.nav.segment_clear(p, p + rotated * ahead) {
                return rotated;
            }
        }
        V2::ZERO
    }

    #[allow(clippy::too_many_arguments)]
    fn soldier(
        &self,
        soldier: &Soldier,
        pos: &Pos,
        body: &Body,
        slot: &SlotRef,
        regiment: Option<(&Anchor, &FormationState, &Order)>,
        vel: &mut Vel,
        facing: &mut Facing,
        fsm: &mut Fsm,
        scratch: &mut Vec<usize>,
    ) {
        let rules = &self.regs.rules.movement;
        let p = pos.p;
        // The slot to hold, if any.
        let target = regiment.and_then(|(anchor, state, order)| {
            let s = state.slots.get(usize::from(slot.slot?))?;
            Some((
                slot_world(anchor, s),
                Angle::new(anchor.facing.radians() + s.facing_offset.radians()),
                order.speed,
            ))
        });
        let Some((slot_pos, slot_facing, mode)) = target else {
            vel.v = V2::ZERO;
            if fsm.state != SoldierState::Idle {
                fsm.state = SoldierState::Idle;
                fsm.since = self.tick;
            }
            return;
        };

        // SIM-CORE-011: Idle <-> MoveToSlot with hysteresis.
        let seek = slot_pos - p;
        let dist = seek.length();
        let next_state = match fsm.state {
            SoldierState::Idle if dist > rules.slot_leave_radius => SoldierState::MoveToSlot,
            SoldierState::MoveToSlot if dist < rules.slot_arrive_radius => SoldierState::Idle,
            other => other,
        };
        if next_state != fsm.state {
            fsm.state = next_state;
            fsm.since = self.tick;
        }

        // SIM-MOVE-020: v_max (fatigue and status multipliers arrive in Phase 2).
        let unit = self.regs.units.get(soldier.unit);
        let dir = if dist > S::ZERO {
            seek * (S::ONE / dist)
        } else {
            V2::ZERO
        };
        let v_max = mode_speed(unit, mode)
            * zone_move_mult(self.map, self.regs, p)
            * slope_mult(self.map, rules, p, dir);

        let v_des = if fsm.state == SoldierState::MoveToSlot {
            seek_velocity(seek, v_max, self.dt, rules.arrive_damping)
        } else {
            V2::ZERO
        };
        let v_des = self.avoid(p, v_des);
        let sep = self.separation(soldier.id, p, body.r, scratch);
        // SIM-MOVE-024.
        let v = (v_des + sep).clamp_length(v_max);
        vel.v = v;

        let wanted = if dist <= rules.slot_arrive_radius {
            slot_facing
        } else if v.length_sq() > S::ZERO {
            Angle::from_direction(v)
        } else {
            facing.theta
        };
        let max_turn = rules.soldier_turn_rate * S::PI / S::from_i32(180) * self.dt;
        facing.theta = facing.theta.turn_toward(wanted, max_turn);
    }
}

/// Stage 4 `soldier_steer`: writes `Vel`, `Facing` and `Fsm` per soldier.
#[allow(clippy::too_many_arguments)]
pub fn soldier_steer(
    mut soldiers: Query<(
        &Soldier,
        &Pos,
        &Body,
        &SlotRef,
        &mut Vel,
        &mut Facing,
        &mut Fsm,
    )>,
    regiments: RegimentRead,
    bodies: Query<&'static Body>,
    ids: Res<Ids>,
    regs: Res<Regs>,
    map: Res<MapRes>,
    nav: Res<NavGridRes>,
    grid: Res<SpatialGridRes>,
    clock: Res<Clock>,
) {
    let max_radius = regs
        .0
        .units
        .iter()
        .map(|(_, u)| u.soldier_radius)
        .fold(S::ZERO, |a, b| a.max(b));
    let steer = Steer {
        regs: &regs.0,
        map: &map.0,
        nav: &nav.0,
        grid: &grid.0,
        bodies: &bodies,
        tick: clock.tick,
        dt: tick_dt(),
        max_radius,
    };
    let steer = &steer;
    let ids = &ids;
    let regiments = &regiments;
    let run = |scratch: &mut Vec<usize>,
               (soldier, pos, body, slot, mut vel, mut facing, mut fsm): (
        &Soldier,
        &Pos,
        &Body,
        &SlotRef,
        Mut<Vel>,
        Mut<Facing>,
        Mut<Fsm>,
    )| {
        let regiment = ids
            .regiment_entity(soldier.regiment)
            .and_then(|e| regiments.get(e).ok());
        steer.soldier(
            soldier,
            pos,
            body,
            slot,
            regiment,
            &mut vel,
            &mut facing,
            &mut fsm,
            scratch,
        );
    };
    let parallel = bevy_tasks::ComputeTaskPool::try_get().is_some_and(|p| p.thread_num() > 1);
    if parallel {
        soldiers.par_iter_mut().for_each_init(Vec::new, run);
    } else {
        let mut scratch = Vec::new();
        for item in soldiers.iter_mut() {
            run(&mut scratch, item);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrive_damping_caps_the_step_at_the_slot_edge() {
        let dt = S::from_f32_data(0.05);
        let damping = S::HALF;
        let v_max = S::from_i32(4);
        // Far away: full speed toward the slot.
        let v = seek_velocity(V2::new(S::from_i32(10), S::ZERO), v_max, dt, damping);
        assert_eq!(v, V2::new(v_max, S::ZERO));
        // 0.1 m away: at most half the remaining distance per tick.
        let v = seek_velocity(V2::new(S::from_f32_data(0.1), S::ZERO), v_max, dt, damping);
        assert!((v.x - S::ONE).abs() < S::from_f32_data(1e-5), "{v:?}");
        assert_eq!((v * dt).x, S::from_f32_data(0.05));
        assert_eq!(seek_velocity(V2::ZERO, v_max, dt, damping), V2::ZERO);
    }
}
