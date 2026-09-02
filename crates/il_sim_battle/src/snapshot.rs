//! Snapshot and restore (TDD §4.6, SIM-DET-005, REQ-SIM-006).
//!
//! A snapshot holds everything the hash covers plus everything needed to
//! continue. Derived data (spatial grid, nav grid, flow fields, paths, ranks)
//! is never stored; [`BattleWorld::rebuild_derived`] reconstructs it.
//! Content handles are written as `ContentId`s and re-resolved on restore so
//! a registry that changed order does not corrupt a save (SAD §7).

use std::sync::Arc;

use il_core::{Angle, IdAllocator, RegimentId, S, SoldierId, Tick, V2};
use il_data::{ContentId, Registries};
use serde::{Deserialize, Serialize};

use crate::components::{
    Anchor, Body, Facing, FatigueC, Fsm, Health, Morale, MoraleState, Order, OrderKind, Pos,
    PrevFacing, PrevPos, Regiment, SlotRef, Soldier, SoldierState, Vel,
};
use crate::interface::BattleSetup;
use crate::resources::{BattlePhase, Clock, Ids, Phase, Rng, SetupRes, SideState, Sides};
use crate::world::BattleWorld;

/// Bumped whenever the encoding changes; `il_save` migrations key on it.
pub const SNAPSHOT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegimentSnap {
    pub id: RegimentId,
    pub side: u8,
    pub setup_id: u32,
    pub unit_type: ContentId,
    pub anchor_pos: V2,
    pub anchor_facing: Angle<S>,
    pub morale: S,
    pub morale_state: MoraleState,
    pub order: OrderKind,
    pub ammo: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SoldierSnap {
    pub id: SoldierId,
    pub regiment: RegimentId,
    pub p: V2,
    pub v: V2,
    pub facing: Angle<S>,
    pub hp: S,
    pub fatigue: S,
    pub slot: Option<u16>,
    pub fsm_state: SoldierState,
    pub fsm_since: Tick,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdsSnap {
    pub soldiers_next: u32,
    pub regiments_next: u32,
    pub projectiles_next: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub version: u32,
    pub tick: Tick,
    pub phase: BattlePhase,
    pub setup: BattleSetup,
    pub ids: IdsSnap,
    pub rng: Rng,
    pub sides: Vec<SideState>,
    /// Ascending id.
    pub regiments: Vec<RegimentSnap>,
    /// Ascending id.
    pub soldiers: Vec<SoldierSnap>,
    /// Empty until T2-030.
    pub projectiles: Vec<()>,
    /// Empty until T2-031.
    pub pending_damage: Vec<()>,
    /// Battle-flow timer; unused until T2-070.
    pub timer: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RestoreError {
    #[error("snapshot could not be decoded: {0}")]
    Decode(String),
    #[error("snapshot version {found} is not the supported {expected}")]
    VersionMismatch { found: u32, expected: u32 },
    #[error("snapshot refers to unit type {0}, which is not in the registries")]
    UnknownUnitType(ContentId),
    #[error("soldier {soldier} belongs to unknown regiment {regiment}")]
    OrphanSoldier {
        soldier: SoldierId,
        regiment: RegimentId,
    },
    #[error("snapshot is malformed: {0}")]
    Malformed(&'static str),
}

impl Snapshot {
    /// postcard encoding (PRD OQ-2).
    pub fn to_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("snapshot types are postcard-serialisable")
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, RestoreError> {
        let snap: Snapshot =
            postcard::from_bytes(bytes).map_err(|e| RestoreError::Decode(e.to_string()))?;
        if snap.version != SNAPSHOT_VERSION {
            return Err(RestoreError::VersionMismatch {
                found: snap.version,
                expected: SNAPSHOT_VERSION,
            });
        }
        Ok(snap)
    }
}

impl BattleWorld {
    /// Captures the full simulation state. Walks the id-sorted entity lists
    /// so the output is deterministic.
    pub fn snapshot(&self) -> Snapshot {
        let world = &self.world;
        let ids = world.resource::<Ids>();
        let regs = &world.resource::<crate::resources::Regs>().0;

        let regiments = ids
            .regiment_entities
            .iter()
            .map(|(id, entity)| {
                let r = world.get::<Regiment>(*entity).expect("regiment components");
                let anchor = world.get::<Anchor>(*entity).expect("anchor");
                let morale = world.get::<Morale>(*entity).expect("morale");
                let order = world.get::<Order>(*entity).expect("order");
                debug_assert_eq!(*id, r.id);
                RegimentSnap {
                    id: r.id,
                    side: r.side,
                    setup_id: r.setup_id,
                    unit_type: regs.units.id_of(r.unit).clone(),
                    anchor_pos: anchor.pos,
                    anchor_facing: anchor.facing,
                    morale: morale.m,
                    morale_state: morale.state,
                    order: order.kind,
                    ammo: r.ammo,
                }
            })
            .collect();

        let soldiers = ids
            .soldier_entities
            .iter()
            .map(|(id, entity)| {
                let s = world.get::<Soldier>(*entity).expect("soldier components");
                debug_assert_eq!(*id, s.id);
                SoldierSnap {
                    id: s.id,
                    regiment: s.regiment,
                    p: world.get::<Pos>(*entity).expect("pos").p,
                    v: world.get::<Vel>(*entity).expect("vel").v,
                    facing: world.get::<Facing>(*entity).expect("facing").theta,
                    hp: world.get::<Health>(*entity).expect("health").hp,
                    fatigue: world.get::<FatigueC>(*entity).expect("fatigue").f,
                    slot: world.get::<SlotRef>(*entity).expect("slot").slot,
                    fsm_state: world.get::<Fsm>(*entity).expect("fsm").state,
                    fsm_since: world.get::<Fsm>(*entity).expect("fsm").since,
                }
            })
            .collect();

        Snapshot {
            version: SNAPSHOT_VERSION,
            tick: world.resource::<Clock>().tick,
            phase: world.resource::<Phase>().0,
            setup: world
                .resource::<SetupRes>()
                .0
                .clone()
                .unwrap_or_else(empty_setup),
            ids: IdsSnap {
                soldiers_next: ids.soldiers.peek_raw(),
                regiments_next: ids.regiments.peek_raw(),
                projectiles_next: ids.projectiles.peek_raw(),
            },
            rng: world.resource::<Rng>().clone(),
            sides: world.resource::<Sides>().0.clone(),
            regiments,
            soldiers,
            projectiles: Vec::new(),
            pending_damage: Vec::new(),
            timer: 0,
        }
    }

    /// Rebuilds a world from a snapshot against the given registries.
    /// `hash(restore(snapshot(w))) == hash(w)`.
    pub fn restore(snapshot: &Snapshot, regs: Arc<Registries>) -> Result<Self, RestoreError> {
        if snapshot.version != SNAPSHOT_VERSION {
            return Err(RestoreError::VersionMismatch {
                found: snapshot.version,
                expected: SNAPSHOT_VERSION,
            });
        }
        let mut w = BattleWorld::empty(snapshot.rng.seed, regs.clone(), snapshot.phase);
        w.tick = snapshot.tick;
        w.phase = snapshot.phase;
        w.world.resource_mut::<Clock>().tick = snapshot.tick;
        *w.world.resource_mut::<Rng>() = snapshot.rng.clone();
        w.world.resource_mut::<Sides>().0 = snapshot.sides.clone();

        // Regiments first, ascending id; soldiers attach by regiment id.
        let mut regiment_lookup: Vec<(RegimentId, bevy_ecs::entity::Entity)> = Vec::new();
        let mut prev_id: Option<RegimentId> = None;
        for r in &snapshot.regiments {
            if prev_id.is_some_and(|p| p >= r.id) {
                return Err(RestoreError::Malformed(
                    "regiments not in ascending id order",
                ));
            }
            prev_id = Some(r.id);
            let unit = regs
                .units
                .lookup(&r.unit_type)
                .ok_or_else(|| RestoreError::UnknownUnitType(r.unit_type.clone()))?;
            let entity = w
                .world
                .spawn((
                    Regiment {
                        id: r.id,
                        side: r.side,
                        setup_id: r.setup_id,
                        unit,
                        soldiers: Vec::new(),
                        ammo: r.ammo,
                    },
                    Anchor {
                        pos: r.anchor_pos,
                        facing: r.anchor_facing,
                    },
                    Morale {
                        m: r.morale,
                        state: r.morale_state,
                    },
                    Order { kind: r.order },
                ))
                .id();
            w.world
                .resource_mut::<Ids>()
                .regiment_entities
                .push((r.id, entity));
            regiment_lookup.push((r.id, entity));
        }

        let mut prev_id: Option<SoldierId> = None;
        for s in &snapshot.soldiers {
            if prev_id.is_some_and(|p| p >= s.id) {
                return Err(RestoreError::Malformed(
                    "soldiers not in ascending id order",
                ));
            }
            prev_id = Some(s.id);
            let regiment_entity = regiment_lookup
                .binary_search_by_key(&s.regiment, |(id, _)| *id)
                .map(|i| regiment_lookup[i].1)
                .map_err(|_| RestoreError::OrphanSoldier {
                    soldier: s.id,
                    regiment: s.regiment,
                })?;
            let unit = w
                .world
                .get::<Regiment>(regiment_entity)
                .expect("just spawned")
                .unit;
            let (radius, mass, category) = {
                let u = regs.units.get(unit);
                (u.soldier_radius, u.mass, u.category)
            };
            let entity = w
                .world
                .spawn((
                    Soldier {
                        id: s.id,
                        regiment: s.regiment,
                        unit,
                        category,
                    },
                    Pos { p: s.p },
                    // Interpolation state is render-only; start it at the
                    // current position.
                    PrevPos { p: s.p },
                    Vel { v: s.v },
                    Facing { theta: s.facing },
                    PrevFacing { theta: s.facing },
                    Body { r: radius, m: mass },
                    Health { hp: s.hp },
                    FatigueC { f: s.fatigue },
                    SlotRef { slot: s.slot },
                    Fsm {
                        state: s.fsm_state,
                        since: s.fsm_since,
                    },
                ))
                .id();
            w.world
                .resource_mut::<Ids>()
                .soldier_entities
                .push((s.id, entity));
            w.world
                .get_mut::<Regiment>(regiment_entity)
                .expect("just spawned")
                .soldiers
                .push(s.id);
        }

        {
            let mut ids = w.world.resource_mut::<Ids>();
            ids.soldiers = IdAllocator::from_next(snapshot.ids.soldiers_next);
            ids.regiments = IdAllocator::from_next(snapshot.ids.regiments_next);
            ids.projectiles = IdAllocator::from_next(snapshot.ids.projectiles_next);
        }

        w.set_setup(snapshot.setup.clone());
        w.rebuild_derived();
        w.refresh_hash();
        Ok(w)
    }

    /// Reconstructs everything that is derived from hashed state and
    /// therefore not stored in a snapshot (SIM-DET-005). Nothing derived
    /// exists in Phase 0. Phase 1 adds, in this order:
    /// - the spatial grid from positions (T1-031, T1-048),
    /// - the nav grid from the map plus gate states (T1-032),
    /// - flow fields per side (T2-042),
    /// - `Path` components, re-requested rather than stored (T1-048),
    /// - `Rank` from slots (T1-041).
    pub(crate) fn rebuild_derived(&mut self) {}
}

/// Placeholder setup for worlds built with `BattleWorld::empty`.
fn empty_setup() -> BattleSetup {
    BattleSetup {
        map_id: None,
        seed: 0,
        weather: Default::default(),
        time_of_day: 12,
        time_limit_ticks: 48_000,
        reveal_deployment: false,
        sides: Vec::new(),
        victory: Default::default(),
    }
}
