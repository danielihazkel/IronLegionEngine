//! Formations (TDD §7): slot layouts (T1-040), slot assignment, resize and
//! the Stage 2 systems (T1-041), group arrangements (T1-046).

pub mod assign;
pub mod layout;
pub mod systems;

pub use assign::{AssignScratch, AssignSoldier, assign_slots, slot_world};
pub use layout::{
    LayoutFn, Slot, effective_ranks, files_for, files_used, layout_for, layout_slots, ranks_used,
    spacing,
};
pub use systems::{formation_apply, formation_layout, rebuild_formation_derived};
