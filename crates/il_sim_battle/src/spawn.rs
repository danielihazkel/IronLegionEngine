//! `BattleWorld::new`: setup validation and entity spawning
//! (TDD §4.2, SIM-FLOW-019, SIM-CORE-004..006, REQ-PERF-004).

use std::sync::Arc;

use bevy_ecs::prelude::*;
use il_core::{Angle, S, Scalar, SoldierId, Tick, V2};
use il_data::{ContentId, Handle, Registries, UnitType};

use crate::components::{
    Anchor, Attackers, Body, Combat, Facing, FatigueC, Fire, FormationState, Fsm, GeneralTag,
    Health, MeleeState, Morale, MoraleState, Order, Path, Pos, PrevFacing, PrevPos, RangedState,
    Rank, Regiment, RegimentFatigue, SlotRef, Soldier, SoldierState, Vel,
};
use crate::components::{Cooldowns, Energy, Statuses};
use crate::formation::{effective_ranks, layout_slots, slot_world};
use crate::interface::{BattleSetup, RegimentSetup, SOLDIER_CAP};
use crate::map::MapError;
use crate::morale::morale_state;
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
    #[error("side {side}: general unit type {unit_type} is not of category general (SIM-GEN-001)")]
    GeneralCategory { side: usize, unit_type: ContentId },
    #[error("side {side}: bodyguard regiment {id} is not one of the side's regiments")]
    UnknownBodyguard { side: usize, id: u32 },
    #[error("side {side}: a side needs a regiment for its general to ride with")]
    NoBodyguard { side: usize },
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
    #[error("side {side}: the map lists no reinforcement edge {edge:?} for its zone")]
    UnknownReinforcementEdge { side: usize, edge: il_data::MapEdge },
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
        // "each side has a general" (SIM-GEN-001, T2-043): a unit of
        // category `general` riding with one of the side's regiments.
        let Some(general_unit) = regs.units.lookup(&s.general.unit_type) else {
            return Err(SetupError::UnknownGeneralUnitType {
                side,
                unit_type: s.general.unit_type.clone(),
            });
        };
        if regs.units.get(general_unit).category != il_data::UnitCategory::General {
            return Err(SetupError::GeneralCategory {
                side,
                unit_type: s.general.unit_type.clone(),
            });
        }
        match s.general.bodyguard {
            Some(id) if !s.regiments.iter().any(|r| r.id == id) => {
                return Err(SetupError::UnknownBodyguard { side, id });
            }
            None if s.regiments.is_empty() => return Err(SetupError::NoBodyguard { side }),
            _ => {}
        }
        if !map.deployment.iter().any(|d| d.side == s.deployment_zone) {
            return Err(SetupError::MissingDeploymentZone {
                side,
                zone: s.deployment_zone,
            });
        }
        // SIM-FLOW-016 (plan decision 17): the map must list the edge.
        for g in &s.reinforcements {
            if !map
                .reinforcement_edges
                .iter()
                .any(|e| e.side == s.deployment_zone && e.edge == g.edge)
            {
                return Err(SetupError::UnknownReinforcementEdge { side, edge: g.edge });
            }
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

/// Spawns one regiment; with `general`, the side's general rides with it as
/// one extra soldier in the last slot (SIM-GEN-001, T2-043). Returns the
/// general's id.
pub(crate) fn spawn_regiment(
    world: &mut World,
    side: u8,
    setup: &RegimentSetup,
    unit: Handle<UnitType>,
    general: Option<(&crate::interface::GeneralSetup, Handle<UnitType>)>,
    placement: Option<(V2, Angle<S>)>,
) -> Option<SoldierId> {
    let count = setup.count + u16::from(general.is_some());
    let (radius, mass, hp, morale_base, category, template, slots, ranks, ammo, energy, slot_count) = {
        let regs = world.resource::<Regs>();
        let u = regs.0.units.get(unit);
        let template = setup
            .formation
            .as_ref()
            .and_then(|id| regs.0.formations.lookup(id))
            .unwrap_or_else(|| u.default_formation());
        let t = regs.0.formations.get(template);
        let ranks = effective_ranks(t, count, None);
        let mut slots = Vec::with_capacity(usize::from(count));
        layout_slots(t, count, ranks, u.soldier_radius, &mut slots);
        (
            u.soldier_radius,
            u.mass,
            u.hp,
            u.morale_base,
            u.category,
            template,
            slots,
            ranks,
            // SIM-PROJ-003: volleys per soldier from the unit's ranged block;
            // `None` for units that do not shoot.
            u.ranged.as_ref().map(|rg| rg.ammo),
            // SIM-ABIL-006 / SIM-ABIL-003 (T2-050): energy and one cooldown
            // slot per ability, the general's included for its bodyguard.
            u.energy_max,
            u.abilities.len() + general.map_or(0, |(_, g)| regs.0.units.get(g).abilities.len()),
        )
    };

    // T2-070: an auto-placement (deployment, reinforcements) wins over the
    // setup's pre-deploy position.
    let (anchor_pos, facing) = placement.unwrap_or_else(|| {
        (
            setup
                .position
                .map_or(V2::ZERO, |[x, y]| V2::from_f32_data(x, y)),
            Angle::<S>::from_degrees_data(setup.facing_deg.unwrap_or(0.0)),
        )
    });
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
                soldiers: Vec::with_capacity(usize::from(count)),
            },
            anchor,
            // SIM-MOR-001: `morale_base × (1 + exp_bonus × experience)`, and
            // the state that morale falls into (SIM-MOR-003; hastati start
            // Unsettled at 60), so the first tick raises no event.
            {
                let rules = &world.resource::<Regs>().0.rules.morale;
                let m = (morale_base
                    * (S::ONE + rules.exp_bonus * S::from_i32(i32::from(setup.experience.min(9)))))
                .clamp(S::ZERO, S::from_i32(100));
                let mut morale = Morale::new(m, count);
                morale.state = morale_state(m, MoraleState::Steady, rules);
                morale
            },
            // SIM-FAT-005: the mean starts at the roster fatigue.
            RegimentFatigue { mean: fatigue },
            Combat {
                experience: setup.experience.min(9),
                ..Combat::default()
            },
            Order::default(),
            Path::default(),
            FormationState::new(template, ranks, slots.clone(), facing),
            Energy { e: energy },
            Statuses::default(),
            Cooldowns(vec![0; slot_count]),
        ))
        .id();
    if ammo.is_some() {
        world.entity_mut(regiment_entity).insert(Fire::default());
    }
    world
        .resource_mut::<Ids>()
        .regiment_entities
        .push((rid, regiment_entity));

    let mut soldier_ids = Vec::with_capacity(usize::from(count));
    let mut general_id = None;
    for (i, slot) in slots.iter().enumerate() {
        // SIM-FORM-001: soldiers start on their slots. The general takes the
        // last slot with its own unit type and `hp × hp_mult` (SIM-GEN-001).
        let p = slot_world(&anchor, slot);
        let sid = world.resource_mut::<Ids>().soldiers.alloc();
        let is_general = general.is_some() && i + 1 == slots.len();
        let (s_unit, s_category, s_radius, s_mass, s_hp) = match (is_general, general) {
            (true, Some((_, g_unit))) => {
                let regs = world.resource::<Regs>();
                let g = regs.0.units.get(g_unit);
                (
                    g_unit,
                    g.category,
                    g.soldier_radius,
                    g.mass,
                    g.hp * regs.0.rules.general.hp_mult,
                )
            }
            _ => (unit, category, radius, mass, hp),
        };
        let entity = world
            .spawn((
                Soldier {
                    id: sid,
                    regiment: rid,
                    unit: s_unit,
                    category: s_category,
                },
                Pos { p },
                PrevPos { p },
                Vel::default(),
                Facing { theta: facing },
                PrevFacing { theta: facing },
                Body {
                    r: s_radius,
                    m: s_mass,
                },
                Health { hp: s_hp },
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
                MeleeState::default(),
                Attackers::default(),
            ))
            .id();
        if is_general && let Some((g, _)) = general {
            world.entity_mut(entity).insert(GeneralTag { rank: g.rank });
            general_id = Some(sid);
        } else if let Some(ammo) = ammo {
            world
                .entity_mut(entity)
                .insert(RangedState { ammo, cooldown: 0 });
        }
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
    general_id
}

impl BattleWorld {
    /// Validates `setup` (SIM-FLOW-019) and spawns every regiment and
    /// soldier in setup order, so ids ascend side by side, regiment by
    /// regiment. A side whose every regiment carries a `position` starts
    /// confirmed; when all do the battle starts in `Battle`, otherwise in
    /// `Deployment` with the unplaced regiments auto-placed at their zone
    /// centre (SIM-FLOW-011, T2-070).
    pub fn new(setup: &BattleSetup, regs: Arc<Registries>) -> Result<Self, SetupError> {
        validate(setup, &regs)?;
        let all_confirmed = setup
            .sides
            .iter()
            .all(|s| s.regiments.iter().all(|r| r.position.is_some()));
        let phase = if all_confirmed {
            BattlePhase::Battle
        } else {
            BattlePhase::Deployment
        };
        let mut w = BattleWorld::empty(setup.seed, regs.clone(), phase);
        w.install_map(&setup.map_id)?;
        let map = w.map().clone();
        let zone_centres: Vec<V2> = setup
            .sides
            .iter()
            .map(|s| crate::flow_battle::zone_centre(&map, s.deployment_zone))
            .collect();
        w.world.resource_mut::<Sides>().0 = setup
            .sides
            .iter()
            .map(|s| SideState {
                player: s.player,
                faction: s.faction.clone(),
                deployment_zone: s.deployment_zone,
                deployment_confirmed: s.regiments.iter().all(|r| r.position.is_some()),
                defeated: false,
                surrendered: false,
                reinforcements_spawned: 0,
                // SIM-FLOW-001 (T2-042): the edge nearest the deployment zone.
                escape_edge: crate::flow::escape_edge(&map, s.deployment_zone),
                general: None,
                general_regiment: None,
                general_dead: false,
            })
            .collect();
        for (side, s) in setup.sides.iter().enumerate() {
            let bodyguard = s
                .general
                .bodyguard
                .or_else(|| s.regiments.first().map(|r| r.id));
            let general_unit = regs
                .units
                .lookup(&s.general.unit_type)
                .expect("validated above");
            // SIM-FLOW-011 (plan I19): the regiments without a position form a
            // battle line at the zone centre facing the other sides.
            let unplaced: Vec<(&RegimentSetup, Handle<UnitType>, u16)> = s
                .regiments
                .iter()
                .filter(|r| r.position.is_none())
                .map(|r| {
                    let unit = regs.units.lookup(&r.unit_type).expect("validated above");
                    (r, unit, r.count + u16::from(bodyguard == Some(r.id)))
                })
                .collect();
            let others: Vec<V2> = zone_centres
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != side)
                .map(|(_, c)| *c)
                .collect();
            let placements = crate::flow_battle::auto_placements(
                &regs,
                &map,
                &unplaced,
                s.deployment_zone,
                &others,
            );
            let placed: Vec<(u32, (V2, Angle<S>))> = unplaced
                .iter()
                .zip(placements)
                .map(|((r, _, _), p)| (r.id, p))
                .collect();
            for r in &s.regiments {
                let unit = regs.units.lookup(&r.unit_type).expect("validated above");
                let general = (bodyguard == Some(r.id)).then_some((&s.general, general_unit));
                let placement = placed.iter().find(|(id, _)| *id == r.id).map(|(_, p)| *p);
                if let Some(gid) =
                    spawn_regiment(&mut w.world, side as u8, r, unit, general, placement)
                {
                    let rid = w
                        .world
                        .resource::<Ids>()
                        .regiment_entities
                        .last()
                        .map(|(id, _)| *id);
                    let state = &mut w.world.resource_mut::<Sides>().0[side];
                    state.general = Some(gid);
                    state.general_regiment = rid;
                }
            }
        }
        // Reinforcement groups spawn at Stage 16 when their tick comes (T2-070).
        w.set_setup(setup.clone());
        w.rebuild_derived();
        // SIM-VIS-004 (T2-060): every side's mask from the spawn positions.
        crate::visibility::recompute_all(&mut w.world);
        w.refresh_hash();
        Ok(w)
    }
}
