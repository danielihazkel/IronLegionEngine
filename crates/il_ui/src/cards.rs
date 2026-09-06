//! The regiment card strip (T2-090, REQ-UI-001, plan decision 8): one card
//! per own regiment down the left edge, scrolling past a dozen. A card is
//! the unit's name, its strength bar (soldiers over the initial count), a
//! morale dot coloured by state, the fatigue state, the volleys left of a
//! ranged unit, an engaged mark and the control group it belongs to.
//! Clicking selects, Shift-click adds, a double click centres the camera.
//! During the deployment the same strip is the tray (decision 6).

use egui::{Color32, Sense, Stroke, StrokeKind};
use il_core::RegimentId;
use il_data::Locale;
use il_sim_battle::components::MoraleState;

/// Card width in points.
pub const CARD_WIDTH: f32 = 180.0;
/// The strip never grows past this share of the window height.
const STRIP_HEIGHT_FRACTION: f32 = 0.7;

/// Morale dot colours by state: steady, unsettled, shaken, broken,
/// routing, shattered (the F10 overlay's palette).
pub const MORALE_COLOURS: [[u8; 3]; 6] = [
    [80, 220, 80],
    [200, 220, 60],
    [240, 170, 40],
    [240, 90, 40],
    [230, 40, 40],
    [140, 140, 140],
];

/// The colour of a morale state.
pub fn morale_colour(state: MoraleState) -> Color32 {
    let [r, g, b] = MORALE_COLOURS[state as usize];
    Color32::from_rgb(r, g, b)
}

/// One own regiment as the strip shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct RegimentCard {
    pub id: RegimentId,
    /// Localised unit name.
    pub unit: String,
    pub soldiers: u32,
    /// Soldiers the regiment spawned with (`RegimentRow.initial`).
    pub initial: u32,
    pub morale_state: MoraleState,
    /// Localised fatigue state (`il.fatigue.*`).
    pub fatigue: String,
    /// Volleys left; `None` for units without a `ranged` block.
    pub volleys: Option<u16>,
    pub engaged: bool,
    pub selected: bool,
    /// Control group the regiment belongs to, if any (the lowest).
    pub group: Option<u8>,
    /// The side's tint (`il_render::side_tint`, handed in by the app).
    pub tint: [u8; 4],
}

pub struct CardStripModel<'a> {
    pub cards: &'a [RegimentCard],
    pub locale: &'a Locale,
}

/// What a card click asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardAction {
    /// Select the regiment; `add` keeps the rest of the selection (Shift).
    Select { id: RegimentId, add: bool },
    /// Centre the camera on the regiment (double click).
    Centre(RegimentId),
}

fn rgba(c: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

/// Draws the strip; returns the clicks, in card order.
pub fn card_strip(ctx: &egui::Context, model: &CardStripModel<'_>) -> Vec<CardAction> {
    let mut actions = Vec::new();
    if model.cards.is_empty() {
        return actions;
    }
    let l = model.locale;
    let max_height = ctx.content_rect().height() * STRIP_HEIGHT_FRACTION;
    let shift = ctx.input(|i| i.modifiers.shift);
    egui::Window::new("il_cards")
        .anchor(egui::Align2::LEFT_TOP, egui::vec2(8.0, 48.0))
        .title_bar(false)
        .resizable(false)
        .default_width(CARD_WIDTH)
        .show(ctx, |ui| {
            ui.set_width(CARD_WIDTH);
            egui::ScrollArea::vertical()
                .max_height(max_height)
                .show(ui, |ui| {
                    for card in model.cards {
                        if let Some(action) = one_card(ui, card, l, shift) {
                            actions.push(action);
                        }
                    }
                });
        });
    actions
}

fn one_card(ui: &mut egui::Ui, card: &RegimentCard, l: &Locale, shift: bool) -> Option<CardAction> {
    let frame = egui::Frame::group(ui.style()).inner_margin(4.0);
    let inner = frame.show(ui, |ui| {
        ui.set_width(CARD_WIDTH - 16.0);
        ui.horizontal(|ui| {
            ui.colored_label(rgba(card.tint), "\u{25a0}");
            ui.strong(&card.unit);
            if let Some(g) = card.group {
                ui.weak(l.fmt("il.cards.group", &[("n", &g)]));
            }
        });
        let fraction = if card.initial > 0 {
            card.soldiers as f32 / card.initial as f32
        } else {
            0.0
        };
        ui.add(
            egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
                .text(l.fmt(
                    "il.cards.strength",
                    &[("soldiers", &card.soldiers), ("initial", &card.initial)],
                ))
                .desired_height(12.0),
        );
        ui.horizontal(|ui| {
            ui.colored_label(morale_colour(card.morale_state), "\u{25cf}");
            ui.label(&card.fatigue);
            if let Some(v) = card.volleys {
                ui.label(l.fmt("il.cards.volleys", &[("count", &v)]));
            }
            if card.engaged {
                ui.colored_label(Color32::from_rgb(255, 120, 80), l.get("il.cards.engaged"));
            }
        });
    });
    let response = inner.response.interact(Sense::click());
    if card.selected {
        ui.painter().rect_stroke(
            response.rect,
            2.0,
            Stroke::new(2.0, Color32::from_rgb(255, 230, 120)),
            StrokeKind::Outside,
        );
    }
    if response.double_clicked() {
        Some(CardAction::Centre(card.id))
    } else if response.clicked() {
        Some(CardAction::Select {
            id: card.id,
            add: shift,
        })
    } else {
        None
    }
}
