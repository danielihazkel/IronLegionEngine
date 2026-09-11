//! T3-042 (REQ-SIM-042; SIM-MOVE-004, SIM-MOVE-032, SIM-CMBT-016): rivers,
//! fords and bridges proved on `rome:test_field`. A regiment in the ford
//! moves at `move_mult` 0.5 and defends at `ford_defence_mult` 0.7 through
//! the same terrain step the melee applies; a line crosses the bridge in
//! Column with an HPA* path that touches the river only under the bridge;
//! a river cell without a crossing keeps out steering, path following and
//! collision push-out. Every scenario runs on 1 and 8 threads with equal
//! hashes.

mod common;

use il_core::{Angle, PlayerId, RegimentId, S, Scalar, StateHash, V2};
use il_data::{ContentId, Layout};
use il_sim_battle::combat::{terrain_defence_at, terrain_defence_mult};
use il_sim_battle::components::{Anchor, Combat, FormationState, Order, OrderKind, Path};
use il_sim_battle::movement::{slope_mult, zone_move_mult};
use il_sim_battle::resources::Ids;
use il_sim_battle::{
    BattleEvent, BattleSetup, BattleWorld, Command, CommandKind, RegimentSetup, SpeedMode,
};

fn v(x: f32, y: f32) -> V2 {
    V2::from_f32_data(x, y)
}

fn sf(x: f32) -> S {
    S::from_f32_data(x)
}

/// A hastati regiment anchored at `(x, y)` facing `facing_deg`.
fn hastati(id: u32, count: u16, x: f32, y: f32, facing_deg: f32) -> RegimentSetup {
    RegimentSetup {
        position: Some([x, y]),
        facing_deg: Some(facing_deg),
        ..common::regiment(id, "rome:hastati", count, x, facing_deg)
    }
}

fn setup(side0: Vec<RegimentSetup>, side1: Vec<RegimentSetup>) -> BattleSetup {
    let mut s = common::two_sides(20);
    s.sides[0].regiments = side0;
    s.sides[1].regiments = side1;
    s.sides[1].deployment_zone = 1;
    s
}

fn cmd(w: &BattleWorld, player: u8, seq: u16, kind: CommandKind) -> Command {
    Command {
        tick: w.tick().next(),
        player: PlayerId(player),
        seq,
        kind,
    }
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

fn order_kind(w: &BattleWorld, rid: u32) -> OrderKind {
    w.ecs().get::<Order>(entity(w, rid)).unwrap().kind
}

fn layout_of(w: &BattleWorld, rid: u32) -> Layout {
    let state = w.ecs().get::<FormationState>(entity(w, rid)).unwrap();
    w.registries().formations.get(state.template).layout
}

fn zone_name(w: &BattleWorld, p: V2) -> String {
    w.map()
        .zone_at(p)
        .map(|h| w.registries().zones.get(h).id.as_str().to_string())
        .unwrap_or_default()
}

/// Every living soldier stands on a passable nav cell (integrate's
/// push-out, SIM-MOVE-032: a river cell without a crossing is impassable).
fn assert_nobody_in_the_water(w: &BattleWorld, tick: u32) {
    let nav = w.nav_grid();
    for s in w.view().soldiers() {
        assert!(
            nav.is_passable_at(s.pos),
            "tick {tick}: soldier {} of regiment {} stands in an impassable cell at {:?} ({})",
            s.id.0,
            s.regiment.0,
            s.pos,
            zone_name(w, s.pos)
        );
    }
}

/// Runs `body` on a fresh world at 1 and at 8 threads and compares the
/// per-tick hashes it returns.
fn on_both_thread_counts(setup: &BattleSetup, body: impl Fn(&mut BattleWorld) -> Vec<StateHash>) {
    let mut a = BattleWorld::new(setup, common::regs()).unwrap();
    let mut b = BattleWorld::new(setup, common::regs()).unwrap();
    a.set_threads(1);
    b.set_threads(8);
    let ha = body(&mut a);
    let hb = body(&mut b);
    assert_eq!(ha.len(), hb.len());
    if let Some(t) = ha.iter().zip(&hb).position(|(x, y)| x != y) {
        panic!("1 and 8 threads diverge at tick {}", t + 1);
    }
}

fn mean_pos(w: &BattleWorld, rid: u32) -> V2 {
    let rows: Vec<V2> = w
        .view()
        .soldiers()
        .filter(|s| s.regiment == RegimentId(rid))
        .map(|s| s.pos)
        .collect();
    let n = rows.len().max(1) as i32;
    rows.into_iter().fold(V2::ZERO, |a, p| a + p) * (S::ONE / S::from_i32(n))
}

fn mean_y(w: &BattleWorld, rid: u32) -> S {
    let rows: Vec<S> = w
        .view()
        .soldiers()
        .filter(|s| s.regiment == RegimentId(rid))
        .map(|s| s.pos.y)
        .collect();
    let n = rows.len().max(1) as i32;
    rows.into_iter().fold(S::ZERO, |a, y| a + y) / S::from_i32(n)
}

// ------------------------------------------------------------------ ford

#[test]
fn a_regiment_in_the_ford_walks_at_half_speed() {
    // A on the open bank, B inside the 30 m ford (x 635..665, y 275..320;
    // 40 hastati in four ranks are 8 m wide and 3 m deep), both walking
    // 15 m east; the anchor's pace over 100 ticks after the path is served
    // is the unit's walk speed times the zone's `move_mult` and the slope.
    let setup = setup(
        vec![
            hastati(1, 40, 300.0, 150.0, 0.0),
            hastati(2, 40, 645.0, 297.0, 0.0),
        ],
        vec![hastati(3, 20, 300.0, 500.0, 180.0)],
    );
    on_both_thread_counts(&setup, |w| {
        let regs = w.registries().clone();
        let map = w.map().clone();
        let east = V2::new(S::ONE, S::ZERO);
        let a0 = anchor(w, 0).pos;
        let b0 = anchor(w, 1).pos;
        assert_eq!(zone_name(w, a0), "rome:open");
        assert_eq!(zone_name(w, b0), "rome:ford");
        assert!(map.river_at(b0), "the ford lies over the river");
        assert_eq!(zone_move_mult(&map, &regs, a0), S::ONE);
        assert_eq!(zone_move_mult(&map, &regs, b0), S::HALF);
        let slope_a = slope_mult(&map, &regs.rules.movement, a0, east);
        let slope_b = slope_mult(&map, &regs.rules.movement, b0, east);
        let mut hashes = Vec::new();
        let cmds = [
            cmd(
                w,
                0,
                0,
                CommandKind::Move {
                    regiments: vec![RegimentId(0)],
                    target: a0 + V2::new(S::from_i32(15), S::ZERO),
                    facing: None,
                    speed: SpeedMode::Walk,
                },
            ),
            cmd(
                w,
                0,
                1,
                CommandKind::Move {
                    regiments: vec![RegimentId(1)],
                    target: b0 + V2::new(S::from_i32(15), S::ZERO),
                    facing: None,
                    speed: SpeedMode::Walk,
                },
            ),
        ];
        hashes.push(w.step(&cmds).hash);
        for _ in 0..19 {
            hashes.push(w.step(&[]).hash);
        }
        let a1 = anchor(w, 0).pos;
        let b1 = anchor(w, 1).pos;
        let sa1 = mean_pos(w, 0);
        let sb1 = mean_pos(w, 1);
        for _ in 0..100 {
            hashes.push(w.step(&[]).hash);
        }
        let da = anchor(w, 0).pos.distance(a1);
        let db = anchor(w, 1).pos.distance(b1);
        assert!(
            da > S::ONE,
            "the bank regiment walked {da:?} m in 100 ticks"
        );
        assert_eq!(order_kind(w, 1), OrderKind::Move, "B is still walking");
        // d_B / d_A = (0.5 × slope_B) / (1 × slope_A).
        let expected = S::HALF * slope_b / slope_a;
        let ratio = db / da;
        assert!(
            (ratio - expected).abs() <= expected * sf(0.08),
            "ford pace ratio {ratio:?}, expected {expected:?} (A {da:?} m, B {db:?} m)"
        );
        // The soldiers keep up with their slots in the ford: steering reads
        // the zone per soldier, so their mean pace halves the same way.
        let sda = mean_pos(w, 0).distance(sa1);
        let sdb = mean_pos(w, 1).distance(sb1);
        let soldier_ratio = sdb / sda;
        // (A loose band: soldiers lag their moving slots and catch up, so
        // their mean pace is not the anchor's; the halving still shows.)
        assert!(
            (soldier_ratio - expected).abs() <= expected * sf(0.2),
            "soldiers' ford pace ratio {soldier_ratio:?}, expected {expected:?} (A {sda:?} m, B {sdb:?} m)"
        );
        hashes
    });
}

#[test]
fn the_ford_defence_multiplier_applies_through_the_melee_terrain_step() {
    let regs = common::regs();
    let w = common::world(20);
    let map = w.map().clone();
    let ford = v(650.0, 297.0);
    let bank = v(650.0, 260.0);
    let bridge = v(400.0, 310.0);
    let open = v(300.0, 150.0);
    assert_eq!(zone_name(&w, ford), "rome:ford");
    assert_eq!(zone_name(&w, bridge), "rome:bridge");
    assert_eq!(zone_name(&w, bank), "rome:open");
    assert_eq!(
        regs.rules.movement.ford_defence_mult,
        sf(0.7),
        "the flagship's ford_defence_mult"
    );
    // At equal height (attacker and defender on one spot) the terrain step
    // is the zone's factor alone: the ford's 0.7, the bank's and the
    // bridge's 1 (the bridge is a crossing without `ford`).
    assert_eq!(terrain_defence_at(&map, &regs, ford, ford), sf(0.7));
    assert_eq!(terrain_defence_at(&map, &regs, bank, bank), S::ONE);
    assert_eq!(terrain_defence_at(&map, &regs, bridge, bridge), S::ONE);
    assert_eq!(terrain_defence_at(&map, &regs, open, open), S::ONE);
    // From the bank into the ford the ford factor multiplies the height
    // term, never replaces it.
    let height_only = terrain_defence_mult(
        S::ONE,
        false,
        map.height_at(ford),
        map.height_at(bank),
        &regs.rules.movement,
        &regs.rules.combat,
    );
    assert_eq!(
        terrain_defence_at(&map, &regs, bank, ford),
        height_only * sf(0.7)
    );
    assert_eq!(terrain_defence_at(&map, &regs, ford, bank), {
        terrain_defence_mult(
            S::ONE,
            false,
            map.height_at(bank),
            map.height_at(ford),
            &regs.rules.movement,
            &regs.rules.combat,
        )
    });
}

#[test]
fn a_melee_resolves_inside_the_ford() {
    // Defenders stand in the ford facing north; attackers charge them from
    // the north bank. Morale pinned so the fight runs; the fight engages
    // with the defenders still in the ford and men fall on both sides.
    let setup = setup(
        vec![hastati(1, 40, 650.0, 335.0, 270.0)],
        vec![hastati(2, 40, 650.0, 297.0, 90.0)],
    );
    on_both_thread_counts(&setup, |w| {
        assert_eq!(zone_name(w, anchor(w, 1).pos), "rome:ford");
        let mut hashes = Vec::new();
        let attack = cmd(
            w,
            0,
            0,
            CommandKind::AttackRegiment {
                regiments: vec![RegimentId(0)],
                target: RegimentId(1),
            },
        );
        let mut engaged_at: Option<u32> = None;
        let mut deaths: (u32, u32) = (0, 0);
        for t in 0..600u32 {
            let out = if t == 0 {
                w.step(std::slice::from_ref(&attack))
            } else {
                w.step(&[])
            };
            hashes.push(out.hash);
            common::pin_morale(w);
            hashes.push(w.hash());
            for e in &out.events {
                match e {
                    BattleEvent::Engaged { regiment } if engaged_at.is_none() => {
                        engaged_at = Some(t + 1);
                        let a = anchor(w, 1).pos;
                        assert_eq!(
                            zone_name(w, a),
                            "rome:ford",
                            "regiment {} engaged with the defenders' anchor at {a:?}",
                            regiment.0
                        );
                        assert!(w.map().river_at(a));
                    }
                    BattleEvent::SoldierDied { regiment, .. } => {
                        if *regiment == RegimentId(0) {
                            deaths.0 += 1;
                        } else {
                            deaths.1 += 1;
                        }
                    }
                    _ => {}
                }
            }
            assert_nobody_in_the_water(w, t + 1);
        }
        let engaged_at = engaged_at.expect("the attackers reached the ford");
        assert!(engaged_at < 500, "engaged at tick {engaged_at}");
        let combat = *w.ecs().get::<Combat>(entity(w, 1)).unwrap();
        assert!(combat.last_fighting.0 > engaged_at, "the melee went on");
        assert!(
            deaths.0 > 0 && deaths.1 > 0,
            "deaths attackers {} defenders {}",
            deaths.0,
            deaths.1
        );
        hashes
    });
}

// ---------------------------------------------------------------- bridge

/// Samples the string-pulled path every half metre: every point must be
/// passable and the river cells it visits must lie under a crossing.
/// Returns the crossing zones seen, in order.
fn crossings_along(w: &BattleWorld, path: &Path) -> Vec<String> {
    let map = w.map();
    let nav = w.nav_grid();
    let mut seen: Vec<String> = Vec::new();
    for pair in path.waypoints.windows(2) {
        let (a, b) = (pair[0].p, pair[1].p);
        let len = a.distance(b);
        let steps = ((len + len).floor_i32() + 1).max(1);
        for k in 0..=steps {
            let t = S::from_i32(k) / S::from_i32(steps);
            let p = a + (b - a) * t;
            assert!(nav.is_passable_at(p), "path point {p:?} is impassable");
            if map.river_at(p) {
                let z = zone_name(w, p);
                assert!(
                    z == "rome:bridge" || z == "rome:ford",
                    "path point {p:?} is on the river outside a crossing ({z})"
                );
                if seen.last() != Some(&z) {
                    seen.push(z);
                }
            }
        }
    }
    seen
}

#[test]
fn a_line_crosses_the_bridge_in_column_on_an_hpa_path() {
    // 120 hastati in four ranks (30 files × 0.8 m = 24 m) ordered across
    // the river to the south; the only crossing near x = 300 is the 8 m
    // bridge at x = 400.
    let setup = setup(
        vec![hastati(1, 120, 300.0, 150.0, 0.0)],
        vec![hastati(2, 20, 700.0, 520.0, 180.0)],
    );
    on_both_thread_counts(&setup, |w| {
        assert_eq!(layout_of(w, 0), Layout::Line);
        let target = v(300.0, 450.0);
        let mut hashes = vec![
            w.step(&[cmd(
                w,
                0,
                0,
                CommandKind::Move {
                    regiments: vec![RegimentId(0)],
                    target,
                    facing: Some(Angle::from_degrees_data(90.0)),
                    speed: SpeedMode::Run,
                },
            )])
            .hash,
        ];
        // The path is served by Stage 3 of the first tick.
        let path = w.ecs().get::<Path>(entity(w, 0)).unwrap().clone();
        assert!(path.waypoints.len() >= 3, "{:?}", path.waypoints);
        assert_eq!(crossings_along(w, &path), vec!["rome:bridge".to_string()]);
        let narrowest = path
            .waypoints
            .iter()
            .map(|wp| wp.corridor)
            .fold(S::from_i32(9_999), |a, b| a.min(b));
        assert_eq!(narrowest, S::from_i32(8), "the bridge corridor");
        let mut seen_column = false;
        let mut arrived = None;
        for t in 0..12_000u32 {
            hashes.push(w.step(&[]).hash);
            if layout_of(w, 0) == Layout::Column {
                seen_column = true;
            }
            assert_nobody_in_the_water(w, t + 2);
            if order_kind(w, 0) == OrderKind::Idle {
                arrived = Some(t + 2);
                break;
            }
        }
        let arrived = arrived.expect("never arrived");
        assert!(seen_column, "never morphed to a column for the bridge");
        assert_eq!(layout_of(w, 0), Layout::Line, "restored after the bridge");
        let end = anchor(w, 0);
        assert!(end.pos.distance(target) <= w.registries().rules.movement.waypoint_radius);
        assert!(
            end.pos.y > S::from_i32(400),
            "south of the river at tick {arrived}"
        );
        hashes
    });
}

// ----------------------------------------------------------------- river

#[test]
fn the_river_keeps_out_path_following_and_steering() {
    // Ordered straight south across a stretch of river with no crossing,
    // the regiment goes round by the bridge: its anchor and its soldiers
    // are never in a river cell that is not a crossing, and it arrives.
    let setup = setup(
        vec![hastati(1, 40, 300.0, 350.0, 270.0)],
        vec![hastati(2, 20, 700.0, 520.0, 180.0)],
    );
    on_both_thread_counts(&setup, |w| {
        let target = v(300.0, 250.0);
        let mut hashes = vec![
            w.step(&[cmd(
                w,
                0,
                0,
                CommandKind::Move {
                    regiments: vec![RegimentId(0)],
                    target,
                    facing: None,
                    speed: SpeedMode::Run,
                },
            )])
            .hash,
        ];
        let path = w.ecs().get::<Path>(entity(w, 0)).unwrap().clone();
        assert_eq!(crossings_along(w, &path), vec!["rome:bridge".to_string()]);
        let mut arrived = None;
        for t in 0..12_000u32 {
            hashes.push(w.step(&[]).hash);
            let a = anchor(w, 0).pos;
            if w.map().river_at(a) {
                let z = zone_name(w, a);
                assert!(
                    z == "rome:bridge" || z == "rome:ford",
                    "tick {}: the anchor is on the river at {a:?} ({z})",
                    t + 2
                );
            }
            assert_nobody_in_the_water(w, t + 2);
            if order_kind(w, 0) == OrderKind::Idle {
                arrived = Some(t + 2);
                break;
            }
        }
        assert!(arrived.is_some(), "never arrived");
        assert!(anchor(w, 0).pos.distance(target) <= w.registries().rules.movement.waypoint_radius);
        hashes
    });
}

#[test]
fn collision_never_pushes_a_line_into_the_river() {
    // Defenders on the north bank with the river 10 m behind them (river
    // centre y = 300 at x = 300, 12 m wide, so the cells from y 292 are
    // impassable); attackers charge from the south and shove. Morale
    // pinned, 900 ticks: nobody ever stands in the water and the defenders'
    // mean position stays on the bank.
    let setup = setup(
        vec![hastati(1, 60, 300.0, 245.0, 90.0)],
        vec![hastati(2, 40, 300.0, 282.0, 270.0)],
    );
    on_both_thread_counts(&setup, |w| {
        let start_y = mean_y(w, 1);
        let mut hashes = Vec::new();
        let attack = cmd(
            w,
            0,
            0,
            CommandKind::AttackRegiment {
                regiments: vec![RegimentId(0)],
                target: RegimentId(1),
            },
        );
        let mut engaged = false;
        let mut pushed_north = false;
        for t in 0..900u32 {
            let out = if t == 0 {
                w.step(std::slice::from_ref(&attack))
            } else {
                w.step(&[])
            };
            hashes.push(out.hash);
            common::pin_morale(w);
            hashes.push(w.hash());
            engaged |= out
                .events
                .iter()
                .any(|e| matches!(e, BattleEvent::Engaged { .. }));
            assert_nobody_in_the_water(w, t + 1);
            if mean_y(w, 1) > start_y + S::HALF {
                pushed_north = true;
            }
        }
        assert!(engaged, "the attackers never reached the line");
        assert!(pushed_north, "the shove never moved the defenders");
        let end_y = mean_y(w, 1);
        assert!(
            end_y < S::from_i32(292),
            "the defenders' mean y {end_y:?} reached the river cells"
        );
        hashes
    });
}

#[test]
fn zone_ids_used_here_exist() {
    let regs = common::regs();
    for id in ["rome:ford", "rome:bridge", "rome:open"] {
        assert!(regs.zones.contains(&ContentId::new(id).unwrap()), "{id}");
    }
}
