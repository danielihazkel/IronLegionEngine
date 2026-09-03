//! Formations (TDD §7): slot layouts (T1-040), slot assignment and resize
//! (T1-041), group arrangements (T1-046).

pub mod layout;

pub use layout::{
    LayoutFn, Slot, effective_ranks, files_for, files_used, layout_for, layout_slots, ranks_used,
    spacing,
};
