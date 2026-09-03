//! Stage 17: interpolation buffer swap, event flush, state hash
//! (TDD §4.5 `flush_events_and_hash`, SIM-DET-004, REQ-SIM-005).

use bevy_ecs::prelude::*;
use il_core::{StateHash, StateHasher};

use crate::components::{
    Anchor, Facing, FatigueC, FormationState, Fsm, Health, Morale, Order, Path, Pos, PrevFacing,
    PrevPos, Regiment, SlotRef, Vel,
};
use crate::resources::{Clock, Events, Ids, LastHash, Phase, Rng, StepEvents};

/// Hashes exactly the fields SIM-DET-004 lists (as amended in T1-047), in
/// that order: tick; phase; per regiment (ascending id) morale, morale
/// state, soldier count, anchor, order (kind, target, facing, speed,
/// since), formation (template, ranks, files, integrity, morph_until,
/// needs_reform, prior template, laid-out facing), path (waypoints with
/// corridors, next, requested), ammo; per soldier (ascending id) `p`, `v`,
/// facing, `hp`, `fatigue`, FSM state, slot; per projectile (ascending id)
/// `p`, `v`; RNG stream states.
pub fn compute_hash(world: &mut World) -> StateHash {
    let mut h = StateHasher::new();
    h.write(&world.resource::<Clock>().tick);
    h.write(&world.resource::<Phase>().0);

    let ids = world.resource::<Ids>();
    let regiment_entities: Vec<Entity> = ids.regiment_entities.iter().map(|(_, e)| *e).collect();
    let soldier_entities: Vec<Entity> = ids.soldier_entities.iter().map(|(_, e)| *e).collect();

    let mut regiments =
        world.query::<(&Regiment, &Morale, &Anchor, &Order, &FormationState, &Path)>();
    for entity in regiment_entities {
        let (regiment, morale, anchor, order, formation, path) = regiments
            .get(world, entity)
            .expect("regiment entity in Ids has regiment components");
        h.write(&morale.m);
        h.write(&morale.state);
        h.write(&(regiment.soldiers.len() as u32));
        h.write(&anchor.pos);
        h.write(&anchor.facing);
        h.write(order);
        h.write(formation);
        h.write(path);
        h.write(&regiment.ammo);
    }

    let mut soldiers = world.query::<(&Pos, &Vel, &Facing, &Health, &FatigueC, &Fsm, &SlotRef)>();
    for entity in soldier_entities {
        let (pos, vel, facing, health, fatigue, fsm, slot) = soldiers
            .get(world, entity)
            .expect("soldier entity in Ids has soldier components");
        h.write(&pos.p);
        h.write(&vel.v);
        h.write(&facing.theta);
        h.write(&health.hp);
        h.write(&fatigue.f);
        h.write(&fsm.state);
        h.write(&slot.slot);
    }

    // Projectiles: none until Phase 2 (T2-030); the length prefix keeps the
    // layout stable when they arrive.
    h.write_u32(0);

    h.write(world.resource::<Rng>());
    h.finish()
}

/// Copies `Pos → PrevPos` and `Facing → PrevFacing` for the renderer's
/// interpolation, drains events into `StepEvents`, and stores the hash.
pub fn flush_events_and_hash(world: &mut World) {
    let soldier_entities: Vec<Entity> = world
        .resource::<Ids>()
        .soldier_entities
        .iter()
        .map(|(_, e)| *e)
        .collect();
    let mut interp = world.query::<(&Pos, &mut PrevPos, &Facing, &mut PrevFacing)>();
    for entity in soldier_entities {
        if let Ok((pos, mut prev_pos, facing, mut prev_facing)) = interp.get_mut(world, entity) {
            prev_pos.p = pos.p;
            prev_facing.theta = facing.theta;
        }
    }

    let events: Vec<_> = world
        .resource_mut::<Events>()
        .0
        .drain()
        .into_iter()
        .map(|(_, e)| e)
        .collect();
    world.resource_mut::<StepEvents>().0 = events;

    let hash = compute_hash(world);
    world.resource_mut::<LastHash>().0 = hash;
}
