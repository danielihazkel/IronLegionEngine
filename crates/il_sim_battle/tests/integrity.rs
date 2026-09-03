//! T1-045: integrity reads 1.0 for a settled regiment, drops during a
//! wheel and recovers; a large facing order about-faces instead.

mod common;

use il_core::{Angle, RegimentId, S, Scalar};
use il_sim_battle::BattleWorld;
use il_sim_battle::components::{Anchor, FormationState, Order};
use il_sim_battle::formation::{set_facing, spacing};
use il_sim_battle::resources::Ids;

fn entity(w: &BattleWorld, rid: u32) -> bevy_ecs::entity::Entity {
    w.ecs()
        .resource::<Ids>()
        .regiment_entity(RegimentId(rid))
        .unwrap()
}

fn integrity(w: &BattleWorld, rid: u32) -> S {
    w.view().regiment(RegimentId(rid)).unwrap().integrity
}

/// What Stage 0 will do for `SetFacing` from T1-047 on.
fn order_facing(w: &mut BattleWorld, rid: u32, deg: f32) -> bool {
    let e = entity(w, rid);
    let regs = w.registries().clone();
    let (unit, template) = {
        let r = w
            .ecs()
            .get::<il_sim_battle::components::Regiment>(e)
            .unwrap();
        let s = w.ecs().get::<FormationState>(e).unwrap();
        (r.unit, s.template)
    };
    let (_, sr) = spacing(
        regs.formations.get(template),
        regs.units.get(unit).soldier_radius,
    );
    let mut anchor = *w.ecs().get::<Anchor>(e).unwrap();
    let mut order = *w.ecs().get::<Order>(e).unwrap();
    let mut state = w.ecs().get::<FormationState>(e).unwrap().clone();
    let about = set_facing(
        &mut anchor,
        &mut order,
        &mut state,
        &regs.rules.formation,
        sr,
        Angle::from_degrees_data(deg),
    );
    *w.ecs_mut().get_mut::<Anchor>(e).unwrap() = anchor;
    *w.ecs_mut().get_mut::<Order>(e).unwrap() = order;
    *w.ecs_mut().get_mut::<FormationState>(e).unwrap() = state;
    w.recompute_hash();
    about
}

#[test]
fn settled_regiments_read_one_and_a_wheel_dips_then_recovers() {
    let mut w = common::world(120);
    for _ in 0..20 {
        w.step(&[]);
    }
    assert_eq!(integrity(&w, 0), S::ONE);
    assert_eq!(integrity(&w, 1), S::ONE);

    // Regiment 0 faces +x; wheel 90 degrees (under turn_in_place_angle).
    assert!(!order_facing(&mut w, 0, 90.0), "a 90 degree turn wheels");
    let mut lowest = S::ONE;
    for _ in 0..80 {
        w.step(&[]);
        lowest = lowest.min(integrity(&w, 0));
    }
    assert!(
        lowest < S::from_f32_data(0.9),
        "integrity never dipped: {lowest:?}"
    );
    let e = entity(&w, 0);
    assert_eq!(
        w.ecs().get::<Anchor>(e).unwrap().facing,
        Angle::from_degrees_data(90.0),
        "the wheel completed"
    );
    for _ in 0..400 {
        w.step(&[]);
    }
    assert!(
        integrity(&w, 0) >= S::from_f32_data(0.95),
        "{:?}",
        integrity(&w, 0)
    );
    assert_eq!(integrity(&w, 1), S::ONE, "the other regiment never moved");
}

#[test]
fn a_large_turn_about_faces_onto_the_rear_rank() {
    let mut w = common::world(60);
    let e = entity(&w, 1);
    let before = *w.ecs().get::<Anchor>(e).unwrap();
    // Regiment 1 faces 180 degrees; ordering 0 degrees is a full reversal.
    assert!(order_facing(&mut w, 1, 0.0));
    let after = *w.ecs().get::<Anchor>(e).unwrap();
    assert_eq!(after.facing, Angle::from_degrees_data(0.0));
    // Four ranks at 0.96 m: the anchor moved 2.88 m to the rear rank, which
    // for a regiment facing -x lies at +x.
    let moved = after.pos - before.pos;
    assert!(
        (moved.x - S::from_f32_data(2.88)).abs() < S::from_f32_data(1e-3),
        "{moved:?}"
    );
    assert!(moved.y.abs() < S::from_f32_data(1e-3));
    assert!(w.ecs().get::<FormationState>(e).unwrap().needs_reform);
    for _ in 0..300 {
        w.step(&[]);
    }
    assert!(
        integrity(&w, 1) >= S::from_f32_data(0.95),
        "{:?}",
        integrity(&w, 1)
    );
    assert_eq!(
        w.ecs().get::<Anchor>(e).unwrap().facing,
        Angle::from_degrees_data(0.0)
    );
}
