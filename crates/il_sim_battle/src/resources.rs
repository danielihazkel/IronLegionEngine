//! ECS resources of the battle world (TDD §4.4, Phase 0 subset).

use std::collections::BTreeSet;
use std::sync::Arc;

use bevy_ecs::prelude::*;
use il_core::hash::{Hashable, StateHasher};
use il_core::{
    EventQueue, IdAllocator, PlayerId, ProjectileId, RegimentId, RngStream, SoldierId, StateHash,
    StreamId, Tick, impl_hashable_fieldless_enum,
};
use il_data::{ContentId, Registries};
use serde::{Deserialize, Serialize};

use crate::command::{Command, RejectReason};
use crate::events::BattleEvent;
use crate::interface::BattleSetup;
use crate::map::LoadedMap;
use crate::nav::{AStar, NavGrid};
use crate::spatial::SpatialGrid;

/// The tick being simulated is `tick + 1`; `tick` counts completed ticks.
/// Incremented at the start of `step` so the hash at Stage 17 covers the
/// tick just simulated (TDD §15: commands are gathered for `tick() + 1`).
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct Clock {
    pub tick: Tick,
}

/// SIM-FLOW-010..017.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum BattlePhase {
    Deployment = 0,
    Battle = 1,
    Pursuit = 2,
    Ended = 3,
}
impl_hashable_fieldless_enum!(BattlePhase);

#[derive(Resource, Clone, Copy, Debug)]
pub struct Phase(pub BattlePhase);

/// One seeded stream per system (SIM-DET-001).
#[derive(Resource, Clone, Debug, Serialize, Deserialize)]
pub struct Rng {
    pub seed: u64,
    pub streams: [RngStream; StreamId::COUNT],
}

impl Rng {
    pub fn from_seed(seed: u64) -> Self {
        Self {
            seed,
            streams: StreamId::ALL.map(|id| RngStream::from_seed(seed, id)),
        }
    }

    #[inline]
    pub fn stream(&mut self, id: StreamId) -> &mut RngStream {
        &mut self.streams[id.index()]
    }

    /// Seed for `hash_draw` calls belonging to `id` (SIM-DET-002).
    #[inline]
    pub fn draw_seed(&self, id: StreamId) -> u64 {
        RngStream::draw_seed(self.seed, id)
    }
}

impl Hashable for Rng {
    fn hash_state(&self, h: &mut StateHasher) {
        for s in &self.streams {
            s.hash_state(h);
        }
    }
}

/// Id allocators plus the id-sorted entity lists every order-dependent
/// system iterates (SIM-DET-003). The lists are derived data: rebuilt on
/// restore and maintained by spawn and death.
#[derive(Resource, Debug, Default)]
pub struct Ids {
    pub soldiers: IdAllocator<SoldierId>,
    pub regiments: IdAllocator<RegimentId>,
    pub projectiles: IdAllocator<ProjectileId>,
    /// Ascending by `SoldierId`.
    pub soldier_entities: Vec<(SoldierId, Entity)>,
    /// Ascending by `RegimentId`.
    pub regiment_entities: Vec<(RegimentId, Entity)>,
}

impl Ids {
    pub fn regiment_entity(&self, id: RegimentId) -> Option<Entity> {
        self.regiment_entities
            .binary_search_by_key(&id, |(rid, _)| *rid)
            .ok()
            .map(|i| self.regiment_entities[i].1)
    }

    /// Index of a regiment in `regiment_entities` (the per-tick gate is
    /// indexed the same way).
    pub fn regiment_index(&self, id: RegimentId) -> Option<usize> {
        self.regiment_entities
            .binary_search_by_key(&id, |(rid, _)| *rid)
            .ok()
    }

    pub fn soldier_entity(&self, id: SoldierId) -> Option<Entity> {
        self.soldier_entities
            .binary_search_by_key(&id, |(sid, _)| *sid)
            .ok()
            .map(|i| self.soldier_entities[i].1)
    }
}

/// Per-tick event buffer drained at Stage 17.
#[derive(Resource, Debug, Default)]
pub struct Events(pub EventQueue<BattleEvent>);

/// Stage 9 gate (T2-020, derived per tick): one entry per regiment in
/// `Ids.regiment_entities` order. Soldiers of regiments that cannot fight
/// or have no enemy within reach skip the per-soldier target search.
#[derive(Resource, Debug, Default)]
pub struct MeleeGateRes {
    pub side: Vec<u8>,
    pub may_fight: Vec<bool>,
    pub near_enemy: Vec<bool>,
    /// Farthest soldier from the anchor, metres.
    pub extent: Vec<il_core::S>,
}

/// Content registries; sim code reads by handle only (SAD §3 principle 7).
#[derive(Resource, Clone)]
pub struct Regs(pub Arc<Registries>);

/// The battle terrain (TDD §4.4 `Map`), built from `BattleSetup.map_id`
/// at `new` and `restore`; immutable for the whole battle.
#[derive(Resource, Clone)]
pub struct MapRes(pub Arc<LoadedMap>);

/// Soldier positions at `movement.spatial_cell`, rebuilt at Stage 6
/// (TDD §5). Stage 4 reads the previous tick's build; Stage 7 this tick's.
#[derive(Resource, Clone)]
pub struct SpatialGridRes(pub SpatialGrid<SoldierId>);

/// Regiment anchors at `movement.anchor_cell`, for AI and visibility
/// queries (TDD §5).
#[derive(Resource, Clone)]
pub struct AnchorGridRes(pub SpatialGrid<RegimentId>);

/// The nav grid derived from the map (TDD §6.1); rebuilt on restore and
/// when gates change (Phase 5).
#[derive(Resource, Clone)]
pub struct NavGridRes(pub NavGrid);

/// The path search (`AStar` in Phase 1, HPA* from Phase 3).
#[derive(Resource, Default)]
pub struct PathfinderRes(pub AStar);

/// Regiments waiting for a path, served ascending, `paths_per_tick` per
/// tick (SIM-MOVE-005). Derived: rebuilt from `Path.requested` on restore.
#[derive(Resource, Clone, Debug, Default)]
pub struct PathRequests(pub BTreeSet<RegimentId>);

// Rules are `il_data::Rules`, read through `Regs.0.rules` (T1-023), so a
// registry swap (hot reload) carries new tunables too.

/// One entry per side of `BattleSetup.sides`, index = side number.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SideState {
    /// Owner of every regiment on this side (SIM-CMD-003); changed by
    /// `TransferControl`.
    pub player: PlayerId,
    pub faction: ContentId,
    pub deployment_zone: u8,
    pub deployment_confirmed: bool,
    pub defeated: bool,
}

#[derive(Resource, Clone, Debug, Default)]
pub struct Sides(pub Vec<SideState>);

/// The setup this battle was built from; stored in snapshots so restore can
/// re-resolve content ids (TDD §4.6). `None` only for `BattleWorld::empty`.
#[derive(Resource, Clone, Debug, Default)]
pub struct SetupRes(pub Option<BattleSetup>);

/// Commands handed to `step`, consumed by Stage 0.
#[derive(Resource, Debug, Default)]
pub struct CommandInbox(pub Vec<Command>);

/// Rejections collected during Stage 0, returned in `StepOutput`.
#[derive(Resource, Debug, Default)]
pub struct Rejected(pub Vec<(Command, RejectReason)>);

/// Events drained at Stage 17, returned in `StepOutput`.
#[derive(Resource, Debug, Default)]
pub struct StepEvents(pub Vec<BattleEvent>);

/// Hash computed at Stage 17 of the last step (or at construction).
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct LastHash(pub StateHash);

/// Worker threads the schedule runs with (`1` = single-threaded executor).
#[derive(Resource, Clone, Copy, Debug)]
pub struct ThreadCount(pub usize);
