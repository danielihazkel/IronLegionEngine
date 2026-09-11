//! T3-043 (REQ-FORM-009, REQ-FORM-010; SIM-FORM-040..042): group
//! formations driven end to end. Five scattered regiments are sent through
//! the player's `GroupFormation` command for every group kind and land on
//! the arrangement's own placements with its geometry (gaps, echelon steps,
//! the refused flank's angle, the double line's spacing, no crossing); an
//! engine-owned side deploys the same five through the AI's deployment
//! preset (SIM-AI-020) with the same gap and order properties.

mod common;

use il_core::{Angle, PlayerId, RegimentId, S, Scalar, V2};
use il_data::{GroupKind, UnitCategory};
use il_sim_battle::components::{Anchor, FormationState, Order, OrderKind};
use il_sim_battle::formation::{
    Placement, RegimentInfo, arrange_group, effective_ranks, lateral_order,
};
use il_sim_battle::map::polygon_contains;
use il_sim_battle::movement::formation_width;
use il_sim_battle::resources::Ids;
use il_sim_battle::{
    BattleEvent, BattlePhase, BattleSetup, BattleWorld, Command, CommandKind, GeneralSetup,
    RegimentSetup,
};

fn v(x: f32, y: f32) -> V2 {
    V2::from_f32_data(x, y)
}

fn sf(x: f32) -> S {
    S::from_f32_data(x)
}

const EPS: f32 = 1e-3;

fn placed(id: u32, unit: &str, count: u16, x: f32, y: f32, facing_deg: f32) -> RegimentSetup {
    RegimentSetup {
        position: Some([x, y]),
        facing_deg: Some(facing_deg),
        ..common::regiment(id, unit, count, x, facing_deg)
    }
}

fn unplaced(id: u32, unit: &str, count: u16) -> RegimentSetup {
    RegimentSetup {
        position: None,
        facing_deg: None,
        ..common::regiment(id, unit, count, 0.0, 0.0)
    }
}

/// Five regiments scattered west to east out of id order, a cavalry
/// regiment in the middle and a skirmisher second from the left (the
/// `tests/group.rs` set), as regiment ids 0..5 of side 0.
fn five(place: bool) -> Vec<RegimentSetup> {
    let mk = |id: u32, unit: &str, count: u16, x: f32, y: f32| {
        if place {
            placed(id, unit, count, x, y, 90.0)
        } else {
            unplaced(id, unit, count)
        }
    };
    vec![
        mk(1, "rome:hastati", 120, 300.0, 120.0),
        mk(2, "rome:hastati", 160, 100.0, 150.0),
        mk(3, "persia:cavalry", 60, 200.0, 110.0),
        mk(4, "rome:hastati", 120, 400.0, 130.0),
        mk(5, "rome:velites", 120, 150.0, 140.0),
    ]
}

fn setup(side0_player: u8, side0: Vec<RegimentSetup>, bodyguard: Option<u32>) -> BattleSetup {
    let mut s = common::two_sides(20);
    s.sides[0].player = PlayerId(side0_player);
    s.sides[0].regiments = side0;
    s.sides[0].general = GeneralSetup {
        unit_type: common::cid("rome:general"),
        rank: 1,
        name_key: String::new(),
        bodyguard,
    };
    s.sides[1].regiments = vec![placed(9, "rome:hastati", 20, 300.0, 480.0, 270.0)];
    s.sides[1].deployment_zone = 1;
    s
}

fn entity(w: &BattleWorld, rid: u32) -> bevy_ecs::entity::Entity {
    w.ecs()
        .resource::<Ids>()
        .regiment_entity(RegimentId(rid))
        .unwrap()
}

fn anchor(w: &BattleWorld, rid: u32) -> Anchor {
    *w.ecs().get::<Anchor>(entity(w, rid)).unwrap()
}

fn ranks_of(w: &BattleWorld, rid: u32) -> u8 {
    w.ecs().get::<FormationState>(entity(w, rid)).unwrap().ranks
}

fn all_idle(w: &BattleWorld, ids: &[u32]) -> bool {
    ids.iter()
        .all(|&rid| w.ecs().get::<Order>(entity(w, rid)).unwrap().kind == OrderKind::Idle)
}

/// The regiments as `arrange_group` sees them, from the world's rows.
fn infos(w: &BattleWorld, ids: &[u32]) -> Vec<RegimentInfo> {
    let view = w.view();
    let regs = view.regs();
    ids.iter()
        .map(|&rid| {
            let r = view.regiment(RegimentId(rid)).unwrap();
            let unit = regs.units.get(r.unit);
            RegimentInfo {
                id: r.id,
                pos: r.anchor_pos,
                category: unit.category,
                count: u16::try_from(r.soldier_count).unwrap(),
                template: r.formation,
                radius: unit.soldier_radius,
            }
        })
        .collect()
}

/// The regiment's current width in metres (`files × sf`).
fn width_now(w: &BattleWorld, rid: u32) -> S {
    let view = w.view();
    let regs = view.regs();
    let r = view.regiment(RegimentId(rid)).unwrap();
    formation_width(
        regs.formations.get(r.formation),
        r.files,
        regs.units.get(r.unit).soldier_radius,
    )
}

/// Regiment ids from left to right along `right`.
fn order_along(w: &BattleWorld, ids: &[u32], right: V2) -> Vec<u32> {
    let mut v: Vec<(S, u32)> = ids
        .iter()
        .map(|&rid| (anchor(w, rid).pos.dot(right), rid))
        .collect();
    v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap().then(a.1.cmp(&b.1)));
    v.into_iter().map(|(_, id)| id).collect()
}

/// SIM-FORM-041: consecutive regiments along the line are at least `gap`
/// apart edge to edge (widths as laid out now).
fn assert_gaps(w: &BattleWorld, ordered: &[u32], right: V2, gap: S, what: &str) {
    for pair in ordered.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let edge_a = anchor(w, a).pos.dot(right) + width_now(w, a) * S::HALF;
        let edge_b = anchor(w, b).pos.dot(right) - width_now(w, b) * S::HALF;
        assert!(
            edge_b - edge_a >= gap - sf(0.05),
            "{what}: regiments {a} and {b} are {:?} m apart edge to edge, gap {gap:?}",
            edge_b - edge_a
        );
    }
}

fn frame(facing: Angle<S>) -> (V2, V2) {
    let forward = facing.direction();
    (V2::new(forward.y, -forward.x), forward)
}

// ------------------------------------------------------- the player's command

#[test]
fn every_group_kind_lands_on_its_placements_with_the_rule_geometry() {
    let kinds = [
        ("rome:battle_line", GroupKind::BattleLine),
        ("rome:double_line", GroupKind::DoubleLine),
        ("rome:echelon_left", GroupKind::EchelonLeft),
        ("rome:echelon_right", GroupKind::EchelonRight),
        ("rome:refused_left", GroupKind::RefusedLeft),
        ("rome:refused_right", GroupKind::RefusedRight),
    ];
    let ids: Vec<u32> = (0..5).collect();
    let facing = Angle::from_degrees_data(90.0); // north: right = +x
    let (right, forward) = frame(facing);
    let group_anchor = v(300.0, 240.0);
    let width = S::from_i32(300);
    for (tid, kind) in kinds {
        let s = setup(0, five(true), None);
        let mut w = BattleWorld::new(&s, common::regs()).unwrap();
        w.set_threads(8);
        assert_eq!(w.phase(), BattlePhase::Battle);
        let regs = w.registries().clone();
        let template = regs
            .group_formations
            .get(regs.group_formations.lookup(&common::cid(tid)).unwrap())
            .clone();
        assert_eq!(template.kind, kind);
        let rules = regs.rules.formation.clone();
        let gap = if template.gap > S::ZERO {
            template.gap
        } else {
            rules.group_gap
        };
        let before = infos(&w, &ids);
        let expected: Vec<Placement> = arrange_group(
            &template,
            &before,
            group_anchor,
            facing,
            width,
            &rules,
            &regs,
        );
        assert_eq!(expected.len(), 5, "{tid}");
        let expected_order: Vec<u32> = lateral_order(&before, right, template.cavalry_flanks)
            .into_iter()
            .map(|i| before[i].id.0)
            .collect();

        let out = w.step(&[Command {
            tick: w.tick().next(),
            player: PlayerId(0),
            seq: 0,
            kind: CommandKind::GroupFormation {
                regiments: ids.iter().map(|&i| RegimentId(i)).collect(),
                template: common::cid(tid),
                anchor: group_anchor,
                facing,
                width,
            },
        }]);
        assert!(out.rejected.is_empty(), "{tid}: {:?}", out.rejected);
        // The command set every regiment's ranks from its placement.
        for p in &expected {
            let t = regs
                .formations
                .get(w.view().regiment(p.id).unwrap().formation);
            let count = before.iter().find(|i| i.id == p.id).unwrap().count;
            assert_eq!(
                ranks_of(&w, p.id.0),
                effective_ranks(t, count, Some(p.ranks)),
                "{tid}: ranks of regiment {}",
                p.id.0
            );
        }
        let mut ticks = 1;
        while !all_idle(&w, &ids) && ticks < 8_000 {
            w.step(&[]);
            ticks += 1;
        }
        assert!(
            all_idle(&w, &ids),
            "{tid}: not everyone arrived in {ticks} ticks"
        );

        // Landed on the arrangement's placements with their facings.
        let radius = regs.rules.movement.waypoint_radius;
        for p in &expected {
            let a = anchor(&w, p.id.0);
            assert!(
                a.pos.distance(p.anchor) <= radius + sf(EPS),
                "{tid}: regiment {} at {:?}, placed at {:?}",
                p.id.0,
                a.pos,
                p.anchor
            );
            assert_eq!(a.facing, p.facing, "{tid}: facing of regiment {}", p.id.0);
        }
        // SIM-FORM-040: no crossing, the lateral order is the arrangement's.
        let final_order = order_along(&w, &ids, right);
        assert_eq!(final_order, expected_order, "{tid}: lateral order");

        // SIM-FORM-041/042 geometry on the placements themselves (exact) and
        // the gaps on the landed regiments (their widths as laid out now).
        let ahead = |p: &Placement| p.anchor.dot(forward);
        let base = group_anchor.dot(forward);
        let by_lateral: Vec<&Placement> = expected_order
            .iter()
            .map(|id| expected.iter().find(|p| p.id.0 == *id).unwrap())
            .collect();
        let skirmisher = |p: &Placement| {
            matches!(
                before.iter().find(|i| i.id == p.id).unwrap().category,
                UnitCategory::Skirmisher | UnitCategory::Ranged
            )
        };
        let forward_offset = |p: &Placement| {
            if template.skirmishers_forward && skirmisher(p) {
                rules.skirmish_offset
            } else {
                S::ZERO
            }
        };
        let near = |a: S, b: S| (a - b).abs() < sf(EPS);
        match kind {
            GroupKind::BattleLine => {
                assert_gaps(&w, &final_order, right, gap, tid);
                for p in &expected {
                    assert!(near(ahead(p), base + forward_offset(p)), "{tid}: {p:?}");
                }
                let total: S = by_lateral
                    .iter()
                    .map(|p| width_now(&w, p.id.0))
                    .fold(S::ZERO, |a, b| a + b)
                    + gap * S::from_i32(4);
                assert!(
                    (total - width).abs() <= width * rules.width_tolerance + sf(0.5),
                    "{tid}: line width {total:?} for a {width:?} m request"
                );
            }
            GroupKind::DoubleLine => {
                // Alternate regiments into two lines 2 × gap apart.
                let two = gap + gap;
                for (k, p) in by_lateral.iter().enumerate() {
                    let line_back = if k % 2 == 0 { S::ZERO } else { two };
                    assert!(
                        near(ahead(p), base - line_back + forward_offset(p)),
                        "{tid}: regiment {} in position {k}: {:?}",
                        p.id.0,
                        ahead(p) - base
                    );
                }
            }
            GroupKind::EchelonLeft | GroupKind::EchelonRight => {
                // Each successive regiment toward the named flank steps
                // 2 × gap back; the far flank leads on the anchor line.
                assert_gaps(&w, &final_order, right, gap, tid);
                let n = by_lateral.len();
                for (k, p) in by_lateral.iter().enumerate() {
                    let steps = if kind == GroupKind::EchelonLeft {
                        n - 1 - k
                    } else {
                        k
                    };
                    let back = (gap + gap) * S::from_i32(steps as i32);
                    assert!(
                        near(ahead(p), base - back + forward_offset(p)),
                        "{tid}: regiment {} in position {k}: {:?}",
                        p.id.0,
                        ahead(p) - base
                    );
                }
            }
            GroupKind::RefusedLeft | GroupKind::RefusedRight => {
                // The flank-most regiment on the named side is 3 × gap back
                // and turned 45° inward; the rest stand on the line.
                assert_gaps(&w, &final_order, right, gap, tid);
                let n = by_lateral.len();
                let refused = if kind == GroupKind::RefusedLeft {
                    0
                } else {
                    n - 1
                };
                let turn = if kind == GroupKind::RefusedLeft {
                    -45.0
                } else {
                    45.0
                };
                for (k, p) in by_lateral.iter().enumerate() {
                    if k == refused {
                        assert!(
                            near(ahead(p), base - gap * S::from_i32(3) + forward_offset(p)),
                            "{tid}: refused regiment {}: {:?}",
                            p.id.0,
                            ahead(p) - base
                        );
                        assert_eq!(
                            p.facing,
                            Angle::from_degrees_data(90.0 + turn),
                            "{tid}: refused flank turned inward"
                        );
                    } else {
                        assert!(near(ahead(p), base + forward_offset(p)), "{tid}: {p:?}");
                        assert_eq!(p.facing, facing);
                    }
                }
            }
            GroupKind::Custom => unreachable!(),
        }
    }
}

// ------------------------------------------------------ the AI's deployment

#[test]
fn the_ai_deploys_the_five_in_a_battle_line_with_gaps_and_order() {
    // The engine owns side 0: five unplaced regiments plus a 20-man
    // bodyguard (id 5) that the preset keeps behind the centre. Side 1 is
    // placed, so the battle opens in Deployment and the AI's Stage 1
    // commands of tick 1 apply at tick 2.
    let mut regiments = five(false);
    regiments.push(unplaced(6, "rome:hastati", 20));
    let s = setup(255, regiments, Some(6));
    let mut w = BattleWorld::new(&s, common::regs()).unwrap();
    w.set_threads(8);
    assert_eq!(w.phase(), BattlePhase::Deployment);
    let ids: Vec<u32> = (0..5).collect();
    let regs = w.registries().clone();
    let rules = regs.rules.formation.clone();
    // The auto-placement the AI reads its lateral order from.
    let before = infos(&w, &ids);
    let mut confirmed = false;
    for _ in 0..3 {
        let out = w.step(&[]);
        assert!(out.rejected.is_empty(), "{:?}", out.rejected);
        confirmed |= out
            .events
            .iter()
            .any(|e| matches!(e, BattleEvent::DeploymentConfirmed { side: 0 }));
    }
    assert!(confirmed);
    assert_eq!(w.phase(), BattlePhase::Battle);

    let map = w.map().clone();
    let poly = map.deployment_polygon(0).unwrap();
    let facing = anchor(&w, 0).facing;
    let (right, forward) = frame(facing);
    assert!(
        forward.y > S::ZERO,
        "faces the enemy zone to the north: {forward:?}"
    );
    for &rid in &ids {
        assert!(
            polygon_contains(poly, anchor(&w, rid).pos),
            "{rid} outside its zone"
        );
        assert_eq!(anchor(&w, rid).facing, facing, "{rid} faces with the line");
    }
    // SIM-FORM-040/041 through the AI: the lateral order is the
    // arrangement's (cavalry to a flank), the gaps hold, the skirmishers
    // stand `skirmish_offset` ahead, the bodyguard behind the centre.
    let expected_order: Vec<u32> = lateral_order(&before, right, true)
        .into_iter()
        .map(|i| before[i].id.0)
        .collect();
    let final_order = order_along(&w, &ids, right);
    assert_eq!(final_order, expected_order, "lateral order");
    assert_gaps(&w, &final_order, right, rules.group_gap, "ai deployment");
    let cavalry = before
        .iter()
        .find(|i| i.category == UnitCategory::Cavalry)
        .unwrap()
        .id
        .0;
    assert!(
        final_order[0] == cavalry || final_order[4] == cavalry,
        "cavalry on a flank: {final_order:?}"
    );
    let ahead = |rid: u32| anchor(&w, rid).pos.dot(forward);
    let line = ahead(0);
    for &rid in &ids {
        let category = before.iter().find(|i| i.id.0 == rid).unwrap().category;
        let expected = if matches!(category, UnitCategory::Skirmisher | UnitCategory::Ranged) {
            line + rules.skirmish_offset
        } else {
            line
        };
        assert!(
            (ahead(rid) - expected).abs() < sf(EPS),
            "regiment {rid} ({category:?}) is {:?} m ahead of the line",
            ahead(rid) - line
        );
    }
    assert!(
        ahead(5) < line - S::from_i32(10),
        "the bodyguard behind the line"
    );
}
