//! T1-042: a regiment ordered across the test map arrives within
//! `waypoint_radius` facing the ordered direction, wheels at `wheel_rate`,
//! and morphs to a column through the 8 m bridge and back.

mod common;

use il_core::{Angle, RegimentId, S, Scalar, V2};
use il_data::Layout;
use il_sim_battle::components::{Anchor, Order, OrderKind, Path};
use il_sim_battle::resources::Ids;
use il_sim_battle::{BattleWorld, PathRequests, SpeedMode};

fn v(x: f32, y: f32) -> V2 {
    V2::from_f32_data(x, y)
}

fn entity(w: &BattleWorld, rid: u32) -> bevy_ecs::entity::Entity {
    w.ecs()
        .resource::<Ids>()
        .regiment_entity(RegimentId(rid))
        .unwrap()
}

/// What Stage 0 will do for a `Move` command from T1-047 on.
fn order_move(w: &mut BattleWorld, rid: u32, target: V2, facing: Option<f32>, speed: SpeedMode) {
    let e = entity(w, rid);
    let tick = w.tick();
    {
        let mut order = w.ecs_mut().get_mut::<Order>(e).unwrap();
        order.kind = OrderKind::Move;
        order.target = target;
        order.facing = facing.map(Angle::from_degrees_data);
        order.speed = speed;
        order.since = tick;
    }
    w.ecs_mut().get_mut::<Path>(e).unwrap().requested = true;
    w.ecs_mut()
        .resource_mut::<PathRequests>()
        .0
        .insert(RegimentId(rid));
    w.recompute_hash();
}

fn anchor(w: &BattleWorld, rid: u32) -> Anchor {
    *w.ecs().get::<Anchor>(entity(w, rid)).unwrap()
}

fn layout_of(w: &BattleWorld, rid: u32) -> Layout {
    let state = w.view().formation_state(RegimentId(rid)).unwrap();
    w.registries().formations.get(state.template).layout
}

fn run_until_idle(w: &mut BattleWorld, rid: u32, max_ticks: u32) -> (u32, bool) {
    let e = entity(w, rid);
    let mut seen_column = false;
    for t in 0..max_ticks {
        w.step(&[]);
        if layout_of(w, rid) == Layout::Column {
            seen_column = true;
        }
        if w.ecs().get::<Order>(e).unwrap().kind == OrderKind::Idle {
            return (t + 1, seen_column);
        }
    }
    (max_ticks, seen_column)
}

#[test]
fn a_regiment_arrives_and_takes_the_ordered_facing() {
    let mut w = common::world(20);
    let rules = w.registries().rules.movement.clone();
    // Regiment 0 (at 300,150 facing +x) marches 100 m north-east; no
    // crossing needed.
    let target = v(380.0, 230.0);
    order_move(&mut w, 0, target, Some(45.0), SpeedMode::Run);
    let start = anchor(&w, 0);
    let (ticks, _) = run_until_idle(&mut w, 0, 4_000);
    let end = anchor(&w, 0);
    assert!(ticks < 4_000, "never arrived");
    assert!(
        end.pos.distance(target) <= rules.waypoint_radius,
        "arrived at {:?}, {ticks} ticks",
        end.pos
    );
    assert_eq!(end.facing, Angle::from_degrees_data(45.0));
    assert!(
        w.view()
            .formation_state(RegimentId(0))
            .unwrap()
            .needs_reform
            || ticks > 0
    );
    // Speed sanity: ran at most speed_run (4 m/s) and the wheel took time;
    // stragglers (soldiers do not move until T1-043) halve the pace.
    let dist = start.pos.distance(target).to_f32_render();
    let seconds = ticks as f32 * 0.05;
    assert!(
        seconds >= dist / 4.0,
        "{seconds}s for {dist}m is faster than a run"
    );
    assert!(
        seconds <= dist / 0.8 + 5.0,
        "{seconds}s for {dist}m is too slow"
    );
}

#[test]
fn wheeling_is_rate_limited() {
    let mut w = common::world(20);
    let rules = w.registries().rules.movement.clone();
    // Target straight behind (west): the anchor must wheel 180 degrees.
    order_move(&mut w, 1, v(700.0, 150.0), None, SpeedMode::Walk);
    let per_tick = rules.wheel_rate * S::PI / S::from_i32(180) / S::from_i32(20);
    let mut prev = anchor(&w, 1).facing;
    let mut total = S::ZERO;
    // The path is served and followed within the first step.
    for _ in 0..10 {
        w.step(&[]);
        let now = anchor(&w, 1).facing;
        let turned = prev.delta(now).abs();
        assert!(
            turned <= per_tick + S::from_f32_data(1e-4),
            "{turned:?} > {per_tick:?}"
        );
        total = total + turned;
        prev = now;
    }
    assert!(
        total > per_tick * S::from_i32(9),
        "kept wheeling: {total:?}"
    );
}

#[test]
fn crossing_the_bridge_morphs_to_column_and_back() {
    let mut w = common::world(120);
    // 120 hastati in four ranks: 30 files x 0.8 m = 24 m wide, the bridge
    // corridor is 8 m.
    assert_eq!(layout_of(&w, 0), Layout::Line);
    order_move(&mut w, 0, v(300.0, 450.0), Some(90.0), SpeedMode::Run);
    let (ticks, seen_column) = run_until_idle(&mut w, 0, 12_000);
    assert!(ticks < 12_000, "never arrived");
    assert!(seen_column, "never morphed to a column for the bridge");
    assert_eq!(layout_of(&w, 0), Layout::Line, "restored after the bridge");
    let state = w.view().formation_state(RegimentId(0)).unwrap();
    assert!(state.prior_template.is_none());
    let end = anchor(&w, 0);
    assert!(end.pos.distance(v(300.0, 450.0)) <= w.registries().rules.movement.waypoint_radius);
    assert!(end.pos.y > S::from_i32(400), "south of the river");
}

#[test]
fn following_is_identical_across_thread_counts() {
    let mut a = common::world(60);
    let mut b = common::world(60);
    b.set_threads(8);
    for w in [&mut a, &mut b] {
        order_move(w, 0, v(500.0, 500.0), Some(0.0), SpeedMode::Walk);
        order_move(w, 1, v(200.0, 120.0), None, SpeedMode::Run);
    }
    for _ in 0..600 {
        assert_eq!(a.step(&[]).hash, b.step(&[]).hash);
    }
    assert_eq!(anchor(&a, 0), anchor(&b, 0));
    assert_ne!(anchor(&a, 0).pos, v(300.0, 150.0), "it moved");
}
