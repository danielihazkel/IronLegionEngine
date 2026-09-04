//! Combat (TDD §8.1): melee targeting and engagement (T2-020), melee
//! resolution (T2-021), death (T2-022).
//!
//! Stage 3 `pursue_update` steers attack orders toward their target
//! regiment; Stage 9 `melee_gate` decides which regiments can fight at
//! all, `melee_target` picks every soldier's target in parallel and
//! `melee_recount` derives attacker counts and the regiment `engaged`
//! flags in id order.

pub mod gate;
pub mod pursue;
pub mod target;

pub use gate::melee_gate;
pub use pursue::pursue_update;
pub use target::{melee_recount, melee_target, rebuild_attackers};
