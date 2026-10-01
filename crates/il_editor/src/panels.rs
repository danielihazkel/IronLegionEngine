//! The editor's egui panels (T3-060, TDD §16): the top bar (the map's id,
//! undo, redo, save, the target mod folder with its quick picks, the menu
//! button and the last note), the left tool panel with the map summary,
//! and the Escape menu. Panels draw the session and return clicks; the
//! session applies them. Every label is an `il.editor.*` locale key.

use std::fmt::Display;

use il_data::{ContentId, Locale};

use crate::brush::{Falloff, HeightOp};
use crate::session::{BRUSH_MAX_M, BRUSH_MIN_M, BrushSettings, EditorSession, Tool};

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
}

/// Where the tool window sits, below the top bar (logical points).
const TOOLS_TOP: f32 = 72.0;

/// The tools the panel lists with their locale keys.
const TOOLS: &[(Tool, &str)] = &[
    (Tool::Select, "il.editor.tool_select"),
    (Tool::HeightBrush, "il.editor.tool_height"),
    (Tool::ZoneBrush, "il.editor.tool_zone"),
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
        Tool::Select => {}
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
            ui.separator();
            ui.heading(l.get("il.editor.map_title"));
            let def = &s.doc.def;
            ui.label(l.fmt(
                "il.editor.size_line",
                &[
                    ("w", &def.size.w as &dyn Display),
                    ("h", &def.size.h),
                    ("cell", &def.heightmap.cell),
                ],
            ));
            ui.label(l.fmt(
                "il.editor.tags",
                &[("list", &def.campaign_terrain_tags.join(", "))],
            ));
            ui.label(l.fmt(
                "il.editor.weather",
                &[("list", &def.weather_allowed.join(", "))],
            ));
            ui.label(l.fmt("il.editor.base_zone", &[("zone", &def.base_zone.as_str())]));
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
