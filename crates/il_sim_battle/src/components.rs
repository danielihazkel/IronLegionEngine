//! ECS components (TDD §4.3, SIM-CORE-004, SIM-CORE-005), Phase 0 subset.
//! Soldier components are stored as tables, one per column, so per-soldier
//! systems stream through memory.

use bevy_ecs::prelude::*;
use il_core::{
    Angle, Hashable, RegimentId, S, Scalar, SoldierId, StateHasher, TICKS_PER_SECOND, Tick, V2,
    impl_hashable_fieldless_enum, impl_hashable_struct,
};
use il_data::{FormationTemplate, Handle, UnitCategory, UnitType};
use serde::{Deserialize, Serialize};

use crate::command::SpeedMode;
use crate::formation::Slot;

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

/// Collision radius and mass, copied from the unit type (derived, not
/// hashed). `m` is `unit.mass × combat.charge_mass_mult` while the
/// regiment's charge window is open (SIM-CMBT-015, T2-021).
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

/// Rank and file of the soldier's slot (derived from `SlotRef`, not hashed;
/// `file` is `u16` per the Phase 1 plan).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rank {
    pub rank: u8,
    pub file: u16,
}

/// SIM-CMBT-002 / SIM-CMBT-010: the soldier's melee target and the ticks
/// left until its next attack (hashed, snapshotted; T2-020).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct MeleeState {
    pub target: Option<SoldierId>,
    pub cooldown: u16,
}

/// Soldiers currently targeting this one (SIM-CMBT-002 tie-break).
/// Derived: recounted at Stage 9 and on restore, not hashed.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attackers {
    pub n: u8,
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

/// Regiment combat state (T2-020; SIM-CMBT-003, SIM-CMBT-015, SIM-CMBT-017).
/// Hashed and snapshotted.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Combat {
    /// Any soldier is `Fighting` (SIM-CMBT-003), as of the last Stage 9.
    pub engaged: bool,
    /// Last tick `engaged` was true; pursuit re-paths once this is
    /// `retarget_period_ticks` old (plan decision 7).
    pub last_fighting: Tick,
    /// End of the charge window (SIM-CMBT-015); `Tick::ZERO` when none.
    pub charge_until: Tick,
    /// SIM-CMBT-017, `0..=9`.
    pub experience: u8,
    /// Kills credited to this regiment (T2-022).
    pub kills: u32,
}

/// The regiment's formation (SIM-CORE-005, TDD §7). `slots` and
/// `assignment` are derived (recomputed by `formation_layout` and on
/// restore); the rest is state.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct FormationState {
    pub template: Handle<FormationTemplate>,
    pub ranks: u8,
    /// Width in files of the widest rank of the current layout.
    pub files: u16,
    /// Local slot offsets of the current layout (derived).
    pub slots: Vec<Slot>,
    /// Slot per entry of `Regiment.soldiers` (derived scratch).
    pub assignment: Vec<Option<u16>>,
    /// SIM-FORM-030, recomputed every `integrity_period_ticks` (T1-045).
    pub integrity: S,
    /// End of the current morph (SIM-FORM-032); `Tick::ZERO` when none.
    pub morph_until: Tick,
    /// Set by anything SIM-FORM-020 lists; consumed at Stage 2.
    pub needs_reform: bool,
    /// Template to return to after an automatic corridor morph (SIM-MOVE-004).
    pub prior_template: Option<Handle<FormationTemplate>>,
    /// Anchor facing at the last layout, for the `reform_angle` trigger.
    pub laid_out_facing: Angle<S>,
    /// Transient inside Stage 2: a fresh assignment awaits `formation_apply`.
    pub dirty: bool,
}

impl FormationState {
    /// A freshly laid-out state whose `assignment` maps soldier `k` to slot `k`.
    pub fn new(
        template: Handle<FormationTemplate>,
        ranks: u8,
        slots: Vec<Slot>,
        facing: Angle<S>,
    ) -> Self {
        let files = crate::formation::files_used(&slots);
        let assignment = (0..slots.len()).map(|k| Some(k as u16)).collect();
        Self {
            template,
            ranks,
            files,
            slots,
            assignment,
            integrity: S::ONE,
            morph_until: Tick::ZERO,
            needs_reform: false,
            prior_template: None,
            laid_out_facing: facing,
            dirty: false,
        }
    }
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

/// Ticks the casualty ring covers (SIM-MOR-010: five seconds).
pub const DEATHS_RING: usize = 5 * TICKS_PER_SECOND as usize;

/// SIM-MOR-001..004. The casualty ring and initial strength are written
/// by death (T2-022); `rout_count` and `engaged_since` arrive with T2-041.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Morale {
    pub m: S,
    pub state: MoraleState,
    /// Deaths per tick over the last five seconds, indexed `tick % DEATHS_RING`.
    pub deaths_5s: [u16; DEATHS_RING],
    /// Soldiers at spawn (SIM-MOR-011, SIM-FLOW-018).
    pub initial: u16,
}

impl Morale {
    pub fn new(m: S, initial: u16) -> Self {
        Self {
            m,
            state: MoraleState::Steady,
            deaths_5s: [0; DEATHS_RING],
            initial,
        }
    }
}

impl Default for Morale {
    fn default() -> Self {
        Self::new(S::ZERO, 0)
    }
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

impl OrderKind {
    /// Orders that take the regiment somewhere (need a path).
    pub fn moves(self) -> bool {
        matches!(
            self,
            OrderKind::Move | OrderKind::AttackMove | OrderKind::AttackRegiment
        )
    }

    /// Orders that chase a regiment (SIM-CMBT-004).
    pub fn is_attack(self) -> bool {
        matches!(self, OrderKind::AttackMove | OrderKind::AttackRegiment)
    }
}

/// The regiment's current order (SIM-CORE-005). `target` and `facing` are
/// read by the movement systems; `since` is the tick it was issued.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Order {
    pub kind: OrderKind,
    pub target: V2,
    /// The regiment an attack order chases (SIM-CMBT-004/005); `None` for
    /// every other kind and for an `AttackMove` that has not acquired one.
    pub target_regiment: Option<RegimentId>,
    /// Facing to take on arrival, if the order gave one (SIM-MOVE-013).
    pub facing: Option<Angle<S>>,
    pub speed: SpeedMode,
    pub since: Tick,
}

/// One point of a regiment path with the passable corridor width through
/// its nav cell (SIM-MOVE-004: a regiment wider than `corridor` morphs to
/// Column for it).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Waypoint {
    pub p: V2,
    pub corridor: S,
}

/// The string-pulled route of a regiment (SIM-MOVE-002). `waypoints[0]` is
/// the anchor position at request time; `next` indexes the waypoint the
/// anchor moves toward; `requested` marks a pending `PathRequests` entry.
/// Stored and snapshotted rather than re-requested, so a restored run
/// follows the same route (SIM-DET-005).
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Path {
    pub waypoints: Vec<Waypoint>,
    pub next: u16,
    pub requested: bool,
}

impl Path {
    /// Whether there is a waypoint left to reach.
    pub fn is_active(&self) -> bool {
        usize::from(self.next) < self.waypoints.len()
    }

    pub fn current(&self) -> Option<&Waypoint> {
        self.waypoints.get(usize::from(self.next))
    }
}

// ------------------------------------------------------------------ hashing

impl_hashable_struct!(Pos { p });
impl_hashable_struct!(Vel { v });
impl_hashable_struct!(Facing { theta });
impl_hashable_struct!(Health { hp });
impl_hashable_struct!(FatigueC { f });
impl_hashable_struct!(SlotRef { slot });
impl_hashable_struct!(Anchor { pos, facing });
impl_hashable_struct!(Morale {
    m,
    state,
    deaths_5s,
    initial
});
impl_hashable_struct!(MeleeState { target, cooldown });
impl_hashable_struct!(Combat {
    engaged,
    last_fighting,
    charge_until,
    experience,
    kills
});
impl_hashable_struct!(Order {
    kind,
    target,
    target_regiment,
    facing,
    speed,
    since
});
impl_hashable_struct!(Waypoint { p, corridor });
impl_hashable_struct!(Path {
    waypoints,
    next,
    requested
});

impl Hashable for FormationState {
    /// The state fields only: `slots`, `assignment` and `dirty` are derived.
    fn hash_state(&self, h: &mut StateHasher) {
        self.template.hash_state(h);
        self.ranks.hash_state(h);
        self.files.hash_state(h);
        self.integrity.hash_state(h);
        self.morph_until.hash_state(h);
        self.needs_reform.hash_state(h);
        self.prior_template.hash_state(h);
        self.laid_out_facing.hash_state(h);
    }
}

impl Hashable for Fsm {
    fn hash_state(&self, h: &mut StateHasher) {
        self.state.hash_state(h);
        self.since.hash_state(h);
    }
}
