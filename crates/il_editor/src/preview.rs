//! The live nav-grid preview (T3-063, TDD §16): `NavGrid::from_map` on the
//! document as the sim reads it, rebuilt on a worker thread once the edits
//! have been quiet for a few frames, so a stroke never waits on it. The F5
//! overlay draws the newest grid.

use std::sync::Arc;
use std::thread::JoinHandle;

use il_data::Registries;
use il_sim_battle::{LoadedMap, NavGrid};

/// Quiet frames after the last edit before a rebuild starts (the hot
/// reload's cadence).
pub const DEBOUNCE_FRAMES: u8 = 6;

#[derive(Debug, Default)]
pub struct NavPreview {
    /// The newest finished grid.
    pub grid: Option<Arc<NavGrid>>,
    worker: Option<JoinHandle<NavGrid>>,
    /// An edit came after the grid (or the worker) started.
    pending: bool,
    quiet: u8,
}

impl NavPreview {
    pub fn new() -> Self {
        Self::default()
    }

    /// The document changed: rebuild once the edits settle.
    pub fn mark(&mut self) {
        self.pending = true;
        self.quiet = 0;
    }

    /// Whether a rebuild is waiting or running.
    pub fn busy(&self) -> bool {
        self.pending || self.worker.is_some()
    }

    /// Once a frame: takes a finished grid, and starts a worker on `map`
    /// when an edit is pending, the edits have been quiet for
    /// [`DEBOUNCE_FRAMES`] and no worker runs. Returns `true` when a new
    /// grid arrived.
    pub fn poll(&mut self, map: &LoadedMap, regs: &Arc<Registries>) -> bool {
        let mut arrived = false;
        if self.worker.as_ref().is_some_and(JoinHandle::is_finished) {
            let worker = self.worker.take().expect("checked");
            if let Ok(grid) = worker.join() {
                self.grid = Some(Arc::new(grid));
                arrived = true;
            }
        }
        if self.pending && self.worker.is_none() {
            if self.quiet < DEBOUNCE_FRAMES {
                self.quiet += 1;
            } else {
                self.pending = false;
                let map = map.clone();
                let regs = Arc::clone(regs);
                self.worker = Some(std::thread::spawn(move || {
                    NavGrid::from_map(&map, &regs, &regs.rules.movement)
                }));
            }
        }
        arrived
    }

    /// Builds the grid now on this thread (the first frame and the tests);
    /// a running worker's result is dropped.
    pub fn rebuild_now(&mut self, map: &LoadedMap, regs: &Registries) {
        self.worker = None;
        self.pending = false;
        self.grid = Some(Arc::new(NavGrid::from_map(map, regs, &regs.rules.movement)));
    }
}
