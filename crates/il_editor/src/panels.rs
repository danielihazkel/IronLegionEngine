//! The editor's egui panels (T3-060, TDD §16): the top bar (the map's id,
//! undo, redo, save, the target mod folder with its quick picks, the menu
//! button and the last note), the left tool panel with the map summary,
//! and the Escape menu. Panels draw the session and return clicks; the
//! session applies them. Every label is an `il.editor.*` locale key.

use std::fmt::Display;

use il_data::{ContentId, Locale, MapEdge};

use crate::brush::{Falloff, HeightOp};
use crate::session::{
    BRUSH_MAX_M, BRUSH_MIN_M, BrushSettings, EditorSession, MetaDraft, Tool, VectorSettings,
};

/// What a panel click asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum PanelAction {
    Undo,
    Redo,
    Save,
    SetTarget(String),
    OpenMenu,
    CloseMenu,
    Quit,
    Tool(Tool),
    /// The brush parameters changed (T3-061).
    Brush(BrushSettings),
    /// The vector tools' parameters changed (T3-062).
    Vector(VectorSettings),
    /// Commit or drop the polyline or polygon in progress.
    Finish,
    Cancel,
    DeleteSelected,
    ToggleEdge(u8, MapEdge),
    /// The metadata fields as typed; `MetaApply` commits them.
    MetaEdit(MetaDraft),
    MetaApply,
}

const EDGES: &[(MapEdge, &str)] = &[
    (MapEdge::North, "il.editor.edge_north"),
    (MapEdge::South, "il.editor.edge_south"),
    (MapEdge::West, "il.editor.edge_west"),
    (MapEdge::East, "il.editor.edge_east"),
];

const WEATHERS: &[&str] = &["clear", "rain", "fog"];

/// A zone type picker over every loaded zone type.
fn zone_combo(ui: &mut egui::Ui, salt: &str, s: &EditorSession, value: &mut ContentId) {
    let mut ids: Vec<&ContentId> = s.regs.zones.all_ids().collect();
    ids.sort();
    egui::ComboBox::from_id_salt(salt)
        .selected_text(value.as_str())
        .show_ui(ui, |ui| {
            for id in ids {
                ui.selectable_value(value, id.clone(), id.as_str());
            }
        });
}

/// The vector tools' controls, the draft's Finish and Cancel, the select
/// tool's selection (T3-062).
fn vector_controls(ui: &mut egui::Ui, s: &EditorSession, l: &Locale, out: &mut Vec<PanelAction>) {
    let mut v = s.vector.clone();
    match s.tool {
        Tool::River => {
            ui.add(
                egui::Slider::new(&mut v.river_width, 2.0..=80.0).text(l.get("il.editor.width")),
            );
        }
        Tool::Road => {
            zone_combo(ui, "il_editor_road_zone", s, &mut v.road_zone);
            ui.add(egui::Slider::new(&mut v.road_width, 2.0..=40.0).text(l.get("il.editor.width")));
        }
        Tool::Polygon => {
            zone_combo(ui, "il_editor_polygon_zone", s, &mut v.polygon_zone);
            ui.weak(l.get("il.editor.polygon_hint"));
        }
        Tool::Deployment => {
            ui.add(egui::Slider::new(&mut v.side, 0..=7).text(l.get("il.editor.side")));
            ui.label(l.get("il.editor.edges"));
            ui.horizontal_wrapped(|ui| {
                for (edge, key) in EDGES {
                    let on = s
                        .doc
                        .def
                        .reinforcement_edges
                        .iter()
                        .any(|e| e.side == v.side && e.edge == *edge);
                    if ui.selectable_label(on, l.get(key)).clicked() {
                        out.push(PanelAction::ToggleEdge(v.side, *edge));
                    }
                }
            });
        }
        Tool::Structure => {
            ui.horizontal(|ui| {
                for kind in ["wall", "gate", "tower"] {
                    ui.selectable_value(&mut v.structure, kind, kind);
                }
            });
            ui.add(egui::Slider::new(&mut v.side, 0..=7).text(l.get("il.editor.side")));
            ui.weak(l.get("il.editor.inert_hint"));
        }
        Tool::SiegePoint => {
            ui.horizontal(|ui| {
                for kind in ["ladder", "ram", "tower"] {
                    ui.selectable_value(&mut v.siege, kind, kind);
                }
            });
            ui.weak(l.get("il.editor.siege_hint"));
        }
        Tool::Select => {
            ui.weak(l.get("il.editor.select_hint"));
            if let Some(f) = s.selected {
                ui.label(format!("{f:?}"));
                if ui.button(l.get("il.editor.delete")).clicked() {
                    out.push(PanelAction::DeleteSelected);
                }
            }
        }
        Tool::HeightBrush | Tool::ZoneBrush => {}
    }
    if !s.draft.is_empty() {
        ui.label(l.fmt("il.editor.draft_line", &[("n", &s.draft.len())]));
        ui.horizontal(|ui| {
            if ui.button(l.get("il.editor.finish")).clicked() {
                out.push(PanelAction::Finish);
            }
            if ui.button(l.get("il.editor.cancel")).clicked() {
                out.push(PanelAction::Cancel);
            }
        });
    }
    if v != s.vector {
        out.push(PanelAction::Vector(v));
    }
}

/// The metadata fields over the session's draft; Apply commits them.
fn metadata(ui: &mut egui::Ui, s: &EditorSession, l: &Locale, out: &mut Vec<PanelAction>) {
    let mut m = s.meta.clone();
    egui::Grid::new("il_editor_meta")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label(l.get("il.editor.new_id"));
            ui.text_edit_singleline(&mut m.id);
            ui.end_row();
            ui.label(l.get("il.editor.name_key"));
            ui.text_edit_singleline(&mut m.name_key);
            ui.end_row();
            ui.label(l.get("il.editor.new_size"));
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut m.size[0]).range(1.0..=8192.0));
                ui.add(egui::DragValue::new(&mut m.size[1]).range(1.0..=8192.0));
            });
            ui.end_row();
            ui.label(l.get("il.editor.tags_label"));
            ui.text_edit_singleline(&mut m.tags);
            ui.end_row();
            ui.label(l.get("il.editor.weather_label"));
            ui.horizontal(|ui| {
                for w in WEATHERS {
                    let mut on = m.weather.iter().any(|x| x == w);
                    if ui.checkbox(&mut on, *w).changed() {
                        if on {
                            m.weather.push((*w).to_string());
                        } else {
                            m.weather.retain(|x| x != w);
                        }
                    }
                }
            });
            ui.end_row();
            ui.label(l.get("il.editor.new_base_zone"));
            zone_combo(ui, "il_editor_base_zone", s, &mut m.base_zone);
            ui.end_row();
        });
    if m != s.meta {
        out.push(PanelAction::MetaEdit(m.clone()));
    }
    let changed = m != MetaDraft::of(&s.doc);
    if ui
        .add_enabled(changed, egui::Button::new(l.get("il.editor.apply")))
        .clicked()
    {
        out.push(PanelAction::MetaApply);
    }
}

/// Where the tool window sits, below the top bar (logical points).
const TOOLS_TOP: f32 = 72.0;

/// The tools the panel lists with their locale keys.
const TOOLS: &[(Tool, &str)] = &[
    (Tool::Select, "il.editor.tool_select"),
    (Tool::HeightBrush, "il.editor.tool_height"),
    (Tool::ZoneBrush, "il.editor.tool_zone"),
    (Tool::River, "il.editor.tool_river"),
    (Tool::Road, "il.editor.tool_road"),
    (Tool::Polygon, "il.editor.tool_polygon"),
    (Tool::Deployment, "il.editor.tool_deployment"),
    (Tool::Structure, "il.editor.tool_structure"),
    (Tool::SiegePoint, "il.editor.tool_siege"),
];

const OPS: &[(HeightOp, &str)] = &[
    (HeightOp::Raise, "il.editor.op_raise"),
    (HeightOp::Lower, "il.editor.op_lower"),
    (HeightOp::Smooth, "il.editor.op_smooth"),
    (HeightOp::Flatten, "il.editor.op_flatten"),
];

const FALLOFFS: &[(Falloff, &str)] = &[
    (Falloff::Smooth, "il.editor.falloff_smooth"),
    (Falloff::Linear, "il.editor.falloff_linear"),
    (Falloff::Constant, "il.editor.falloff_constant"),
];

/// The brush controls of the active tool; pushes `Brush` when one changed.
fn brush_controls(ui: &mut egui::Ui, s: &EditorSession, l: &Locale, out: &mut Vec<PanelAction>) {
    let mut b = s.brush.clone();
    match s.tool {
        Tool::HeightBrush => {
            ui.label(l.get("il.editor.op"));
            ui.horizontal_wrapped(|ui| {
                for (op, key) in OPS {
                    ui.selectable_value(&mut b.op, *op, l.get(key));
                }
            });
            ui.label(l.get("il.editor.falloff"));
            ui.horizontal_wrapped(|ui| {
                for (f, key) in FALLOFFS {
                    ui.selectable_value(&mut b.falloff, *f, l.get(key));
                }
            });
            ui.add(
                egui::Slider::new(&mut b.radius, BRUSH_MIN_M..=BRUSH_MAX_M)
                    .logarithmic(true)
                    .text(l.get("il.editor.radius")),
            );
            ui.add(
                egui::Slider::new(&mut b.strength, 0.1..=40.0)
                    .logarithmic(true)
                    .text(l.get("il.editor.strength")),
            );
        }
        Tool::ZoneBrush => {
            let mut ids: Vec<&ContentId> = s.regs.zones.all_ids().collect();
            ids.sort();
            egui::ComboBox::from_id_salt("il_editor_zone")
                .selected_text(b.zone.as_str())
                .show_ui(ui, |ui| {
                    for id in ids {
                        ui.selectable_value(&mut b.zone, id.clone(), id.as_str());
                    }
                });
            ui.add(
                egui::Slider::new(&mut b.zone_radius, BRUSH_MIN_M..=BRUSH_MAX_M)
                    .logarithmic(true)
                    .text(l.get("il.editor.radius")),
            );
            ui.weak(l.get("il.editor.zone_hint"));
        }
        _ => {}
    }
    if b != s.brush {
        out.push(PanelAction::Brush(b));
    }
}

/// Draws every panel; returns the clicks in order.
pub fn draw(ctx: &egui::Context, s: &EditorSession) -> Vec<PanelAction> {
    let l = s.locale();
    let mut out = Vec::new();
    top_bar(ctx, s, l, &mut out);
    tool_panel(ctx, s, l, &mut out);
    if s.menu_open {
        menu(ctx, s, l, &mut out);
    }
    out
}

fn top_bar(ctx: &egui::Context, s: &EditorSession, l: &Locale, out: &mut Vec<PanelAction>) {
    egui::Window::new("il_editor_top")
        .anchor(egui::Align2::LEFT_TOP, egui::vec2(8.0, 8.0))
        .title_bar(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong(format!(
                    "{}{}",
                    s.doc.def.id.as_str(),
                    if s.doc.dirty {
                        l.get("il.editor.dirty_mark")
                    } else {
                        ""
                    }
                ));
                ui.separator();
                if ui
                    .add_enabled(
                        s.history.can_undo(),
                        egui::Button::new(l.get("il.editor.undo")),
                    )
                    .clicked()
                {
                    out.push(PanelAction::Undo);
                }
                if ui
                    .add_enabled(
                        s.history.can_redo(),
                        egui::Button::new(l.get("il.editor.redo")),
                    )
                    .clicked()
                {
                    out.push(PanelAction::Redo);
                }
                ui.separator();
                ui.label(l.get("il.editor.target"));
                let mut target = s.target.clone();
                let field = ui.add(
                    egui::TextEdit::singleline(&mut target)
                        .hint_text(l.get("il.editor.target_hint"))
                        .desired_width(280.0),
                );
                if field.changed() {
                    out.push(PanelAction::SetTarget(target.clone()));
                }
                for root in &s.mod_roots {
                    let name = root
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| root.display().to_string());
                    if ui.small_button(name).clicked() {
                        out.push(PanelAction::SetTarget(
                            root.display().to_string().replace('\\', "/"),
                        ));
                    }
                }
                if ui.small_button("editor_out").clicked() {
                    out.push(PanelAction::SetTarget("tests/mods/editor_out".to_string()));
                }
                if ui
                    .add_enabled(
                        !s.target.trim().is_empty(),
                        egui::Button::new(l.get("il.editor.save")),
                    )
                    .clicked()
                {
                    out.push(PanelAction::Save);
                }
                ui.separator();
                if ui.button(l.get("il.editor.menu")).clicked() {
                    out.push(PanelAction::OpenMenu);
                }
            });
            if let Some(note) = &s.note {
                if s.note_is_error {
                    ui.colored_label(egui::Color32::from_rgb(255, 120, 120), note);
                } else {
                    ui.weak(note);
                }
            }
        });
}

fn tool_panel(ctx: &egui::Context, s: &EditorSession, l: &Locale, out: &mut Vec<PanelAction>) {
    egui::Window::new("il_editor_tools")
        .anchor(egui::Align2::LEFT_TOP, egui::vec2(8.0, TOOLS_TOP))
        .title_bar(false)
        .resizable(false)
        .default_width(220.0)
        .show(ctx, |ui| {
            ui.heading(l.get("il.editor.tools"));
            for (tool, key) in TOOLS {
                if ui.selectable_label(s.tool == *tool, l.get(key)).clicked() {
                    out.push(PanelAction::Tool(*tool));
                }
            }
            brush_controls(ui, s, l, out);
            vector_controls(ui, s, l, out);
            ui.separator();
            ui.heading(l.get("il.editor.map_title"));
            metadata(ui, s, l, out);
            let def = &s.doc.def;
            ui.label(l.fmt(
                "il.editor.counts",
                &[
                    ("zones", &def.zones.len() as &dyn Display),
                    ("rivers", &def.rivers.len()),
                    ("sides", &def.deployment.len()),
                    ("edges", &def.reinforcement_edges.len()),
                ],
            ));
            ui.separator();
            ui.weak(l.fmt(
                "il.editor.history_line",
                &[("steps", &s.history.undo_len())],
            ));
        });
}

fn menu(ctx: &egui::Context, s: &EditorSession, l: &Locale, out: &mut Vec<PanelAction>) {
    egui::Window::new(l.get("il.editor.menu_title"))
        .id(egui::Id::new("il_editor_menu"))
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .collapsible(false)
        .resizable(false)
        .default_width(260.0)
        .show(ctx, |ui| {
            ui.vertical_centered_justified(|ui| {
                if ui.button(l.get("il.editor.resume")).clicked() {
                    out.push(PanelAction::CloseMenu);
                }
                if ui
                    .add_enabled(
                        !s.target.trim().is_empty(),
                        egui::Button::new(l.get("il.editor.save")),
                    )
                    .clicked()
                {
                    out.push(PanelAction::Save);
                }
                if s.quit_armed {
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 180, 80),
                        l.get("il.editor.unsaved"),
                    );
                }
                if ui.button(l.get("il.editor.quit")).clicked() {
                    out.push(PanelAction::Quit);
                }
            });
        });
}
