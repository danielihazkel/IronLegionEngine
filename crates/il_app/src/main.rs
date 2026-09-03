//! Iron Legion application shell (TDD §15, SAD §6.1).
//!
//! Phase 1: a main menu that starts a custom battle from a scenario file,
//! the battle state on a fixed-step accumulator, rendered and driven
//! through bindings (T1-070). A scenario on the command line skips the menu.

mod app;
mod bench;
mod profiler;
mod session;
mod state;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, anyhow};
use clap::Parser;
use il_data::Registries;
use winit::event_loop::EventLoop;

use crate::app::{App, Launch, start_battle};
use crate::state::{AppState, MenuState};

#[derive(Parser, Debug)]
#[command(name = "il_app", version, about = "Iron Legion Engine")]
struct Args {
    /// A `BattleSetup` scenario file in JSON5; without it the main menu opens.
    scenario: Option<PathBuf>,
    /// Mod root holding `mod.json5`, `content/` and `assets/`.
    #[arg(long, default_value = "game")]
    content_root: PathBuf,
    /// Extra mod folder to load after the game; repeatable (T1-082).
    #[arg(long = "mod")]
    mods: Vec<PathBuf>,
    /// Folder the main menu lists scenario files from.
    #[arg(long, default_value = "tests/scenarios")]
    scenarios_dir: PathBuf,
    /// Simulation worker threads (`1` = single-threaded executor).
    #[arg(long, default_value_t = 1)]
    threads: usize,
    /// Render 32,768 synthetic sprites with vsync off, print the frame time,
    /// exit (T1-051 acceptance test).
    #[arg(long)]
    bench_sprites: bool,
    /// Show localisation keys instead of strings (REQ-LOC-001 check).
    #[arg(long)]
    show_keys: bool,
}

/// With the `dev` feature the app watches the mod folders and swaps
/// reloaded registries into the sim between ticks (T1-025).
#[cfg(feature = "dev")]
pub type HotReloadHandle = Option<il_data::hot_reload::HotReload>;
#[cfg(not(feature = "dev"))]
pub type HotReloadHandle = ();

#[cfg_attr(not(feature = "dev"), allow(clippy::let_unit_value))]
fn load_registries(
    root: &Path,
    mods: &[PathBuf],
) -> anyhow::Result<(Arc<Registries>, HotReloadHandle)> {
    let mut roots = vec![root.to_path_buf()];
    roots.extend(mods.iter().cloned());
    let set = il_data::discover_set(&roots).map_err(|d| anyhow!("content errors:\n{d}"))?;
    let regs = Arc::new(il_data::load(&set).map_err(|d| anyhow!("content errors:\n{d}"))?);
    let hot_reload = hot_reload_handle(set, &regs);
    Ok((regs, hot_reload))
}

#[cfg(feature = "dev")]
fn hot_reload_handle(set: il_data::ModSet, regs: &Arc<Registries>) -> HotReloadHandle {
    match il_data::hot_reload::HotReload::new(set, regs.clone()) {
        Ok(h) => Some(h),
        Err(e) => {
            eprintln!("hot reload disabled: {e}");
            None
        }
    }
}

#[cfg(not(feature = "dev"))]
fn hot_reload_handle(_set: il_data::ModSet, _regs: &Arc<Registries>) -> HotReloadHandle {}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let (regs, hot_reload) = load_registries(&args.content_root, &args.mods)?;
    regs.locale.set_show_keys(args.show_keys);
    let launch = Launch {
        content_root: args.content_root.clone(),
        mods: args.mods.clone(),
        scenarios_dir: args.scenarios_dir.clone(),
        threads: args.threads,
        bench_sprites: args.bench_sprites,
    };
    let state = match &args.scenario {
        Some(path) => AppState::Battle(Box::new(start_battle(path, regs.clone(), args.threads)?)),
        None => {
            let mut mods = vec![args.content_root.clone()];
            mods.extend(args.mods.iter().cloned());
            AppState::MainMenu(MenuState::scan(&args.scenarios_dir, mods))
        }
    };

    let event_loop = EventLoop::new().context("creating the event loop")?;
    let mut app = App::new(state, launch, regs, hot_reload);
    event_loop
        .run_app(&mut app)
        .context("running the event loop")?;
    Ok(())
}
