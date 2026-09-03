//! Iron Legion application shell (TDD §15, SAD §6.1).
//!
//! Phase 1: opens a window, runs a scenario's `BattleWorld` on a fixed-step
//! accumulator and renders it. Menus arrive in T1-070.

mod app;
mod session;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, anyhow};
use clap::Parser;
use il_core::PlayerId;
use il_data::Registries;
use il_sim_battle::{BattleSetup, BattleWorld};
use winit::event_loop::EventLoop;

use crate::app::App;
use crate::session::BattleSession;

#[derive(Parser, Debug)]
#[command(name = "il_app", version, about = "Iron Legion Engine")]
struct Args {
    /// A `BattleSetup` scenario file in JSON5.
    scenario: PathBuf,
    /// Mod root holding `mod.json5` and `content/`.
    #[arg(long, default_value = "game")]
    content_root: PathBuf,
    /// Simulation worker threads (`1` = single-threaded executor).
    #[arg(long, default_value_t = 1)]
    threads: usize,
}

fn load_setup(path: &Path) -> anyhow::Result<BattleSetup> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading scenario {}", path.display()))?;
    json5::from_str(&text).with_context(|| format!("parsing scenario {}", path.display()))
}

fn load_registries(root: &Path) -> anyhow::Result<Arc<Registries>> {
    Registries::load_root(root)
        .map(Arc::new)
        .map_err(|d| anyhow!("content errors:\n{d}"))
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let regs = load_registries(&args.content_root)?;
    let setup = load_setup(&args.scenario)?;
    let mut world = BattleWorld::new(&setup, regs).map_err(|e| anyhow!("{e}"))?;
    world.set_threads(args.threads);
    let session = BattleSession::new(world, PlayerId(0));

    let event_loop = EventLoop::new().context("creating the event loop")?;
    let mut app = App::new(session);
    event_loop
        .run_app(&mut app)
        .context("running the event loop")?;
    Ok(())
}
