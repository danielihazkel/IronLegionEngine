//! T2-070: battle phases (SIM-FLOW-010..017). A scripted battle passes
//! through Deployment, Battle, Pursuit and Ended in order; deployment
//! placement, confirmation and the timeout; withdrawal; surrender;
//! reinforcements; the time limit; the pursuit timer; the frozen end;
//! determinism across threads and restores in two phases.

mod common;

use common::cid;
use il_core::{Angle, PlayerId, RegimentId, S, Scalar, StateHash, Tick, V2};
use il_data::MapEdge;
use il_sim_battle::components::{Combat, Fsm, Order, OrderKind, Regiment, SoldierState};
use il_sim_battle::resources::{Ids, Sides};
use il_sim_battle::{
    BattleEvent, BattlePhase, BattleSetup, BattleWorld, Command, CommandKind, FireMode,
    RegimentSetup, ReinforcementGroup, RejectReason, SetupError, SideSetup, SpeedMode,
};

fn reg(id: u32, unit: &str, count: u16, position: Option<[f32; 2]>) -> RegimentSetup {
    RegimentSetup {
        id,
        unit_type: cid(unit),
        count,
        experience: 0,
        fatigue: 0.0,
        formation: Some(cid("rome:line")),
        position,
        facing_deg: None,
    }
}

fn side(player: u8, regiments: Vec<RegimentSetup>) -> SideSetup {
    SideSetup {
        deployment_zone: player,
        ..common::side(player, regiments)
    }
}

fn setup(sides: Vec<SideSetup>) -> BattleSetup {
    BattleSetup {
        map_id: cid("rome:test_field"),
        seed: 11,
        weather: Default::default(),
        time_of_day: 12,
        time_limit_ticks: 48_000,
        reveal_deployment: false,
        sides,
        victory: Default::default(),
    }
}

fn command(tick: u32, player: u8, kind: CommandKind) -> Command {
    Command {
        tick: Tick(tick),
        player: PlayerId(player),
        seq: 0,
        kind,
    }
}

struct Run {
    hashes: Vec<StateHash>,
    events: Vec<(u32, BattleEvent)>,
    rejected: Vec<(u32, RejectReason)>,
}

fn run(w: &mut BattleWorld, commands: &[Command], until: u32) -> Run {
    let mut out = Run {
        hashes: Vec::new(),
        events: Vec::new(),
        rejected: Vec::new(),
    };
    while w.tick().0 < until {
        let next = w.tick().next();
        let due: Vec<Command> = commands
            .iter()
            .filter(|c| c.tick == next)
            .cloned()
            .collect();
        let step = w.step(&due);
        out.hashes.push(w.hash());
        out.events
            .extend(step.events.into_iter().map(|e| (next.0, e)));
        out.rejected
            .extend(step.rejected.into_iter().map(|(_, r)| (next.0, r)));
    }
    out
}

fn phases(events: &[(u32, BattleEvent)]) -> Vec<(u32, BattlePhase, BattlePhase)> {
    events
        .iter()
        .filter_map(|(t, e)| match e {
            BattleEvent::PhaseChanged { from, to } => Some((*t, *from, *to)),
            _ => None,
        })
        .collect()
}

fn anchor(w: &BattleWorld, id: u32) -> V2 {
    w.view().regiment(RegimentId(id)).unwrap().anchor_pos
}

/// The `tests/scenarios/phases_all_four.json5` setup: nobody pre-placed,
/// 240 hastati against 60 velites.
fn all_four() -> BattleSetup {
    setup(vec![
        side(
            0,
            vec![
                reg(1, "rome:hastati", 120, None),
                reg(2, "rome:hastati", 120, None),
            ],
        ),
        side(1, vec![reg(3, "rome:velites", 60, None)]),
    ])
}

fn all_four_commands() -> Vec<Command> {
    vec![
        command(
            5,
            0,
            CommandKind::Deploy {
                regiment: RegimentId(1),
                position: V2::new(S::from_i32(420), S::from_i32(160)),
                facing: Angle::from_direction(V2::new(S::ZERO, S::ONE)),
                template: None,
            },
        ),
        command(10, 0, CommandKind::ConfirmDeployment),
        command(10, 1, CommandKind::ConfirmDeployment),
        command(
            20,
            0,
            CommandKind::FireMode {
                regiments: vec![RegimentId(0), RegimentId(1)],
                mode: FireMode::Hold,
            },
        ),
        Command {
            seq: 1,
            ..command(
                20,
                0,
                CommandKind::SetSpeedMode {
                    regiments: vec![RegimentId(0), RegimentId(1)],
                    mode: SpeedMode::Run,
                },
            )
        },
        Command {
            seq: 2,
            ..command(
                20,
                0,
                CommandKind::AttackMove {
                    regiments: vec![RegimentId(0), RegimentId(1)],
                    target: V2::new(S::from_i32(400), S::from_i32(480)),
                },
            )
        },
    ]
}

#[test]
fn a_scripted_battle_passes_through_all_four_phases_in_order() {
    let regs = common::regs();
    let mut w = BattleWorld::new(&all_four(), regs).unwrap();
    assert_eq!(w.phase(), BattlePhase::Deployment);
    // Auto-placement: side 0 in zone 0 (y 40..200), side 1 in zone 1
    // (y 400..560), facing each other, no two anchors on one spot.
    let a0 = anchor(&w, 0);
    let a1 = anchor(&w, 1);
    let a2 = anchor(&w, 2);
    assert!(a0.y > S::from_i32(40) && a0.y < S::from_i32(200), "{a0:?}");
    assert!(a1.y > S::from_i32(40) && a1.y < S::from_i32(200), "{a1:?}");
    assert!(a2.y > S::from_i32(400) && a2.y < S::from_i32(560), "{a2:?}");
    assert!(a0.distance(a1) > S::from_i32(20), "{a0:?} {a1:?}");
    assert!(!w.ecs().resource::<Sides>().0[0].deployment_confirmed);
    let commands = all_four_commands();
    let out = run(&mut w, &commands, 8_000);
    assert!(out.rejected.is_empty(), "{:?}", out.rejected);
    // The tick-5 deploy moved regiment 1 and nothing else moved before tick 10.
    assert!(
        out.events
            .iter()
            .any(|(t, e)| *t == 10 && matches!(e, BattleEvent::DeploymentConfirmed { side: 1 }))
    );
    let ph = phases(&out.events);
    assert_eq!(ph.len(), 3, "{ph:?}");
    assert_eq!(ph[0], (10, BattlePhase::Deployment, BattlePhase::Battle));
    assert_eq!(ph[1].1, BattlePhase::Battle);
    assert_eq!(ph[1].2, BattlePhase::Pursuit);
    assert_eq!(ph[2].1, BattlePhase::Pursuit);
    assert_eq!(ph[2].2, BattlePhase::Ended);
    assert!(ph[1].0 > 20 && ph[2].0 > ph[1].0, "{ph:?}");
    let ended: Vec<_> = out
        .events
        .iter()
        .filter_map(|(t, e)| match e {
            BattleEvent::Ended { result } => Some((*t, result.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].0, ph[2].0);
    assert_eq!(ended[0].1.winner, Some(0));
    assert_eq!(w.phase(), BattlePhase::Ended);
    assert_eq!(w.view().flow().winner, Some(0));
    // Frozen: orders are refused, pause is not, nothing moves.
    run(&mut w, &[], 8_010);
    let t = w.tick().next();
    let step = w.step(&[
        command(
            t.0,
            0,
            CommandKind::Move {
                regiments: vec![RegimentId(0)],
                target: V2::ZERO,
                facing: None,
                speed: SpeedMode::Walk,
            },
        ),
        Command {
            seq: 1,
            ..command(t.0, 0, CommandKind::Pause)
        },
    ]);
    assert_eq!(step.rejected.len(), 1);
    assert_eq!(step.rejected[0].1, RejectReason::WrongPhase);
    let before = anchor(&w, 0);
    let until = w.tick().0 + 50;
    run(&mut w, &[], until);
    assert_eq!(anchor(&w, 0), before, "nothing moves after the end");
}

#[test]
fn deploy_checks_the_zone_and_the_phase_and_the_timeout_ends_the_deployment() {
    let regs = common::regs();
    let mut w = BattleWorld::new(&all_four(), regs.clone()).unwrap();
    let inside = V2::new(S::from_i32(300), S::from_i32(100));
    let outside = V2::new(S::from_i32(300), S::from_i32(300));
    let facing = Angle::from_direction(V2::new(S::ZERO, S::ONE));
    let deploy = |tick, position| {
        command(
            tick,
            0,
            CommandKind::Deploy {
                regiment: RegimentId(0),
                position,
                facing,
                template: None,
            },
        )
    };
    let out = run(
        &mut w,
        &[
            deploy(1, outside),
            deploy(2, inside),
            command(
                3,
                0,
                CommandKind::Move {
                    regiments: vec![RegimentId(0)],
                    target: inside,
                    facing: None,
                    speed: SpeedMode::Walk,
                },
            ),
        ],
        3,
    );
    assert_eq!(
        out.rejected
            .iter()
            .map(|(_, r)| r.clone())
            .collect::<Vec<_>>(),
        vec![
            RejectReason::OutsideDeploymentZone {
                regiment: RegimentId(0)
            },
            RejectReason::WrongPhase
        ]
    );
    assert_eq!(anchor(&w, 0), inside);
    let e = w
        .ecs()
        .resource::<Ids>()
        .regiment_entity(RegimentId(0))
        .unwrap();
    let soldiers = w.ecs().get::<Regiment>(e).unwrap().soldiers.clone();
    for sid in soldiers {
        let se = w.ecs().resource::<Ids>().soldier_entity(sid).unwrap();
        let p = w.ecs().get::<il_sim_battle::components::Pos>(se).unwrap().p;
        assert!(p.distance(inside) < S::from_i32(40), "{p:?}");
    }
    // Nobody confirms: still deploying after 500 ticks with the default
    // timeout of 0 (none)...
    run(&mut w, &[], 500);
    assert_eq!(w.phase(), BattlePhase::Deployment);
    // ...and an engine-AI side deploys and confirms by itself through
    // its Stage 1 commands (T2-081): queued at tick 1, applied at tick 2.
    let mut ai = all_four();
    ai.sides[1].player = PlayerId::ENGINE_AI;
    let mut w = BattleWorld::new(&ai, regs.clone()).unwrap();
    let out = run(&mut w, &[command(3, 0, CommandKind::ConfirmDeployment)], 3);
    assert!(
        out.events
            .iter()
            .any(|(t, e)| *t == 2 && matches!(e, BattleEvent::DeploymentConfirmed { side: 1 }))
    );
    assert_eq!(
        phases(&out.events),
        vec![(3, BattlePhase::Deployment, BattlePhase::Battle)]
    );
    assert_eq!(w.view().flow().battle_start, Tick(3));
}

#[test]
fn pre_placed_sides_start_in_battle_as_before() {
    let w = common::world(20);
    assert_eq!(w.phase(), BattlePhase::Battle);
    assert!(
        w.ecs()
            .resource::<Sides>()
            .0
            .iter()
            .all(|s| s.deployment_confirmed)
    );
    assert_eq!(w.view().flow().battle_start, Tick::ZERO);
}

#[test]
fn withdrawing_regiments_march_off_as_survivors_and_lose_the_battle() {
    // Side 1's velites withdraw north from y = 150 (their escape edge is
    // North, 450 m away) without a fight; side 0 stands.
    let regs = common::regs();
    let s = setup(vec![
        side(0, vec![reg(1, "rome:hastati", 20, Some([300.0, 150.0]))]),
        side(1, vec![reg(2, "rome:velites", 20, Some([600.0, 150.0]))]),
    ]);
    let mut w = BattleWorld::new(&s, regs).unwrap();
    assert_eq!(w.phase(), BattlePhase::Battle);
    let first = run(
        &mut w,
        &[command(
            1,
            1,
            CommandKind::Withdraw {
                regiments: vec![RegimentId(1)],
            },
        )],
        1,
    );
    assert!(first.rejected.is_empty(), "{:?}", first.rejected);
    assert!(first.events.iter().any(
        |(_, e)| matches!(e, BattleEvent::Withdrawing { regiment } if *regiment == RegimentId(1))
    ));
    let e = w
        .ecs()
        .resource::<Ids>()
        .regiment_entity(RegimentId(1))
        .unwrap();
    assert_eq!(w.ecs().get::<Order>(e).unwrap().kind, OrderKind::Withdraw);
    assert_eq!(w.ecs().get::<Order>(e).unwrap().speed, SpeedMode::March);
    let soldiers = w.ecs().get::<Regiment>(e).unwrap().soldiers.clone();
    assert!(soldiers.iter().all(|sid| {
        let se = w.ecs().resource::<Ids>().soldier_entity(*sid).unwrap();
        w.ecs().get::<Fsm>(se).unwrap().state == SoldierState::Withdrawing
    }));
    // A regiment that is withdrawing does not count as standing: the side
    // is defeated at once (Stage 16 of tick 1) and the battle goes to
    // Pursuit, then ends when no router remains (nobody routs: the
    // withdrawers are not routers).
    let mut out = run(&mut w, &[], 1_500);
    let mut events = first.events;
    events.append(&mut out.events);
    out.events = events;
    let ph = phases(&out.events);
    assert_eq!(
        ph.first().map(|p| (p.1, p.2)),
        Some((BattlePhase::Battle, BattlePhase::Pursuit))
    );
    assert_eq!(
        ph.last().map(|p| (p.1, p.2)),
        Some((BattlePhase::Pursuit, BattlePhase::Ended))
    );
    let c = w.ecs().get::<Combat>(e).unwrap();
    assert_eq!(c.fled, 0);
    assert_eq!(w.view().flow().winner, Some(0));
    // The withdrawers still on the field when the pursuit closed remain
    // survivors in the result; those that reached the edge count as
    // withdrawn.
    let result = out
        .events
        .iter()
        .find_map(|(_, e)| match e {
            BattleEvent::Ended { result } => Some(result.clone()),
            _ => None,
        })
        .unwrap();
    let r = &result.sides[1].regiments[0];
    assert_eq!(r.initial, 21);
    assert_eq!(r.survivors, 21, "{r:?}");
    assert_eq!(r.killed + r.fled, 0);
    assert_eq!(
        u32::from(r.survivors),
        out.events
            .iter()
            .filter(|(_, e)| matches!(e, BattleEvent::SoldierWithdrew { .. }))
            .count() as u32
            + w.view()
                .regiment(RegimentId(1))
                .map_or(0, |x| x.soldier_count)
    );
}

#[test]
fn withdraw_on_a_routing_regiment_is_ignored_and_surrender_ends_the_battle() {
    let regs = common::regs();
    let mut w = common::world(20);
    let e = w
        .ecs()
        .resource::<Ids>()
        .regiment_entity(RegimentId(1))
        .unwrap();
    w.ecs_mut()
        .get_mut::<il_sim_battle::components::Morale>(e)
        .unwrap()
        .m = S::ZERO;
    w.recompute_hash();
    w.step(&[]);
    assert!(matches!(
        w.view().regiment(RegimentId(1)).unwrap().morale_state,
        il_sim_battle::components::MoraleState::Routing
            | il_sim_battle::components::MoraleState::Shattered
    ));
    let t = w.tick().next();
    let step = w.step(&[command(
        t.0,
        1,
        CommandKind::Withdraw {
            regiments: vec![RegimentId(1)],
        },
    )]);
    assert!(
        step.rejected.is_empty(),
        "ignored, not refused: {:?}",
        step.rejected
    );
    assert_ne!(w.ecs().get::<Order>(e).unwrap().kind, OrderKind::Withdraw);

    let mut w = BattleWorld::new(&common::two_sides(20), regs).unwrap();
    let out = run(&mut w, &[command(1, 1, CommandKind::Surrender)], 3);
    assert!(
        out.events
            .iter()
            .any(|(t, e)| *t == 1 && matches!(e, BattleEvent::Surrendered { side: 1 }))
    );
    let ph = phases(&out.events);
    assert_eq!(
        ph.first().map(|p| (p.0, p.2)),
        Some((1, BattlePhase::Pursuit))
    );
    assert_eq!(
        ph.last().map(|p| p.2),
        Some(BattlePhase::Ended),
        "no routers: the pursuit ends at once"
    );
    assert_eq!(w.view().flow().winner, Some(0));
}

#[test]
fn reinforcements_arrive_in_column_at_their_edge_and_are_validated() {
    let regs = common::regs();
    let mut s = setup(vec![
        side(0, vec![reg(1, "rome:hastati", 20, Some([300.0, 150.0]))]),
        side(1, vec![reg(2, "rome:hastati", 20, Some([340.0, 150.0]))]),
    ]);
    s.sides[0].reinforcements = vec![ReinforcementGroup {
        arrival_tick: 100,
        edge: MapEdge::South,
        regiments: vec![
            reg(10, "rome:velites", 30, None),
            reg(11, "rome:hastati", 40, None),
        ],
    }];
    // An edge the map does not list for the zone is refused.
    let mut bad = s.clone();
    bad.sides[0].reinforcements[0].edge = MapEdge::East;
    assert!(matches!(
        BattleWorld::new(&bad, regs.clone()),
        Err(SetupError::UnknownReinforcementEdge { side: 0, .. })
    ));
    let mut w = BattleWorld::new(&s, regs.clone()).unwrap();
    assert_eq!(w.regiment_count(), 2);
    let out = run(&mut w, &[], 99);
    assert_eq!(w.regiment_count(), 2, "not before tick 100");
    assert!(!w.ecs().resource::<Sides>().0[0].defeated);
    let out2 = run(&mut w, &[], 100);
    assert_eq!(w.regiment_count(), 4);
    assert!(
        out2.events
            .iter()
            .any(|(t, e)| *t == 100 && matches!(e, BattleEvent::ReinforcementsArrived { side: 0 }))
    );
    assert!(
        out.events
            .iter()
            .all(|(_, e)| !matches!(e, BattleEvent::ReinforcementsArrived { .. }))
    );
    let map = w.map();
    let a = anchor(&w, 2);
    let b = anchor(&w, 3);
    assert!(
        a.y < S::from_i32(5) && b.y < S::from_i32(5),
        "south edge: {a:?} {b:?}"
    );
    assert!((a.x - map.width * S::HALF).abs() < S::from_i32(40));
    assert!(a.x < b.x, "side by side in setup order");
    let rows: Vec<_> = w.view().regiments().collect();
    assert_eq!(rows[2].soldier_count, 30);
    assert_eq!(rows[3].soldier_count, 40);
    let column = regs.formations.lookup(&cid("rome:column")).unwrap();
    assert_eq!(rows[2].formation, column);
    assert_eq!(rows[2].side, 0);
    assert_eq!(w.ecs().resource::<Sides>().0[0].reinforcements_spawned, 1);
    // Ids ascend past the initial ones and the new regiments fight on.
    assert!(rows[3].id > rows[1].id);
    run(&mut w, &[], 200);
    assert_eq!(w.regiment_count(), 4);
    // The cap: a group that would pass it is dropped with an event.
    let mut huge = s.clone();
    huge.sides[0].reinforcements[0].regiments[1].count = 33_000;
    assert!(matches!(
        BattleWorld::new(&huge, regs),
        Err(SetupError::OverCap { .. })
    ));
}

#[test]
fn the_time_limit_picks_the_winner_by_setup_side_policy_or_draw() {
    let regs = common::regs();
    let base = || {
        let mut s = setup(vec![
            side(0, vec![reg(1, "rome:hastati", 30, Some([300.0, 150.0]))]),
            side(1, vec![reg(2, "rome:hastati", 10, Some([500.0, 150.0]))]),
        ]);
        s.time_limit_ticks = 50;
        s
    };
    // Defender policy (the flagship default) with no side named: a draw.
    let mut w = BattleWorld::new(&base(), regs.clone()).unwrap();
    let out = run(&mut w, &[], 60);
    let ph = phases(&out.events);
    assert_eq!(ph, vec![(50, BattlePhase::Battle, BattlePhase::Ended)]);
    assert_eq!(w.view().flow().winner, None);
    // The setup's side wins.
    let mut named = base();
    named.victory.timeout_winner = Some(1);
    let mut w = BattleWorld::new(&named, regs.clone()).unwrap();
    run(&mut w, &[], 60);
    assert_eq!(w.view().flow().winner, Some(1));
    // MostSoldiers through the `tests/mods/timeout_most_soldiers` override.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let regs_ms = il_data::Registries::load_roots(&[
        root.join("game"),
        root.join("tests/mods/timeout_most_soldiers"),
    ])
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        regs_ms.rules.battle_flow.timeout_winner,
        il_data::TimeoutWinner::MostSoldiers
    );
    let mut w = BattleWorld::new(&base(), std::sync::Arc::new(regs_ms)).unwrap();
    run(&mut w, &[], 60);
    assert_eq!(w.view().flow().winner, Some(0));
}

#[test]
fn the_pursuit_timer_counts_the_remaining_routers_as_fled() {
    // 120 hastati against 20 velites pinned nowhere: the velites break,
    // the pursuit runs `pursuit_ticks` at most and the routers still on
    // the field are counted as fled.
    let regs = common::regs();
    // Open ground at y = 150 (y = 300 lies in the river band, where nobody
    // reaches anybody).
    let s = setup(vec![
        side(0, vec![reg(1, "rome:hastati", 120, Some([300.0, 150.0]))]),
        side(1, vec![reg(2, "rome:velites", 20, Some([340.0, 150.0]))]),
    ]);
    let mut w = BattleWorld::new(&s, regs.clone()).unwrap();
    // Both hold fire: javelins would break the hastati first.
    let commands = [
        command(
            1,
            0,
            CommandKind::FireMode {
                regiments: vec![RegimentId(0)],
                mode: FireMode::Hold,
            },
        ),
        command(
            1,
            1,
            CommandKind::FireMode {
                regiments: vec![RegimentId(1)],
                mode: FireMode::Hold,
            },
        ),
        Command {
            seq: 1,
            ..command(
                1,
                0,
                CommandKind::AttackRegiment {
                    regiments: vec![RegimentId(0)],
                    target: RegimentId(1),
                },
            )
        },
    ];
    let out = run(&mut w, &commands, 6_000);
    let ph = phases(&out.events);
    assert_eq!(ph.len(), 2, "{ph:?}");
    let pursuit_start = ph[0].0;
    let ended = ph[1].0;
    assert!(
        ended - pursuit_start <= regs.rules.battle_flow.pursuit_ticks,
        "{ph:?}"
    );
    // The pursuit closes when no Routing soldier remains: they left, died or
    // rallied (a rally ends it too, SIM-FLOW-015); nobody routs afterwards.
    let r = w.view().regiment(RegimentId(1)).unwrap();
    assert!(
        !w.ecs()
            .resource::<Ids>()
            .soldier_entities
            .iter()
            .any(|(_, e)| {
                w.ecs()
                    .get::<Fsm>(*e)
                    .is_some_and(|f| f.state == SoldierState::Routing)
            }),
        "routers left on the field"
    );
    let result = out
        .events
        .iter()
        .find_map(|(_, e)| match e {
            BattleEvent::Ended { result } => Some(result.clone()),
            _ => None,
        })
        .unwrap();
    let rr = &result.sides[1].regiments[0];
    assert_eq!(rr.initial, 21);
    assert_eq!(u32::from(rr.survivors), r.soldier_count);
    assert_eq!(rr.survivors + rr.killed + rr.fled, 21);
    assert_eq!(result.summary.total_fled, u32::from(rr.fled));
}

#[test]
fn phases_are_deterministic_across_threads_and_restores() {
    let regs = common::regs();
    let s = all_four();
    let commands = all_four_commands();
    let mut w = BattleWorld::new(&s, regs.clone()).unwrap();
    let mut hashes = run(&mut w, &commands, 7).hashes;
    let during_deployment = w.snapshot();
    assert_eq!(during_deployment.phase, BattlePhase::Deployment);
    hashes.extend(run(&mut w, &commands, 8_000).hashes);
    assert_eq!(w.phase(), BattlePhase::Ended);

    let mut w8 = BattleWorld::new(&s, regs.clone()).unwrap();
    w8.set_threads(8);
    let hashes8 = run(&mut w8, &commands, 8_000).hashes;
    if let Some(i) = hashes.iter().zip(&hashes8).position(|(a, b)| a != b) {
        panic!("1 vs 8 threads diverge at tick {}", i + 1);
    }
    let mut restored = BattleWorld::restore(&during_deployment, regs.clone()).unwrap();
    assert_eq!(restored.hash(), hashes[6]);
    let tail = run(&mut restored, &commands, 8_000).hashes;
    if let Some(i) = hashes[7..].iter().zip(&tail).position(|(a, b)| a != b) {
        panic!("restored (deployment) run diverges at tick {}", 8 + i);
    }
    // And once more from inside the pursuit.
    let mut w = BattleWorld::new(&s, regs.clone()).unwrap();
    let mut t = 0;
    while w.phase() != BattlePhase::Pursuit && t < 8_000 {
        t += 1;
        run(&mut w, &commands, t);
    }
    assert_eq!(w.phase(), BattlePhase::Pursuit);
    let snap = w.snapshot();
    let h = w.hash();
    let mut restored = BattleWorld::restore(&snap, regs).unwrap();
    assert_eq!(restored.hash(), h);
    let a = run(&mut w, &commands, t + 400).hashes;
    let b = run(&mut restored, &commands, t + 400).hashes;
    assert_eq!(a, b, "restored (pursuit) run diverges");
}
