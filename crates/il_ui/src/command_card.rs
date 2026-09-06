//! The command card (T2-090, REQ-UI-001, plan decision 9): the selected
//! regiments' rows (the T1-070 selection card, plan I7) and, under them,
//! every order a key can give: halt, attack-move (arms the cursor),
//! withdraw, the fire and run toggles, the first selected unit type's
//! formation templates, the ability slots and the group-formation presets.
//! Every button yields the `CommandAction` the app turns into the same
//! `UiIntent` the key would (REQ-INP-006). During the deployment the card
//! keeps only the formations and the presets, which then re-lay the whole
//! side in its zone (decision 6).

use std::fmt::Display;

use egui::Color32;
use il_data::{ContentId, Locale};
use il_sim_battle::FireMode;

use crate::panels::SelectedRegiment;

/// One ability slot as the card shows it (T2-050 lines).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbilitySlot {
    /// 1-based slot (the `ability_n` key).
    pub key: u8,
    pub name: String,
    /// Localised readiness (`ready` or the cooldown).
    pub state: String,
    pub ready: bool,
}

pub struct CommandCardModel<'a> {
    pub selection: &'a [SelectedRegiment],
    /// The first selected ranged regiment's fire mode; `None` without one.
    pub fire: Option<FireMode>,
    /// The run toggle.
    pub run: bool,
    pub armed_attack_move: bool,
    /// `(formation_n key, localised name)` of the first selected unit type.
    pub formations: Vec<(u8, String)>,
    pub abilities: Vec<AbilitySlot>,
    /// Group-formation presets: `(template id, localised name)`.
    pub presets: Vec<(ContentId, String)>,
    pub deploying: bool,
    pub locale: &'a Locale,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandAction {
    Halt,
    /// Arm (or disarm) the attack-move cursor.
    ArmAttackMove,
    Withdraw,
    ToggleFire,
    ToggleRun,
    /// `formation_n`.
    Formation(u8),
    /// `ability_n`.
    Ability(u8),
    /// A group-formation preset.
    Preset(ContentId),
}

/// The selected regiments' rows (unit, soldiers, formation, order, morale,
/// fatigue, abilities, statuses).
pub fn selection_grid(ui: &mut egui::Ui, rows: &[SelectedRegiment], l: &Locale) {
    egui::Grid::new("selection").striped(true).show(ui, |ui| {
        for r in rows {
            ui.monospace(format!("#{}", r.id.0));
            ui.label(&r.unit);
            ui.label(l.fmt("il.battle.soldiers", &[("count", &r.soldiers)]));
            ui.label(l.fmt(
                "il.battle.formation",
                &[
                    ("formation", &r.formation as &dyn Display),
                    ("ranks", &r.ranks),
                ],
            ));
            ui.label(&r.order);
            ui.label(&r.morale);
            ui.label(&r.fatigue);
            ui.label(r.abilities.join(" \u{b7} "));
            ui.label(r.statuses.join(" \u{b7} "));
            ui.end_row();
        }
    });
}

/// Draws the card; returns the click, if any. Nothing is drawn outside the
/// deployment when nothing is selected.
pub fn command_card(ctx: &egui::Context, model: &CommandCardModel<'_>) -> Option<CommandAction> {
    if model.selection.is_empty() && !model.deploying {
        return None;
    }
    let l = model.locale;
    let mut action = None;
    egui::Window::new("il_command_card")
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -8.0))
        .title_bar(false)
        .resizable(false)
        .show(ctx, |ui| {
            if model.selection.is_empty() {
                ui.label(l.get("il.command.none_selected"));
            } else {
                selection_grid(ui, model.selection, l);
            }
            ui.separator();
            if !model.deploying && !model.selection.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    if ui.button(l.get("il.command.halt")).clicked() {
                        action = Some(CommandAction::Halt);
                    }
                    let attack = if model.armed_attack_move {
                        egui::Button::new(l.get("il.command.attack_move_armed"))
                            .fill(Color32::from_rgb(120, 60, 30))
                    } else {
                        egui::Button::new(l.get("il.command.attack_move"))
                    };
                    if ui.add(attack).clicked() {
                        action = Some(CommandAction::ArmAttackMove);
                    }
                    if ui.button(l.get("il.command.withdraw")).clicked() {
                        action = Some(CommandAction::Withdraw);
                    }
                    if let Some(fire) = model.fire {
                        let label = if fire == FireMode::Hold {
                            l.get("il.command.fire_at_will")
                        } else {
                            l.get("il.command.hold_fire")
                        };
                        if ui.button(label).clicked() {
                            action = Some(CommandAction::ToggleFire);
                        }
                    }
                    let run = if model.run {
                        l.get("il.command.walk")
                    } else {
                        l.get("il.command.run")
                    };
                    if ui.button(run).clicked() {
                        action = Some(CommandAction::ToggleRun);
                    }
                });
            }
            if !model.formations.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for (key, name) in &model.formations {
                        let text = l.fmt(
                            "il.command.keyed",
                            &[("key", &format!("F{key}") as &dyn Display), ("name", name)],
                        );
                        if ui.button(text).clicked() {
                            action = Some(CommandAction::Formation(*key));
                        }
                    }
                });
            }
            if !model.deploying && !model.abilities.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for a in &model.abilities {
                        let text = l.fmt(
                            "il.battle.ability",
                            &[
                                ("key", &a.key as &dyn Display),
                                ("name", &a.name),
                                ("state", &a.state),
                            ],
                        );
                        if ui.add_enabled(a.ready, egui::Button::new(text)).clicked() {
                            action = Some(CommandAction::Ability(a.key));
                        }
                    }
                });
            }
            if !model.presets.is_empty() {
                let label = if model.deploying {
                    l.get("il.command.preset_deploy")
                } else {
                    l.get("il.command.preset")
                };
                egui::ComboBox::from_id_salt("il_presets")
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        for (id, name) in &model.presets {
                            if ui.selectable_label(false, name).clicked() {
                                action = Some(CommandAction::Preset(id.clone()));
                            }
                        }
                    });
            }
        });
    action
}
