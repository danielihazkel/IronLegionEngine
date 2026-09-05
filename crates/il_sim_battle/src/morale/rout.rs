//! Routing, rally and shatter (T2-042; SIM-MOR-030..033, SIM-CORE-011,
//! TDD §8.3). Called from Stage 14 `morale_tick`; every write happens in
//! one exclusive pass in ascending regiment id.

use bevy_ecs::prelude::*;
use il_core::{Angle, RegimentId, S, Scalar, Tick, V2};

use crate::command::SpeedMode;
use crate::components::{
    Anchor, FormationState, Fsm, MeleeState, Morale, MoraleState, Order, OrderKind, Path, Pos,
    Regiment, SoldierState,
};
use crate::events::BattleEvent;
use crate::resources::{
    AnchorGridRes, Events, Ids, MoraleShocks, PathRequests, Regs, Shock, ShockKind,
};

/// SIM-MOR-030/032 (plan decisions 3 and G26): the regiment whose morale
/// fell to `t_routing` routs, or shatters when this would be its
/// `max_routs`-th rout, when fewer than `shatter_strength` of its initial
/// strength remain, or when its morale is 0. Returns the state entered.
/// The order becomes `Idle` at `Run` with the path dropped, every soldier
/// enters `Routing` and loses its melee target, and allies within
/// `rout_shock_radius` are queued a `Rout` shock (SIM-MOR-033).
pub fn enter_routing(world: &mut World, entity: Entity, tick: Tick) -> MoraleState {
    let regs = world.resource::<Regs>().0.clone();
    let m = &regs.rules.morale;
    let (id, side, anchor, soldiers) = {
        let r = world.get::<Regiment>(entity).expect("regiment");
        let a = world.get::<Anchor>(entity).expect("anchor");
        (r.id, r.side, a.pos, r.soldiers.clone())
    };
    let state = {
        let mut morale = world.get_mut::<Morale>(entity).expect("morale");
        let count = S::from_i32(soldiers.len() as i32);
        let initial = S::from_i32(i32::from(morale.initial));
        let shatter = morale.rout_count.saturating_add(1) >= m.max_routs
            || count < initial * m.shatter_strength
            || morale.m <= S::ZERO;
        morale.rout_count = morale.rout_count.saturating_add(1);
        morale.state = if shatter {
            MoraleState::Shattered
        } else {
            MoraleState::Routing
        };
        morale.state
    };
    if let Some(mut order) = world.get_mut::<Order>(entity) {
        order.kind = OrderKind::Idle;
        order.target_regiment = None;
        order.facing = None;
        order.speed = SpeedMode::Run;
        order.since = tick;
    }
    if let Some(mut path) = world.get_mut::<Path>(entity) {
        path.waypoints.clear();
        path.next = 0;
        path.requested = false;
    }
    world.resource_mut::<PathRequests>().0.remove(&id);
    for sid in &soldiers {
        let Some(e) = world.resource::<Ids>().soldier_entity(*sid) else {
            continue;
        };
        if let Some(mut fsm) = world.get_mut::<Fsm>(e) {
            fsm.state = SoldierState::Routing;
            fsm.since = tick;
        }
        if let Some(mut melee) = world.get_mut::<MeleeState>(e) {
            melee.target = None;
        }
    }
    // SIM-MOR-033: contagion to allies still standing (next tick).
    let neighbours: Vec<RegimentId> = {
        let grid = &world.resource::<AnchorGridRes>().0;
        let mut found = Vec::new();
        grid.query_circle_indices(anchor, m.rout_shock_radius, &mut found);
        found
            .iter()
            .map(|&k| grid.entries()[k])
            .filter(|e| e.id != id)
            .filter(|e| {
                world
                    .get::<Regiment>(e.entity)
                    .is_some_and(|r| r.side == side && !r.soldiers.is_empty())
                    && world.get::<Morale>(e.entity).is_some_and(|mo| {
                        !matches!(mo.state, MoraleState::Routing | MoraleState::Shattered)
                    })
            })
            .map(|e| e.id)
            .collect()
    };
    for regiment in neighbours {
        world.resource_mut::<MoraleShocks>().0.push(Shock {
            regiment,
            kind: ShockKind::Rout,
        });
    }
    if state == MoraleState::Shattered {
        world
            .resource_mut::<Events>()
            .0
            .push(tick, BattleEvent::Shattered { regiment: id });
    }
    state
}

/// SIM-MOR-031 (plan decision 2, G27): a Routing regiment with `m` at or
/// above `t_routing + rally_margin` and no enemy anchor within
/// `rally_safe_radius` of its centroid rallies into Shaken: it halts
/// (order `Idle` at `Walk`), faces the nearest enemy if any, reforms at
/// its centroid (the anchor already sits there) and its soldiers return to
/// their slots. Returns whether it rallied.
pub fn try_rally(
    world: &mut World,
    entity: Entity,
    tick: Tick,
    m: S,
    nearest_enemy: Option<(V2, S)>,
) -> bool {
    let rules = &world.resource::<Regs>().0.rules.morale;
    let (t_routing, margin, safe) = (rules.t_routing, rules.rally_margin, rules.rally_safe_radius);
    if m < t_routing + margin || nearest_enemy.is_some_and(|(_, d)| d <= safe) {
        return false;
    }
    let (id, soldiers) = {
        let r = world.get::<Regiment>(entity).expect("regiment");
        (r.id, r.soldiers.clone())
    };
    world.get_mut::<Morale>(entity).expect("morale").state = MoraleState::Shaken;
    if let Some(mut order) = world.get_mut::<Order>(entity) {
        order.kind = OrderKind::Idle;
        order.speed = SpeedMode::Walk;
        order.since = tick;
    }
    if let Some((enemy, _)) = nearest_enemy
        && let Some(mut anchor) = world.get_mut::<Anchor>(entity)
        && enemy != anchor.pos
    {
        anchor.facing = Angle::from_direction(enemy - anchor.pos);
    }
    if let Some(mut f) = world.get_mut::<FormationState>(entity) {
        f.needs_reform = true;
    }
    for sid in &soldiers {
        if let Some(e) = world.resource::<Ids>().soldier_entity(*sid)
            && let Some(mut fsm) = world.get_mut::<Fsm>(e)
        {
            fsm.state = SoldierState::MoveToSlot;
            fsm.since = tick;
        }
    }
    world
        .resource_mut::<Events>()
        .0
        .push(tick, BattleEvent::Rallied { regiment: id });
    true
}

/// Plan decision 10: a Routing or Shattered regiment's anchor follows its
/// soldiers' centroid, so contagion, rally checks and the overlay see
/// where it really is. Ascending regiment id; regiments without soldiers
/// keep their last anchor.
pub fn follow_centroid(world: &mut World) {
    let regiment_entities: Vec<Entity> = world
        .resource::<Ids>()
        .regiment_entities
        .iter()
        .map(|(_, e)| *e)
        .collect();
    for entity in regiment_entities {
        let routing = world
            .get::<Morale>(entity)
            .is_some_and(|m| matches!(m.state, MoraleState::Routing | MoraleState::Shattered));
        if !routing {
            continue;
        }
        let Some(soldiers) = world.get::<Regiment>(entity).map(|r| r.soldiers.clone()) else {
            continue;
        };
        if soldiers.is_empty() {
            continue;
        }
        let mut sum = V2::ZERO;
        let mut n = 0;
        for sid in &soldiers {
            if let Some(e) = world.resource::<Ids>().soldier_entity(*sid)
                && let Some(p) = world.get::<Pos>(e)
            {
                sum += p.p;
                n += 1;
            }
        }
        if n > 0
            && let Some(mut anchor) = world.get_mut::<Anchor>(entity)
        {
            anchor.pos = sum * (S::ONE / S::from_i32(n));
        }
    }
}
