//! The 18-stage battle schedule (SAD §6.2, TDD §4.5, SIM-DET-007).
//!
//! One `Schedule` with one `SystemSet` per stage, chained in order. The stage
//! order is part of the determinism contract; moving a system across stages
//! is an ADR. Each stage holds a no-op system named after it until its real
//! systems arrive in later phases, so the schedule shape is fixed from Phase 0.

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::{ScheduleLabel, SingleThreadedExecutor};

use crate::command::apply_commands;
use crate::hash::flush_events_and_hash;

/// Label of the one battle schedule.
#[derive(ScheduleLabel, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BattleSchedule;

/// The stages, in execution order.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    pub const ALL: [Stage; 18] = [
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
}

// Placeholder systems, one per stage without real systems yet. Their names
// show up in profiler spans (T1-060) so the empty stages are visible.
fn stage_ai() {}
fn stage_formation() {}
fn stage_regiment_movement() {}
fn stage_soldier_steering() {}
fn stage_integrate() {}
fn stage_spatial_grid() {}
fn stage_collision() {}
fn stage_visibility() {}
fn stage_targeting() {}
fn stage_combat() {}
fn stage_projectiles() {}
fn stage_abilities() {}
fn stage_fatigue() {}
fn stage_morale() {}
fn stage_death() {}
fn stage_battle_flow() {}

/// Builds the battle schedule with the single-threaded executor.
/// [`crate::BattleWorld::set_threads`] swaps the executor.
pub fn build_schedule() -> Schedule {
    let mut schedule = Schedule::new(BattleSchedule);
    schedule.set_executor(SingleThreadedExecutor::new());
    schedule.configure_sets(
        (
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
        )
            .chain(),
    );
    schedule.add_systems((
        apply_commands.in_set(Stage::ApplyCommands),
        stage_ai.in_set(Stage::Ai),
        stage_formation.in_set(Stage::Formation),
        stage_regiment_movement.in_set(Stage::RegimentMovement),
        stage_soldier_steering.in_set(Stage::SoldierSteering),
        stage_integrate.in_set(Stage::Integrate),
        stage_spatial_grid.in_set(Stage::SpatialGrid),
        stage_collision.in_set(Stage::Collision),
        stage_visibility.in_set(Stage::Visibility),
        stage_targeting.in_set(Stage::Targeting),
        stage_combat.in_set(Stage::Combat),
        stage_projectiles.in_set(Stage::Projectiles),
        stage_abilities.in_set(Stage::Abilities),
        stage_fatigue.in_set(Stage::Fatigue),
        stage_morale.in_set(Stage::Morale),
        stage_death.in_set(Stage::Death),
        stage_battle_flow.in_set(Stage::BattleFlow),
        flush_events_and_hash.in_set(Stage::EventsAndHash),
    ));
    schedule
}
