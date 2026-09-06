//! T2-081: the regiment AI and AI deployment (SIM-AI-020..022, SIM-AI-002,
//! SIM-CMD-005): square against cavalry, phalanx when engaged frontally,
//! deployment in a battle line, engage, hold fire behind a friend,
//! abilities under arrows, fall back, the bodyguard, command hygiene,
//! determinism across threads and a restore, and a replay with the AI off.

mod common;

use common::cid;
use il_core::{Angle, PlayerId, RegimentId, S, Scalar, StateHash, Tick, V2};
use il_data::Layout;
use il_sim_battle::components::{Anchor, Morale, Regiment};
use il_sim_battle::resources::{Ids, Sides};
use il_sim_battle::{
    AiState, ArmyPlan, BattleEvent, BattlePhase, BattleSetup, BattleWorld, Command, CommandKind,
    FireMode, RegimentSetup, SideSetup, Snapshot, StepOutput, polygon_contains,
};

fn regiment(id: u32, unit: &str, count: u16, x: f32, facing_deg: f32) -> RegimentSetup {
    common::regiment(id, unit, count, x, facing_deg)
}

/// A side in its own zone (0 or 1) owned by `player`.
fn side(player: u8, zone: u8, regiments: Vec<RegimentSetup>) -> SideSetup {
    SideSetup {
        deployment_zone: zone,
        ..common::side(player, regiments)
    }
}

fn setup(sides: Vec<SideSetup>) -> BattleSetup {
    BattleSetup {
        sides,
        ..common::two_sides(1)
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

/// Steps `ticks` ticks feeding `script` (by tick), collecting every event
/// with its tick, the AI commands and the rejections.
struct Run {
    events: Vec<(u32, BattleEvent)>,
    ai: Vec<Command>,
    rejected: usize,
}

fn run(w: &mut BattleWorld, script: &[Command], ticks: u32) -> Run {
    let mut r = Run {
        events: Vec::new(),
        ai: Vec::new(),
        rejected: 0,
    };
    for _ in 0..ticks {
        let next = w.tick().next();
        let cmds: Vec<Command> = script.iter().filter(|c| c.tick == next).cloned().collect();
        let out = w.step(&cmds);
        record(&mut r, next, &out);
    }
    r
}

fn record(r: &mut Run, tick: Tick, out: &StepOutput) {
    r.events
        .extend(out.events.iter().map(|e| (tick.0, e.clone())));
    r.ai.extend(out.ai_commands.iter().cloned());
    r.rejected += out.rejected.len();
}

fn regiment_entity(w: &BattleWorld, id: RegimentId) -> bevy_ecs::entity::Entity {
    w.ecs().resource::<Ids>().regiment_entity(id).unwrap()
}

fn anchor(w: &BattleWorld, id: RegimentId) -> V2 {
    w.view().regiment(id).unwrap().anchor_pos
}

fn layout(w: &BattleWorld, id: RegimentId) -> Layout {
    let row = w.view().regiment(id).unwrap();
    w.registries().formations.get(row.formation).layout
}

fn set_morale(w: &mut BattleWorld, id: RegimentId, m: i32) {
    let e = regiment_entity(w, id);
    w.ecs_mut().get_mut::<Morale>(e).unwrap().m = S::from_i32(m);
    w.recompute_hash();
}

/// (a) SIM-AI-021: an AI infantry regiment forms Square as enemy cavalry
/// closes, and not before.
#[test]
fn ai_infantry_forms_square_against_approaching_cavalry() {
    let s = setup(vec![
        side(0, 0, vec![regiment(1, "persia:cavalry", 60, 350.0, 0.0)]),
        side(255, 1, vec![regiment(2, "rome:hastati", 120, 500.0, 180.0)]),
    ]);
    let mut w = BattleWorld::new(&s, common::regs()).unwrap();
    let (cav, foot) = (RegimentId(0), RegimentId(1));
    assert_eq!(layout(&w, foot), Layout::Line);
    let charge = command(
        1,
        0,
        CommandKind::AttackRegiment {
            regiments: vec![cav],
            target: foot,
        },
    );
    let mut squared_at = None;
    let mut still_line_far = false;
    let mut rejected = 0;
    for _ in 0..1500 {
        let next = w.tick().next();
        let cmds = if next == Tick(1) {
            vec![charge.clone()]
        } else {
            vec![]
        };
        let out = w.step(&cmds);
        rejected += out.rejected.len();
        let d = anchor(&w, cav).distance(anchor(&w, foot));
        if layout(&w, foot) == Layout::Square {
            squared_at = Some((next.0, d));
            break;
        }
        // Not before the cavalry is within 60 m (the square action scores
        // 1.5 × (1 − d / 60) against its 0.5 threshold: at 40 m).
        if d > S::from_i32(60) {
            still_line_far = true;
        }
    }
    let (tick, d) = squared_at.expect("the hastati squared up");
    assert!(
        still_line_far,
        "the square came before the cavalry was within 60 m"
    );
    assert!(
        d <= S::from_i32(42) && d > S::from_i32(5),
        "square formed at {d:?} m (tick {tick})"
    );
    assert_eq!(rejected, 0);
}

/// (b) SIM-AI-021: AI hoplites in line switch to Phalanx once engaged
/// frontally.
#[test]
fn ai_hoplites_form_phalanx_when_engaged_frontally() {
    let mut hop = regiment(1, "greece:hoplite", 160, 500.0, 180.0);
    hop.formation = Some(cid("rome:line"));
    let s = setup(vec![
        side(0, 0, vec![regiment(2, "rome:hastati", 160, 400.0, 0.0)]),
        side(255, 1, vec![hop]),
    ]);
    let mut w = BattleWorld::new(&s, common::regs()).unwrap();
    let (hastati, hoplites) = (RegimentId(0), RegimentId(1));
    assert_eq!(layout(&w, hoplites), Layout::Line);
    let script = [
        command(
            1,
            0,
            CommandKind::FireMode {
                regiments: vec![hastati],
                mode: FireMode::Hold,
            },
        ),
        command(
            1,
            0,
            CommandKind::AttackRegiment {
                regiments: vec![hastati],
                target: hoplites,
            },
        ),
    ];
    let mut engaged_at = None;
    let mut phalanx_at = None;
    for _ in 0..2500 {
        let next = w.tick().next();
        let cmds: Vec<Command> = script.iter().filter(|c| c.tick == next).cloned().collect();
        let out = w.step(&cmds);
        assert!(out.rejected.is_empty(), "{:?}", out.rejected);
        if engaged_at.is_none()
            && out
                .events
                .iter()
                .any(|e| matches!(e, BattleEvent::Engaged { regiment } if *regiment == hoplites))
        {
            engaged_at = Some(next.0);
        }
        if layout(&w, hoplites) == Layout::Phalanx {
            phalanx_at = Some(next.0);
            break;
        }
        common::pin_morale(&mut w);
    }
    let engaged = engaged_at.expect("the hastati reached the hoplites");
    let phalanx = phalanx_at.expect("the hoplites formed a phalanx");
    assert!(
        phalanx > engaged && phalanx <= engaged + 45,
        "phalanx at {phalanx}, engaged at {engaged}"
    );
}

/// (c) SIM-AI-020: an unplaced AI side deploys in a battle line (ranged
/// ahead, cavalry on the flanks, the bodyguard behind the centre) inside
/// its zone and confirms, all through Stage 1 commands.
#[test]
fn ai_side_deploys_in_a_battle_line_and_confirms() {
    let unplaced = |id: u32, unit: &str, count: u16| RegimentSetup {
        position: None,
        facing_deg: None,
        ..regiment(id, unit, count, 0.0, 0.0)
    };
    let s = setup(vec![
        side(0, 0, vec![regiment(1, "rome:hastati", 60, 300.0, 90.0)]),
        side(
            255,
            1,
            vec![
                unplaced(2, "rome:hastati", 80),
                unplaced(3, "rome:hastati", 80),
                unplaced(4, "rome:velites", 60),
                unplaced(5, "persia:cavalry", 30),
            ],
        ),
    ]);
    let mut w = BattleWorld::new(&s, common::regs()).unwrap();
    assert_eq!(w.phase(), BattlePhase::Deployment);
    let r = run(&mut w, &[], 3);
    assert_eq!(r.rejected, 0);
    // Tick 1's Stage 1 queued four Deploys and a confirmation for tick 2.
    let deploys =
        r.ai.iter()
            .filter(|c| c.tick == Tick(2) && matches!(c.kind, CommandKind::Deploy { .. }))
            .count();
    assert_eq!(deploys, 4, "{:?}", r.ai);
    assert!(
        r.ai.iter()
            .any(|c| c.tick == Tick(2) && matches!(c.kind, CommandKind::ConfirmDeployment))
    );
    assert!(r.ai.iter().all(|c| c.player == PlayerId::ENGINE_AI));
    assert!(
        r.events
            .iter()
            .any(|(t, e)| *t == 2 && matches!(e, BattleEvent::DeploymentConfirmed { side: 1 }))
    );
    assert!(r.events.iter().any(|(t, e)| *t == 2
        && matches!(
            e,
            BattleEvent::PhaseChanged {
                from: BattlePhase::Deployment,
                to: BattlePhase::Battle
            }
        )));
    assert_eq!(w.phase(), BattlePhase::Battle);
    // Geometry: everyone inside zone 1's polygon, facing the enemy zone
    // (south), velites ahead, cavalry on a flank, bodyguard behind.
    let map = w.map().clone();
    let poly = map.deployment_polygon(1).unwrap();
    let (a, b, v, c) = (RegimentId(1), RegimentId(2), RegimentId(3), RegimentId(4));
    for id in [a, b, v, c] {
        assert!(
            polygon_contains(poly, anchor(&w, id)),
            "{id:?} outside its zone"
        );
    }
    let facing = w.view().regiment(b).unwrap().anchor_facing;
    let forward = facing.direction();
    assert!(forward.y < S::ZERO, "faces south: {forward:?}");
    let right = V2::new(forward.y, -forward.x);
    let ahead = |id: RegimentId| anchor(&w, id).dot(forward);
    let lateral = |id: RegimentId| anchor(&w, id).dot(right);
    assert!(
        ahead(v) > ahead(b) + S::from_i32(5),
        "velites ahead of the line"
    );
    assert!(
        ahead(a) < ahead(b) - S::from_i32(30),
        "the bodyguard behind the centre"
    );
    assert!(
        lateral(c) > lateral(b) + S::from_i32(10) || lateral(c) < lateral(b) - S::from_i32(10),
        "cavalry on a flank"
    );
}

/// (d) `engage_nearest`: an enemy standing 30 m away draws an
/// `AttackRegiment` at the first due tick, once.
#[test]
fn ai_regiment_engages_a_close_enemy_once() {
    // The bodyguard is the far regiment: SIM-AI-022 keeps a bodyguard out
    // of an even fight (test (h)).
    let mut s = setup(vec![
        side(0, 0, vec![regiment(1, "rome:hastati", 120, 470.0, 0.0)]),
        side(
            255,
            1,
            vec![
                regiment(2, "rome:hastati", 120, 500.0, 180.0),
                regiment(3, "rome:hastati", 20, 700.0, 180.0),
            ],
        ),
    ]);
    s.sides[1].general.bodyguard = Some(3);
    let mut w = BattleWorld::new(&s, common::regs()).unwrap();
    let r = run(&mut w, &[], 60);
    assert_eq!(r.rejected, 0);
    let attacks: Vec<&Command> =
        r.ai.iter()
            .filter(|c| matches!(c.kind, CommandKind::AttackRegiment { .. }))
            .collect();
    assert_eq!(attacks.len(), 1, "{attacks:?}");
    assert!(
        matches!(&attacks[0].kind, CommandKind::AttackRegiment { regiments, target } if regiments == &vec![RegimentId(1)] && *target == RegimentId(0))
    );
    assert!(
        attacks[0].tick.0 <= 22,
        "first due tick: {:?}",
        attacks[0].tick
    );
    assert_eq!(
        w.view().regiment(RegimentId(1)).unwrap().order,
        il_sim_battle::components::OrderKind::AttackRegiment
    );
}

/// (e) `hold_fire` when an own regiment stands in the line of fire, and
/// back to fire at will when it steps aside.
#[test]
fn ai_skirmishers_hold_fire_behind_a_friend() {
    let s = setup(vec![
        side(0, 0, vec![regiment(1, "rome:hastati", 60, 440.0, 0.0)]),
        side(
            255,
            1,
            vec![
                regiment(2, "rome:hastati", 60, 470.0, 180.0),
                regiment(3, "rome:velites", 60, 500.0, 180.0),
            ],
        ),
    ]);
    let mut w = BattleWorld::new(&s, common::regs()).unwrap();
    let (friend, velites) = (RegimentId(1), RegimentId(2));
    let r = run(&mut w, &[], 25);
    assert_eq!(r.rejected, 0);
    assert!(r.ai.iter().any(|c| matches!(
        &c.kind,
        CommandKind::FireMode { regiments, mode: FireMode::Hold } if regiments == &vec![velites]
    )));
    assert_eq!(
        w.view().regiment(velites).unwrap().fire,
        Some(FireMode::Hold)
    );
    // The friend moves out of the line: 80 m to the side.
    let e = regiment_entity(&w, friend);
    w.ecs_mut().get_mut::<Anchor>(e).unwrap().pos = V2::from_f32_data(470.0, 230.0);
    w.recompute_hash();
    let r = run(&mut w, &[], 25);
    assert!(r.ai.iter().any(|c| matches!(
        &c.kind,
        CommandKind::FireMode { regiments, mode: FireMode::FireAtWill } if regiments == &vec![velites]
    )));
    assert_eq!(
        w.view().regiment(velites).unwrap().fire,
        Some(FireMode::FireAtWill)
    );
}

/// (f) `use_ability`: AI hastati under javelins raise the testudo at their
/// first due tick and not again while it cools down.
#[test]
fn ai_hastati_use_testudo_under_arrows() {
    let s = setup(vec![
        side(0, 0, vec![regiment(1, "rome:velites", 120, 470.0, 0.0)]),
        side(255, 1, vec![regiment(2, "rome:hastati", 120, 500.0, 180.0)]),
    ]);
    let mut w = BattleWorld::new(&s, common::regs()).unwrap();
    let hastati = RegimentId(1);
    let mut r = Run {
        events: Vec::new(),
        ai: Vec::new(),
        rejected: 0,
    };
    for _ in 0..400 {
        let next = w.tick().next();
        let out = w.step(&[]);
        record(&mut r, next, &out);
        common::pin_morale(&mut w);
    }
    assert_eq!(r.rejected, 0);
    let uses: Vec<u32> = r
        .ai
        .iter()
        .filter(
            |c| matches!(&c.kind, CommandKind::UseAbility { regiment, .. } if *regiment == hastati),
        )
        .map(|c| c.tick.0)
        .collect();
    assert_eq!(uses.len(), 1, "{uses:?}");
    assert!(uses[0] <= 22);
    assert!(r.events.iter().any(|(t, e)| *t == uses[0]
        && matches!(e, BattleEvent::AbilityUsed { regiment, .. } if *regiment == hastati)));
    assert!(!w.view().statuses(hastati).is_empty() || w.tick().0 > uses[0] + 400);
}

/// (g) `fall_back`: a shaken regiment with a plan walks to the reserve
/// position behind the line.
#[test]
fn ai_regiment_falls_back_when_its_morale_is_low() {
    let s = setup(vec![
        side(0, 0, vec![regiment(1, "rome:hastati", 120, 440.0, 0.0)]),
        side(255, 1, vec![regiment(2, "rome:hastati", 120, 500.0, 180.0)]),
    ]);
    let mut w = BattleWorld::new(&s, common::regs()).unwrap();
    let me = RegimentId(1);
    let plan = ArmyPlan {
        line_anchor: anchor(&w, me),
        line_facing: Angle::from_degrees_data(180.0),
        ..ArmyPlan::default()
    };
    w.ecs_mut().resource_mut::<AiState>().plans = vec![None, Some(plan)];
    set_morale(&mut w, me, 20);
    let mut r = Run {
        events: Vec::new(),
        ai: Vec::new(),
        rejected: 0,
    };
    for _ in 0..25 {
        let next = w.tick().next();
        let out = w.step(&[]);
        record(&mut r, next, &out);
        set_morale(&mut w, me, 20);
    }
    assert_eq!(r.rejected, 0);
    let moves: Vec<V2> =
        r.ai.iter()
            .filter_map(|c| match &c.kind {
                CommandKind::Move {
                    regiments, target, ..
                } if regiments == &vec![me] => Some(*target),
                _ => None,
            })
            .collect();
    assert_eq!(moves.len(), 1, "{moves:?}");
    // 60 m behind a line facing west (180°) is 60 m east.
    let expected = V2::from_f32_data(560.0, 150.0);
    assert!(
        moves[0].distance(expected) < S::from_i32(2),
        "{:?}",
        moves[0]
    );
    assert_eq!(
        w.view().regiment(me).unwrap().order,
        il_sim_battle::components::OrderKind::Move
    );
}

/// (h) SIM-AI-022: the bodyguard follows the side's other regiments and
/// does not engage an even enemy.
#[test]
fn ai_bodyguard_follows_the_centroid_and_does_not_engage() {
    let mut s = setup(vec![
        side(0, 0, vec![regiment(1, "rome:hastati", 120, 470.0, 0.0)]),
        side(
            255,
            1,
            vec![
                regiment(2, "rome:hastati", 120, 500.0, 180.0),
                regiment(3, "rome:hastati", 120, 560.0, 180.0),
            ],
        ),
    ]);
    s.sides[1].general.bodyguard = Some(2);
    let mut w = BattleWorld::new(&s, common::regs()).unwrap();
    let (bodyguard, other) = (RegimentId(1), RegimentId(2));
    assert_eq!(
        w.ecs().resource::<Sides>().0[1].general_regiment,
        Some(bodyguard)
    );
    let r = run(&mut w, &[], 45);
    assert_eq!(r.rejected, 0);
    assert!(!r.ai.iter().any(|c| matches!(
        &c.kind,
        CommandKind::AttackRegiment { regiments, .. } if regiments == &vec![bodyguard]
    )));
    let follow: Vec<V2> =
        r.ai.iter()
            .filter_map(|c| match &c.kind {
                CommandKind::Move {
                    regiments, target, ..
                } if regiments == &vec![bodyguard] => Some(*target),
                _ => None,
            })
            .collect();
    assert_eq!(follow.len(), 1, "{follow:?}");
    assert!(follow[0].distance(anchor(&w, other)) < S::from_i32(2));
}

fn fight() -> BattleSetup {
    setup(vec![
        side(
            0,
            0,
            vec![
                regiment(1, "rome:hastati", 100, 380.0, 0.0),
                regiment(2, "rome:velites", 60, 360.0, 0.0),
            ],
        ),
        side(
            255,
            1,
            vec![
                regiment(3, "rome:hastati", 100, 520.0, 180.0),
                regiment(4, "greece:hoplite", 80, 540.0, 180.0),
                regiment(5, "persia:cavalry", 30, 560.0, 180.0),
            ],
        ),
    ])
}

fn attack_all() -> Vec<Command> {
    vec![command(
        1,
        0,
        CommandKind::AttackMove {
            regiments: vec![RegimentId(0), RegimentId(1)],
            target: V2::from_f32_data(540.0, 150.0),
        },
    )]
}

/// (i) Command hygiene: over a whole fight the AI never has a command
/// rejected (plan I6).
#[test]
fn ai_commands_are_never_rejected_over_a_fight() {
    let mut w = BattleWorld::new(&fight(), common::regs()).unwrap();
    let r = run(&mut w, &attack_all(), 3000);
    assert_eq!(r.rejected, 0);
    assert!(
        r.ai.len() > 10,
        "the AI decided something: {} commands",
        r.ai.len()
    );
}

/// (j) Determinism: one thread against eight with a restore between two
/// regiment periods; the AI commands match tick by tick.
#[test]
fn ai_battle_is_deterministic_across_threads_and_a_restore() {
    let regs = common::regs();
    let s = fight();
    let script = attack_all();
    let mut a = BattleWorld::new(&s, regs.clone()).unwrap();
    let mut b = BattleWorld::new(&s, regs.clone()).unwrap();
    b.set_threads(8);
    let mut snap: Option<Snapshot> = None;
    let mut hashes: Vec<StateHash> = Vec::new();
    let mut ai_log: Vec<Vec<Command>> = Vec::new();
    for _ in 0..3000 {
        let next = a.tick().next();
        let cmds: Vec<Command> = script.iter().filter(|c| c.tick == next).cloned().collect();
        let oa = a.step(&cmds);
        let ob = b.step(&cmds);
        assert_eq!(oa.hash, ob.hash, "threads diverged at tick {}", next.0);
        assert_eq!(oa.ai_commands, ob.ai_commands);
        hashes.push(oa.hash);
        ai_log.push(oa.ai_commands.clone());
        if next == Tick(1011) {
            snap = Some(a.snapshot());
        }
    }
    let mut c = BattleWorld::restore(&snap.unwrap(), regs).unwrap();
    for tick in 1012..=3000u32 {
        let out = c.step(&[]);
        assert_eq!(
            out.hash,
            hashes[tick as usize - 1],
            "restore diverged at tick {tick}"
        );
        assert_eq!(out.ai_commands, ai_log[tick as usize - 1]);
    }
}

/// (k) Networking Spec §2.7: feeding the logged AI commands to a run with
/// the AI off reproduces the same battle (the outbox itself is hashed
/// state, so the per-tick hashes differ by exactly that; positions,
/// counts and events agree).
#[test]
fn logged_ai_commands_replay_the_battle_with_the_ai_off() {
    let regs = common::regs();
    let s = fight();
    let script = attack_all();
    let mut live = BattleWorld::new(&s, regs.clone()).unwrap();
    let mut replay = BattleWorld::new(&s, regs).unwrap();
    replay.set_ai_enabled(false);
    let mut pending: Vec<Command> = Vec::new();
    for _ in 0..1500 {
        let next = live.tick().next();
        let cmds: Vec<Command> = script.iter().filter(|c| c.tick == next).cloned().collect();
        let out = live.step(&cmds);
        let mut fed = cmds.clone();
        fed.append(&mut pending);
        let out_replay = replay.step(&fed);
        pending = out.ai_commands.clone();
        assert!(out_replay.ai_commands.is_empty());
        assert_eq!(
            out.events, out_replay.events,
            "events diverged at tick {}",
            next.0
        );
        assert_eq!(out.rejected, out_replay.rejected);
        let a: Vec<_> = live
            .view()
            .soldiers()
            .map(|s| (s.id, s.pos, s.hp))
            .collect();
        let b: Vec<_> = replay
            .view()
            .soldiers()
            .map(|s| (s.id, s.pos, s.hp))
            .collect();
        assert_eq!(a, b, "soldiers diverged at tick {}", next.0);
    }
    let count = |w: &BattleWorld| {
        w.view()
            .regiments()
            .map(|r| (r.id, r.soldier_count))
            .collect::<Vec<_>>()
    };
    assert_eq!(count(&live), count(&replay));
    assert!(
        live.ecs()
            .resource::<Ids>()
            .regiment_entities
            .iter()
            .all(|(id, e)| live.ecs().get::<Regiment>(*e).is_some() && id.0 < 5)
    );
}
