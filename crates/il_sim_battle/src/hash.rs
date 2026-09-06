//! Stage 17: interpolation buffer swap, event flush, state hash
//! (TDD §4.5 `flush_events_and_hash`, SIM-DET-004, REQ-SIM-005).

use bevy_ecs::prelude::*;
use il_core::{StateHash, StateHasher};

use crate::components::{
    Anchor, Combat, Cooldowns, Energy, Facing, FatigueC, Fire, FormationState, Fsm, GeneralTag,
    Health, MeleeState, Morale, Order, Path, Pos, PrevFacing, PrevPos, RangedState, Regiment,
    RegimentFatigue, SlotRef, Statuses, Vel,
};
use crate::resources::{
    BattleFlow, Clock, Events, Ids, LastHash, MoraleShocks, PendingDamage, Phase, Projectiles,
    Regs, Rng, Sides, StepEvents,
};
use crate::visibility::Visibility;

/// Hashes exactly the fields SIM-DET-004 lists (as amended in T1-047,
/// T2-020, T2-030, T2-040 and T2-050), in that order: tick; phase; battle
/// flow (battle start, pursuit start, winner); per side (ascending index)
/// escape edge, deployment confirmed, defeated, surrendered, reinforcement
/// groups spawned, general, general regiment, general dead; per regiment
/// (ascending id) morale, morale state, soldier count, anchor, order (kind,
/// target, target regiment, facing, speed, since), formation (template,
/// ranks, files, integrity, morph_until, needs_reform, prior template,
/// laid-out facing), path (waypoints with corridors, next, requested), fire
/// state (present for ranged units: mode, target, cooldown), combat
/// (engaged, last fighting, charge_until, experience, kills, fled,
/// withdrawn), casualty ring, initial strength, rout count, engaged since,
/// arc hits, fatigue mean, energy, cooldowns (length-prefixed), statuses
/// (length-prefixed: source ability id, remaining, stacks, hostile); per
/// soldier (ascending id) `p`, `v`, facing, `hp`, `fatigue`, FSM state,
/// slot, melee (target, cooldown), ranged state (present for ranged units:
/// ammo, cooldown), general rank (present for the general); per projectile
/// (ascending id) every launch field; pending damage in queue order; morale
/// shocks in queue order; per side (ascending) the visibility mask
/// (length-prefixed, regiment order); RNG stream states.
pub fn compute_hash(world: &mut World) -> StateHash {
    let mut h = StateHasher::new();
    h.write(&world.resource::<Clock>().tick);
    h.write(&world.resource::<Phase>().0);
    h.write(world.resource::<BattleFlow>());
    for side in &world.resource::<Sides>().0 {
        side.hash_state(&mut h);
    }

    let regs = world.resource::<Regs>().0.clone();
    let ids = world.resource::<Ids>();
    let regiment_entities: Vec<Entity> = ids.regiment_entities.iter().map(|(_, e)| *e).collect();
    let soldier_entities: Vec<Entity> = ids.soldier_entities.iter().map(|(_, e)| *e).collect();

    let mut regiments = world.query::<(
        &Regiment,
        &Morale,
        &Anchor,
        &Order,
        &FormationState,
        &Path,
        &Combat,
        Option<&Fire>,
        &RegimentFatigue,
        &Energy,
        &Cooldowns,
        &Statuses,
    )>();
    for entity in regiment_entities {
        let (
            regiment,
            morale,
            anchor,
            order,
            formation,
            path,
            combat,
            fire,
            fatigue,
            energy,
            cooldowns,
            statuses,
        ) = regiments
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
        h.write(&fire.copied());
        h.write(combat);
        h.write(&morale.deaths_5s);
        h.write(&morale.initial);
        h.write(&morale.rout_count);
        h.write(&morale.engaged_since);
        h.write(&morale.arc_hit);
        h.write(fatigue);
        h.write(energy);
        h.write(&cooldowns.0);
        h.write(&(statuses.list.len() as u32));
        for s in &statuses.list {
            h.write(regs.abilities.id_of(s.source));
            h.write(&s.remaining);
            h.write(&s.stacks);
            h.write(&s.hostile);
        }
    }

    let mut soldiers = world.query::<(
        &Pos,
        &Vel,
        &Facing,
        &Health,
        &FatigueC,
        &Fsm,
        &SlotRef,
        &MeleeState,
        Option<&RangedState>,
        Option<&GeneralTag>,
    )>();
    for entity in soldier_entities {
        let (pos, vel, facing, health, fatigue, fsm, slot, melee, ranged, general) = soldiers
            .get(world, entity)
            .expect("soldier entity in Ids has soldier components");
        h.write(&pos.p);
        h.write(&vel.v);
        h.write(&facing.theta);
        h.write(&health.hp);
        h.write(&fatigue.f);
        h.write(&fsm.state);
        h.write(&slot.slot);
        h.write(melee);
        h.write(&ranged.copied());
        h.write(&general.map(|g| g.rank));
    }

    // Projectiles ascend by id in the list (T2-030); the pending damage
    // queue hashes in queue order (T2-031 applies it in `(tick, target)`
    // order with the queue order breaking ties).
    h.write(world.resource::<Projectiles>().0.as_slice());
    h.write(world.resource::<PendingDamage>().0.as_slice());
    h.write(world.resource::<MoraleShocks>().0.as_slice());
    // T2-050 layout / T2-060 content: each side's visibility mask.
    let vis = world.resource::<Visibility>();
    h.write(&(vis.masks.len() as u32));
    for m in &vis.masks {
        h.write(m.as_slice());
    }

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
