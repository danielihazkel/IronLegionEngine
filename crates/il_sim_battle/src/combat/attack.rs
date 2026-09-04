//! Stage 10: `melee_attack` and `apply_outcomes` (T2-021; SIM-CMBT-010,
//! SIM-CMBT-011, SIM-CMBT-013..018, TDD §8.1).
//!
//! `melee_attack` runs in parallel over fighting soldiers: each one counts
//! its cooldown down and, at zero and within reach, rolls one attack with
//! `hash_draw(combat_melee, tick, id, 0)` and records an `AttackOutcome`
//! into a shared buffer. `apply_outcomes` then sorts the buffer by
//! attacker id and applies the damage in that order (SAD §8 rule 2),
//! queuing every soldier whose hp crossed zero for Stage 15.

use std::sync::Mutex;

use bevy_ecs::prelude::*;
use il_core::{S, Scalar, SoldierId, StreamId, hash_draw};
use il_data::{Layout, UnitCategory};

use crate::combat::formulas::{
    Arc, arc_mults, attack_arc, aura_attack_mult, braced, charge_mults, cooldown_ticks,
    experience_mult, fatigue_mults, hit_probability, melee_damage, morale_mults, status_mult,
    terrain_defence_mult,
};
use crate::components::{
    Body, Combat, Facing, FatigueC, FormationState, Fsm, Health, MeleeState, Morale, Order, Pos,
    Rank, Regiment, Soldier, SoldierState,
};
use crate::resources::{Clock, Ids, MapRes, Regs, Rng};

/// One attack, hit or miss (TDD §8.1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AttackOutcome {
    pub attacker: SoldierId,
    pub target: SoldierId,
    pub hit: bool,
    pub damage: S,
    pub arc: Arc,
}

/// The attacks of the current tick, in thread order until `apply_outcomes`
/// sorts them (transient: empty at Stage 17, never snapshotted).
#[derive(Resource, Default)]
pub struct Outcomes(pub Mutex<Vec<AttackOutcome>>);

/// Soldiers whose hp crossed zero this tick, with their killer, in
/// application order; drained by Stage 15 (T2-022). Transient.
#[derive(Resource, Debug, Default)]
pub struct Kills(pub Vec<(SoldierId, Option<SoldierId>)>);

type Attacker<'w, 's> = Query<
    'w,
    's,
    (
        &'static Soldier,
        &'static Pos,
        &'static Body,
        &'static Facing,
        &'static Rank,
        &'static Fsm,
        &'static FatigueC,
        &'static mut MeleeState,
    ),
>;
type AttackerItem<'a> = (
    &'a Soldier,
    &'a Pos,
    &'a Body,
    &'a Facing,
    &'a Rank,
    &'a Fsm,
    &'a FatigueC,
    Mut<'a, MeleeState>,
);
type Defender<'w, 's> = Query<
    'w,
    's,
    (
        &'static Soldier,
        &'static Pos,
        &'static Body,
        &'static Facing,
        &'static FatigueC,
    ),
>;
type RegimentRead<'w, 's> = Query<
    'w,
    's,
    (
        &'static Regiment,
        &'static Order,
        &'static FormationState,
        &'static Combat,
        &'static Morale,
    ),
>;

struct Ctx<'a, 'w, 's> {
    ids: &'a Ids,
    regs: &'a il_data::Registries,
    map: &'a crate::map::LoadedMap,
    defenders: &'a Defender<'w, 's>,
    regiments: &'a RegimentRead<'w, 's>,
    tick: il_core::Tick,
    seed: u64,
    out: &'a Mutex<Vec<AttackOutcome>>,
}

impl Ctx<'_, '_, '_> {
    fn attack(
        &self,
        (soldier, pos, body, facing, rank, fsm, fatigue, mut melee): AttackerItem<'_>,
    ) {
        if fsm.state != SoldierState::Fighting {
            return;
        }
        let Some(target) = melee.target else {
            return;
        };
        // SIM-CMBT-010: count down; attack at zero.
        if melee.cooldown > 0 {
            melee.cooldown -= 1;
            return;
        }
        let Some(te) = self.ids.soldier_entity(target) else {
            return;
        };
        let Ok((other, pos_j, body_j, facing_j, fatigue_j)) = self.defenders.get(te) else {
            return;
        };
        let (Some(ri), Some(rj)) = (
            self.ids
                .regiment_entity(soldier.regiment)
                .and_then(|e| self.regiments.get(e).ok()),
            self.ids
                .regiment_entity(other.regiment)
                .and_then(|e| self.regiments.get(e).ok()),
        ) else {
            return;
        };
        let (_, order_i, form_i, combat_i, morale_i) = ri;
        let (_, order_j, form_j, combat_j, morale_j) = rj;
        let rules = &self.regs.rules;
        let c = &rules.combat;
        let unit_i = self.regs.units.get(soldier.unit);
        let unit_j = self.regs.units.get(other.unit);
        let template_i = self.regs.formations.get(form_i.template);
        let template_j = self.regs.formations.get(form_j.template);

        // SIM-CMBT-001 / SIM-CMBT-012: within reach, or wait (cooldown stays 0).
        let second_rank =
            rank.rank == 1 && (unit_i.second_rank_attack || template_i.layout == Layout::Phalanx);
        let reach = if second_rank {
            unit_i.reach + c.second_rank_reach_bonus
        } else {
            unit_i.reach
        };
        let p_i = pos.p;
        let p_j = pos_j.p;
        if p_i.distance(p_j) > body.r + body_j.r + reach {
            return;
        }

        // SIM-CMBT-014 / SIM-CMBT-015.
        let arc = attack_arc(facing_j.theta, p_i - p_j, unit_j.frontal_arc_deg);
        let charging = self.tick.0 < combat_i.charge_until.0;
        let braced_j = braced(
            unit_j.anti_cavalry_bonus,
            form_j.integrity,
            order_j.kind,
            combat_j.engaged,
            arc,
            c,
        );
        let negated = braced_j && soldier.category == UnitCategory::Cavalry;
        let (charge_mult, charge_dmg_mult) =
            charge_mults(unit_i.charge_bonus, charging, negated, c);
        let arc_i = attack_arc(facing.theta, p_j - p_i, unit_i.frontal_arc_deg);
        let anti_cav_i = if other.category == UnitCategory::Cavalry
            && braced(
                unit_i.anti_cavalry_bonus,
                form_i.integrity,
                order_i.kind,
                combat_i.engaged,
                arc_i,
                c,
            ) {
            S::ONE + unit_i.anti_cavalry_bonus
        } else {
            S::ONE
        };

        // SIM-CMBT-011.
        let fm_i = fatigue_mults(fatigue.f, &rules.fatigue);
        let fm_j = fatigue_mults(fatigue_j.f, &rules.fatigue);
        let mm_i = morale_mults(morale_i.state, &rules.morale);
        let mm_j = morale_mults(morale_j.state, &rules.morale);
        let exp = experience_mult(combat_i.experience, c);
        let status = status_mult();
        let a = unit_i.attack
            * fm_i.attack
            * mm_i.attack
            * (S::ONE + template_i.integrity_bonus_attack * form_i.integrity)
            * charge_mult
            * exp
            * status
            * aura_attack_mult()
            * anti_cav_i;
        let (dmg_mult, def_mult) = arc_mults(arc, c);
        let (zone_mult, ford) = self.map.zone_at(p_j).map_or((S::ONE, false), |h| {
            let z = self.regs.zones.get(h);
            (z.defence_mult, z.ford)
        });
        let terrain = terrain_defence_mult(
            zone_mult,
            ford,
            self.map.height_at(p_j),
            self.map.height_at(p_i),
            &rules.movement,
            c,
        );
        let d = unit_j.defence
            * fm_j.defence
            * mm_j.defence
            * (S::ONE + template_j.integrity_bonus_defence * form_j.integrity)
            * def_mult
            * terrain
            * status;
        let p = hit_probability(a, d, c);
        // SIM-DET-002: draw index 0 is the hit roll.
        let hit = hash_draw::<S>(self.seed, self.tick, soldier.id.0, 0) < p;
        let damage = if hit {
            melee_damage(
                unit_i.damage,
                unit_j.armour,
                unit_i.armour_penetration,
                charge_dmg_mult * dmg_mult * exp,
                c,
            )
        } else {
            S::ZERO
        };
        self.out
            .lock()
            .expect("outcome buffer")
            .push(AttackOutcome {
                attacker: soldier.id,
                target,
                hit,
                damage,
                arc,
            });
        melee.cooldown = cooldown_ticks(
            unit_i.attack_interval_ticks,
            fm_i.interval,
            mm_i.interval,
            status,
        );
    }
}

/// Stage 10 `melee_attack` (parallel; writes only its own `MeleeState`).
#[allow(clippy::too_many_arguments)]
pub fn melee_attack(
    mut attackers: Attacker,
    defenders: Defender,
    regiments: RegimentRead,
    ids: Res<Ids>,
    regs: Res<Regs>,
    map: Res<MapRes>,
    clock: Res<Clock>,
    rng: Res<Rng>,
    outcomes: Res<Outcomes>,
) {
    let ctx = Ctx {
        ids: &ids,
        regs: &regs.0,
        map: &map.0,
        defenders: &defenders,
        regiments: &regiments,
        tick: clock.tick,
        seed: rng.draw_seed(StreamId::CombatMelee),
        out: &outcomes.0,
    };
    let ctx = &ctx;
    let run = |item: AttackerItem<'_>| ctx.attack(item);
    let parallel = bevy_tasks::ComputeTaskPool::try_get().is_some_and(|p| p.thread_num() > 1);
    if parallel {
        attackers.par_iter_mut().for_each(run);
    } else {
        attackers.iter_mut().for_each(run);
    }
}

/// Stage 10 `apply_outcomes` (SIM-CMBT-018): hits land in ascending
/// attacker id; a soldier whose hp crosses zero is queued in `Kills` for
/// Stage 15 with its killer. Damage on a soldier already at or below zero
/// is applied and ignored, so the order of hits within a tick cannot
/// change who is credited.
pub fn apply_outcomes(world: &mut World) {
    let mut outcomes = {
        let buffer = world.resource::<Outcomes>();
        core::mem::take(&mut *buffer.0.lock().expect("outcome buffer"))
    };
    if outcomes.is_empty() {
        return;
    }
    outcomes.sort_by_key(|o| o.attacker);
    let mut kills = Vec::new();
    for o in &outcomes {
        if !o.hit {
            continue;
        }
        let Some(e) = world.resource::<Ids>().soldier_entity(o.target) else {
            continue;
        };
        let Some(mut health) = world.get_mut::<Health>(e) else {
            continue;
        };
        let before = health.hp;
        health.hp = before - o.damage;
        if before > S::ZERO && health.hp <= S::ZERO {
            kills.push((o.target, Some(o.attacker)));
        }
    }
    world.resource_mut::<Kills>().0.extend(kills);
}
