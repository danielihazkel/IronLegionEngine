//! The vector tools and the metadata panel's edits on the session (T3-062,
//! TDD §16): rivers, roads, zone polygons (fords and bridges among them),
//! deployment polygons and reinforcement edges, structures and siege
//! points, the select tool's vertex drag, insert and delete, and the
//! metadata applied as one history step. Every committed edit is one
//! history step and rebuilds the view; a drag rebuilds once at its end.

use glam::Vec2;
use il_core::{S, Scalar};
use il_data::{ContentId, DeploymentZone, MapEdge, MapSize, ReinforcementEdge, River, ZonePolygon};
use il_render::terrain::project;
use il_ui::{Action, Gesture};
use serde_json::{Value, json};

use crate::document::MAX_ZONES;
use crate::session::{EditorInput, EditorSession, Tool};
use crate::vector::{
    Feature, json_pair, pick_edge, pick_vertex, record_points, resample, road_outline,
    set_record_points, to_v2, to_vec2,
};

/// Pixels within which a click picks a vertex or an edge.
pub const PICK_PX: f32 = 8.0;
/// The `hp` a placed structure carries (inert until Phase 5).
const STRUCTURE_HP: u32 = 1000;

impl EditorSession {
    /// The vector tools' gestures: `editor_paint` clicks add a point (or
    /// place a point record), `editor_alt` finishes; the select tool picks,
    /// drags, inserts and deletes vertices.
    pub(crate) fn vector_input(&mut self, input: &EditorInput<'_>) {
        let (b, i) = (input.bindings, input.input);
        if self.tool == Tool::Select {
            self.select_input(input);
            return;
        }
        if !matches!(
            self.tool,
            Tool::River
                | Tool::Road
                | Tool::Polygon
                | Tool::Deployment
                | Tool::Structure
                | Tool::SiegePoint
        ) || input.pointer_over_ui
        {
            return;
        }
        if let Some(Gesture::Click { pos, .. }) = i.gesture(b, Action::EditorPaint) {
            let at = self.ground_point(pos, input.screen);
            self.tool_click(at);
        }
        if let Some(Gesture::Click { .. }) = i.gesture(b, Action::EditorAlt) {
            self.tool_finish();
        }
    }

    /// One point of the active vector tool at world point `at`.
    pub fn tool_click(&mut self, at: Vec2) {
        let at = self.clamp_to_map(at);
        match self.tool {
            Tool::Structure if self.vector.structure != "wall" => {
                let rec = json!({
                    "kind": self.vector.structure,
                    "at": json_pair(at),
                    "hp": STRUCTURE_HP,
                    "faction_side": self.vector.side,
                });
                self.begin_edit();
                self.doc.def.structures.push(rec);
                self.edited();
            }
            Tool::SiegePoint if !self.draft.is_empty() => {
                let from = self.draft[0];
                let d = at - from;
                let facing = d.y.atan2(d.x).to_degrees().rem_euclid(360.0).round();
                let rec = json!({
                    "kind": self.vector.siege,
                    "at": json_pair(from),
                    "facing": facing,
                });
                self.draft.clear();
                self.begin_edit();
                self.doc.def.siege_points.push(rec);
                self.edited();
            }
            _ => self.draft.push(at),
        }
    }

    /// Commits the polyline or polygon in progress; too few points drop it.
    pub fn tool_finish(&mut self) {
        let pts = std::mem::take(&mut self.draft);
        let v = |p: &Vec2| to_v2(*p);
        match self.tool {
            Tool::River if pts.len() >= 2 => {
                self.begin_edit();
                self.doc.def.rivers.push(River {
                    width: S::from_f32_data(self.vector.river_width),
                    points: pts.iter().map(v).collect(),
                });
                self.edited();
            }
            Tool::Road if pts.len() >= 2 => {
                let outline = road_outline(&pts, self.vector.road_width);
                if outline.len() >= 3 && self.zone_room() {
                    // Roads go under the crossings, so a road drawn over a
                    // bridge or a ford keeps the crossing on top.
                    let at = self
                        .doc
                        .def
                        .zones
                        .iter()
                        .position(|z| z.zone.is_some_and(|h| self.regs.zones.get(h).crossing))
                        .unwrap_or(self.doc.def.zones.len());
                    self.begin_edit();
                    self.doc.def.zones.insert(
                        at,
                        ZonePolygon {
                            type_id: self.vector.road_zone.clone(),
                            zone: None,
                            polygon: outline.iter().map(v).collect(),
                        },
                    );
                    self.edited();
                }
            }
            Tool::Polygon if pts.len() >= 3 => {
                if !self.zone_room() {
                    return;
                }
                self.begin_edit();
                self.doc.def.zones.push(ZonePolygon {
                    type_id: self.vector.polygon_zone.clone(),
                    zone: None,
                    polygon: pts.iter().map(v).collect(),
                });
                self.edited();
            }
            Tool::Deployment if pts.len() >= 3 => {
                let side = self.vector.side;
                let zone = DeploymentZone {
                    side,
                    polygon: pts.iter().map(v).collect(),
                };
                self.begin_edit();
                match self.doc.def.deployment.iter_mut().find(|d| d.side == side) {
                    Some(d) => *d = zone,
                    None => {
                        self.doc.def.deployment.push(zone);
                        self.doc.def.deployment.sort_by_key(|d| d.side);
                    }
                }
                self.edited();
            }
            Tool::Structure if pts.len() >= 2 => {
                let rec = json!({
                    "kind": "wall",
                    "polyline": pts.iter().map(|p| json_pair(*p)).collect::<Vec<Value>>(),
                    "hp": STRUCTURE_HP,
                    "faction_side": self.vector.side,
                });
                self.begin_edit();
                self.doc.def.structures.push(rec);
                self.edited();
            }
            _ => {}
        }
    }

    /// Whether another zone polygon fits under [`MAX_ZONES`] (the unbaked
    /// paint counted); notes it when not.
    fn zone_room(&mut self) -> bool {
        let ok = self.doc.def.zones.len() + self.doc.raster_zones.len() < MAX_ZONES;
        if !ok {
            let regs = self.regs.clone();
            self.set_note(
                regs.locale.get("il.editor.too_many_zones").to_string(),
                true,
            );
        }
        ok
    }

    fn clamp_to_map(&self, p: Vec2) -> Vec2 {
        let (w, h) = (
            self.loaded.width.to_f32_render(),
            self.loaded.height.to_f32_render(),
        );
        Vec2::new(p.x.clamp(0.0, w), p.y.clamp(0.0, h))
    }

    /// Adds or removes the reinforcement edge `(side, edge)`.
    pub fn toggle_edge(&mut self, side: u8, edge: MapEdge) {
        self.begin_edit();
        let edges = &mut self.doc.def.reinforcement_edges;
        match edges.iter().position(|e| e.side == side && e.edge == edge) {
            Some(k) => {
                edges.remove(k);
            }
            None => {
                edges.push(ReinforcementEdge { side, edge });
            }
        }
        self.edited();
    }

    /// Every feature, in drawing order.
    pub fn features(&self) -> Vec<Feature> {
        let d = &self.doc.def;
        (0..d.zones.len())
            .map(Feature::Zone)
            .chain((0..d.rivers.len()).map(Feature::River))
            .chain((0..d.deployment.len()).map(Feature::Deployment))
            .chain((0..d.structures.len()).map(Feature::Structure))
            .chain((0..d.siege_points.len()).map(Feature::SiegePoint))
            .collect()
    }

    /// A feature's vertices in metres; `None` when it no longer exists.
    pub fn feature_points(&self, f: Feature) -> Option<Vec<Vec2>> {
        let d = &self.doc.def;
        let pts = |v: &[il_core::V2]| v.iter().map(|p| to_vec2(*p)).collect();
        Some(match f {
            Feature::Zone(i) => pts(&d.zones.get(i)?.polygon),
            Feature::River(i) => pts(&d.rivers.get(i)?.points),
            Feature::Deployment(i) => pts(&d.deployment.get(i)?.polygon),
            Feature::Structure(i) => record_points(d.structures.get(i)?),
            Feature::SiegePoint(i) => record_points(d.siege_points.get(i)?),
        })
    }

    /// Replaces a feature's vertices.
    pub fn set_feature_points(&mut self, f: Feature, pts: &[Vec2]) {
        let d = &mut self.doc.def;
        let v: Vec<il_core::V2> = pts.iter().map(|p| to_v2(*p)).collect();
        match f {
            Feature::Zone(i) => d.zones[i].polygon = v,
            Feature::River(i) => d.rivers[i].points = v,
            Feature::Deployment(i) => d.deployment[i].polygon = v,
            Feature::Structure(i) => set_record_points(&mut d.structures[i], pts),
            Feature::SiegePoint(i) => set_record_points(&mut d.siege_points[i], pts),
        }
    }

    /// Removes a whole feature.
    fn remove_feature(&mut self, f: Feature) {
        let d = &mut self.doc.def;
        match f {
            Feature::Zone(i) => {
                d.zones.remove(i);
            }
            Feature::River(i) => {
                d.rivers.remove(i);
            }
            Feature::Deployment(i) => {
                d.deployment.remove(i);
            }
            Feature::Structure(i) => {
                d.structures.remove(i);
            }
            Feature::SiegePoint(i) => {
                d.siege_points.remove(i);
            }
        }
        self.selected = None;
    }

    /// The selected feature removed (the panel's Delete).
    pub fn delete_selected(&mut self) {
        if let Some(f) = self.selected {
            self.begin_edit();
            self.remove_feature(f);
            self.edited();
        }
    }

    /// The vertex nearest screen point `cursor` within [`PICK_PX`], over
    /// every feature (the selected one first on a tie).
    pub fn pick_vertex_at(&self, cursor: Vec2, screen: Vec2) -> Option<(Feature, usize)> {
        let mut best: Option<(Feature, usize, f32)> = None;
        let mut feats = self.features();
        if let Some(sel) = self.selected {
            feats.retain(|f| *f != sel);
            feats.insert(0, sel);
        }
        for f in feats {
            let Some(pts) = self.feature_points(f) else {
                continue;
            };
            let scr: Vec<Vec2> = pts
                .iter()
                .map(|p| project(&self.loaded, &self.camera, screen, *p))
                .collect();
            if let Some((k, d)) = pick_vertex(&scr, cursor, PICK_PX)
                && best.is_none_or(|(_, _, bd)| d < bd)
            {
                best = Some((f, k, d));
            }
        }
        best.map(|(f, k, _)| (f, k))
    }

    /// The edge nearest `cursor` within [`PICK_PX`]: the feature, the index
    /// the new vertex takes.
    fn pick_edge_at(&self, cursor: Vec2, screen: Vec2) -> Option<(Feature, usize)> {
        let mut best: Option<(Feature, usize, f32)> = None;
        for f in self.features() {
            let Some(pts) = self.feature_points(f) else {
                continue;
            };
            if pts.len() < 2 {
                continue;
            }
            let scr: Vec<Vec2> = pts
                .iter()
                .map(|p| project(&self.loaded, &self.camera, screen, *p))
                .collect();
            if let Some((k, _, d)) = pick_edge(&scr, f.closed(), cursor, PICK_PX)
                && best.is_none_or(|(_, _, bd)| d < bd)
            {
                best = Some((f, k + 1, d));
            }
        }
        best.map(|(f, k, _)| (f, k))
    }

    /// Moves vertex `k` of `f` to `at` (no history step; the drag owns it).
    pub fn move_vertex(&mut self, f: Feature, k: usize, at: Vec2) {
        let at = self.clamp_to_map(at);
        if let Some(mut pts) = self.feature_points(f)
            && k < pts.len()
        {
            pts[k] = at;
            self.set_feature_points(f, &pts);
        }
    }

    /// Inserts a vertex at `at` before index `k` of `f` (one history step).
    pub fn insert_vertex(&mut self, f: Feature, k: usize, at: Vec2) {
        let at = self.clamp_to_map(at);
        if let Some(mut pts) = self.feature_points(f) {
            self.begin_edit();
            pts.insert(k.min(pts.len()), at);
            self.set_feature_points(f, &pts);
            self.selected = Some(f);
            self.edited();
        }
    }

    /// Deletes vertex `k` of `f`; a feature left under its minimum is
    /// removed whole (one history step).
    pub fn delete_vertex(&mut self, f: Feature, k: usize) {
        let Some(mut pts) = self.feature_points(f) else {
            return;
        };
        self.begin_edit();
        if pts.len() <= f.min_vertices() {
            self.remove_feature(f);
        } else {
            pts.remove(k);
            self.set_feature_points(f, &pts);
        }
        self.edited();
    }

    /// The select tool: a click on a vertex selects its feature, on an
    /// edge inserts a vertex there, elsewhere clears the selection; a drag
    /// from a vertex moves it (one history step, the view rebuilt at the
    /// end); `editor_alt` on a vertex deletes it.
    fn select_input(&mut self, input: &EditorInput<'_>) {
        let (b, i, screen) = (input.bindings, input.input, input.screen);
        if let Some(drag) = i.drag(b, Action::EditorPaint) {
            if self.dragging.is_none() {
                if input.pointer_over_ui {
                    return;
                }
                let Some((f, k)) = self.pick_vertex_at(drag.from, screen) else {
                    return;
                };
                self.begin_edit();
                self.dragging = Some((f, k));
                self.selected = Some(f);
            }
            if let Some((f, k)) = self.dragging {
                let at = self.ground_point(drag.to, screen);
                self.move_vertex(f, k, at);
            }
            return;
        }
        if self.dragging.take().is_some() {
            self.edited();
        }
        if input.pointer_over_ui {
            return;
        }
        if let Some(Gesture::Click { pos, .. }) = i.gesture(b, Action::EditorPaint) {
            if let Some((f, _)) = self.pick_vertex_at(pos, screen) {
                self.selected = Some(f);
            } else if let Some((f, k)) = self.pick_edge_at(pos, screen) {
                let at = self.ground_point(pos, screen);
                self.insert_vertex(f, k, at);
            } else {
                self.selected = None;
            }
        }
        if let Some(Gesture::Click { pos, .. }) = i.gesture(b, Action::EditorAlt)
            && let Some((f, k)) = self.pick_vertex_at(pos, screen)
        {
            self.delete_vertex(f, k);
        }
    }

    /// Applies the metadata panel's draft as one history step; a bad id or
    /// size is noted and nothing changes. A size change keeps the height
    /// samples and painted cells that still fit and repeats the edge past
    /// them.
    pub fn apply_meta(&mut self) {
        let m = self.meta.clone();
        let regs = self.regs.clone();
        let l = &regs.locale;
        let Ok(id) = ContentId::new(m.id.trim()) else {
            self.set_note(l.fmt("il.editor.bad_id", &[("id", &m.id)]), true);
            return;
        };
        let [w, h] = m.size;
        if !(1.0..=8192.0).contains(&w) || !(1.0..=8192.0).contains(&h) {
            self.set_note(l.get("il.editor.bad_size").to_string(), true);
            return;
        }
        self.begin_edit();
        let (old_cols, old_rows) = self.doc.height_dims();
        let d = &mut self.doc.def;
        d.id = id;
        d.name_key = m.name_key.trim().to_string();
        d.campaign_terrain_tags = m
            .tags
            .split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect();
        d.weather_allowed = m.weather.clone();
        d.base_zone = m.base_zone.clone();
        let resized = d.size.w.to_f32_render() != w || d.size.h.to_f32_render() != h;
        if resized {
            d.size = MapSize {
                w: S::from_f32_data(w),
                h: S::from_f32_data(h),
            };
            let (cols, rows) = self.doc.height_dims();
            self.doc.heights = resample(&self.doc.heights, old_cols, old_rows, cols, rows);
            self.resize_raster();
            self.camera_framed = false;
        }
        self.edited();
    }

    /// The feature a diagnostic's field names (`zones[3].polygon`,
    /// `rivers[0]`, `deployment[1]`, `reinforcement_edges[0]` for its
    /// side's zone, `structures[2]`, `siege_points[0]`).
    pub fn feature_of_field(&self, field: &str) -> Option<Feature> {
        let (head, rest) = field.split_once('[')?;
        let index: usize = rest.split(']').next()?.parse().ok()?;
        Some(match head {
            "zones" => Feature::Zone(index),
            "rivers" => Feature::River(index),
            "deployment" => Feature::Deployment(index),
            "structures" => Feature::Structure(index),
            "siege_points" => Feature::SiegePoint(index),
            "reinforcement_edges" => {
                let side = self.doc.def.reinforcement_edges.get(index)?.side;
                Feature::Deployment(
                    self.doc
                        .def
                        .deployment
                        .iter()
                        .position(|d| d.side == side)?,
                )
            }
            _ => return None,
        })
    }

    /// Centres the camera on the feature `field` names and selects it.
    pub fn focus_field(&mut self, field: &str) {
        let Some(f) = self.feature_of_field(field) else {
            return;
        };
        let Some(pts) = self.feature_points(f).filter(|p| !p.is_empty()) else {
            return;
        };
        let sum = pts.iter().fold(Vec2::ZERO, |a, p| a + *p);
        self.camera.center = sum / pts.len() as f32;
        self.selected = Some(f);
    }

    /// Clips or pads the paint raster to the map's new zone grid.
    fn resize_raster(&mut self) {
        if self.doc.zone_raster.is_empty() {
            return;
        }
        let cell = self.doc.raster_cell;
        let cols = (self.doc.def.size.w.to_f32_render() / cell).ceil().max(1.0) as u32;
        let rows = (self.doc.def.size.h.to_f32_render() / cell).ceil().max(1.0) as u32;
        let (oc, or) = (self.doc.raster_cols, self.doc.raster_rows);
        let mut next = vec![0u8; cols as usize * rows as usize];
        for j in 0..rows.min(or) {
            for i in 0..cols.min(oc) {
                next[(j * cols + i) as usize] = self.doc.zone_raster[(j * oc + i) as usize];
            }
        }
        self.doc.zone_raster = next;
        self.doc.raster_cols = cols;
        self.doc.raster_rows = rows;
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use super::*;
    use crate::document::{BlankMap, MapDocument};

    fn session() -> EditorSession {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game");
        let regs = Arc::new(il_data::load_roots(&[root]).unwrap_or_else(|d| panic!("{d}")));
        let doc = MapDocument::blank(
            &BlankMap {
                id: ContentId::new("edt:t").unwrap(),
                size: [400.0, 300.0],
                height_cell: 4.0,
                base_zone: ContentId::new("rome:open").unwrap(),
            },
            &regs,
        )
        .unwrap();
        EditorSession::open(doc, regs, Vec::new(), None).unwrap()
    }

    #[test]
    fn polygons_rivers_and_roads_commit_one_step_each() {
        let mut s = session();
        s.tool = Tool::Polygon;
        s.vector.polygon_zone = ContentId::new("rome:bridge").unwrap();
        for p in [(196.0, 140.0), (204.0, 140.0), (204.0, 160.0)] {
            s.tool_click(Vec2::new(p.0, p.1));
        }
        s.tool_finish();
        s.tool = Tool::Road;
        s.vector.road_zone = ContentId::new("rome:road").unwrap();
        s.tool_click(Vec2::new(200.0, 0.0));
        s.tool_click(Vec2::new(200.0, 300.0));
        s.tool_finish();
        s.tool = Tool::River;
        s.tool_click(Vec2::new(0.0, 150.0));
        s.tool_finish(); // one point: dropped
        s.tool_click(Vec2::new(0.0, 150.0));
        s.tool_click(Vec2::new(400.0, 150.0));
        s.tool_finish();
        assert_eq!(s.history.undo_len(), 3);
        let z = &s.doc.def.zones;
        assert_eq!(z.len(), 2);
        assert_eq!(
            z[0].type_id.as_str(),
            "rome:road",
            "the road went under the bridge"
        );
        assert_eq!(z[1].type_id.as_str(), "rome:bridge");
        assert_eq!(s.doc.def.rivers.len(), 1);
        assert_eq!(s.doc.def.rivers[0].width, S::from_i32(12));
        assert!(s.loaded.river_at(il_core::V2::from_f32_data(100.0, 150.0)));
    }

    #[test]
    fn vertices_move_insert_and_delete() {
        let mut s = session();
        s.tool = Tool::Polygon;
        for p in [(10.0, 10.0), (50.0, 10.0), (50.0, 50.0), (10.0, 50.0)] {
            s.tool_click(Vec2::new(p.0, p.1));
        }
        s.tool_finish();
        let f = Feature::Zone(0);
        s.move_vertex(f, 2, Vec2::new(60.0, 70.0));
        s.insert_vertex(f, 1, Vec2::new(30.0, 0.0));
        assert_eq!(
            s.feature_points(f).unwrap(),
            vec![
                Vec2::new(10.0, 10.0),
                Vec2::new(30.0, 0.0),
                Vec2::new(50.0, 10.0),
                Vec2::new(60.0, 70.0),
                Vec2::new(10.0, 50.0),
            ]
        );
        s.move_vertex(f, 0, Vec2::new(-20.0, 900.0));
        assert_eq!(
            s.feature_points(f).unwrap()[0],
            Vec2::new(0.0, 300.0),
            "clamped to the map"
        );
        s.delete_vertex(f, 1);
        s.delete_vertex(f, 1);
        assert_eq!(s.feature_points(f).unwrap().len(), 3);
        s.delete_vertex(f, 0);
        assert!(
            s.doc.def.zones.is_empty(),
            "under three vertices: removed whole"
        );
        s.undo();
        assert_eq!(s.doc.def.zones.len(), 1);
    }

    #[test]
    fn deployment_edges_structures_and_siege_points() {
        let mut s = session();
        s.tool = Tool::Deployment;
        s.vector.side = 1;
        for p in [(0.0, 250.0), (400.0, 250.0), (400.0, 300.0)] {
            s.tool_click(Vec2::new(p.0, p.1));
        }
        s.tool_finish();
        assert_eq!(s.doc.def.deployment.len(), 2, "side 1 replaced");
        assert_eq!(s.doc.def.deployment[1].polygon.len(), 3);
        s.toggle_edge(1, MapEdge::North);
        assert!(!s.doc.def.reinforcement_edges.iter().any(|e| e.side == 1));
        s.toggle_edge(1, MapEdge::East);
        assert!(
            s.doc
                .def
                .reinforcement_edges
                .iter()
                .any(|e| e.side == 1 && e.edge == MapEdge::East)
        );
        s.tool = Tool::Structure;
        s.vector.structure = "tower";
        s.tool_click(Vec2::new(100.0, 100.0));
        s.vector.structure = "wall";
        s.tool_click(Vec2::new(10.0, 10.0));
        s.tool_click(Vec2::new(90.0, 10.0));
        s.tool_finish();
        s.tool = Tool::SiegePoint;
        s.vector.siege = "ram";
        s.tool_click(Vec2::new(200.0, 200.0));
        s.tool_click(Vec2::new(200.0, 250.0));
        let st = &s.doc.def.structures;
        assert_eq!(st[0]["kind"], "tower");
        assert_eq!(st[1]["polyline"].as_array().unwrap().len(), 2);
        assert_eq!(s.doc.def.siege_points[0]["facing"], 90.0);
        // The records survive the writer and the schema.
        let text = s.doc.text();
        assert!(text.contains("\"kind\":\"ram\""), "{text}");
    }

    #[test]
    fn metadata_applies_as_one_step_and_resizes() {
        let mut s = session();
        s.doc.heights[0] = 7.0;
        s.meta.id = "edt:renamed".into();
        s.meta.tags = "river, hills ,".into();
        s.meta.size = [600.0, 200.0];
        s.apply_meta();
        assert_eq!(s.doc.def.id.as_str(), "edt:renamed");
        assert_eq!(s.doc.def.campaign_terrain_tags, ["river", "hills"]);
        assert_eq!(s.doc.height_dims(), (151, 51));
        assert_eq!(s.doc.heights.len(), 151 * 51);
        assert_eq!(s.doc.heights[0], 7.0);
        assert_eq!(s.loaded.width, S::from_i32(600));
        assert_eq!(s.history.undo_len(), 1);
        s.meta.id = "Bad Id".into();
        s.apply_meta();
        assert_eq!(
            s.doc.def.id.as_str(),
            "edt:renamed",
            "a bad id changes nothing"
        );
        assert!(s.note_is_error);
    }
}
