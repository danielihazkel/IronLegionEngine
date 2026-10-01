//! The editor session (T3-060, TDD §16): the document, its history, the
//! active tool, the camera, the terrain the renderer shows and what the
//! frame's input and panel clicks do to them. The app owns the window, the
//! frame job and the transitions; it hands the session the frame's input
//! (`EditorInput`) and applies the `EditorEffect`s that come back.

use std::path::PathBuf;
use std::sync::Arc;

use glam::Vec2;
use il_core::{S, Scalar};
use il_data::{ContentId, Locale, Registries};
use il_render::terrain::{ground_height, project};
use il_render::{Camera, LineScene, TerrainMesh, side_tint};
use il_sim_battle::{LoadedMap, MapError};
use il_ui::{Action, Bindings, Gesture, InputState};

use crate::brush::{Block, Falloff, HeightDab, HeightOp, height_dab, zone_dab};
use crate::document::{History, MapDocument, Saved};
use crate::panels::{self, PanelAction};
use crate::vector::{record_points, to_vec2};

/// The active tool (T3-060 select, T3-061 the brushes, T3-062 the vector
/// tools). The parameters live in [`BrushSettings`] and
/// [`VectorSettings`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    /// Pick, drag, insert and delete vertices of every feature.
    Select,
    HeightBrush,
    ZoneBrush,
    /// A polyline appended to `rivers` with the panel's width.
    River,
    /// A polyline widened into a road zone polygon.
    Road,
    /// A zone polygon of the panel's type (fords and bridges included).
    Polygon,
    /// The deployment polygon of the panel's side (replacing the old one).
    Deployment,
    /// A wall polyline, or a gate or tower point (inert until Phase 5).
    Structure,
    /// A ladder, ram or tower point and its facing (inert until Phase 5).
    SiegePoint,
}

/// The vector tools' parameters, set in the tool panel (T3-062).
#[derive(Clone, Debug, PartialEq)]
pub struct VectorSettings {
    pub river_width: f32,
    pub road_width: f32,
    pub road_zone: ContentId,
    pub polygon_zone: ContentId,
    /// The side the deployment tool, the edge toggles and the structures'
    /// `faction_side` use.
    pub side: u8,
    /// `wall`, `gate` or `tower`.
    pub structure: &'static str,
    /// `ladder`, `ram` or `tower`.
    pub siege: &'static str,
}

/// The metadata panel's fields as typed, applied as one history step.
#[derive(Clone, Debug, PartialEq)]
pub struct MetaDraft {
    pub id: String,
    pub name_key: String,
    pub size: [f32; 2],
    /// Comma-separated.
    pub tags: String,
    pub weather: Vec<String>,
    pub base_zone: ContentId,
}

impl MetaDraft {
    pub fn of(doc: &MapDocument) -> Self {
        let d = &doc.def;
        Self {
            id: d.id.as_str().to_string(),
            name_key: d.name_key.clone(),
            size: [d.size.w.to_f32_render(), d.size.h.to_f32_render()],
            tags: d.campaign_terrain_tags.join(", "),
            weather: d.weather_allowed.clone(),
            base_zone: d.base_zone.clone(),
        }
    }
}

/// The brushes' parameters, set in the tool panel (T3-061).
#[derive(Clone, Debug, PartialEq)]
pub struct BrushSettings {
    pub op: HeightOp,
    pub falloff: Falloff,
    /// Height brush radius, metres.
    pub radius: f32,
    /// Metres per second (Raise, Lower) or approach rate per second.
    pub strength: f32,
    /// The zone type the zone brush paints.
    pub zone: ContentId,
    /// Zone brush radius, metres.
    pub zone_radius: f32,
}

/// Brush radius limits, metres; `editor_brush_grow` / `shrink` step by 25 %.
pub const BRUSH_MIN_M: f32 = 2.0;
pub const BRUSH_MAX_M: f32 = 400.0;
const BRUSH_STEP: f32 = 1.25;
/// The time a single click's dab counts for, seconds.
const CLICK_DT: f32 = 0.1;
const CURSOR: [u8; 4] = [255, 255, 255, 200];
/// A polyline in progress and the selected feature's vertex handles.
const DRAFT: [u8; 4] = [255, 220, 80, 230];
/// Structures and siege points (inert until Phase 5).
const INERT: [u8; 4] = [170, 170, 170, 220];

/// A stroke in progress: Flatten's target height.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Stroke {
    target: f32,
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
    pub brush: BrushSettings,
    pub vector: VectorSettings,
    /// The vector tool's points so far (T3-062).
    pub draft: Vec<Vec2>,
    /// The select tool's feature.
    pub selected: Option<crate::vector::Feature>,
    /// The vertex a select drag moves.
    pub dragging: Option<(crate::vector::Feature, usize)>,
    pub meta: MetaDraft,
    /// The ground point under the cursor this frame.
    pub cursor_world: Option<Vec2>,
    stroke: Option<Stroke>,
    pub(crate) camera_framed: bool,
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
        let mut zones: Vec<&ContentId> = regs.zones.all_ids().collect();
        zones.sort();
        let zone = zones
            .iter()
            .find(|z| z.as_str().ends_with(":forest"))
            .or(zones.first())
            .map_or_else(|| doc.def.base_zone.clone(), |z| (*z).clone());
        let brush = BrushSettings {
            op: HeightOp::Raise,
            falloff: Falloff::Smooth,
            radius: 40.0,
            strength: 4.0,
            zone,
            zone_radius: 16.0,
        };
        let pick = |suffix: &str, crossing: Option<bool>| -> ContentId {
            zones
                .iter()
                .find(|z| z.as_str().ends_with(suffix))
                .or_else(|| {
                    zones.iter().find(|z| {
                        crossing.is_none_or(|c| {
                            regs.zones
                                .lookup(z)
                                .is_some_and(|h| regs.zones.get(h).crossing == c)
                        })
                    })
                })
                .map_or_else(|| doc.def.base_zone.clone(), |z| (*z).clone())
        };
        let vector = VectorSettings {
            river_width: 12.0,
            road_width: 8.0,
            road_zone: pick(":road", Some(false)),
            polygon_zone: pick(":bridge", Some(true)),
            side: 0,
            structure: "wall",
            siege: "ladder",
        };
        let meta = MetaDraft::of(&doc);
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
            brush,
            vector,
            draft: Vec::new(),
            selected: None,
            dragging: None,
            meta,
            cursor_world: None,
            stroke: None,
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
        self.meta = MetaDraft::of(&self.doc);
        if self
            .selected
            .is_some_and(|f| self.feature_points(f).is_none())
        {
            self.selected = None;
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

    pub(crate) fn set_note(&mut self, text: String, error: bool) {
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
        let painted = !self.doc.zone_raster.is_empty();
        let result = self.doc.save(&root);
        if painted {
            // The paint became polygons: resolve them and redraw.
            self.rebuild_view();
        }
        match result {
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
            if self.draft.is_empty() {
                self.menu_open = !self.menu_open;
                self.quit_armed = false;
            } else {
                // Escape drops a polyline or polygon in progress first.
                self.draft.clear();
            }
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
        let grow = i.pressed(b, Action::EditorBrushGrow);
        let shrink = i.pressed(b, Action::EditorBrushShrink);
        if grow || shrink {
            let f = if grow { BRUSH_STEP } else { 1.0 / BRUSH_STEP };
            let r = match self.tool {
                Tool::ZoneBrush => &mut self.brush.zone_radius,
                _ => &mut self.brush.radius,
            };
            *r = (*r * f).clamp(BRUSH_MIN_M, BRUSH_MAX_M);
        }
        self.cursor_world = i.cursor().map(|c| self.ground_point(c, input.screen));
        self.brush_input(input);
        self.vector_input(input);
        if std::mem::take(&mut self.terrain_dirty) {
            effects.push(EditorEffect::TerrainChanged);
        }
        effects
    }

    /// The ground point under screen pixel `s`: the height-zero pick
    /// refined against the terrain so a hill is hit where it is drawn.
    pub fn ground_point(&self, s: Vec2, screen: Vec2) -> Vec2 {
        let cam = &self.camera;
        let mut p = cam.screen_to_world(s, screen);
        for _ in 0..3 {
            let h = ground_height(&self.loaded, p);
            p = cam.screen_to_world(s + Vec2::new(0.0, h * cam.zoom * cam.elevation), screen);
        }
        p
    }

    /// The brushes' gestures: a click is one dab, a drag a stroke of one
    /// dab per frame; one history step per gesture.
    fn brush_input(&mut self, input: &EditorInput<'_>) {
        if !matches!(self.tool, Tool::HeightBrush | Tool::ZoneBrush) {
            return;
        }
        let (b, i) = (input.bindings, input.input);
        if let Some(drag) = i.drag(b, Action::EditorPaint) {
            if self.stroke.is_none() && input.pointer_over_ui {
                return;
            }
            let at = self.ground_point(drag.to, input.screen);
            if self.stroke.is_none() {
                self.begin_stroke(at);
            }
            self.dab(at, input.dt);
            return;
        }
        if self.stroke.is_some() {
            self.end_stroke();
        }
        if input.pointer_over_ui {
            return;
        }
        if let Some(Gesture::Click { pos, .. }) = i.gesture(b, Action::EditorPaint) {
            let at = self.ground_point(pos, input.screen);
            self.begin_stroke(at);
            self.dab(at, CLICK_DT);
            self.end_stroke();
        }
    }

    /// Starts a brush stroke at `at`: one history step, Flatten's target.
    pub fn begin_stroke(&mut self, at: Vec2) {
        self.begin_edit();
        self.stroke = Some(Stroke {
            target: ground_height(&self.loaded, at),
        });
    }

    /// Ends the stroke: the map's mean height follows the new ground.
    pub fn end_stroke(&mut self) {
        if self.stroke.take().is_some() {
            self.loaded.refresh_mean_height();
            self.doc.dirty = true;
            self.quit_armed = false;
        }
    }

    /// One dab of the active brush at `at` over `dt` seconds; patches the
    /// sim view and the terrain of the touched block only.
    pub fn dab(&mut self, at: Vec2, dt: f32) {
        let Some(stroke) = self.stroke else {
            return;
        };
        match self.tool {
            Tool::HeightBrush => {
                let (cols, rows) = self.doc.height_dims();
                let scale = self.doc.def.heightmap.scale.to_f32_render();
                let block = height_dab(
                    &mut self.doc.heights,
                    cols,
                    rows,
                    self.loaded.height_cell.to_f32_render(),
                    65_535.0 * scale,
                    &HeightDab {
                        op: self.brush.op,
                        falloff: self.brush.falloff,
                        centre: at,
                        radius: self.brush.radius,
                        strength: self.brush.strength,
                        dt,
                        target: stroke.target,
                    },
                );
                self.patch_heights(block);
            }
            Tool::ZoneBrush => {
                let (cols, rows) = (self.loaded.zone_cols, self.loaded.zone_rows);
                let cell = self.loaded.zone_cell.to_f32_render();
                self.doc.ensure_raster(cols, rows, cell);
                let known = self.doc.raster_zones.len();
                let Some(k) = self.doc.raster_value(&self.brush.zone) else {
                    let regs = self.regs.clone();
                    self.set_note(
                        regs.locale.get("il.editor.too_many_zones").to_string(),
                        true,
                    );
                    return;
                };
                if self.doc.raster_zones.len() > known {
                    let Some(h) = self.regs.zones.lookup(&self.brush.zone) else {
                        return;
                    };
                    self.loaded.zone_handles.push(h);
                    self.terrain.refresh_palette(&self.loaded, &self.regs);
                }
                let block = zone_dab(
                    &mut self.doc.zone_raster,
                    cols,
                    rows,
                    cell,
                    at,
                    self.brush.zone_radius,
                    k,
                );
                self.patch_zones(block);
            }
            _ => {}
        }
    }

    /// Copies the quantised heights of `block` into the sim view and the
    /// terrain mesh.
    fn patch_heights(&mut self, block: Block) {
        if block.is_empty() {
            return;
        }
        let cols = self.loaded.height_cols;
        let scale = self.doc.def.heightmap.scale;
        let scale_f = scale.to_f32_render();
        for j in block.j0..block.j1 {
            for i in block.i0..block.i1 {
                let k = (j * cols + i) as usize;
                let raw = (self.doc.heights[k] / scale_f).round().clamp(0.0, 65_535.0) as u16;
                self.loaded.heights[k] = S::from_i32(i32::from(raw)) * scale;
            }
        }
        self.terrain
            .patch_heights(&self.loaded, block.i0, block.j0, block.i1, block.j1);
        self.terrain_dirty = true;
    }

    /// Copies the painted cells of `block` into the sim view and the
    /// terrain's zone texels.
    fn patch_zones(&mut self, block: Block) {
        if block.is_empty() {
            return;
        }
        let cols = self.loaded.zone_cols;
        for j in block.j0..block.j1 {
            for i in block.i0..block.i1 {
                let slot = (j * cols + i) as usize;
                let k = self.doc.zone_raster[slot];
                if k != 0 {
                    self.loaded.zones[slot] = self.doc.loaded_index(k);
                }
            }
        }
        self.terrain.patch_zones(
            &self.loaded,
            &self.regs,
            block.i0,
            block.j0,
            block.i1,
            block.j1,
        );
        self.terrain_dirty = true;
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
                PanelAction::Tool(tool) => {
                    self.end_stroke();
                    self.draft.clear();
                    self.tool = tool;
                }
                PanelAction::Brush(settings) => self.brush = settings,
                PanelAction::Vector(settings) => self.vector = settings,
                PanelAction::Finish => self.tool_finish(),
                PanelAction::Cancel => self.draft.clear(),
                PanelAction::DeleteSelected => self.delete_selected(),
                PanelAction::ToggleEdge(side, edge) => self.toggle_edge(side, edge),
                PanelAction::MetaEdit(draft) => self.meta = draft,
                PanelAction::MetaApply => self.apply_meta(),
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
        // From the document, so a dragged vertex moves its outline at once.
        for d in &self.doc.def.deployment {
            let pts: Vec<Vec2> = d.polygon.iter().map(|p| to_vec2(*p)).collect();
            ground_polyline(map, cam, screen, &pts, true, side_tint(d.side), lines);
        }
        for rec in self
            .doc
            .def
            .structures
            .iter()
            .chain(&self.doc.def.siege_points)
        {
            let pts = record_points(rec);
            if pts.len() == 1 {
                let p = pts[0];
                for d in [Vec2::new(3.0, 3.0), Vec2::new(3.0, -3.0)] {
                    ground_polyline(map, cam, screen, &[p - d, p + d], false, INERT, lines);
                }
            } else {
                ground_polyline(map, cam, screen, &pts, false, INERT, lines);
            }
        }
        if !self.draft.is_empty() {
            let mut pts = self.draft.clone();
            pts.extend(self.cursor_world);
            ground_polyline(map, cam, screen, &pts, false, DRAFT, lines);
        }
        if let Some(f) = self.selected
            && let Some(pts) = self.feature_points(f)
        {
            for p in pts {
                let s = project(map, cam, screen, p);
                lines.circle(s, 4.0, 8, DRAFT);
            }
        }
        let radius = match self.tool {
            Tool::HeightBrush => Some(self.brush.radius),
            Tool::ZoneBrush => Some(self.brush.zone_radius),
            _ => None,
        };
        if let (Some(r), Some(c)) = (radius, self.cursor_world) {
            let ring: Vec<Vec2> = (0..48)
                .map(|k| {
                    let a = k as f32 / 48.0 * std::f32::consts::TAU;
                    c + Vec2::new(a.cos(), a.sin()) * r
                })
                .collect();
            ground_polyline(map, cam, screen, &ring, true, CURSOR, lines);
        }
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
