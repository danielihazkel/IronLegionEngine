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
use il_core::{SoldierId, Tick};

use crate::combat::Kills;
use crate::components::{
    Combat, DEATHS_RING, FormationState, MeleeState, Morale, Pos, Regiment, Soldier,
};
use crate::events::BattleEvent;
use crate::resources::{Clock, Events, Ids, SpatialGridRes};
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
        if let Some(re) = world.resource::<Ids>().regiment_entity(regiment) {
            let removed = {
                let mut r = world.get_mut::<Regiment>(re).expect("regiment");
                let k = r.soldiers.binary_search(victim).ok();
                if let Some(k) = k {
                    r.soldiers.remove(k);
                }
                k
            };
            if let Some(mut f) = world.get_mut::<FormationState>(re) {
                // SIM-FORM-021: the assignment stays parallel to the soldier
                // list; the layout itself is rebuilt at the next Stage 2.
                if let Some(k) = removed
                    && k < f.assignment.len()
                {
                    f.assignment.remove(k);
                }
                f.needs_reform = true;
            }
            if let Some(mut m) = world.get_mut::<Morale>(re) {
                m.deaths_5s[slot] = m.deaths_5s[slot].saturating_add(1);
            }
        }
        // Kill credit goes to the killer's regiment, resolved when the kill
        // was recorded, so a killer that fell earlier still counts.
        if let Some(kr) = kill.killer_regiment
            && let Some(kre) = world.resource::<Ids>().regiment_entity(kr)
            && let Some(mut c) = world.get_mut::<Combat>(kre)
        {
            c.kills = c.kills.saturating_add(1);
        }
    }

    // Nobody targets the dead; a fighter without a target holds still
    // until its next retarget tick (SIM-CORE-011).
    let soldier_entities: Vec<Entity> = world
        .resource::<Ids>()
        .soldier_entities
        .iter()
        .map(|(_, e)| *e)
        .collect();
    for e in &soldier_entities {
        if let Some(mut m) = world.get_mut::<MeleeState>(*e)
            && let Some(t) = m.target
            && dead_ids.binary_search(&t).is_ok()
        {
            m.target = None;
        }
    }

    // Leave the id lists, then the world, then the grid (SIM-CORE-008: the
    // sim removes the dead from every query at once).
    let dead_entities: Vec<Entity> = {
        let ids = world.resource::<Ids>();
        dead_ids
            .iter()
            .filter_map(|id| ids.soldier_entity(*id))
            .collect()
    };
    world
        .resource_mut::<Ids>()
        .soldier_entities
        .retain(|(id, _)| dead_ids.binary_search(id).is_err());
    for e in dead_entities {
        world.despawn(e);
    }
    let mut grid = world.resource_mut::<SpatialGridRes>();
    let alive: Vec<Entry<SoldierId>> = grid
        .0
        .entries()
        .iter()
        .filter(|e| dead_ids.binary_search(&e.id).is_err())
        .copied()
        .collect();
    grid.0.rebuild(alive);
}
