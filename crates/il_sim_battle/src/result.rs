//! `BattleResult` (SIM-FLOW-018, TDD §4.2; T2-070 the counts and the
//! verdict, T2-071 the experience, ammo, general fate and loot).

use bevy_ecs::prelude::*;
use il_core::{S, Scalar};

use crate::components::{Combat, Morale, RangedState, Regiment};
use crate::interface::{BattleResult, BattleSummary, GeneralFate, RegimentResult, SideResult};
use crate::morale::general::general_fate;
use crate::resources::{BattleFlow, BattlePhase, Clock, Ids, Phase, Regs, SetupRes, Sides};

/// SIM-FLOW-018 (plan decision 19): experience points,
/// `floor(exp_per_kill × kills + exp_survive × [the regiment has survivors])`.
pub fn experience_gain(kills: u32, survived: bool, exp_per_kill: S, exp_survive: S) -> u16 {
    let survived = if survived { S::ONE } else { S::ZERO };
    let points =
        exp_per_kill * S::from_i32(kills.min(i32::MAX as u32) as i32) + exp_survive * survived;
    u16::try_from(points.floor_i32().max(0)).unwrap_or(u16::MAX)
}

/// The result as it stands now (plan decision 23): `winner` only once the
/// phase is Ended, `duration_ticks` since the Battle phase began, and per
/// side, per regiment in ascending id, `initial`, `survivors` (on the field
/// plus withdrawn), `fled`, `killed = initial − survivors − fled`, the
/// experience points, the ammo left (the sum over the living soldiers) and,
/// per side, the general's fate (SIM-GEN-004 with `lost` = another side
/// won) and the loot (`loot_per_enemy_killed × enemy soldiers killed`, the
/// winner only). Reinforcement groups that never spawned are listed with
/// `arrived: false` and their full count as survivors.
pub fn compute(world: &World) -> BattleResult {
    let phase = world.resource::<Phase>().0;
    let flow = *world.resource::<BattleFlow>();
    let tick = world.resource::<Clock>().tick;
    let sides = &world.resource::<Sides>().0;
    let setup = world.resource::<SetupRes>().0.as_ref();
    let ids = world.resource::<Ids>();
    let rules = &world.resource::<Regs>().0.rules.battle_flow;
    let winner = (phase == BattlePhase::Ended)
        .then_some(flow.winner)
        .flatten();
    let mut out: Vec<SideResult> = (0..sides.len())
        .map(|_| SideResult {
            regiments: Vec::new(),
            general_fate: GeneralFate::Alive,
            loot: 0,
        })
        .collect();
    let mut killed_per_side = vec![0u32; sides.len()];
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
        total_fled += u32::from(c.fled);
        let ammo_left: u32 = r
            .soldiers
            .iter()
            .filter_map(|sid| ids.soldier_entity(*sid))
            .filter_map(|e| world.get::<RangedState>(e))
            .map(|s| u32::from(s.ammo))
            .sum();
        if let Some(k) = killed_per_side.get_mut(usize::from(r.side)) {
            *k += u32::from(killed);
        }
        if let Some(side) = out.get_mut(usize::from(r.side)) {
            side.regiments.push(RegimentResult {
                id: r.setup_id,
                initial: m.initial,
                survivors,
                fled: c.fled,
                killed,
                experience_gain: experience_gain(
                    c.kills,
                    survivors > 0,
                    rules.exp_per_kill,
                    rules.exp_survive,
                ),
                ammo_left: u16::try_from(ammo_left).unwrap_or(u16::MAX),
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
    let total_killed: u32 = killed_per_side.iter().sum();
    for (s, side) in out.iter_mut().enumerate() {
        let lost = winner.is_some_and(|w| usize::from(w) != s);
        side.general_fate = general_fate(world, s as u8, lost);
        if winner == Some(s as u8) {
            let enemy_dead = total_killed - killed_per_side[s];
            side.loot = i64::from(
                (rules.loot_per_enemy_killed * S::from_i32(enemy_dead.min(i32::MAX as u32) as i32))
                    .floor_i32(),
            );
        }
    }
    BattleResult {
        winner,
        // Since the Battle phase began, frozen at the end, zero while deploying.
        duration_ticks: match phase {
            BattlePhase::Deployment => 0,
            BattlePhase::Ended => flow.ended_at.0.saturating_sub(flow.battle_start.0),
            _ => tick.0.saturating_sub(flow.battle_start.0),
        },
        sides: out,
        summary: BattleSummary {
            total_killed,
            total_fled,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn experience_points_floor_kills_and_survival() {
        let per_kill = S::from_f32_data(0.01);
        let survive = S::ONE;
        assert_eq!(experience_gain(0, true, per_kill, survive), 1);
        assert_eq!(experience_gain(0, false, per_kill, survive), 0);
        assert_eq!(experience_gain(250, true, per_kill, survive), 3);
        assert_eq!(experience_gain(99, false, per_kill, survive), 0);
    }
}
