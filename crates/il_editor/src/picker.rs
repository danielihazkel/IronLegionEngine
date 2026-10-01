//! The editor's picker screen (T3-060): the main menu's Editor button opens
//! it; it lists the registry's maps to open and a New form (id, size,
//! height cell, base zone) for a blank one.

use il_data::{ContentId, Locale, Registries};

use crate::document::BlankMap;

/// The largest map side the schema allows, metres.
pub const MAX_SIZE: f32 = 8192.0;
/// The largest height cell the schema allows, metres.
pub const MAX_CELL: f32 = 64.0;

#[derive(Clone, Debug, PartialEq)]
pub struct PickerState {
    /// The registry's maps: id and localised name.
    pub maps: Vec<(ContentId, String)>,
    /// The zone types the New form offers as the base zone.
    pub zones: Vec<(ContentId, String)>,
    pub new_id: String,
    pub new_w: f32,
    pub new_h: f32,
    pub new_cell: f32,
    /// Index into `zones`.
    pub new_zone: usize,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PickerAction {
    Open(ContentId),
    New(BlankMap),
    Back,
}

impl PickerState {
    /// Lists the content; the New form's id defaults to
    /// `<namespace>:new_map`, its base zone to the first passable type
    /// named `open` when there is one.
    pub fn new(regs: &Registries, namespace: &str) -> Self {
        let l = &regs.locale;
        let maps = regs
            .maps
            .iter()
            .map(|(_, m)| (m.id.clone(), l.get(&m.name_key).to_string()))
            .collect();
        let zones: Vec<(ContentId, String)> = regs
            .zones
            .iter()
            .filter(|(_, z)| z.passable)
            .map(|(_, z)| (z.id.clone(), l.get(&z.name_key).to_string()))
            .collect();
        let new_zone = zones
            .iter()
            .position(|(id, _)| id.as_str().ends_with(":open"))
            .unwrap_or(0);
        Self {
            maps,
            zones,
            new_id: format!("{namespace}:new_map"),
            new_w: 800.0,
            new_h: 600.0,
            new_cell: 4.0,
            new_zone,
            error: None,
        }
    }

    /// The form's blank map, or the localised reason it is not one.
    pub fn blank(&self, l: &Locale) -> Result<BlankMap, String> {
        let id = ContentId::new(self.new_id.trim())
            .map_err(|_| l.fmt("il.editor.bad_id", &[("id", &self.new_id)]))?;
        if !(self.new_w > 0.0
            && self.new_w <= MAX_SIZE
            && self.new_h > 0.0
            && self.new_h <= MAX_SIZE)
        {
            return Err(l.get("il.editor.bad_size").to_string());
        }
        if !(self.new_cell > 0.0 && self.new_cell <= MAX_CELL) {
            return Err(l.get("il.editor.bad_cell").to_string());
        }
        let base_zone = self
            .zones
            .get(self.new_zone)
            .map(|(id, _)| id.clone())
            .ok_or_else(|| l.get("il.editor.bad_zone").to_string())?;
        Ok(BlankMap {
            id,
            size: [self.new_w, self.new_h],
            height_cell: self.new_cell,
            base_zone,
        })
    }
}

/// Draws the screen; returns the click, if any.
pub fn picker_screen(
    ctx: &egui::Context,
    state: &mut PickerState,
    l: &Locale,
) -> Option<PickerAction> {
    let mut action = None;
    egui::Window::new("il_editor_picker")
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .title_bar(false)
        .resizable(false)
        .default_width(420.0)
        .show(ctx, |ui| {
            ui.heading(l.get("il.editor.picker_title"));
            ui.add_space(8.0);
            ui.label(l.get("il.editor.open"));
            egui::ScrollArea::vertical()
                .max_height(240.0)
                .show(ui, |ui| {
                    for (id, name) in &state.maps {
                        if ui.button(format!("{name}  ({})", id.as_str())).clicked() {
                            action = Some(PickerAction::Open(id.clone()));
                        }
                    }
                });
            ui.separator();
            ui.label(l.get("il.editor.new_title"));
            egui::Grid::new("il_editor_new")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label(l.get("il.editor.new_id"));
                    ui.text_edit_singleline(&mut state.new_id);
                    ui.end_row();
                    ui.label(l.get("il.editor.new_size"));
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut state.new_w)
                                .range(1.0..=MAX_SIZE)
                                .speed(10.0),
                        );
                        ui.label("×");
                        ui.add(
                            egui::DragValue::new(&mut state.new_h)
                                .range(1.0..=MAX_SIZE)
                                .speed(10.0),
                        );
                    });
                    ui.end_row();
                    ui.label(l.get("il.editor.new_cell"));
                    ui.add(
                        egui::DragValue::new(&mut state.new_cell)
                            .range(0.5..=MAX_CELL)
                            .speed(0.5),
                    );
                    ui.end_row();
                    ui.label(l.get("il.editor.new_base_zone"));
                    let current = state
                        .zones
                        .get(state.new_zone)
                        .map_or("", |(_, n)| n.as_str())
                        .to_string();
                    egui::ComboBox::from_id_salt("il_editor_base_zone")
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            for (i, (_, name)) in state.zones.iter().enumerate() {
                                ui.selectable_value(&mut state.new_zone, i, name);
                            }
                        });
                    ui.end_row();
                });
            if ui.button(l.get("il.editor.create")).clicked() {
                match state.blank(l) {
                    Ok(b) => action = Some(PickerAction::New(b)),
                    Err(e) => state.error = Some(e),
                }
            }
            if let Some(e) = &state.error {
                ui.colored_label(egui::Color32::from_rgb(255, 120, 120), e);
            }
            ui.add_space(12.0);
            if ui.button(l.get("il.menu.back")).clicked() {
                action = Some(PickerAction::Back);
            }
        });
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn regs() -> Registries {
        let root: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game");
        il_data::load_roots(&[root]).unwrap_or_else(|d| panic!("{d}"))
    }

    #[test]
    fn the_form_validates_its_fields() {
        let regs = regs();
        let l = &regs.locale;
        let mut state = PickerState::new(&regs, "mymod");
        assert!(
            state
                .maps
                .iter()
                .any(|(id, _)| id.as_str() == "rome:test_field")
        );
        assert_eq!(state.zones[state.new_zone].0.as_str(), "rome:open");
        let blank = state.blank(l).unwrap();
        assert_eq!(blank.id.as_str(), "mymod:new_map");
        assert_eq!(blank.size, [800.0, 600.0]);
        state.new_id = "Bad Id".into();
        assert!(state.blank(l).is_err());
        state.new_id = "mymod:x".into();
        state.new_w = 0.0;
        assert!(state.blank(l).is_err());
        state.new_w = 100.0;
        state.new_cell = 100.0;
        assert!(state.blank(l).is_err());
    }

    /// The screen draws headless.
    #[test]
    fn the_picker_draws_headless() {
        let regs = regs();
        let mut state = PickerState::new(&regs, "rome");
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        assert!(picker_screen(&ctx, &mut state, &regs.locale).is_none());
        let mut out = ctx.end_pass();
        out.textures_delta.clear();
    }
}
