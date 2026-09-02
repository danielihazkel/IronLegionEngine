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
    }
}
