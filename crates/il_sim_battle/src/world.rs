//! `BattleWorld`: the headless battle simulation (TDD §4.2).

use std::sync::Arc;

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::{MultiThreadedExecutor, SingleThreadedExecutor};
use bevy_tasks::{ComputeTaskPool, TaskPoolBuilder};
use il_core::{Angle, RegimentId, S, Scalar, SoldierId, StateHash, Tick, V2};
use il_data::{ContentId, Registries};

use crate::command::{Command, RejectReason};
use crate::components::{Anchor, Facing, Pos};
use crate::events::BattleEvent;
use crate::hash::compute_hash;
use crate::interface::BattleSetup;
use crate::map::{FLAT_MAP_ID, LoadedMap, MapError};
use crate::nav::NavGrid;
use crate::resources::{
    AnchorGridRes, BattlePhase, Clock, CommandInbox, Events, Ids, LastHash, MapRes, NavGridRes,
    PathRequests, PathfinderRes, Phase, Regs, Rejected, Rng, SetupRes, Sides, SpatialGridRes,
    StepEvents, ThreadCount,
};
use crate::schedule::{NoopObserver, Stage, StageObserver, build_schedules};
use crate::spatial::SpatialGrid;
use crate::view::{BattleView, ViewQueries};

/// Result of one `step`.
#[derive(Clone, Debug)]
pub struct StepOutput {
    /// State hash at the end of the tick (SIM-DET-004).
    pub hash: StateHash,
    /// Events emitted during the tick, in emission order.
    pub events: Vec<BattleEvent>,
    /// Commands Stage 0 refused, with the reason.
    pub rejected: Vec<(Command, RejectReason)>,
}

/// Side of the placeholder map of [`BattleWorld::empty`], metres.
const FLAT_MAP_SIZE: i32 = 1024;

/// Why `install_map` failed; mapped onto `SetupError` and `RestoreError`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum InstallMapError {
    UnknownMap(ContentId),
    Map(MapError),
}

impl From<MapError> for InstallMapError {
    fn from(e: MapError) -> Self {
        InstallMapError::Map(e)
    }
}

pub struct BattleWorld {
    pub(crate) world: World,
    pub(crate) view_queries: ViewQueries,
    /// One schedule per stage, in `Stage::ALL` order.
    pub(crate) schedules: Vec<Schedule>,
    pub(crate) tick: Tick,
    pub(crate) phase: BattlePhase,
}

impl core::fmt::Debug for BattleWorld {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BattleWorld")
            .field("tick", &self.tick)
            .field("phase", &self.phase)
            .field("regiments", &self.regiment_count())
            .field("soldiers", &self.soldier_count())
            .field("hash", &self.hash())
            .finish()
    }
}

impl BattleWorld {
    /// A world with resources but no entities. `BattleWorld::new` (T0-032)
    /// and `restore` (T0-034) build on this; tools use it for micro-tests.
    pub fn empty(seed: u64, regs: Arc<Registries>, phase: BattlePhase) -> Self {
        let mut world = World::new();
        world.insert_resource(Clock::default());
        world.insert_resource(Phase(phase));
        world.insert_resource(Rng::from_seed(seed));
        world.insert_resource(Ids::default());
        world.insert_resource(Events::default());
        world.insert_resource(Regs(regs.clone()));
        world.insert_resource(Sides::default());
        world.insert_resource(CommandInbox::default());
        world.insert_resource(Rejected::default());
        world.insert_resource(StepEvents::default());
        world.insert_resource(LastHash::default());
        world.insert_resource(ThreadCount(1));
        world.insert_resource(SetupRes(None));
        let flat_map = LoadedMap::flat(S::from_i32(FLAT_MAP_SIZE), S::from_i32(FLAT_MAP_SIZE));
        world.insert_resource(NavGridRes(NavGrid::from_map(
            &flat_map,
            &regs,
            &regs.rules.movement,
        )));
        world.insert_resource(PathfinderRes::default());
        world.insert_resource(PathRequests::default());
        world.insert_resource(MapRes(Arc::new(flat_map)));
        // Dimensioned by the Stage 6 system once the map and rules are known.
        let flat = S::from_i32(FLAT_MAP_SIZE);
        world.insert_resource(SpatialGridRes(SpatialGrid::new(flat, flat, S::ZERO)));
        world.insert_resource(AnchorGridRes(SpatialGrid::new(flat, flat, S::ZERO)));
        let view_queries = ViewQueries::new(&mut world);
        let mut w = Self {
            world,
            view_queries,
            schedules: build_schedules(),
            tick: Tick::ZERO,
            phase,
        };
        w.refresh_hash();
        w
    }

    pub(crate) fn set_setup(&mut self, setup: BattleSetup) {
        self.world.resource_mut::<SetupRes>().0 = Some(setup);
    }

    /// Builds the terrain of `map_id` from the registries and installs it
    /// (`new`, `restore`). `FLAT_MAP_ID` keeps the placeholder map.
    pub(crate) fn install_map(&mut self, map_id: &ContentId) -> Result<(), InstallMapError> {
        if map_id.as_str() == FLAT_MAP_ID {
            return Ok(());
        }
        let map = {
            let regs = &self.world.resource::<Regs>().0;
            let handle = regs
                .maps
                .lookup(map_id)
                .ok_or_else(|| InstallMapError::UnknownMap(map_id.clone()))?;
            LoadedMap::from_def(regs.maps.get(handle), regs.rules.movement.zone_cell)?
        };
        self.world.resource_mut::<MapRes>().0 = Arc::new(map);
        Ok(())
    }

    /// The battle terrain.
    pub fn map(&self) -> &Arc<LoadedMap> {
        &self.world.resource::<MapRes>().0
    }

    /// The nav grid derived from the map.
    pub fn nav_grid(&self) -> &NavGrid {
        &self.world.resource::<NavGridRes>().0
    }

    /// The setup this world was built from, if any.
    pub fn setup(&self) -> Option<&BattleSetup> {
        self.world.resource::<SetupRes>().0.as_ref()
    }

    /// Living soldier ids, ascending.
    pub fn soldier_ids(&self) -> impl Iterator<Item = SoldierId> + '_ {
        self.world
            .resource::<Ids>()
            .soldier_entities
            .iter()
            .map(|(id, _)| *id)
    }

    /// Regiment ids, ascending.
    pub fn regiment_ids(&self) -> impl Iterator<Item = RegimentId> + '_ {
        self.world
            .resource::<Ids>()
            .regiment_entities
            .iter()
            .map(|(id, _)| *id)
    }

    pub fn soldier_count(&self) -> usize {
        self.world.resource::<Ids>().soldier_entities.len()
    }

    pub fn regiment_count(&self) -> usize {
        self.world.resource::<Ids>().regiment_entities.len()
    }

    /// Recomputes `LastHash` from the current state (construction, restore)
    /// and refreshes the view queries, since every caller has just changed
    /// the entity set or component values.
    pub(crate) fn refresh_hash(&mut self) {
        self.view_queries.refresh(&self.world);
        let hash = compute_hash(&mut self.world);
        self.world.resource_mut::<LastHash>().0 = hash;
    }

    /// Runs exactly one tick. Commands must be stamped with `tick() + 1`
    /// (the tick this call simulates) or they are rejected as stale.
    pub fn step(&mut self, commands: &[Command]) -> StepOutput {
        self.step_observed(commands, &mut NoopObserver)
    }

    /// [`step`](Self::step) with a callback around every stage, for the
    /// profiler and the benches. The observer never influences the sim.
    pub fn step_observed(
        &mut self,
        commands: &[Command],
        observer: &mut dyn StageObserver,
    ) -> StepOutput {
        self.tick = self.tick.next();
        self.world.resource_mut::<Clock>().tick = self.tick;
        self.world.resource_mut::<CommandInbox>().0 = commands.to_vec();

        for (schedule, stage) in self.schedules.iter_mut().zip(Stage::ALL) {
            observer.begin(stage);
            schedule.run(&mut self.world);
            observer.end(stage);
        }
        self.view_queries.refresh(&self.world);

        self.phase = self.world.resource::<Phase>().0;
        StepOutput {
            hash: self.world.resource::<LastHash>().0,
            events: core::mem::take(&mut self.world.resource_mut::<StepEvents>().0),
            rejected: core::mem::take(&mut self.world.resource_mut::<Rejected>().0),
        }
    }

    /// Completed ticks; the next `step` simulates `tick() + 1`.
    pub fn tick(&self) -> Tick {
        self.tick
    }

    pub fn phase(&self) -> BattlePhase {
        self.phase
    }

    /// The hash of the current state; equals `StepOutput.hash` of the last
    /// step, or the hash of the initial state before any step.
    pub fn hash(&self) -> StateHash {
        self.world.resource::<LastHash>().0
    }

    /// Chooses the executor: `n <= 1` runs the schedule single-threaded;
    /// larger `n` uses the multi-threaded executor on the process-global
    /// `ComputeTaskPool`, whose size is fixed by the first such call in the
    /// process. The determinism test runs 1 and N (REQ-SIM-008).
    pub fn set_threads(&mut self, n: usize) {
        if n <= 1 {
            for s in &mut self.schedules {
                s.set_executor(SingleThreadedExecutor::new());
            }
            self.world.resource_mut::<ThreadCount>().0 = 1;
        } else {
            ComputeTaskPool::get_or_init(|| TaskPoolBuilder::new().num_threads(n).build());
            for s in &mut self.schedules {
                s.set_executor(MultiThreadedExecutor::new());
            }
            self.world.resource_mut::<ThreadCount>().0 = n;
        }
    }

    pub fn threads(&self) -> usize {
        self.world.resource::<ThreadCount>().0
    }

    /// Read-only view for render, UI and AI (TDD §4.2). Cheap to build; the
    /// query states behind it are cached and refreshed by `step`.
    pub fn view(&self) -> BattleView<'_> {
        BattleView::new(&self.world, &self.view_queries, self.tick, self.phase)
    }

    /// Swaps in hot-reloaded registries between ticks (T1-025). The new
    /// layout must extend the old one (every old id at its old index), which
    /// `il_data::hot_reload` guarantees, so handles held by entities stay
    /// valid; values copied at spawn (`Body`) are not refreshed.
    pub fn replace_registries(&mut self, regs: Arc<Registries>) {
        debug_assert!(
            {
                let old = &self.world.resource::<Regs>().0;
                old.units
                    .all_ids()
                    .zip(regs.units.all_ids())
                    .all(|(a, b)| a == b)
                    && old.units.slots() <= regs.units.slots()
            },
            "hot-reloaded registries must keep the old layout"
        );
        self.world.resource_mut::<Regs>().0 = regs;
    }

    /// The registries the world reads.
    pub fn registries(&self) -> &Arc<Registries> {
        &self.world.resource::<Regs>().0
    }

    /// Read-only ECS access for tests and tools; presentation code uses
    /// [`view`](Self::view).
    pub fn ecs(&self) -> &World {
        &self.world
    }

    /// Tools only (like [`ecs_mut`](Self::ecs_mut)): moves every soldier and
    /// regiment anchor by `delta` and sets every facing. Drives
    /// `il_app --demo-circle` until movement exists (T1-043); removed then.
    pub fn debug_translate_all(&mut self, delta: V2, facing: Option<Angle<S>>) {
        for (mut pos, mut f) in self
            .world
            .query::<(&mut Pos, &mut Facing)>()
            .iter_mut(&mut self.world)
        {
            pos.p += delta;
            if let Some(theta) = facing {
                f.theta = theta;
            }
        }
        for mut anchor in self.world.query::<&mut Anchor>().iter_mut(&mut self.world) {
            anchor.pos += delta;
            if let Some(theta) = facing {
                anchor.facing = theta;
            }
        }
        self.refresh_hash();
    }

    /// Mutable access for tests and tools only. Gameplay never mutates the
    /// world from outside `step` (REQ-SIM-003); call [`recompute_hash`]
    /// after using this so `hash()` is honest again.
    ///
    /// [`recompute_hash`]: Self::recompute_hash
    pub fn ecs_mut(&mut self) -> &mut World {
        &mut self.world
    }

    /// Recomputes and returns the hash of the current state.
    pub fn recompute_hash(&mut self) -> StateHash {
        self.refresh_hash();
        self.hash()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use il_core::StateHash;

    fn empty_world() -> BattleWorld {
        BattleWorld::empty(42, Arc::new(Registries::default()), BattlePhase::Battle)
    }

    /// Golden: the hash of an empty world at seed 42 after 0, 1 and 2 ticks.
    /// Changes whenever the hash layout or the RNG seeding changes.
    const GOLDEN: [u64; 3] = [
        0x4226_7c65_56cc_0c67,
        0x62a8_cc14_a409_afc1,
        0xf654_3cbc_0c77_47e7,
    ];

    #[test]
    fn step_advances_tick_and_hash_changes_with_tick() {
        let mut w = empty_world();
        assert_eq!(w.tick(), Tick::ZERO);
        let h0 = w.hash();
        let out1 = w.step(&[]);
        assert_eq!(w.tick(), Tick(1));
        assert_eq!(out1.hash, w.hash());
        assert!(out1.events.is_empty());
        assert!(out1.rejected.is_empty());
        let out2 = w.step(&[]);
        assert_eq!(w.tick(), Tick(2));
        assert_ne!(h0, out1.hash);
        assert_ne!(out1.hash, out2.hash);
        let got = [h0.0, out1.hash.0, out2.hash.0];
        assert_eq!(
            got, GOLDEN,
            "golden mismatch; actual: [0x{:016x}, 0x{:016x}, 0x{:016x}]",
            got[0], got[1], got[2]
        );
    }

    #[test]
    fn same_seed_same_hashes_across_thread_counts() {
        let mut a = empty_world();
        let mut b = empty_world();
        b.set_threads(4);
        assert_eq!(a.threads(), 1);
        assert_eq!(b.threads(), 4);
        for _ in 0..50 {
            assert_eq!(a.step(&[]).hash, b.step(&[]).hash);
        }
        let mut c = BattleWorld::empty(43, Arc::new(Registries::default()), BattlePhase::Battle);
        assert_ne!(c.step(&[]).hash, StateHash(GOLDEN[1]));
    }

    #[test]
    fn step_observed_visits_every_stage_in_order_and_matches_step() {
        struct Recorder(Vec<(bool, Stage)>);
        impl StageObserver for Recorder {
            fn begin(&mut self, stage: Stage) {
                self.0.push((true, stage));
            }
            fn end(&mut self, stage: Stage) {
                self.0.push((false, stage));
            }
        }
        let mut a = empty_world();
        let mut b = empty_world();
        let mut rec = Recorder(Vec::new());
        let ha = a.step_observed(&[], &mut rec).hash;
        let hb = b.step(&[]).hash;
        assert_eq!(ha, hb);
        let expected: Vec<(bool, Stage)> = Stage::ALL
            .iter()
            .flat_map(|s| [(true, *s), (false, *s)])
            .collect();
        assert_eq!(rec.0, expected);
    }

    #[test]
    fn schedule_has_eighteen_stages_in_order() {
        assert_eq!(crate::Stage::ALL.len(), 18);
        assert_eq!(crate::Stage::ALL[0], crate::Stage::ApplyCommands);
        assert_eq!(crate::Stage::ALL[17], crate::Stage::EventsAndHash);
    }
}
