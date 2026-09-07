//! The winit application handler: window, renderer, input, the app state
//! machine and the frame loop (T1-050, T1-051, T1-052, T1-061, T1-062,
//! T1-070; SAD §6.1, TDD §15).
//!
//! Every key and mouse gesture goes through `il_ui::InputState` and the
//! `Bindings` loaded from `content/input/bindings.json5`; nothing here names
//! a key code (REQ-INP-005). The frame: poll input → intents → commands
//! queued on the session → `advance` (accumulator, capped catch-up) →
//! snapshot with `alpha` → render → egui → apply the state transition.

use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::anyhow;
use glam::Vec2;
use il_core::{PlayerId, RegimentId, Scalar, V2};
use il_data::Registries;
use il_render::{
    AtlasId, Camera, ClearColour, DebugFlags, EguiPaint, FrameScene, LineScene, RenderSnapshot,
    Renderer, SetAtlas, SnapshotInput, SpriteScene, TerrainMesh, build_debug_lines, build_snapshot,
    deployment_outlines, ground_height, scene_from_snapshot,
};
use il_sim_battle::{
    BattlePhase, BattleSetup, BattleView, BattleWorld, ScriptedCommands, SpeedMode,
};
use il_ui::{
    Action, Bindings, CardAction, CardStripModel, CommandAction, DragFormation, Gesture, HudAction,
    HudModel, InputState, MinimapAction, MinimapInput, OrderContext, PauseAction, PauseModel,
    Selection, UiContext, UiIntent, battle_hud, card_strip, casualties_line, command_card,
    commands_for, drag_formation, drag_formation_preview, event_panel, own_regiments, pause_menu,
    pick_enemy_regiment, pick_regiment, preset_deploy_commands, profiler_overlay, regiments_in_box,
    regiments_of_type_on_screen, selection_box, selection_centroid,
};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::window::{Window, WindowId};

use crate::HotReloadHandle;
use crate::audio::AppAudio;
use crate::battle_ui::{self, Armed, BattleUi};
use crate::bench::SpriteBench;
use crate::menus::draft_from;
use crate::profiler::Profiler;
use crate::session::BattleSession;
use crate::settings::{self, Settings};
use crate::state::{AppState, MenuState, Transition};

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

/// What the app needs to start battles from the menu.
pub struct Launch {
    /// Mod root holding `mod.json5`, `content/` and `assets/`.
    pub content_root: PathBuf,
    /// Extra mod roots after the game, in load order.
    pub mods: Vec<PathBuf>,
    /// Where the main menu looks for scenario files.
    pub scenarios_dir: PathBuf,
    /// Simulation worker threads.
    pub threads: usize,
    /// Render the synthetic sprite bench instead of the game (T1-051).
    pub bench_sprites: bool,
    /// Players handed to the engine AI at tick 1 (`--ai`, T2-081).
    pub ai: Vec<PlayerId>,
    /// Where replays are written (T2-101, plan decision 17).
    pub replays_dir: PathBuf,
    /// Where the quick save lives (T2-101, plan decision 14).
    pub saves_dir: PathBuf,
    /// The user's settings and where they are saved (T2-091).
    pub settings: Settings,
    pub settings_path: PathBuf,
    /// `--mute`: the master volume starts at 0 (T2-100).
    pub mute: bool,
}

pub struct App {
    pub(crate) state: AppState,
    pub(crate) launch: Launch,
    pub(crate) regs: Arc<Registries>,
    #[allow(dead_code, reason = "unused without the dev feature")]
    pub(crate) hot_reload: HotReloadHandle,
    pub(crate) window: Option<Arc<Window>>,
    pub(crate) renderer: Option<Renderer>,
    pub(crate) ui: Option<UiContext>,
    pub(crate) input: InputState,
    pub(crate) bindings: Bindings,
    pub(crate) selection: Selection,
    /// The run toggle: new movement orders run instead of walk.
    pub(crate) run: bool,
    pub(crate) profiler: Profiler,
    pub(crate) show_profiler: bool,
    /// Debug overlays (T1-054), `dev` builds only.
    pub(crate) debug: DebugFlags,
    /// One atlas per sprite set, in registry order.
    pub(crate) atlases: Vec<AtlasId>,
    pub(crate) camera: Option<Camera>,
    pub(crate) snapshot: RenderSnapshot,
    pub(crate) scene: SpriteScene,
    pub(crate) lines: LineScene,
    pub(crate) bench: Option<SpriteBench>,
    /// Requested this frame, applied after rendering.
    pub(crate) transition: Option<Transition>,
    /// The battle screen's panels' state (T2-090).
    pub(crate) battle_ui: BattleUi,
    /// The settings' `ui_scale` (1.0 until T2-091).
    pub(crate) ui_scale_user: f32,
    /// The egui zoom factor last applied (decision 7).
    pub(crate) zoom_applied: f32,
    /// The audio engine and router (T2-100).
    pub(crate) audio: AppAudio,
    /// This battle's replay has been written (once per battle, T2-101).
    pub(crate) replay_written: bool,
    /// The replay file the last write produced (shown on the result window).
    pub(crate) last_replay: Option<PathBuf>,
    pub(crate) started: Instant,
    pub(crate) last_frame: Option<Instant>,
    pub(crate) frames: u32,
    /// Wall time spent inside `BattleWorld::step` since the last title refresh.
    pub(crate) step_seconds: f64,
    pub(crate) ticks_since_title: u32,
}

/// Parses the registry's bindings with the settings' overrides (T2-091),
/// printing what it had to skip.
fn load_bindings(regs: &Registries, user: &Settings) -> Bindings {
    let (bindings, errors) = settings::effective_bindings(regs, user);
    for e in errors {
        eprintln!("bindings: {e}");
    }
    bindings
}

/// Builds a session for a setup built in memory (the custom battle builder
/// or a rematch, T2-091).
pub fn start_from_setup(
    setup: BattleSetup,
    stem: String,
    regs: Arc<Registries>,
    threads: usize,
    ai: Vec<PlayerId>,
) -> anyhow::Result<BattleSession> {
    let mut world = BattleWorld::new(&setup, regs).map_err(|e| anyhow!("{e}"))?;
    world.set_threads(threads);
    Ok(BattleSession::new(world, PlayerId(0), ScriptedCommands::default(), ai).with_stem(stem))
}

/// Builds a session for a scenario file (the main menu's "custom battle").
pub fn start_battle(
    path: &Path,
    regs: Arc<Registries>,
    threads: usize,
    ai: Vec<PlayerId>,
) -> anyhow::Result<BattleSession> {
    let scenario = il_cli::load_scenario(path)?;
    let mut world = BattleWorld::new(&scenario.setup, regs).map_err(|e| anyhow!("{e}"))?;
    world.set_threads(threads);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "battle".to_string());
    Ok(BattleSession::new(world, PlayerId(0), scenario.script(), ai).with_stem(stem))
}

fn speed_mode(run: bool) -> SpeedMode {
    if run { SpeedMode::Run } else { SpeedMode::Walk }
}

impl App {
    pub fn new(
        state: AppState,
        launch: Launch,
        regs: Arc<Registries>,
        hot_reload: HotReloadHandle,
    ) -> Self {
        let bindings = load_bindings(&regs, &launch.settings);
        let ui_scale_user = launch.settings.ui_scale;
        let audio = AppAudio::new(launch.mute, &launch.settings.volume);
        Self {
            state,
            launch,
            regs,
            hot_reload,
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
            transition: None,
            battle_ui: BattleUi::default(),
            ui_scale_user,
            zoom_applied: 1.0,
            audio,
            replay_written: false,
            last_replay: None,
            started: Instant::now(),
            last_frame: None,
            frames: 0,
            step_seconds: 0.0,
            ticks_since_title: 0,
        }
    }

    /// The menu the app returns to: rescans the scenario folder.
    pub fn menu(&self) -> MenuState {
        let mut mods = vec![self.launch.content_root.clone()];
        mods.extend(self.launch.mods.iter().cloned());
        MenuState::scan(&self.launch.scenarios_dir, mods)
    }

    /// Uploads every sprite set of the registry, in registry order, so the
    /// snapshot's sprite-set index maps straight onto `atlases`.
    fn load_atlases(&mut self) -> anyhow::Result<()> {
        let renderer = self.renderer.as_mut().expect("renderer exists");
        let assets_root = self.launch.content_root.join("assets");
        for (_, set) in self.regs.sprite_sets.iter() {
            self.atlases.push(renderer.load_atlas(set, &assets_root)?);
        }
        Ok(())
    }

    /// Builds and uploads the battle's terrain mesh (T1-053).
    fn load_terrain(&mut self) {
        let Some(session) = self.state.session() else {
            return;
        };
        let mesh = TerrainMesh::build(session.world.map(), &self.regs);
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.set_terrain(&mesh);
        }
    }

    /// Per-battle state reset when a battle starts or ends.
    fn reset_battle_state(&mut self) {
        self.camera = None;
        self.selection = Selection::new();
        self.run = false;
        self.profiler = Profiler::default();
        self.snapshot = RenderSnapshot::default();
        self.scene = SpriteScene::default();
        self.lines.clear();
        self.battle_ui = BattleUi::default();
        self.replay_written = false;
        self.audio.stop_battle();
        self.load_terrain();
    }

    /// Writes the battle's replay once (T2-101, decision 17): at the end,
    /// on quit, on a load over it and on the window closing. A playback
    /// records nothing.
    fn write_replay_now(&mut self) {
        let Some(session) = self.state.session() else {
            return;
        };
        if self.replay_written || session.is_replay() || session.hashes().is_empty() {
            return;
        }
        self.replay_written = true;
        let Some(replay) = session.replay() else {
            return;
        };
        match crate::replay_io::write_replay(
            &self.launch.replays_dir,
            session.scenario_stem(),
            &self.regs,
            &replay,
        ) {
            Ok(path) => {
                eprintln!("replay written to {}", path.display());
                self.last_replay = Some(path);
            }
            Err(e) => eprintln!("replay not written: {e:#}"),
        }
    }

    /// Ctrl+S (decision 14, plan I13): the quick save, refused after the
    /// end and in a playback.
    fn quick_save(&mut self) {
        let path = self.launch.saves_dir.join(crate::replay_io::QUICK_SAVE);
        let regs = self.regs.clone();
        let Some(session) = self.state.session_mut() else {
            return;
        };
        let l = &regs.locale;
        if session.is_replay() || session.world.phase() == BattlePhase::Ended {
            let text = l.get("il.replay.not_now").to_string();
            session.note(text);
            return;
        }
        let Some(save) = session.save() else {
            return;
        };
        let text = match crate::replay_io::write_save(&path, &regs, &save) {
            Ok(()) => l.fmt("il.replay.saved", &[("path", &path.display())]),
            Err(e) => l.fmt("il.replay.save_failed", &[("error", &format!("{e:#}"))]),
        };
        session.note(text);
    }

    /// Opens or closes the pause menu (decision 10): opening pauses, closing
    /// restores the pause state from before.
    fn toggle_pause_menu(&mut self) {
        let open = !self.battle_ui.pause_open;
        let Some(session) = self.state.session_mut() else {
            return;
        };
        if open {
            self.battle_ui.pause_was_paused = session.paused();
            if !session.paused() {
                session.set_paused(true);
            }
        } else if !self.battle_ui.pause_was_paused && session.paused() {
            session.set_paused(false);
        }
        self.battle_ui.pause_open = open;
        self.refresh_title();
    }

    /// Queues the commands the intents mean for the current selection
    /// (REQ-INP-006); the keys and the panels both end here.
    fn send_intents(&mut self, intents: &[UiIntent]) {
        if intents.is_empty() || self.selection.is_empty() {
            return;
        }
        let run = self.run;
        let Some(session) = self.state.session_mut() else {
            return;
        };
        let mut kinds = Vec::new();
        {
            let view = session.world.view();
            let ctx = OrderContext {
                view: &view,
                regiments: &self.selection.regiments,
                speed: speed_mode(run),
            };
            for intent in intents {
                kinds.extend(commands_for(intent, &ctx));
            }
        }
        for kind in kinds {
            session.queue(kind);
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
            let mut camera = match self.state.session() {
                Some(session) => {
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
                None => Camera::new(Vec2::ZERO),
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

    /// Developer toggles, pause, speed and the pause menu (bindings
    /// `toggle_profiler`, `debug_*`, `pause`, `speed_up`, `speed_down`,
    /// `pause_menu`; Escape first disarms an attack-move, plan I4).
    fn apply_toggles(&mut self) {
        if self.input.pressed(&self.bindings, Action::PauseMenu) {
            if self.battle_ui.armed.is_some() {
                self.battle_ui.armed = None;
            } else {
                self.toggle_pause_menu();
            }
        }
        // T2-101: the quick save and load.
        if self.input.pressed(&self.bindings, Action::QuickSave) {
            self.quick_save();
        }
        if self.input.pressed(&self.bindings, Action::QuickLoad) {
            let path = self.launch.saves_dir.join(crate::replay_io::QUICK_SAVE);
            self.transition = Some(Transition::LoadSave(path));
        }
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
                (Action::DebugMorale, &mut flags.morale),
                (Action::DebugFlow, &mut flags.flow),
                (Action::DebugLos, &mut flags.los),
                (Action::DebugAi, &mut flags.ai),
            ] {
                if input.pressed(b, action) {
                    *flag = !*flag;
                }
            }
        }
        // SIM-FLOW-011 (T2-070): Enter ends the deployment.
        if input.pressed(b, Action::ConfirmDeployment)
            && let Some(session) = self.state.session_mut()
        {
            session.queue(il_sim_battle::CommandKind::ConfirmDeployment);
        }
        let b = &self.bindings;
        let input = &self.input;
        let mut hud = None;
        if input.pressed(b, Action::Pause) {
            hud = Some(HudAction::TogglePause);
        }
        if input.pressed(b, Action::SpeedUp) {
            hud = Some(HudAction::SpeedUp);
        }
        if input.pressed(b, Action::SpeedDown) {
            hud = Some(HudAction::SpeedDown);
        }
        if let Some(action) = hud {
            self.hud_action(action);
        }
    }

    /// Pause and speed go to the session (as commands and multipliers); the
    /// Menu button opens the pause menu; Confirm ends the deployment.
    fn hud_action(&mut self, action: HudAction) {
        if action == HudAction::OpenMenu {
            if !self.battle_ui.pause_open {
                self.toggle_pause_menu();
            }
            return;
        }
        let Some(session) = self.state.session_mut() else {
            return;
        };
        match action {
            HudAction::TogglePause => {
                let paused = session.paused();
                session.set_paused(!paused);
            }
            HudAction::SpeedUp => session.set_speed((session.speed() * 2.0).min(MAX_SPEED)),
            HudAction::SpeedDown => session.set_speed((session.speed() * 0.5).max(MIN_SPEED)),
            HudAction::ConfirmDeployment => {
                session.queue(il_sim_battle::CommandKind::ConfirmDeployment);
            }
            HudAction::OpenMenu => {}
        }
        self.refresh_title();
    }

    /// The pause menu's clicks (decision 10).
    fn pause_action(&mut self, action: PauseAction) {
        match action {
            PauseAction::Resume => self.toggle_pause_menu(),
            PauseAction::Surrender => {
                if let Some(session) = self.state.session_mut() {
                    session.surrender();
                }
                self.toggle_pause_menu();
            }
            // T2-091: the settings screen over the paused battle.
            PauseAction::Settings => {
                self.battle_ui.settings = Some(Box::new(il_ui::SettingsState::new(draft_from(
                    &self.launch.settings,
                    &self.regs,
                ))));
            }
            PauseAction::Quit => self.transition = Some(Transition::QuitToMenu),
        }
    }

    /// The card strip's clicks (decision 8).
    fn card_actions(&mut self, actions: &[CardAction]) {
        for action in actions {
            match *action {
                CardAction::Select { id, add } => self.selection.click(Some(id), add),
                CardAction::Centre(id) => {
                    let anchor = self
                        .state
                        .session()
                        .and_then(|s| s.world.view().regiment(id))
                        .map(|r| {
                            Vec2::new(
                                r.anchor_pos.x.to_f32_render(),
                                r.anchor_pos.y.to_f32_render(),
                            )
                        });
                    if let Some(a) = anchor {
                        self.camera_mut().center = a;
                    }
                }
            }
        }
    }

    /// The command card's clicks (decision 9): the same intents as the keys.
    fn command_action(&mut self, action: CommandAction) {
        let intent = match action {
            CommandAction::Halt => UiIntent::Halt,
            CommandAction::ArmAttackMove => {
                self.battle_ui.armed = match self.battle_ui.armed {
                    Some(Armed::AttackMove) => None,
                    None => Some(Armed::AttackMove),
                };
                return;
            }
            CommandAction::Withdraw => UiIntent::Withdraw,
            CommandAction::ToggleFire => UiIntent::ToggleFire,
            CommandAction::ToggleRun => {
                self.run = !self.run;
                UiIntent::SpeedMode(speed_mode(self.run))
            }
            CommandAction::Formation(n) => UiIntent::Formation(n),
            CommandAction::Ability(n) => {
                let cursor = self
                    .input
                    .cursor()
                    .and_then(|c| self.camera.map(|cam| cam.screen_to_world(c, self.screen())))
                    .unwrap_or(Vec2::ZERO);
                UiIntent::Ability { slot: n, cursor }
            }
            CommandAction::Preset(template) => {
                // During the deployment the preset re-lays the whole side,
                // selection or not (decision 6).
                let deploying = self
                    .state
                    .session()
                    .is_some_and(|s| s.world.phase() == BattlePhase::Deployment);
                if deploying {
                    if let Some(session) = self.state.session_mut()
                        && let Some(side) = session.observer_side()
                    {
                        let kinds = preset_deploy_commands(&session.world.view(), side, &template);
                        for kind in kinds {
                            session.queue(kind);
                        }
                    }
                    return;
                }
                UiIntent::GroupPreset { template }
            }
        };
        self.send_intents(&[intent]);
    }

    /// The minimap's clicks (decision 11).
    fn minimap_action(&mut self, action: MinimapAction) {
        match action {
            MinimapAction::Pan(world) => self.camera_mut().center = world,
            MinimapAction::Order(world) => self.send_intents(&[UiIntent::Move { target: world }]),
        }
    }

    /// Selection gestures (REQ-INP-002): click, shift-click, box, double
    /// click by type, select all, control groups. Only the local player's
    /// regiments can be selected.
    fn apply_selection_input(&mut self) {
        let screen = self.screen();
        let Some(camera) = self.camera else {
            return;
        };
        // An armed cursor owns the left button (plan I4).
        if self.battle_ui.armed.is_some() {
            return;
        }
        let Some(session) = self.state.session() else {
            return;
        };
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

    /// Orders (REQ-INP-003): right click moves, or attacks a visible enemy
    /// under the cursor; right drag lays a line (T1-062 gesture); the armed
    /// attack-move cursor's left click (T2-090); halt, withdraw, run toggle,
    /// formation and ability hotkeys. Every intent becomes Commands queued
    /// on the session (REQ-INP-006).
    fn apply_order_input(&mut self) {
        let screen = self.screen();
        let Some(camera) = self.camera else {
            return;
        };
        if self.selection.is_empty() {
            self.battle_ui.armed = None;
            return;
        }
        let Some(session) = self.state.session() else {
            return;
        };
        let b = &self.bindings;
        let input = &self.input;
        let unproject = |p: Vec2| camera.screen_to_world(p, screen);
        let mut intents: Vec<UiIntent> = Vec::new();
        let mut armed = self.battle_ui.armed;
        if input.pressed(b, Action::OrderAttackMove) {
            armed = match armed {
                Some(Armed::AttackMove) => None,
                None => Some(Armed::AttackMove),
            };
        }
        if armed == Some(Armed::AttackMove) {
            // The armed cursor: a left click on the ground attack-moves, a
            // right click only cancels (plan decision 5).
            if let Some(Gesture::Click { pos, .. }) = input.gesture(b, Action::Select) {
                intents.push(UiIntent::AttackMove {
                    target: unproject(pos),
                });
                armed = None;
            } else if input.gesture(b, Action::OrderMove).is_some()
                || input.gesture(b, Action::OrderDragFormation).is_some()
            {
                armed = None;
            }
        } else if let Some(Gesture::DragEnd { from, to, .. }) =
            input.gesture(b, Action::OrderDragFormation)
        {
            let centroid = selection_centroid(&session.world.view(), &self.selection.regiments)
                .unwrap_or(unproject(from));
            let flip = input.held(b, Action::OrderFlipFacing);
            if let Some(drag) = drag_formation(unproject(from), unproject(to), centroid, flip) {
                intents.push(UiIntent::DragFormation(drag));
            }
        } else if let Some(Gesture::Click { pos, .. }) = input.gesture(b, Action::OrderMove) {
            let view = session.world.view();
            let enemy = session.observer_side().and_then(|side| {
                let picker = Picker {
                    view: &view,
                    camera,
                    screen,
                    player: session.local_player(),
                };
                pick_enemy_regiment(&view, &picker.project(), camera.zoom, side, pos)
            });
            intents.push(match enemy {
                Some(target) => UiIntent::AttackRegiment { target },
                None => UiIntent::Move {
                    target: unproject(pos),
                },
            });
        }
        self.battle_ui.armed = armed;
        if input.pressed(b, Action::OrderHalt) {
            intents.push(UiIntent::Halt);
        }
        if input.pressed(b, Action::OrderWithdraw) {
            intents.push(UiIntent::Withdraw);
        }
        if input.pressed(b, Action::ToggleRun) {
            self.run = !self.run;
            intents.push(UiIntent::SpeedMode(speed_mode(self.run)));
        }
        if input.pressed(b, Action::ToggleFire) {
            intents.push(UiIntent::ToggleFire);
        }
        for n in 1..=il_ui::orders::ABILITY_HOTKEYS {
            if input.pressed(b, Action::Ability(n)) {
                let cursor = input.cursor().map(unproject).unwrap_or(Vec2::ZERO);
                intents.push(UiIntent::Ability { slot: n, cursor });
            }
        }
        for n in 1..=FORMATION_HOTKEYS {
            if input.pressed(b, Action::Formation(n)) {
                intents.push(UiIntent::Formation(n));
            }
        }
        self.send_intents(&intents);
    }

    /// The drag-formation preview (line and facing arrow) while the right
    /// button is down, in screen pixels.
    fn drag_preview(&self) -> Option<(Vec2, Vec2, Vec2)> {
        let session = self.state.session()?;
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

    /// Steps the sim for this frame's wall time (hot reload first).
    fn advance_battle(&mut self, dt: f64) {
        let Some(session) = self.state.session_mut() else {
            return;
        };
        #[cfg(feature = "dev")]
        if let Some(hr) = self.hot_reload.as_mut() {
            if let Some(regs) = hr.poll() {
                session.world.replace_registries(regs.clone());
                self.bindings = load_bindings(&regs, &self.launch.settings);
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
        let outputs = session.advance_with(dt, &mut self.profiler);
        self.step_seconds += before.elapsed().as_secs_f64();
        let stepped = outputs.len() as u32;
        self.audio.collect(&outputs);
        self.ticks_since_title += stepped;
        self.profiler.frame(dt, stepped);
    }

    /// T2-100: the frame's events to the audio router, starting the
    /// battle's sound set on the first frame a session is up.
    fn route_audio(&mut self, screen: Vec2) {
        let camera = *self.camera_mut();
        let Some(session) = self.state.session() else {
            return;
        };
        let observer_side = session.observer_side();
        if !self.audio.has_battle() {
            let view = session.world.view();
            let faction = observer_side
                .and_then(|s| view.sides().get(usize::from(s)))
                .map(|s| s.faction.clone());
            let assets_root = self.launch.content_root.join("assets");
            self.audio
                .start_battle(&self.regs, faction.as_ref(), &assets_root);
        }
        let now_ms = self.started.elapsed().as_millis() as u64;
        self.audio.frame(
            &session.world.view(),
            &camera,
            screen,
            now_ms,
            observer_side,
        );
    }

    /// Colour of a projectile segment (pale wood on any ground).
    const PROJECTILE_COLOUR: [u8; 4] = [236, 224, 186, 255];

    /// Builds the render snapshot and sprite scene for the battle.
    fn build_battle_scene(&mut self, screen: Vec2, time: f32) {
        let camera = *self.camera_mut();
        let Some(session) = self.state.session() else {
            return;
        };
        let input = SnapshotInput {
            alpha: session.alpha(),
            camera,
            screen,
            selected: &self.selection.regiments,
            corpses: session.corpses(),
            // SIM-VIS-004 (T2-060): the local player's fog of war.
            observer_side: session.observer_side(),
        };
        build_snapshot(&session.world.view(), &input, &mut self.snapshot);
        self.lines.clear();
        deployment_outlines(session.world.map(), &camera, screen, &mut self.lines);
        il_render::ghost_markers(
            session.world.map(),
            &self.snapshot.ghosts,
            &camera,
            screen,
            &mut self.lines,
        );
        // Projectiles as short lifted segments (T2-031, plan decision 9).
        for p in &self.snapshot.projectiles {
            let a = camera.world_to_screen(Vec2::from(p.a), p.height, screen);
            let b = camera.world_to_screen(Vec2::from(p.b), p.height, screen);
            self.lines.segment(a, b, Self::PROJECTILE_COLOUR);
        }
        if DEV {
            // The flow overlay shows the selected regiment's side (else 0).
            let view = session.world.view();
            let flow_side = self
                .selection
                .regiments
                .first()
                .and_then(|id| view.regiment(*id))
                .map_or(0, |r| r.side);
            build_debug_lines(
                &view,
                self.debug,
                flow_side,
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

    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = self
            .last_frame
            .map_or(0.0, |t| now.duration_since(t).as_secs_f64());
        self.last_frame = Some(now);
        self.input.begin_frame(self.started.elapsed().as_secs_f64());
        let screen = self.screen();
        let time = self.started.elapsed().as_secs_f32();

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
        } else if self.state.is_battle() {
            // T2-091: a settings screen capturing a chord owns the input.
            let capturing = self.capturing_chord();
            if capturing {
                self.capture_chord();
            } else {
                self.apply_camera_input(dt as f32);
                self.apply_toggles();
                self.apply_selection_input();
                self.apply_order_input();
            }
            self.advance_battle(dt);
            if self.state.session().is_some_and(|s| s.result().is_some()) {
                self.write_replay_now();
            }
            self.build_battle_scene(screen, time);
            self.route_audio(screen);
        }

        let drag_preview = self.drag_preview();
        let mut ui_out = None;
        // REQ-UI-006 (decision 7): text scales with the window height.
        if let (Some(ui), Some(window)) = (self.ui.as_ref(), self.window.as_ref()) {
            let logical_height = (screen.y / window.scale_factor() as f32).max(1.0);
            let zoom = battle_ui::zoom_factor(logical_height, self.ui_scale_user);
            if (zoom - self.zoom_applied).abs() > 1e-3 {
                ui.ctx().set_zoom_factor(zoom);
                self.zoom_applied = zoom;
            }
        }
        if let (Some(ui), Some(window), None) =
            (self.ui.as_mut(), self.window.as_ref(), &self.bench)
        {
            match &self.state {
                AppState::Battle(session) => {
                    let mut stats = self.profiler.stats();
                    stats.soldiers = self.snapshot.counts.soldiers;
                    stats.regiments = self.snapshot.counts.regiments;
                    stats.visible_soldiers = self.snapshot.counts.visible_soldiers;
                    stats.accumulator_alpha = session.alpha();
                    let show_profiler = self.show_profiler;
                    let box_drag = self
                        .input
                        .drag(&self.bindings, Action::BoxSelect)
                        .or_else(|| self.input.drag(&self.bindings, Action::BoxSelectAdd))
                        .map(|d| (d.from, d.to));
                    let phase = session.world.phase();
                    let hud = HudModel {
                        tick: session.world.tick(),
                        phase: self
                            .regs
                            .locale
                            .get(match phase {
                                BattlePhase::Deployment => "il.battle.phase.deployment",
                                BattlePhase::Battle => "il.battle.phase.battle",
                                BattlePhase::Pursuit => "il.battle.phase.pursuit",
                                BattlePhase::Ended => "il.battle.phase.ended",
                            })
                            .to_string(),
                        deploying: phase == BattlePhase::Deployment,
                        paused: session.paused(),
                        speed: session.speed(),
                        run: self.run,
                        commands: session.command_log().len() + session.ai_log().len(),
                        locale: &self.regs.locale,
                    };
                    let locale = &self.regs.locale;
                    let events: Vec<_> = session.events().iter().cloned().collect();
                    // T2-090: the panels' models.
                    let rows = battle_ui::selection_rows(session, &self.selection);
                    let cards = battle_ui::card_models(session, &self.selection);
                    let card_model = CardStripModel {
                        cards: &cards,
                        locale,
                    };
                    let command = battle_ui::command_model(
                        session,
                        &self.selection,
                        &rows,
                        &self.regs,
                        self.run,
                        self.battle_ui.armed,
                    );
                    let tallies = battle_ui::tallies(session);
                    let camera = self.camera.unwrap_or_else(|| Camera::new(Vec2::ZERO));
                    let mini = battle_ui::minimap_data(
                        session,
                        &self.selection.regiments,
                        &camera,
                        screen,
                    );
                    let map = session.world.map();
                    let minimap_input = MinimapInput {
                        map,
                        zone_colours: &mini.zone_colours,
                        zone_crossing: &mini.zone_crossing,
                        discs: &mini.discs,
                        blocks: &mini.blocks,
                        viewport: mini.viewport,
                    };
                    let pause = PauseModel {
                        can_surrender: session.observer_side().is_some()
                            && phase != BattlePhase::Ended
                            && !session.is_replay(),
                        has_settings: true,
                        locale,
                    };
                    let result_sides = session
                        .result()
                        .map(|r| battle_ui::result_sides(session, r))
                        .unwrap_or_default();
                    let can_rematch = !session.is_replay() && session.setup().is_some();
                    let battle_settings = &mut self.battle_ui.settings;
                    let mut settings_click = None;
                    let pause_open = self.battle_ui.pause_open;
                    let armed = self.battle_ui.armed;
                    let replay_path = self.last_replay.as_ref().map(|p| p.display().to_string());
                    let minimap = &mut self.battle_ui.minimap;
                    let mut hud_click = None;
                    let mut card_clicks = Vec::new();
                    let mut command_click = None;
                    let mut minimap_click = None;
                    let mut pause_click = None;
                    let mut result_click = None;
                    let out = ui.run(window, |ctx| {
                        if show_profiler {
                            profiler_overlay(ctx, locale, &stats);
                            event_panel(ctx, locale, &events);
                        }
                        if let Some((from, to)) = box_drag {
                            selection_box(ctx, from, to);
                        }
                        if let Some((from, to, tip)) = drag_preview {
                            drag_formation_preview(ctx, from, to, tip);
                        }
                        if armed.is_some() {
                            ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
                        }
                        casualties_line(ctx, &tallies, locale);
                        card_clicks = card_strip(ctx, &card_model);
                        command_click = command_card(ctx, &command);
                        minimap_click = minimap.show(ctx, &minimap_input, locale);
                        hud_click = battle_hud(ctx, &hud);
                        if pause_open {
                            pause_click = pause_menu(ctx, &pause);
                        }
                        if let Some(state) = battle_settings.as_mut() {
                            settings_click = il_ui::settings_screen(ctx, state, locale, true);
                        }
                        // T2-070/T2-091: the result screen once the battle ended.
                        if let Some(result) = session.result() {
                            result_click = il_ui::result_screen(
                                ctx,
                                &il_ui::ResultScreenModel {
                                    winner: result.winner,
                                    duration: il_ui::clock(il_core::Tick(result.duration_ticks)),
                                    sides: &result_sides,
                                    replay_path: replay_path.as_deref(),
                                    can_rematch,
                                    locale,
                                },
                            );
                        }
                    });
                    ui_out = Some(out);
                    if let Some(action) = hud_click {
                        self.hud_action(action);
                    }
                    self.card_actions(&card_clicks);
                    if let Some(action) = command_click {
                        self.command_action(action);
                    }
                    if let Some(action) = minimap_click {
                        self.minimap_action(action);
                    }
                    if let Some(action) = pause_click {
                        self.pause_action(action);
                    }
                    match result_click {
                        Some(il_ui::ResultAction::Menu) => {
                            self.transition = Some(Transition::QuitToMenu);
                        }
                        Some(il_ui::ResultAction::Rematch) => self.rematch(),
                        None => {}
                    }
                    if let Some(action) = settings_click
                        && let Some(mut state) = self.battle_ui.settings.take()
                        && !self.settings_action(&mut state, action)
                    {
                        self.battle_ui.settings = Some(state);
                    }
                }
                AppState::MainMenu(_) => {}
            }
        }
        // T2-091: the menu screens (they need the app mutably, so the
        // egui context is taken out for the call).
        if !self.state.is_battle() && self.bench.is_none() {
            if self.capturing_chord() {
                self.capture_chord();
            }
            if let (Some(mut ui), Some(window)) = (self.ui.take(), self.window.clone()) {
                let (out, exit) = self.menu_frame(&mut ui, &window);
                ui_out = Some(out);
                self.ui = Some(ui);
                if exit {
                    event_loop.exit();
                }
            }
        }
        let mut paint = ui_out.as_mut().map(|o| EguiPaint {
            textures_delta: &mut o.textures_delta,
            primitives: &o.primitives,
            pixels_per_point: o.pixels_per_point,
        });
        let empty = SpriteScene::default();
        let (sprites, camera) = match (&self.bench, self.state.is_battle()) {
            (Some(bench), _) => (&bench.scene, None),
            (None, true) => (&self.scene, self.camera),
            (None, false) => (&empty, None),
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
        self.apply_transition();
    }

    /// Applies the frame's state transition (SAD §6.1).
    fn apply_transition(&mut self) {
        let Some(transition) = self.transition.take() else {
            return;
        };
        // Leaving a battle writes its replay first (T2-101, decision 17).
        if matches!(transition, Transition::QuitToMenu | Transition::LoadSave(_)) {
            self.write_replay_now();
        }
        let regs = self.regs.clone();
        let regs_build = regs.clone();
        let regs_load = regs.clone();
        let threads = self.launch.threads;
        let ai = self.launch.ai.clone();
        let menu = self.menu();
        let state = std::mem::replace(&mut self.state, AppState::MainMenu(MenuState::default()));
        self.state = state.apply(
            transition,
            |path| start_battle(path, regs, threads, ai),
            |setup, stem, ai| start_from_setup(setup, stem, regs_build, threads, ai),
            |path| crate::replay_io::load_save(path, regs_load, threads),
            || menu,
        );
        self.reset_battle_state();
        self.refresh_title();
    }

    /// The result screen's Rematch (decision 12): the same setup with the
    /// next seed, the same `--ai` list and stem.
    fn rematch(&mut self) {
        let Some(session) = self.state.session() else {
            return;
        };
        let Some(setup) = session.setup() else {
            return;
        };
        let mut setup = setup.clone();
        setup.seed = setup.seed.wrapping_add(1);
        self.transition = Some(Transition::StartSetup {
            setup: Box::new(setup),
            stem: session.scenario_stem().to_string(),
            ai: self.launch.ai.clone(),
        });
    }

    /// ` — replay 120/600`, ` — replay OK` or ` — replay MISMATCH at tick
    /// N` in a playback (decision 18).
    fn replay_suffix(&self) -> String {
        let Some(session) = self.state.session() else {
            return String::new();
        };
        let l = &self.regs.locale;
        match session.mode() {
            crate::session::SessionMode::Live => String::new(),
            crate::session::SessionMode::Replay { expected, .. } => {
                let text = match session.replay_mismatch() {
                    Some(tick) => l.fmt("il.replay.title_mismatch", &[("tick", &tick.0)]),
                    None if session.replay_finished() => l.get("il.replay.title_ok").to_string(),
                    None => l.fmt(
                        "il.replay.title_playing",
                        &[
                            ("tick", &session.world.tick().0 as &dyn Display),
                            ("ticks", &expected.len()),
                        ],
                    ),
                };
                format!(" — {text}")
            }
        }
    }

    fn refresh_title(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let title = match (&self.bench, &self.state) {
            (Some(bench), _) => {
                format!("Iron Legion — sprite bench — frame {}", bench.frames_done)
            }
            (None, AppState::MainMenu(_)) => self.regs.locale.get("il.app.title").to_string(),
            (None, AppState::Battle(session)) => {
                let per_tick_ms = if self.ticks_since_title > 0 {
                    self.step_seconds * 1000.0 / f64::from(self.ticks_since_title)
                } else {
                    0.0
                };
                let cam = self.camera.unwrap_or_else(|| Camera::new(Vec2::ZERO));
                let l = &self.regs.locale;
                let run = if self.run {
                    format!(" ({})", l.get("il.battle.running"))
                } else {
                    String::new()
                };
                let paused = if session.paused() {
                    format!(" — {}", l.get("il.battle.paused"))
                } else {
                    String::new()
                };
                l.fmt(
                    "il.app.battle_title",
                    &[
                        ("title", &l.get("il.app.title") as &dyn Display),
                        ("tick", &session.world.tick().0),
                        ("drawn", &self.snapshot.counts.visible_soldiers),
                        ("total", &self.snapshot.counts.soldiers),
                        ("ms", &format!("{per_tick_ms:.2}")),
                        ("speed", &format!("{:.2}", session.speed())),
                        ("selected", &self.selection.len()),
                        ("run", &run),
                        ("commands", &session.command_log().len()),
                        ("zoom", &format!("{:.1}", cam.zoom)),
                        ("rot", &cam.rotation),
                        ("paused", &paused),
                    ],
                ) + &debug_suffix(self.debug)
                    + &ai_suffix(&session.world.view(), self.debug)
                    + &self.replay_suffix()
            }
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

/// ` — dbg: nav slots` for the enabled overlays, empty when none.
/// With the AI overlay on, each engine-owned side's stance (T2-081).
fn ai_suffix(view: &il_sim_battle::BattleView, flags: DebugFlags) -> String {
    if !flags.ai {
        return String::new();
    }
    let stances: Vec<String> = view
        .sides()
        .iter()
        .enumerate()
        .filter(|(_, s)| s.player == PlayerId::ENGINE_AI)
        .map(|(i, _)| {
            let stance = view.ai_plan(i as u8).map_or("no plan".to_string(), |p| {
                format!("{:?}", p.stance).to_lowercase()
            });
            format!("s{i} {stance}")
        })
        .collect();
    if stances.is_empty() {
        String::new()
    } else {
        format!(" — ai: {}", stances.join(", "))
    }
}

fn debug_suffix(flags: DebugFlags) -> String {
    let names = [
        (flags.nav_grid, "nav"),
        (flags.slots, "slots"),
        (flags.paths, "paths"),
        (flags.anchors, "anchors"),
        (flags.spatial_cells, "cells"),
        (flags.morale, "morale"),
        (flags.flow, "flow"),
        (flags.los, "los"),
        (flags.ai, "ai"),
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
        self.window = Some(window);
        // T2-091: vsync and fullscreen from the settings file.
        self.apply_window_settings();
        if self.launch.bench_sprites {
            let renderer = self.renderer.as_mut().expect("renderer exists");
            renderer.set_vsync(false);
            let (w, h) = renderer.size();
            self.bench = Some(SpriteBench::new(&self.atlases, w as f32, h as f32));
        }
        event_loop.set_control_flow(ControlFlow::Poll);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let consumed = match (self.ui.as_mut(), self.window.as_ref()) {
            (Some(ui), Some(window)) => ui.on_window_event(window, &event),
            _ => false,
        };
        self.input.on_window_event(&event, consumed);
        match event {
            WindowEvent::CloseRequested => {
                self.write_replay_now();
                event_loop.exit();
            }
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
