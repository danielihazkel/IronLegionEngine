//! `BattleWorld`: the headless battle simulation (TDD §4.2).

use std::sync::Arc;

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::{MultiThreadedExecutor, SingleThreadedExecutor};
use bevy_tasks::{ComputeTaskPool, TaskPoolBuilder};
use il_core::{StateHash, Tick};
use il_data::Registries;

use crate::command::{Command, RejectReason};
use crate::events::BattleEvent;
use crate::hash::compute_hash;
use crate::resources::{
    BattlePhase, Clock, CommandInbox, Events, Ids, LastHash, Phase, Regs, Rejected, Rng, Rules,
    RulesRes, Sides, StepEvents, ThreadCount,
};
use crate::schedule::build_schedule;

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

pub struct BattleWorld {
    pub(crate) world: World,
    pub(crate) schedule: Schedule,
    tick: Tick,
    phase: BattlePhase,
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
        world.insert_resource(Regs(regs));
        world.insert_resource(RulesRes(Arc::new(Rules::default())));
        world.insert_resource(Sides::default());
        world.insert_resource(CommandInbox::default());
        world.insert_resource(Rejected::default());
        world.insert_resource(StepEvents::default());
        world.insert_resource(LastHash::default());
        world.insert_resource(ThreadCount(1));
        let mut w = Self {
            world,
            schedule: build_schedule(),
            tick: Tick::ZERO,
            phase,
        };
        w.refresh_hash();
        w
    }

    /// Recomputes `LastHash` from the current state (construction, restore).
    pub(crate) fn refresh_hash(&mut self) {
        let hash = compute_hash(&mut self.world);
        self.world.resource_mut::<LastHash>().0 = hash;
    }

    /// Runs exactly one tick. Commands must be stamped with `tick() + 1`
    /// (the tick this call simulates) or they are rejected as stale.
    pub fn step(&mut self, commands: &[Command]) -> StepOutput {
        self.tick = self.tick.next();
        self.world.resource_mut::<Clock>().tick = self.tick;
        self.world.resource_mut::<CommandInbox>().0 = commands.to_vec();

        self.schedule.run(&mut self.world);

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
            self.schedule.set_executor(SingleThreadedExecutor::new());
            self.world.resource_mut::<ThreadCount>().0 = 1;
        } else {
            ComputeTaskPool::get_or_init(|| TaskPoolBuilder::new().num_threads(n).build());
            self.schedule.set_executor(MultiThreadedExecutor::new());
            self.world.resource_mut::<ThreadCount>().0 = n;
        }
    }

    pub fn threads(&self) -> usize {
        self.world.resource::<ThreadCount>().0
    }

    /// Read-only access for tests, tools and (Phase 1) the renderer.
    pub fn ecs(&self) -> &World {
        &self.world
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
    fn schedule_has_eighteen_stages_in_order() {
        assert_eq!(crate::Stage::ALL.len(), 18);
        assert_eq!(crate::Stage::ALL[0], crate::Stage::ApplyCommands);
        assert_eq!(crate::Stage::ALL[17], crate::Stage::EventsAndHash);
    }
}
