//! Iron Legion application shell (TDD §15, SAD §6.1).
//!
//! Phase 1: opens a window, runs a scenario's `BattleWorld` on a fixed-step
//! accumulator and renders it. Menus arrive in T1-070.

mod app;
mod bench;
mod profiler;
mod session;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, anyhow};
use clap::Parser;
use il_core::PlayerId;
use il_data::Registries;
use il_sim_battle::{BattleSetup, BattleWorld};
use winit::event_loop::EventLoop;

use crate::app::{App, Mode};
use crate::session::BattleSession;

#[derive(Parser, Debug)]
#[command(name = "il_app", version, about = "Iron Legion Engine")]
struct Args {
    /// A `BattleSetup` scenario file in JSON5 (not needed with --bench-sprites).
    scenario: Option<PathBuf>,
    /// Mod root holding `mod.json5`, `content/` and `assets/`.
    #[arg(long, default_value = "game")]
    content_root: PathBuf,
    /// Simulation worker threads (`1` = single-threaded executor).
    #[arg(long, default_value_t = 1)]
    threads: usize,
    /// Render 32,768 synthetic sprites with vsync off, print the frame time,
    /// exit (T1-051 acceptance test).
    #[arg(long)]
    bench_sprites: bool,
    /// Walk every regiment around a circle so interpolation and facing can be
    /// checked before movement exists (T1-052 acceptance check).
    #[arg(long)]
    demo_circle: bool,
    /// Show localisation keys instead of strings (REQ-LOC-001 check).
    #[arg(long)]
    show_keys: bool,
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
    regs.locale.set_show_keys(args.show_keys);
    let mode = if args.bench_sprites {
        Mode::BenchSprites
    } else {
        let scenario = args
            .scenario
            .as_deref()
            .ok_or_else(|| anyhow!("a scenario file is required (or pass --bench-sprites)"))?;
        let setup = load_setup(scenario)?;
        let mut world = BattleWorld::new(&setup, regs.clone()).map_err(|e| anyhow!("{e}"))?;
        world.set_threads(args.threads);
        Mode::Battle(Box::new(BattleSession::new(world, PlayerId(0))))
    };

    let event_loop = EventLoop::new().context("creating the event loop")?;
    let mut app = App::new(mode, regs, args.content_root, args.demo_circle);
    event_loop
        .run_app(&mut app)
        .context("running the event loop")?;
    Ok(())
}
