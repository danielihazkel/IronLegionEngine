//! `BattleWorld::new`: setup validation and entity spawning
//! (TDD §4.2, SIM-FLOW-019, SIM-CORE-004..006, REQ-PERF-004).

use std::sync::Arc;

use bevy_ecs::prelude::*;
use il_core::{Angle, S, Scalar, SoldierId, Tick, V2};
use il_data::{ContentId, Handle, Registries, UnitType};

use crate::components::{
    Anchor, Attackers, Body, Combat, Facing, FatigueC, Fire, FormationState, Fsm, GeneralTag,
    GroupTallies, Health, MeleeState, Morale, MoraleState, Order, Path, Pos, PrevFacing, PrevPos,
    RangedState, Rank, Regiment, RegimentFatigue, SlotRef, Soldier, SoldierState, UnitGroup, Vel,
};
use crate::components::{Cooldowns, Energy, Statuses};
use crate::composition::{self, Composition};
use crate::formation::{effective_ranks, label_slots, layout_slots, slot_world};
use crate::interface::{BattleSetup, RegimentSetup, SOLDIER_CAP, SetupForm};
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
    #[error("side {side}: unknown AI profile {id}")]
    UnknownAiProfile { side: usize, id: ContentId },
    #[error("side {side}: regiment {regiment} at ({x}, {y}) is outside the map")]
    PositionOutOfMap {
        side: usize,
        regiment: u32,
        x: f32,
        y: f32,
    },
    #[error(
        "side {side}: regiment {regiment} gives both the unit_type/count shorthand and a units list (SIM-FORM-012)"
    )]
    BothUnitForms { side: usize, regiment: u32 },
    #[error("side {side}: regiment {regiment} names no unit (SIM-FORM-012)")]
    EmptyComposition { side: usize, regiment: u32 },
    #[error("side {side}: regiment {regiment} names unknown unit {unit_type} in its composition")]
    UnknownUnit {
        side: usize,
        regiment: u32,
        unit_type: ContentId,
    },
    #[error("side {side}: regiment {regiment} has a unit group with zero soldiers")]
    GroupCountZero { side: usize, regiment: u32 },
}

/// A soft finding of the setup check (T3-040): the battle builds, the
/// caller reports it (`il_cli` on stderr, the app in its event panel).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupWarning {
    pub side: u8,
    pub regiment: u32,
    pub text: String,
}

impl std::fmt::Display for SetupWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "side {}: regiment {}: {}",
            self.side, self.regiment, self.text
        )
    }
}

/// SIM-FORM-014: a mixed regiment whose groups share no formation takes
/// the first group's list, with a warning naming it.
pub fn setup_warnings(setup: &BattleSetup, regs: &Registries) -> Vec<SetupWarning> {
    let mut out = Vec::new();
    for (side, s) in setup.sides.iter().enumerate() {
        let all = s
            .regiments
            .iter()
            .chain(s.reinforcements.iter().flat_map(|g| g.regiments.iter()));
        for r in all {
            let units = composition::resolve_groups(regs, r);
            if units.len() > 1 && Composition::of(regs, &units).disjoint_formations {
                out.push(SetupWarning {
                    side: side as u8,
                    regiment: r.id,
                    text: "its unit groups share no formation; the first group's list is used (SIM-FORM-014)".to_string(),
                });
            }
        }
    }
    out
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
        // T2-080 (plan decision 12): the side's profile override must exist.
        if let Some(id) = &s.ai_profile
            && !regs.ai_profiles.contains(id)
        {
            return Err(SetupError::UnknownAiProfile {
                side,
                id: id.clone(),
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
            // SIM-FORM-012 (T3-040): one form, every group's unit known and
            // at least one soldier per group.
            match r.form() {
                Err(SetupForm::BothForms) => {
                    return Err(SetupError::BothUnitForms {
                        side,
                        regiment: r.id,
                    });
                }
                Err(SetupForm::Empty) => {
                    return Err(SetupError::EmptyComposition {
                        side,
                        regiment: r.id,
                    });
                }
                Ok(()) => {}
            }
            for (k, g) in r.groups().iter().enumerate() {
                if !regs.units.contains(&g.unit_type) {
                    return Err(if r.units.is_empty() {
                        SetupError::UnknownUnitType {
                            side,
                            unit_type: g.unit_type.clone(),
                        }
                    } else {
                        SetupError::UnknownUnit {
                            side,
                            regiment: r.id,
                            unit_type: g.unit_type.clone(),
                        }
                    });
                }
                if g.count == 0 && !r.units.is_empty() {
                    return Err(SetupError::GroupCountZero {
                        side,
                        regiment: r.id,
                    });
                }
                if k >= usize::from(u8::MAX) {
                    return Err(SetupError::GroupCountZero {
                        side,
                        regiment: r.id,
                    });
                }
            }
            if r.total() == 0 {
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

/// Spawns one regiment from its composition (SIM-FORM-012/013, T3-040);
/// with `general`, the side's general rides with it as one extra soldier
/// in group 0 (SIM-GEN-001, T2-043). Soldiers spawn group by group in
/// list order, each on the first free slot labelled its category, else the
/// first free unlabelled slot (rank-major), so a zoneless template holds
/// the groups front to back and a zoned one its zones. Returns the
/// general's id.
pub(crate) fn spawn_regiment(
    world: &mut World,
    side: u8,
    setup: &RegimentSetup,
    general: Option<(&crate::interface::GeneralSetup, Handle<UnitType>)>,
    placement: Option<(V2, Angle<S>)>,
) -> Option<SoldierId> {
    let count = setup.total() + u16::from(general.is_some());
    let (units, morale_base, template, slots, assignment, ranks, energy, slot_count, any_ranged) = {
        let regs = world.resource::<Regs>();
        let mut units = composition::resolve_groups(&regs.0, setup);
        if general.is_some() {
            // The general rides in group 0 and counts in it (SIM-FORM-015).
            units[0].count = units[0].count.saturating_add(1);
        }
        let comp = Composition::of(&regs.0, &units);
        let radius = composition::widest_radius(&regs.0, &units);
        let u0 = regs.0.units.get(units[0].unit);
        let template = setup
            .formation
            .as_ref()
            .and_then(|id| regs.0.formations.lookup(id))
            .unwrap_or_else(|| u0.default_formation());
        let t = regs.0.formations.get(template);
        let ranks = effective_ranks(t, count, None);
        let mut slots = Vec::with_capacity(usize::from(count));
        layout_slots(t, count, ranks, radius, &mut slots);
        // Category counts of the spawn: the groups as set up (group 0
        // without the general it will carry) plus the general's own.
        let as_set_up = units_without_general(&units, general.is_some());
        let counts = composition::category_counts(
            &regs.0,
            &as_set_up,
            general.map(|(_, g)| regs.0.units.get(g).category),
        );
        label_slots(t, &mut slots, &counts);
        // The spawn assignment (SIM-FORM-013): group by group, the first
        // free slot of the soldier's category, else the first free
        // unlabelled slot, in slot order.
        let mut taken = vec![false; slots.len()];
        let mut assignment: Vec<Option<u16>> = Vec::with_capacity(usize::from(count));
        let mut place = |category: UnitCategoryOf| {
            let pick = slots
                .iter()
                .enumerate()
                .find(|(k, s)| !taken[*k] && s.category == Some(category.0))
                .or_else(|| {
                    slots
                        .iter()
                        .enumerate()
                        .find(|(k, s)| !taken[*k] && s.category.is_none())
                })
                .or_else(|| slots.iter().enumerate().find(|(k, _)| !taken[*k]))
                .map(|(k, _)| k);
            if let Some(k) = pick {
                taken[k] = true;
            }
            assignment.push(pick.map(|k| k as u16));
        };
        for (g, group) in units.iter().enumerate() {
            let category = regs.0.units.get(group.unit).category;
            let n = if g == 0 && general.is_some() {
                group.count - 1
            } else {
                group.count
            };
            for _ in 0..n {
                place(UnitCategoryOf(category));
            }
        }
        if let Some((_, g_unit)) = general {
            place(UnitCategoryOf(regs.0.units.get(g_unit).category));
        }
        (
            units,
            composition::weighted_mean(&regs.0, &as_set_up, |u| u.morale_base),
            template,
            slots,
            assignment,
            ranks,
            // SIM-ABIL-006 / SIM-ABIL-003 (T2-050): energy and one cooldown
            // slot per ability, the first group's unit's (SIM-FORM-014) and
            // the general's for its bodyguard.
            u0.energy_max,
            u0.abilities.len() + general.map_or(0, |(_, g)| regs.0.units.get(g).abilities.len()),
            comp.any_ranged,
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
    let experience = setup.experience_mean().min(9);

    let rid = world.resource_mut::<Ids>().regiments.alloc();
    let regiment_entity = world
        .spawn((
            Regiment {
                id: rid,
                side,
                setup_id: setup.id,
                unit: units[0].unit,
                units: units.clone(),
                soldiers: Vec::with_capacity(usize::from(count)),
            },
            anchor,
            // SIM-MOR-001: `morale_base × (1 + exp_bonus × experience)`, and
            // the state that morale falls into (SIM-MOR-003; hastati start
            // Unsettled at 60), so the first tick raises no event. A mixed
            // regiment's base is the count-weighted mean of its groups'.
            {
                let rules = &world.resource::<Regs>().0.rules.morale;
                let m = (morale_base
                    * (S::ONE + rules.exp_bonus * S::from_i32(i32::from(experience))))
                .clamp(S::ZERO, S::from_i32(100));
                let mut morale = Morale::new(m, count);
                morale.state = morale_state(m, MoraleState::Steady, rules);
                morale
            },
            // SIM-FAT-005: the mean starts at the roster fatigue.
            RegimentFatigue { mean: fatigue },
            Combat {
                experience,
                ..Combat::default()
            },
            Order::default(),
            Path::default(),
            {
                let mut state = FormationState::new(template, ranks, slots.clone(), facing);
                state.assignment = assignment.clone();
                state
            },
            Energy { e: energy },
            Statuses::default(),
            Cooldowns(vec![0; slot_count]),
            GroupTallies::new(units.len()),
        ))
        .id();
    if any_ranged {
        world.entity_mut(regiment_entity).insert(Fire::default());
    }
    world
        .resource_mut::<Ids>()
        .regiment_entities
        .push((rid, regiment_entity));

    // The soldiers in spawn order: every group's, then the general.
    let mut roster: Vec<(Handle<UnitType>, u8, bool)> = Vec::with_capacity(usize::from(count));
    for (g, group) in units.iter().enumerate() {
        let n = if g == 0 && general.is_some() {
            group.count - 1
        } else {
            group.count
        };
        for _ in 0..n {
            roster.push((group.unit, g as u8, false));
        }
    }
    if let Some((_, g_unit)) = general {
        roster.push((g_unit, 0, true));
    }
    let mut soldier_ids = Vec::with_capacity(usize::from(count));
    let mut general_id = None;
    for (i, (s_unit, group, is_general)) in roster.into_iter().enumerate() {
        // SIM-FORM-001: soldiers start on their slots. The general carries
        // its own unit type and `hp × hp_mult` (SIM-GEN-001).
        let slot_index = assignment.get(i).copied().flatten();
        let slot = slot_index.map(|k| slots[usize::from(k)]);
        let p = slot.map_or(anchor.pos, |slot| slot_world(&anchor, &slot));
        let sid = world.resource_mut::<Ids>().soldiers.alloc();
        let (s_category, s_radius, s_mass, s_hp, ammo) = {
            let regs = world.resource::<Regs>();
            let u = regs.0.units.get(s_unit);
            let hp = if is_general {
                u.hp * regs.0.rules.general.hp_mult
            } else {
                u.hp
            };
            (
                u.category,
                u.soldier_radius,
                u.mass,
                hp,
                // SIM-PROJ-003: volleys per soldier from the unit's ranged
                // block; the general never volleys.
                if is_general {
                    None
                } else {
                    u.ranged.as_ref().map(|rg| rg.ammo)
                },
            )
        };
        let entity = world
            .spawn((
                Soldier {
                    id: sid,
                    regiment: rid,
                    unit: s_unit,
                    category: s_category,
                    group,
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
                SlotRef { slot: slot_index },
                slot.map_or(Rank::default(), |slot| Rank {
                    rank: slot.rank,
                    file: slot.file,
                }),
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

/// A category as the spawn assignment matches it.
struct UnitCategoryOf(il_data::UnitCategory);

/// The groups as set up (group 0 without the general it carries), for the
/// count-weighted means.
fn units_without_general(units: &[UnitGroup], general: bool) -> Vec<UnitGroup> {
    let mut v = units.to_vec();
    if general && let Some(g0) = v.first_mut() {
        g0.count = g0.count.saturating_sub(1);
    }
    v
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
        let warnings = setup_warnings(setup, &regs);
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
        w.setup_warnings = warnings;
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
            let unplaced: Vec<(&RegimentSetup, Vec<UnitGroup>, u16)> = s
                .regiments
                .iter()
                .filter(|r| r.position.is_none())
                .map(|r| {
                    let units = composition::resolve_groups(&regs, r);
                    (r, units, r.total() + u16::from(bodyguard == Some(r.id)))
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
                let general = (bodyguard == Some(r.id)).then_some((&s.general, general_unit));
                let placement = placed.iter().find(|(id, _)| *id == r.id).map(|(_, p)| *p);
                if let Some(gid) = spawn_regiment(&mut w.world, side as u8, r, general, placement) {
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
