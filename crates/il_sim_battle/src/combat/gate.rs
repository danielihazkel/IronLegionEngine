//! Stage 9, first: `melee_gate` (T2-020, SAD §12 T-10).
//!
//! Targeting must not cost every soldier a grid query every tick when the
//! armies are far apart. The gate walks each regiment once for its extent
//! (the farthest soldier from the anchor) and then asks the anchor grid
//! whether any enemy regiment lies within the two extents plus the engage
//! radius; only soldiers of regiments that pass run the per-soldier search.
//! Exclusive and O(soldiers + regiments): cheaper than one parallel pass
//! with a merge, and it never touches a component another stage writes.

use bevy_ecs::prelude::*;
use il_core::{RegimentId, S, Scalar};

use crate::components::{Anchor, Morale, MoraleState, Order, OrderKind, Pos, Regiment};
use crate::resources::{AnchorGridRes, Ids, MeleeGateRes, Regs};
use crate::spatial::Entry;

/// Regiments that may take or hold melee targets: Idle or attacking, not
/// routing, with soldiers left.
fn may_fight(regiment: &Regiment, order: &Order, morale: &Morale) -> bool {
    !regiment.soldiers.is_empty()
        && matches!(
            order.kind,
            OrderKind::Idle | OrderKind::AttackMove | OrderKind::AttackRegiment
        )
        && !matches!(morale.state, MoraleState::Routing | MoraleState::Shattered)
}

/// Stage 9 `melee_gate`: fills `MeleeGateRes` for this tick.
pub fn melee_gate(world: &mut World) {
    let (engage_radius, slack) = {
        let c = &world.resource::<Regs>().0.rules.combat;
        (c.engage_radius, c.reach_slack + c.second_rank_reach_bonus)
    };
    let regiment_entities: Vec<(RegimentId, Entity)> =
        world.resource::<Ids>().regiment_entities.clone();
    let n = regiment_entities.len();

    // Pass 1: side, eligibility and extent per regiment.
    let mut side = vec![0u8; n];
    let mut may = vec![false; n];
    let mut extent = vec![S::ZERO; n];
    let mut anchors = vec![il_core::V2::ZERO; n];
    for (i, (_, entity)) in regiment_entities.iter().enumerate() {
        let (Some(regiment), Some(anchor), Some(order), Some(morale)) = (
            world.get::<Regiment>(*entity),
            world.get::<Anchor>(*entity),
            world.get::<Order>(*entity),
            world.get::<Morale>(*entity),
        ) else {
            continue;
        };
        side[i] = regiment.side;
        may[i] = may_fight(regiment, order, morale);
        anchors[i] = anchor.pos;
        let ids = world.resource::<Ids>();
        let mut far = S::ZERO;
        for &sid in &regiment.soldiers {
            if let Some(e) = ids.soldier_entity(sid)
                && let Some(pos) = world.get::<Pos>(e)
            {
                far = far.max(pos.p.distance_sq(anchor.pos));
            }
        }
        extent[i] = far.sqrt();
    }
    let extent_max = extent.iter().fold(S::ZERO, |a, b| a.max(*b));

    // Pass 2: enemy within reach of each eligible regiment.
    let mut near = vec![false; n];
    let mut found: Vec<Entry<RegimentId>> = Vec::new();
    let grid = &world.resource::<AnchorGridRes>().0;
    let ids = world.resource::<Ids>();
    for i in 0..n {
        if !may[i] {
            continue;
        }
        let reach = extent[i] + extent_max + engage_radius + slack;
        grid.query_circle(anchors[i], reach, &mut found);
        near[i] = found.iter().any(|e| {
            ids.regiment_index(e.id).is_some_and(|j| {
                j != i
                    && side[j] != side[i]
                    && !world
                        .get::<Regiment>(e.entity)
                        .is_none_or(|r| r.soldiers.is_empty())
                    && anchors[i].distance(anchors[j])
                        <= extent[i] + extent[j] + engage_radius + slack
            })
        });
    }

    let mut gate = world.resource_mut::<MeleeGateRes>();
    gate.side = side;
    gate.may_fight = may;
    gate.near_enemy = near;
    gate.extent = extent;
}
