//! `BattleResult` (SIM-FLOW-018, TDD §4.2; T2-070 lands the counts and the
//! verdict, T2-071 the experience, ammo, general fate and loot).

use bevy_ecs::prelude::*;

use crate::components::{Combat, Morale, Regiment};
use crate::interface::{BattleResult, BattleSummary, GeneralFate, RegimentResult, SideResult};
use crate::resources::{BattleFlow, BattlePhase, Clock, Ids, Phase, SetupRes, Sides};

/// The result as it stands now (plan decision 23): `winner` only once the
/// phase is Ended, `duration_ticks` since the Battle phase began, and per
/// side, per regiment in ascending id, `initial`, `survivors` (on the field
/// plus withdrawn), `fled` and `killed = initial − survivors − fled`.
/// Reinforcement groups that never spawned are listed with `arrived: false`
/// and their full count as survivors.
pub fn compute(world: &World) -> BattleResult {
    let phase = world.resource::<Phase>().0;
    let flow = *world.resource::<BattleFlow>();
    let tick = world.resource::<Clock>().tick;
    let sides = &world.resource::<Sides>().0;
    let setup = world.resource::<SetupRes>().0.as_ref();
    let ids = world.resource::<Ids>();
    let mut out: Vec<SideResult> = (0..sides.len())
        .map(|_| SideResult {
            regiments: Vec::new(),
            general_fate: GeneralFate::Alive,
            loot: 0,
        })
        .collect();
    let mut total_killed = 0u32;
    let mut total_fled = 0u32;
    for (_, entity) in &ids.regiment_entities {
        let (Some(r), Some(m), Some(c)) = (
            world.get::<Regiment>(*entity),
            world.get::<Morale>(*entity),
            world.get::<Combat>(*entity),
        ) else {
            continue;
        };
        let survivors = (r.soldiers.len() as u16).saturating_add(c.withdrawn);
        let killed = m.initial.saturating_sub(survivors).saturating_sub(c.fled);
        total_killed += u32::from(killed);
        total_fled += u32::from(c.fled);
        if let Some(side) = out.get_mut(usize::from(r.side)) {
            side.regiments.push(RegimentResult {
                id: r.setup_id,
                initial: m.initial,
                survivors,
                fled: c.fled,
                killed,
                experience_gain: 0,
                ammo_left: 0,
                arrived: true,
            });
        }
    }
    // SIM-FLOW-016: groups still pending never fought.
    if let Some(setup) = setup {
        for (s, side) in setup.sides.iter().enumerate() {
            let spawned = sides
                .get(s)
                .map_or(0, |st| usize::from(st.reinforcements_spawned));
            for group in side.reinforcements.iter().skip(spawned) {
                for r in &group.regiments {
                    if let Some(side) = out.get_mut(s) {
                        side.regiments.push(RegimentResult {
                            id: r.id,
                            initial: r.count,
                            survivors: r.count,
                            fled: 0,
                            killed: 0,
                            experience_gain: 0,
                            ammo_left: 0,
                            arrived: false,
                        });
                    }
                }
            }
        }
    }
    BattleResult {
        winner: (phase == BattlePhase::Ended)
            .then_some(flow.winner)
            .flatten(),
        duration_ticks: tick.0.saturating_sub(flow.battle_start.0),
        sides: out,
        summary: BattleSummary {
            total_killed,
            total_fled,
        },
    }
}
