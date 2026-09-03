//! T1-032: the nav grid of the test map, and path serving through Stage 3
//! (same request → same path at 1 and 8 threads).

mod common;

use il_core::{RegimentId, S, Scalar, V2};
use il_sim_battle::components::{Order, OrderKind, Path};
use il_sim_battle::{BattleEvent, NavGrid, PathRequests, PathResult, Pathfinder};

fn v(x: f32, y: f32) -> V2 {
    V2::from_f32_data(x, y)
}

#[test]
fn test_map_nav_grid_marks_rock_river_and_crossings() {
    let regs = common::regs();
    let w = common::world(10);
    let nav: &NavGrid = w.nav_grid();
    assert_eq!(nav.cell(), S::from_i32(4));
    assert_eq!((nav.cols(), nav.rows()), (200, 150));
    let at = |x: f32, y: f32| nav.cost(nav.cell_of(v(x, y)).0, nav.cell_of(v(x, y)).1);
    assert_eq!(at(300.0, 150.0), 100, "open ground");
    assert_eq!(at(150.0, 450.0), 150, "forest");
    assert_eq!(at(700.0, 360.0), 250, "marsh");
    assert_eq!(at(600.0, 500.0), 0, "rock is impassable");
    assert_eq!(at(100.0, 294.0), 0, "river is impassable");
    assert_eq!(at(300.0, 300.0), 0, "river is impassable");
    assert_eq!(at(100.0, 280.0), 100, "river bank");
    assert_eq!(at(398.0, 310.0), 100, "bridge");
    assert_eq!(at(650.0, 297.0), 200, "ford");
    // The bridge is two nav cells wide: an 8 m corridor.
    assert_eq!(nav.corridor_width_at(v(398.0, 310.0)), S::from_i32(8));
    assert!(nav.corridor_width_at(v(300.0, 150.0)) > S::from_i32(100));
    let _ = regs;
}

#[test]
fn paths_cross_the_river_at_the_bridge_or_ford() {
    let w = common::world(10);
    let nav = w.nav_grid();
    let mut astar = il_sim_battle::AStar::new();
    let mut out = Vec::new();
    // West side: the bridge (x = 400) is the only crossing nearby.
    assert_eq!(
        astar.find(nav, v(300.0, 150.0), v(300.0, 450.0), &mut out),
        PathResult::Found
    );
    assert!(out.len() >= 3, "{out:?}");
    let crosses_at_bridge = out.windows(2).any(|s| {
        (s[0].y <= S::from_i32(310)) != (s[1].y <= S::from_i32(310)) && {
            let t = (S::from_i32(310) - s[0].y) / (s[1].y - s[0].y);
            let x = s[0].x + (s[1].x - s[0].x) * t;
            x >= S::from_i32(396) && x <= S::from_i32(404)
        }
    });
    assert!(crosses_at_bridge, "{out:?}");
    for pair in out.windows(2) {
        assert!(nav.segment_clear(pair[0], pair[1]));
    }
}

fn request(w: &mut il_sim_battle::BattleWorld, rid: u32, target: V2) {
    let entity = w
        .ecs()
        .resource::<il_sim_battle::resources::Ids>()
        .regiment_entity(RegimentId(rid))
        .unwrap();
    {
        let mut order = w.ecs_mut().get_mut::<Order>(entity).unwrap();
        order.kind = OrderKind::Move;
        order.target = target;
    }
    w.ecs_mut().get_mut::<Path>(entity).unwrap().requested = true;
    w.ecs_mut()
        .resource_mut::<PathRequests>()
        .0
        .insert(RegimentId(rid));
    w.recompute_hash();
}

fn path_of(w: &il_sim_battle::BattleWorld, rid: u32) -> Path {
    let entity = w
        .ecs()
        .resource::<il_sim_battle::resources::Ids>()
        .regiment_entity(RegimentId(rid))
        .unwrap();
    w.ecs().get::<Path>(entity).unwrap().clone()
}

#[test]
fn stage_3_serves_requests_identically_across_thread_counts() {
    let mut a = common::world(10);
    let mut b = common::world(10);
    b.set_threads(8);
    for w in [&mut a, &mut b] {
        request(w, 0, v(300.0, 450.0));
        request(w, 1, v(700.0, 450.0));
        assert!(path_of(w, 0).requested);
        w.step(&[]);
    }
    let (pa, pb) = (path_of(&a, 0), path_of(&b, 0));
    assert_eq!(pa, pb);
    assert!(pa.is_active() && !pa.requested);
    assert_eq!(pa.next, 1);
    assert_eq!(pa.waypoints[0].p, v(300.0, 150.0), "starts at the anchor");
    assert_eq!(pa.waypoints.last().unwrap().p, v(300.0, 450.0));
    assert!(
        pa.waypoints.iter().any(|wp| wp.corridor <= S::from_i32(8)),
        "passes the bridge: {pa:?}"
    );
    assert_eq!(path_of(&a, 1), path_of(&b, 1));
    assert!(a.ecs().resource::<PathRequests>().0.is_empty());
    assert_eq!(a.hash(), b.hash());
}

#[test]
fn unreachable_targets_drop_the_order_with_an_event() {
    let mut w = common::world(10);
    // Deep inside the rock outcrop, snapped to its edge: still reachable.
    request(&mut w, 0, v(610.0, 500.0));
    let out = w.step(&[]);
    assert!(out.events.is_empty(), "{:?}", out.events);
    assert!(path_of(&w, 0).is_active());
    let last = path_of(&w, 0).waypoints.last().unwrap().p;
    assert!(w.nav_grid().is_passable_at(last));

    // A start with no passable cell within reach cannot happen on the test
    // map; fake one by asking from a world with a solid nav grid.
    let mut w = common::world(10);
    {
        let solid = NavGrid::from_costs(S::from_i32(4), 200, 150, vec![0; 200 * 150]);
        w.ecs_mut().resource_mut::<il_sim_battle::NavGridRes>().0 = solid;
    }
    request(&mut w, 1, v(500.0, 400.0));
    let out = w.step(&[]);
    assert!(
        out.events.iter().any(
            |e| matches!(e, BattleEvent::PathNotFound { regiment } if *regiment == RegimentId(1))
        ),
        "{:?}",
        out.events
    );
    let entity = w
        .ecs()
        .resource::<il_sim_battle::resources::Ids>()
        .regiment_entity(RegimentId(1))
        .unwrap();
    assert_eq!(w.ecs().get::<Order>(entity).unwrap().kind, OrderKind::Idle);
    assert!(!path_of(&w, 1).is_active());
}
