//! The campaign ↔ battle contract: `BattleSetup` in, `BattleResult` out
//! (TDD §4.2 `interface`, SAD §6.4, SIM-FLOW-019, REQ-SIM-060..063).
//!
//! Plain serialisable structs. A scenario file is a [`Scenario`] in JSON5:
//! a `BattleSetup` plus optional `commands`; optional fields default so
//! minimal files stay short.

use il_core::{PlayerId, Tick};
use il_data::ContentId;
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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeneralSetup {
    pub unit_type: ContentId,
    #[serde(default = "default_rank")]
    pub rank: u8,
    #[serde(default)]
    pub name_key: String,
}

fn default_rank() -> u8 {
    1
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegimentSetup {
    /// Campaign regiment id, echoed in `RegimentResult`.
    pub id: u32,
    pub unit_type: ContentId,
    pub count: u16,
    #[serde(default)]
    pub experience: u8,
    /// Data-side `f32`, converted with `from_f32_data` at spawn.
    #[serde(default)]
    pub fatigue: f32,
    #[serde(default)]
    pub formation: Option<ContentId>,
    /// TEMPORARY (Phase 0, SAD §12 T-7): anchor position in world units,
    /// because deployment zones do not exist until T2-070. Removed then.
    #[serde(default)]
    pub position: Option<[f32; 2]>,
    /// TEMPORARY (Phase 0, SAD §12 T-7): anchor facing in degrees,
    /// counter-clockwise from +x. Removed with `position`.
    #[serde(default)]
    pub facing_deg: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReinforcementGroup {
    pub arrival_tick: u32,
    /// Map edge index the group enters from.
    pub edge: u8,
    pub regiments: Vec<RegimentSetup>,
}

impl BattleSetup {
    /// Soldiers at start plus pending reinforcements (SIM-CORE-006).
    pub fn soldier_total(&self) -> u32 {
        self.sides
            .iter()
            .flat_map(|s| {
                s.regiments
                    .iter()
                    .chain(s.reinforcements.iter().flat_map(|g| g.regiments.iter()))
            })
            .map(|r| u32::from(r.count))
            .sum()
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
}

impl Scenario {
    /// The command stream as a tick-ordered script.
    pub fn script(&self) -> ScriptedCommands {
        ScriptedCommands::new(self.commands.clone())
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegimentResult {
    pub id: u32,
    pub initial: u16,
    pub survivors: u16,
    pub fled: u16,
    pub killed: u16,
    pub experience_gain: u16,
    pub ammo_left: u16,
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
                  general: { unit_type: "rome:hastati" },
                  regiments: [ { id: 1, unit_type: "rome:hastati", count: 500, position: [-100, 0] } ] },
                { faction: "rome:rome", player: 1,
                  general: { unit_type: "rome:hastati" },
                  regiments: [ { id: 2, unit_type: "rome:hastati", count: 500, facing_deg: 180 } ],
                  reinforcements: [ { arrival_tick: 100, edge: 1,
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
        assert_eq!(setup.soldier_total(), 1020);
        let json = serde_json::to_string(&setup).unwrap();
        let back: BattleSetup = serde_json::from_str(&json).unwrap();
        assert_eq!(back, setup);
    }
}
