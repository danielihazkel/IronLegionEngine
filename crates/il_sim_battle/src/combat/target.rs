//! Stage 9: `melee_target` and `melee_recount` (T2-020; SIM-CMBT-001..003,
//! SIM-CMBT-010 stagger, SIM-CMBT-012, SIM-CORE-011, TDD §8.1).
//!
//! `melee_target` runs in parallel over soldiers and writes only the
//! soldier's own `Fsm` and `MeleeState`; it reads this tick's spatial grid
//! (rebuilt at the end of Stage 7) and the previous tick's attacker
//! counts. `melee_recount` then derives `Attackers` and each regiment's
//! `engaged` flag in ascending id order (SAD §8 rule 2).

use bevy_ecs::prelude::*;
use il_core::{S, Scalar, SoldierId, Tick};
use il_data::Layout;

use crate::command::SpeedMode;
use crate::components::{
    Anchor, Attackers, Body, Combat, Fsm, MeleeState, Order, Pos, Rank, Regiment, Soldier,
    SoldierState,
};
use crate::events::BattleEvent;
use crate::formation::slot_world;
use crate::resources::{Clock, Events, Ids, MeleeGateRes, Regs, SpatialGridRes};

type OtherSoldiers<'w, 's> = Query<'w, 's, (&'static Soldier, &'static Body)>;
type RegimentRead<'w, 's> =
    Query<'w, 's, (&'static Anchor, &'static crate::components::FormationState)>;
type AttackerRead<'w, 's> = Query<'w, 's, &'static Attackers>;

/// Stage 9 `melee_target` (SIM-CMBT-002): on its stagger tick a soldier
/// keeps a target still within `reach + reach_slack`, else takes the
/// enemy within `engage_radius` with the fewest attackers, nearest, lowest
/// id; entering `Fighting` staggers the first attack by `id %
/// attack_interval_ticks` (SIM-CMBT-010). Soldiers of regiments the gate
/// excluded, or without a target, fall back to `MoveToSlot`.
#[allow(clippy::too_many_arguments)]
pub fn melee_target(
    mut soldiers: Query<(&Soldier, &Pos, &Body, &Rank, &mut Fsm, &mut MeleeState)>,
    others: OtherSoldiers,
    attackers: AttackerRead,
    regiments: RegimentRead,
    ids: Res<Ids>,
    regs: Res<Regs>,
    grid: Res<SpatialGridRes>,
    gate: Res<MeleeGateRes>,
    clock: Res<Clock>,
) {
    let tick = clock.tick;
    let rules = &regs.0.rules.combat;
    let period = u32::from(rules.retarget_period_ticks.max(1));
    let max_radius = regs
        .0
        .units
        .iter()
        .map(|(_, u)| u.soldier_radius)
        .fold(S::ZERO, |a, b| a.max(b));
    let ctx = Ctx {
        ids: &ids,
        regs: &regs.0,
        grid: &grid.0,
        gate: &gate,
        others: &others,
        attackers: &attackers,
        regiments: &regiments,
        tick,
        period,
        max_radius,
    };
    let ctx = &ctx;
    let run = |scratch: &mut Vec<usize>,
               (soldier, pos, body, rank, mut fsm, mut melee): (
        &Soldier,
        &Pos,
        &Body,
        &Rank,
        Mut<Fsm>,
        Mut<MeleeState>,
    )| {
        ctx.soldier(soldier, pos, body, rank, &mut fsm, &mut melee, scratch);
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

struct Ctx<'a, 'w, 's> {
    ids: &'a Ids,
    regs: &'a il_data::Registries,
    grid: &'a crate::spatial::SpatialGrid<SoldierId>,
    gate: &'a MeleeGateRes,
    others: &'a OtherSoldiers<'w, 's>,
    attackers: &'a AttackerRead<'w, 's>,
    regiments: &'a RegimentRead<'w, 's>,
    tick: Tick,
    period: u32,
    max_radius: S,
}

impl Ctx<'_, '_, '_> {
    /// The grid entry of a living soldier, if any.
    fn entry(&self, id: SoldierId) -> Option<&crate::spatial::Entry<SoldierId>> {
        let entries = self.grid.entries();
        entries
            .binary_search_by_key(&id, |e| e.id)
            .ok()
            .map(|i| &entries[i])
    }

    #[allow(clippy::too_many_arguments)]
    fn soldier(
        &self,
        soldier: &Soldier,
        pos: &Pos,
        body: &Body,
        rank: &Rank,
        fsm: &mut Fsm,
        melee: &mut MeleeState,
        scratch: &mut Vec<usize>,
    ) {
        let rules = &self.regs.rules.combat;
        let leave = |fsm: &mut Fsm, melee: &mut MeleeState| {
            melee.target = None;
            if fsm.state == SoldierState::Fighting {
                fsm.state = SoldierState::MoveToSlot;
                fsm.since = self.tick;
            }
        };
        // Only Idle, MoveToSlot and Fighting soldiers fight (SIM-CORE-011).
        if !matches!(
            fsm.state,
            SoldierState::Idle | SoldierState::MoveToSlot | SoldierState::Fighting
        ) {
            melee.target = None;
            return;
        }
        let Some(ri) = self.ids.regiment_index(soldier.regiment) else {
            leave(fsm, melee);
            return;
        };
        if !self.gate.may_fight[ri] || !self.gate.near_enemy[ri] {
            leave(fsm, melee);
            return;
        }
        // SIM-CMBT-002: staggered by id.
        if self.tick.0 % self.period != soldier.id.0 % self.period {
            return;
        }
        let my_side = self.gate.side[ri];
        let unit = self.regs.units.get(soldier.unit);
        let p = pos.p;

        // SIM-CMBT-012: second-rank attackers reach past the soldier ahead.
        let regiment = self
            .ids
            .regiment_entity(soldier.regiment)
            .and_then(|e| self.regiments.get(e).ok());
        let second_rank = rank.rank == 1
            && regiment.is_some_and(|(_, state)| {
                unit.second_rank_attack
                    || self.regs.formations.get(state.template).layout == Layout::Phalanx
            });
        let reach = if second_rank {
            unit.reach + rules.second_rank_reach_bonus
        } else {
            unit.reach
        };
        let ahead: Option<il_core::V2> = if second_rank {
            regiment.and_then(|(anchor, state)| {
                state
                    .slots
                    .iter()
                    .find(|s| s.rank == 0 && s.file == rank.file)
                    .map(|s| slot_world(anchor, s))
            })
        } else {
            None
        };

        // Keep the current target while it lives and stays within slack.
        if let Some(t) = melee.target
            && let Some(e) = self.entry(t)
            && let Ok((_, body_j)) = self.others.get(e.entity)
            && e.pos.distance(p) <= body.r + body_j.r + reach + rules.reach_slack
        {
            return;
        }

        // Search: fewest attackers, then nearest, then lowest id (the grid
        // hands entries back in ascending id, so a strict `<` keeps it).
        let radius = rules.engage_radius.max(body.r + self.max_radius + reach);
        self.grid.query_circle_indices(p, radius, scratch);
        let entries = self.grid.entries();
        let mut best: Option<(u8, S, SoldierId)> = None;
        for &k in scratch.iter() {
            let e = &entries[k];
            if e.id == soldier.id {
                continue;
            }
            let Ok((other, body_j)) = self.others.get(e.entity) else {
                continue;
            };
            let Some(rj) = self.ids.regiment_index(other.regiment) else {
                continue;
            };
            if self.gate.side[rj] == my_side {
                continue;
            }
            let d_sq = e.pos.distance_sq(p);
            let in_reach = match ahead {
                // A second-rank soldier targets what the soldier ahead of it
                // could reach, if it is also within its own extended reach.
                Some(a) => {
                    let front = body.r + body_j.r + unit.reach;
                    e.pos.distance_sq(a) <= front * front
                        && d_sq <= (body.r + body_j.r + reach) * (body.r + body_j.r + reach)
                }
                None => d_sq <= rules.engage_radius * rules.engage_radius,
            };
            if !in_reach {
                continue;
            }
            let n = self.attackers.get(e.entity).map_or(0, |a| a.n);
            let key = (n, d_sq, e.id);
            if best.is_none_or(|b| (key.0, key.1) < (b.0, b.1)) {
                best = Some(key);
            }
        }

        match best {
            Some((_, _, id)) => {
                melee.target = Some(id);
                if fsm.state != SoldierState::Fighting {
                    fsm.state = SoldierState::Fighting;
                    fsm.since = self.tick;
                    let interval = u32::from(unit.attack_interval_ticks.max(1));
                    melee.cooldown = (soldier.id.0 % interval) as u16;
                }
            }
            None => leave(fsm, melee),
        }
    }
}

/// Recomputes `Attackers` from the targets and each regiment's `engaged`
/// flag from its soldiers, both in ascending id order. `emit` pushes
/// `Engaged` events on the false-to-true edge (Stage 9); restore passes
/// `false`.
fn recount(world: &mut World, emit: bool) {
    let tick = world.resource::<Clock>().tick;
    let soldier_entities: Vec<(SoldierId, Entity)> =
        world.resource::<Ids>().soldier_entities.clone();
    for (_, e) in &soldier_entities {
        if let Some(mut a) = world.get_mut::<Attackers>(*e) {
            a.n = 0;
        }
    }
    for (_, e) in &soldier_entities {
        let Some(target) = world.get::<MeleeState>(*e).and_then(|m| m.target) else {
            continue;
        };
        if let Some(te) = world.resource::<Ids>().soldier_entity(target)
            && let Some(mut a) = world.get_mut::<Attackers>(te)
        {
            a.n = a.n.saturating_add(1);
        }
    }

    let regiment_entities: Vec<Entity> = world
        .resource::<Ids>()
        .regiment_entities
        .iter()
        .map(|(_, e)| *e)
        .collect();
    let (window, mass_mult) = {
        let c = &world.resource::<Regs>().0.rules.combat;
        (u32::from(c.charge_window_ticks), c.charge_mass_mult)
    };
    for entity in regiment_entities {
        // The first fighter's target names the charged regiment.
        let (rid, engaged, struck, running, unit, soldiers) = {
            let Some(regiment) = world.get::<Regiment>(entity) else {
                continue;
            };
            let ids = world.resource::<Ids>();
            let mut struck = None;
            let engaged = regiment.soldiers.iter().any(|&sid| {
                let Some(e) = ids.soldier_entity(sid) else {
                    return false;
                };
                let fighting = world
                    .get::<Fsm>(e)
                    .is_some_and(|f| f.state == SoldierState::Fighting);
                if fighting && struck.is_none() {
                    struck = world
                        .get::<MeleeState>(e)
                        .and_then(|m| m.target)
                        .and_then(|t| ids.soldier_entity(t))
                        .and_then(|te| world.get::<Soldier>(te))
                        .map(|s| s.regiment);
                }
                fighting
            });
            let running = world
                .get::<Order>(entity)
                .is_some_and(|o| o.speed == SpeedMode::Run);
            (
                regiment.id,
                engaged,
                struck,
                running,
                regiment.unit,
                regiment.soldiers.clone(),
            )
        };
        let Some(mut combat) = world.get_mut::<Combat>(entity) else {
            continue;
        };
        let was = combat.engaged;
        combat.engaged = engaged;
        if engaged {
            combat.last_fighting = tick;
        }
        // SIM-CMBT-015: the window closes on its last tick, and a running
        // regiment that just made contact opens a new one.
        let mut mass = None;
        if combat.charge_until == tick {
            mass = Some(S::ONE);
        }
        let charge = engaged && !was && running && tick.0 >= combat.charge_until.0 && window > 0;
        if charge {
            combat.charge_until = Tick(tick.0 + window);
            mass = Some(mass_mult);
        }
        if let Some(mult) = mass {
            let base = world.resource::<Regs>().0.units.get(unit).mass;
            set_mass(world, &soldiers, base * mult);
        }
        if !emit {
            continue;
        }
        if engaged && !was {
            world
                .resource_mut::<Events>()
                .0
                .push(tick, BattleEvent::Engaged { regiment: rid });
        }
        if charge && let Some(target) = struck {
            world.resource_mut::<Events>().0.push(
                tick,
                BattleEvent::Charge {
                    regiment: rid,
                    target,
                },
            );
        }
    }
}

/// `Body.m` of every listed soldier (SIM-CMBT-015 charge push).
fn set_mass(world: &mut World, soldiers: &[SoldierId], m: S) {
    for &sid in soldiers {
        if let Some(e) = world.resource::<Ids>().soldier_entity(sid)
            && let Some(mut body) = world.get_mut::<Body>(e)
        {
            body.m = m;
        }
    }
}

/// Restore: `Body.m` is derived, so regiments inside a charge window get
/// their multiplied mass back (TDD §4.6).
pub fn rebuild_charge_mass(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    let mult = world.resource::<Regs>().0.rules.combat.charge_mass_mult;
    let regiment_entities: Vec<Entity> = world
        .resource::<Ids>()
        .regiment_entities
        .iter()
        .map(|(_, e)| *e)
        .collect();
    for entity in regiment_entities {
        let charging = world
            .get::<Combat>(entity)
            .is_some_and(|c| c.charge_until.0 > tick.0);
        if !charging {
            continue;
        }
        let (unit, soldiers) = {
            let r = world.get::<Regiment>(entity).expect("regiment");
            (r.unit, r.soldiers.clone())
        };
        let base = world.resource::<Regs>().0.units.get(unit).mass;
        set_mass(world, &soldiers, base * mult);
    }
}

/// Stage 9 `melee_recount` (SIM-CMBT-003).
pub fn melee_recount(world: &mut World) {
    recount(world, true);
}

/// Restore: `Attackers` and `engaged` are derived (TDD §4.6).
pub fn rebuild_attackers(world: &mut World) {
    recount(world, false);
}
