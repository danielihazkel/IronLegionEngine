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
    /// Regenerate the placeholder sprite sheets and frame tables (T1-051).
    Genart(GenartArgs),
    /// Load the given mod roots and print every diagnostic; exit 1 on errors.
    Validate(ValidateArgs),
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
            };
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            il_cli::run(&opts, &mut lock)?;
            Ok(())
        }
        Command::Genart(a) => {
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            il_cli::genart::generate(&a.mod_root, &mut lock)
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
