//! The winit application handler: window, renderer, input, frame loop
//! (T1-050, T1-051, T1-052, T1-061, T1-062).
//!
//! Every key and mouse gesture goes through `il_ui::InputState` and the
//! `Bindings` loaded from `content/input/bindings.json5`; nothing here names
//! a key code (REQ-INP-005).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use glam::Vec2;
use il_core::{PlayerId, RegimentId, Scalar, V2};
use il_data::Registries;
use il_render::{
    AtlasId, Camera, ClearColour, DebugFlags, EguiPaint, FrameScene, LineScene, RenderSnapshot,
    Renderer, SetAtlas, SnapshotInput, SpriteScene, TerrainMesh, build_debug_lines, build_snapshot,
    deployment_outlines, ground_height, scene_from_snapshot,
};
use il_sim_battle::{BattleView, SpeedMode};
use il_ui::{
    Action, Bindings, DragFormation, Gesture, InputState, OrderContext, Selection, UiContext,
    UiIntent, commands_for, drag_formation, drag_formation_preview, own_regiments, pick_regiment,
    profiler_overlay, regiments_in_box, regiments_of_type_on_screen, selection_box,
    selection_centroid,
};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow};
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
/// Zoom factor per mouse-wheel line or key press.
const WHEEL_ZOOM_STEP: f32 = 1.15;
/// Metres added around the regiments when the starting camera frames them.
const CAMERA_FIT_MARGIN_M: f32 = 60.0;
/// Speed multiplier range for the speed keys.
const MIN_SPEED: f32 = 0.125;
const MAX_SPEED: f32 = 8.0;
/// Length of the drag preview's facing arrow as a fraction of the drag width.
const PREVIEW_ARROW_FRACTION: f32 = 0.25;
/// The preview arrow is never shorter than this many metres.
const PREVIEW_ARROW_MIN_M: f32 = 6.0;
/// Highest formation hotkey the app polls (`formation_1`..`formation_9`).
const FORMATION_HOTKEYS: u8 = 9;
/// Developer tooling compiled in (`dev` feature): profiler and debug overlays.
const DEV: bool = cfg!(feature = "dev");

pub enum Mode {
    Battle(Box<BattleSession>),
    BenchSprites,
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
    input: InputState,
    bindings: Bindings,
    selection: Selection,
    /// The run toggle: new movement orders run instead of walk.
    run: bool,
    profiler: Profiler,
    show_profiler: bool,
    /// Debug overlays (T1-054), `dev` builds only.
    debug: DebugFlags,
    /// One atlas per sprite set, in registry order.
    atlases: Vec<AtlasId>,
    camera: Option<Camera>,
    snapshot: RenderSnapshot,
    scene: SpriteScene,
    lines: LineScene,
    bench: Option<SpriteBench>,
    started: Instant,
    last_frame: Option<Instant>,
    frames: u32,
    /// Wall time spent inside `BattleWorld::step` since the last title refresh.
    step_seconds: f64,
    ticks_since_title: u32,
}

/// Parses the registry's bindings, printing what it had to skip.
fn load_bindings(regs: &Registries) -> Bindings {
    let (bindings, errors) = Bindings::from_content(&regs.input);
    for e in errors {
        eprintln!("bindings: {e}");
    }
    bindings
}

impl App {
    pub fn new(
        mode: Mode,
        regs: Arc<Registries>,
        hot_reload: HotReloadHandle,
        content_root: PathBuf,
    ) -> Self {
        let bindings = load_bindings(&regs);
        Self {
            mode,
            regs,
            hot_reload,
            content_root,
            window: None,
            renderer: None,
            ui: None,
            input: InputState::new(),
            bindings,
            selection: Selection::new(),
            run: false,
            profiler: Profiler::default(),
            show_profiler: DEV,
            debug: DebugFlags::default(),
            atlases: Vec::new(),
            camera: None,
            snapshot: RenderSnapshot::default(),
            scene: SpriteScene::default(),
            lines: LineScene::default(),
            bench: None,
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

    /// Camera bindings (REQ-INP-004): key pan, edge scroll, snap rotation,
    /// wheel and key zoom about the cursor, drag pan.
    fn apply_camera_input(&mut self, dt: f32) {
        let screen = self.screen();
        let b = &self.bindings;
        let input = &self.input;
        let mut pan = Vec2::ZERO;
        if input.held(b, Action::CameraPanLeft) {
            pan.x += 1.0;
        }
        if input.held(b, Action::CameraPanRight) {
            pan.x -= 1.0;
        }
        if input.held(b, Action::CameraPanUp) {
            pan.y += 1.0;
        }
        if input.held(b, Action::CameraPanDown) {
            pan.y -= 1.0;
        }
        if pan != Vec2::ZERO {
            pan = pan.normalize() * KEY_PAN_PX_PER_S * dt;
        }
        let drag = input.drag(b, Action::CameraDrag);
        if let Some(c) = input.cursor()
            && drag.is_none()
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
        if drag.is_some() {
            pan += input.cursor_delta();
        }
        let rotate = i8::from(input.pressed(b, Action::CameraRotateRight))
            - i8::from(input.pressed(b, Action::CameraRotateLeft));
        let zoom_lines = input.wheel_for(b, Action::CameraZoomIn)
            - input.wheel_for(b, Action::CameraZoomOut)
            + f32::from(input.pressed(b, Action::CameraZoomIn))
            - f32::from(input.pressed(b, Action::CameraZoomOut));
        let anchor = input.cursor().unwrap_or(screen * 0.5);

        if pan != Vec2::ZERO {
            self.camera_mut().pan_screen(pan);
        }
        if rotate != 0 {
            self.camera_mut().rotate(rotate);
        }
        if zoom_lines != 0.0 {
            self.camera_mut()
                .zoom_at(WHEEL_ZOOM_STEP.powf(zoom_lines), anchor, screen);
        }
    }

    /// Developer toggles, pause and speed (bindings `toggle_profiler`,
    /// `debug_*`, `pause`, `speed_up`, `speed_down`).
    fn apply_toggles(&mut self) {
        let b = &self.bindings;
        let input = &self.input;
        if DEV {
            if input.pressed(b, Action::ToggleProfiler) {
                self.show_profiler = !self.show_profiler;
            }
            let flags = &mut self.debug;
            for (action, flag) in [
                (Action::DebugNavGrid, &mut flags.nav_grid),
                (Action::DebugSlots, &mut flags.slots),
                (Action::DebugPaths, &mut flags.paths),
                (Action::DebugAnchors, &mut flags.anchors),
                (Action::DebugSpatial, &mut flags.spatial_cells),
            ] {
                if input.pressed(b, action) {
                    *flag = !*flag;
                }
            }
        }
        let Mode::Battle(session) = &mut self.mode else {
            return;
        };
        let mut changed = false;
        if input.pressed(b, Action::Pause) {
            let paused = session.paused();
            session.set_paused(!paused);
            changed = true;
        }
        if input.pressed(b, Action::SpeedUp) {
            session.set_speed((session.speed() * 2.0).min(MAX_SPEED));
            changed = true;
        }
        if input.pressed(b, Action::SpeedDown) {
            session.set_speed((session.speed() * 0.5).max(MIN_SPEED));
            changed = true;
        }
        if changed {
            self.refresh_title();
        }
    }

    /// Selection gestures (REQ-INP-002): click, shift-click, box, double
    /// click by type, select all, control groups. Only the local player's
    /// regiments can be selected.
    fn apply_selection_input(&mut self) {
        let Mode::Battle(session) = &self.mode else {
            return;
        };
        let Some(camera) = self.camera else {
            return;
        };
        let screen = self.screen();
        let player = session.local_player();
        let view = session.world.view();
        let b = &self.bindings;
        let input = &self.input;
        let picker = Picker {
            view: &view,
            camera,
            screen,
            player,
        };

        // Gestures, most specific first: a double click is also a click, so
        // the type selection must win over the plain one.
        if let Some(Gesture::Click { pos, .. }) = input.gesture(b, Action::SelectType) {
            let hit = picker.pick(pos);
            if let Some(id) = hit {
                let ids = regiments_of_type_on_screen(&view, &picker.project(), player, id, screen);
                self.selection.set(ids);
            } else {
                self.selection.click(None, false);
            }
        } else if let Some(Gesture::Click { pos, .. }) = input.gesture(b, Action::SelectAdd) {
            self.selection.click(picker.pick(pos), true);
        } else if let Some(Gesture::Click { pos, .. }) = input.gesture(b, Action::Select) {
            self.selection.click(picker.pick(pos), false);
        }
        if let Some(Gesture::DragEnd { from, to, .. }) = input.gesture(b, Action::BoxSelectAdd) {
            self.selection.box_select(picker.in_box(from, to), true);
        } else if let Some(Gesture::DragEnd { from, to, .. }) = input.gesture(b, Action::BoxSelect)
        {
            self.selection.box_select(picker.in_box(from, to), false);
        }
        if input.pressed(b, Action::SelectAll) {
            self.selection.set(own_regiments(&view, player));
        }
        for n in 0..il_ui::GROUPS {
            let group = n as u8;
            if input.pressed(b, Action::GroupSet(group)) {
                self.selection.set_group(n);
            }
            if input.pressed(b, Action::GroupRecall(group)) {
                self.selection.recall_group(n, input.mods().shift);
            }
        }
        // Regiments that died or changed hands leave every set.
        let own = own_regiments(&view, player);
        self.selection.retain(|id| own.contains(&id));
    }

    /// Orders (REQ-INP-003): right click moves, right drag lays a line
    /// (T1-062 gesture), halt, run toggle, formation hotkeys. Every intent
    /// becomes Commands queued on the session (REQ-INP-006).
    fn apply_order_input(&mut self) {
        let screen = self.screen();
        let Some(camera) = self.camera else {
            return;
        };
        if self.selection.is_empty() {
            return;
        }
        let Mode::Battle(session) = &mut self.mode else {
            return;
        };
        let b = &self.bindings;
        let input = &self.input;
        let unproject = |p: Vec2| camera.screen_to_world(p, screen);
        let mut intents: Vec<UiIntent> = Vec::new();
        if let Some(Gesture::DragEnd { from, to, .. }) =
            input.gesture(b, Action::OrderDragFormation)
        {
            let centroid = selection_centroid(&session.world.view(), &self.selection.regiments)
                .unwrap_or(unproject(from));
            let flip = input.held(b, Action::OrderFlipFacing);
            if let Some(drag) = drag_formation(unproject(from), unproject(to), centroid, flip) {
                intents.push(UiIntent::DragFormation(drag));
            }
        } else if let Some(Gesture::Click { pos, .. }) = input.gesture(b, Action::OrderMove) {
            intents.push(UiIntent::Move {
                target: unproject(pos),
            });
        }
        if input.pressed(b, Action::OrderHalt) {
            intents.push(UiIntent::Halt);
        }
        if input.pressed(b, Action::ToggleRun) {
            self.run = !self.run;
            intents.push(UiIntent::SpeedMode(speed_mode(self.run)));
        }
        for n in 1..=FORMATION_HOTKEYS {
            if input.pressed(b, Action::Formation(n)) {
                intents.push(UiIntent::Formation(n));
            }
        }
        if intents.is_empty() {
            return;
        }
        let mut kinds = Vec::new();
        {
            let view = session.world.view();
            let ctx = OrderContext {
                view: &view,
                regiments: &self.selection.regiments,
                speed: speed_mode(self.run),
            };
            for intent in &intents {
                kinds.extend(commands_for(intent, &ctx));
            }
        }
        for kind in kinds {
            session.queue(kind);
        }
    }

    /// The drag-formation preview (line and facing arrow) while the right
    /// button is down, in screen pixels.
    fn drag_preview(&self) -> Option<(Vec2, Vec2, Vec2)> {
        let Mode::Battle(session) = &self.mode else {
            return None;
        };
        let camera = self.camera?;
        let drag = self
            .input
            .drag(&self.bindings, Action::OrderDragFormation)?;
        if self.selection.is_empty() {
            return None;
        }
        let screen = self.screen();
        let from = camera.screen_to_world(drag.from, screen);
        let to = camera.screen_to_world(drag.to, screen);
        let centroid =
            selection_centroid(&session.world.view(), &self.selection.regiments).unwrap_or(from);
        let flip = self.input.held(&self.bindings, Action::OrderFlipFacing);
        let DragFormation {
            anchor,
            forward,
            width,
        } = drag_formation(from, to, centroid, flip)?;
        let tip = anchor + forward * (width * PREVIEW_ARROW_FRACTION).max(PREVIEW_ARROW_MIN_M);
        Some((drag.from, drag.to, camera.world_to_screen(tip, 0.0, screen)))
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = self
            .last_frame
            .map_or(0.0, |t| now.duration_since(t).as_secs_f64());
        self.last_frame = Some(now);
        self.input.begin_frame(self.started.elapsed().as_secs_f64());
        self.apply_camera_input(dt as f32);
        self.apply_toggles();
        self.apply_selection_input();
        self.apply_order_input();
        let screen = self.screen();
        let time = self.started.elapsed().as_secs_f32();

        match &mut self.mode {
            Mode::Battle(session) => {
                #[cfg(feature = "dev")]
                if let Some(hr) = self.hot_reload.as_mut() {
                    if let Some(regs) = hr.poll() {
                        session.world.replace_registries(regs.clone());
                        self.bindings = load_bindings(&regs);
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
                selected: &self.selection.regiments,
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

        let box_drag = self
            .input
            .drag(&self.bindings, Action::BoxSelect)
            .or_else(|| self.input.drag(&self.bindings, Action::BoxSelectAdd))
            .map(|d| (d.from, d.to));
        let drag_preview = self.drag_preview();
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
                    if let Some((from, to)) = box_drag {
                        selection_box(ctx, from, to);
                    }
                    if let Some((from, to, tip)) = drag_preview {
                        drag_formation_preview(ctx, from, to, tip);
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

        self.input.end_frame();
        self.frames += 1;
        if self.frames.is_multiple_of(TITLE_EVERY_FRAMES) {
            self.refresh_title();
        }
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
                    "Iron Legion — tick {} — {}/{} soldiers drawn — sim {:.2} ms/tick — speed x{:.2} — {} selected{} — {} commands — zoom {:.1} px/m rot {}{}",
                    session.world.tick().0,
                    self.snapshot.counts.visible_soldiers,
                    self.snapshot.counts.soldiers,
                    per_tick_ms,
                    session.speed(),
                    self.selection.len(),
                    if self.run { " (run)" } else { "" },
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

/// Hit testing through the frame's camera (`il_ui` never sees `Camera`, so
/// it gets a projection closure).
struct Picker<'a, 'w> {
    view: &'a BattleView<'w>,
    camera: Camera,
    screen: Vec2,
    player: PlayerId,
}

impl Picker<'_, '_> {
    fn project(&self) -> impl Fn(V2) -> Vec2 + '_ {
        let map = self.view.map();
        move |w: V2| {
            let p = Vec2::new(w.x.to_f32_render(), w.y.to_f32_render());
            self.camera
                .world_to_screen(p, ground_height(map, p), self.screen)
        }
    }

    fn pick(&self, cursor: Vec2) -> Option<RegimentId> {
        pick_regiment(
            self.view,
            &self.project(),
            self.camera.zoom,
            self.player,
            cursor,
        )
    }

    fn in_box(&self, a: Vec2, b: Vec2) -> std::collections::BTreeSet<RegimentId> {
        regiments_in_box(self.view, &self.project(), self.player, a, b)
    }
}

fn speed_mode(run: bool) -> SpeedMode {
    if run { SpeedMode::Run } else { SpeedMode::Walk }
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
        let consumed = match (self.ui.as_mut(), self.window.as_ref()) {
            (Some(ui), Some(window)) => ui.on_window_event(window, &event),
            _ => false,
        };
        self.input.on_window_event(&event, consumed);
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(r) = self.renderer.as_mut() {
                    r.resize(size.width, size.height);
                }
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
