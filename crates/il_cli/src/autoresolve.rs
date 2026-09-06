//! `il_cli autoresolve` (T2-071; REQ-SIM-061, REQ-SIM-063, TDD §9): runs a
//! scenario headless to its end and prints the `BattleResult` as JSON. The
//! sides are driven by the scenario's scripted commands only until T2-082
//! supplies the battle AI (plan decision 2); a battle that has not ended by
//! `max_ticks` still prints its result, with no winner, and the caller
//! reports it through the exit code.

use std::io::Write;
use std::path::PathBuf;

use anyhow::Context;
use il_sim_battle::{BattlePhase, BattleResult, BattleWorld};

pub struct AutoresolveOptions {
    pub scenario: PathBuf,
    /// Ticks to run at most; the default is the setup's time limit plus the
    /// deployment timeout and the pursuit length, so a scripted battle
    /// always reaches Ended on its own.
    pub max_ticks: Option<u32>,
    pub threads: usize,
    /// Write the JSON here instead of `out`.
    pub json: Option<PathBuf>,
    pub content_root: PathBuf,
    pub mods: Vec<PathBuf>,
}

/// The default tick cap of a scenario (plan I29).
pub fn default_max_ticks(world: &BattleWorld) -> u32 {
    let rules = &world.registries().rules.battle_flow;
    let time_limit = world
        .setup()
        .map_or(rules.time_limit_ticks, |s| s.time_limit_ticks);
    time_limit
        .saturating_add(rules.deploy_timeout_ticks)
        .saturating_add(rules.pursuit_ticks)
        .saturating_add(1)
}

/// Runs the scenario and returns its result and whether the battle ended.
pub fn autoresolve(
    opts: &AutoresolveOptions,
    out: &mut dyn Write,
) -> anyhow::Result<(BattleResult, bool)> {
    let regs = crate::load_registries_with_mods(&opts.content_root, &opts.mods)?;
    let scenario = crate::load_scenario(&opts.scenario)?;
    let mut script = scenario.script();
    let mut world = BattleWorld::new(&scenario.setup, regs)?;
    world.set_threads(opts.threads.max(1));
    let max_ticks = opts.max_ticks.unwrap_or_else(|| default_max_ticks(&world));
    while world.phase() != BattlePhase::Ended && world.tick().0 < max_ticks {
        let commands = script.take_for(world.tick().next());
        world.step(&commands);
    }
    let ended = world.phase() == BattlePhase::Ended;
    let result = world.result();
    let text = serde_json::to_string_pretty(&result)?;
    match &opts.json {
        Some(path) => std::fs::write(path, text.as_bytes())
            .with_context(|| format!("writing {}", path.display()))?,
        None => {
            out.write_all(text.as_bytes())?;
            out.write_all(b"\n")?;
        }
    }
    Ok((result, ended))
}
