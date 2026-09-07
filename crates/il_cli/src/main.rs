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
    /// Regenerate the placeholder battle sounds and the sound set (T2-100).
    Gensound(GensoundArgs),
    /// Load the given mod roots and print every diagnostic; exit 1 on errors.
    Validate(ValidateArgs),
    /// Run the scenario outcome bands over many seeds and print the table (T2-110).
    Bands(BandsArgs),
    /// Run a scenario headless to its end and print the BattleResult as JSON (T2-071).
    Autoresolve(AutoresolveArgs),
    /// Print a replay's header, or re-simulate it and compare every hash (T2-101).
    Replay(ReplayArgs),
}

#[derive(Args)]
struct ReplayArgs {
    /// A replay file (`.ilrp`) written by il_app or `autoresolve --record-replay`.
    file: PathBuf,
    /// Re-simulate with the loaded content and report the first divergent
    /// tick (exit 1); without it only the header prints.
    #[arg(long)]
    verify: bool,
    /// Worker threads for the re-simulation; 1 runs the single-threaded executor.
    #[arg(long, default_value_t = 1)]
    threads: usize,
    /// Mod root with mod.json5 and content/.
    #[arg(long, default_value = "game")]
    content_root: PathBuf,
    /// Extra mod roots loaded after the game, in order.
    #[arg(long = "mod")]
    mods: Vec<PathBuf>,
    /// Verify even when the loaded content's hash differs from the replay's.
    #[arg(long)]
    force: bool,
}

#[derive(Args)]
struct AutoresolveArgs {
    /// Scenario file: a BattleSetup plus optional scripted commands.
    scenario: PathBuf,
    /// Stop after this many ticks even if the battle has not ended (exit 2);
    /// default: the time limit plus the deployment timeout and the pursuit.
    #[arg(long)]
    max_ticks: Option<u32>,
    /// Worker threads; 1 runs the single-threaded executor.
    #[arg(long, default_value_t = 1)]
    threads: usize,
    /// Write the JSON here instead of stdout.
    #[arg(long)]
    json: Option<PathBuf>,
    /// Mod root with mod.json5 and content/.
    #[arg(long, default_value = "game")]
    content_root: PathBuf,
    /// Extra mod roots loaded after the game, in order.
    #[arg(long = "mod")]
    mods: Vec<PathBuf>,
    /// Players the engine AI takes at tick 1: `all` (default; the scenario's
    /// scripted commands are dropped), `none` (scripted run) or `1,2`.
    #[arg(long, default_value = "all")]
    ai: String,
    /// Write the battle's replay (`.ilrp`) here (T2-101).
    #[arg(long)]
    record_replay: Option<PathBuf>,
}

#[derive(Args)]
struct BenchArgs {
    /// Soldier count: a multiple of 200 (2000, 10000, 20000).
    #[arg(long, default_value_t = 2000, conflicts_with = "scenario")]
    soldiers: u32,
    /// Time this scenario file instead of the generated setup (T2-111);
    /// the baseline keys it by the file's stem (`perf_10k`).
    #[arg(long)]
    scenario: Option<PathBuf>,
    /// Ticks to step; the generated command stream spans 600 (the
    /// default), a scenario file gets 1,200.
    #[arg(long)]
    ticks: Option<u32>,
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
struct BandsArgs {
    /// Folder of band files (tests/scenarios/bands) or one file.
    #[arg(default_value = "tests/scenarios/bands")]
    dir: PathBuf,
    /// Override every file's seed count.
    #[arg(long)]
    seeds: Option<u32>,
    /// Cap every file's tick limit (smoke runs).
    #[arg(long)]
    max_ticks: Option<u32>,
    /// Seeds run in parallel on this many threads (each world single-threaded).
    #[arg(long, default_value_t = 4)]
    jobs: usize,
    /// Write the report as JSON.
    #[arg(long)]
    json: Option<PathBuf>,
    /// Mod root with mod.json5 and content/.
    #[arg(long, default_value = "game")]
    content_root: PathBuf,
    /// Extra mod folder to load after the game; repeatable.
    #[arg(long = "mod")]
    mods: Vec<PathBuf>,
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
struct GensoundArgs {
    /// Mod root; samples go to `<root>/assets/sounds/`, the set to
    /// `<root>/content/sounds/battle.json5`.
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
            let ticks = a
                .ticks
                .unwrap_or(if a.scenario.is_some() { 1200 } else { 600 });
            let opts = il_cli::bench::BenchOptions {
                soldiers: a.soldiers,
                scenario: a.scenario,
                ticks,
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
        Command::Gensound(a) => {
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            il_cli::gensound::generate(&a.mod_root, &mut lock)
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
        Command::Bands(a) => {
            let opts = il_cli::bands::BandOptions {
                dir: a.dir,
                seeds: a.seeds,
                max_ticks: a.max_ticks,
                jobs: a.jobs,
                json: a.json,
                content_root: a.content_root,
                mods: a.mods,
            };
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            let report = il_cli::bands::run_bands(&opts, &mut lock)?;
            if report.failed > 0 {
                std::process::exit(1);
            }
            Ok(())
        }
        Command::Autoresolve(a) => {
            let opts = il_cli::autoresolve::AutoresolveOptions {
                scenario: a.scenario,
                max_ticks: a.max_ticks,
                threads: a.threads,
                json: a.json,
                content_root: a.content_root,
                mods: a.mods,
                ai: il_cli::autoresolve::AiPlayers::parse(&a.ai)?,
                record_replay: a.record_replay,
            };
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            let (_, ended) = il_cli::autoresolve::autoresolve(&opts, &mut lock)?;
            if !ended {
                eprintln!("the battle did not end within the tick cap");
                std::process::exit(2);
            }
            Ok(())
        }
        Command::Replay(a) => {
            let opts = il_cli::replay::ReplayOptions {
                file: a.file,
                verify: a.verify,
                threads: a.threads,
                content_root: a.content_root,
                mods: a.mods,
                force: a.force,
            };
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            let outcome = il_cli::replay::replay(&opts, &mut lock)?;
            let code = outcome.exit_code();
            if code != 0 {
                std::process::exit(code);
            }
            Ok(())
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
