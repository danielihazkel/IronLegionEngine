//! Battle flow (SIM-FLOW-010..017, TDD §4.5 Stage 16; T2-070): the phase
//! machine, deployment auto-placement, reinforcements, defeat detection,
//! the pursuit timer and the verdict.
//!
//! Phases: `Deployment → Battle → Pursuit → Ended`. Stages outside the
//! phase's set are skipped by `step` (`Stage::runs_in`), so nothing moves
//! or fights before the deployment ends and nothing changes after the end.

use bevy_ecs::prelude::*;
use il_core::{Angle, RegimentId, S, Scalar, Tick, V2};
use il_data::{ContentId, GroupFormationTemplate, GroupKind, Handle, MapEdge, TimeoutWinner};

use crate::components::{
    Fsm, Morale, MoraleState, Order, OrderKind, Regiment, SoldierState, UnitGroup,
};
use crate::composition::{Composition, widest_radius};
use crate::events::BattleEvent;
use crate::formation::{RegimentInfo, arrange_group, effective_ranks};
use crate::interface::{RegimentSetup, ReinforcementGroup, SOLDIER_CAP};
use crate::map::LoadedMap;
use crate::movement::regiment::formation_width;
use crate::resources::{
    BattleFlow, BattlePhase, Clock, Events, Ids, MapRes, Phase, Regs, SetupRes, Sides,
};
use crate::visibility::Visibility;

/// The vertex mean of a side's deployment polygon, or the map centre.
pub fn zone_centre(map: &LoadedMap, zone: u8) -> V2 {
    match map.deployment_polygon(zone) {
        Some(poly) if !poly.is_empty() => {
            let sum = poly.iter().fold(V2::ZERO, |acc, p| acc + *p);
            sum * (S::ONE / S::from_i32(poly.len() as i32))
        }
        _ => V2::new(map.width * S::HALF, map.height * S::HALF),
    }
}

/// Where a side's auto-placed regiments face (plan I19): toward the mean
/// of the other sides' zone centres, or the map centre with no other side.
pub fn facing_toward(map: &LoadedMap, from: V2, others: &[V2]) -> Angle<S> {
    let target = if others.is_empty() {
        V2::new(map.width * S::HALF, map.height * S::HALF)
    } else {
        others.iter().fold(V2::ZERO, |acc, p| acc + *p)
            * (S::ONE / S::from_i32(others.len() as i32))
    };
    let d = target - from;
    if d.length_sq() > S::ZERO {
        Angle::from_direction(d.normalized_or_zero())
    } else {
        Angle::default()
    }
}

/// A `GroupFormationTemplate` built in code (plan G37): the battle line
/// used for auto-placement, without a content id to look up.
fn battle_line_template(gap: S) -> GroupFormationTemplate {
    GroupFormationTemplate {
        id: ContentId::new("il:auto_battle_line").expect("valid id"),
        name_key: String::new(),
        kind: GroupKind::BattleLine,
        gap,
        skirmishers_forward: false,
        cavalry_flanks: false,
        lines: 1,
        deprecated: None,
    }
}

/// SIM-FLOW-011 (plan I19): anchors and facings for the regiments of one
/// side that have no `position`: a battle line at the zone centre facing
/// the enemy, in the regiments' setup order. Returns one placement per
/// input regiment, in input order.
pub fn auto_placements(
    regs: &il_data::Registries,
    map: &LoadedMap,
    regiments: &[(&RegimentSetup, Vec<UnitGroup>, u16)],
    zone: u8,
    others: &[V2],
) -> Vec<(V2, Angle<S>)> {
    let centre = zone_centre(map, zone);
    let facing = facing_toward(map, centre, others);
    let infos: Vec<RegimentInfo> = regiments
        .iter()
        .enumerate()
        .map(|(k, (r, units, count))| {
            let comp = Composition::of(regs, units);
            let template = r
                .formation
                .as_ref()
                .and_then(|id| regs.formations.lookup(id))
                .unwrap_or_else(|| regs.units.get(comp.first).default_formation());
            RegimentInfo {
                id: RegimentId(k as u32),
                pos: centre,
                category: comp.category,
                count: *count,
                template,
                radius: widest_radius(regs, units),
            }
        })
        .collect();
    let template = battle_line_template(regs.rules.formation.group_gap);
    // A width no line needs to deepen for.
    let placements = arrange_group(
        &template,
        &infos,
        centre,
        facing,
        S::from_i32(100_000),
        &regs.rules.formation,
        regs,
    );
    regiments
        .iter()
        .enumerate()
        .map(|(k, _)| {
            placements
                .iter()
                .find(|p| p.id == RegimentId(k as u32))
                .map_or((centre, facing), |p| (p.anchor, p.facing))
        })
        .collect()
}

/// The midpoint of a map edge and the facing into the map (SIM-FLOW-016).
pub fn edge_entry(map: &LoadedMap, edge: MapEdge) -> (V2, Angle<S>, V2) {
    let (w, h) = (map.width, map.height);
    match edge {
        MapEdge::West => (
            V2::new(S::ZERO, h * S::HALF),
            Angle::from_direction(V2::new(S::ONE, S::ZERO)),
            V2::new(S::ZERO, S::ONE),
        ),
        MapEdge::East => (
            V2::new(w, h * S::HALF),
            Angle::from_direction(V2::new(-S::ONE, S::ZERO)),
            V2::new(S::ZERO, S::ONE),
        ),
        MapEdge::South => (
            V2::new(w * S::HALF, S::ZERO),
            Angle::from_direction(V2::new(S::ZERO, S::ONE)),
            V2::new(S::ONE, S::ZERO),
        ),
        MapEdge::North => (
            V2::new(w * S::HALF, h),
            Angle::from_direction(V2::new(S::ZERO, -S::ONE)),
            V2::new(S::ONE, S::ZERO),
        ),
    }
}

/// Spawns one reinforcement group in Column at its edge midpoint, the
/// regiments side by side along the edge (`group_gap` apart), facing into
/// the map. Returns the regiments spawned.
fn spawn_group(world: &mut World, side: u8, group: &ReinforcementGroup) -> Vec<RegimentId> {
    let regs = world.resource::<Regs>().0.clone();
    let map = world.resource::<MapRes>().0.clone();
    let (mid, facing, along) = edge_entry(&map, group.edge);
    let gap = regs.rules.formation.group_gap;
    // Widths in Column (the composition's first Column template, else the
    // first group's default), then cumulative offsets centred on the
    // midpoint.
    let widths: Vec<(S, Handle<il_data::FormationTemplate>)> = group
        .regiments
        .iter()
        .map(|r| {
            let units = crate::composition::resolve_groups(&regs, r);
            let comp = Composition::of(&regs, &units);
            let template = comp
                .column_template(&regs)
                .unwrap_or_else(|| regs.units.get(comp.first).default_formation());
            let t = regs.formations.get(template);
            let ranks = effective_ranks(t, r.total(), None);
            let files = crate::formation::files_for(r.total(), ranks.max(1));
            (
                formation_width(t, files.max(1), widest_radius(&regs, &units)),
                template,
            )
        })
        .collect();
    let total = widths.iter().fold(S::ZERO, |acc, (w, _)| acc + *w)
        + gap * S::from_i32(widths.len().saturating_sub(1) as i32);
    let mut cursor = -total * S::HALF;
    let mut spawned = Vec::with_capacity(group.regiments.len());
    for (r, (w, template)) in group.regiments.iter().zip(&widths) {
        let centre = cursor + *w * S::HALF;
        cursor = cursor + *w + gap;
        let anchor = map.clamp(mid + along * centre);
        let setup = RegimentSetup {
            formation: Some(regs.formations.id_of(*template).clone()),
            position: None,
            facing_deg: None,
            ..r.clone()
        };
        crate::spawn::spawn_regiment(world, side, &setup, None, Some((anchor, facing)));
        if let Some((id, _)) = world.resource::<Ids>().regiment_entities.last() {
            spawned.push(*id);
        }
    }
    spawned
}

/// SIM-FLOW-016 / SIM-CORE-006: every group whose `arrival_tick` (since the
/// Battle phase began) has come, side then group order; a group that would
/// pass the cap is dropped with an event.
fn spawn_due_reinforcements(world: &mut World, tick: Tick) {
    let battle_start = world.resource::<BattleFlow>().battle_start;
    let since = tick.0.saturating_sub(battle_start.0);
    let Some(setup) = world.resource::<SetupRes>().0.clone() else {
        return;
    };
    let mut spawned_any = false;
    for (s, side) in setup.sides.iter().enumerate() {
        loop {
            let done = usize::from(world.resource::<Sides>().0[s].reinforcements_spawned);
            let Some(group) = side.reinforcements.get(done) else {
                break;
            };
            if group.arrival_tick > since {
                break;
            }
            let incoming: u32 = group.regiments.iter().map(|r| u32::from(r.total())).sum();
            let alive = world.soldier_count_alive();
            world.resource_mut::<Sides>().0[s].reinforcements_spawned = (done + 1) as u8;
            if alive + incoming > SOLDIER_CAP {
                world.resource_mut::<Events>().0.push(
                    tick,
                    BattleEvent::ReinforcementsDropped {
                        side: s as u8,
                        group: done as u8,
                    },
                );
                continue;
            }
            spawn_group(world, s as u8, group);
            spawned_any = true;
            world
                .resource_mut::<Events>()
                .0
                .push(tick, BattleEvent::ReinforcementsArrived { side: s as u8 });
        }
    }
    if spawned_any {
        let sides = world.resource::<Sides>().0.len();
        let regiments = world.resource::<Ids>().regiment_entities.len();
        world.resource_mut::<Visibility>().resize(sides, regiments);
        world.refresh_view_queries();
    }
}

trait AliveCount {
    fn soldier_count_alive(&self) -> u32;
    fn refresh_view_queries(&mut self);
}

impl AliveCount for World {
    fn soldier_count_alive(&self) -> u32 {
        self.resource::<Ids>().soldier_entities.len() as u32
    }

    fn refresh_view_queries(&mut self) {
        // The cached view queries pick the new archetypes up at the end of
        // the step (`step_observed` refreshes them); nothing to do here.
    }
}

/// SIM-FLOW-013/014/017: a side is defeated when it surrendered, or has
/// no regiment with soldiers that stands (Steady..Broken) and is not
/// withdrawing, with no reinforcement group pending.
fn evaluate_defeat(world: &mut World) {
    let n_sides = world.resource::<Sides>().0.len();
    let pending: Vec<bool> = {
        let setup = world.resource::<SetupRes>().0.as_ref();
        let sides = &world.resource::<Sides>().0;
        (0..n_sides)
            .map(|s| {
                setup.is_some_and(|st| {
                    st.sides.get(s).is_some_and(|sd| {
                        usize::from(sides[s].reinforcements_spawned) < sd.reinforcements.len()
                    })
                })
            })
            .collect()
    };
    let mut standing = vec![false; n_sides];
    for (_, e) in &world.resource::<Ids>().regiment_entities {
        let (Some(r), Some(m), Some(o)) = (
            world.get::<Regiment>(*e),
            world.get::<Morale>(*e),
            world.get::<Order>(*e),
        ) else {
            continue;
        };
        if !r.soldiers.is_empty()
            && !matches!(m.state, MoraleState::Routing | MoraleState::Shattered)
            && o.kind != OrderKind::Withdraw
            && let Some(s) = standing.get_mut(usize::from(r.side))
        {
            *s = true;
        }
    }
    let mut sides = world.resource_mut::<Sides>();
    for (s, side) in sides.0.iter_mut().enumerate() {
        side.defeated = side.surrendered || (!standing[s] && !pending[s]);
    }
}

/// Living soldiers per side on the field.
fn field_counts(world: &World) -> Vec<u32> {
    let n = world.resource::<Sides>().0.len();
    let mut counts = vec![0u32; n];
    for (_, e) in &world.resource::<Ids>().regiment_entities {
        if let Some(r) = world.get::<Regiment>(*e)
            && let Some(c) = counts.get_mut(usize::from(r.side))
        {
            *c += r.soldiers.len() as u32;
        }
    }
    counts
}

/// SIM-FLOW-013 (plan decision 18): the winner when the timer expires.
fn timeout_winner(world: &World) -> Option<u8> {
    if let Some(side) = world
        .resource::<SetupRes>()
        .0
        .as_ref()
        .and_then(|s| s.victory.timeout_winner)
    {
        return Some(side);
    }
    match world.resource::<Regs>().0.rules.battle_flow.timeout_winner {
        TimeoutWinner::Defender => None,
        TimeoutWinner::MostSoldiers => {
            let counts = field_counts(world);
            let best = counts.iter().copied().max().unwrap_or(0);
            let leaders: Vec<u8> = counts
                .iter()
                .enumerate()
                .filter(|(_, c)| **c == best)
                .map(|(i, _)| i as u8)
                .collect();
            (leaders.len() == 1).then(|| leaders[0])
        }
    }
}

/// Moves the phase, emitting `PhaseChanged` and, on Ended, the result.
fn set_phase(world: &mut World, to: BattlePhase, tick: Tick) {
    let from = world.resource::<Phase>().0;
    if from == to {
        return;
    }
    world.resource_mut::<Phase>().0 = to;
    world
        .resource_mut::<Events>()
        .0
        .push(tick, BattleEvent::PhaseChanged { from, to });
    if to == BattlePhase::Ended {
        world.resource_mut::<BattleFlow>().ended_at = tick;
        // The AI decided this tick for a battle that has now ended: its
        // commands for the next tick would only be rejected as
        // `WrongPhase` (T3-010; SIM-AI-002).
        world.resource_mut::<crate::ai::AiState>().outbox.clear();
        let result = crate::result::compute(world);
        world.resource_mut::<Events>().0.push(
            tick,
            BattleEvent::Ended {
                result: Box::new(result),
            },
        );
    }
}

/// Whether any Routing or Shattered regiment still has soldiers on the field.
fn routers_remain(world: &World) -> bool {
    world
        .resource::<Ids>()
        .soldier_entities
        .iter()
        .any(|(_, e)| {
            world
                .get::<Fsm>(*e)
                .is_some_and(|f| f.state == SoldierState::Routing)
        })
}

/// Stage 16 `battle_flow`.
pub fn battle_flow(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    let phase = world.resource::<Phase>().0;
    let rules = world.resource::<Regs>().0.rules.battle_flow.clone();
    match phase {
        BattlePhase::Deployment => {
            // SIM-FLOW-011: engine-AI sides deploy and confirm through
            // Stage 1 commands (T2-081); everyone confirms at Stage 0.
            let all = world
                .resource::<Sides>()
                .0
                .iter()
                .all(|s| s.deployment_confirmed);
            let timed_out = rules.deploy_timeout_ticks > 0 && tick.0 >= rules.deploy_timeout_ticks;
            if all || timed_out {
                world.resource_mut::<BattleFlow>().battle_start = tick;
                set_phase(world, BattlePhase::Battle, tick);
            }
        }
        BattlePhase::Battle => {
            // A battle with fewer than two sides (tools, fixtures) has no
            // verdict: nothing is defeated and the timer does not run.
            if world.resource::<Sides>().0.len() < 2 {
                return;
            }
            spawn_due_reinforcements(world, tick);
            evaluate_defeat(world);
            let alive: Vec<u8> = world
                .resource::<Sides>()
                .0
                .iter()
                .enumerate()
                .filter(|(_, s)| !s.defeated)
                .map(|(i, _)| i as u8)
                .collect();
            let time_limit = world
                .resource::<SetupRes>()
                .0
                .as_ref()
                .map_or(rules.time_limit_ticks, |s| s.time_limit_ticks);
            let battle_start = world.resource::<BattleFlow>().battle_start;
            if alive.is_empty() {
                world.resource_mut::<BattleFlow>().winner = None;
                set_phase(world, BattlePhase::Ended, tick);
            } else if alive.len() == 1 && world.resource::<Sides>().0.len() > 1 {
                {
                    let mut flow = world.resource_mut::<BattleFlow>();
                    flow.winner = Some(alive[0]);
                    flow.pursuit_start = tick;
                }
                set_phase(world, BattlePhase::Pursuit, tick);
            } else if tick.0.saturating_sub(battle_start.0) >= time_limit {
                let winner = timeout_winner(world);
                world.resource_mut::<BattleFlow>().winner = winner;
                set_phase(world, BattlePhase::Ended, tick);
            }
        }
        BattlePhase::Pursuit => {
            let pursuit_start = world.resource::<BattleFlow>().pursuit_start;
            let over = tick.0.saturating_sub(pursuit_start.0) >= rules.pursuit_ticks;
            if over || !routers_remain(world) {
                // SIM-FLOW-015 (plan I24): the routers still on the field escape.
                crate::combat::death::escape_all_routers(world);
                set_phase(world, BattlePhase::Ended, tick);
            }
        }
        BattlePhase::Ended => {}
    }
}
