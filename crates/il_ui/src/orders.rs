//! Orders (T1-062, REQ-INP-003, REQ-INP-006, TDD §11): the drag-formation
//! gesture's geometry and the conversion of every `UiIntent` into
//! `CommandKind`s. The app stamps them with the tick and a per-player
//! `seq` (`BattleSession::queue`), so a drag on one regiment yields
//! `SetFormation` then `Move` with consecutive sequence numbers.
//!
//! Geometry is in world metres on the ground plane; the app unprojects the
//! screen points first. Forward is the perpendicular of the drag segment
//! that points away from the selection's centroid (plan decision 16), so
//! dragging a line in front of the troops faces them toward it; `flip`
//! (the `order_flip_facing` modifier) turns them the other way.

use std::collections::{BTreeMap, BTreeSet};

use glam::Vec2;
use il_core::{Angle, RegimentId, S, Scalar, V2};
use il_data::{ContentId, GroupKind};
use il_sim_battle::{BattleView, CommandKind, SpeedMode, ranks_for_width};

/// Drags shorter than this (metres) are clicks in disguise; no order.
pub const MIN_DRAG_WIDTH_M: f32 = 1.0;

/// A drag-formation gesture resolved to world geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DragFormation {
    /// Midpoint of the drag: the line's centre.
    pub anchor: Vec2,
    /// Unit vector the regiments will face.
    pub forward: Vec2,
    /// Length of the drag: the line's requested width in metres.
    pub width: f32,
}

impl DragFormation {
    pub fn facing(&self) -> Angle<S> {
        Angle::from_direction(V2::from_f32_data(self.forward.x, self.forward.y))
    }
}

/// Resolves a drag from `from` to `to` (world metres) for a selection
/// centred on `centroid`. `None` when the drag is too short to mean a line.
pub fn drag_formation(from: Vec2, to: Vec2, centroid: Vec2, flip: bool) -> Option<DragFormation> {
    let d = to - from;
    let width = d.length();
    if width.is_nan() || width < MIN_DRAG_WIDTH_M {
        return None;
    }
    let right = d / width;
    let anchor = (from + to) * 0.5;
    let mut forward = Vec2::new(-right.y, right.x);
    if forward.dot(anchor - centroid) < 0.0 {
        forward = -forward;
    }
    if flip {
        forward = -forward;
    }
    Some(DragFormation {
        anchor,
        forward,
        width,
    })
}

/// What the player asked for; see [`commands_for`] for what the sim gets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UiIntent {
    /// Right click: walk or run there, keep the current facing rule.
    Move {
        target: Vec2,
    },
    /// Right drag: a line of the given width facing `forward`.
    DragFormation(DragFormation),
    Halt,
    /// The unit type's n-th formation template (1-based); regiments whose
    /// type has no n-th template are skipped.
    Formation(u8),
    /// The run toggle, applied to the selection's current orders too.
    SpeedMode(SpeedMode),
}

/// What conversion needs besides the intent.
pub struct OrderContext<'a, 'w> {
    pub view: &'a BattleView<'w>,
    /// The selection, ascending (a `BTreeSet` keeps the order stable).
    pub regiments: &'a BTreeSet<RegimentId>,
    /// The speed mode new movement orders carry (the run toggle).
    pub speed: SpeedMode,
}

fn v2(p: Vec2) -> V2 {
    V2::from_f32_data(p.x, p.y)
}

/// Centroid of the selection's anchors, for the gesture's facing rule.
pub fn selection_centroid(view: &BattleView, regiments: &BTreeSet<RegimentId>) -> Option<Vec2> {
    let mut sum = Vec2::ZERO;
    let mut n = 0.0;
    for id in regiments {
        if let Some(r) = view.regiment(*id) {
            sum += Vec2::new(
                r.anchor_pos.x.to_f32_render(),
                r.anchor_pos.y.to_f32_render(),
            );
            n += 1.0;
        }
    }
    (n > 0.0).then(|| sum / n)
}

/// The first `battle_line` group template in the registries.
pub fn battle_line_template(view: &BattleView) -> Option<ContentId> {
    view.regs()
        .group_formations
        .iter()
        .find(|(_, t)| t.kind == GroupKind::BattleLine)
        .map(|(_, t)| t.id.clone())
}

/// `UiIntent → CommandKind`s, in the order they must be queued. Empty when
/// nothing is selected or the intent cannot apply (no battle-line template
/// in the content, a drag on a regiment that vanished).
pub fn commands_for(intent: &UiIntent, ctx: &OrderContext<'_, '_>) -> Vec<CommandKind> {
    let regiments: Vec<RegimentId> = ctx.regiments.iter().copied().collect();
    if regiments.is_empty() {
        return Vec::new();
    }
    match intent {
        UiIntent::Move { target } => vec![CommandKind::Move {
            regiments,
            target: v2(*target),
            facing: None,
            speed: ctx.speed,
        }],
        UiIntent::Halt => vec![CommandKind::Halt { regiments }],
        UiIntent::SpeedMode(mode) => vec![CommandKind::SetSpeedMode {
            regiments,
            mode: *mode,
        }],
        UiIntent::Formation(n) => formation_commands(ctx, &regiments, *n),
        UiIntent::DragFormation(drag) => drag_commands(ctx, regiments, drag),
    }
}

fn formation_commands(
    ctx: &OrderContext<'_, '_>,
    regiments: &[RegimentId],
    n: u8,
) -> Vec<CommandKind> {
    let Some(index) = usize::from(n).checked_sub(1) else {
        return Vec::new();
    };
    let units = &ctx.view.regs().units;
    // One SetFormation per template, templates in ContentId order.
    let mut by_template: BTreeMap<ContentId, Vec<RegimentId>> = BTreeMap::new();
    for id in regiments {
        let Some(r) = ctx.view.regiment(*id) else {
            continue;
        };
        if let Some(template) = units.get(r.unit).formation_ids.get(index) {
            by_template.entry(template.clone()).or_default().push(*id);
        }
    }
    by_template
        .into_iter()
        .map(|(template, regiments)| CommandKind::SetFormation {
            regiments,
            template,
            ranks: None,
        })
        .collect()
}

fn drag_commands(
    ctx: &OrderContext<'_, '_>,
    regiments: Vec<RegimentId>,
    drag: &DragFormation,
) -> Vec<CommandKind> {
    let facing = drag.facing();
    let target = v2(drag.anchor);
    if let [id] = regiments[..] {
        // SIM-FORM-042 with n = 1: pick the ranks that fit the drag width,
        // then move there facing the drag.
        let Some(r) = ctx.view.regiment(id) else {
            return Vec::new();
        };
        let regs = ctx.view.regs();
        let template = regs.formations.get(r.formation);
        let radius = regs.units.get(r.unit).soldier_radius;
        let count = u16::try_from(r.soldier_count).unwrap_or(u16::MAX);
        let ranks = ranks_for_width(
            template,
            count,
            radius,
            S::from_f32_data(drag.width),
            regs.rules.formation.width_tolerance,
        );
        return vec![
            CommandKind::SetFormation {
                regiments: vec![id],
                template: template.id.clone(),
                ranks: Some(ranks),
            },
            CommandKind::Move {
                regiments: vec![id],
                target,
                facing: Some(facing),
                speed: ctx.speed,
            },
        ];
    }
    let Some(template) = battle_line_template(ctx.view) else {
        return Vec::new();
    };
    // GroupFormation moves each regiment at its current order speed, so the
    // speed mode goes first.
    vec![
        CommandKind::SetSpeedMode {
            regiments: regiments.clone(),
            mode: ctx.speed,
        },
        CommandKind::GroupFormation {
            regiments,
            template,
            anchor: target,
            facing,
            width: S::from_f32_data(drag.width),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-4
    }

    #[test]
    fn forward_points_away_from_the_selection_and_flip_reverses_it() {
        // Troops at the origin, a line dragged 100 m north of them, left to right.
        let from = Vec2::new(-50.0, 100.0);
        let to = Vec2::new(50.0, 100.0);
        let d = drag_formation(from, to, Vec2::ZERO, false).unwrap();
        assert!(close(d.anchor, Vec2::new(0.0, 100.0)));
        assert!(close(d.forward, Vec2::new(0.0, 1.0)), "{:?}", d.forward);
        assert!((d.width - 100.0).abs() < 1e-4);
        // Dragging right to left gives the same line and the same facing.
        let r = drag_formation(to, from, Vec2::ZERO, false).unwrap();
        assert!(close(r.anchor, d.anchor) && close(r.forward, d.forward));
        // Alt turns them to face the troops' old position.
        let f = drag_formation(from, to, Vec2::ZERO, true).unwrap();
        assert!(close(f.forward, Vec2::new(0.0, -1.0)));
        // A line dragged south of the troops faces south.
        let s = drag_formation(
            Vec2::new(-50.0, -100.0),
            Vec2::new(50.0, -100.0),
            Vec2::ZERO,
            false,
        )
        .unwrap();
        assert!(close(s.forward, Vec2::new(0.0, -1.0)));
    }

    #[test]
    fn diagonal_drags_face_perpendicular_and_facing_is_the_forward_angle() {
        let d = drag_formation(
            Vec2::new(0.0, 0.0),
            Vec2::new(30.0, 40.0),
            Vec2::new(100.0, 0.0),
            false,
        )
        .unwrap();
        assert!((d.width - 50.0).abs() < 1e-4);
        assert!(
            d.forward.dot(Vec2::new(0.6, 0.8)).abs() < 1e-5,
            "perpendicular to the drag"
        );
        assert!(
            d.forward.dot(Vec2::new(100.0, 0.0) - d.anchor) < 0.0,
            "away from the centroid"
        );
        let f = d.facing().direction();
        assert!((f.x.to_f32_render() - d.forward.x).abs() < 1e-4);
        assert!((f.y.to_f32_render() - d.forward.y).abs() < 1e-4);
    }

    #[test]
    fn a_drag_under_a_metre_is_not_a_formation() {
        assert!(drag_formation(Vec2::ZERO, Vec2::new(0.5, 0.0), Vec2::ZERO, false).is_none());
        assert!(drag_formation(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, false).is_none());
    }

    #[test]
    fn a_line_through_the_centroid_still_has_a_facing() {
        let d = drag_formation(
            Vec2::new(-10.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::ZERO,
            false,
        )
        .unwrap();
        assert!(close(d.forward, Vec2::new(0.0, 1.0)));
    }
}
