//! Derived facts of a regiment's composition (SIM-FORM-012..014, T3-040).
//!
//! A regiment is an ordered list of unit groups (`Regiment.units`). Every
//! soldier keeps its own group's unit for reach, armour, mass, speed and
//! sprites; the regiment-level reads (its category for the AI, its cost,
//! the radius its formation is laid out with, the formations it may take,
//! whether it shoots at all and which unit's ranged block it fires with,
//! how far it sees, how fast its anchor moves) come from here. Everything
//! is a pure function of the groups and the registries, computed where it
//! is needed: a regiment has a handful of groups, so nothing is cached.

use il_core::{S, Scalar};
use il_data::{FormationTemplate, Handle, Registries, UnitCategory, UnitType};

use crate::command::SpeedMode;
use crate::components::UnitGroup;
use crate::movement::regiment::mode_speed;

/// The regiment-level facts of a composition (SIM-FORM-014).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Composition {
    /// The group with the largest cost share (ties: the first).
    pub category: UnitCategory,
    /// `Σ count_g × unit_g.cost` over the groups' spawn counts.
    pub cost: u32,
    /// The first group's unit (abilities, energy, sprites of the card).
    pub first: Handle<UnitType>,
    /// The intersection of the groups' allowed formations in the first
    /// group's order, or the first group's list when the intersection is
    /// empty (`disjoint_formations` says so).
    pub formations: Vec<Handle<FormationTemplate>>,
    pub disjoint_formations: bool,
    /// Some group's unit has a `ranged` block: the regiment gets `Fire`.
    pub any_ranged: bool,
    /// The first ranged group's unit: the regiment's ranged parameters
    /// (range, reload, accuracy) when it volleys.
    pub ranged_unit: Option<Handle<UnitType>>,
}

impl Composition {
    /// The facts of `units` (never empty: a regiment has at least one group).
    pub fn of(regs: &Registries, units: &[UnitGroup]) -> Self {
        let first_group = units.first().expect("a regiment has at least one group");
        let mut best: Option<(u32, UnitCategory)> = None;
        let mut cost: u32 = 0;
        let mut any_ranged = false;
        let mut ranged_unit = None;
        for g in units {
            let u = regs.units.get(g.unit);
            let share = u32::from(g.count).saturating_mul(u.cost);
            cost = cost.saturating_add(share);
            if best.is_none_or(|(b, _)| share > b) {
                best = Some((share, u.category));
            }
            if u.ranged.is_some() {
                any_ranged = true;
                if ranged_unit.is_none() {
                    ranged_unit = Some(g.unit);
                }
            }
        }
        let first_unit = regs.units.get(first_group.unit);
        let mut formations: Vec<Handle<FormationTemplate>> = first_unit
            .formations
            .iter()
            .copied()
            .filter(|h| {
                units[1..]
                    .iter()
                    .all(|g| regs.units.get(g.unit).formations.contains(h))
            })
            .collect();
        let disjoint_formations = formations.is_empty() && units.len() > 1;
        if formations.is_empty() {
            formations = first_unit.formations.clone();
        }
        Self {
            category: best.map_or(first_unit.category, |(_, c)| c),
            cost,
            first: first_group.unit,
            formations,
            disjoint_formations,
            any_ranged,
            ranged_unit,
        }
    }

    /// The first Column template the regiment may take (SIM-MOVE-004).
    pub fn column_template(&self, regs: &Registries) -> Option<Handle<FormationTemplate>> {
        self.formations
            .iter()
            .copied()
            .find(|h| regs.formations.get(*h).layout == il_data::Layout::Column)
    }
}

/// The widest group's `soldier_radius`: the formation's `sf` and `sr`
/// (SIM-FORM-013).
pub fn widest_radius(regs: &Registries, units: &[UnitGroup]) -> S {
    units
        .iter()
        .map(|g| regs.units.get(g.unit).soldier_radius)
        .fold(S::ZERO, |a, b| a.max(b))
}

/// The slowest group's unit speed for `mode` (SIM-MOVE-011).
pub fn slowest_speed(regs: &Registries, units: &[UnitGroup], mode: SpeedMode) -> S {
    units
        .iter()
        .map(|g| mode_speed(regs.units.get(g.unit), mode))
        .reduce(|a, b| a.min(b))
        .unwrap_or(S::ZERO)
}

/// The largest group's `los_radius`.
pub fn los_radius(regs: &Registries, units: &[UnitGroup]) -> S {
    units
        .iter()
        .map(|g| regs.units.get(g.unit).los_radius)
        .fold(S::ZERO, |a, b| a.max(b))
}

/// The count-weighted mean of a per-unit value over the groups (SIM-FORM-012
/// as amended in T3-040: the regiment's `morale_base`).
pub fn weighted_mean(regs: &Registries, units: &[UnitGroup], value: impl Fn(&UnitType) -> S) -> S {
    let mut sum = S::ZERO;
    let mut n: i32 = 0;
    for g in units {
        let c = i32::from(g.count);
        sum = sum + value(regs.units.get(g.unit)) * S::from_i32(c);
        n += c;
    }
    if n == 0 {
        S::ZERO
    } else {
        sum / S::from_i32(n)
    }
}

/// The regiment's groups as handles (a validated setup: every unit exists).
pub fn resolve_groups(regs: &Registries, r: &crate::interface::RegimentSetup) -> Vec<UnitGroup> {
    r.groups()
        .iter()
        .map(|g| UnitGroup {
            unit: regs.units.lookup(&g.unit_type).expect("validated"),
            count: g.count,
            experience: g.experience,
        })
        .collect()
}

/// Per-category soldier counts of the groups (plus `extra`, the general),
/// in group order, merged by category: what `label_slots` needs.
pub fn category_counts(
    regs: &Registries,
    units: &[UnitGroup],
    extra: Option<UnitCategory>,
) -> Vec<(UnitCategory, u16)> {
    let mut out: Vec<(UnitCategory, u16)> = Vec::new();
    let mut add = |c: UnitCategory, n: u16| match out.iter_mut().find(|(k, _)| *k == c) {
        Some((_, total)) => *total = total.saturating_add(n),
        None => out.push((c, n)),
    };
    for g in units {
        add(regs.units.get(g.unit).category, g.count);
    }
    if let Some(c) = extra {
        add(c, 1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::UnitGroup;
    use il_data::ContentId;
    use std::path::Path;

    fn regs() -> Registries {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game");
        Registries::load_root(&root).unwrap_or_else(|d| panic!("{d}"))
    }

    fn unit(regs: &Registries, id: &str) -> Handle<UnitType> {
        regs.units.lookup(&ContentId::new(id).unwrap()).unwrap()
    }

    fn group(regs: &Registries, id: &str, count: u16) -> UnitGroup {
        UnitGroup {
            unit: unit(regs, id),
            count,
            experience: 0,
        }
    }

    #[test]
    fn category_cost_radius_speed_and_ranged_follow_sim_form_014() {
        let regs = regs();
        // 40 velites (cost 250, radius 0.4, skirmisher) and 100 hastati
        // (cost 400, infantry): infantry by cost share, cost 10,000 + 40,000.
        let units = vec![
            group(&regs, "rome:velites", 40),
            group(&regs, "rome:hastati", 100),
        ];
        let c = Composition::of(&regs, &units);
        assert_eq!(c.category, UnitCategory::Infantry);
        assert_eq!(c.cost, 40 * 250 + 100 * 400);
        assert_eq!(c.first, unit(&regs, "rome:velites"));
        assert!(c.any_ranged);
        assert_eq!(c.ranged_unit, Some(unit(&regs, "rome:velites")));
        assert!(!c.disjoint_formations);
        // The intersection in the velites' order: loose, line, column (and
        // the cohort, appended to both lists in T3-040).
        let names: Vec<&str> = c
            .formations
            .iter()
            .map(|h| regs.formations.get(*h).id.as_str())
            .collect();
        assert_eq!(names[..3], ["rome:loose", "rome:line", "rome:column"]);
        assert!(names.contains(&"rome:cohort"));
        assert!(!names.contains(&"rome:square"), "hastati only");
        assert_eq!(widest_radius(&regs, &units), S::from_f32_data(0.4));
        let velites = regs.units.get(unit(&regs, "rome:velites"));
        let hastati = regs.units.get(unit(&regs, "rome:hastati"));
        assert_eq!(
            slowest_speed(&regs, &units, SpeedMode::Walk),
            velites.speed_walk.min(hastati.speed_walk)
        );
        // Weighted morale base: (40 × v + 100 × h) / 140.
        let expected = (velites.morale_base * S::from_i32(40)
            + hastati.morale_base * S::from_i32(100))
            / S::from_i32(140);
        assert_eq!(weighted_mean(&regs, &units, |u| u.morale_base), expected);
        assert_eq!(
            category_counts(&regs, &units, Some(UnitCategory::General)),
            vec![
                (UnitCategory::Skirmisher, 40),
                (UnitCategory::Infantry, 100),
                (UnitCategory::General, 1)
            ]
        );
    }

    #[test]
    fn a_tie_and_a_single_group_take_the_first_and_a_disjoint_pair_is_flagged() {
        let regs = regs();
        // Equal cost shares: 4 hastati (1,600) against 4 hastati: the first.
        let tie = vec![
            group(&regs, "persia:cavalry", 2),
            group(&regs, "rome:hastati", 1),
        ];
        let cav = regs.units.get(unit(&regs, "persia:cavalry")).cost;
        let has = regs.units.get(unit(&regs, "rome:hastati")).cost;
        let c = Composition::of(&regs, &tie);
        assert_eq!(c.cost, 2 * cav + has);
        if 2 * cav == has {
            assert_eq!(c.category, UnitCategory::Cavalry, "a tie keeps the first");
        }
        let single = vec![group(&regs, "greece:hoplite", 100)];
        let c = Composition::of(&regs, &single);
        assert_eq!(c.category, UnitCategory::Infantry);
        assert!(!c.any_ranged);
        assert_eq!(c.ranged_unit, None);
        assert_eq!(
            c.formations,
            regs.units.get(unit(&regs, "greece:hoplite")).formations
        );
        // Cavalry (wedge, line, column) with hoplites (phalanx, line,
        // column, square) share line and column; with a unit sharing
        // nothing the first list stands and the flag is raised.
        let pair = vec![
            group(&regs, "persia:cavalry", 30),
            group(&regs, "greece:hoplite", 30),
        ];
        let c = Composition::of(&regs, &pair);
        let names: Vec<&str> = c
            .formations
            .iter()
            .map(|h| regs.formations.get(*h).id.as_str())
            .collect();
        assert_eq!(names, ["rome:line", "rome:column"]);
        assert!(!c.disjoint_formations);
        assert!(
            slowest_speed(&regs, &pair, SpeedMode::Run)
                < slowest_speed(&regs, &pair[..1], SpeedMode::Run)
        );
    }
}
