//! `BattleWorld::new`: setup validation and entity spawning
//! (TDD §4.2, SIM-FLOW-019, SIM-CORE-004..006, REQ-PERF-004).

use std::sync::Arc;

use bevy_ecs::prelude::*;
use il_core::{Angle, S, Scalar, Tick, V2};
use il_data::{ContentId, Handle, Registries, UnitType};

use crate::components::{
    Anchor, Body, Facing, FatigueC, Fsm, Health, Morale, MoraleState, Order, Path, Pos, PrevFacing,
    PrevPos, Regiment, SlotRef, Soldier, SoldierState, Vel,
};
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

/// Smallest `f` with `f * f >= n`.
fn ceil_sqrt(n: u32) -> u32 {
    let mut f = 1u32;
    while f * f < n {
        f += 1;
    }
    f
}

/// Placement of soldier `i` in a plain grid of `files` columns: lateral
/// offset from the centre line and depth behind the front rank, in units of
/// `spacing`. Real formations replace this in T1-040 / T1-041.
fn grid_offsets(i: u32, files: u32, spacing: S) -> (S, S) {
    let col = i % files;
    let row = i / files;
    // (2 * col - (files - 1)) / 2 keeps the front rank centred on the anchor
    // using only integer-derived scalars.
    let lateral = S::from_i32((2 * col) as i32 - (files - 1) as i32) * spacing * S::HALF;
    let depth = S::from_i32(row as i32) * spacing;
    (lateral, depth)
}

pub(crate) fn spawn_regiment(
    world: &mut World,
    side: u8,
    setup: &RegimentSetup,
    unit: Handle<UnitType>,
    ammo: u16,
) {
    let (radius, mass, hp, morale_base, category) = {
        let regs = world.resource::<Regs>();
        let u = regs.0.units.get(unit);
        (u.soldier_radius, u.mass, u.hp, u.morale_base, u.category)
    };

    let anchor_pos = setup
        .position
        .map_or(V2::ZERO, |[x, y]| V2::from_f32_data(x, y));
    let facing = Angle::<S>::from_degrees_data(setup.facing_deg.unwrap_or(0.0));
    let forward = facing.direction();
    // `perp` is 90° counter-clockwise; the right-hand side is the other way.
    let right = -forward.perp();
    let spacing = radius * S::from_i32(3);
    let files = ceil_sqrt(u32::from(setup.count));
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
            Anchor {
                pos: anchor_pos,
                facing,
            },
            Morale {
                m: morale_base,
                state: MoraleState::Steady,
            },
            Order::default(),
            Path::default(),
        ))
        .id();
    world
        .resource_mut::<Ids>()
        .regiment_entities
        .push((rid, regiment_entity));

    let mut soldier_ids = Vec::with_capacity(usize::from(setup.count));
    for i in 0..u32::from(setup.count) {
        let (lateral, depth) = grid_offsets(i, files, spacing);
        let p = anchor_pos + right * lateral - forward * depth;
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
                SlotRef::default(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_is_centred_and_fills_rows() {
        assert_eq!(ceil_sqrt(1), 1);
        assert_eq!(ceil_sqrt(2), 2);
        assert_eq!(ceil_sqrt(4), 2);
        assert_eq!(ceil_sqrt(5), 3);
        assert_eq!(ceil_sqrt(500), 23);
        // Three files: lateral offsets -s, 0, +s; second row one spacing back.
        let s = S::from_i32(2);
        assert_eq!(grid_offsets(0, 3, s), (-s, S::ZERO));
        assert_eq!(grid_offsets(1, 3, s), (S::ZERO, S::ZERO));
        assert_eq!(grid_offsets(2, 3, s), (s, S::ZERO));
        assert_eq!(grid_offsets(3, 3, s), (-s, s));
        // Two files: offsets ±s/2.
        assert_eq!(grid_offsets(0, 2, s), (-S::ONE, S::ZERO));
        assert_eq!(grid_offsets(1, 2, s), (S::ONE, S::ZERO));
    }
}
