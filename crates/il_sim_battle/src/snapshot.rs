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

use crate::command::{FireMode, SpeedMode};
use crate::components::{
    Anchor, Attackers, Body, Combat, DEATHS_RING, Facing, FatigueC, Fire, FormationState, Fsm,
    Health, MeleeState, Morale, MoraleState, Order, OrderKind, Path, Pos, PrevFacing, PrevPos,
    RangedState, Rank, Regiment, SlotRef, Soldier, SoldierState, Vel, Waypoint,
};
use crate::interface::BattleSetup;
use crate::map::{FLAT_MAP_ID, MapError};
use crate::resources::{
    BattlePhase, Clock, Ids, Pending, PendingDamage, Phase, Projectile, Projectiles, Rng, SetupRes,
    SideState, Sides,
};
use crate::world::{BattleWorld, InstallMapError};

/// Bumped whenever the encoding changes; `il_save` migrations key on it.
/// 2: `map_id` required in the setup (T1-030).
/// 3: combat state (T2-020): order target regiment, regiment `Combat`,
///    morale casualty ring and initial strength, soldier `MeleeState`.
/// 4: `files` stored (T2-022): it is hashed state that the live world only
///    refreshes at Stage 2, so a snapshot taken after deaths must carry it.
/// 5: ranged state (T2-030): regiment `ammo` replaced by an optional
///    `Fire`, soldiers carry an optional `RangedState`, the projectile list
///    and the pending damage queue are stored.
pub const SNAPSHOT_VERSION: u32 = 5;

/// A ranged regiment's `Fire` component.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FireSnap {
    pub mode: FireMode,
    pub target: Option<RegimentId>,
    pub cooldown: u16,
}

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
    pub order_target: V2,
    pub order_facing: Option<Angle<S>>,
    pub order_speed: SpeedMode,
    pub order_since: Tick,
    /// The stored route (SIM-DET-005 as amended in T1-032).
    pub path: Vec<Waypoint>,
    pub path_next: u16,
    pub path_requested: bool,
    pub formation: ContentId,
    pub ranks: u8,
    pub files: u16,
    pub integrity: S,
    pub morph_until: Tick,
    pub needs_reform: bool,
    pub prior_formation: Option<ContentId>,
    pub laid_out_facing: Angle<S>,
    /// Present exactly when the unit has a `ranged` block (T2-030).
    pub fire: Option<FireSnap>,
    pub order_target_regiment: Option<RegimentId>,
    pub engaged: bool,
    pub last_fighting: Tick,
    pub charge_until: Tick,
    pub experience: u8,
    pub kills: u32,
    /// `DEATHS_RING` entries.
    pub deaths_5s: Vec<u16>,
    pub initial: u16,
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
    pub melee_target: Option<SoldierId>,
    pub melee_cooldown: u16,
    /// `(ammo, cooldown)`, present exactly when the unit has a `ranged`
    /// block (T2-030).
    pub ranged: Option<(u16, u16)>,
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
    /// Ascending id (T2-030).
    pub projectiles: Vec<Projectile>,
    /// Queue order (T2-030; applied from T2-031).
    pub pending_damage: Vec<Pending>,
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
    #[error("snapshot refers to map {0}, which is not in the registries")]
    UnknownMap(ContentId),
    #[error("snapshot refers to formation {0}, which is not in the registries")]
    UnknownFormation(ContentId),
    #[error("{0}")]
    Map(MapError),
    #[error("soldier {soldier} belongs to unknown regiment {regiment}")]
    OrphanSoldier {
        soldier: SoldierId,
        regiment: RegimentId,
    },
    #[error("snapshot is malformed: {0}")]
    Malformed(&'static str),
}

impl From<InstallMapError> for RestoreError {
    fn from(e: InstallMapError) -> Self {
        match e {
            InstallMapError::UnknownMap(id) => RestoreError::UnknownMap(id),
            InstallMapError::Map(m) => RestoreError::Map(m),
        }
    }
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
                let path = world.get::<Path>(*entity).expect("path");
                let formation = world.get::<FormationState>(*entity).expect("formation");
                let combat = world.get::<Combat>(*entity).expect("combat");
                let fire = world.get::<Fire>(*entity).map(|f| FireSnap {
                    mode: f.mode,
                    target: f.target,
                    cooldown: f.cooldown,
                });
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
                    order_target: order.target,
                    order_facing: order.facing,
                    order_speed: order.speed,
                    order_since: order.since,
                    path: path.waypoints.clone(),
                    path_next: path.next,
                    path_requested: path.requested,
                    formation: regs.formations.id_of(formation.template).clone(),
                    ranks: formation.ranks,
                    files: formation.files,
                    integrity: formation.integrity,
                    morph_until: formation.morph_until,
                    needs_reform: formation.needs_reform,
                    prior_formation: formation
                        .prior_template
                        .map(|h| regs.formations.id_of(h).clone()),
                    laid_out_facing: formation.laid_out_facing,
                    fire,
                    order_target_regiment: order.target_regiment,
                    engaged: combat.engaged,
                    last_fighting: combat.last_fighting,
                    charge_until: combat.charge_until,
                    experience: combat.experience,
                    kills: combat.kills,
                    deaths_5s: morale.deaths_5s.to_vec(),
                    initial: morale.initial,
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
                    melee_target: world.get::<MeleeState>(*entity).expect("melee").target,
                    melee_cooldown: world.get::<MeleeState>(*entity).expect("melee").cooldown,
                    ranged: world
                        .get::<RangedState>(*entity)
                        .map(|r| (r.ammo, r.cooldown)),
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
            projectiles: world.resource::<Projectiles>().0.clone(),
            pending_damage: world.resource::<PendingDamage>().0.clone(),
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
        w.install_map(&snapshot.setup.map_id)?;
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
            let template = regs
                .formations
                .lookup(&r.formation)
                .ok_or_else(|| RestoreError::UnknownFormation(r.formation.clone()))?;
            let deaths_5s: [u16; DEATHS_RING] = r
                .deaths_5s
                .as_slice()
                .try_into()
                .map_err(|_| RestoreError::Malformed("casualty ring length"))?;
            let prior_template = match &r.prior_formation {
                Some(id) => Some(
                    regs.formations
                        .lookup(id)
                        .ok_or_else(|| RestoreError::UnknownFormation(id.clone()))?,
                ),
                None => None,
            };
            let entity = w
                .world
                .spawn((
                    Regiment {
                        id: r.id,
                        side: r.side,
                        setup_id: r.setup_id,
                        unit,
                        soldiers: Vec::new(),
                    },
                    Anchor {
                        pos: r.anchor_pos,
                        facing: r.anchor_facing,
                    },
                    Morale {
                        m: r.morale,
                        state: r.morale_state,
                        deaths_5s,
                        initial: r.initial,
                    },
                    Combat {
                        engaged: r.engaged,
                        last_fighting: r.last_fighting,
                        charge_until: r.charge_until,
                        experience: r.experience,
                        kills: r.kills,
                    },
                    Order {
                        kind: r.order,
                        target: r.order_target,
                        target_regiment: r.order_target_regiment,
                        facing: r.order_facing,
                        speed: r.order_speed,
                        since: r.order_since,
                    },
                    Path {
                        waypoints: r.path.clone(),
                        next: r.path_next,
                        requested: r.path_requested,
                    },
                    FormationState {
                        template,
                        ranks: r.ranks,
                        files: r.files,
                        slots: Vec::new(),
                        assignment: Vec::new(),
                        integrity: r.integrity,
                        morph_until: r.morph_until,
                        needs_reform: r.needs_reform,
                        prior_template,
                        laid_out_facing: r.laid_out_facing,
                        dirty: false,
                    },
                ))
                .id();
            if let Some(f) = &r.fire {
                w.world.entity_mut(entity).insert(Fire {
                    mode: f.mode,
                    target: f.target,
                    cooldown: f.cooldown,
                });
            }
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
                    Rank::default(),
                    Fsm {
                        state: s.fsm_state,
                        since: s.fsm_since,
                    },
                    MeleeState {
                        target: s.melee_target,
                        cooldown: s.melee_cooldown,
                    },
                    Attackers::default(),
                ))
                .id();
            if let Some((ammo, cooldown)) = s.ranged {
                w.world
                    .entity_mut(entity)
                    .insert(RangedState { ammo, cooldown });
            }
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
        {
            let mut prev: Option<il_core::ProjectileId> = None;
            for p in &snapshot.projectiles {
                if prev.is_some_and(|q| q >= p.id) {
                    return Err(RestoreError::Malformed(
                        "projectiles not in ascending id order",
                    ));
                }
                prev = Some(p.id);
            }
            let mut projectiles = w.world.resource_mut::<Projectiles>();
            projectiles.0.clear();
            projectiles.0.extend(snapshot.projectiles.iter().copied());
            w.world.resource_mut::<PendingDamage>().0 = snapshot.pending_damage.clone();
        }

        w.set_setup(snapshot.setup.clone());
        w.rebuild_derived();
        w.refresh_hash();
        Ok(w)
    }

    /// Reconstructs everything that is derived from hashed state and
    /// therefore not stored in a snapshot (SIM-DET-005); also run by `new`
    /// after spawning. The map itself is installed by `restore` before this
    /// runs. Phase 1 adds, in this order:
    /// - the spatial and anchor grids from positions (T1-031),
    /// - the nav grid from the map plus gate states (T1-032),
    /// - the path request queue from `Path.requested` flags (T1-032; the
    ///   paths themselves are stored),
    /// - flow fields per side (T2-042),
    /// - formation slots from template, count and ranks, and `Rank` from
    ///   `SlotRef` (T1-041),
    /// - attacker counts from the melee targets (T2-020),
    /// - the charge mass of regiments inside a charge window (T2-021).
    pub(crate) fn rebuild_derived(&mut self) {
        use bevy_ecs::system::RunSystemOnce;
        self.world
            .run_system_once(crate::spatial::rebuild_spatial_grids)
            .expect("grid rebuild has no failing params");
        let nav = {
            let regs = &self.world.resource::<crate::resources::Regs>().0;
            let map = &self.world.resource::<crate::resources::MapRes>().0;
            crate::nav::NavGrid::from_map(map, regs, &regs.rules.movement)
        };
        self.world.resource_mut::<crate::resources::NavGridRes>().0 = nav;
        let requested: Vec<RegimentId> = {
            let ids = self.world.resource::<Ids>();
            ids.regiment_entities
                .iter()
                .filter(|(_, e)| self.world.get::<Path>(*e).is_some_and(|p| p.requested))
                .map(|(id, _)| *id)
                .collect()
        };
        let mut queue = self.world.resource_mut::<crate::resources::PathRequests>();
        queue.0.clear();
        queue.0.extend(requested);
        crate::formation::rebuild_formation_derived(&mut self.world);
        crate::combat::rebuild_attackers(&mut self.world);
        crate::combat::rebuild_charge_mass(&mut self.world);
    }
}

/// Placeholder setup for worlds built with `BattleWorld::empty`.
fn empty_setup() -> BattleSetup {
    BattleSetup {
        map_id: ContentId::new(FLAT_MAP_ID).expect("valid id"),
        seed: 0,
        weather: Default::default(),
        time_of_day: 12,
        time_limit_ticks: 48_000,
        reveal_deployment: false,
        sides: Vec::new(),
        victory: Default::default(),
    }
}
