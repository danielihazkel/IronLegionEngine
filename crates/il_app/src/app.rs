//! The winit application handler: window, renderer, camera input, frame loop
//! (T1-050, T1-051, T1-052).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use glam::Vec2;
use il_core::{Angle, RegimentId, S, Scalar, V2};
use il_render::{
    AtlasId, Camera, CategoryAtlas, ClearColour, EguiPaint, RenderSnapshot, Renderer,
    SnapshotInput, SpriteScene, SpriteSheet, build_snapshot, scene_from_snapshot,
};
use il_ui::{UiContext, profiler_overlay};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::bench::SpriteBench;
use crate::profiler::Profiler;
use crate::session::BattleSession;

/// Frames between title refreshes (the title shows the tick and sim cost).
const TITLE_EVERY_FRAMES: u32 = 15;
/// Keyboard pan speed in screen pixels per second.
const KEY_PAN_PX_PER_S: f32 = 700.0;
/// Edge-scroll band in pixels and speed in pixels per second.
const EDGE_BAND_PX: f32 = 10.0;
const EDGE_PAN_PX_PER_S: f32 = 600.0;
/// Zoom factor per mouse-wheel line.
const WHEEL_ZOOM_STEP: f32 = 1.15;
/// Developer tooling compiled in (`dev` feature): profiler overlay, F1 toggle.
const DEV: bool = cfg!(feature = "dev");

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

#[derive(Default)]
struct PanKeys {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
}

pub struct App {
    mode: Mode,
    content_root: PathBuf,
    /// `--demo-circle`: walk every regiment around a circle (T1-052 check).
    demo_circle: bool,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    ui: Option<UiContext>,
    profiler: Profiler,
    show_profiler: bool,
    /// One atlas per entry of `CATEGORIES`.
    atlases: Vec<AtlasId>,
    camera: Option<Camera>,
    snapshot: RenderSnapshot,
    scene: SpriteScene,
    selected: BTreeSet<RegimentId>,
    bench: Option<SpriteBench>,
    pan_keys: PanKeys,
    cursor: Option<Vec2>,
    middle_down: bool,
    started: Instant,
    last_frame: Option<Instant>,
    frames: u32,
    /// Wall time spent inside `BattleWorld::step` since the last title refresh.
    step_seconds: f64,
    ticks_since_title: u32,
}

impl App {
    pub fn new(mode: Mode, content_root: PathBuf, demo_circle: bool) -> Self {
        Self {
            mode,
            content_root,
            demo_circle,
            window: None,
            renderer: None,
            ui: None,
            profiler: Profiler::default(),
            show_profiler: DEV,
            atlases: Vec::new(),
            camera: None,
            snapshot: RenderSnapshot::default(),
            scene: SpriteScene::default(),
            selected: BTreeSet::new(),
            bench: None,
            pan_keys: PanKeys::default(),
            cursor: None,
            middle_down: false,
            started: Instant::now(),
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

    fn screen(&self) -> Vec2 {
        let (w, h) = self.renderer.as_ref().map_or((1, 1), Renderer::size);
        Vec2::new(w as f32, h as f32)
    }

    /// Camera centred on the mean regiment anchor the first time it is needed.
    fn camera_mut(&mut self) -> &mut Camera {
        if self.camera.is_none() {
            let center = match &self.mode {
                Mode::Battle(session) => {
                    let view = session.world.view();
                    let mut sum = Vec2::ZERO;
                    let mut n = 0.0;
                    for r in view.regiments() {
                        sum += Vec2::new(
                            r.anchor_pos.x.to_f32_render(),
                            r.anchor_pos.y.to_f32_render(),
                        );
                        n += 1.0;
                    }
                    if n > 0.0 { sum / n } else { Vec2::ZERO }
                }
                Mode::BenchSprites => Vec2::ZERO,
            };
            self.camera = Some(Camera::new(center));
        }
        self.camera.as_mut().expect("camera set above")
    }

    fn apply_camera_input(&mut self, dt: f32) {
        let screen = self.screen();
        let mut pan = Vec2::ZERO;
        if self.pan_keys.left {
            pan.x += 1.0;
        }
        if self.pan_keys.right {
            pan.x -= 1.0;
        }
        if self.pan_keys.up {
            pan.y += 1.0;
        }
        if self.pan_keys.down {
            pan.y -= 1.0;
        }
        if pan != Vec2::ZERO {
            pan = pan.normalize() * KEY_PAN_PX_PER_S * dt;
        }
        if let Some(c) = self.cursor
            && !self.middle_down
        {
            let mut edge = Vec2::ZERO;
            if c.x <= EDGE_BAND_PX {
                edge.x += 1.0;
            } else if c.x >= screen.x - EDGE_BAND_PX {
                edge.x -= 1.0;
            }
            if c.y <= EDGE_BAND_PX {
                edge.y += 1.0;
            } else if c.y >= screen.y - EDGE_BAND_PX {
                edge.y -= 1.0;
            }
            pan += edge * EDGE_PAN_PX_PER_S * dt;
        }
        if pan != Vec2::ZERO {
            self.camera_mut().pan_screen(pan);
        }
    }

    /// Moves every regiment along a 40 m circle at 20 Hz: prev/current
    /// positions differ every tick, so interpolation stutter is visible.
    fn demo_circle_step(session: &mut BattleSession) {
        let t = session.world.tick().0 as f32 * il_core::TICK_SECONDS;
        let omega = std::f32::consts::TAU / 40.0;
        let radius = 40.0;
        let v = Vec2::new(-(omega * t).sin(), (omega * t).cos()) * radius * omega;
        let delta = v * il_core::TICK_SECONDS;
        let delta = V2::from_f32_data(delta.x, delta.y);
        let facing = Angle::<S>::from_direction(delta);
        session.world.debug_translate_all(delta, Some(facing));
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = self
            .last_frame
            .map_or(0.0, |t| now.duration_since(t).as_secs_f64());
        self.last_frame = Some(now);
        self.apply_camera_input(dt as f32);
        let screen = self.screen();
        let time = self.started.elapsed().as_secs_f32();

        match &mut self.mode {
            Mode::Battle(session) => {
                let before = Instant::now();
                let stepped = session.advance_with(dt, &mut self.profiler).len() as u32;
                self.step_seconds += before.elapsed().as_secs_f64();
                self.ticks_since_title += stepped;
                self.profiler.frame(dt, stepped);
                if self.demo_circle {
                    for _ in 0..stepped {
                        Self::demo_circle_step(session);
                    }
                }
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

        if let Mode::Battle(_) = &self.mode {
            let camera = *self.camera_mut();
            let Mode::Battle(session) = &self.mode else {
                unreachable!()
            };
            let input = SnapshotInput {
                alpha: session.alpha(),
                camera,
                screen,
                selected: &self.selected,
            };
            build_snapshot(&session.world.view(), &input, &mut self.snapshot);
            if let Some(renderer) = self.renderer.as_ref() {
                let categories: Vec<CategoryAtlas<'_>> = self
                    .atlases
                    .iter()
                    .map(|id| CategoryAtlas {
                        atlas: *id,
                        sheet: &renderer.atlas(*id).expect("loaded atlas").sheet,
                    })
                    .collect();
                scene_from_snapshot(&self.snapshot, screen, time, &categories, &mut self.scene);
            }
        }

        let mut ui_out = match (self.ui.as_mut(), self.window.as_ref(), &self.mode) {
            (Some(ui), Some(window), Mode::Battle(session)) => {
                let mut stats = self.profiler.stats();
                stats.soldiers = self.snapshot.counts.soldiers;
                stats.regiments = self.snapshot.counts.regiments;
                stats.visible_soldiers = self.snapshot.counts.visible_soldiers;
                stats.accumulator_alpha = session.alpha();
                let show = self.show_profiler;
                Some(ui.run(window, |ctx| {
                    if show {
                        profiler_overlay(ctx, &stats);
                    }
                }))
            }
            _ => None,
        };
        let mut paint = ui_out.as_mut().map(|o| EguiPaint {
            textures_delta: &mut o.textures_delta,
            primitives: &o.primitives,
            pixels_per_point: o.pixels_per_point,
        });
        let scene = match (&self.mode, &self.bench) {
            (Mode::BenchSprites, Some(bench)) => &bench.scene,
            _ => &self.scene,
        };
        if let Some(renderer) = self.renderer.as_mut()
            && let Err(e) = renderer.render(ClearColour::FIELD, scene, paint.as_mut())
        {
            eprintln!("fatal render error: {e}");
            std::process::exit(1);
        }

        self.frames += 1;
        if self.frames.is_multiple_of(TITLE_EVERY_FRAMES) {
            self.refresh_title();
        }
    }

    /// Temporary keyboard handling until bindings arrive (T1-061): WASD and
    /// arrows pan, Q/E snap-rotate, Space pauses, `+`/`-` change speed.
    fn key(&mut self, event: &KeyEvent) {
        let pressed = event.state == ElementState::Pressed;
        if let PhysicalKey::Code(code) = event.physical_key {
            match code {
                KeyCode::KeyW | KeyCode::ArrowUp => self.pan_keys.up = pressed,
                KeyCode::KeyS | KeyCode::ArrowDown => self.pan_keys.down = pressed,
                KeyCode::KeyA | KeyCode::ArrowLeft => self.pan_keys.left = pressed,
                KeyCode::KeyD | KeyCode::ArrowRight => self.pan_keys.right = pressed,
                KeyCode::KeyQ if pressed && !event.repeat => self.camera_mut().rotate(-1),
                KeyCode::KeyE if pressed && !event.repeat => self.camera_mut().rotate(1),
                KeyCode::F1 if pressed && !event.repeat && DEV => {
                    self.show_profiler = !self.show_profiler;
                }
                _ => {}
            }
        }
        if !pressed || event.repeat {
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
            _ => return,
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
                let cam = self.camera.unwrap_or_else(|| Camera::new(Vec2::ZERO));
                format!(
                    "Iron Legion — tick {} — {}/{} soldiers drawn — sim {:.2} ms/tick — speed x{:.2} — {} commands — zoom {:.1} px/m rot {}{}",
                    session.world.tick().0,
                    self.snapshot.counts.visible_soldiers,
                    self.snapshot.counts.soldiers,
                    per_tick_ms,
                    session.speed(),
                    session.command_log().len(),
                    cam.zoom,
                    cam.rotation,
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
        self.ui = Some(UiContext::new(&window));
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
        if let (Some(ui), Some(window)) = (self.ui.as_mut(), self.window.as_ref()) {
            let consumed = ui.on_window_event(window, &event);
            let always = matches!(
                event,
                WindowEvent::CloseRequested
                    | WindowEvent::Resized(_)
                    | WindowEvent::RedrawRequested
            );
            if consumed && !always {
                return;
            }
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(r) = self.renderer.as_mut() {
                    r.resize(size.width, size.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => self.key(&event),
            WindowEvent::CursorMoved { position, .. } => {
                let p = Vec2::new(position.x as f32, position.y as f32);
                if self.middle_down
                    && let Some(prev) = self.cursor
                {
                    self.camera_mut().pan_screen(p - prev);
                }
                self.cursor = Some(p);
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor = None;
                self.middle_down = false;
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Middle,
                ..
            } => self.middle_down = state == ElementState::Pressed,
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                let screen = self.screen();
                let anchor = self.cursor.unwrap_or(screen * 0.5);
                self.camera_mut()
                    .zoom_at(WHEEL_ZOOM_STEP.powf(lines), anchor, screen);
            }
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
