//! Iron Legion headless battle simulation (TDD §4, SAD §3).
//!
//! Commands in, events out; one `step` is exactly one 20 Hz tick through the
//! 18-stage schedule; the state hash and snapshot make every run verifiable.

pub mod command;
pub mod components;
pub mod events;
pub mod hash;
pub mod resources;
pub mod schedule;
pub mod world;

pub use command::{AbilityTarget, Command, CommandKind, FireMode, RejectReason, SpeedMode};
pub use events::BattleEvent;
pub use resources::{BattlePhase, Rules};
pub use schedule::Stage;
pub use world::{BattleWorld, StepOutput};
