//! The editor session (T3-060, TDD §16): the document, its history, the
//! active tool, the camera, the terrain the renderer shows and what the
//! frame's input and panel clicks do to them. The app owns the window, the
//! frame job and the transitions; it hands the session the frame's input
//! (`EditorInput`) and applies the `EditorEffect`s that come back.

use std::path::PathBuf;
use std::sync::Arc;

use glam::Vec2;
use il_core::Scalar;
use il_data::{Locale, Registries};
use il_render::terrain::project;
use il_render::{Camera, LineScene, TerrainMesh, deployment_outlines, side_tint};
use il_sim_battle::{LoadedMap, MapError};
use il_ui::{Action, Bindings, InputState};

use crate::document::{History, MapDocument, Saved};
use crate::panels::{self, PanelAction};

/// The active tool (T3-060: select only; the brushes arrive with T3-061,
/// the vector tools with T3-062).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
}

/// What the app hands the session each frame.
pub struct EditorInput<'a> {
    pub bindings: &'a Bindings,
    pub input: &'a InputState,
    /// The window in physical pixels.
    pub screen: Vec2,
    pub dt: f32,
    /// egui owns the pointer (a panel is under it) or the keyboard (a text
    /// field is focused): the tools stay idle.
    pub pointer_over_ui: bool,
    pub keyboard_in_ui: bool,
}

/// What the app must do after a frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorEffect {
    /// The terrain changed: upload `session.terrain` with the next job.
    TerrainChanged,
    /// A save landed (already noted on screen; the app logs it).
    Saved(Saved),
    QuitToMenu,
}

/// Colours of the editor's line overlays.
const BORDER: [u8; 4] = [255, 255, 255, 120];
const RIVER: [u8; 4] = [90, 150, 230, 220];
const RIVER_EDGE: [u8; 4] = [90, 150, 230, 120];
/// Metres per outline segment along a polygon edge (they follow the ground).
const OUTLINE_STEP_M: f32 = 8.0;
/// Width of the reinforcement edge band, metres inside the map edge.
const EDGE_BAND_M: f32 = 6.0;

pub struct EditorSession {
    pub doc: MapDocument,
    pub history: History,
    pub tool: Tool,
    pub camera: Camera,
    pub regs: Arc<Registries>,
    /// The mod roots the app loaded, in load order (the quick picks).
    pub mod_roots: Vec<PathBuf>,
    /// The save target's path field.
    pub target: String,
    /// The document as the sim reads it, rebuilt with the terrain.
    pub loaded: LoadedMap,
    /// The CPU terrain mesh the renderer draws; `terrain_dirty` asks the app
    /// to upload it again.
    pub terrain: TerrainMesh,
    pub terrain_dirty: bool,
    /// The last save's or failure's text, shown in the top bar.
    pub note: Option<String>,
    pub note_is_error: bool,
    /// The Escape menu.
    pub menu_open: bool,
    /// Quit was asked once with unsaved changes; the next Quit goes.
    pub quit_armed: bool,
    /// The nav preview overlay (drawn from T3-063).
    pub show_nav: bool,
    camera_framed: bool,
}

impl EditorSession {
    /// Opens `doc`; `target` prefills the save path field.
    pub fn open(
        mut doc: MapDocument,
        regs: Arc<Registries>,
        mod_roots: Vec<PathBuf>,
        target: Option<PathBuf>,
    ) -> Result<Self, MapError> {
        doc.resolve_zones(&regs);
        let loaded = doc.to_loaded(&regs)?;
        let terrain = TerrainMesh::build(&loaded, &regs);
        let centre = Vec2::new(
            loaded.width.to_f32_render() * 0.5,
            loaded.height.to_f32_render() * 0.5,
        );
        Ok(Self {
            doc,
            history: History::new(),
            tool: Tool::Select,
            camera: Camera::new(centre),
            regs,
            mod_roots,
            target: target.map(|p| display(&p)).unwrap_or_default(),
            loaded,
            terrain,
            terrain_dirty: true,
            note: None,
            note_is_error: false,
            menu_open: false,
            quit_armed: false,
            show_nav: false,
            camera_framed: false,
        })
    }

    pub fn locale(&self) -> &Locale {
        &self.regs.locale
    }

    /// A hot reload swapped the registries: zone handles and colours may
    /// have moved.
    pub fn set_registries(&mut self, regs: Arc<Registries>) {
        self.regs = regs;
        self.rebuild_view();
    }

    /// Rebuilds the sim view and the terrain from the document.
    pub fn rebuild_view(&mut self) {
        self.doc.resolve_zones(&self.regs);
        match self.doc.to_loaded(&self.regs) {
            Ok(loaded) => {
                self.loaded = loaded;
                self.terrain = TerrainMesh::build(&self.loaded, &self.regs);
                self.terrain_dirty = true;
            }
            Err(e) => self.set_note(e.to_string(), true),
        }
    }

    /// Frames the whole map the first time the window size is known.
    fn ensure_framed(&mut self, screen: Vec2) {
        if self.camera_framed || screen.x <= 1.0 {
            return;
        }
        self.camera_framed = true;
        let w = self.loaded.width.to_f32_render() + 2.0 * OUTLINE_STEP_M;
        let h = self.loaded.height.to_f32_render() + 2.0 * OUTLINE_STEP_M;
        let fit = (screen.x / w).min(screen.y / (h * self.camera.pitch));
        self.camera.zoom = fit.clamp(Camera::MIN_ZOOM, Camera::DEFAULT_ZOOM);
    }

    fn set_note(&mut self, text: String, error: bool) {
        self.note = Some(text);
        self.note_is_error = error;
    }

    /// Records the document before an edit; the caller then edits `doc`
    /// and calls [`edited`](Self::edited).
    pub fn begin_edit(&mut self) {
        self.history.push(self.doc.clone());
    }

    /// After an edit: marks the document dirty and rebuilds the view.
    pub fn edited(&mut self) {
        self.doc.dirty = true;
        self.quit_armed = false;
        self.rebuild_view();
    }

    pub fn undo(&mut self) {
        if self.history.undo(&mut self.doc) {
            self.quit_armed = false;
            self.rebuild_view();
        }
    }

    pub fn redo(&mut self) {
        if self.history.redo(&mut self.doc) {
            self.quit_armed = false;
            self.rebuild_view();
        }
    }

    /// Saves into the target folder; the outcome lands in the note.
    pub fn save(&mut self) -> Option<EditorEffect> {
        let l = self.regs.clone();
        let l = &l.locale;
        if self.target.trim().is_empty() {
            self.set_note(l.get("il.editor.no_target").to_string(), true);
            return None;
        }
        let root = PathBuf::from(self.target.trim());
        match self.doc.save(&root) {
            Ok(saved) => {
                self.set_note(
                    l.fmt(
                        "il.editor.saved",
                        &[
                            ("json5", &display(&saved.json5)),
                            ("hgt", &display(&saved.hgt)),
                        ],
                    ),
                    false,
                );
                self.quit_armed = false;
                Some(EditorEffect::Saved(saved))
            }
            Err(e) => {
                self.set_note(
                    l.fmt("il.editor.save_failed", &[("error", &e.to_string())]),
                    true,
                );
                None
            }
        }
    }

    /// The frame's keys and gestures (the camera keys are the app's).
    pub fn handle_input(&mut self, input: &EditorInput<'_>) -> Vec<EditorEffect> {
        self.ensure_framed(input.screen);
        let mut effects = Vec::new();
        let (b, i) = (input.bindings, input.input);
        if input.keyboard_in_ui {
            return effects;
        }
        if i.pressed(b, Action::PauseMenu) {
            self.menu_open = !self.menu_open;
            self.quit_armed = false;
        }
        if i.pressed(b, Action::QuickSave)
            && let Some(e) = self.save()
        {
            effects.push(e);
        }
        if i.pressed(b, Action::EditorUndo) {
            self.undo();
        }
        if i.pressed(b, Action::EditorRedo) {
            self.redo();
        }
        if i.pressed(b, Action::DebugNavGrid) {
            self.show_nav = !self.show_nav;
        }
        if std::mem::take(&mut self.terrain_dirty) {
            effects.push(EditorEffect::TerrainChanged);
        }
        effects
    }

    /// Draws the panels and applies their clicks.
    pub fn ui(&mut self, ctx: &egui::Context) -> Vec<EditorEffect> {
        let actions = panels::draw(ctx, self);
        let mut effects = Vec::new();
        for action in actions {
            match action {
                PanelAction::Undo => self.undo(),
                PanelAction::Redo => self.redo(),
                PanelAction::Save => {
                    if let Some(e) = self.save() {
                        effects.push(e);
                    }
                }
                PanelAction::SetTarget(t) => {
                    self.target = t;
                    self.quit_armed = false;
                }
                PanelAction::OpenMenu => {
                    self.menu_open = true;
                    self.quit_armed = false;
                }
                PanelAction::CloseMenu => {
                    self.menu_open = false;
                    self.quit_armed = false;
                }
                PanelAction::Quit => {
                    if self.doc.dirty && !self.quit_armed {
                        self.quit_armed = true;
                    } else {
                        effects.push(EditorEffect::QuitToMenu);
                    }
                }
                PanelAction::Tool(tool) => self.tool = tool,
            }
        }
        if std::mem::take(&mut self.terrain_dirty) {
            effects.push(EditorEffect::TerrainChanged);
        }
        effects
    }

    /// The frame's line overlays: the map border, zone polygons in their
    /// type's colour, rivers with their width, the deployment polygons per
    /// side and the reinforcement edge bands.
    pub fn build_lines(&self, screen: Vec2, lines: &mut LineScene) {
        let map = &self.loaded;
        let cam = &self.camera;
        let (w, h) = (map.width.to_f32_render(), map.height.to_f32_render());
        ground_polyline(
            map,
            cam,
            screen,
            &[
                Vec2::ZERO,
                Vec2::new(w, 0.0),
                Vec2::new(w, h),
                Vec2::new(0.0, h),
            ],
            true,
            BORDER,
            lines,
        );
        for z in &self.doc.def.zones {
            let colour = z.zone.map_or([255, 255, 255, 200], |hd| {
                let c = self.regs.zones.get(hd).colour.0;
                [c[0], c[1], c[2], 220]
            });
            let pts: Vec<Vec2> = z
                .polygon
                .iter()
                .map(|p| Vec2::new(p.x.to_f32_render(), p.y.to_f32_render()))
                .collect();
            ground_polyline(map, cam, screen, &pts, true, colour, lines);
        }
        for r in &self.doc.def.rivers {
            let pts: Vec<Vec2> = r
                .points
                .iter()
                .map(|p| Vec2::new(p.x.to_f32_render(), p.y.to_f32_render()))
                .collect();
            ground_polyline(map, cam, screen, &pts, false, RIVER, lines);
            let half = r.width.to_f32_render() * 0.5;
            for side in [-1.0, 1.0] {
                let offset: Vec<Vec2> = pts
                    .windows(2)
                    .flat_map(|s| {
                        let d = (s[1] - s[0]).normalize_or_zero();
                        let n = Vec2::new(-d.y, d.x) * half * side;
                        [s[0] + n, s[1] + n]
                    })
                    .collect();
                for seg in offset.chunks(2) {
                    ground_polyline(map, cam, screen, seg, false, RIVER_EDGE, lines);
                }
            }
        }
        deployment_outlines(map, cam, screen, lines);
        for e in &self.doc.def.reinforcement_edges {
            let tint = side_tint(e.side);
            let b = EDGE_BAND_M;
            let (a, c) = match e.edge {
                il_data::MapEdge::South => (Vec2::new(0.0, b), Vec2::new(w, b)),
                il_data::MapEdge::North => (Vec2::new(0.0, h - b), Vec2::new(w, h - b)),
                il_data::MapEdge::West => (Vec2::new(b, 0.0), Vec2::new(b, h)),
                il_data::MapEdge::East => (Vec2::new(w - b, 0.0), Vec2::new(w - b, h)),
            };
            ground_polyline(map, cam, screen, &[a, c], false, tint, lines);
        }
    }

    /// ` — editor — rome:test_field*` for the window title.
    pub fn title_suffix(&self) -> String {
        format!(
            "{}{}",
            self.doc.def.id.as_str(),
            if self.doc.dirty { "*" } else { "" }
        )
    }
}

fn display(p: &std::path::Path) -> String {
    p.display().to_string().replace('\\', "/")
}

/// A polyline drawn along the ground, each edge in `OUTLINE_STEP_M` steps.
fn ground_polyline(
    map: &LoadedMap,
    camera: &Camera,
    screen: Vec2,
    points: &[Vec2],
    closed: bool,
    colour: [u8; 4],
    lines: &mut LineScene,
) {
    let n = points.len();
    if n < 2 {
        return;
    }
    let edges = if closed { n } else { n - 1 };
    for k in 0..edges {
        let a = points[k];
        let b = points[(k + 1) % n];
        let steps = ((b - a).length() / OUTLINE_STEP_M).ceil().max(1.0) as u32;
        let mut prev = project(map, camera, screen, a);
        for s in 1..=steps {
            let t = s as f32 / steps as f32;
            let cur = project(map, camera, screen, a + (b - a) * t);
            lines.segment(prev, cur, colour);
            prev = cur;
        }
    }
}
