//! T2-042: a broken regiment flees to its own edge along passable cells,
//! rallies when safe, shatters on the second rout, spreads panic, is
//! easier to hit, and everything is deterministic across threads and a
//! restore (SIM-MOR-030..034, SIM-FLOW-001..003).

mod common;

use il_core::{PlayerId, RegimentId, S, Scalar, StateHash, Tick, V2};
use il_data::MapEdge;
use il_sim_battle::components::{Combat, Morale, MoraleState, Regiment, SoldierState};
use il_sim_battle::resources::{Ids, Sides};
use il_sim_battle::{
    BattleEvent, BattleSetup, BattleWorld, Command, CommandKind, FireMode, RegimentSetup,
};

fn at(id: u32, unit: &str, count: u16, x: f32, y: f32, facing_deg: f32) -> RegimentSetup {
    RegimentSetup {
        id,
        unit_type: common::cid(unit),
        count,
        experience: 0,
        fatigue: 0.0,
        formation: None,
        position: Some([x, y]),
        facing_deg: Some(facing_deg),
    }
}

fn one_side(regiments: Vec<RegimentSetup>) -> BattleSetup {
    BattleSetup {
        map_id: common::cid("rome:test_field"),
        seed: 42,
        weather: Default::default(),
        time_of_day: 12,
        time_limit_ticks: 48_000,
        reveal_deployment: false,
        sides: vec![common::side(0, regiments)],
        victory: Default::default(),
    }
}

fn regiment_entity(w: &BattleWorld, id: u32) -> bevy_ecs::entity::Entity {
    w.ecs()
        .resource::<Ids>()
        .regiment_entity(RegimentId(id))
        .expect("regiment exists")
}

fn morale(w: &BattleWorld, id: u32) -> Morale {
    *w.ecs().get::<Morale>(regiment_entity(w, id)).unwrap()
}

fn combat(w: &BattleWorld, id: u32) -> Combat {
    *w.ecs().get::<Combat>(regiment_entity(w, id)).unwrap()
}

fn soldiers(w: &BattleWorld, id: u32) -> usize {
    w.ecs()
        .get::<Regiment>(regiment_entity(w, id))
        .unwrap()
        .soldiers
        .len()
}

fn set_morale(w: &mut BattleWorld, id: u32, m: f32) {
    let e = regiment_entity(w, id);
    w.ecs_mut().get_mut::<Morale>(e).unwrap().m = S::from_f32_data(m);
    w.recompute_hash();
}

fn hold_fire(w: &mut BattleWorld) {
    let entities: Vec<_> = w
        .ecs()
        .resource::<Ids>()
        .regiment_entities
        .iter()
        .map(|(_, e)| *e)
        .collect();
    for e in entities {
        if let Some(mut fire) = w.ecs_mut().get_mut::<il_sim_battle::components::Fire>(e) {
            fire.mode = FireMode::Hold;
        }
    }
    w.recompute_hash();
}

/// Steps to `until` collecting the events named by `keep`; every soldier
/// must stand on a passable cell after every tick.
fn run(
    w: &mut BattleWorld,
    commands: &[Command],
    until: u32,
    events: &mut Vec<(u32, BattleEvent)>,
) -> Vec<StateHash> {
    let mut hashes = Vec::new();
    while w.tick().0 < until {
        let next = w.tick().next();
        let batch: Vec<Command> = commands
            .iter()
            .filter(|c| c.tick == next)
            .cloned()
            .collect();
        let out = w.step(&batch);
        assert!(out.rejected.is_empty(), "{:?}", out.rejected);
        for e in &out.events {
            if matches!(
                e,
                BattleEvent::MoraleChanged { .. }
                    | BattleEvent::Rallied { .. }
                    | BattleEvent::Shattered { .. }
                    | BattleEvent::SoldierFled { .. }
                    | BattleEvent::SoldierDied { .. }
            ) {
                events.push((next.0, e.clone()));
            }
        }
        let nav = w.nav_grid();
        for s in w.view().soldiers() {
            assert!(
                nav.is_passable_at(s.pos),
                "soldier {:?} in rock at {:?}",
                s.id,
                s.pos
            );
        }
        hashes.push(out.hash);
    }
    hashes
}

fn fled_events(events: &[(u32, BattleEvent)], regiment: u32) -> usize {
    events
        .iter()
        .filter(
            |(_, e)| matches!(e, BattleEvent::SoldierFled { regiment: r, .. } if r.0 == regiment),
        )
        .count()
}

/// SIM-FLOW-001 (plan decision 9): on the test map side 0's deployment
/// band lies along the south edge and side 1's along the north edge.
#[test]
fn escape_edges_face_the_deployment_zones() {
    let mut setup = common::two_sides(10);
    setup.sides[1].deployment_zone = 1;
    let w = BattleWorld::new(&setup, common::regs()).unwrap();
    let sides = &w.ecs().resource::<Sides>().0;
    assert_eq!(sides[0].escape_edge, MapEdge::South);
    assert_eq!(sides[1].escape_edge, MapEdge::North);
    let field = w.view().flow_field(0).unwrap();
    assert_eq!(field.edge(), MapEdge::South);
    assert_eq!(
        field.direction_at(w.nav_grid(), V2::from_f32_data(300.0, 150.0)),
        V2::new(S::ZERO, -S::ONE)
    );
}

/// Done-when: a broken regiment flees to its own edge along passable
/// cells and leaves the field (SIM-MOR-030, SIM-FLOW-002).
#[test]
fn a_broken_regiment_flees_to_its_edge_and_leaves() {
    let mut w = common::world(100);
    hold_fire(&mut w);
    set_morale(&mut w, 0, 0.0);
    let mut events = Vec::new();
    run(&mut w, &[], 1, &mut events);
    let m = morale(&w, 0);
    // Recovery lifts the 0 above zero within the tick, so this is a plain
    // rout, not the SIM-MOR-032 morale-zero shatter.
    assert_eq!(m.state, MoraleState::Routing);
    assert_eq!(m.rout_count, 1);
    let order = *w
        .ecs()
        .get::<il_sim_battle::components::Order>(regiment_entity(&w, 0))
        .unwrap();
    assert_eq!(order.kind, il_sim_battle::components::OrderKind::Idle);
    assert_eq!(order.speed, il_sim_battle::SpeedMode::Run);
    assert!(
        w.view()
            .soldiers()
            .filter(|s| s.regiment.0 == 0)
            .all(|s| s.state == SoldierState::Routing)
    );
    assert!(events.iter().any(|(_, e)| matches!(e, BattleEvent::MoraleChanged { regiment, to: MoraleState::Routing, .. } if regiment.0 == 0)));

    // 150 m south at run: gone well inside a minute.
    run(&mut w, &[], 1_200, &mut events);
    assert_eq!(soldiers(&w, 0), 0, "everyone left");
    assert_eq!(combat(&w, 0).fled, 101, "100 plus the general");
    assert_eq!(fled_events(&events, 0), 101);
    assert!(
        !events
            .iter()
            .any(|(_, e)| matches!(e, BattleEvent::SoldierDied { .. }))
    );
    assert_eq!(soldiers(&w, 1), 101, "the enemy was untouched");
    assert!(
        w.view().regiment(RegimentId(0)).is_some(),
        "the entity stays"
    );
    assert_eq!(w.view().regiment(RegimentId(0)).unwrap().fled, 101);
}

/// SIM-MOR-031: a Routing regiment rallies at `t_routing + rally_margin`
/// when no enemy is within `rally_safe_radius`, and not otherwise.
#[test]
fn a_routing_regiment_rallies_when_safe() {
    let mut w = BattleWorld::new(
        &one_side(vec![at(1, "rome:hastati", 60, 300.0, 150.0, 0.0)]),
        common::regs(),
    )
    .unwrap();
    set_morale(&mut w, 0, 10.0);
    let mut events = Vec::new();
    run(&mut w, &[], 1, &mut events);
    assert_eq!(morale(&w, 0).state, MoraleState::Routing);
    assert_eq!(morale(&w, 0).rout_count, 1);
    run(&mut w, &[], 60, &mut events);
    let before = w.view().regiment(RegimentId(0)).unwrap().anchor_pos;
    assert!(before.y < S::from_i32(145), "ran south: {before:?}");
    // Nobody around, but a scattered regiment's integrity drains a little
    // within the tick: push it past the rally line (30).
    set_morale(&mut w, 0, 40.0);
    run(&mut w, &[], 61, &mut events);
    assert_eq!(morale(&w, 0).state, MoraleState::Shaken);
    assert!(
        events
            .iter()
            .any(|(_, e)| matches!(e, BattleEvent::Rallied { .. }))
    );
    assert!(
        w.view()
            .soldiers()
            .all(|s| s.state != SoldierState::Routing)
    );
    run(&mut w, &[], 400, &mut events);
    assert_eq!(
        soldiers(&w, 0),
        61,
        "nobody left the field (60 plus the general)"
    );
    assert!(w.view().regiment(RegimentId(0)).unwrap().integrity > S::from_f32_data(0.9));

    // With an enemy 40 m away the same morale does not rally.
    let mut setup = common::two_sides(60);
    setup.sides[1].regiments[0].position = Some([340.0, 150.0]);
    let mut w = BattleWorld::new(&setup, common::regs()).unwrap();
    hold_fire(&mut w);
    set_morale(&mut w, 0, 10.0);
    let mut events = Vec::new();
    run(&mut w, &[], 1, &mut events);
    assert_eq!(morale(&w, 0).state, MoraleState::Routing);
    set_morale(&mut w, 0, 40.0);
    run(&mut w, &[], 2, &mut events);
    assert_eq!(
        morale(&w, 0).state,
        MoraleState::Routing,
        "enemy within 50 m"
    );
}

/// Done-when (plan decision 3): the second rout shatters; a regiment under
/// `shatter_strength` shatters on its first.
#[test]
fn the_second_rout_shatters() {
    let mut w = BattleWorld::new(
        &one_side(vec![at(1, "rome:hastati", 60, 300.0, 150.0, 0.0)]),
        common::regs(),
    )
    .unwrap();
    let mut events = Vec::new();
    set_morale(&mut w, 0, 10.0);
    run(&mut w, &[], 1, &mut events);
    assert_eq!(morale(&w, 0).state, MoraleState::Routing);
    set_morale(&mut w, 0, 30.0);
    run(&mut w, &[], 2, &mut events);
    assert_eq!(morale(&w, 0).state, MoraleState::Shaken);
    set_morale(&mut w, 0, 10.0);
    run(&mut w, &[], 3, &mut events);
    assert_eq!(morale(&w, 0).state, MoraleState::Shattered);
    assert_eq!(morale(&w, 0).rout_count, 2);
    assert_eq!(
        events
            .iter()
            .filter(|(_, e)| matches!(e, BattleEvent::Shattered { .. }))
            .count(),
        1
    );
    set_morale(&mut w, 0, 100.0);
    run(&mut w, &[], 40, &mut events);
    assert_eq!(
        morale(&w, 0).state,
        MoraleState::Shattered,
        "no rally out of Shattered"
    );
    assert!(
        w.view()
            .soldiers()
            .all(|s| s.state == SoldierState::Routing)
    );

    // 12 of 60 left is under a quarter: the first rout shatters.
    let mut w = BattleWorld::new(
        &one_side(vec![at(1, "rome:hastati", 60, 300.0, 150.0, 0.0)]),
        common::regs(),
    )
    .unwrap();
    {
        let e = regiment_entity(&w, 0);
        let victims: Vec<_> = w.ecs().get::<Regiment>(e).unwrap().soldiers[12..].to_vec();
        for v in victims {
            let se = w.ecs().resource::<Ids>().soldier_entity(v).unwrap();
            w.ecs_mut()
                .get_mut::<il_sim_battle::components::Health>(se)
                .unwrap()
                .hp = S::ZERO;
            w.ecs_mut()
                .resource_mut::<il_sim_battle::combat::Kills>()
                .0
                .push(il_sim_battle::combat::Kill {
                    victim: v,
                    killer: None,
                    killer_regiment: None,
                });
        }
    }
    let mut events = Vec::new();
    run(&mut w, &[], 1, &mut events);
    assert_eq!(soldiers(&w, 0), 12);
    set_morale(&mut w, 0, 10.0);
    run(&mut w, &[], 2, &mut events);
    assert_eq!(morale(&w, 0).state, MoraleState::Shattered);
}

/// SIM-MOR-033: allies within `rout_shock_radius` lose `rout_shock` on the
/// tick after a rout; farther allies do not.
#[test]
fn a_rout_shocks_its_neighbours() {
    let mut w = BattleWorld::new(
        &one_side(vec![
            at(1, "rome:hastati", 60, 300.0, 150.0, 0.0),
            at(2, "rome:hastati", 60, 300.0, 170.0, 0.0),
            at(3, "rome:hastati", 60, 300.0, 210.0, 0.0),
        ]),
        common::regs(),
    )
    .unwrap();
    let mut events = Vec::new();
    run(&mut w, &[], 10, &mut events);
    let (near0, far0) = (morale(&w, 1).m, morale(&w, 2).m);
    set_morale(&mut w, 0, 10.0);
    run(&mut w, &[], 11, &mut events);
    assert_eq!(morale(&w, 0).state, MoraleState::Routing);
    let (near1, far1) = (morale(&w, 1).m, morale(&w, 2).m);
    run(&mut w, &[], 12, &mut events);
    let (near2, far2) = (morale(&w, 1).m, morale(&w, 2).m);
    // Recovery drifts everyone up by 0.15 per tick; the shock is -5 once.
    assert!(
        near1 > near0 && far1 > far0,
        "no shock on the rout tick itself"
    );
    assert!(
        near1 - near2 > S::from_f32_data(4.5),
        "near ally shocked: {near1:?} -> {near2:?}"
    );
    assert!(far2 > far1, "far ally untouched: {far1:?} -> {far2:?}");
}

/// SIM-MOR-034: routers are hit with `pursuit_hit_mult`; cavalry chasing
/// them runs. Determinism across threads and a restore mid-rout.
#[test]
fn routers_are_easier_to_hit_and_everything_is_deterministic() {
    // A rear cavalry charge into a hastati line that will break.
    let mut setup = common::two_sides(120);
    setup.sides[0].regiments = vec![at(1, "rome:hastati", 120, 300.0, 150.0, 0.0)];
    setup.sides[1].regiments = vec![at(2, "persia:cavalry", 60, 240.0, 150.0, 0.0)];
    let commands = [
        Command {
            tick: Tick(1),
            player: PlayerId(1),
            seq: 0,
            kind: CommandKind::SetSpeedMode {
                regiments: vec![RegimentId(1)],
                mode: il_sim_battle::SpeedMode::Run,
            },
        },
        Command {
            tick: Tick(2),
            player: PlayerId(1),
            seq: 0,
            kind: CommandKind::AttackRegiment {
                regiments: vec![RegimentId(1)],
                target: RegimentId(0),
            },
        },
    ];
    let mut w = BattleWorld::new(&setup, common::regs()).unwrap();
    hold_fire(&mut w);
    let mut events = Vec::new();
    let mut hashes = run(&mut w, &commands, 800, &mut events);
    let snap = w.snapshot();
    hashes.extend(run(&mut w, &commands, 2_400, &mut events));
    let routed = events
        .iter()
        .any(|(_, e)| matches!(e, BattleEvent::MoraleChanged { regiment, to, .. } if regiment.0 == 0 && matches!(to, MoraleState::Routing | MoraleState::Shattered)));
    assert!(routed, "the hastati broke: {events:?}");
    let died_after = events
        .iter()
        .filter(|(t, e)| {
            matches!(e, BattleEvent::SoldierDied { regiment, .. } if regiment.0 == 0) && *t > 800
        })
        .count();
    assert!(
        died_after > 0 || combat(&w, 0).fled > 0,
        "pursuit killed or the routers escaped: {:?}",
        combat(&w, 0)
    );

    let mut w8 = BattleWorld::new(&setup, common::regs()).unwrap();
    hold_fire(&mut w8);
    w8.set_threads(8);
    let hashes8 = run(&mut w8, &commands, 2_400, &mut Vec::new());
    if let Some(t) = hashes.iter().zip(&hashes8).position(|(a, b)| a != b) {
        panic!("1 vs 8 threads diverged at tick {}", t + 1);
    }

    let mut restored = BattleWorld::restore(&snap, common::regs()).unwrap();
    assert_eq!(restored.hash(), hashes[799]);
    let tail = run(&mut restored, &commands, 2_400, &mut Vec::new());
    if let Some(t) = hashes[800..].iter().zip(&tail).position(|(a, b)| a != b) {
        panic!("restore diverged at tick {}", 801 + t);
    }
}

/// SIM-MOR-034 formula: the pursuit multiplier applies after the clamp and
/// is clamped again at `max_hit`.
#[test]
fn pursuit_hit_probability_is_clamped() {
    let regs = common::regs();
    let c = &regs.rules.combat;
    let p = il_sim_battle::combat::hit_probability(S::from_i32(30), S::from_i32(30), c);
    assert_eq!(p, c.base_hit);
    assert_eq!(
        (p * c.pursuit_hit_mult).min(c.max_hit),
        S::from_f32_data(0.75)
    );
    let p = il_sim_battle::combat::hit_probability(S::from_i32(90), S::from_i32(10), c);
    assert_eq!((p * c.pursuit_hit_mult).min(c.max_hit), c.max_hit);
}
