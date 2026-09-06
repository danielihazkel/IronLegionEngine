//! Abilities (SIM-ABIL-001..007, TDD §8.3; T2-050): status effects on
//! regiments, per-slot cooldowns and energy.
//!
//! `UseAbility` is validated and applied at Stage 0 (`apply::use_ability`,
//! called from `command.rs`), so a status is in force for the same tick's
//! combat; Stage 12 `ability_tick` counts cooldowns and durations down,
//! regenerates energy and drops expired statuses, in ascending regiment id
//! (SIM-ABIL-007). The ten stat multipliers of a regiment's statuses are
//! cached in `Statuses.mults` (derived: refreshed whenever the list changes
//! and on restore) and read by melee, ranged, movement, fatigue, morale
//! and visibility.

pub mod apply;
pub mod status;
pub mod tick;

pub use apply::use_ability;
pub use status::{apply_status, expire, rebuild_status_mults, refresh_mults, slots};
pub use tick::ability_tick;
