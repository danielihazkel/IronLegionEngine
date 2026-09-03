//! `BattleView`: read-only access to the battle for render, UI and AI
//! (TDD §4.2, SAD §5.2: presentation crates receive `&BattleWorld` and never
//! hold `&mut`).
//!
//! Query states are cached in `BattleWorld` and refreshed after every
//! structural change (`step`, spawn, restore, `recompute_hash`), so building
//! a view is free and iteration streams the component tables.

use std::sync::Arc;

use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryState;
use il_core::{Angle, RegimentId, S, SoldierId, Tick, V2};
use il_data::{FormationTemplate, Handle, Registries, UnitCategory, UnitType};

use crate::components::{
    Anchor, Facing, FormationState, Fsm, Health, Morale, MoraleState, Order, OrderKind, Path, Pos,
    PrevFacing, PrevPos, Regiment, SlotRef, Soldier, SoldierState,
};
use crate::map::LoadedMap;
use crate::nav::NavGrid;
use crate::resources::{
    AnchorGridRes, BattlePhase, Ids, MapRes, NavGridRes, Regs, SideState, Sides, SpatialGridRes,
};
use crate::spatial::SpatialGrid;

type SoldierData = (
    &'static Soldier,
    &'static Pos,
    &'static PrevPos,
    &'static Facing,
    &'static PrevFacing,
    &'static Fsm,
    &'static Health,
    &'static SlotRef,
);
type RegimentData = (
    &'static Regiment,
    &'static Anchor,
    &'static Order,
    &'static Morale,
    &'static FormationState,
);

/// Cached query states behind every `BattleView`.
pub(crate) struct ViewQueries {
    soldier: QueryState<SoldierData>,
    regiment: QueryState<RegimentData>,
}

impl ViewQueries {
    pub(crate) fn new(world: &mut World) -> Self {
        Self {
            soldier: QueryState::new(world),
            regiment: QueryState::new(world),
        }
    }

    /// Picks up new archetypes; call after anything that spawns or despawns.
    pub(crate) fn refresh(&mut self, world: &World) {
        self.soldier.update_archetypes(world);
        self.regiment.update_archetypes(world);
    }
}

/// One soldier as the presentation layer sees it. Plain copies, no borrows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoldierRow {
    pub id: SoldierId,
    pub regiment: RegimentId,
    pub unit: Handle<UnitType>,
    pub category: UnitCategory,
    pub pos: V2,
    pub prev_pos: V2,
    pub facing: Angle<S>,
    pub prev_facing: Angle<S>,
    pub state: SoldierState,
    pub hp: S,
    pub slot: Option<u16>,
}

/// One regiment as the presentation layer sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegimentRow {
    pub id: RegimentId,
    pub side: u8,
    pub unit: Handle<UnitType>,
    pub anchor_pos: V2,
    pub anchor_facing: Angle<S>,
    pub order: OrderKind,
    pub morale: S,
    pub morale_state: MoraleState,
    pub soldier_count: u32,
    /// SIM-FORM-030, as of the last `integrity_period_ticks` boundary.
    pub integrity: S,
    pub formation: Handle<FormationTemplate>,
    pub ranks: u8,
    pub files: u16,
}

/// Borrowed, read-only view over a `BattleWorld`.
pub struct BattleView<'w> {
    world: &'w World,
    q: &'w ViewQueries,
    tick: Tick,
    phase: BattlePhase,
}

fn soldier_row(
    (s, pos, prev, facing, prev_facing, fsm, health, slot): (
        &Soldier,
        &Pos,
        &PrevPos,
        &Facing,
        &PrevFacing,
        &Fsm,
        &Health,
        &SlotRef,
    ),
) -> SoldierRow {
    SoldierRow {
        id: s.id,
        regiment: s.regiment,
        unit: s.unit,
        category: s.category,
        pos: pos.p,
        prev_pos: prev.p,
        facing: facing.theta,
        prev_facing: prev_facing.theta,
        state: fsm.state,
        hp: health.hp,
        slot: slot.slot,
    }
}

fn regiment_row(
    (r, anchor, order, morale, formation): (&Regiment, &Anchor, &Order, &Morale, &FormationState),
) -> RegimentRow {
    RegimentRow {
        id: r.id,
        side: r.side,
        unit: r.unit,
        anchor_pos: anchor.pos,
        anchor_facing: anchor.facing,
        order: order.kind,
        morale: morale.m,
        morale_state: morale.state,
        soldier_count: r.soldiers.len() as u32,
        integrity: formation.integrity,
        formation: formation.template,
        ranks: formation.ranks,
        files: formation.files,
    }
}

impl<'w> BattleView<'w> {
    pub(crate) fn new(
        world: &'w World,
        q: &'w ViewQueries,
        tick: Tick,
        phase: BattlePhase,
    ) -> Self {
        Self {
            world,
            q,
            tick,
            phase,
        }
    }

    /// Completed ticks.
    pub fn tick(&self) -> Tick {
        self.tick
    }

    pub fn phase(&self) -> BattlePhase {
        self.phase
    }

    pub fn regs(&self) -> &'w Arc<Registries> {
        &self.world.resource::<Regs>().0
    }

    /// The battle terrain.
    pub fn map(&self) -> &'w LoadedMap {
        &self.world.resource::<MapRes>().0
    }

    /// The nav grid derived from the map.
    pub fn nav_grid(&self) -> &'w NavGrid {
        &self.world.resource::<NavGridRes>().0
    }

    /// Soldier grid as rebuilt at Stage 6 of the last completed tick.
    pub fn spatial_grid(&self) -> &'w SpatialGrid<SoldierId> {
        &self.world.resource::<SpatialGridRes>().0
    }

    /// Regiment anchor grid as rebuilt at Stage 6 of the last completed tick.
    pub fn anchor_grid(&self) -> &'w SpatialGrid<RegimentId> {
        &self.world.resource::<AnchorGridRes>().0
    }

    /// Sides by index; `sides()[regiment.side]` gives the owning player.
    pub fn sides(&self) -> &'w [SideState] {
        &self.world.resource::<Sides>().0
    }

    pub fn soldier_count(&self) -> usize {
        self.world.resource::<Ids>().soldier_entities.len()
    }

    pub fn regiment_count(&self) -> usize {
        self.world.resource::<Ids>().regiment_entities.len()
    }

    /// Every soldier in table order: the fastest iteration, for the render
    /// snapshot. Order is not part of any contract.
    pub fn soldiers_unordered(&self) -> impl Iterator<Item = SoldierRow> + 'w {
        self.q.soldier.iter_manual(self.world).map(soldier_row)
    }

    /// Every soldier in ascending `SoldierId` order.
    pub fn soldiers(&self) -> impl Iterator<Item = SoldierRow> + 'w {
        let ids = &self.world.resource::<Ids>().soldier_entities;
        self.q
            .soldier
            .iter_many_manual(self.world, ids.iter().map(|(_, e)| *e))
            .map(soldier_row)
    }

    pub fn soldier(&self, id: SoldierId) -> Option<SoldierRow> {
        let entity = self.world.resource::<Ids>().soldier_entity(id)?;
        self.q
            .soldier
            .get_manual(self.world, entity)
            .ok()
            .map(soldier_row)
    }

    /// Every regiment in ascending `RegimentId` order.
    pub fn regiments(&self) -> impl Iterator<Item = RegimentRow> + 'w {
        let ids = &self.world.resource::<Ids>().regiment_entities;
        self.q
            .regiment
            .iter_many_manual(self.world, ids.iter().map(|(_, e)| *e))
            .map(regiment_row)
    }

    pub fn regiment(&self, id: RegimentId) -> Option<RegimentRow> {
        let entity = self.world.resource::<Ids>().regiment_entity(id)?;
        self.q
            .regiment
            .get_manual(self.world, entity)
            .ok()
            .map(regiment_row)
    }

    /// The regiment's formation state (slots are local offsets; see
    /// `formation::slot_world`).
    pub fn formation_state(&self, id: RegimentId) -> Option<&'w FormationState> {
        let entity = self.world.resource::<Ids>().regiment_entity(id)?;
        self.world.get::<FormationState>(entity)
    }

    /// The regiment's current path.
    pub fn path(&self, id: RegimentId) -> Option<&'w Path> {
        let entity = self.world.resource::<Ids>().regiment_entity(id)?;
        self.world.get::<Path>(entity)
    }
}
