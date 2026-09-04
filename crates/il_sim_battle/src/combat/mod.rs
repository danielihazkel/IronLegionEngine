//! Combat (TDD §8.1): melee targeting and engagement (T2-020), melee
//! resolution (T2-021), death (T2-022).
//!
//! Stage 3 `pursue_update` steers attack orders toward their target
//! regiment; Stage 9 `melee_gate` decides which regiments can fight at
//! all, `melee_target` picks every soldier's target in parallel and
//! `melee_recount` derives attacker counts, the regiment `engaged` flags
//! and the charge windows in id order; Stage 10 `melee_attack` rolls the
//! attacks in parallel and `apply_outcomes` lands them in attacker id
//! order; Stage 15 `resolve_deaths` removes the fallen. `formulas` holds
//! the pure rule functions.

//!
//! Ranged (T2-030, T2-031): Stage 9 `ranged_target` picks each shooting
//! regiment's target on its stagger tick; Stage 10 `ranged_fire` computes
//! the shots in parallel and `ranged_spawn` turns them into projectiles in
//! shooter id order; Stage 11 `projectile_stage` lands them and applies
//! the damage queue.

pub mod attack;
pub mod death;
pub mod formulas;
pub mod gate;
pub mod pursue;
pub mod ranged;
pub mod target;

pub use attack::{AttackOutcome, Kill, Kills, Outcomes, apply_outcomes, melee_attack};
pub use death::{resolve_deaths, ring_slot};
pub use formulas::{
    Arc, FatigueMults, apex_height, arc_mults, attack_arc, braced, charge_mults, cooldown_ticks,
    experience_mult, fatigue_mults, flight_ticks, footprint_area, hit_probability, melee_damage,
    morale_mults, range_mult, ranged_damage, scatter, stat_hit_probability, terrain_defence_mult,
};
pub use gate::melee_gate;
pub use pursue::pursue_update;
pub use ranged::{
    Shot, Shots, pick_victim, projectile_stage, ranged_fire, ranged_spawn, ranged_target,
    segment_hits_circle,
};
pub use target::{melee_recount, melee_target, rebuild_attackers, rebuild_charge_mass};
