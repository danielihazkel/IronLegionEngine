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

use crate::combat::formulas::StatMults;
use crate::components::{
    Anchor, GeneralTag, Morale, MoraleState, Order, OrderKind, Pos, Regiment, Soldier, Statuses,
};
use crate::resources::{AnchorGridRes, Ids, MeleeGateRes, Regs, Sides};
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
    let mut status = vec![StatMults::default(); n];
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
        // SIM-ABIL-005 (T2-050): the cached multipliers, for Stage 10.
        status[i] = world
            .get::<Statuses>(*entity)
            .map_or_else(StatMults::default, |s| s.mults);
    }
    // The extents in one pass over the living soldiers (T2-111): the
    // regiment lists hold exactly the spawned soldiers, so a table scan
    // gives the same maxima as walking each list through `Ids`.
    {
        let mut far_sq = vec![S::ZERO; n];
        let mut soldiers = world.query::<(&Soldier, &Pos)>();
        let ids = world.resource::<Ids>();
        for (soldier, pos) in soldiers.iter(world) {
            if let Some(i) = ids.regiment_index(soldier.regiment) {
                far_sq[i] = far_sq[i].max(pos.p.distance_sq(anchors[i]));
            }
        }
        for (e, f) in extent.iter_mut().zip(far_sq) {
            *e = f.sqrt();
        }
    }
    let extent_max = extent.iter().fold(S::ZERO, |a, b| a.max(*b));

    // Pass 2: enemy within reach of each eligible regiment.
    let mut near = vec![false; n];
    let mut found: Vec<Entry<RegimentId>> = Vec::new();
    let mut scratch: Vec<usize> = Vec::new();
    let grid = &world.resource::<AnchorGridRes>().0;
    let ids = world.resource::<Ids>();
    for i in 0..n {
        if !may[i] {
            continue;
        }
        let reach = extent[i] + extent_max + engage_radius + slack;
        grid.query_circle_with(anchors[i], reach, &mut scratch, &mut found);
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

    // SIM-GEN-002 (T2-043): each side's aura circle this tick.
    let auras: Vec<Option<(il_core::V2, S)>> = {
        let g = &world.resource::<Regs>().0.rules.general;
        let ids = world.resource::<Ids>();
        world
            .resource::<Sides>()
            .0
            .iter()
            .map(|s| {
                if s.general_dead {
                    return None;
                }
                let ge = ids.soldier_entity(s.general?)?;
                let suspended = s
                    .general_regiment
                    .and_then(|r| ids.regiment_entity(r))
                    .and_then(|re| world.get::<Morale>(re))
                    .is_some_and(|m| {
                        matches!(m.state, MoraleState::Routing | MoraleState::Shattered)
                    });
                if suspended {
                    return None;
                }
                let rank = world.get::<GeneralTag>(ge)?.rank;
                let pos = world.get::<Pos>(ge)?.p;
                let radius = g.aura_radius
                    + g.aura_per_rank * S::from_i32(i32::from(rank.saturating_sub(1)));
                Some((pos, radius))
            })
            .collect()
    };
    let in_aura: Vec<bool> = (0..n)
        .map(|i| {
            auras
                .get(usize::from(side[i]))
                .copied()
                .flatten()
                .is_some_and(|(pos, radius)| anchors[i].distance(pos) <= radius)
        })
        .collect();

    let mut gate = world.resource_mut::<MeleeGateRes>();
    gate.side = side;
    gate.may_fight = may;
    gate.near_enemy = near;
    gate.extent = extent;
    gate.in_aura = in_aura;
    gate.status = status;
}
