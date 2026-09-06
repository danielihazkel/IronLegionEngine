//! Status effects: stacking (SIM-ABIL-004), expiry, the cached multipliers
//! and a regiment's ability slots (SIM-ABIL-003).

use bevy_ecs::prelude::*;
use il_data::{Ability, Handle, Registries, Stacking, UnitType};

use crate::combat::formulas::status_mults;
use crate::components::{Regiment, StatusEffect, Statuses};
use crate::resources::{Ids, Regs, Sides};

/// SIM-ABIL-004: applies `source` to `list`. A new source is appended
/// (list order is application order); an active one combines per the
/// ability's `stacking`: `Refresh` resets the duration, `Stack` bumps the
/// count up to `max_stacks` and resets the duration, `Highest` keeps the
/// longer remaining duration and the higher count. `hostile` (an enemy
/// applied it: debuffs only) is recorded on a new entry. Returns whether an
/// entry was added.
pub fn apply_status(
    list: &mut Vec<StatusEffect>,
    source: Handle<Ability>,
    a: &Ability,
    hostile: bool,
) -> bool {
    let duration = a.duration_ticks;
    if let Some(s) = list.iter_mut().find(|s| s.source == source) {
        match a.stacking {
            Stacking::Refresh => s.remaining = duration,
            Stacking::Stack => {
                s.stacks = s.stacks.saturating_add(1).min(a.max_stacks.max(1));
                s.remaining = duration;
            }
            Stacking::Highest => {
                s.remaining = s.remaining.max(duration);
                s.stacks = s.stacks.max(1);
            }
        }
        false
    } else {
        list.push(StatusEffect {
            source,
            remaining: duration,
            stacks: 1,
            hostile,
        });
        true
    }
}

/// Counts every status down by one tick and removes the ones that reached
/// zero, keeping list order; returns the removed sources in that order.
pub fn expire(list: &mut Vec<StatusEffect>) -> Vec<Handle<Ability>> {
    let mut gone = Vec::new();
    list.retain_mut(|s| {
        s.remaining = s.remaining.saturating_sub(1);
        if s.remaining == 0 {
            gone.push(s.source);
            false
        } else {
            true
        }
    });
    gone
}

/// Recomputes the cached multipliers from the list.
pub fn refresh_mults(statuses: &mut Statuses, regs: &Registries) {
    statuses.mults = status_mults(&statuses.list, regs);
}

/// Restore path: every regiment's cached multipliers from its stored list.
pub fn rebuild_status_mults(world: &mut World) {
    let regs = world.resource::<Regs>().0.clone();
    let entities: Vec<Entity> = world
        .resource::<Ids>()
        .regiment_entities
        .iter()
        .map(|(_, e)| *e)
        .collect();
    for e in entities {
        if let Some(mut s) = world.get_mut::<Statuses>(e) {
            refresh_mults(&mut s, &regs);
        }
    }
}

/// SIM-ABIL-003 (plan decision 9): the regiment's ability slots, aligned
/// with `Cooldowns`: its unit's abilities, then its general unit's while
/// this is the side's bodyguard regiment and the general is alive and on
/// the field. Slots past the unit's own list go dark when the general is
/// gone (their cooldowns keep counting).
pub fn slots(world: &World, entity: Entity) -> Vec<Handle<Ability>> {
    let regs = &world.resource::<Regs>().0;
    let Some(regiment) = world.get::<Regiment>(entity) else {
        return Vec::new();
    };
    let mut out = regs.units.get(regiment.unit).abilities.clone();
    if let Some(general_unit) = general_unit_of(world, regiment) {
        out.extend(regs.units.get(general_unit).abilities.iter().copied());
    }
    out
}

/// The general unit riding with this regiment while it lives, if any.
fn general_unit_of(world: &World, regiment: &Regiment) -> Option<Handle<UnitType>> {
    let side = world
        .resource::<Sides>()
        .0
        .get(usize::from(regiment.side))?;
    if side.general_dead || side.general_regiment != Some(regiment.id) {
        return None;
    }
    let ge = world.resource::<Ids>().soldier_entity(side.general?)?;
    let soldier = world.get::<crate::components::Soldier>(ge)?;
    Some(soldier.unit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use il_core::{S, Scalar};
    use il_data::{ContentId, Effect, Stat, Targeting};

    fn ability(stacking: Stacking, max_stacks: u8) -> Ability {
        Ability {
            id: ContentId::new("t:a").unwrap(),
            name_key: "t.a".into(),
            description_key: None,
            icon: None,
            targeting: Targeting::SelfTarget,
            radius: S::ZERO,
            range: S::ZERO,
            cooldown_ticks: 10,
            duration_ticks: 100,
            energy_cost: S::ZERO,
            effects: vec![Effect::Buff {
                stat: Stat::Attack,
                mult: S::from_f32_data(1.5),
                add: S::ZERO,
            }],
            stacking,
            max_stacks,
            requires_not_engaged: false,
            requires_not_moving: false,
            deprecated: None,
        }
    }

    fn handle() -> Handle<Ability> {
        let mut reg = il_data::Registry::<Ability>::new();
        reg.insert(ability(Stacking::Refresh, 1)).unwrap()
    }

    #[test]
    fn refresh_resets_the_duration_and_keeps_one_stack() {
        let a = ability(Stacking::Refresh, 1);
        let h = handle();
        let mut list = Vec::new();
        assert!(apply_status(&mut list, h, &a, false));
        list[0].remaining = 5;
        assert!(!apply_status(&mut list, h, &a, false));
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].remaining, list[0].stacks), (100, 1));
    }

    #[test]
    fn stack_counts_up_to_the_cap() {
        let a = ability(Stacking::Stack, 3);
        let h = handle();
        let mut list = Vec::new();
        for _ in 0..5 {
            list.iter_mut()
                .for_each(|s: &mut StatusEffect| s.remaining = 1);
            apply_status(&mut list, h, &a, false);
        }
        assert_eq!(list[0].stacks, 3);
        assert_eq!(
            list[0].remaining, 100,
            "every application resets the duration"
        );
    }

    #[test]
    fn highest_keeps_the_longer_duration() {
        let a = ability(Stacking::Highest, 1);
        let h = handle();
        let mut list = vec![StatusEffect {
            source: h,
            remaining: 250,
            stacks: 1,
            hostile: false,
        }];
        apply_status(&mut list, h, &a, false);
        assert_eq!(list[0].remaining, 250);
        list[0].remaining = 3;
        apply_status(&mut list, h, &a, false);
        assert_eq!(list[0].remaining, 100);
    }

    #[test]
    fn expire_removes_in_order_and_reports_the_sources() {
        let h = handle();
        let mut list = vec![
            StatusEffect {
                source: h,
                remaining: 1,
                stacks: 1,
                hostile: false,
            },
            StatusEffect {
                source: h,
                remaining: 2,
                stacks: 1,
                hostile: false,
            },
        ];
        let gone = expire(&mut list);
        assert_eq!(gone, vec![h]);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].remaining, 1);
    }
}
