//! The winit application handler: window, renderer, camera input, frame loop
//! (T1-050, T1-051, T1-052).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use glam::Vec2;
use il_core::{RegimentId, Scalar};
use il_data::Registries;
use il_render::{
    AtlasId, Camera, ClearColour, DebugFlags, EguiPaint, FrameScene, LineScene, RenderSnapshot,
    Renderer, SetAtlas, SnapshotInput, SpriteScene, TerrainMesh, build_debug_lines, build_snapshot,
    deployment_outlines, scene_from_snapshot,
};
use il_ui::{UiContext, profiler_overlay};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::HotReloadHandle;
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
/// Metres added around the regiments when the starting camera frames them.
const CAMERA_FIT_MARGIN_M: f32 = 60.0;
/// Developer tooling compiled in (`dev` feature): profiler overlay, F1 toggle.
const DEV: bool = cfg!(feature = "dev");

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
    regs: Arc<Registries>,
    #[allow(dead_code, reason = "unused without the dev feature")]
    hot_reload: HotReloadHandle,
    content_root: PathBuf,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    ui: Option<UiContext>,
    profiler: Profiler,
    show_profiler: bool,
    /// F2..F6 overlays (T1-054), `dev` builds only.
    debug: DebugFlags,
    /// One atlas per sprite set, in registry order.
    atlases: Vec<AtlasId>,
    camera: Option<Camera>,
    snapshot: RenderSnapshot,
    scene: SpriteScene,
    lines: LineScene,
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
    pub fn new(
        mode: Mode,
        regs: Arc<Registries>,
        hot_reload: HotReloadHandle,
        content_root: PathBuf,
    ) -> Self {
        Self {
            mode,
            regs,
            hot_reload,
            content_root,
            window: None,
            renderer: None,
            ui: None,
            profiler: Profiler::default(),
            show_profiler: DEV,
            debug: DebugFlags::default(),
            atlases: Vec::new(),
            camera: None,
            snapshot: RenderSnapshot::default(),
            scene: SpriteScene::default(),
            lines: LineScene::default(),
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

    /// Uploads every sprite set of the registry, in registry order, so the
    /// snapshot's sprite-set index maps straight onto `atlases`.
    fn load_atlases(&mut self) -> anyhow::Result<()> {
        let renderer = self.renderer.as_mut().expect("renderer exists");
        let assets_root = self.content_root.join("assets");
        for (_, set) in self.regs.sprite_sets.iter() {
            self.atlases.push(renderer.load_atlas(set, &assets_root)?);
        }
        Ok(())
    }

    /// Builds and uploads the battle's terrain mesh (T1-053).
    fn load_terrain(&mut self) {
        let Mode::Battle(session) = &self.mode else {
            return;
        };
        let mesh = TerrainMesh::build(session.world.map(), &self.regs);
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.set_terrain(&mesh);
        }
    }

    fn screen(&self) -> Vec2 {
        let (w, h) = self.renderer.as_ref().map_or((1, 1), Renderer::size);
        Vec2::new(w as f32, h as f32)
    }

    /// Camera framing every regiment anchor the first time it is needed:
    /// centred on their bounding box, zoomed out (never past the default)
    /// until the box fits the window with a margin.
    fn camera_mut(&mut self) -> &mut Camera {
        if self.camera.is_none() {
            let screen = self.screen();
            let mut camera = match &self.mode {
                Mode::Battle(session) => {
                    let view = session.world.view();
                    let mut min = Vec2::splat(f32::INFINITY);
                    let mut max = Vec2::splat(f32::NEG_INFINITY);
                    for r in view.regiments() {
                        let a = Vec2::new(
                            r.anchor_pos.x.to_f32_render(),
                            r.anchor_pos.y.to_f32_render(),
                        );
                        min = min.min(a);
                        max = max.max(a);
                    }
                    if min.x.is_finite() {
                        let mut cam = Camera::new((min + max) * 0.5);
                        let extent = max - min + Vec2::splat(CAMERA_FIT_MARGIN_M);
                        let fit = (screen.x / extent.x).min(screen.y / (extent.y * cam.pitch));
                        cam.zoom = fit.clamp(Camera::MIN_ZOOM, Camera::DEFAULT_ZOOM);
                        cam
                    } else {
                        Camera::new(Vec2::ZERO)
                    }
                }
                Mode::BenchSprites => Camera::new(Vec2::ZERO),
            };
            camera.zoom = camera.zoom.clamp(Camera::MIN_ZOOM, Camera::MAX_ZOOM);
            self.camera = Some(camera);
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
                #[cfg(feature = "dev")]
                if let Some(hr) = self.hot_reload.as_mut() {
                    if let Some(regs) = hr.poll() {
                        session.world.replace_registries(regs.clone());
                        self.regs = regs;
                    }
                    for event in hr.take_events() {
                        match event {
                            il_data::hot_reload::ReloadEvent::Failed(diags) => {
                                eprintln!("hot reload rejected (previous content kept):\n{diags}");
                            }
                            other => eprintln!("hot reload: {other:?}"),
                        }
                    }
                }
                let before = Instant::now();
                let stepped = session.advance_with(dt, &mut self.profiler).len() as u32;
                self.step_seconds += before.elapsed().as_secs_f64();
                self.ticks_since_title += stepped;
                self.profiler.frame(dt, stepped);
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
            self.lines.clear();
            deployment_outlines(session.world.map(), &camera, screen, &mut self.lines);
            if DEV {
                build_debug_lines(
                    &session.world.view(),
                    self.debug,
                    &camera,
                    screen,
                    &mut self.lines,
                );
            }
            if let Some(renderer) = self.renderer.as_ref() {
                let sets: Vec<SetAtlas<'_>> = self
                    .atlases
                    .iter()
                    .map(|id| SetAtlas {
                        atlas: *id,
                        set: &renderer.atlas(*id).expect("loaded atlas").set,
                    })
                    .collect();
                scene_from_snapshot(&self.snapshot, screen, time, &sets, &mut self.scene);
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
        let (sprites, camera) = match (&self.mode, &self.bench) {
            (Mode::BenchSprites, Some(bench)) => (&bench.scene, None),
            _ => (&self.scene, self.camera),
        };
        let frame_scene = FrameScene {
            clear: ClearColour::FIELD,
            camera,
            sprites,
            lines: &self.lines,
        };
        if let Some(renderer) = self.renderer.as_mut()
            && let Err(e) = renderer.render(&frame_scene, paint.as_mut())
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
    /// arrows pan, Q/E snap-rotate, Space pauses, `+`/`-` change speed, F1
    /// the profiler, F2..F6 the debug overlays (nav grid, slots, paths,
    /// anchors, spatial cells).
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
                KeyCode::F2 if pressed && !event.repeat && DEV => {
                    self.debug.nav_grid = !self.debug.nav_grid;
                }
                KeyCode::F3 if pressed && !event.repeat && DEV => {
                    self.debug.slots = !self.debug.slots;
                }
                KeyCode::F4 if pressed && !event.repeat && DEV => {
                    self.debug.paths = !self.debug.paths;
                }
                KeyCode::F5 if pressed && !event.repeat && DEV => {
                    self.debug.anchors = !self.debug.anchors;
                }
                KeyCode::F6 if pressed && !event.repeat && DEV => {
                    self.debug.spatial_cells = !self.debug.spatial_cells;
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
                ) + &debug_suffix(self.debug)
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

/// ` — dbg: nav slots` for the enabled overlays, empty when none.
fn debug_suffix(flags: DebugFlags) -> String {
    let names = [
        (flags.nav_grid, "nav"),
        (flags.slots, "slots"),
        (flags.paths, "paths"),
        (flags.anchors, "anchors"),
        (flags.spatial_cells, "cells"),
    ];
    let on: Vec<&str> = names.iter().filter(|(f, _)| *f).map(|(_, n)| *n).collect();
    if on.is_empty() {
        String::new()
    } else {
        format!(" — dbg: {}", on.join(" "))
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
        self.load_terrain();
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
