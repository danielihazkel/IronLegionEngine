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

use crate::combat::{fatigue_mults, morale_mults};
use crate::command::SpeedMode;
use crate::components::Statuses;
use crate::components::{
    Anchor, Body, Facing, FatigueC, FormationState, Fsm, MeleeState, Morale, MoraleState, Order,
    Pos, Rank, Regiment, SlotRef, Soldier, SoldierState, Vel,
};
use crate::formation::slot_world;
use crate::map::LoadedMap;
use crate::movement::regiment::{mode_speed, slope_mult, tick_dt, zone_move_mult};
use crate::nav::NavGrid;
use crate::resources::{Clock, FlowFields, Ids, MapRes, NavGridRes, Regs, SpatialGridRes};
use crate::spatial::SpatialGrid;
use il_data::Layout;

/// SIM-MOVE-023: the rotations tried, in order, when the look-ahead segment
/// crosses an impassable cell (degrees).
const AVOID_DEGREES: [i32; 10] = [15, -15, 30, -30, 45, -45, 60, -60, 90, -90];

type RegimentRead<'w, 's> = Query<
    'w,
    's,
    (
        &'static Regiment,
        &'static Anchor,
        &'static FormationState,
        &'static Order,
        &'static Morale,
        &'static Statuses,
    ),
>;
/// One regiment as a soldier reads it.
type RegimentItem<'a> = (
    &'a Regiment,
    &'a Anchor,
    &'a FormationState,
    &'a Order,
    &'a Morale,
    &'a Statuses,
);

/// The per-soldier query of `soldier_steer`.
type SteerQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static Soldier,
        &'static Pos,
        &'static Body,
        &'static SlotRef,
        &'static Rank,
        &'static MeleeState,
        &'static FatigueC,
        &'static mut Vel,
        &'static mut Facing,
        &'static mut Fsm,
    ),
>;
/// One item of `SteerQuery`.
type SteerItem<'a> = (
    &'a Soldier,
    &'a Pos,
    &'a Body,
    &'a SlotRef,
    &'a Rank,
    &'a MeleeState,
    &'a FatigueC,
    Mut<'a, Vel>,
    Mut<'a, Facing>,
    Mut<'a, Fsm>,
);

/// One soldier's steering inputs and outputs, so the closure stays small.
struct Steer<'a, 'w, 's> {
    regs: &'a Registries,
    map: &'a LoadedMap,
    nav: &'a NavGrid,
    grid: &'a SpatialGrid<il_core::SoldierId>,
    /// Neighbour radii (grid entries carry the entity).
    bodies: &'a Query<'w, 's, &'static Body>,
    /// Target regiments (SIM-MOR-034: cavalry chase routers at run).
    soldiers: &'a Query<'w, 's, &'static Soldier>,
    regiments: &'a RegimentRead<'w, 's>,
    ids: &'a Ids,
    /// Escape fields per side (SIM-FLOW-002).
    flow: &'a FlowFields,
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

    /// T2-020 (SIM-CORE-011, plan decision 6): a fighting soldier seeks its
    /// target's previous-tick position and stops at `r_i + r_j + reach`;
    /// separation and avoidance still apply and the facing tracks the
    /// target. Slot seeking is suspended. Without a target (cleared by a
    /// death) it holds still until its next retarget tick.
    #[allow(clippy::too_many_arguments)]
    fn fight(
        &self,
        soldier: &Soldier,
        p: V2,
        body: &Body,
        rank: &Rank,
        melee: &MeleeState,
        speed_mult: S,
        regiment: Option<RegimentItem<'_>>,
        vel: &mut Vel,
        facing: &mut Facing,
        scratch: &mut Vec<usize>,
    ) {
        let rules = &self.regs.rules.movement;
        let combat = &self.regs.rules.combat;
        let unit = self.regs.units.get(soldier.unit);
        let mut mode = regiment.map_or(SpeedMode::Walk, |(_, _, _, o, _, _)| o.speed);
        // SIM-CMBT-012: second-rank fighters stop a reach bonus further back.
        let second_rank = rank.rank == 1
            && regiment.is_some_and(|(_, _, state, _, _, _)| {
                unit.second_rank_attack
                    || self.regs.formations.get(state.template).layout == Layout::Phalanx
            });
        let reach = if second_rank {
            unit.reach + combat.second_rank_reach_bonus
        } else {
            unit.reach
        };
        let target = melee.target.and_then(|t| {
            let entries = self.grid.entries();
            entries
                .binary_search_by_key(&t, |e| e.id)
                .ok()
                .map(|i| &entries[i])
        });
        // SIM-MOR-034: cavalry chasing a routing target runs.
        if soldier.category == il_data::UnitCategory::Cavalry
            && let Some(e) = target
            && self
                .soldiers
                .get(e.entity)
                .ok()
                .and_then(|s| self.ids.regiment_entity(s.regiment))
                .and_then(|re| self.regiments.get(re).ok())
                .is_some_and(|(_, _, _, _, m, _)| {
                    matches!(m.state, MoraleState::Routing | MoraleState::Shattered)
                })
        {
            mode = SpeedMode::Run;
        }
        let mut wanted = None;
        let mut v_max = mode_speed(unit, mode) * speed_mult;
        let v_des = match target {
            Some(e) if e.pos != p => {
                let r_j = self.bodies.get(e.entity).map_or(self.max_radius, |b| b.r);
                let to = e.pos - p;
                let dist = to.length();
                let dir = to * (S::ONE / dist);
                v_max = v_max
                    * zone_move_mult(self.map, self.regs, p)
                    * slope_mult(self.map, rules, p, dir);
                wanted = Some(Angle::from_direction(dir));
                let stop = body.r + r_j + reach;
                seek_velocity(
                    dir * (dist - stop).max(S::ZERO),
                    v_max,
                    self.dt,
                    rules.arrive_damping,
                )
            }
            _ => V2::ZERO,
        };
        let v_des = self.avoid(p, v_des);
        let sep = self.separation(soldier.id, p, body.r, scratch);
        vel.v = (v_des + sep).clamp_length(v_max);
        if let Some(w) = wanted {
            let max_turn = rules.soldier_turn_rate * S::PI / S::from_i32(180) * self.dt;
            facing.theta = facing.theta.turn_toward(w, max_turn);
        }
    }

    /// SIM-FLOW-002 (T2-042): `v_des = field(p) × v_max` with separation
    /// and avoidance as usual; no arrive damping; the facing tracks the
    /// velocity. Without a field (no side, an unreachable pocket) only
    /// separation acts.
    #[allow(clippy::too_many_arguments)]
    fn flee(
        &self,
        soldier: &Soldier,
        p: V2,
        body: &Body,
        speed_mult: S,
        side: Option<u8>,
        mode: SpeedMode,
        vel: &mut Vel,
        facing: &mut Facing,
        scratch: &mut Vec<usize>,
    ) {
        let rules = &self.regs.rules.movement;
        let unit = self.regs.units.get(soldier.unit);
        let dir = side
            .and_then(|s| self.flow.for_side(s))
            .map_or(V2::ZERO, |f| f.direction_at(self.nav, p));
        let v_max = mode_speed(unit, mode)
            * speed_mult
            * zone_move_mult(self.map, self.regs, p)
            * slope_mult(self.map, rules, p, dir);
        let v_des = self.avoid(p, dir * v_max);
        let sep = self.separation(soldier.id, p, body.r, scratch);
        let v = (v_des + sep).clamp_length(v_max);
        vel.v = v;
        if v.length_sq() > S::ZERO {
            let max_turn = rules.soldier_turn_rate * S::PI / S::from_i32(180) * self.dt;
            facing.theta = facing.theta.turn_toward(Angle::from_direction(v), max_turn);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn soldier(
        &self,
        soldier: &Soldier,
        pos: &Pos,
        body: &Body,
        slot: &SlotRef,
        rank: &Rank,
        melee: &MeleeState,
        fatigue: &FatigueC,
        regiment: Option<RegimentItem<'_>>,
        vel: &mut Vel,
        facing: &mut Facing,
        fsm: &mut Fsm,
        scratch: &mut Vec<usize>,
    ) {
        let rules = &self.regs.rules.movement;
        let p = pos.p;
        // SIM-MOVE-020 (T2-040/T2-050): own fatigue, the regiment's morale
        // state and its status effects scale every branch's `v_max`.
        let speed_mult = fatigue_mults(fatigue.f, &self.regs.rules.fatigue).speed
            * regiment.map_or(S::ONE, |(_, _, _, _, m, st)| {
                morale_mults(m.state, &self.regs.rules.morale).speed * st.mults.speed
            });
        // SIM-FLOW-002 / SIM-MOR-030 (T2-042): routing and withdrawing
        // soldiers follow the escape field; this comes before every other
        // branch so a rout is never cancelled by a missing slot.
        if matches!(fsm.state, SoldierState::Routing | SoldierState::Withdrawing) {
            let mode = if fsm.state == SoldierState::Routing {
                SpeedMode::Run
            } else {
                SpeedMode::March
            };
            let side = regiment.map(|(r, _, _, _, _, _)| r.side);
            self.flee(
                soldier, p, body, speed_mult, side, mode, vel, facing, scratch,
            );
            return;
        }
        if fsm.state == SoldierState::Fighting {
            self.fight(
                soldier, p, body, rank, melee, speed_mult, regiment, vel, facing, scratch,
            );
            return;
        }
        // The slot to hold, if any.
        let target = regiment.and_then(|(_, anchor, state, order, _, _)| {
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

        // SIM-MOVE-020: v_max (the status multiplier arrives with T2-050).
        let unit = self.regs.units.get(soldier.unit);
        let dir = if dist > S::ZERO {
            seek * (S::ONE / dist)
        } else {
            V2::ZERO
        };
        let v_max = mode_speed(unit, mode)
            * speed_mult
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
    mut soldiers: SteerQuery,
    regiments: RegimentRead,
    bodies: Query<&'static Body>,
    soldier_regiments: Query<&'static Soldier>,
    ids: Res<Ids>,
    regs: Res<Regs>,
    map: Res<MapRes>,
    nav: Res<NavGridRes>,
    grid: Res<SpatialGridRes>,
    flow: Res<FlowFields>,
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
        soldiers: &soldier_regiments,
        regiments: &regiments,
        ids: &ids,
        flow: &flow,
        tick: clock.tick,
        dt: tick_dt(),
        max_radius,
    };
    let steer = &steer;
    let ids = &ids;
    let regiments = &regiments;
    let run = |scratch: &mut Vec<usize>,
               (soldier, pos, body, slot, rank, melee, fatigue, mut vel, mut facing, mut fsm): SteerItem<
        '_,
    >| {
        let regiment = ids
            .regiment_entity(soldier.regiment)
            .and_then(|e| regiments.get(e).ok());
        steer.soldier(
            soldier,
            pos,
            body,
            slot,
            rank,
            melee,
            fatigue,
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
