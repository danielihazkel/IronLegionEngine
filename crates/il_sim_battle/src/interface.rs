//! The campaign ↔ battle contract: `BattleSetup` in, `BattleResult` out
//! (TDD §4.2 `interface`, SAD §6.4, SIM-FLOW-019, REQ-SIM-060..063).
//!
//! Plain serialisable structs. A scenario file is a [`Scenario`] in JSON5:
//! a `BattleSetup` plus optional `commands`; optional fields default so
//! minimal files stay short.

use il_core::{PlayerId, Tick};
use il_data::ContentId;
use il_data::MapEdge;
use serde::{Deserialize, Serialize};

use crate::command::Command;

/// SIM-CORE-006, REQ-PERF-004.
pub const SOLDIER_CAP: u32 = 32_768;

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Weather {
    #[default]
    Clear,
    Rain,
    Fog,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VictoryRules {
    /// Side that wins when the time limit expires; `None` = draw.
    #[serde(default)]
    pub timeout_winner: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BattleSetup {
    /// The battle map (`content/maps/`); required since T1-030.
    pub map_id: ContentId,
    pub seed: u64,
    #[serde(default)]
    pub weather: Weather,
    /// Hour 0..24.
    #[serde(default = "default_time_of_day")]
    pub time_of_day: u8,
    /// `battle_flow.time_limit_ticks` default (Simulation Spec §15.1).
    #[serde(default = "default_time_limit_ticks")]
    pub time_limit_ticks: u32,
    #[serde(default)]
    pub reveal_deployment: bool,
    pub sides: Vec<SideSetup>,
    #[serde(default)]
    pub victory: VictoryRules,
}

fn default_time_of_day() -> u8 {
    12
}

fn default_time_limit_ticks() -> u32 {
    48_000
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SideSetup {
    pub faction: ContentId,
    /// Human or AI player id; `255` is the engine AI.
    pub player: PlayerId,
    #[serde(default)]
    pub deployment_zone: u8,
    pub general: GeneralSetup,
    pub regiments: Vec<RegimentSetup>,
    #[serde(default)]
    pub reinforcements: Vec<ReinforcementGroup>,
    /// The AI profile this side decides with when the engine owns it;
    /// the faction's `ai_profile` when absent (T2-080, plan decision 12).
    #[serde(default)]
    pub ai_profile: Option<ContentId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeneralSetup {
    /// A unit type of category `general` (SIM-GEN-001).
    pub unit_type: ContentId,
    #[serde(default = "default_rank")]
    pub rank: u8,
    #[serde(default)]
    pub name_key: String,
    /// `RegimentSetup.id` of the bodyguard regiment the general spawns in
    /// as one extra soldier; the side's first regiment when absent
    /// (T2-043, plan decision 4).
    #[serde(default)]
    pub bodyguard: Option<u32>,
}

fn default_rank() -> u8 {
    1
}

/// One unit group of a regiment's composition (SIM-FORM-012, T3-040): the
/// unit, how many, and the group's experience.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitGroupSetup {
    pub unit_type: ContentId,
    pub count: u16,
    #[serde(default)]
    pub experience: u8,
}

/// Which of the two regiment forms a setup got wrong (SIM-FORM-012).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupForm {
    /// Both the `unit_type` / `count` / `experience` shorthand and `units`.
    BothForms,
    /// Neither form: no `unit_type` and an empty `units`.
    Empty,
}

/// A regiment of a side (SIM-FORM-012). On the wire a JSON5 file gives the
/// single-group shorthand (`unit_type`, `count`, `experience`) or the
/// `units` composition; the absent form is left out of a written file
/// (`RegimentSetupWire`), while the binary encodings of snapshots and
/// replays, which are not self-describing, always carry every field
/// (`RegimentSetupFull`).
#[derive(Clone, Debug, PartialEq)]
pub struct RegimentSetup {
    /// Campaign regiment id, echoed in `RegimentResult`.
    pub id: u32,
    /// The single-group shorthand (SIM-FORM-012): `unit_type` + `count`
    /// (+ `experience`), or the `units` composition, never both.
    pub unit_type: Option<ContentId>,
    pub count: Option<u16>,
    pub experience: Option<u8>,
    /// The ordered unit groups of a mixed regiment (T3-040); empty in the
    /// shorthand form. `groups()` gives either form as a list.
    pub units: Vec<UnitGroupSetup>,
    /// Data-side `f32`, converted with `from_f32_data` at spawn.
    pub fatigue: f32,
    pub formation: Option<ContentId>,
    /// Pre-deploy override (PRD OQ-9, T2-070): the anchor position in world
    /// units. A regiment with a position spawns deployed there (checked
    /// against the map, not the deployment polygon), and a side whose every
    /// regiment has one starts with its deployment confirmed; when every
    /// side does, the battle starts in the Battle phase. Without it the
    /// regiment is auto-placed at its zone centre and awaits `Deploy`.
    pub position: Option<[f32; 2]>,
    /// Anchor facing in degrees, counter-clockwise from +x, for a
    /// pre-deployed regiment (default 0).
    pub facing_deg: Option<f32>,
}

/// The human-readable form (JSON5 files): the unset form is left out.
#[derive(Serialize, Deserialize)]
struct RegimentSetupWire {
    id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unit_type: Option<ContentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    count: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    experience: Option<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    units: Vec<UnitGroupSetup>,
    #[serde(default)]
    fatigue: f32,
    #[serde(default)]
    formation: Option<ContentId>,
    #[serde(default)]
    position: Option<[f32; 2]>,
    #[serde(default)]
    facing_deg: Option<f32>,
}

/// The binary form (postcard): every field, always.
#[derive(Serialize, Deserialize)]
struct RegimentSetupFull {
    id: u32,
    unit_type: Option<ContentId>,
    count: Option<u16>,
    experience: Option<u8>,
    units: Vec<UnitGroupSetup>,
    fatigue: f32,
    formation: Option<ContentId>,
    position: Option<[f32; 2]>,
    facing_deg: Option<f32>,
}

macro_rules! setup_forms {
    ($t:ident) => {
        impl From<&RegimentSetup> for $t {
            fn from(r: &RegimentSetup) -> Self {
                Self {
                    id: r.id,
                    unit_type: r.unit_type.clone(),
                    count: r.count,
                    experience: r.experience,
                    units: r.units.clone(),
                    fatigue: r.fatigue,
                    formation: r.formation.clone(),
                    position: r.position,
                    facing_deg: r.facing_deg,
                }
            }
        }
        impl From<$t> for RegimentSetup {
            fn from(w: $t) -> Self {
                Self {
                    id: w.id,
                    unit_type: w.unit_type,
                    count: w.count,
                    experience: w.experience,
                    units: w.units,
                    fatigue: w.fatigue,
                    formation: w.formation,
                    position: w.position,
                    facing_deg: w.facing_deg,
                }
            }
        }
    };
}
setup_forms!(RegimentSetupWire);
setup_forms!(RegimentSetupFull);

impl Serialize for RegimentSetup {
    fn serialize<Ser: serde::Serializer>(&self, s: Ser) -> Result<Ser::Ok, Ser::Error> {
        if s.is_human_readable() {
            RegimentSetupWire::from(self).serialize(s)
        } else {
            RegimentSetupFull::from(self).serialize(s)
        }
    }
}

impl<'de> Deserialize<'de> for RegimentSetup {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        if d.is_human_readable() {
            RegimentSetupWire::deserialize(d).map(Self::from)
        } else {
            RegimentSetupFull::deserialize(d).map(Self::from)
        }
    }
}

impl RegimentSetup {
    /// A single-group regiment with the defaults (no experience, fresh, the
    /// unit's first formation, auto-placed).
    pub fn single(id: u32, unit_type: ContentId, count: u16) -> Self {
        Self {
            id,
            unit_type: Some(unit_type),
            count: Some(count),
            experience: None,
            units: Vec::new(),
            fatigue: 0.0,
            formation: None,
            position: None,
            facing_deg: None,
        }
    }

    /// A mixed regiment of `units` in list order (SIM-FORM-012).
    pub fn mixed(id: u32, units: Vec<UnitGroupSetup>) -> Self {
        Self {
            units,
            ..Self::single(id, ContentId::new("il:none").expect("valid id"), 0)
        }
        .without_shorthand()
    }

    fn without_shorthand(mut self) -> Self {
        self.unit_type = None;
        self.count = None;
        self.experience = None;
        self
    }

    /// Whether exactly one form is given (SIM-FORM-012).
    pub fn form(&self) -> Result<(), SetupForm> {
        let shorthand =
            self.unit_type.is_some() || self.count.is_some() || self.experience.is_some();
        match (shorthand, self.units.is_empty()) {
            (true, false) => Err(SetupForm::BothForms),
            (false, true) => Err(SetupForm::Empty),
            _ if self.unit_type.is_none() && self.units.is_empty() => Err(SetupForm::Empty),
            _ => Ok(()),
        }
    }

    /// The composition as an ordered list of groups: `units`, or the
    /// shorthand as one group (an absent `count` reads 0, which the setup
    /// check rejects as `EmptyRegiment`).
    pub fn groups(&self) -> Vec<UnitGroupSetup> {
        if !self.units.is_empty() {
            return self.units.clone();
        }
        self.unit_type
            .clone()
            .map(|unit_type| UnitGroupSetup {
                unit_type,
                count: self.count.unwrap_or(0),
                experience: self.experience.unwrap_or(0),
            })
            .into_iter()
            .collect()
    }

    /// Soldiers in the regiment: the groups' counts summed (saturating).
    pub fn total(&self) -> u16 {
        if !self.units.is_empty() {
            return self
                .units
                .iter()
                .fold(0u16, |a, g| a.saturating_add(g.count));
        }
        self.count.unwrap_or(0)
    }

    /// The regiment's experience level: the count-weighted mean of the
    /// groups', floored (SIM-FORM-012 as amended in T3-040).
    pub fn experience_mean(&self) -> u8 {
        let groups = self.groups();
        let n: u32 = groups.iter().map(|g| u32::from(g.count)).sum();
        if n == 0 {
            return groups.first().map_or(0, |g| g.experience);
        }
        let sum: u32 = groups
            .iter()
            .map(|g| u32::from(g.count) * u32::from(g.experience))
            .sum();
        u8::try_from(sum / n).unwrap_or(u8::MAX)
    }

    /// The first group's unit type (the shorthand's), if any.
    pub fn first_unit(&self) -> Option<&ContentId> {
        self.units
            .first()
            .map(|g| &g.unit_type)
            .or(self.unit_type.as_ref())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReinforcementGroup {
    /// Ticks after the Battle phase begins (SIM-FLOW-016).
    pub arrival_tick: u32,
    /// The map edge the group enters from; the map must list it for the
    /// side's deployment zone (`reinforcement_edges`, T2-070).
    pub edge: MapEdge,
    pub regiments: Vec<RegimentSetup>,
}

impl BattleSetup {
    /// Soldiers at start plus pending reinforcements plus one general per
    /// side (SIM-CORE-006).
    pub fn soldier_total(&self) -> u32 {
        self.sides
            .iter()
            .flat_map(|s| {
                s.regiments
                    .iter()
                    .chain(s.reinforcements.iter().flat_map(|g| g.regiments.iter()))
            })
            .map(|r| u32::from(r.total()))
            .sum::<u32>()
            + self.sides.len() as u32
    }
}

/// A scenario file (T1-081, REQ-TEST-002): a `BattleSetup` plus an optional
/// scripted command stream that `il_cli run` and `il_app` feed to the sim
/// tick by tick. Commands may appear in any order; they are sorted by
/// `(tick, player, seq)` when the script is built.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    #[serde(flatten)]
    pub setup: BattleSetup,
    #[serde(default)]
    pub commands: Vec<Command>,
    /// How far the determinism test runs this file (T2-111/T2-112, plan
    /// decision 4); absent means the default budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub determinism: Option<DeterminismBudget>,
}

/// The determinism test's budget for one scenario file: run `ticks` ticks,
/// snapshot and restore at `snapshot_at` (REQ-TEST-002, TDD §17).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeterminismBudget {
    pub ticks: u32,
    pub snapshot_at: u32,
}

impl DeterminismBudget {
    /// The budget of a file without a `determinism` block.
    pub const DEFAULT: Self = Self {
        ticks: 10_000,
        snapshot_at: 5_000,
    };
}

impl Default for DeterminismBudget {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl Scenario {
    /// The command stream as a tick-ordered script.
    pub fn script(&self) -> ScriptedCommands {
        ScriptedCommands::new(self.commands.clone())
    }

    /// The file's determinism budget, or the default.
    pub fn determinism_budget(&self) -> DeterminismBudget {
        self.determinism.unwrap_or_default()
    }
}

/// A tick-ordered command stream handed to `step` one tick at a time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScriptedCommands {
    commands: Vec<Command>,
    next: usize,
}

impl ScriptedCommands {
    pub fn new(mut commands: Vec<Command>) -> Self {
        commands.sort_by_key(|c| (c.tick, c.player, c.seq));
        Self { commands, next: 0 }
    }

    /// Every command stamped `tick` or earlier that has not been taken yet
    /// (stale ones are still handed over, so the sim rejects them visibly).
    pub fn take_for(&mut self, tick: Tick) -> Vec<Command> {
        let start = self.next;
        while self.next < self.commands.len() && self.commands[self.next].tick <= tick {
            self.next += 1;
        }
        self.commands[start..self.next].to_vec()
    }

    pub fn remaining(&self) -> usize {
        self.commands.len() - self.next
    }

    /// The commands not yet taken, in order (a battle save keeps them so
    /// the loaded battle's script continues; T2-101).
    pub fn remaining_commands(&self) -> &[Command] {
        &self.commands[self.next..]
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

// --------------------------------------------------------------- results

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum GeneralFate {
    Alive,
    Wounded,
    Dead,
    Captured,
}

/// One unit group's counts in the result (SIM-FORM-015, T3-040; filled
/// from T3-041): the rows of a regiment sum to its totals.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitGroupResult {
    pub unit_type: ContentId,
    pub initial: u16,
    pub survivors: u16,
    pub killed: u16,
    pub fled: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegimentResult {
    pub id: u32,
    pub initial: u16,
    pub survivors: u16,
    pub fled: u16,
    pub killed: u16,
    pub experience_gain: u16,
    pub ammo_left: u16,
    /// False for a reinforcement group that never entered the field
    /// (T2-070); such regiments count in full as survivors.
    #[serde(default = "default_true")]
    pub arrived: bool,
    /// One row per unit group in setup order (SIM-FORM-015).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub units: Vec<UnitGroupResult>,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideResult {
    pub regiments: Vec<RegimentResult>,
    pub general_fate: GeneralFate,
    pub loot: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BattleSummary {
    pub total_killed: u32,
    pub total_fled: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BattleResult {
    pub winner: Option<u8>,
    pub duration_ticks: u32,
    pub sides: Vec<SideResult>,
    pub summary: BattleSummary,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_json5_scenario_parses_with_defaults() {
        let setup: BattleSetup = json5::from_str(
            r#"{
              map_id: "rome:test_field",
              seed: 42,
              sides: [
                { faction: "rome:rome", player: 0,
                  general: { unit_type: "rome:general" },
                  regiments: [ { id: 1, unit_type: "rome:hastati", count: 500, position: [-100, 0] } ] },
                { faction: "rome:rome", player: 1,
                  general: { unit_type: "rome:general", bodyguard: 2 },
                  regiments: [ { id: 2, unit_type: "rome:hastati", count: 500, facing_deg: 180 } ],
                  reinforcements: [ { arrival_tick: 100, edge: "north",
                    regiments: [ { id: 3, unit_type: "rome:hastati", count: 20 } ] } ] },
              ],
            }"#,
        )
        .unwrap();
        assert_eq!(setup.map_id.as_str(), "rome:test_field");
        assert_eq!(setup.time_of_day, 12);
        assert_eq!(setup.time_limit_ticks, 48_000);
        assert_eq!(setup.weather, Weather::Clear);
        assert_eq!(setup.sides[0].general.rank, 1);
        assert_eq!(setup.sides[1].player, PlayerId(1));
        assert_eq!(setup.sides[0].regiments[0].position, Some([-100.0, 0.0]));
        assert_eq!(setup.sides[1].regiments[0].facing_deg, Some(180.0));
        assert_eq!(setup.sides[1].general.bodyguard, Some(2));
        assert_eq!(setup.sides[1].reinforcements[0].edge, MapEdge::North);
        assert_eq!(setup.sides[0].ai_profile, None);
        assert_eq!(setup.soldier_total(), 1022);
        let json = serde_json::to_string(&setup).unwrap();
        let back: BattleSetup = serde_json::from_str(&json).unwrap();
        assert_eq!(back, setup);
    }
}
