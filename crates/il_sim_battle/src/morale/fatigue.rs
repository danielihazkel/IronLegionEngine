//! Stage 13 fatigue (T2-040; SIM-FAT-001..005, TDD §8.3).
//!
//! `fatigue_tick` runs in parallel over soldiers and writes only each
//! soldier's own `FatigueC`; the activity comes from the soldier's FSM
//! state, whether its regiment's anchor is following a path this tick
//! (`movement::anchor_moves`) and the order's speed mode (plan decision 5
//! as built: soldiers standing in formation churn at up to 1 m/s from
//! separation and collision, so neither their velocity nor their
//! displacement can tell standing from walking; the anchor can). `regiment_fatigue_mean` refreshes `RegimentFatigue` every
//! `FATIGUE_MEAN_PERIOD` ticks in ascending regiment id.

use bevy_ecs::prelude::*;
use il_core::{S, Scalar};
use il_data::FatigueRules;

use crate::command::SpeedMode;
use crate::components::{
    Combat, FatigueC, Fsm, Order, Path, Pos, Regiment, RegimentFatigue, Soldier, SoldierState,
};
use crate::movement::regiment::{anchor_moves, tick_dt};
use crate::resources::{Clock, Ids, MapRes, Regs};

/// SIM-FAT-002: what a soldier is doing this tick, for the rate table.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Walk,
    March,
    Run,
    Fighting,
    Routing,
}

/// SIM-FAT-003: for the UI and morale only (SIM-FAT-004: the multipliers
/// are continuous in `f`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum FatigueState {
    Fresh = 0,
    Active = 1,
    Tired = 2,
    Exhausted = 3,
}

/// SIM-FAT-005: ticks between refreshes of the regiment mean.
pub const FATIGUE_MEAN_PERIOD: u32 = 10;

/// SIM-FAT-002 (plan decision 5): dead soldiers accumulate nothing (`None`);
/// routing and withdrawing soldiers pay `rate_routing`, fighters
/// `rate_fighting`; otherwise a soldier whose regiment stands
/// (`regiment_moving == false`) recovers and one whose regiment follows a
/// path pays the order's speed mode.
pub fn activity(
    state: SoldierState,
    regiment_moving: bool,
    order_speed: SpeedMode,
) -> Option<Activity> {
    Some(match state {
        SoldierState::Dead => return None,
        SoldierState::Routing | SoldierState::Withdrawing => Activity::Routing,
        SoldierState::Fighting => Activity::Fighting,
        SoldierState::Idle | SoldierState::MoveToSlot => {
            if !regiment_moving {
                Activity::Idle
            } else {
                match order_speed {
                    SpeedMode::Walk => Activity::Walk,
                    SpeedMode::Run => Activity::Run,
                    SpeedMode::March => Activity::March,
                }
            }
        }
    })
}

/// SIM-FAT-002: `weather.fatigue_mult` is `1` until the weather rules of
/// Phase 4, like the `status_mult` placeholder of T2-050.
pub fn weather_fatigue_mult() -> S {
    S::ONE
}

/// SIM-FAT-002: the per-second rate of `activity` for a unit with
/// `armour` and `fatigue_rate_mult` on ground with `zone_fatigue_mult`.
/// Armour adds `armour_rate × armour` to every positive base rate before
/// the multipliers.
pub fn fatigue_rate(
    activity: Activity,
    armour: S,
    fatigue_rate_mult: S,
    zone_fatigue_mult: S,
    r: &FatigueRules,
) -> S {
    let base = match activity {
        Activity::Idle => r.rate_idle,
        Activity::Walk => r.rate_walk,
        Activity::March => r.rate_march,
        Activity::Run => r.rate_run,
        Activity::Fighting => r.rate_fighting,
        Activity::Routing => r.rate_routing,
    };
    let base = if base > S::ZERO {
        base + r.armour_rate * armour
    } else {
        base
    };
    base * fatigue_rate_mult * zone_fatigue_mult * weather_fatigue_mult()
}

/// SIM-FAT-003: the state of fatigue `f` by the three upper bounds.
pub fn fatigue_state(f: S, r: &FatigueRules) -> FatigueState {
    if f < r.thresholds[0] {
        FatigueState::Fresh
    } else if f < r.thresholds[1] {
        FatigueState::Active
    } else if f < r.thresholds[2] {
        FatigueState::Tired
    } else {
        FatigueState::Exhausted
    }
}

type FatigueItem<'a> = (&'a Soldier, &'a Pos, &'a Fsm, Mut<'a, FatigueC>);
type RegimentRead<'w, 's> = Query<'w, 's, (&'static Order, &'static Path, &'static Combat)>;

/// Stage 13 `fatigue_tick` (parallel; writes only its own `FatigueC`).
/// SIM-FAT-002: `F ← clamp(F + rate × dt, 0, 1)`.
pub fn fatigue_tick(
    mut soldiers: Query<(&Soldier, &Pos, &Fsm, &mut FatigueC)>,
    regiments: RegimentRead,
    ids: Res<Ids>,
    regs: Res<Regs>,
    map: Res<MapRes>,
) {
    let dt = tick_dt();
    let regs = &regs.0;
    let map = &map.0;
    let ids = &ids;
    let regiments = &regiments;
    let rules = &regs.rules.fatigue;
    let run = |(soldier, pos, fsm, mut fatigue): FatigueItem<'_>| {
        let (moving, order_speed) = ids
            .regiment_entity(soldier.regiment)
            .and_then(|e| regiments.get(e).ok())
            .map_or((false, SpeedMode::Walk), |(o, path, combat)| {
                (anchor_moves(o, path, combat), o.speed)
            });
        let Some(act) = activity(fsm.state, moving, order_speed) else {
            return;
        };
        let zone = map
            .zone_at(pos.p)
            .map_or(S::ONE, |h| regs.zones.get(h).fatigue_mult);
        let unit = regs.units.get(soldier.unit);
        let rate = fatigue_rate(act, unit.armour, unit.fatigue_rate_mult, zone, rules);
        fatigue.f = (fatigue.f + rate * dt).clamp(S::ZERO, S::ONE);
    };
    let parallel = bevy_tasks::ComputeTaskPool::try_get().is_some_and(|p| p.thread_num() > 1);
    if parallel {
        soldiers.par_iter_mut().for_each(run);
    } else {
        soldiers.iter_mut().for_each(run);
    }
}

/// Stage 13 `regiment_fatigue_mean` (SIM-FAT-005): every
/// `FATIGUE_MEAN_PERIOD` ticks, the mean of `FatigueC` over the living
/// soldiers of every regiment, ascending id; an empty regiment reads `0`.
pub fn regiment_fatigue_mean(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    if !tick.0.is_multiple_of(FATIGUE_MEAN_PERIOD) {
        return;
    }
    let regiment_entities: Vec<Entity> = world
        .resource::<Ids>()
        .regiment_entities
        .iter()
        .map(|(_, e)| *e)
        .collect();
    for entity in regiment_entities {
        let Some(soldiers) = world.get::<Regiment>(entity).map(|r| r.soldiers.clone()) else {
            continue;
        };
        let mut sum = S::ZERO;
        let mut n = 0;
        for sid in &soldiers {
            if let Some(e) = world.resource::<Ids>().soldier_entity(*sid)
                && let Some(f) = world.get::<FatigueC>(e)
            {
                sum = sum + f.f;
                n += 1;
            }
        }
        let mean = if n == 0 {
            S::ZERO
        } else {
            sum / S::from_i32(n)
        };
        if let Some(mut rf) = world.get_mut::<RegimentFatigue>(entity) {
            rf.mean = mean;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> FatigueRules {
        let mut r = il_data::Rules::zeroed().fatigue;
        let sf = S::from_f32_data;
        r.rate_idle = sf(-0.010);
        r.rate_walk = sf(0.004);
        r.rate_march = sf(0.002);
        r.rate_run = sf(0.020);
        r.rate_fighting = sf(0.015);
        r.rate_routing = sf(0.020);
        r.armour_rate = sf(0.0002);
        r.thresholds = [sf(0.25), sf(0.5), sf(0.75)];
        r
    }

    #[test]
    fn activity_follows_state_then_motion_then_order() {
        assert_eq!(activity(SoldierState::Dead, true, SpeedMode::Run), None);
        for state in [SoldierState::Routing, SoldierState::Withdrawing] {
            assert_eq!(
                activity(state, false, SpeedMode::Walk),
                Some(Activity::Routing)
            );
        }
        assert_eq!(
            activity(SoldierState::Fighting, true, SpeedMode::Run),
            Some(Activity::Fighting)
        );
        for state in [SoldierState::Idle, SoldierState::MoveToSlot] {
            assert_eq!(activity(state, false, SpeedMode::Run), Some(Activity::Idle));
            assert_eq!(activity(state, true, SpeedMode::Walk), Some(Activity::Walk));
            assert_eq!(activity(state, true, SpeedMode::Run), Some(Activity::Run));
            assert_eq!(
                activity(state, true, SpeedMode::March),
                Some(Activity::March)
            );
        }
    }

    #[test]
    fn armour_adds_to_positive_rates_only() {
        let r = rules();
        let sf = S::from_f32_data;
        let (hastati, velites) = (sf(8.0), sf(2.0));
        assert_eq!(
            fatigue_rate(Activity::Run, hastati, S::ONE, S::ONE, &r),
            sf(0.020) + sf(0.0002) * hastati
        );
        assert_eq!(
            fatigue_rate(Activity::Run, velites, S::ONE, S::ONE, &r),
            sf(0.020) + sf(0.0002) * velites
        );
        assert_eq!(
            fatigue_rate(Activity::Idle, hastati, S::ONE, S::ONE, &r),
            sf(-0.010)
        );
        // Zone and unit multipliers scale the whole rate, recovery included.
        assert_eq!(
            fatigue_rate(Activity::Idle, hastati, sf(2.0), S::HALF, &r),
            sf(-0.010) * sf(2.0) * S::HALF
        );
        assert_eq!(
            fatigue_rate(Activity::March, velites, S::ONE, S::from_i32(2), &r),
            (sf(0.002) + sf(0.0002) * velites) * S::from_i32(2)
        );
    }

    #[test]
    fn fatigue_states_split_at_the_thresholds() {
        let r = rules();
        let sf = S::from_f32_data;
        assert_eq!(fatigue_state(S::ZERO, &r), FatigueState::Fresh);
        assert_eq!(fatigue_state(sf(0.2499), &r), FatigueState::Fresh);
        assert_eq!(fatigue_state(sf(0.25), &r), FatigueState::Active);
        assert_eq!(fatigue_state(sf(0.5), &r), FatigueState::Tired);
        assert_eq!(fatigue_state(sf(0.75), &r), FatigueState::Exhausted);
        assert_eq!(fatigue_state(S::ONE, &r), FatigueState::Exhausted);
    }
}
