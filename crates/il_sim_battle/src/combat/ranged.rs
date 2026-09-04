//! Ranged combat (T2-030, T2-031; SIM-PROJ-001..006, SIM-PROJ-009, TDD
//! §8.2).
//!
//! Stage 9 `ranged_target` (exclusive, ascending regiment id) picks each
//! shooting regiment's enemy regiment on its stagger tick and fills the
//! per-tick [`RangedGateRes`]. Stage 10 `ranged_fire` runs in parallel over
//! the soldiers that carry a [`RangedState`], writes only that component,
//! and records every shot into a shared buffer; `ranged_spawn` (exclusive)
//! sorts the buffer by shooter id, allocates projectile ids in that order,
//! spends ammo, resets the cooldowns and emits the events (SAD §8 rule 2).
//! Stage 11 `projectile_stage` (exclusive) lands the projectiles whose
//! tick has come, applies the damage queue in `(tick, target)` order and
//! drops the landed projectiles from the list.

use std::collections::BTreeMap;
use std::sync::Mutex;

use bevy_ecs::prelude::*;
use il_core::{
    RegimentId, S, Scalar, SoldierId, StreamId, TICKS_PER_SECOND, Tick, V2, hash_draw,
    hash_draw_bits,
};
use il_data::ProjectileArc;

use crate::combat::attack::{Kill, Kills};
use crate::combat::formulas::{
    apex_height, attack_arc, cooldown_ticks, fatigue_mults, flight_ticks, range_mult,
    ranged_damage, scatter,
};
use crate::command::FireMode;
use crate::components::{
    Anchor, Body, Facing, FatigueC, Fire, Fsm, Health, Morale, MoraleState, Pos, RangedState,
    Regiment, Soldier, SoldierState, Vel,
};
use crate::events::BattleEvent;
use crate::resources::{
    AnchorGridRes, Clock, Events, Ids, MapRes, MeleeGateRes, Pending, PendingDamage, Projectile,
    Projectiles, RangedGateRes, Regs, Rng, SpatialGridRes,
};
use crate::spatial::Entry;

/// One would-be projectile of the current tick, or a refused direct shot
/// (`blocked` names the friendly regiment in the line of fire).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shot {
    pub shooter: SoldierId,
    pub regiment: RegimentId,
    pub side: u8,
    pub target_regiment: RegimentId,
    pub start: V2,
    pub end: V2,
    pub land_tick: Tick,
    pub apex: S,
    pub arc: ProjectileArc,
    pub damage: S,
    pub pen: S,
    /// The shooter's fatigue interval multiplier (SIM-FAT-004), for the
    /// volley cooldown reduction of SIM-PROJ-003.
    pub fatigue_interval: S,
    pub blocked: Option<RegimentId>,
}

/// The shots of the current tick, in thread order until `ranged_spawn`
/// sorts them (transient: empty at Stage 17, never snapshotted).
#[derive(Resource, Default)]
pub struct Shots(pub Mutex<Vec<Shot>>);

/// SIM-VIS-004 placeholder until T2-060: every regiment is visible.
fn visible(_side: u8, _target: RegimentId) -> bool {
    true
}

/// SIM-PROJ-009 (plan decision 11): whether the segment `a → b` passes
/// through the circle `(c, r)`.
pub fn segment_hits_circle(a: V2, b: V2, c: V2, r: S) -> bool {
    let ab = b - a;
    let len_sq = ab.length_sq();
    let t = if len_sq > S::ZERO {
        ((c - a).dot(ab) / len_sq).clamp(S::ZERO, S::ONE)
    } else {
        S::ZERO
    };
    let closest = a + ab * t;
    closest.distance_sq(c) <= r * r
}

/// Soldiers of the regiment at `entity` whose distance from `from` lies in
/// `[lo, hi]` (SIM-PROJ-001's range annulus).
fn count_in_annulus(world: &World, entity: Entity, from: V2, lo: S, hi: S) -> u32 {
    let Some(regiment) = world.get::<Regiment>(entity) else {
        return 0;
    };
    let ids = world.resource::<Ids>();
    let mut n = 0;
    for &sid in &regiment.soldiers {
        if let Some(e) = ids.soldier_entity(sid)
            && let Some(pos) = world.get::<Pos>(e)
        {
            let d = pos.p.distance(from);
            if d >= lo && d <= hi {
                n += 1;
            }
        }
    }
    n
}

/// Stage 9 `ranged_target` (SIM-PROJ-001, SIM-PROJ-002): every
/// `ranged_retarget_ticks` (staggered by regiment id) a `fire_at_will`
/// regiment takes the visible enemy regiment with the most soldiers inside
/// its annulus, measured from its anchor with the height-adjusted range,
/// keeping the current target while it still has one there; `target` mode
/// uses the ordered regiment and falls back to `fire_at_will` once it is
/// emptied; `hold` fires nothing. Fills `RangedGateRes` for Stage 10.
pub fn ranged_target(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    let (period, volley, block_dist, height_range, height_ref) = {
        let c = &world.resource::<Regs>().0.rules.combat;
        (
            u32::from(c.ranged_retarget_ticks.max(1)),
            c.volley,
            c.friendly_block_dist,
            c.height_range,
            c.height_ref,
        )
    };
    let map = world.resource::<MapRes>().0.clone();
    let regiment_entities: Vec<(RegimentId, Entity)> =
        world.resource::<Ids>().regiment_entities.clone();
    let n = regiment_entities.len();
    let mut may_fire = vec![false; n];
    let mut target = vec![None; n];
    let mut volley_ready = vec![false; n];
    let mut blockers: Vec<Vec<RegimentId>> = vec![Vec::new(); n];
    let extent: Vec<S> = {
        let gate = world.resource::<MeleeGateRes>();
        if gate.extent.len() == n {
            gate.extent.clone()
        } else {
            vec![S::ZERO; n]
        }
    };
    let extent_max = extent.iter().fold(S::ZERO, |a, b| a.max(*b));
    let mut found: Vec<Entry<RegimentId>> = Vec::new();

    for (i, (rid, entity)) in regiment_entities.iter().enumerate() {
        let Some(fire) = world.get::<Fire>(*entity).copied() else {
            continue;
        };
        let (side, alive, unit, anchor, morale_ok) = {
            let r = world.get::<Regiment>(*entity).expect("regiment");
            let a = world.get::<Anchor>(*entity).expect("anchor");
            let m = world.get::<Morale>(*entity).expect("morale");
            (
                r.side,
                !r.soldiers.is_empty(),
                r.unit,
                a.pos,
                !matches!(m.state, MoraleState::Routing | MoraleState::Shattered),
            )
        };
        let ranged = {
            let regs = &world.resource::<Regs>().0;
            regs.units.get(unit).ranged.clone()
        };
        let Some(ranged) = ranged else {
            continue;
        };
        let alive_regiment = |world: &World, t: RegimentId| {
            world
                .resource::<Ids>()
                .regiment_entity(t)
                .and_then(|e| world.get::<Regiment>(e))
                .is_some_and(|r| !r.soldiers.is_empty())
        };

        let mut mode = fire.mode;
        let mut current = fire.target;
        // Plan decision 13: an ordered target that is gone hands the
        // regiment back to fire-at-will.
        if let FireMode::Target(t) = mode
            && (!alive_regiment(world, t) || !visible(side, t))
        {
            mode = FireMode::FireAtWill;
            current = None;
        }
        if let Some(t) = current
            && !alive_regiment(world, t)
        {
            current = None;
        }

        let stagger = tick.0 % period == rid.0 % period;
        let h_i = map.height_at(anchor);
        let annulus = |world: &World, t: RegimentId| -> u32 {
            let Some(te) = world.resource::<Ids>().regiment_entity(t) else {
                return 0;
            };
            let a_t = world.get::<Anchor>(te).map_or(anchor, |a| a.pos);
            let hi = ranged.range * range_mult(h_i, map.height_at(a_t), height_range, height_ref);
            count_in_annulus(world, te, anchor, ranged.min_range, hi)
        };
        let chosen = if stagger || current.is_none() {
            match mode {
                FireMode::Hold => None,
                FireMode::Target(t) => (annulus(world, t) >= 1).then_some(t),
                FireMode::FireAtWill => {
                    let radius = ranged.range * (S::ONE + height_range) + extent_max;
                    world
                        .resource::<AnchorGridRes>()
                        .0
                        .query_circle(anchor, radius, &mut found);
                    let mut best: Option<(u32, RegimentId)> = None;
                    // The current target is counted first so an equal count
                    // keeps it (SIM-PROJ-001 "preferring the current target").
                    if let Some(t) = current {
                        let c = annulus(world, t);
                        if c >= 1 {
                            best = Some((c, t));
                        }
                    }
                    if best.is_none() {
                        for e in &found {
                            let Some(j) = world.resource::<Ids>().regiment_index(e.id) else {
                                continue;
                            };
                            let enemy = world
                                .get::<Regiment>(e.entity)
                                .is_some_and(|r| r.side != side && !r.soldiers.is_empty());
                            if !enemy || !visible(side, e.id) {
                                continue;
                            }
                            let hi = ranged.range
                                * range_mult(h_i, map.height_at(e.pos), height_range, height_ref);
                            if anchor.distance(e.pos) > hi + extent[j] {
                                continue;
                            }
                            let c = count_in_annulus(world, e.entity, anchor, ranged.min_range, hi);
                            // Entries arrive ascending id: a strict `>` keeps
                            // the lower id on ties.
                            if c >= 1 && best.is_none_or(|(bc, _)| c > bc) {
                                best = Some((c, e.id));
                            }
                        }
                    }
                    best.map(|(_, t)| t)
                }
            }
        } else {
            current
        };

        if let Some(mut f) = world.get_mut::<Fire>(*entity) {
            f.mode = mode;
            f.target = chosen;
        }
        may_fire[i] = chosen.is_some() && alive && morale_ok;
        target[i] = chosen;
        volley_ready[i] = !volley || fire.cooldown == 0;

        // SIM-PROJ-009: friendly regiments that could mask a direct shot.
        if may_fire[i] && ranged.arc == ProjectileArc::Direct {
            let radius = block_dist + extent[i] + extent_max;
            world
                .resource::<AnchorGridRes>()
                .0
                .query_circle(anchor, radius, &mut found);
            for e in &found {
                if e.id == *rid {
                    continue;
                }
                let Some(j) = world.resource::<Ids>().regiment_index(e.id) else {
                    continue;
                };
                let friendly = world
                    .get::<Regiment>(e.entity)
                    .is_some_and(|r| r.side == side && !r.soldiers.is_empty());
                if friendly && anchor.distance(e.pos) <= block_dist + extent[i] + extent[j] {
                    blockers[i].push(e.id);
                }
            }
        }
    }

    let mut gate = world.resource_mut::<RangedGateRes>();
    gate.may_fire = may_fire;
    gate.target = target;
    gate.volley_ready = volley_ready;
    gate.blockers = blockers;
}

type Shooter<'w, 's> = Query<
    'w,
    's,
    (
        &'static Soldier,
        &'static Pos,
        &'static Fsm,
        &'static FatigueC,
        &'static mut RangedState,
    ),
>;
type ShooterItem<'a> = (
    &'a Soldier,
    &'a Pos,
    &'a Fsm,
    &'a FatigueC,
    Mut<'a, RangedState>,
);
type TargetRead<'w, 's> = Query<'w, 's, (&'static Pos, &'static Vel)>;
type RegimentRead<'w, 's> = Query<'w, 's, (&'static Regiment, &'static Anchor)>;

struct Ctx<'a, 'w, 's> {
    ids: &'a Ids,
    regs: &'a il_data::Registries,
    map: &'a crate::map::LoadedMap,
    targets: &'a TargetRead<'w, 's>,
    regiments: &'a RegimentRead<'w, 's>,
    gate: &'a RangedGateRes,
    extent: &'a [S],
    tick: Tick,
    seed: u64,
    volley: bool,
    out: &'a Mutex<Vec<Shot>>,
}

impl Ctx<'_, '_, '_> {
    fn fire(&self, (soldier, pos, fsm, fatigue, mut ranged): ShooterItem<'_>) {
        let Some(ri) = self.ids.regiment_index(soldier.regiment) else {
            return;
        };
        if !self.gate.may_fire[ri] {
            return;
        }
        // SIM-PROJ-003: in volley mode the regiment's cooldown gates the
        // throw; otherwise the soldier's own counts down to its throw tick.
        if self.volley {
            if !self.gate.volley_ready[ri] {
                return;
            }
        } else if ranged.cooldown > 0 {
            ranged.cooldown -= 1;
            if ranged.cooldown > 0 {
                return;
            }
        }
        if ranged.ammo == 0 {
            return;
        }
        // Plan decision 8: not Fighting, Routing or Withdrawing.
        if !matches!(fsm.state, SoldierState::Idle | SoldierState::MoveToSlot) {
            return;
        }
        let Some(t) = self.gate.target[ri] else {
            return;
        };
        let Some((regiment_t, _)) = self
            .ids
            .regiment_entity(t)
            .and_then(|e| self.regiments.get(e).ok())
        else {
            return;
        };
        if regiment_t.soldiers.is_empty() {
            return;
        }
        let Some((regiment_i, _)) = self
            .ids
            .regiment_entity(soldier.regiment)
            .and_then(|e| self.regiments.get(e).ok())
        else {
            return;
        };
        let rules = &self.regs.rules;
        let c = &rules.combat;
        let unit = self.regs.units.get(soldier.unit);
        let Some(rg) = unit.ranged.as_ref() else {
            return;
        };

        // SIM-PROJ-003: the aimed soldier (draw index 1) and its predicted
        // position after the flight.
        let k = hash_draw_bits(self.seed, self.tick, soldier.id.0, 1) as usize
            % regiment_t.soldiers.len();
        let j = regiment_t.soldiers[k];
        let Some((pos_j, vel_j)) = self
            .ids
            .soldier_entity(j)
            .and_then(|e| self.targets.get(e).ok())
        else {
            return;
        };
        let p_i = pos.p;
        let h_i = self.map.height_at(p_i);
        let in_annulus = |p_j: V2| {
            let d = p_i.distance(p_j);
            let hi =
                rg.range * range_mult(h_i, self.map.height_at(p_j), c.height_range, c.height_ref);
            (d >= rg.min_range && d <= hi).then_some(d)
        };
        // SIM-PROJ-001 annulus, per soldier: a pick outside it gives way to
        // the nearest soldier of the target regiment inside it (ties lowest
        // id); none there means no throw this volley and the ammo is kept.
        let (p_j, v_j, d0) = match in_annulus(pos_j.p) {
            Some(d) => (pos_j.p, vel_j.v, d),
            None => {
                let mut best: Option<(S, V2, V2)> = None;
                for &sid in &regiment_t.soldiers {
                    let Some((p, v)) = self
                        .ids
                        .soldier_entity(sid)
                        .and_then(|e| self.targets.get(e).ok())
                    else {
                        continue;
                    };
                    if let Some(d) = in_annulus(p.p)
                        && best.is_none_or(|(bd, _, _)| d < bd)
                    {
                        best = Some((d, p.p, v.v));
                    }
                }
                let Some((d, p, v)) = best else {
                    return;
                };
                (p, v, d)
            }
        };
        let ticks = flight_ticks(rg.arc, d0, rg.projectile_speed, c.gravity);
        let dt = S::ONE / S::from_i32(TICKS_PER_SECOND as i32);
        let aim = p_j + v_j * (S::from_i32(ticks as i32) * dt);
        // SIM-PROJ-004: draw index 0 is the scatter angle.
        let end = aim
            + scatter(
                d0,
                rg.accuracy,
                c.scatter_scale,
                hash_draw::<S>(self.seed, self.tick, soldier.id.0, 0),
            );

        // SIM-PROJ-009: a direct shot is refused while a friendly footprint
        // lies on the line of fire within `friendly_block_dist`.
        let mut blocked = None;
        if rg.arc == ProjectileArc::Direct {
            let dir = (end - p_i).normalized_or_zero();
            let reach = p_i + dir * d0.min(c.friendly_block_dist);
            for &b in &self.gate.blockers[ri] {
                let Some(bi) = self.ids.regiment_index(b) else {
                    continue;
                };
                let Some((_, anchor_b)) = self
                    .ids
                    .regiment_entity(b)
                    .and_then(|e| self.regiments.get(e).ok())
                else {
                    continue;
                };
                if segment_hits_circle(p_i, reach, anchor_b.pos, self.extent[bi]) {
                    blocked = Some(b);
                    break;
                }
            }
        }
        let fm = fatigue_mults(fatigue.f, &rules.fatigue);
        let shot = Shot {
            shooter: soldier.id,
            regiment: soldier.regiment,
            side: regiment_i.side,
            target_regiment: t,
            start: p_i,
            end,
            land_tick: Tick(self.tick.0 + ticks),
            apex: apex_height(rg.arc, d0, c),
            arc: rg.arc,
            damage: rg.damage,
            pen: rg.armour_penetration,
            fatigue_interval: fm.interval,
            blocked,
        };
        self.out.lock().expect("shot buffer").push(shot);
        if blocked.is_none() && !self.volley {
            ranged.cooldown = cooldown_ticks(rg.reload_ticks, fm.interval, S::ONE, S::ONE);
        }
    }
}

/// Stage 10 `ranged_fire` (SIM-PROJ-003, SIM-PROJ-004, SIM-PROJ-009;
/// parallel, writes only its own `RangedState`).
#[allow(clippy::too_many_arguments)]
pub fn ranged_fire(
    mut shooters: Shooter,
    targets: TargetRead,
    regiments: RegimentRead,
    ids: Res<Ids>,
    regs: Res<Regs>,
    map: Res<MapRes>,
    clock: Res<Clock>,
    rng: Res<Rng>,
    gate: Res<RangedGateRes>,
    melee_gate: Res<MeleeGateRes>,
    shots: Res<Shots>,
) {
    if gate.may_fire.len() != ids.regiment_entities.len() || !gate.may_fire.iter().any(|m| *m) {
        return;
    }
    let extent: Vec<S> = if melee_gate.extent.len() == ids.regiment_entities.len() {
        melee_gate.extent.clone()
    } else {
        vec![S::ZERO; ids.regiment_entities.len()]
    };
    let ctx = Ctx {
        ids: &ids,
        regs: &regs.0,
        map: &map.0,
        targets: &targets,
        regiments: &regiments,
        gate: &gate,
        extent: &extent,
        tick: clock.tick,
        seed: rng.draw_seed(StreamId::CombatRanged),
        volley: regs.0.rules.combat.volley,
        out: &shots.0,
    };
    let ctx = &ctx;
    let run = |item: ShooterItem<'_>| ctx.fire(item);
    let parallel = bevy_tasks::ComputeTaskPool::try_get().is_some_and(|p| p.thread_num() > 1);
    if parallel {
        shooters.par_iter_mut().for_each(run);
    } else {
        shooters.iter_mut().for_each(run);
    }
}

#[derive(Default)]
struct VolleyStats {
    count: u16,
    blocked: Option<RegimentId>,
    fatigue_interval: S,
}

/// Stage 10 `ranged_spawn` (SIM-PROJ-003, SIM-PROJ-008 cap check): the
/// shots land in the projectile list in ascending shooter id with ids
/// allocated in that order, ammo is spent, `VolleyFired` and `FireBlocked`
/// are emitted per regiment, volley cooldowns reset to `reload_ticks ×`
/// the largest fatigue interval multiplier of the volley and every regiment
/// cooldown then counts down once (so the period is exactly `reload_ticks`).
pub fn ranged_spawn(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    let mut shots = {
        let buffer = world.resource::<Shots>();
        core::mem::take(&mut *buffer.0.lock().expect("shot buffer"))
    };
    shots.sort_by_key(|s| s.shooter);

    let (cap, volley) = {
        let c = &world.resource::<Regs>().0.rules.combat;
        (c.projectile_cap as usize, c.volley)
    };
    let mut stats: BTreeMap<RegimentId, VolleyStats> = BTreeMap::new();
    for shot in &shots {
        let entry = stats.entry(shot.regiment).or_default();
        if let Some(b) = shot.blocked {
            entry.blocked.get_or_insert(b);
            continue;
        }
        let live = world.resource::<Projectiles>().0.len();
        if live >= cap {
            // Over the cap the volley resolves statistically (T2-032);
            // until then the throw is refused and the ammo kept.
            continue;
        }
        let id = world.resource_mut::<Ids>().projectiles.alloc();
        world.resource_mut::<Projectiles>().0.push(Projectile {
            id,
            shooter: shot.shooter,
            shooter_regiment: shot.regiment,
            side: shot.side,
            launch_tick: tick,
            land_tick: shot.land_tick,
            start: shot.start,
            end: shot.end,
            apex: shot.apex,
            arc: shot.arc,
            damage: shot.damage,
            pen: shot.pen,
        });
        if let Some(e) = world.resource::<Ids>().soldier_entity(shot.shooter)
            && let Some(mut r) = world.get_mut::<RangedState>(e)
        {
            r.ammo = r.ammo.saturating_sub(1);
        }
        entry.count += 1;
        entry.fatigue_interval = entry.fatigue_interval.max(shot.fatigue_interval);
    }

    for (rid, s) in &stats {
        let Some(entity) = world.resource::<Ids>().regiment_entity(*rid) else {
            continue;
        };
        if let Some(blocker) = s.blocked {
            world.resource_mut::<Events>().0.push(
                tick,
                BattleEvent::FireBlocked {
                    regiment: *rid,
                    blocker,
                },
            );
        }
        if s.count == 0 {
            continue;
        }
        world.resource_mut::<Events>().0.push(
            tick,
            BattleEvent::VolleyFired {
                regiment: *rid,
                count: s.count,
            },
        );
        if volley {
            let reload = world
                .get::<Regiment>(entity)
                .map(|r| r.unit)
                .and_then(|u| {
                    let regs = &world.resource::<Regs>().0;
                    regs.units.get(u).ranged.as_ref().map(|rg| rg.reload_ticks)
                })
                .unwrap_or(1);
            if let Some(mut fire) = world.get_mut::<Fire>(entity) {
                fire.cooldown = cooldown_ticks(reload, s.fatigue_interval, S::ONE, S::ONE);
            }
        }
    }

    // Reset then count down: a regiment that threw at tick `t` reads zero
    // again at `t + reload_ticks` (plan H1).
    let regiment_entities: Vec<Entity> = world
        .resource::<Ids>()
        .regiment_entities
        .iter()
        .map(|(_, e)| *e)
        .collect();
    for e in regiment_entities {
        if let Some(mut fire) = world.get_mut::<Fire>(e)
            && fire.cooldown > 0
        {
            fire.cooldown -= 1;
        }
    }
}

// ------------------------------------------------------------ Stage 11 (T2-031)

/// SIM-PROJ-006: the soldier a projectile landing at `land` strikes among
/// `candidates` (`(id, distance to the landing point, collision radius)`,
/// ascending id): the nearest whose circle, grown by `projectile_radius`,
/// covers the point; ties keep the lowest id (strict `<` over an
/// ascending-id input).
pub fn pick_victim(
    candidates: impl IntoIterator<Item = (SoldierId, S, S)>,
    projectile_radius: S,
) -> Option<SoldierId> {
    let mut best: Option<(S, SoldierId)> = None;
    for (id, d, r) in candidates {
        if d <= r + projectile_radius && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, id));
        }
    }
    best.map(|(_, id)| id)
}

/// SIM-PROJ-006: lands every projectile whose `land_tick` is `tick`, in
/// ascending id: the grid at the landing point (this tick's Stage 7
/// build) gives the candidates, the impact arc is read from the victim's
/// facing toward the shooter, and the damage is queued for this tick.
fn projectile_land(world: &mut World, tick: Tick) {
    let landing: Vec<Projectile> = world
        .resource::<Projectiles>()
        .0
        .iter()
        .filter(|p| p.land_tick == tick)
        .copied()
        .collect();
    if landing.is_empty() {
        return;
    }
    let (projectile_radius, max_radius) = {
        let regs = &world.resource::<Regs>().0;
        let max_radius = regs
            .units
            .iter()
            .map(|(_, u)| u.soldier_radius)
            .fold(S::ZERO, |a, b| a.max(b));
        (regs.rules.combat.projectile_radius, max_radius)
    };
    let mut scratch: Vec<usize> = Vec::new();
    let mut queued: Vec<Pending> = Vec::new();
    let mut events: Vec<BattleEvent> = Vec::new();
    for p in &landing {
        let victim = {
            let grid = &world.resource::<SpatialGridRes>().0;
            grid.query_circle_indices(p.end, max_radius + projectile_radius, &mut scratch);
            let entries = grid.entries();
            pick_victim(
                scratch.iter().map(|&k| {
                    let e = &entries[k];
                    let r = world.get::<Body>(e.entity).map_or(S::ZERO, |b| b.r);
                    (e.id, e.pos.distance(p.end), r)
                }),
                projectile_radius,
            )
        };
        if let Some(victim_id) = victim
            && let Some(ve) = world.resource::<Ids>().soldier_entity(victim_id)
            && let (Some(soldier), Some(facing)) =
                (world.get::<Soldier>(ve), world.get::<Facing>(ve))
        {
            let regs = &world.resource::<Regs>().0;
            let unit = regs.units.get(soldier.unit);
            let arc = attack_arc(facing.theta, p.start - p.end, unit.frontal_arc_deg);
            let damage = ranged_damage(
                p.damage,
                unit.armour,
                p.pen,
                arc,
                unit.shield,
                &regs.rules.combat,
            );
            queued.push(Pending {
                apply_tick: tick,
                target: victim_id,
                damage,
                shooter: p.shooter,
                shooter_regiment: p.shooter_regiment,
            });
        }
        events.push(BattleEvent::ProjectileLanded {
            pos: p.end,
            hit: victim.is_some(),
            victim,
        });
    }
    world.resource_mut::<PendingDamage>().0.extend(queued);
    let mut queue = world.resource_mut::<Events>();
    for e in events {
        queue.0.push(tick, e);
    }
}

/// TDD §8.2 `apply_pending_damage`: the entries whose tick has come land
/// in `(apply_tick, target id)` order (a stable sort, so the queue order
/// breaks ties); a soldier whose hp crosses zero joins `Kills` with the
/// shooter and its regiment. Entries for a future tick stay queued.
fn apply_pending_damage(world: &mut World, tick: Tick) {
    let mut queue = core::mem::take(&mut world.resource_mut::<PendingDamage>().0);
    if queue.is_empty() {
        return;
    }
    queue.sort_by_key(|p| (p.apply_tick, p.target));
    let mut later = Vec::with_capacity(queue.len());
    let mut kills = Vec::new();
    for p in queue {
        if p.apply_tick.0 > tick.0 {
            later.push(p);
            continue;
        }
        let Some(e) = world.resource::<Ids>().soldier_entity(p.target) else {
            continue;
        };
        let Some(mut health) = world.get_mut::<Health>(e) else {
            continue;
        };
        let before = health.hp;
        health.hp = before - p.damage;
        if before > S::ZERO && health.hp <= S::ZERO {
            kills.push(Kill {
                victim: p.target,
                killer: Some(p.shooter),
                killer_regiment: Some(p.shooter_regiment),
            });
        }
    }
    world.resource_mut::<PendingDamage>().0 = later;
    world.resource_mut::<Kills>().0.extend(kills);
}

/// Stage 11 `projectile_stage` (T2-031): land, apply the damage queue,
/// drop the landed projectiles. Exclusive, one defined order.
pub fn projectile_stage(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    projectile_land(world, tick);
    apply_pending_damage(world, tick);
    world
        .resource_mut::<Projectiles>()
        .0
        .retain(|p| p.land_tick.0 > tick.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f32, y: f32) -> V2 {
        V2::from_f32_data(x, y)
    }

    #[test]
    fn victim_is_the_nearest_covered_circle_and_ties_take_the_lowest_id() {
        let r = S::from_f32_data(0.4);
        let pr = S::from_f32_data(0.3);
        let s = |x: f32| S::from_f32_data(x);
        // Nearest wins over a lower id.
        assert_eq!(
            pick_victim([(SoldierId(1), s(0.5), r), (SoldierId(2), s(0.2), r)], pr),
            Some(SoldierId(2))
        );
        // Equal distances: the lowest id (first in ascending input).
        assert_eq!(
            pick_victim([(SoldierId(3), s(0.5), r), (SoldierId(4), s(0.5), r)], pr),
            Some(SoldierId(3))
        );
        // Outside r + projectile_radius (0.7 m): nothing is hit.
        assert_eq!(
            pick_victim([(SoldierId(3), s(0.71), r), (SoldierId(4), s(2.0), r)], pr),
            None
        );
        // A bigger soldier (cavalry, 0.7 m) is covered from further away.
        assert_eq!(
            pick_victim([(SoldierId(5), s(0.9), s(0.7))], pr),
            Some(SoldierId(5))
        );
        assert_eq!(pick_victim([], pr), None);
    }

    #[test]
    fn segment_circle_intersection() {
        // Circle of radius 2 at (5, 0): the x axis segment crosses it.
        assert!(segment_hits_circle(
            v(0.0, 0.0),
            v(10.0, 0.0),
            v(5.0, 0.0),
            S::from_i32(2)
        ));
        // A parallel segment 3 m to the side misses it.
        assert!(!segment_hits_circle(
            v(0.0, 3.0),
            v(10.0, 3.0),
            v(5.0, 0.0),
            S::from_i32(2)
        ));
        // A short segment that stops before the circle misses it.
        assert!(!segment_hits_circle(
            v(0.0, 0.0),
            v(2.0, 0.0),
            v(5.0, 0.0),
            S::from_i32(2)
        ));
        // A degenerate segment tests its point.
        assert!(segment_hits_circle(
            v(4.0, 0.0),
            v(4.0, 0.0),
            v(5.0, 0.0),
            S::from_i32(2)
        ));
    }
}
