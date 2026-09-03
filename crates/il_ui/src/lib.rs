//! Iron Legion UI (`il_ui`, TDD §11, REQ-TECH-004).
//!
//! egui context and event plumbing plus the profiler overlay (T1-060);
//! bindings, input state, selection and hit testing (T1-061); orders and
//! the drag-formation gesture arrive in T1-062. The UI only ever reads the
//! sim through `BattleView` and emits Commands (SAD §5.2); it never sees the
//! renderer, so hit testing takes a projection closure from the app.

pub mod bindings;
pub mod context;
pub mod input;
pub mod overlay;
pub mod pick;
pub mod profiler;
pub mod selection;

pub use bindings::{Action, BindingError, Bindings, Button, Chord, Mods, Trigger, parse_chord};
pub use context::{UiContext, UiOutput};
pub use egui;
pub use input::{Drag, Gesture, InputState, gesture_matches};
pub use overlay::selection_box;
pub use pick::{
    Project, own_regiments, owned, pick_regiment, regiments_in_box, regiments_of_type_on_screen,
};
pub use profiler::{ProfilerStats, StageStat, profiler_overlay};
pub use selection::{GROUPS, Selection};
