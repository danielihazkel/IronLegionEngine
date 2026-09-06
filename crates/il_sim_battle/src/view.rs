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
use il_core::{Angle, ProjectileId, RegimentId, S, SoldierId, Tick, V2};
use il_data::{
    Ability, FormationTemplate, Handle, ProjectileArc, Registries, UnitCategory, UnitType,
};

use crate::command::FireMode;
use crate::components::{
    Anchor, Combat, Facing, FatigueC, Fire, FormationState, Fsm, GeneralTag, Health, MeleeState,
    Morale, MoraleState, Order, OrderKind, Path, Pos, PrevFacing, PrevPos, RangedState, Regiment,
    RegimentFatigue, SlotRef, Soldier, SoldierState, Statuses,
};
use crate::components::{Cooldowns, Energy};
use crate::map::LoadedMap;
use crate::nav::NavGrid;
use crate::resources::{
    AnchorGridRes, BattlePhase, Ids, MapRes, NavGridRes, Projectiles, Regs, SideState, Sides,
    SpatialGridRes,
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
    &'static MeleeState,
    Option<&'static RangedState>,
    &'static FatigueC,
    Option<&'static GeneralTag>,
);
type RegimentData = (
    &'static Regiment,
    &'static Anchor,
    &'static Order,
    &'static Morale,
    &'static FormationState,
    &'static Combat,
    Option<&'static Fire>,
    &'static RegimentFatigue,
    &'static Energy,
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
    /// Melee target while `Fighting` (T2-020).
    pub target: Option<SoldierId>,
    /// Volleys left; `None` for units without a `ranged` block (T2-030).
    pub ammo: Option<u16>,
    /// SIM-FAT-001, in `[0, 1]` (T2-040).
    pub fatigue: S,
    /// The general's rank; `None` for everyone else (SIM-GEN-001, T2-043).
    pub general: Option<u8>,
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
    /// SIM-CMBT-003 (T2-020).
    pub engaged: bool,
    /// Fire mode and current ranged target; `None` for units without a
    /// `ranged` block (T2-030).
    pub fire: Option<FireMode>,
    pub fire_target: Option<RegimentId>,
    /// SIM-FAT-005: mean soldier fatigue as of the last ten-tick refresh
    /// (T2-040).
    pub fatigue_mean: S,
    /// Soldiers that fled the field (SIM-MOR-032; written from T2-042).
    pub fled: u16,
    /// Times the regiment routed (SIM-MOR-031; written from T2-042).
    pub rout_count: u8,
    /// Soldiers that left the field withdrawing (SIM-FLOW-014; written from
    /// T2-070).
    pub withdrawn: u16,
    /// SIM-ABIL-006 (T2-050).
    pub energy: S,
}

/// One ability slot of a regiment (T2-050): the ability and the ticks
/// until it may be used again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbilityRow {
    pub ability: Handle<Ability>,
    pub cooldown: u16,
}

/// One active status effect of a regiment (T2-050).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatusRow {
    pub ability: Handle<Ability>,
    pub remaining: u16,
    pub stacks: u8,
    pub hostile: bool,
}

/// One projectile in flight (T2-030); the renderer evaluates the arc
/// itself from these launch values (`Projectile::position_at`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectileRow {
    pub id: ProjectileId,
    pub side: u8,
    pub start: V2,
    pub end: V2,
    pub apex: S,
    pub arc: ProjectileArc,
    pub launch_tick: Tick,
    pub land_tick: Tick,
}

/// Borrowed, read-only view over a `BattleWorld`.
pub struct BattleView<'w> {
    world: &'w World,
    q: &'w ViewQueries,
    tick: Tick,
    phase: BattlePhase,
}

type SoldierItem<'a> = (
    &'a Soldier,
    &'a Pos,
    &'a PrevPos,
    &'a Facing,
    &'a PrevFacing,
    &'a Fsm,
    &'a Health,
    &'a SlotRef,
    &'a MeleeState,
    Option<&'a RangedState>,
    &'a FatigueC,
    Option<&'a GeneralTag>,
);
type RegimentItem<'a> = (
    &'a Regiment,
    &'a Anchor,
    &'a Order,
    &'a Morale,
    &'a FormationState,
    &'a Combat,
    Option<&'a Fire>,
    &'a RegimentFatigue,
    &'a Energy,
);

fn soldier_row(
    (s, pos, prev, facing, prev_facing, fsm, health, slot, melee, ranged, fatigue, general): SoldierItem<
        '_,
    >,
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
        target: melee.target,
        ammo: ranged.map(|r| r.ammo),
        fatigue: fatigue.f,
        general: general.map(|g| g.rank),
    }
}

fn regiment_row(
    (r, anchor, order, morale, formation, combat, fire, fatigue, energy): RegimentItem<'_>,
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
        engaged: combat.engaged,
        fire: fire.map(|f| f.mode),
        fire_target: fire.and_then(|f| f.target),
        fatigue_mean: fatigue.mean,
        fled: combat.fled,
        rout_count: morale.rout_count,
        withdrawn: combat.withdrawn,
        energy: energy.e,
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

    /// The side's escape flow field (SIM-FLOW-001, T2-042).
    pub fn flow_field(&self, side: u8) -> Option<&'w crate::flow::FlowField> {
        self.world
            .resource::<crate::resources::FlowFields>()
            .for_side(side)
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

    /// The regiment's ability slots with their cooldowns (SIM-ABIL-003,
    /// T2-050): its unit's abilities, then its general's while it lives.
    pub fn abilities(&self, id: RegimentId) -> Vec<AbilityRow> {
        let Some(entity) = self.world.resource::<Ids>().regiment_entity(id) else {
            return Vec::new();
        };
        let cooldowns = self.world.get::<Cooldowns>(entity);
        crate::abilities::slots(self.world, entity)
            .into_iter()
            .enumerate()
            .map(|(i, ability)| AbilityRow {
                ability,
                cooldown: cooldowns.and_then(|c| c.0.get(i).copied()).unwrap_or(0),
            })
            .collect()
    }

    /// The regiment's active status effects in application order (T2-050).
    pub fn statuses(&self, id: RegimentId) -> Vec<StatusRow> {
        let Some(entity) = self.world.resource::<Ids>().regiment_entity(id) else {
            return Vec::new();
        };
        self.world
            .get::<Statuses>(entity)
            .map(|s| {
                s.list
                    .iter()
                    .map(|e| StatusRow {
                        ability: e.source,
                        remaining: e.remaining,
                        stacks: e.stacks,
                        hostile: e.hostile,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The regiment's current path.
    pub fn path(&self, id: RegimentId) -> Option<&'w Path> {
        let entity = self.world.resource::<Ids>().regiment_entity(id)?;
        self.world.get::<Path>(entity)
    }

    /// Volleys left in the regiment: the most any of its soldiers still
    /// carries (SIM-FLOW-018 `ammo_left`); `0` for units without `ranged`.
    pub fn ammo(&self, id: RegimentId) -> u16 {
        let ids = self.world.resource::<Ids>();
        let Some(entity) = ids.regiment_entity(id) else {
            return 0;
        };
        let Some(regiment) = self.world.get::<Regiment>(entity) else {
            return 0;
        };
        regiment
            .soldiers
            .iter()
            .filter_map(|sid| ids.soldier_entity(*sid))
            .filter_map(|e| self.world.get::<RangedState>(e))
            .map(|r| r.ammo)
            .max()
            .unwrap_or(0)
    }

    /// Every projectile in flight, ascending id (T2-030).
    pub fn projectiles(&self) -> impl Iterator<Item = ProjectileRow> + 'w {
        self.world
            .resource::<Projectiles>()
            .0
            .iter()
            .map(|p| ProjectileRow {
                id: p.id,
                side: p.side,
                start: p.start,
                end: p.end,
                apex: p.apex,
                arc: p.arc,
                launch_tick: p.launch_tick,
                land_tick: p.land_tick,
            })
    }

    pub fn projectile_count(&self) -> usize {
        self.world.resource::<Projectiles>().0.len()
    }
}
