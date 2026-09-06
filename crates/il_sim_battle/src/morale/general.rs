//! The general's fate (T2-043; SIM-GEN-004, TDD §8.3). `BattleResult`
//! (T2-071) calls this once the battle has ended and the losing side is
//! known.

use bevy_ecs::prelude::*;

use crate::components::{GeneralTag, Health, Morale, MoraleState, Soldier};
use crate::interface::GeneralFate;
use crate::resources::{Ids, Regs, Sides};

/// SIM-GEN-004 (plan decision 15): `Dead` if the general died; `Captured`
/// if the side lost and its bodyguard shattered; `Wounded` if the general
/// stands with less than `wounded_hp` of `unit.hp × hp_mult`; else `Alive`
/// (a general that fled the field alive is `Alive`, or `Captured` on the
/// losing side with a shattered bodyguard).
pub fn general_fate(world: &World, side: u8, lost: bool) -> GeneralFate {
    let Some(state) = world.resource::<Sides>().0.get(usize::from(side)) else {
        return GeneralFate::Alive;
    };
    if state.general_dead {
        return GeneralFate::Dead;
    }
    let ids = world.resource::<Ids>();
    let shattered = state
        .general_regiment
        .and_then(|r| ids.regiment_entity(r))
        .and_then(|re| world.get::<Morale>(re))
        .is_some_and(|m| m.state == MoraleState::Shattered);
    if lost && shattered {
        return GeneralFate::Captured;
    }
    let wounded = state
        .general
        .and_then(|g| ids.soldier_entity(g))
        .and_then(|e| {
            let hp = world.get::<Health>(e)?.hp;
            let unit = world.get::<Soldier>(e)?.unit;
            world.get::<GeneralTag>(e)?;
            let regs = &world.resource::<Regs>().0;
            let full = regs.units.get(unit).hp * regs.rules.general.hp_mult;
            Some(hp < regs.rules.general.wounded_hp * full)
        })
        .unwrap_or(false);
    if wounded {
        GeneralFate::Wounded
    } else {
        GeneralFate::Alive
    }
}
