//! The winit application handler: window, renderer, frame loop (T1-050,
//! T1-051).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use il_render::{AtlasId, ClearColour, Renderer, SpriteScene, SpriteSheet};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::bench::SpriteBench;
use crate::session::BattleSession;

/// Frames between title refreshes (the title shows the tick and sim cost).
const TITLE_EVERY_FRAMES: u32 = 15;

/// Unit categories with a placeholder sheet, in `UnitCategory` order.
pub const CATEGORIES: [&str; 6] = [
    "infantry",
    "cavalry",
    "ranged",
    "skirmisher",
    "general",
    "siege",
];

pub enum Mode {
    Battle(Box<BattleSession>),
    BenchSprites,
}

pub struct App {
    mode: Mode,
    content_root: PathBuf,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    /// One atlas per entry of `CATEGORIES`.
    atlases: Vec<AtlasId>,
    scene: SpriteScene,
    bench: Option<SpriteBench>,
    last_frame: Option<Instant>,
    frames: u32,
    /// Wall time spent inside `BattleWorld::step` since the last title refresh.
    step_seconds: f64,
    ticks_since_title: u32,
}

impl App {
    pub fn new(mode: Mode, content_root: PathBuf) -> Self {
        Self {
            mode,
            content_root,
            window: None,
            renderer: None,
            atlases: Vec::new(),
            scene: SpriteScene::default(),
            bench: None,
            last_frame: None,
            frames: 0,
            step_seconds: 0.0,
            ticks_since_title: 0,
        }
    }

    fn load_atlases(&mut self) -> anyhow::Result<()> {
        let renderer = self.renderer.as_mut().expect("renderer exists");
        let assets_root = self.content_root.join("assets");
        for category in CATEGORIES {
            let table = self
                .content_root
                .join("content/sprites")
                .join(format!("{category}.json5"));
            let sheet = SpriteSheet::load(&table)?;
            self.atlases.push(renderer.load_atlas(sheet, &assets_root)?);
        }
        Ok(())
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = self
            .last_frame
            .map_or(0.0, |t| now.duration_since(t).as_secs_f64());
        self.last_frame = Some(now);

        match &mut self.mode {
            Mode::Battle(session) => {
                let before = Instant::now();
                let stepped = session.advance(dt).len() as u32;
                self.step_seconds += before.elapsed().as_secs_f64();
                self.ticks_since_title += stepped;
                // Soldiers are drawn from a RenderSnapshot from T1-052 on.
                self.scene.clear();
            }
            Mode::BenchSprites => {
                if let Some(bench) = self.bench.as_mut() {
                    if self.frames > 0 {
                        bench.record(dt);
                    }
                    if bench.done() {
                        let pass = bench.report();
                        event_loop.exit();
                        if !pass {
                            std::process::exit(2);
                        }
                        return;
                    }
                }
            }
        }

        let scene = match (&self.mode, &self.bench) {
            (Mode::BenchSprites, Some(bench)) => &bench.scene,
            _ => &self.scene,
        };
        if let Some(renderer) = self.renderer.as_mut()
            && let Err(e) = renderer.render(ClearColour::FIELD, scene)
        {
            eprintln!("fatal render error: {e}");
            std::process::exit(1);
        }

        self.frames += 1;
        if self.frames.is_multiple_of(TITLE_EVERY_FRAMES) {
            self.refresh_title();
        }
    }

    /// Temporary keyboard handling until bindings arrive (T1-061): Space
    /// pauses, `+`/`-` change speed.
    fn key(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed || event.repeat {
            return;
        }
        let Mode::Battle(session) = &mut self.mode else {
            return;
        };
        match &event.logical_key {
            Key::Named(NamedKey::Space) => {
                let paused = session.paused();
                session.set_paused(!paused);
            }
            Key::Character(c) if c == "+" || c == "=" => {
                let speed = (session.speed() * 2.0).min(8.0);
                session.set_speed(speed);
            }
            Key::Character(c) if c == "-" => {
                let speed = (session.speed() * 0.5).max(0.125);
                session.set_speed(speed);
            }
            _ => {}
        }
        self.refresh_title();
    }

    fn refresh_title(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let title = match &self.mode {
            Mode::Battle(session) => {
                let per_tick_ms = if self.ticks_since_title > 0 {
                    self.step_seconds * 1000.0 / f64::from(self.ticks_since_title)
                } else {
                    0.0
                };
                format!(
                    "Iron Legion — tick {} — {} soldiers — sim {:.2} ms/tick — speed x{:.2} — {} commands{}",
                    session.world.tick().0,
                    session.world.soldier_count(),
                    per_tick_ms,
                    session.speed(),
                    session.command_log().len(),
                    if session.paused() { " — paused" } else { "" }
                )
            }
            Mode::BenchSprites => format!(
                "Iron Legion — sprite bench — frame {}",
                self.bench.as_ref().map_or(0, |b| b.frames_done)
            ),
        };
        window.set_title(&title);
        self.step_seconds = 0.0;
        self.ticks_since_title = 0;
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("Iron Legion")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 800.0));
        let window = match event_loop.create_window(attributes) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("cannot create the window: {e}");
                event_loop.exit();
                return;
            }
        };
        let size = window.inner_size();
        match Renderer::new(window.clone(), size.width, size.height) {
            Ok(r) => self.renderer = Some(r),
            Err(e) => {
                eprintln!("cannot initialise the renderer: {e}");
                event_loop.exit();
                return;
            }
        }
        if let Err(e) = self.load_atlases() {
            eprintln!("cannot load the sprite sheets: {e:#}");
            event_loop.exit();
            return;
        }
        if matches!(self.mode, Mode::BenchSprites) {
            let renderer = self.renderer.as_mut().expect("renderer exists");
            renderer.set_vsync(false);
            let (w, h) = renderer.size();
            self.bench = Some(SpriteBench::new(&self.atlases, w as f32, h as f32));
        }
        event_loop.set_control_flow(ControlFlow::Poll);
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(r) = self.renderer.as_mut() {
                    r.resize(size.width, size.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => self.key(&event),
            WindowEvent::RedrawRequested => self.frame(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }
}
