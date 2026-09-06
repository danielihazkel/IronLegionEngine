//! `UseAbility` (SIM-ABIL-003): validation and application at Stage 0.

use bevy_ecs::prelude::*;
use il_core::{RegimentId, S, Scalar, Tick, V2};
use il_data::{ContentId, Targeting};

use crate::abilities::status::{apply_status, refresh_mults, slots};
use crate::command::{AbilityFail, AbilityTarget, RejectReason};
use crate::components::{Anchor, Combat, Cooldowns, Energy, Order, Path, Regiment, Statuses};
use crate::events::BattleEvent;
use crate::movement::regiment::anchor_moves;
use crate::resources::{AnchorGridRes, Events, Ids, MapRes, Regs};
use crate::spatial::Entry;
use crate::visibility::Visibility;

/// Validates and applies `UseAbility` for `entity` (already known to exist,
/// be owned by the player and not be routing). Checks, in order: the
/// ability id, ownership of a slot, the cooldown, the energy, the target
/// against the targeting kind and range, then the conditions. On success
/// the slot cooldown is set, the energy deducted, the status applied to
/// every target regiment (plan decision 10: buffs reach same-side targets,
/// enemies get the debuffs only; ascending regiment id for areas) and
/// `AbilityUsed` is emitted.
pub fn use_ability(
    world: &mut World,
    entity: Entity,
    ability: &ContentId,
    target: &AbilityTarget,
    tick: Tick,
) -> Result<(), RejectReason> {
    let regs = world.resource::<Regs>().0.clone();
    let handle = regs
        .abilities
        .lookup(ability)
        .ok_or_else(|| RejectReason::UnknownContent(ability.clone()))?;
    let (rid, side, anchor) = {
        let r = world.get::<Regiment>(entity).expect("validated");
        let a = world.get::<Anchor>(entity).expect("anchor");
        (r.id, r.side, a.pos)
    };
    let fail = |why: AbilityFail| RejectReason::Ability {
        regiment: rid,
        ability: ability.clone(),
        why,
    };
    let slot = slots(world, entity)
        .iter()
        .position(|h| *h == handle)
        .ok_or_else(|| fail(AbilityFail::NotOwned))?;
    let a = regs.abilities.get(handle);
    if world
        .get::<Cooldowns>(entity)
        .and_then(|c| c.0.get(slot).copied())
        .unwrap_or(0)
        > 0
    {
        return Err(fail(AbilityFail::OnCooldown));
    }
    if world
        .get::<Energy>(entity)
        .is_some_and(|e| e.e < a.energy_cost)
    {
        return Err(fail(AbilityFail::NoEnergy));
    }

    // The target set (ascending regiment id) and the range check.
    let in_range = |p: V2| a.range <= S::ZERO || anchor.distance(p) <= a.range;
    let targets: Vec<(Entity, u8)> = match (a.targeting, target) {
        (Targeting::SelfTarget, AbilityTarget::SelfTarget) => vec![(entity, side)],
        (Targeting::RegimentAlly | Targeting::RegimentEnemy, AbilityTarget::Regiment(t)) => {
            let te = world
                .resource::<Ids>()
                .regiment_entity(*t)
                .ok_or_else(|| fail(AbilityFail::BadTarget))?;
            let (t_side, alive, t_pos) = {
                let r = world.get::<Regiment>(te).expect("regiment");
                let p = world.get::<Anchor>(te).expect("anchor").pos;
                (r.side, !r.soldiers.is_empty(), p)
            };
            let wanted_ally = a.targeting == Targeting::RegimentAlly;
            if !alive || (t_side == side) != wanted_ally {
                return Err(fail(AbilityFail::BadTarget));
            }
            // SIM-VIS-004: enemies must be visible to the user's side.
            if !wanted_ally && !visible(world, side, *t) {
                return Err(fail(AbilityFail::BadTarget));
            }
            if !in_range(t_pos) {
                return Err(fail(AbilityFail::OutOfRange));
            }
            vec![(te, t_side)]
        }
        (Targeting::Point, AbilityTarget::Point(p)) => {
            if !world.resource::<MapRes>().0.in_bounds(*p) {
                return Err(fail(AbilityFail::BadTarget));
            }
            if !in_range(*p) {
                return Err(fail(AbilityFail::OutOfRange));
            }
            regiments_within(world, *p, a.radius)
        }
        (Targeting::Area, AbilityTarget::SelfTarget) => regiments_within(world, anchor, a.radius),
        _ => return Err(fail(AbilityFail::BadTarget)),
    };

    // Conditions.
    if a.requires_not_engaged && world.get::<Combat>(entity).is_some_and(|c| c.engaged) {
        return Err(fail(AbilityFail::Engaged));
    }
    if a.requires_not_moving {
        let moving = {
            let o = world.get::<Order>(entity).expect("order");
            let p = world.get::<Path>(entity).expect("path");
            let c = world.get::<Combat>(entity).expect("combat");
            anchor_moves(o, p, c)
        };
        if moving {
            return Err(fail(AbilityFail::Moving));
        }
    }

    // Apply.
    let has_buff = a
        .effects
        .iter()
        .any(|e| matches!(e, il_data::Effect::Buff { .. }));
    let has_debuff = a
        .effects
        .iter()
        .any(|e| matches!(e, il_data::Effect::Debuff { .. }));
    if let Some(mut c) = world.get_mut::<Cooldowns>(entity)
        && let Some(t) = c.0.get_mut(slot)
    {
        *t = a.cooldown_ticks;
    }
    if let Some(mut e) = world.get_mut::<Energy>(entity) {
        e.e = (e.e - a.energy_cost).max(S::ZERO);
    }
    let mut applied: u8 = 0;
    for (te, t_side) in targets {
        let hostile = t_side != side;
        // A friendly target takes every effect; an enemy only the debuffs.
        if (hostile && !has_debuff) || (!hostile && !has_buff && !has_debuff) {
            continue;
        }
        if let Some(mut s) = world.get_mut::<Statuses>(te) {
            apply_status(&mut s.list, handle, a, hostile);
            refresh_mults(&mut s, &regs);
            applied = applied.saturating_add(1);
        }
    }
    world.resource_mut::<Events>().0.push(
        tick,
        BattleEvent::AbilityUsed {
            regiment: rid,
            ability: ability.clone(),
            targets: applied,
        },
    );
    Ok(())
}

/// Regiments with soldiers whose anchor lies within `radius` of `centre`,
/// ascending id, with their sides.
fn regiments_within(world: &World, centre: V2, radius: S) -> Vec<(Entity, u8)> {
    let mut found: Vec<Entry<RegimentId>> = Vec::new();
    world
        .resource::<AnchorGridRes>()
        .0
        .query_circle(centre, radius, &mut found);
    found
        .iter()
        .filter_map(|e| {
            let r = world.get::<Regiment>(e.entity)?;
            (!r.soldiers.is_empty()).then_some((e.entity, r.side))
        })
        .collect()
}

/// SIM-VIS-004 hook (T2-060): whether `side` sees regiment `target`.
fn visible(world: &World, side: u8, target: RegimentId) -> bool {
    world
        .resource::<Ids>()
        .regiment_index(target)
        .is_none_or(|i| world.resource::<Visibility>().sees(side, i))
}
