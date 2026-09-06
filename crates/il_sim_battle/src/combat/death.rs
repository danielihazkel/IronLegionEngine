//! Stage 15: `resolve_deaths` (T2-022; SIM-CORE-008, SIM-CMBT-018,
//! SIM-FORM-021, TDD §8.1).
//!
//! The kills queued by Stage 10 are resolved in ascending victim id: the
//! soldier leaves its regiment's list (and the parallel slot assignment),
//! the casualty ring and the killer's regiment are updated, a
//! `SoldierDied` event carries the position for the render-only corpse,
//! dangling melee targets are cleared, and the entity is despawned and
//! dropped from `Ids` and the spatial grid so nothing downstream sees it.
//! Exclusive: every write happens in one defined order.

use bevy_ecs::prelude::*;
use il_core::{RegimentId, SoldierId, Tick, V2};

use crate::combat::Kills;
use crate::components::{
    Combat, DEATHS_RING, FormationState, Fsm, MeleeState, Morale, Pos, Regiment, Soldier,
    SoldierState,
};
use crate::events::BattleEvent;
use crate::resources::{
    Clock, Events, FlowFields, Ids, MoraleShocks, NavGridRes, Shock, ShockKind, Sides,
    SpatialGridRes,
};
use crate::spatial::Entry;

/// The casualty ring slot of a tick (SIM-MOR-010: a five-second window
/// with no head pointer; each tick owns one slot).
pub fn ring_slot(tick: Tick) -> usize {
    tick.0 as usize % DEATHS_RING
}

/// Stage 15 `resolve_deaths`.
pub fn resolve_deaths(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    let slot = ring_slot(tick);

    // Every tick owns its ring slot, so the window never carries stale
    // counts from five seconds ago.
    let regiment_entities: Vec<Entity> = world
        .resource::<Ids>()
        .regiment_entities
        .iter()
        .map(|(_, e)| *e)
        .collect();
    for e in &regiment_entities {
        if let Some(mut morale) = world.get_mut::<Morale>(*e) {
            morale.deaths_5s[slot] = 0;
        }
    }

    let mut dead = core::mem::take(&mut world.resource_mut::<Kills>().0);
    if dead.is_empty() {
        return;
    }
    dead.sort_by_key(|k| k.victim);
    dead.dedup_by_key(|k| k.victim);
    let dead_ids: Vec<SoldierId> = dead.iter().map(|k| k.victim).collect();

    for kill in &dead {
        let victim = &kill.victim;
        let Some(entity) = world.resource::<Ids>().soldier_entity(*victim) else {
            continue;
        };
        let (regiment, pos) = {
            let s = world.get::<Soldier>(entity).expect("soldier");
            let p = world.get::<Pos>(entity).expect("pos");
            (s.regiment, p.p)
        };
        world.resource_mut::<Events>().0.push(
            tick,
            BattleEvent::SoldierDied {
                id: *victim,
                regiment,
                killer: kill.killer,
                pos,
            },
        );
        if let Some(re) = detach_from_regiment(world, regiment, *victim)
            && let Some(mut m) = world.get_mut::<Morale>(re)
        {
            m.deaths_5s[slot] = m.deaths_5s[slot].saturating_add(1);
        }
        // Kill credit goes to the killer's regiment, resolved when the kill
        // was recorded, so a killer that fell earlier still counts.
        if let Some(kr) = kill.killer_regiment
            && let Some(kre) = world.resource::<Ids>().regiment_entity(kr)
            && let Some(mut c) = world.get_mut::<Combat>(kre)
        {
            c.kills = c.kills.saturating_add(1);
        }
        // SIM-GEN-003 (T2-043): the side's general fell.
        let side = world
            .resource::<Ids>()
            .regiment_entity(regiment)
            .and_then(|re| world.get::<Regiment>(re))
            .map(|r| r.side);
        if let Some(side) = side
            && world
                .resource::<Sides>()
                .0
                .get(usize::from(side))
                .is_some_and(|s| s.general == Some(*victim) && !s.general_dead)
        {
            general_died(world, side, *victim, tick);
        }
    }

    remove_soldiers(world, &dead_ids);
}

/// SIM-GEN-003 / SIM-MOR-014: marks the side's general dead, queues the
/// death shock for every regiment of the side (ascending; applied at the
/// next tick's Stage 14, halved for Shaken or worse then) and emits
/// `GeneralDied`. The aura ends with the flag (`melee_gate`).
fn general_died(world: &mut World, side: u8, soldier: SoldierId, tick: Tick) {
    world.resource_mut::<Sides>().0[usize::from(side)].general_dead = true;
    let regiments: Vec<RegimentId> = world
        .resource::<Ids>()
        .regiment_entities
        .iter()
        .filter(|(_, e)| world.get::<Regiment>(*e).is_some_and(|r| r.side == side))
        .map(|(id, _)| *id)
        .collect();
    for regiment in regiments {
        world.resource_mut::<MoraleShocks>().0.push(Shock {
            regiment,
            kind: ShockKind::GeneralDeath,
        });
    }
    world
        .resource_mut::<Events>()
        .0
        .push(tick, BattleEvent::GeneralDied { side, soldier });
}

/// SIM-FORM-021: takes `victim` out of its regiment's soldier list and the
/// parallel slot assignment and requests a reform; the layout itself is
/// rebuilt at the next Stage 2. Returns the regiment entity.
pub(crate) fn detach_from_regiment(
    world: &mut World,
    regiment: RegimentId,
    victim: SoldierId,
) -> Option<Entity> {
    let re = world.resource::<Ids>().regiment_entity(regiment)?;
    let removed = {
        let mut r = world.get_mut::<Regiment>(re).expect("regiment");
        let k = r.soldiers.binary_search(&victim).ok();
        if let Some(k) = k {
            r.soldiers.remove(k);
        }
        k
    };
    if let Some(mut f) = world.get_mut::<FormationState>(re) {
        if let Some(k) = removed
            && k < f.assignment.len()
        {
            f.assignment.remove(k);
        }
        f.needs_reform = true;
    }
    Some(re)
}

/// Takes the (ascending) soldiers out of every query at once (SIM-CORE-008):
/// nobody targets them any more (a fighter without a target holds still
/// until its next retarget tick, SIM-CORE-011), they leave `Ids`, the
/// world and the spatial grid.
pub(crate) fn remove_soldiers(world: &mut World, gone: &[SoldierId]) {
    let soldier_entities: Vec<Entity> = world
        .resource::<Ids>()
        .soldier_entities
        .iter()
        .map(|(_, e)| *e)
        .collect();
    for e in &soldier_entities {
        if let Some(mut m) = world.get_mut::<MeleeState>(*e)
            && let Some(t) = m.target
            && gone.binary_search(&t).is_ok()
        {
            m.target = None;
        }
    }
    let entities: Vec<Entity> = {
        let ids = world.resource::<Ids>();
        gone.iter()
            .filter_map(|id| ids.soldier_entity(*id))
            .collect()
    };
    world
        .resource_mut::<Ids>()
        .soldier_entities
        .retain(|(id, _)| gone.binary_search(id).is_err());
    for e in entities {
        world.despawn(e);
    }
    let mut grid = world.resource_mut::<SpatialGridRes>();
    let alive: Vec<Entry<SoldierId>> = grid
        .0
        .entries()
        .iter()
        .filter(|e| gone.binary_search(&e.id).is_err())
        .copied()
        .collect();
    grid.0.rebuild(alive);
}

/// Stage 15 `resolve_fled` (T2-042; SIM-FLOW-002, SIM-MOR-032), after
/// `resolve_deaths` so a soldier killed on the tick it reaches the edge is
/// a death and never both: every Routing or Withdrawing soldier standing
/// in a cell of its side's escape edge leaves the battle, ascending id;
/// `Combat.fled` counts it, `SoldierFled` carries the position. No corpse,
/// no casualty ring, no kill credit.
pub fn resolve_fled(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    let fled: Vec<(SoldierId, RegimentId, V2, SoldierState)> = {
        let ids = world.resource::<Ids>();
        let nav = &world.resource::<NavGridRes>().0;
        let flow = world.resource::<FlowFields>();
        ids.soldier_entities
            .iter()
            .filter_map(|(id, e)| {
                let fsm = world.get::<Fsm>(*e)?;
                if !matches!(fsm.state, SoldierState::Routing | SoldierState::Withdrawing) {
                    return None;
                }
                let regiment = world.get::<Soldier>(*e)?.regiment;
                let side = world.get::<Regiment>(ids.regiment_entity(regiment)?)?.side;
                let p = world.get::<Pos>(*e)?.p;
                flow.for_side(side)
                    .is_some_and(|f| f.is_exit(nav, p))
                    .then_some((*id, regiment, p, fsm.state))
            })
            .collect()
    };
    if fled.is_empty() {
        return;
    }
    for (id, regiment, pos, state) in &fled {
        // SIM-FLOW-002/014 (T2-070): a withdrawer is a survivor, a router
        // is fled.
        let withdrawing = *state == SoldierState::Withdrawing;
        world.resource_mut::<Events>().0.push(
            tick,
            if withdrawing {
                BattleEvent::SoldierWithdrew {
                    id: *id,
                    regiment: *regiment,
                    pos: *pos,
                }
            } else {
                BattleEvent::SoldierFled {
                    id: *id,
                    regiment: *regiment,
                    pos: *pos,
                }
            },
        );
        if let Some(re) = detach_from_regiment(world, *regiment, *id)
            && let Some(mut c) = world.get_mut::<Combat>(re)
        {
            if withdrawing {
                c.withdrawn = c.withdrawn.saturating_add(1);
            } else {
                c.fled = c.fled.saturating_add(1);
            }
        }
    }
    let ids: Vec<SoldierId> = fled.iter().map(|f| f.0).collect();
    remove_soldiers(world, &ids);
}

/// SIM-FLOW-015 (T2-070, plan I24): at the end of the pursuit every
/// Routing soldier still on the field escapes: counted as fled, one
/// `SoldierFled` each, removed like the dead (ascending id).
pub fn escape_all_routers(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    let routers: Vec<(SoldierId, RegimentId, V2)> = {
        let ids = world.resource::<Ids>();
        ids.soldier_entities
            .iter()
            .filter_map(|(id, e)| {
                let fsm = world.get::<Fsm>(*e)?;
                (fsm.state == SoldierState::Routing).then(|| {
                    let regiment = world.get::<Soldier>(*e).map(|s| s.regiment)?;
                    let p = world.get::<Pos>(*e)?.p;
                    Some((*id, regiment, p))
                })?
            })
            .collect()
    };
    if routers.is_empty() {
        return;
    }
    for (id, regiment, pos) in &routers {
        world.resource_mut::<Events>().0.push(
            tick,
            BattleEvent::SoldierFled {
                id: *id,
                regiment: *regiment,
                pos: *pos,
            },
        );
        if let Some(re) = detach_from_regiment(world, *regiment, *id)
            && let Some(mut c) = world.get_mut::<Combat>(re)
        {
            c.fled = c.fled.saturating_add(1);
        }
    }
    let ids: Vec<SoldierId> = routers.iter().map(|f| f.0).collect();
    remove_soldiers(world, &ids);
}
