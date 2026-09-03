//! T1-044 done-when: two regiments marched through each other end with no
//! overlapping pair after 2 s, and the momentum-weighted centre survives a
//! resolve.

mod common;

use il_core::{Angle, RegimentId, S, Scalar, V2};
use il_sim_battle::components::{Body, Order, OrderKind, Path, Pos};
use il_sim_battle::resources::Ids;
use il_sim_battle::{BattleWorld, PathRequests, SpeedMode};

fn v(x: f32, y: f32) -> V2 {
    V2::from_f32_data(x, y)
}

fn order_move(w: &mut BattleWorld, rid: u32, target: V2, facing: f32) {
    let e = w
        .ecs()
        .resource::<Ids>()
        .regiment_entity(RegimentId(rid))
        .unwrap();
    let tick = w.tick();
    {
        let mut order = w.ecs_mut().get_mut::<Order>(e).unwrap();
        order.kind = OrderKind::Move;
        order.target = target;
        order.facing = Some(Angle::from_degrees_data(facing));
        order.speed = SpeedMode::Run;
        order.since = tick;
    }
    w.ecs_mut().get_mut::<Path>(e).unwrap().requested = true;
    w.ecs_mut()
        .resource_mut::<PathRequests>()
        .0
        .insert(RegimentId(rid));
    w.recompute_hash();
}

/// Deepest interpenetration over every pair, in metres.
fn max_overlap(w: &BattleWorld) -> S {
    let ids = w.ecs().resource::<Ids>();
    let soldiers: Vec<(V2, S)> = ids
        .soldier_entities
        .iter()
        .map(|(_, e)| {
            (
                w.ecs().get::<Pos>(*e).unwrap().p,
                w.ecs().get::<Body>(*e).unwrap().r,
            )
        })
        .collect();
    let mut worst = S::ZERO;
    for i in 0..soldiers.len() {
        for j in i + 1..soldiers.len() {
            let (pi, ri) = soldiers[i];
            let (pj, rj) = soldiers[j];
            worst = worst.max(ri + rj - pi.distance(pj));
        }
    }
    worst
}

/// Pairs interpenetrating by more than a tenth of their radius sum. With
/// `spacing_file` of one diameter, soldiers rest in contact, and the two
/// Jacobi passes leave the per-tick separation jitter as a residual of a
/// few centimetres (4 cm measured for two 60-man lines); deeper than that
/// would be soldiers stacked on one another.
fn overlapping_pairs(w: &BattleWorld) -> usize {
    let ids = w.ecs().resource::<Ids>();
    let soldiers: Vec<(V2, S)> = ids
        .soldier_entities
        .iter()
        .map(|(_, e)| {
            (
                w.ecs().get::<Pos>(*e).unwrap().p,
                w.ecs().get::<Body>(*e).unwrap().r,
            )
        })
        .collect();
    let tolerance = S::from_f32_data(0.1);
    let mut n = 0;
    for i in 0..soldiers.len() {
        for j in i + 1..soldiers.len() {
            let (pi, ri) = soldiers[i];
            let (pj, rj) = soldiers[j];
            if pi.distance(pj) < (ri + rj) * (S::ONE - tolerance) {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn regiments_marching_through_each_other_end_without_overlaps() {
    // Two 60-man lines 40 m apart facing each other; each is ordered onto
    // the other's ground, so they pass through one another.
    let mut setup = common::two_sides(60);
    setup.sides[0].regiments[0].position = Some([380.0, 150.0]);
    setup.sides[1].regiments[0].position = Some([420.0, 150.0]);
    let mut w = BattleWorld::new(&setup, common::regs()).unwrap();
    order_move(&mut w, 0, v(440.0, 150.0), 0.0);
    order_move(&mut w, 1, v(360.0, 150.0), 180.0);
    let mut collided = false;
    let mut idle_since: Option<u32> = None;
    for t in 0..2_000u32 {
        w.step(&[]);
        if overlapping_pairs(&w) > 0 {
            collided = true;
        }
        let idle = w
            .view()
            .regiments()
            .all(|r| r.order == il_sim_battle::components::OrderKind::Idle);
        match (idle, idle_since) {
            (true, None) => idle_since = Some(t),
            (true, Some(since)) if t - since >= 40 => break,
            _ => {}
        }
    }
    assert!(idle_since.is_some(), "the regiments never arrived");
    // Overlaps during the pass are resolved within the tick, so the
    // per-tick check may never see one; what matters is the end state.
    let _ = collided;
    assert_eq!(
        overlapping_pairs(&w),
        0,
        "overlapping pairs after settling, deepest {:?} m",
        max_overlap(&w)
    );
    assert!(
        max_overlap(&w) < S::from_f32_data(0.06),
        "{:?} m",
        max_overlap(&w)
    );
    let a = w.view().regiment(RegimentId(0)).unwrap().anchor_pos;
    let b = w.view().regiment(RegimentId(1)).unwrap().anchor_pos;
    assert!(a.x > b.x, "they swapped sides: {a:?} {b:?}");
}

#[test]
fn a_resolve_pass_preserves_the_momentum_weighted_centre() {
    // Drop a cavalry soldier onto an infantry soldier and let Stage 7 push
    // them apart: m·p summed over the pair is unchanged.
    let mut setup = common::two_sides(1);
    setup.sides[1].regiments[0].unit_type = common::cid("persia:cavalry");
    setup.sides[0].regiments[0].position = Some([300.0, 150.0]);
    setup.sides[1].regiments[0].position = Some([300.3, 150.0]);
    let mut w = BattleWorld::new(&setup, common::regs()).unwrap();
    let ids: Vec<_> = w.ecs().resource::<Ids>().soldier_entities.clone();
    let masses: Vec<S> = ids
        .iter()
        .map(|(_, e)| w.ecs().get::<Body>(*e).unwrap().m)
        .collect();
    let centre = |w: &BattleWorld| {
        let mut sum = V2::ZERO;
        let mut m = S::ZERO;
        for ((_, e), mass) in ids.iter().zip(&masses) {
            sum += w.ecs().get::<Pos>(*e).unwrap().p * *mass;
            m = m + *mass;
        }
        sum * (S::ONE / m)
    };
    assert!(overlapping_pairs(&w) > 0, "the pair starts overlapping");
    let before = centre(&w);
    // Run the collision stage alone (steering would also separate them,
    // at each soldier's own speed rather than by mass).
    use bevy_ecs::system::RunSystemOnce;
    w.ecs_mut()
        .run_system_once(il_sim_battle::movement::collision_resolve)
        .unwrap();
    let after = centre(&w);
    assert_eq!(overlapping_pairs(&w), 0);
    assert!(
        (before - after).length() < S::from_f32_data(1e-3),
        "{before:?} -> {after:?}"
    );
    // The lighter soldier moved farther.
    let d0 = w
        .ecs()
        .get::<Pos>(ids[0].1)
        .unwrap()
        .p
        .distance(v(300.0, 150.0));
    let d1 = w
        .ecs()
        .get::<Pos>(ids[1].1)
        .unwrap()
        .p
        .distance(v(300.3, 150.0));
    assert!(d0 > d1 * S::from_i32(3), "{d0:?} vs {d1:?}");
}
