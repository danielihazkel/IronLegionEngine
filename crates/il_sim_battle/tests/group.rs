//! T1-046 done-when: a battle line of five regiments at 300 m lands within
//! 10 % of 300 m and the regiments keep their lateral order; the other five
//! kinds place every regiment.

mod common;

use il_core::{Angle, RegimentId, S, Scalar, V2};
use il_data::{ContentId, GroupKind, Registries, UnitCategory};
use il_sim_battle::formation::group::{RegimentInfo, arrange_group, arranged_width};

fn v(x: f32, y: f32) -> V2 {
    V2::from_f32_data(x, y)
}

fn infos(regs: &Registries) -> Vec<RegimentInfo> {
    let line = regs
        .formations
        .lookup(&ContentId::new("rome:line").unwrap())
        .unwrap();
    let wedge = regs
        .formations
        .lookup(&ContentId::new("rome:wedge").unwrap())
        .unwrap();
    // Five regiments scattered west to east (out of order by id) with a
    // cavalry regiment in the middle and a skirmisher second from the left.
    vec![
        RegimentInfo {
            id: RegimentId(0),
            pos: v(300.0, 100.0),
            category: UnitCategory::Infantry,
            count: 120,
            template: line,
            radius: S::from_f32_data(0.4),
        },
        RegimentInfo {
            id: RegimentId(1),
            pos: v(100.0, 130.0),
            category: UnitCategory::Infantry,
            count: 160,
            template: line,
            radius: S::from_f32_data(0.4),
        },
        RegimentInfo {
            id: RegimentId(2),
            pos: v(200.0, 90.0),
            category: UnitCategory::Cavalry,
            count: 60,
            template: wedge,
            radius: S::from_f32_data(0.7),
        },
        RegimentInfo {
            id: RegimentId(3),
            pos: v(400.0, 110.0),
            category: UnitCategory::Infantry,
            count: 120,
            template: line,
            radius: S::from_f32_data(0.4),
        },
        RegimentInfo {
            id: RegimentId(4),
            pos: v(150.0, 120.0),
            category: UnitCategory::Skirmisher,
            count: 120,
            template: line,
            radius: S::from_f32_data(0.4),
        },
    ]
}

fn group(regs: &Registries, id: &str) -> il_data::GroupFormationTemplate {
    regs.group_formations
        .get(
            regs.group_formations
                .lookup(&ContentId::new(id).unwrap())
                .unwrap(),
        )
        .clone()
}

#[test]
fn a_battle_line_matches_the_requested_width_and_keeps_lateral_order() {
    let regs = common::regs();
    let mut t = group(&regs, "rome:battle_line");
    t.cavalry_flanks = false;
    t.skirmishers_forward = false;
    let infos = infos(&regs);
    let facing = Angle::from_degrees_data(90.0); // north: right = +x
    let width = S::from_i32(300);
    let placements = arrange_group(
        &t,
        &infos,
        v(400.0, 300.0),
        facing,
        width,
        &regs.rules.formation,
        &regs,
    );
    assert_eq!(placements.len(), 5);
    let right = V2::new(S::ONE, S::ZERO);
    let total = arranged_width(&placements, &infos, &regs, right);
    let tol = width * regs.rules.formation.width_tolerance;
    assert!(
        (total - width).abs() <= tol,
        "line is {total:?} m wide for a {width:?} m request"
    );
    // West-to-east input order 1, 4, 2, 0, 3 is preserved: no crossing.
    let mut by_x: Vec<(S, RegimentId)> = placements.iter().map(|p| (p.anchor.x, p.id)).collect();
    by_x.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let ids: Vec<u32> = by_x.iter().map(|(_, id)| id.0).collect();
    assert_eq!(ids, vec![1, 4, 2, 0, 3]);
    // Same facing everywhere, centred on the anchor, sensible ranks.
    for p in &placements {
        assert_eq!(p.facing, facing);
        assert!(p.ranks >= 1);
        assert!((p.anchor.y - S::from_i32(300)).abs() < S::from_f32_data(1e-3));
    }
    let centre = (by_x[0].0 + by_x[4].0) * S::HALF;
    assert!(
        (centre - S::from_i32(400)).abs() < S::from_i32(30),
        "{centre:?}"
    );
}

#[test]
fn flanks_and_skirmishers_and_the_other_kinds() {
    let regs = common::regs();
    let infos = infos(&regs);
    let facing = Angle::from_degrees_data(90.0);
    let rules = &regs.rules.formation;
    // Cavalry to a flank, skirmishers ahead by skirmish_offset.
    let t = group(&regs, "rome:battle_line");
    let p = arrange_group(
        &t,
        &infos,
        v(400.0, 300.0),
        facing,
        S::from_i32(300),
        rules,
        &regs,
    );
    let cav = p.iter().find(|p| p.id == RegimentId(2)).unwrap();
    let xs: Vec<S> = p.iter().map(|p| p.anchor.x).collect();
    let min_x = xs.iter().fold(S::from_i32(9_999), |a, b| a.min(*b));
    assert_eq!(
        cav.anchor.x, min_x,
        "the single cavalry regiment takes the left flank"
    );
    let skirm = p.iter().find(|p| p.id == RegimentId(4)).unwrap();
    assert!(
        (skirm.anchor.y - S::from_i32(300) - rules.skirmish_offset).abs() < S::from_f32_data(1e-3)
    );

    for (id, kind) in [
        ("rome:double_line", GroupKind::DoubleLine),
        ("rome:echelon_left", GroupKind::EchelonLeft),
        ("rome:echelon_right", GroupKind::EchelonRight),
        ("rome:refused_left", GroupKind::RefusedLeft),
        ("rome:refused_right", GroupKind::RefusedRight),
    ] {
        let mut t = group(&regs, id);
        assert_eq!(t.kind, kind);
        t.cavalry_flanks = false;
        t.skirmishers_forward = false;
        let p = arrange_group(
            &t,
            &infos,
            v(400.0, 300.0),
            facing,
            S::from_i32(200),
            rules,
            &regs,
        );
        assert_eq!(p.len(), 5, "{id}");
        let ids: Vec<u32> = p.iter().map(|p| p.id.0).collect();
        assert_eq!(ids, vec![0, 1, 2, 3, 4], "{id}: ascending id");
        let ys: Vec<S> = p.iter().map(|p| p.anchor.y).collect();
        match kind {
            GroupKind::DoubleLine => {
                let back = ys.iter().filter(|y| **y < S::from_i32(300)).count();
                assert_eq!(back, 2, "{id}: two regiments in the second line");
            }
            GroupKind::EchelonLeft => {
                // Leftmost (id 1) trails the most, rightmost (id 3) leads.
                let y1 = p.iter().find(|p| p.id == RegimentId(1)).unwrap().anchor.y;
                let y3 = p.iter().find(|p| p.id == RegimentId(3)).unwrap().anchor.y;
                assert!(y1 < y3 && y3 == S::from_i32(300), "{id}: {y1:?} {y3:?}");
            }
            GroupKind::EchelonRight => {
                let y1 = p.iter().find(|p| p.id == RegimentId(1)).unwrap().anchor.y;
                let y3 = p.iter().find(|p| p.id == RegimentId(3)).unwrap().anchor.y;
                assert!(y3 < y1 && y1 == S::from_i32(300), "{id}: {y1:?} {y3:?}");
            }
            GroupKind::RefusedLeft => {
                let l = p.iter().find(|p| p.id == RegimentId(1)).unwrap();
                assert!(
                    (l.anchor.y - (S::from_i32(300) - rules.group_gap * S::from_i32(3))).abs()
                        < S::from_f32_data(1e-3)
                );
                assert_eq!(
                    l.facing,
                    Angle::from_degrees_data(45.0),
                    "turned inward (clockwise)"
                );
                assert!(p.iter().filter(|p| p.facing == facing).count() == 4);
            }
            GroupKind::RefusedRight => {
                let r = p.iter().find(|p| p.id == RegimentId(3)).unwrap();
                assert_eq!(
                    r.facing,
                    Angle::from_degrees_data(135.0),
                    "turned inward (counter-clockwise)"
                );
            }
            _ => unreachable!(),
        }
    }
}
