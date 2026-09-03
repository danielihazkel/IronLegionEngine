//! Iron Legion headless CLI library (REQ-TOOL-001, TDD §17).
//!
//! The `run` subcommand lives here so integration tests can drive it
//! in-process; `main.rs` is a thin clap wrapper.

pub mod bench;
pub mod genart;
pub mod genmap;
pub mod validate;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, anyhow};
use il_core::{StateHash, Tick};
use il_data::Registries;
use il_data::json5::{FileId, parse_json5};
use il_sim_battle::{BattleSetup, BattleWorld, Scenario, Snapshot};

/// Options of `il_cli run`.
#[derive(Clone, Debug)]
pub struct RunOptions {
    /// A `BattleSetup` in JSON5.
    pub scenario: PathBuf,
    /// Run until this many ticks have completed.
    pub ticks: u32,
    /// Print `tick,hash` every K ticks; `0` prints nothing.
    pub hash_every: u32,
    /// `1` = single-threaded executor.
    pub threads: usize,
    /// Write `snapshot.bin` beside the scenario after this tick and continue.
    pub snapshot_at: Option<u32>,
    /// Start from this snapshot instead of the scenario's initial state.
    pub restore_from: Option<PathBuf>,
    /// Write the hash lines here instead of stdout.
    pub hash_log: Option<PathBuf>,
    /// Mod root holding `mod.json5` and `content/`.
    pub content_root: PathBuf,
    /// Extra mod folders loaded after the game (T1-082).
    pub mods: Vec<PathBuf>,
}

impl RunOptions {
    pub fn new(scenario: impl Into<PathBuf>, ticks: u32) -> Self {
        Self {
            scenario: scenario.into(),
            ticks,
            hash_every: 1,
            threads: 1,
            snapshot_at: None,
            restore_from: None,
            hash_log: None,
            content_root: PathBuf::from("game"),
            mods: Vec::new(),
        }
    }
}

/// Parses a scenario file (a `BattleSetup` plus optional `commands`) with
/// the engine's own JSON5 parser (T1-081).
pub fn load_scenario(path: &Path) -> anyhow::Result<Scenario> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading scenario {}", path.display()))?;
    parse_scenario(&text).with_context(|| format!("parsing scenario {}", path.display()))
}

/// [`load_scenario`] on text already in memory.
pub fn parse_scenario(text: &str) -> anyhow::Result<Scenario> {
    let value = parse_json5(text, FileId(0)).map_err(|e| anyhow!("{e}"))?;
    serde_json::from_value(value.to_json()).map_err(|e| anyhow!("{e}"))
}

/// The setup half of a scenario file.
pub fn load_setup(path: &Path) -> anyhow::Result<BattleSetup> {
    load_scenario(path).map(|s| s.setup)
}

/// Loads the registries from a mod root, turning diagnostics into an error
/// that lists every one of them.
pub fn load_registries(content_root: &Path) -> anyhow::Result<Arc<Registries>> {
    load_registries_with_mods(content_root, &[])
}

/// Loads the game root plus extra mod folders (each holding a `mod.json5`).
pub fn load_registries_with_mods(
    content_root: &Path,
    mods: &[PathBuf],
) -> anyhow::Result<Arc<Registries>> {
    let mut roots = vec![content_root.to_path_buf()];
    roots.extend(mods.iter().cloned());
    Registries::load_roots(&roots)
        .map(Arc::new)
        .map_err(|d| anyhow!("content errors in {}:\n{d}", content_root.display()))
}

/// Path of the snapshot `--snapshot-at` writes: `snapshot.bin` beside the
/// scenario file.
pub fn snapshot_path(scenario: &Path) -> PathBuf {
    scenario
        .parent()
        .map_or_else(|| PathBuf::from("snapshot.bin"), |d| d.join("snapshot.bin"))
}

/// Runs a scenario and returns every `(tick, hash)` pair at the chosen
/// cadence, also writing them as `tick,hash` lines to `out` (or the hash
/// log). The hash is 16 lower-case hex digits.
pub fn run(opts: &RunOptions, out: &mut dyn Write) -> anyhow::Result<Vec<(Tick, StateHash)>> {
    let regs = load_registries_with_mods(&opts.content_root, &opts.mods)?;
    let scenario = load_scenario(&opts.scenario)?;
    let mut script = scenario.script();

    let mut world = match &opts.restore_from {
        Some(path) => {
            let bytes = std::fs::read(path)
                .with_context(|| format!("reading snapshot {}", path.display()))?;
            let snap = Snapshot::from_bytes(&bytes)?;
            let world = BattleWorld::restore(&snap, regs)?;
            // Commands up to the restored tick were consumed by the run
            // that wrote the snapshot.
            script.take_for(world.tick());
            world
        }
        None => BattleWorld::new(&scenario.setup, regs)?,
    };
    world.set_threads(opts.threads);

    let mut log_file = match &opts.hash_log {
        Some(path) => Some(std::io::BufWriter::new(
            std::fs::File::create(path)
                .with_context(|| format!("creating hash log {}", path.display()))?,
        )),
        None => None,
    };

    let mut hashes = Vec::new();
    while world.tick().0 < opts.ticks {
        let commands = script.take_for(world.tick().next());
        let step = world.step(&commands);
        let tick = world.tick();
        if opts.hash_every > 0 && tick.0 % opts.hash_every == 0 {
            hashes.push((tick, step.hash));
            let line = format!("{},{}\n", tick.0, step.hash);
            match log_file.as_mut() {
                Some(f) => f.write_all(line.as_bytes())?,
                None => out.write_all(line.as_bytes())?,
            }
        }
        if opts.snapshot_at == Some(tick.0) {
            let path = snapshot_path(&opts.scenario);
            std::fs::write(&path, world.snapshot().to_bytes())
                .with_context(|| format!("writing snapshot {}", path.display()))?;
        }
    }
    if let Some(f) = log_file.as_mut() {
        f.flush()?;
    }
    Ok(hashes)
}
