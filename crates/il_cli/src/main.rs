//! `il_cli`: headless scenario runner and hash printer (REQ-TOOL-001).

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use il_cli::RunOptions;

#[derive(Parser)]
#[command(name = "il_cli", version, about = "Iron Legion headless tools")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a scenario for N ticks and print `tick,hash` lines.
    Run(RunArgs),
    /// Time every schedule stage on a generated move/reform battle (T1-080).
    Bench(BenchArgs),
    /// Regenerate the placeholder sprite sheets and frame tables (T1-051).
    Genart(GenartArgs),
    /// Regenerate the Phase 1 test map and its heightmap (T1-030).
    Genmap(GenmapArgs),
    /// Load the given mod roots and print every diagnostic; exit 1 on errors.
    Validate(ValidateArgs),
}

#[derive(Args)]
struct BenchArgs {
    /// Soldier count: a multiple of 200 (2000, 10000, 20000).
    #[arg(long, default_value_t = 2000)]
    soldiers: u32,
    /// Ticks to step; the scripted command stream spans 600.
    #[arg(long, default_value_t = 600)]
    ticks: u32,
    /// Worker threads; 1 runs the single-threaded executor.
    #[arg(long, default_value_t = 8)]
    threads: usize,
    /// Mod root with mod.json5 and content/.
    #[arg(long, default_value = "game")]
    content_root: PathBuf,
    /// Write the report as JSON.
    #[arg(long)]
    json: Option<PathBuf>,
    /// Compare stage means against this baseline (benches/baseline.json).
    #[arg(long)]
    baseline: Option<PathBuf>,
    /// With --baseline: exit 1 when any stage is more than 20 % slower.
    #[arg(long)]
    strict: bool,
    /// Insert this run into a baseline file (created if missing).
    #[arg(long)]
    record_baseline: Option<PathBuf>,
    /// Machine description stored with --record-baseline.
    #[arg(long)]
    machine: Option<String>,
    /// Date stored with --record-baseline.
    #[arg(long)]
    recorded: Option<String>,
}

#[derive(Args)]
struct ValidateArgs {
    /// Mod roots; the first is the game.
    #[arg(default_value = "game")]
    roots: Vec<PathBuf>,
    /// Fail on warnings too.
    #[arg(long)]
    deny_warnings: bool,
    /// Print the load order, hashes and registry counts.
    #[arg(long)]
    verbose: bool,
}

#[derive(Args)]
struct GenartArgs {
    /// Mod root; sheets go to `<root>/assets/sprites/units/`, frame tables to
    /// `<root>/content/sprites/`.
    #[arg(long, default_value = "game")]
    mod_root: PathBuf,
}

#[derive(Args)]
struct GenmapArgs {
    /// Mod root; the map goes to `<root>/content/maps/`, the heightmap to
    /// `<root>/assets/maps/`.
    #[arg(long, default_value = "game")]
    mod_root: PathBuf,
    /// ContentId of the map.
    #[arg(long, default_value = "rome:test_field")]
    id: String,
    #[arg(long, default_value_t = 7)]
    seed: u64,
}

#[derive(Args)]
struct RunArgs {
    /// Scenario file: a BattleSetup in JSON5.
    scenario: PathBuf,
    /// Number of ticks to simulate.
    #[arg(long)]
    ticks: u32,
    /// Print a hash line every K ticks (0 = never).
    #[arg(long, default_value_t = 1)]
    hash_every: u32,
    /// Worker threads; 1 runs the single-threaded executor.
    #[arg(long, default_value_t = 1)]
    threads: usize,
    /// Write snapshot.bin beside the scenario after this tick and continue.
    #[arg(long)]
    snapshot_at: Option<u32>,
    /// Start from a snapshot written by --snapshot-at.
    #[arg(long)]
    restore_from: Option<PathBuf>,
    /// Write hash lines to this file instead of stdout.
    #[arg(long)]
    hash_log: Option<PathBuf>,
    /// Mod root with mod.json5 and content/.
    #[arg(long, default_value = "game")]
    content_root: PathBuf,
    /// Extra mod folder to load after the game; repeatable.
    #[arg(long = "mod")]
    mods: Vec<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Run(a) => {
            let opts = RunOptions {
                scenario: a.scenario,
                ticks: a.ticks,
                hash_every: a.hash_every,
                threads: a.threads,
                snapshot_at: a.snapshot_at,
                restore_from: a.restore_from,
                hash_log: a.hash_log,
                content_root: a.content_root,
                mods: a.mods,
            };
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            il_cli::run(&opts, &mut lock)?;
            Ok(())
        }
        Command::Bench(a) => {
            let opts = il_cli::bench::BenchOptions {
                soldiers: a.soldiers,
                ticks: a.ticks,
                threads: a.threads,
                content_root: a.content_root,
                json: a.json,
                baseline: a.baseline,
                strict: a.strict,
                record_baseline: a.record_baseline,
                machine: a.machine,
                recorded: a.recorded,
            };
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            let (_, regressions) = il_cli::bench::bench(&opts, &mut lock)?;
            if opts.strict && !regressions.is_empty() {
                return Err(il_cli::bench::strict_error(&regressions));
            }
            Ok(())
        }
        Command::Genart(a) => {
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            il_cli::genart::generate(&a.mod_root, &mut lock)
        }
        Command::Genmap(a) => {
            let opts = il_cli::genmap::GenmapOptions {
                mod_root: a.mod_root,
                id: a.id,
                seed: a.seed,
            };
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            il_cli::genmap::generate(&opts, &mut lock)
        }
        Command::Validate(a) => {
            let opts = il_cli::validate::ValidateOptions {
                roots: a.roots,
                deny_warnings: a.deny_warnings,
                verbose: a.verbose,
            };
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            let report = il_cli::validate::validate(&opts, &mut lock)?;
            if !report.ok(opts.deny_warnings) {
                std::process::exit(1);
            }
            Ok(())
        }
    }
}
