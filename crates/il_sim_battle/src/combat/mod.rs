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

pub mod attack;
pub mod death;
pub mod formulas;
pub mod gate;
pub mod pursue;
pub mod target;

pub use attack::{AttackOutcome, Kills, Outcomes, apply_outcomes, melee_attack};
pub use death::{resolve_deaths, ring_slot};
pub use formulas::{
    Arc, FatigueMults, arc_mults, attack_arc, braced, charge_mults, cooldown_ticks,
    experience_mult, fatigue_mults, hit_probability, melee_damage, morale_mults,
    terrain_defence_mult,
};
pub use gate::melee_gate;
pub use pursue::pursue_update;
pub use target::{melee_recount, melee_target, rebuild_attackers, rebuild_charge_mass};
