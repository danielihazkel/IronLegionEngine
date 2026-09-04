//! T2-020: melee targeting and engagement (SIM-CMBT-001..005, SIM-CORE-011).
//!
//! Two opposing lines marched into contact produce a stable front: each
//! fighting soldier holds at most `reach + reach_slack` from its target,
//! the regiments are `engaged`, the attacker's anchor halts, attacker
//! counts match a recount, and the result is identical at 1 and 8 threads
//! and across a mid-clash snapshot.

mod common;

use std::collections::BTreeMap;

use common::*;
use il_core::{PlayerId, RegimentId, S, Scalar, SoldierId, StateHash, Tick, V2};
use il_sim_battle::components::{
    Anchor, Attackers, Combat, Fsm, MeleeState, Order, OrderKind, Regiment, Soldier, SoldierState,
};
use il_sim_battle::resources::Ids;
use il_sim_battle::{BattleSetup, BattleWorld, Command, CommandKind, RegimentSetup, RejectReason};

fn at(id: u32, unit: &str, count: u16, x: f32, y: f32, facing_deg: f32) -> RegimentSetup {
    RegimentSetup {
        id,
        unit_type: cid(unit),
        count,
        experience: 0,
        fatigue: 0.0,
        formation: None,
        position: Some([x, y]),
        facing_deg: Some(facing_deg),
    }
}

/// Two hastati regiments 40 m apart facing each other.
fn close_sides(count: u16) -> BattleSetup {
    let mut setup = two_sides(count);
    setup.sides[0].regiments = vec![at(1, "rome:hastati", count, 300.0, 150.0, 0.0)];
    setup.sides[1].regiments = vec![at(2, "rome:hastati", count, 340.0, 150.0, 180.0)];
    setup
}

fn command(tick: u32, player: u8, seq: u16, kind: CommandKind) -> Command {
    Command {
        tick: Tick(tick),
        player: PlayerId(player),
        seq,
        kind,
    }
}

fn attack_move(tick: u32, player: u8, regiments: &[u32], x: f32, y: f32) -> Command {
    command(
        tick,
        player,
        0,
        CommandKind::AttackMove {
            regiments: regiments.iter().map(|&r| RegimentId(r)).collect(),
            target: V2::from_f32_data(x, y),
        },
    )
}

fn attack_regiment(tick: u32, player: u8, regiments: &[u32], target: u32) -> Command {
    command(
        tick,
        player,
        0,
        CommandKind::AttackRegiment {
            regiments: regiments.iter().map(|&r| RegimentId(r)).collect(),
            target: RegimentId(target),
        },
    )
}

/// Steps `world` to `until`, feeding `commands` by tick, returning the
/// per-tick hashes; every command must be accepted.
fn run(world: &mut BattleWorld, commands: &[Command], until: u32) -> Vec<StateHash> {
    let mut hashes = Vec::new();
    while world.tick().0 < until {
        let next = world.tick().next();
        let batch: Vec<Command> = commands
            .iter()
            .filter(|c| c.tick == next)
            .cloned()
            .collect();
        let out = world.step(&batch);
        assert!(
            out.rejected.is_empty(),
            "tick {}: {:?}",
            next.0,
            out.rejected
        );
        hashes.push(out.hash);
    }
    hashes
}

fn regiment_entity(w: &BattleWorld, id: u32) -> bevy_ecs::entity::Entity {
    w.ecs()
        .resource::<Ids>()
        .regiment_entity(RegimentId(id))
        .expect("regiment exists")
}

fn order(w: &BattleWorld, id: u32) -> Order {
    *w.ecs().get::<Order>(regiment_entity(w, id)).unwrap()
}

fn fighting(w: &BattleWorld) -> Vec<(SoldierId, RegimentId, V2, Option<SoldierId>)> {
    w.view()
        .soldiers()
        .filter(|s| s.state == SoldierState::Fighting)
        .map(|s| (s.id, s.regiment, s.pos, s.target))
        .collect()
}

#[test]
fn two_lines_marched_into_contact_form_a_stable_front() {
    let setup = close_sides(120);
    let commands = [attack_move(1, 0, &[0], 340.0, 150.0)];
    let mut w = BattleWorld::new(&setup, regs()).unwrap();
    w.set_threads(1);
    let regs = w.registries().clone();
    let combat = regs.rules.combat.clone();

    let hashes = run(&mut w, &commands, 1_100);
    let anchor_before = w.ecs().get::<Anchor>(regiment_entity(&w, 0)).unwrap().pos;
    run(&mut w, &commands, 1_200);
    let anchor_after = w.ecs().get::<Anchor>(regiment_entity(&w, 0)).unwrap().pos;

    // Both regiments are engaged and the attacker's anchor holds.
    for id in [0, 1] {
        let c = w.ecs().get::<Combat>(regiment_entity(&w, id)).unwrap();
        assert!(c.engaged, "regiment {id} not engaged");
        assert_eq!(c.last_fighting, w.tick());
    }
    assert_eq!(order(&w, 0).kind, OrderKind::AttackMove);
    assert_eq!(order(&w, 0).target_regiment, Some(RegimentId(1)));
    assert_eq!(anchor_before, anchor_after, "engaged anchor moved");

    // A front: many fighters on each side, each within reach + slack of
    // its target (a small tolerance covers the drift between stagger ticks).
    let view = w.view();
    let fighters = fighting(&w);
    let per_side: BTreeMap<RegimentId, usize> =
        fighters.iter().fold(BTreeMap::new(), |mut m, f| {
            *m.entry(f.1).or_default() += 1;
            m
        });
    assert!(
        per_side.get(&RegimentId(0)).copied().unwrap_or(0) >= 20
            && per_side.get(&RegimentId(1)).copied().unwrap_or(0) >= 20,
        "fighters per side {per_side:?}"
    );
    let tolerance = S::from_f32_data(0.3);
    let mut close = 0;
    for (id, _, pos, target) in &fighters {
        let target = target.expect("fighting soldiers have targets");
        let other = view.soldier(target).expect("target alive");
        let r_i = regs
            .units
            .get(view.soldier(*id).unwrap().unit)
            .soldier_radius;
        let u = regs.units.get(other.unit);
        let limit = r_i + u.soldier_radius + u.reach + combat.reach_slack + tolerance;
        if pos.distance(other.pos) <= limit {
            close += 1;
        }
    }
    assert!(
        close * 10 >= fighters.len() * 8,
        "{close} of {} fighters within reach + slack",
        fighters.len()
    );

    // Attacker counts equal a fresh recount.
    let mut recount: BTreeMap<SoldierId, u8> = BTreeMap::new();
    for (_, _, _, target) in &fighters {
        *recount.entry(target.unwrap()).or_default() += 1;
    }
    for (sid, e) in &w.ecs().resource::<Ids>().soldier_entities {
        let n = w.ecs().get::<Attackers>(*e).unwrap().n;
        assert_eq!(
            n,
            recount.get(sid).copied().unwrap_or(0),
            "attackers of {sid:?}"
        );
    }
    // Non-fighting soldiers hold no target.
    for (_, e) in &w.ecs().resource::<Ids>().soldier_entities {
        let fsm = w.ecs().get::<Fsm>(*e).unwrap();
        let melee = w.ecs().get::<MeleeState>(*e).unwrap();
        if fsm.state != SoldierState::Fighting {
            assert_eq!(melee.target, None);
        }
    }

    // Same hashes at 8 threads.
    let mut w8 = BattleWorld::new(&setup, regs.clone()).unwrap();
    w8.set_threads(8);
    let hashes8 = run(&mut w8, &commands, 1_100);
    if let Some(i) = hashes.iter().zip(&hashes8).position(|(a, b)| a != b) {
        panic!("1 vs 8 threads diverge at tick {}", i + 1);
    }
}

#[test]
fn a_mid_clash_snapshot_restores_and_continues_identically() {
    let setup = close_sides(60);
    let commands = [attack_move(1, 0, &[0], 340.0, 150.0)];
    let mut w = BattleWorld::new(&setup, regs()).unwrap();
    run(&mut w, &commands, 700);
    assert!(!fighting(&w).is_empty(), "no fight by tick 700");
    let snap = w.snapshot();
    let mut restored = BattleWorld::restore(&snap, regs()).unwrap();
    assert_eq!(restored.hash(), w.hash());
    let a = run(&mut w, &commands, 1_000);
    let b = run(&mut restored, &commands, 1_000);
    if let Some(i) = a.iter().zip(&b).position(|(x, y)| x != y) {
        panic!("restored run diverges at tick {}", 700 + i + 1);
    }
}

#[test]
fn attack_regiment_is_validated_and_pursued() {
    let setup = two_sides(40);
    let mut w = BattleWorld::new(&setup, regs()).unwrap();
    // Own side, unknown, then a real enemy.
    let out = w.step(&[attack_regiment(1, 0, &[0], 0)]);
    assert!(matches!(
        out.rejected.as_slice(),
        [(_, RejectReason::InvalidTarget(RegimentId(0)))]
    ));
    let out = w.step(&[attack_regiment(2, 0, &[0], 7)]);
    assert!(matches!(
        out.rejected.as_slice(),
        [(_, RejectReason::UnknownRegiment(RegimentId(7)))]
    ));
    let out = w.step(&[attack_regiment(3, 0, &[0], 1)]);
    assert!(out.rejected.is_empty(), "{:?}", out.rejected);
    let o = order(&w, 0);
    assert_eq!(o.kind, OrderKind::AttackRegiment);
    assert_eq!(o.target_regiment, Some(RegimentId(1)));
    let start = w.ecs().get::<Anchor>(regiment_entity(&w, 0)).unwrap().pos;
    run(&mut w, &[], 200);
    let now = w.ecs().get::<Anchor>(regiment_entity(&w, 0)).unwrap().pos;
    assert!(
        now.x > start.x + S::from_i32(5),
        "not pursuing: {start:?} -> {now:?}"
    );
    // The target regiment's anchor is the path's destination.
    let enemy = w.ecs().get::<Anchor>(regiment_entity(&w, 1)).unwrap().pos;
    let path = w.view().path(RegimentId(0)).unwrap();
    let last = path.waypoints.last().unwrap().p;
    assert!(
        last.distance(enemy) < S::from_i32(5),
        "{last:?} vs {enemy:?}"
    );
}

#[test]
fn attack_move_acquires_an_enemy_within_the_radius_only() {
    // The enemy anchor 30 m north of the attack-move line is acquired.
    let mut near = two_sides(40);
    near.sides[0].regiments = vec![at(1, "rome:hastati", 40, 300.0, 150.0, 0.0)];
    near.sides[1].regiments = vec![at(2, "rome:hastati", 40, 360.0, 180.0, 180.0)];
    let mut w = BattleWorld::new(&near, regs()).unwrap();
    run(&mut w, &[attack_move(1, 0, &[0], 420.0, 150.0)], 900);
    assert_eq!(order(&w, 0).target_regiment, Some(RegimentId(1)));
    assert_eq!(order(&w, 0).kind, OrderKind::AttackMove);

    // 70 m off the line it is ignored and the move completes.
    let mut far = near.clone();
    far.sides[1].regiments = vec![at(2, "rome:hastati", 40, 360.0, 220.0, 180.0)];
    let mut w = BattleWorld::new(&far, regs()).unwrap();
    run(&mut w, &[attack_move(1, 0, &[0], 420.0, 150.0)], 2_400);
    assert_eq!(order(&w, 0).target_regiment, None);
    assert_eq!(order(&w, 0).kind, OrderKind::Idle);
    let anchor = w.ecs().get::<Anchor>(regiment_entity(&w, 0)).unwrap().pos;
    assert!(anchor.distance(V2::from_f32_data(420.0, 150.0)) < S::from_i32(5));
}

#[test]
fn an_attack_move_resumes_its_move_when_the_target_is_emptied() {
    let mut setup = two_sides(40);
    setup.sides[0].regiments = vec![at(1, "rome:hastati", 40, 300.0, 150.0, 0.0)];
    setup.sides[1].regiments = vec![at(2, "rome:hastati", 40, 340.0, 160.0, 180.0)];
    let mut w = BattleWorld::new(&setup, regs()).unwrap();
    let commands = [attack_move(1, 0, &[0], 500.0, 150.0)];
    run(&mut w, &commands, 300);
    assert_eq!(order(&w, 0).target_regiment, Some(RegimentId(1)));

    // Empty regiment 1 the way death will (T2-022): despawn its soldiers
    // and drop them from the id lists.
    let victims: Vec<SoldierId> = w
        .ecs()
        .get::<Regiment>(regiment_entity(&w, 1))
        .unwrap()
        .soldiers
        .clone();
    for sid in &victims {
        let e = w.ecs().resource::<Ids>().soldier_entity(*sid).unwrap();
        assert_eq!(w.ecs().get::<Soldier>(e).unwrap().regiment, RegimentId(1));
        w.ecs_mut().despawn(e);
        w.ecs_mut()
            .resource_mut::<Ids>()
            .soldier_entities
            .retain(|(id, _)| id != sid);
    }
    let e1 = regiment_entity(&w, 1);
    w.ecs_mut()
        .get_mut::<Regiment>(e1)
        .unwrap()
        .soldiers
        .clear();
    w.ecs_mut()
        .get_mut::<Regiment>(e1)
        .unwrap()
        .soldiers
        .clear();

    run(&mut w, &commands, 2_600);
    let o = order(&w, 0);
    assert_eq!(o.target_regiment, None);
    assert_eq!(o.kind, OrderKind::Idle, "the move to (500, 150) completed");
    let anchor = w.ecs().get::<Anchor>(regiment_entity(&w, 0)).unwrap().pos;
    assert!(anchor.distance(V2::from_f32_data(500.0, 150.0)) < S::from_i32(5));
}
