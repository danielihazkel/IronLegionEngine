//! Stage 12 `ability_tick` (SIM-ABIL-003, 004, 006, 007).

use bevy_ecs::prelude::*;
use il_core::Scalar;

use crate::abilities::status::{expire, refresh_mults};
use crate::components::{Cooldowns, Energy, Regiment, Statuses};
use crate::events::BattleEvent;
use crate::movement::regiment::tick_dt;
use crate::resources::{Clock, Events, Ids, Regs};

/// Stage 12: per regiment in ascending id, every slot cooldown counts down,
/// energy regenerates toward `unit.energy_max`, every status counts down
/// and the expired ones are removed (`StatusExpired` per source, list
/// order) with the cached multipliers refreshed.
pub fn ability_tick(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    let regs = world.resource::<Regs>().0.clone();
    let dt = tick_dt();
    let entities: Vec<Entity> = world
        .resource::<Ids>()
        .regiment_entities
        .iter()
        .map(|(_, e)| *e)
        .collect();
    for e in entities {
        if let Some(mut c) = world.get_mut::<Cooldowns>(e) {
            for t in &mut c.0 {
                *t = t.saturating_sub(1);
            }
        }
        let (rid, unit) = match world.get::<Regiment>(e) {
            Some(r) => (r.id, r.unit),
            None => continue,
        };
        {
            let u = regs.units.get(unit);
            if let Some(mut energy) = world.get_mut::<Energy>(e)
                && u.energy_regen > il_core::S::ZERO
            {
                energy.e = (energy.e + u.energy_regen * dt).min(u.energy_max);
            }
        }
        let gone = match world.get_mut::<Statuses>(e) {
            Some(mut s) if !s.list.is_empty() => {
                let gone = expire(&mut s.list);
                if !gone.is_empty() {
                    refresh_mults(&mut s, &regs);
                }
                gone
            }
            _ => Vec::new(),
        };
        for source in gone {
            world.resource_mut::<Events>().0.push(
                tick,
                BattleEvent::StatusExpired {
                    regiment: rid,
                    ability: regs.abilities.id_of(source).clone(),
                },
            );
        }
    }
}
