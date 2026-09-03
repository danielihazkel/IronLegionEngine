//! The 18-stage battle schedule (SAD §6.2, TDD §4.5, SIM-DET-007).
//!
//! One `Schedule` per stage, run in [`Stage::ALL`] order by
//! `BattleWorld::step`. Stages were already totally ordered (chained sets),
//! so splitting them loses no parallelism and lets the app time each stage
//! through a [`StageObserver`] without any clock inside the sim (T1-060).
//! The stage order is part of the determinism contract; moving a system
//! across stages is an ADR. Each empty stage holds a no-op system named after
//! it until its real systems arrive.

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::{ScheduleLabel, SingleThreadedExecutor};

use crate::command::apply_commands;
use crate::formation::{formation_apply, formation_integrity, formation_layout};
use crate::hash::flush_events_and_hash;
use crate::movement::{collision_resolve, integrate, regiment_follow_path, soldier_steer};
use crate::nav::serve_path_requests;
use crate::spatial::rebuild_spatial_grids;

/// The stages, in execution order. Doubles as the label of each stage's
/// schedule and as the system set inside it.
#[derive(SystemSet, ScheduleLabel, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    /// Stage 0: commands sorted by `(player, seq)`; mutate orders.
    ApplyCommands,
    /// Stage 1: regiment and army utility AI.
    Ai,
    /// Stage 2: slot layouts and reform assignment.
    Formation,
    /// Stage 3: path following, anchor movement, wheeling.
    RegimentMovement,
    /// Stage 4: seek, separation, avoidance to desired velocity.
    SoldierSteering,
    /// Stage 5: position += velocity × dt; clamp to map.
    Integrate,
    /// Stage 6: rebuild spatial buckets.
    SpatialGrid,
    /// Stage 7: circle push resolution in deterministic pair order.
    Collision,
    /// Stage 8: regiment LOS and fog of war.
    Visibility,
    /// Stage 9: melee and ranged target selection.
    Targeting,
    /// Stage 10: attack cycles, hit rolls, damage, projectile spawn.
    Combat,
    /// Stage 11: projectile arcs, landing, damage.
    Projectiles,
    /// Stage 12: cooldowns, effects, status expiry.
    Abilities,
    /// Stage 13: fatigue accumulation and recovery.
    Fatigue,
    /// Stage 14: morale factors and state transitions.
    Morale,
    /// Stage 15: remove dead soldiers, update regiments.
    Death,
    /// Stage 16: phase transitions, victory, pursuit, timers.
    BattleFlow,
    /// Stage 17: flush events, state hash, interpolation buffer swap.
    EventsAndHash,
}

impl Stage {
    pub const COUNT: usize = 18;

    pub const ALL: [Stage; Stage::COUNT] = [
        Stage::ApplyCommands,
        Stage::Ai,
        Stage::Formation,
        Stage::RegimentMovement,
        Stage::SoldierSteering,
        Stage::Integrate,
        Stage::SpatialGrid,
        Stage::Collision,
        Stage::Visibility,
        Stage::Targeting,
        Stage::Combat,
        Stage::Projectiles,
        Stage::Abilities,
        Stage::Fatigue,
        Stage::Morale,
        Stage::Death,
        Stage::BattleFlow,
        Stage::EventsAndHash,
    ];

    /// Position in [`Stage::ALL`] (0..18).
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Stable display name (the variant name).
    pub fn name(self) -> &'static str {
        match self {
            Stage::ApplyCommands => "ApplyCommands",
            Stage::Ai => "Ai",
            Stage::Formation => "Formation",
            Stage::RegimentMovement => "RegimentMovement",
            Stage::SoldierSteering => "SoldierSteering",
            Stage::Integrate => "Integrate",
            Stage::SpatialGrid => "SpatialGrid",
            Stage::Collision => "Collision",
            Stage::Visibility => "Visibility",
            Stage::Targeting => "Targeting",
            Stage::Combat => "Combat",
            Stage::Projectiles => "Projectiles",
            Stage::Abilities => "Abilities",
            Stage::Fatigue => "Fatigue",
            Stage::Morale => "Morale",
            Stage::Death => "Death",
            Stage::BattleFlow => "BattleFlow",
            Stage::EventsAndHash => "EventsAndHash",
        }
    }
}

/// Receives a callback around every stage of a `step` (profiler, benches).
/// The sim never reads a clock; observers may.
pub trait StageObserver {
    fn begin(&mut self, stage: Stage);
    fn end(&mut self, stage: Stage);
}

/// The observer `BattleWorld::step` uses.
pub struct NoopObserver;

impl StageObserver for NoopObserver {
    fn begin(&mut self, _stage: Stage) {}
    fn end(&mut self, _stage: Stage) {}
}

// Placeholder systems, one per stage without real systems yet, so every
// stage shows up in the profiler with its own timing.
fn stage_ai() {}
fn stage_visibility() {}
fn stage_targeting() {}
fn stage_combat() {}
fn stage_projectiles() {}
fn stage_abilities() {}
fn stage_fatigue() {}
fn stage_morale() {}
fn stage_death() {}
fn stage_battle_flow() {}

fn stage_schedule(stage: Stage) -> Schedule {
    let mut s = Schedule::new(stage);
    s.set_executor(SingleThreadedExecutor::new());
    match stage {
        Stage::ApplyCommands => s.add_systems(apply_commands.in_set(stage)),
        Stage::Ai => s.add_systems(stage_ai.in_set(stage)),
        Stage::Formation => s.add_systems(
            (formation_layout, formation_apply, formation_integrity)
                .chain()
                .in_set(stage),
        ),
        Stage::RegimentMovement => s.add_systems(
            (serve_path_requests, regiment_follow_path)
                .chain()
                .in_set(stage),
        ),
        Stage::SoldierSteering => s.add_systems(soldier_steer.in_set(stage)),
        Stage::Integrate => s.add_systems(integrate.in_set(stage)),
        Stage::SpatialGrid => s.add_systems(rebuild_spatial_grids.in_set(stage)),
        Stage::Collision => s.add_systems(collision_resolve.in_set(stage)),
        Stage::Visibility => s.add_systems(stage_visibility.in_set(stage)),
        Stage::Targeting => s.add_systems(stage_targeting.in_set(stage)),
        Stage::Combat => s.add_systems(stage_combat.in_set(stage)),
        Stage::Projectiles => s.add_systems(stage_projectiles.in_set(stage)),
        Stage::Abilities => s.add_systems(stage_abilities.in_set(stage)),
        Stage::Fatigue => s.add_systems(stage_fatigue.in_set(stage)),
        Stage::Morale => s.add_systems(stage_morale.in_set(stage)),
        Stage::Death => s.add_systems(stage_death.in_set(stage)),
        Stage::BattleFlow => s.add_systems(stage_battle_flow.in_set(stage)),
        Stage::EventsAndHash => s.add_systems(flush_events_and_hash.in_set(stage)),
    };
    s
}

/// Builds the 18 per-stage schedules, in [`Stage::ALL`] order, with the
/// single-threaded executor. [`crate::BattleWorld::set_threads`] swaps it.
pub fn build_schedules() -> Vec<Schedule> {
    Stage::ALL.into_iter().map(stage_schedule).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_index_matches_all_order_and_names_are_unique() {
        for (i, stage) in Stage::ALL.iter().enumerate() {
            assert_eq!(stage.index(), i);
        }
        let mut names: Vec<&str> = Stage::ALL.iter().map(|s| s.name()).collect();
        names.dedup();
        assert_eq!(names.len(), Stage::COUNT);
        assert_eq!(build_schedules().len(), Stage::COUNT);
    }
}
