//! Iron Legion UI (`il_ui`, TDD §11, REQ-TECH-004).
//!
//! egui context and event plumbing plus the profiler overlay (T1-060);
//! input state, bindings, selection and gestures arrive in T1-061/T1-062.
//! The UI only ever reads the sim through `BattleView` and emits Commands
//! (SAD §5.2).

pub mod context;
pub mod profiler;

pub use context::{UiContext, UiOutput};
pub use egui;
pub use profiler::{ProfilerStats, StageStat, profiler_overlay};
