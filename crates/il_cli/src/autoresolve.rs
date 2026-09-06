//! `il_cli autoresolve` (T2-071, T2-082; REQ-SIM-061, REQ-SIM-063, TDD §9):
//! runs a scenario headless to its end and prints the `BattleResult` as
//! JSON. By default every side is handed to the engine AI before tick 1
//! (plan decision 17) and the scenario's scripted commands are dropped;
//! `--ai none` keeps the scripted run, `--ai 1,2` hands over those players
//! only. A battle that has not ended by `max_ticks` still prints its
//! result, with no winner, and the caller reports it through the exit code.

use std::io::Write;
use std::path::PathBuf;

use anyhow::Context;
use il_core::{PlayerId, Tick};
use il_sim_battle::{BattlePhase, BattleResult, BattleWorld, Command, CommandKind};

/// Which players the engine takes over (`--ai`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AiPlayers {
    All,
    None,
    Players(Vec<PlayerId>),
}

impl AiPlayers {
    /// `all`, `none` or a comma-separated player list (`1,2`).
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        match text.trim() {
            "all" => Ok(AiPlayers::All),
            "none" => Ok(AiPlayers::None),
            list => list
                .split(',')
                .map(|p| {
                    p.trim()
                        .parse::<u8>()
                        .map(PlayerId)
                        .with_context(|| format!("--ai: {p:?} is not a player id"))
                })
                .collect::<anyhow::Result<Vec<_>>>()
                .map(AiPlayers::Players),
        }
    }

    fn takes(&self, player: PlayerId) -> bool {
        player != PlayerId::ENGINE_AI
            && match self {
                AiPlayers::All => true,
                AiPlayers::None => false,
                AiPlayers::Players(list) => list.contains(&player),
            }
    }
}

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
    /// Players handed to the engine at tick 1 (default every player).
    pub ai: AiPlayers,
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
    let mut world = BattleWorld::new(&scenario.setup, regs)?;
    world.set_threads(opts.threads.max(1));
    let max_ticks = opts.max_ticks.unwrap_or_else(|| default_max_ticks(&world));

    // The players the engine takes, ascending and once each; their scripted
    // commands would be `NotOwner`, so they are dropped with a note.
    let mut taken: Vec<PlayerId> = scenario
        .setup
        .sides
        .iter()
        .map(|s| s.player)
        .filter(|p| opts.ai.takes(*p))
        .collect();
    taken.sort();
    taken.dedup();
    let dropped = scenario
        .commands
        .iter()
        .filter(|c| taken.contains(&c.player))
        .count();
    if dropped > 0 {
        eprintln!(
            "autoresolve: {dropped} scripted command(s) of AI-driven player(s) ignored (--ai none keeps them)"
        );
    }
    let kept: Vec<Command> = scenario
        .commands
        .iter()
        .filter(|c| !taken.contains(&c.player))
        .cloned()
        .collect();
    let mut script = il_sim_battle::ScriptedCommands::new(kept);
    let transfers: Vec<Command> = taken
        .iter()
        .enumerate()
        .map(|(seq, from)| Command {
            tick: Tick(1),
            player: PlayerId::ENGINE_AI,
            seq: seq as u16,
            kind: CommandKind::TransferControl {
                from: *from,
                to: PlayerId::ENGINE_AI,
            },
        })
        .collect();

    while world.phase() != BattlePhase::Ended && world.tick().0 < max_ticks {
        let next = world.tick().next();
        let mut commands = script.take_for(next);
        if next == Tick(1) {
            commands.extend(transfers.iter().cloned());
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_players_parse() {
        assert_eq!(AiPlayers::parse("all").unwrap(), AiPlayers::All);
        assert_eq!(AiPlayers::parse(" none ").unwrap(), AiPlayers::None);
        assert_eq!(
            AiPlayers::parse("1, 2").unwrap(),
            AiPlayers::Players(vec![PlayerId(1), PlayerId(2)])
        );
        assert!(AiPlayers::parse("x").is_err());
        assert!(!AiPlayers::All.takes(PlayerId::ENGINE_AI));
        assert!(AiPlayers::Players(vec![PlayerId(1)]).takes(PlayerId(1)));
        assert!(!AiPlayers::Players(vec![PlayerId(1)]).takes(PlayerId(0)));
    }
}
