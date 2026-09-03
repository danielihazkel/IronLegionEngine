//! `BattleWorld::new`: setup validation and entity spawning
//! (TDD §4.2, SIM-FLOW-019, SIM-CORE-004..006, REQ-PERF-004).

use std::sync::Arc;

use bevy_ecs::prelude::*;
use il_core::{Angle, S, Scalar, Tick, V2};
use il_data::{ContentId, Handle, Registries, UnitType};

use crate::components::{
    Anchor, Body, Facing, FatigueC, FormationState, Fsm, Health, Morale, MoraleState, Order, Path,
    Pos, PrevFacing, PrevPos, Rank, Regiment, SlotRef, Soldier, SoldierState, Vel,
};
use crate::formation::{effective_ranks, layout_slots, slot_world};
use crate::interface::{BattleSetup, RegimentSetup, SOLDIER_CAP};
use crate::map::MapError;
use crate::resources::{BattlePhase, Ids, Regs, SideState, Sides};
use crate::world::{BattleWorld, InstallMapError};

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum SetupError {
    #[error("a battle needs at least one side")]
    NoSides,
    #[error("{count} soldiers exceed the cap of {cap} (SIM-CORE-006)")]
    OverCap { count: u32, cap: u32 },
    #[error("side {side}: unknown unit type {unit_type}")]
    UnknownUnitType { side: usize, unit_type: ContentId },
    #[error("side {side}: general unit type {unit_type} is unknown")]
    UnknownGeneralUnitType { side: usize, unit_type: ContentId },
    #[error("side {side}: regiment {regiment} has zero soldiers")]
    EmptyRegiment { side: usize, regiment: u32 },
    #[error("side {side}: regiment {regiment} names unknown formation {formation}")]
    UnknownFormation {
        side: usize,
        regiment: u32,
        formation: ContentId,
    },
    #[error("side {side}: more than 255 sides are not supported")]
    TooManySides { side: usize },
    #[error("unknown map {0}")]
    UnknownMap(ContentId),
    #[error("{0}")]
    Map(MapError),
    #[error("side {side}: the map defines no deployment polygon for zone {zone}")]
    MissingDeploymentZone { side: usize, zone: u8 },
    #[error("side {side}: regiment {regiment} at ({x}, {y}) is outside the map")]
    PositionOutOfMap {
        side: usize,
        regiment: u32,
        x: f32,
        y: f32,
    },
}

impl From<InstallMapError> for SetupError {
    fn from(e: InstallMapError) -> Self {
        match e {
            InstallMapError::UnknownMap(id) => SetupError::UnknownMap(id),
            InstallMapError::Map(m) => SetupError::Map(m),
        }
    }
}

/// SIM-FLOW-019 validation: cap, unit types exist, one general per side,
/// the map exists and defines a deployment polygon for every side's zone,
/// every (temporary) placement position lies on the map.
pub fn validate(setup: &BattleSetup, regs: &Registries) -> Result<(), SetupError> {
    if setup.sides.is_empty() {
        return Err(SetupError::NoSides);
    }
    let map = regs
        .maps
        .lookup(&setup.map_id)
        .map(|h| regs.maps.get(h))
        .ok_or_else(|| SetupError::UnknownMap(setup.map_id.clone()))?;
    let count = setup.soldier_total();
    if count > SOLDIER_CAP {
        return Err(SetupError::OverCap {
            count,
            cap: SOLDIER_CAP,
        });
    }
    for (side, s) in setup.sides.iter().enumerate() {
        if side > usize::from(u8::MAX) {
            return Err(SetupError::TooManySides { side });
        }
        // "each side has a general": the general's unit type must resolve.
        // The general is not spawned until T2-043.
        if !regs.units.contains(&s.general.unit_type) {
            return Err(SetupError::UnknownGeneralUnitType {
                side,
                unit_type: s.general.unit_type.clone(),
            });
        }
        if !map.deployment.iter().any(|d| d.side == s.deployment_zone) {
            return Err(SetupError::MissingDeploymentZone {
                side,
                zone: s.deployment_zone,
            });
        }
        let all = s
            .regiments
            .iter()
            .chain(s.reinforcements.iter().flat_map(|g| g.regiments.iter()));
        for r in all {
            if !regs.units.contains(&r.unit_type) {
                return Err(SetupError::UnknownUnitType {
                    side,
                    unit_type: r.unit_type.clone(),
                });
            }
            if r.count == 0 {
                return Err(SetupError::EmptyRegiment {
                    side,
                    regiment: r.id,
                });
            }
            if let Some(f) = &r.formation
                && !regs.formations.contains(f)
            {
                return Err(SetupError::UnknownFormation {
                    side,
                    regiment: r.id,
                    formation: f.clone(),
                });
            }
            if let Some([x, y]) = r.position
                && !(x >= 0.0
                    && y >= 0.0
                    && S::from_f32_data(x) <= map.size.w
                    && S::from_f32_data(y) <= map.size.h)
            {
                return Err(SetupError::PositionOutOfMap {
                    side,
                    regiment: r.id,
                    x,
                    y,
                });
            }
        }
    }
    Ok(())
}

pub(crate) fn spawn_regiment(
    world: &mut World,
    side: u8,
    setup: &RegimentSetup,
    unit: Handle<UnitType>,
    ammo: u16,
) {
    let (radius, mass, hp, morale_base, category, template, slots, ranks) = {
        let regs = world.resource::<Regs>();
        let u = regs.0.units.get(unit);
        let template = setup
            .formation
            .as_ref()
            .and_then(|id| regs.0.formations.lookup(id))
            .unwrap_or_else(|| u.default_formation());
        let t = regs.0.formations.get(template);
        let ranks = effective_ranks(t, setup.count, None);
        let mut slots = Vec::with_capacity(usize::from(setup.count));
        layout_slots(t, setup.count, ranks, u.soldier_radius, &mut slots);
        (
            u.soldier_radius,
            u.mass,
            u.hp,
            u.morale_base,
            u.category,
            template,
            slots,
            ranks,
        )
    };

    let anchor_pos = setup
        .position
        .map_or(V2::ZERO, |[x, y]| V2::from_f32_data(x, y));
    let facing = Angle::<S>::from_degrees_data(setup.facing_deg.unwrap_or(0.0));
    let anchor = Anchor {
        pos: anchor_pos,
        facing,
    };
    let fatigue = S::from_f32_data(setup.fatigue);

    let rid = world.resource_mut::<Ids>().regiments.alloc();
    let regiment_entity = world
        .spawn((
            Regiment {
                id: rid,
                side,
                setup_id: setup.id,
                unit,
                soldiers: Vec::with_capacity(usize::from(setup.count)),
                ammo,
            },
            anchor,
            Morale {
                m: morale_base,
                state: MoraleState::Steady,
            },
            Order::default(),
            Path::default(),
            FormationState::new(template, ranks, slots.clone(), facing),
        ))
        .id();
    world
        .resource_mut::<Ids>()
        .regiment_entities
        .push((rid, regiment_entity));

    let mut soldier_ids = Vec::with_capacity(usize::from(setup.count));
    for (i, slot) in slots.iter().enumerate() {
        // SIM-FORM-001: soldiers start on their slots.
        let p = slot_world(&anchor, slot);
        let sid = world.resource_mut::<Ids>().soldiers.alloc();
        let entity = world
            .spawn((
                Soldier {
                    id: sid,
                    regiment: rid,
                    unit,
                    category,
                },
                Pos { p },
                PrevPos { p },
                Vel::default(),
                Facing { theta: facing },
                PrevFacing { theta: facing },
                Body { r: radius, m: mass },
                Health { hp },
                FatigueC { f: fatigue },
                SlotRef {
                    slot: Some(i as u16),
                },
                Rank {
                    rank: slot.rank,
                    file: slot.file,
                },
                Fsm {
                    state: SoldierState::Idle,
                    since: Tick::ZERO,
                },
            ))
            .id();
        world
            .resource_mut::<Ids>()
            .soldier_entities
            .push((sid, entity));
        soldier_ids.push(sid);
    }
    world
        .get_mut::<Regiment>(regiment_entity)
        .expect("just spawned")
        .soldiers = soldier_ids;
}

impl BattleWorld {
    /// Validates `setup` (SIM-FLOW-019) and spawns every regiment and
    /// soldier in setup order, so ids ascend side by side, regiment by
    /// regiment. Phase 0 starts directly in `Battle`; the deployment phase
    /// arrives in T2-070.
    pub fn new(setup: &BattleSetup, regs: Arc<Registries>) -> Result<Self, SetupError> {
        validate(setup, &regs)?;
        let mut w = BattleWorld::empty(setup.seed, regs.clone(), BattlePhase::Battle);
        w.install_map(&setup.map_id)?;
        w.world.resource_mut::<Sides>().0 = setup
            .sides
            .iter()
            .map(|s| SideState {
                player: s.player,
                faction: s.faction.clone(),
                deployment_zone: s.deployment_zone,
                deployment_confirmed: true,
                defeated: false,
            })
            .collect();
        for (side, s) in setup.sides.iter().enumerate() {
            for r in &s.regiments {
                let unit = regs.units.lookup(&r.unit_type).expect("validated above");
                spawn_regiment(&mut w.world, side as u8, r, unit, 0);
            }
        }
        // Reinforcement groups are validated but spawn only in T2-070.
        w.set_setup(setup.clone());
        w.rebuild_derived();
        w.refresh_hash();
        Ok(w)
    }
}
