//! Iron Legion headless battle simulation (TDD §4, SAD §3).
//!
//! Commands in, events out; one `step` is exactly one 20 Hz tick through the
//! 18-stage schedule; the state hash and snapshot make every run verifiable.

pub mod combat;
pub mod command;
pub mod components;
pub mod events;
pub mod formation;
pub mod hash;
pub mod interface;
pub mod map;
pub mod movement;
pub mod nav;
pub mod resources;
pub mod schedule;
pub mod snapshot;
pub mod spatial;
pub mod spawn;
pub mod view;
pub mod world;

pub use command::{AbilityTarget, Command, CommandKind, FireMode, RejectReason, SpeedMode};
pub use events::BattleEvent;
pub use formation::{
    AssignScratch, AssignSoldier, Slot, assign_slots, effective_ranks, layout_for, layout_slots,
    ranks_for_width, slot_world,
};
pub use il_data::Rules;
pub use interface::{
    BattleResult, BattleSetup, GeneralFate, GeneralSetup, RegimentResult, RegimentSetup,
    ReinforcementGroup, SOLDIER_CAP, Scenario, ScriptedCommands, SideResult, SideSetup,
    VictoryRules, Weather,
};
pub use map::{FLAT_MAP_ID, LoadedMap, MapError, polygon_contains};
pub use nav::{AStar, NavGrid, PathResult, Pathfinder, string_pull};
pub use resources::{
    AnchorGridRes, BattlePhase, MapRes, NavGridRes, PathRequests, PathfinderRes, SpatialGridRes,
};
pub use schedule::{NoopObserver, Stage, StageObserver};
pub use snapshot::{RestoreError, SNAPSHOT_VERSION, Snapshot};
pub use spatial::{Entry as GridEntry, SpatialGrid};
pub use spawn::SetupError;
pub use view::{BattleView, RegimentRow, SoldierRow};
pub use world::{BattleWorld, StepOutput};
