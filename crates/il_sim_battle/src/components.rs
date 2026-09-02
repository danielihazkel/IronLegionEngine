//! ECS components (TDD §4.3, SIM-CORE-004, SIM-CORE-005), Phase 0 subset.
//! Soldier components are stored as tables, one per column, so per-soldier
//! systems stream through memory.

use bevy_ecs::prelude::*;
use il_core::{
    Angle, Hashable, RegimentId, S, SoldierId, StateHasher, Tick, V2, impl_hashable_fieldless_enum,
    impl_hashable_struct,
};
use il_data::{Handle, UnitCategory, UnitType};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- soldiers

#[derive(Component, Clone, Debug)]
pub struct Soldier {
    pub id: SoldierId,
    pub regiment: RegimentId,
    pub unit: Handle<UnitType>,
    pub category: UnitCategory,
}

/// Position at the end of the current tick.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Pos {
    pub p: V2,
}

/// Position at the end of the previous tick (render interpolation only).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct PrevPos {
    pub p: V2,
}

#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Vel {
    pub v: V2,
}

#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Facing {
    pub theta: Angle<S>,
}

/// Facing at the end of the previous tick (render interpolation only).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct PrevFacing {
    pub theta: Angle<S>,
}

/// Collision radius and mass, copied from the unit type (derived, not hashed).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Body {
    pub r: S,
    pub m: S,
}

#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Health {
    pub hp: S,
}

/// Per-soldier fatigue in `[0, 1]`.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct FatigueC {
    pub f: S,
}

/// Index of the formation slot this soldier holds, if any.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct SlotRef {
    pub slot: Option<u16>,
}

/// SIM-CORE-010.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum SoldierState {
    #[default]
    Idle = 0,
    MoveToSlot = 1,
    Fighting = 2,
    Routing = 3,
    Withdrawing = 4,
    Dead = 5,
}
impl_hashable_fieldless_enum!(SoldierState);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Fsm {
    pub state: SoldierState,
    /// Tick the state was entered.
    pub since: Tick,
}

// --------------------------------------------------------------- regiments

#[derive(Component, Clone, Debug)]
pub struct Regiment {
    pub id: RegimentId,
    /// Index into `Sides`; the owner is `Sides[side].player`.
    pub side: u8,
    /// Campaign regiment id echoed into `BattleResult` (`RegimentSetup.id`).
    pub setup_id: u32,
    pub unit: Handle<UnitType>,
    /// Ascending ids of living soldiers.
    pub soldiers: Vec<SoldierId>,
    /// Volleys left across the regiment; `0` for units without `ranged`.
    pub ammo: u16,
}

#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Anchor {
    pub pos: V2,
    pub facing: Angle<S>,
}

/// SIM-MOR-002.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum MoraleState {
    #[default]
    Steady = 0,
    Unsettled = 1,
    Shaken = 2,
    Broken = 3,
    Routing = 4,
    Shattered = 5,
}
impl_hashable_fieldless_enum!(MoraleState);

/// Value and state only in Phase 0; factors and rings arrive in T2-041.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Morale {
    pub m: S,
    pub state: MoraleState,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum OrderKind {
    #[default]
    Idle = 0,
    Move = 1,
    AttackMove = 2,
    AttackRegiment = 3,
    Withdraw = 4,
}
impl_hashable_fieldless_enum!(OrderKind);

/// Current order; target, speed and `since` arrive with movement (T1-042).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Order {
    pub kind: OrderKind,
}

// ------------------------------------------------------------------ hashing

impl_hashable_struct!(Pos { p });
impl_hashable_struct!(Vel { v });
impl_hashable_struct!(Facing { theta });
impl_hashable_struct!(Health { hp });
impl_hashable_struct!(FatigueC { f });
impl_hashable_struct!(SlotRef { slot });
impl_hashable_struct!(Anchor { pos, facing });
impl_hashable_struct!(Morale { m, state });
impl_hashable_struct!(Order { kind });

impl Hashable for Fsm {
    fn hash_state(&self, h: &mut StateHasher) {
        self.state.hash_state(h);
        self.since.hash_state(h);
    }
}
