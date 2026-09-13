//! The render thread (T3-030, REQ-RNDR-007; SAD §8, TDD §10.1 Threading).
//!
//! The main thread builds everything a frame draws (the sprite and line
//! scenes, the tessellated UI, the camera) and hands it over as one owned
//! [`FrameJob`]. The thread owns the `Renderer` (device, surface, buffers,
//! present) and never touches the sim. The hand-over is a one-slot mailbox:
//! a job still waiting when the next arrives is replaced and counted as a
//! dropped frame, so the accumulator never waits on the GPU. The main thread
//! paces itself by waiting until the job was picked up, at most one display
//! period (`RenderHost::wait_taken`). `RenderHost::Direct` runs the same
//! `render_job` inline for `--single-thread-render`, so toggling the flag
//! changes nothing but the profiler rows.

use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use il_data::SpriteSet;

use crate::atlas::Rgba8Image;
use crate::camera::Camera;
use crate::egui_pass::EguiPaint;
use crate::lines::LineScene;
use crate::renderer::{ClearColour, RenderError, Renderer};
use crate::sprite::SpriteScene;
use crate::terrain::TerrainMesh;

/// One frame of tessellated egui output, owned so it can cross the thread
/// (the mirror of [`EguiPaint`], which borrows).
pub struct UiFrame {
    pub textures_delta: egui::TexturesDelta,
    pub primitives: Vec<egui::ClippedPrimitive>,
    pub pixels_per_point: f32,
}

impl<'a> From<&'a mut UiFrame> for EguiPaint<'a> {
    fn from(ui: &'a mut UiFrame) -> Self {
        EguiPaint {
            textures_delta: &mut ui.textures_delta,
            primitives: &ui.primitives,
            pixels_per_point: ui.pixels_per_point,
        }
    }
}

/// A sprite sheet decoded on the main thread, uploaded where the device
/// lives. Ids are assigned in submission order.
pub struct AtlasUpload {
    pub set: SpriteSet,
    pub image: Rgba8Image,
}

/// Everything one frame needs, owned. The render snapshot itself stays on
/// the main thread: the scene is built there, so the thread never reads it.
pub struct FrameJob {
    pub sprites: SpriteScene,
    pub lines: LineScene,
    pub ui: Option<UiFrame>,
    /// Camera for the terrain pass; `None` skips the terrain.
    pub camera: Option<Camera>,
    pub clear: ClearColour,
    /// The window's new inner size, applied before the frame.
    pub resize: Option<(u32, u32)>,
    pub vsync: Option<bool>,
    /// A battle's terrain, uploaded once (set when a battle starts).
    pub terrain: Option<Arc<TerrainMesh>>,
    pub clear_terrain: bool,
    /// Sprite sheets to upload before the frame, in id order.
    pub atlases: Vec<AtlasUpload>,
}

impl Default for FrameJob {
    fn default() -> Self {
        Self {
            sprites: SpriteScene::default(),
            lines: LineScene::default(),
            ui: None,
            camera: None,
            clear: ClearColour::FIELD,
            resize: None,
            vsync: None,
            terrain: None,
            clear_terrain: false,
            atlases: Vec::new(),
        }
    }
}

impl FrameJob {
    /// Empties the job for reuse, keeping the vectors' capacity.
    pub fn clear(&mut self) {
        self.sprites.clear();
        self.lines.clear();
        self.ui = None;
        self.camera = None;
        self.resize = None;
        self.vsync = None;
        self.terrain = None;
        self.clear_terrain = false;
        self.atlases.clear();
    }
}

/// What the profiler shows for the render side.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RenderStats {
    /// Smoothed wall time of one `render_job` (acquire, upload, submit,
    /// present), milliseconds.
    pub render_ms: f32,
    pub last_ms: f32,
    /// Frames rendered since the start.
    pub presented: u64,
    /// Jobs replaced before they were rendered.
    pub dropped: u64,
    /// Smoothed presented frames per second.
    pub fps: f32,
    /// A fatal error on the render side; nothing renders after it.
    pub error: Option<String>,
}

impl RenderStats {
    fn record(&mut self, ms: f32, now: Instant, last: &mut Option<Instant>, dropped: u64) {
        self.last_ms = ms;
        self.render_ms = if self.presented == 0 {
            ms
        } else {
            self.render_ms * 0.9 + ms * 0.1
        };
        if let Some(prev) = *last {
            let dt = now.duration_since(prev).as_secs_f32();
            if dt > 0.0 {
                let fps = 1.0 / dt;
                self.fps = if self.presented <= 1 {
                    fps
                } else {
                    self.fps * 0.9 + fps * 0.1
                };
            }
        }
        *last = Some(now);
        self.presented += 1;
        self.dropped = dropped;
    }
}

struct SlotState<T> {
    pending: Option<T>,
    spent: Option<T>,
    quit: bool,
    dropped: u64,
}

/// The one-slot mailbox between the two threads.
struct JobSlot<T> {
    state: Mutex<SlotState<T>>,
    changed: Condvar,
}

impl<T> JobSlot<T> {
    fn new() -> Self {
        Self {
            state: Mutex::new(SlotState {
                pending: None,
                spent: None,
                quit: false,
                dropped: 0,
            }),
            changed: Condvar::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SlotState<T>> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Puts a job in the slot, replacing (and returning) one still waiting.
    fn put(&self, job: T) -> Option<T> {
        let mut s = self.lock();
        let old = s.pending.replace(job);
        if old.is_some() {
            s.dropped += 1;
        }
        self.changed.notify_all();
        old
    }

    /// Waits for a job; `None` once `quit` was called.
    fn take_blocking(&self) -> Option<T> {
        let mut s = self.lock();
        loop {
            if s.quit {
                return None;
            }
            if let Some(job) = s.pending.take() {
                self.changed.notify_all();
                return Some(job);
            }
            s = self.changed.wait(s).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Waits until the slot is empty (the consumer took the job) or the
    /// deadline passes; true when it was taken in time.
    fn wait_empty(&self, deadline: Instant) -> bool {
        let mut s = self.lock();
        loop {
            if s.pending.is_none() || s.quit {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            s = self
                .changed
                .wait_timeout(s, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    fn recycle_put(&self, job: T) {
        self.lock().spent = Some(job);
    }

    fn recycle_take(&self) -> Option<T> {
        self.lock().spent.take()
    }

    fn dropped(&self) -> u64 {
        self.lock().dropped
    }

    fn quit(&self) {
        self.lock().quit = true;
        self.changed.notify_all();
    }
}

/// The thread that owns the `Renderer`. The renderer is created on the
/// main thread (winit hands out the window handle nowhere else on Windows)
/// and moved here; the surface, device and buffers then live and die on
/// this thread.
pub struct RenderThread {
    slot: Arc<JobSlot<FrameJob>>,
    stats: Arc<Mutex<RenderStats>>,
    handle: Option<JoinHandle<()>>,
}

impl RenderThread {
    /// Spawns the thread and moves the renderer onto it.
    pub fn spawn(mut renderer: Renderer) -> Result<Self, RenderError> {
        let slot = Arc::new(JobSlot::new());
        let stats = Arc::new(Mutex::new(RenderStats::default()));
        let worker_slot = slot.clone();
        let worker_stats = stats.clone();
        let handle = std::thread::Builder::new()
            .name("il_render".to_string())
            .spawn(move || {
                let mut last_present = None;
                while let Some(mut job) = worker_slot.take_blocking() {
                    let start = Instant::now();
                    let result = renderer.render_job(&mut job);
                    let now = Instant::now();
                    let ms = now.duration_since(start).as_secs_f32() * 1000.0;
                    {
                        let mut s = worker_stats.lock().unwrap_or_else(|e| e.into_inner());
                        s.record(ms, now, &mut last_present, worker_slot.dropped());
                        if let Err(e) = result {
                            s.error = Some(e.to_string());
                            break;
                        }
                    }
                    job.clear();
                    worker_slot.recycle_put(job);
                }
            })
            .map_err(|_| RenderError::ThreadStopped)?;
        Ok(Self {
            slot,
            stats,
            handle: Some(handle),
        })
    }

    pub fn submit(&self, job: FrameJob) {
        if let Some(mut replaced) = self.slot.put(job) {
            replaced.clear();
            self.slot.recycle_put(replaced);
        }
    }

    pub fn wait_taken(&self, deadline: Instant) -> bool {
        self.slot.wait_empty(deadline)
    }

    pub fn recycle(&self) -> Option<FrameJob> {
        self.slot.recycle_take()
    }

    pub fn stats(&self) -> RenderStats {
        let mut s = self.stats.lock().unwrap_or_else(|e| e.into_inner()).clone();
        s.dropped = self.slot.dropped();
        s
    }
}

impl Drop for RenderThread {
    fn drop(&mut self) {
        self.slot.quit();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// The single-thread mode's state: the renderer and the stats the thread
/// would otherwise keep.
pub struct DirectHost {
    renderer: Renderer,
    stats: RenderStats,
    last_present: Option<Instant>,
    spare: Option<FrameJob>,
}

/// Where the frames go: the render thread, or the renderer inline on the
/// calling thread (`--single-thread-render`).
pub enum RenderHost {
    Direct(Box<DirectHost>),
    Thread(RenderThread),
}

impl RenderHost {
    /// Creates the renderer on the calling (main) thread, then either keeps
    /// it here or moves it onto the render thread.
    pub fn new(
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        width: u32,
        height: u32,
        single_thread: bool,
    ) -> Result<Self, RenderError> {
        let renderer = Renderer::new(target, width, height)?;
        if single_thread {
            Ok(Self::Direct(Box::new(DirectHost {
                renderer,
                stats: RenderStats::default(),
                last_present: None,
                spare: None,
            })))
        } else {
            Ok(Self::Thread(RenderThread::spawn(renderer)?))
        }
    }

    pub fn is_threaded(&self) -> bool {
        matches!(self, Self::Thread(_))
    }

    /// Hands the frame over (or renders it now in the direct mode).
    pub fn submit(&mut self, mut job: FrameJob) {
        match self {
            Self::Direct(d) => {
                if d.stats.error.is_some() {
                    return;
                }
                let start = Instant::now();
                let result = d.renderer.render_job(&mut job);
                let now = Instant::now();
                d.stats.record(
                    now.duration_since(start).as_secs_f32() * 1000.0,
                    now,
                    &mut d.last_present,
                    0,
                );
                if let Err(e) = result {
                    d.stats.error = Some(e.to_string());
                }
                job.clear();
                d.spare = Some(job);
            }
            Self::Thread(t) => t.submit(job),
        }
    }

    /// Waits until the last job was picked up, at most until `deadline`;
    /// true when it was (always, in the direct mode).
    pub fn wait_taken(&self, deadline: Instant) -> bool {
        match self {
            Self::Direct(_) => true,
            Self::Thread(t) => t.wait_taken(deadline),
        }
    }

    /// A spent job whose buffers can be reused.
    pub fn recycle(&mut self) -> Option<FrameJob> {
        match self {
            Self::Direct(d) => d.spare.take(),
            Self::Thread(t) => t.recycle(),
        }
    }

    pub fn stats(&self) -> RenderStats {
        match self {
            Self::Direct(d) => d.stats.clone(),
            Self::Thread(t) => t.stats(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn put_replaces_a_waiting_job_and_counts_it() {
        let slot = JobSlot::new();
        assert!(slot.put(1).is_none());
        assert_eq!(slot.put(2), Some(1));
        assert_eq!(slot.dropped(), 1);
        assert_eq!(slot.take_blocking(), Some(2));
        assert!(slot.wait_empty(Instant::now()), "nothing pending");
    }

    #[test]
    fn wait_empty_returns_when_taken_and_false_at_the_deadline() {
        let slot = Arc::new(JobSlot::new());
        slot.put(7);
        let start = Instant::now();
        assert!(!slot.wait_empty(start + Duration::from_millis(20)));
        assert!(start.elapsed() >= Duration::from_millis(20));
        let consumer = {
            let slot = slot.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(30));
                slot.take_blocking()
            })
        };
        assert!(slot.wait_empty(Instant::now() + Duration::from_secs(5)));
        assert_eq!(consumer.join().unwrap(), Some(7));
    }

    #[test]
    fn quit_wakes_a_blocked_taker_and_recycle_returns_the_spent_job() {
        let slot = Arc::new(JobSlot::<u8>::new());
        let taker = {
            let slot = slot.clone();
            std::thread::spawn(move || slot.take_blocking())
        };
        std::thread::sleep(Duration::from_millis(20));
        slot.quit();
        assert_eq!(taker.join().unwrap(), None);
        assert!(slot.recycle_take().is_none());
        slot.recycle_put(9);
        assert_eq!(slot.recycle_take(), Some(9));
        assert!(slot.recycle_take().is_none());
    }

    #[test]
    fn stats_smooth_the_render_time_and_count_frames() {
        let mut s = RenderStats::default();
        let mut last = None;
        let t0 = Instant::now();
        s.record(4.0, t0, &mut last, 0);
        assert_eq!(s.render_ms, 4.0);
        assert_eq!(s.presented, 1);
        assert_eq!(s.fps, 0.0, "one frame gives no interval");
        s.record(8.0, t0 + Duration::from_millis(10), &mut last, 3);
        assert!((s.render_ms - 4.4).abs() < 1e-5);
        assert_eq!(s.presented, 2);
        assert_eq!(s.dropped, 3);
        assert!((s.fps - 100.0).abs() < 1.0);
        let mut job = FrameJob {
            clear_terrain: true,
            resize: Some((3, 4)),
            ..FrameJob::default()
        };
        job.clear();
        assert!(!job.clear_terrain && job.resize.is_none());
    }
}
