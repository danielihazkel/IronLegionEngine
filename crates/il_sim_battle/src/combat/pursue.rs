//! Stage 3, first: `pursue_update` (T2-020; SIM-CMBT-004, SIM-CMBT-005).
//!
//! Attack orders chase a regiment, not a point: every `pursue_repath_ticks`
//! (staggered by regiment id, and on the tick the order was issued) an
//! attacking regiment drops a dead target, an `AttackMove` regiment
//! acquires the nearest enemy anchor within `attack_move_radius`, and a
//! regiment with a live target re-requests its path toward that anchor
//! unless it is engaged (its anchor holds while soldiers fight, plan
//! decision 7) or fought within `retarget_period_ticks`. Inside
//! `charge_distance` a unit with a charge bonus switches to `run` and
//! stays there (SIM-CMBT-004). Exclusive, ascending regiment id.

use bevy_ecs::prelude::*;
use il_core::{RegimentId, Scalar};

use crate::command::SpeedMode;
use crate::components::{Anchor, Combat, Order, OrderKind, Path, Regiment};
use crate::resources::{AnchorGridRes, Clock, Ids, PathRequests, Regs};
use crate::spatial::Entry;

/// Whether `id` names a regiment with living soldiers.
pub(crate) fn regiment_alive(world: &World, id: RegimentId) -> bool {
    world
        .resource::<Ids>()
        .regiment_entity(id)
        .and_then(|e| world.get::<Regiment>(e))
        .is_some_and(|r| !r.soldiers.is_empty())
}

fn request_path(world: &mut World, entity: Entity, rid: RegimentId) {
    if let Some(mut path) = world.get_mut::<Path>(entity) {
        *path = Path {
            waypoints: Vec::new(),
            next: 0,
            requested: true,
        };
    }
    world.resource_mut::<PathRequests>().0.insert(rid);
}

/// SIM-CMBT-005: the nearest enemy regiment with soldiers whose anchor is
/// within `radius` of `from` (ties by ascending id).
fn acquire(world: &World, from: il_core::V2, side: u8, radius: il_core::S) -> Option<RegimentId> {
    let mut found: Vec<Entry<RegimentId>> = Vec::new();
    world
        .resource::<AnchorGridRes>()
        .0
        .query_circle(from, radius, &mut found);
    let mut best: Option<(il_core::S, RegimentId)> = None;
    for e in &found {
        let Some(r) = world.get::<Regiment>(e.entity) else {
            continue;
        };
        if r.side == side || r.soldiers.is_empty() {
            continue;
        }
        let d = e.pos.distance_sq(from);
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, e.id));
        }
    }
    best.map(|(_, id)| id)
}

/// Stage 3 `pursue_update`.
pub fn pursue_update(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    let (repath, charge_distance, acquire_radius, quiet_ticks) = {
        let c = &world.resource::<Regs>().0.rules.combat;
        (
            u32::from(c.pursue_repath_ticks.max(1)),
            c.charge_distance,
            c.attack_move_radius,
            u32::from(c.retarget_period_ticks),
        )
    };
    let regiment_entities: Vec<(RegimentId, Entity)> =
        world.resource::<Ids>().regiment_entities.clone();
    for (rid, entity) in regiment_entities {
        let Some(order) = world.get::<Order>(entity).copied() else {
            continue;
        };
        if !order.kind.is_attack() {
            continue;
        }
        if tick.0 % repath != rid.0 % repath && order.since != tick {
            continue;
        }
        let (side, unit, anchor_pos) = {
            let r = world.get::<Regiment>(entity).expect("regiment");
            let a = world.get::<Anchor>(entity).expect("anchor");
            (r.side, r.unit, a.pos)
        };
        let combat = world.get::<Combat>(entity).copied().unwrap_or_default();

        // 1. A dead target ends an AttackRegiment; an AttackMove resumes
        //    its move to the ordered point (SIM-CMBT-005).
        let mut target = order.target_regiment;
        if let Some(t) = target
            && !regiment_alive(world, t)
        {
            target = None;
            if let Some(mut o) = world.get_mut::<Order>(entity) {
                o.target_regiment = None;
            }
            if order.kind == OrderKind::AttackRegiment {
                crate::command::halt(world, entity);
                continue;
            }
            request_path(world, entity, rid);
        }

        // 2. Acquisition.
        if target.is_none()
            && order.kind == OrderKind::AttackMove
            && let Some(t) = acquire(world, anchor_pos, side, acquire_radius)
        {
            target = Some(t);
            if let Some(mut o) = world.get_mut::<Order>(entity) {
                o.target_regiment = Some(t);
            }
            request_path(world, entity, rid);
        }

        // 3. Pursuit.
        let Some(t) = target else {
            continue;
        };
        if combat.engaged {
            continue;
        }
        let target_pos = world
            .resource::<Ids>()
            .regiment_entity(t)
            .and_then(|e| world.get::<Anchor>(e))
            .map(|a| a.pos)
            .expect("alive target has an anchor");
        let quiet = tick.0.saturating_sub(combat.last_fighting.0) >= quiet_ticks;
        let path_active = world
            .get::<Path>(entity)
            .is_some_and(|p| p.requested || p.is_active());
        if quiet || !path_active {
            request_path(world, entity, rid);
        }
        let charge_bonus = world.resource::<Regs>().0.units.get(unit).charge_bonus;
        if charge_bonus > il_core::S::ZERO
            && anchor_pos.distance(target_pos) <= charge_distance
            && let Some(mut o) = world.get_mut::<Order>(entity)
        {
            o.speed = SpeedMode::Run;
        }
    }
}
