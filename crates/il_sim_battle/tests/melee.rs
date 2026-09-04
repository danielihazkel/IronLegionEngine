//! T2-021: melee resolution (SIM-CMBT-010..018).
//!
//! A hastati clash wounds both sides identically at 1 and 8 threads and
//! across a mid-fight restore; a rear attack hurts more than a frontal one
//! over several seeds; a running cavalry regiment opens a charge window on
//! contact and its soldiers carry the charge mass only inside it.

mod common;

use common::*;
use std::sync::Arc;

use il_core::{PlayerId, RegimentId, S, Scalar, SoldierId, StateHash, Tick};
use il_sim_battle::components::{Body, Combat, Health, Regiment};
use il_sim_battle::resources::Ids;
use il_sim_battle::{
    BattleEvent, BattleSetup, BattleWorld, Command, CommandKind, RegimentSetup, SpeedMode,
};

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

fn command(tick: u32, player: u8, kind: CommandKind) -> Command {
    Command {
        tick: Tick(tick),
        player: PlayerId(player),
        seq: 0,
        kind,
    }
}

fn attack_regiment(tick: u32, player: u8, regiment: u32, target: u32) -> Command {
    command(
        tick,
        player,
        CommandKind::AttackRegiment {
            regiments: vec![RegimentId(regiment)],
            target: RegimentId(target),
        },
    )
}

fn run(world: &mut BattleWorld, commands: &[Command], until: u32) -> Vec<StateHash> {
    run_collecting(world, commands, until, &mut Vec::new())
}

/// A death as seen from the events: victim, killer, victim's regiment, the
/// killer's regiment (when the killer was still alive after the tick).
type Death = (SoldierId, Option<SoldierId>, RegimentId, Option<RegimentId>);

/// [`run`] that also collects every death.
fn run_collecting(
    world: &mut BattleWorld,
    commands: &[Command],
    until: u32,
    deaths: &mut Vec<Death>,
) -> Vec<StateHash> {
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
        for e in &out.events {
            if let BattleEvent::SoldierDied {
                id,
                regiment,
                killer,
                ..
            } = e
            {
                let killer_regiment =
                    killer.and_then(|k| world.view().soldier(k).map(|s| s.regiment));
                deaths.push((*id, *killer, *regiment, killer_regiment));
            }
        }
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

/// Sum of hp over a regiment's soldiers.
fn hp_sum(w: &BattleWorld, id: u32) -> S {
    let ids = w.ecs().resource::<Ids>();
    let r = w.ecs().get::<Regiment>(regiment_entity(w, id)).unwrap();
    r.soldiers
        .iter()
        .filter_map(|s| ids.soldier_entity(*s))
        .map(|e| w.ecs().get::<Health>(e).unwrap().hp)
        .fold(S::ZERO, |a, b| a + b)
}

fn two_hastati(count: u16, x0: f32, f0: f32, x1: f32, f1: f32) -> BattleSetup {
    let mut setup = two_sides(count);
    setup.sides[0].regiments = vec![at(1, "rome:hastati", count, x0, 150.0, f0)];
    setup.sides[1].regiments = vec![at(2, "rome:hastati", count, x1, 150.0, f1)];
    setup
}

#[test]
fn a_hastati_clash_wounds_both_sides_deterministically() {
    let setup = two_hastati(120, 300.0, 0.0, 340.0, 180.0);
    let commands = [attack_regiment(1, 0, 0, 1)];
    let regs = regs();
    let full0 = S::from_i32(120)
        * regs
            .units
            .get(regs.units.lookup(&cid("rome:hastati")).unwrap())
            .hp;

    let mut w = BattleWorld::new(&setup, regs.clone()).unwrap();
    let mut deaths = Vec::new();
    let mut hashes = run_collecting(&mut w, &commands, 1_000, &mut deaths);
    let snap = w.snapshot();
    hashes.extend(run_collecting(&mut w, &commands, 3_000, &mut deaths));
    assert!(hp_sum(&w, 0) < full0, "side 0 took no damage");
    assert!(hp_sum(&w, 1) < full0, "side 1 took no damage");
    assert!(!deaths.is_empty(), "nobody died in 3,000 ticks");
    for (victim, killer, regiment, killer_regiment) in &deaths {
        assert!(killer.is_some(), "{victim:?} died without a killer");
        assert!(
            w.ecs().resource::<Ids>().soldier_entity(*victim).is_none(),
            "{victim:?} still alive"
        );
        if let Some(kr) = killer_regiment {
            assert_ne!(kr, regiment, "{victim:?} killed by an ally");
        }
    }
    assert_eq!(w.soldier_count(), 240 - deaths.len());

    // Eight threads, and a restore from tick 1,000, reproduce the hashes.
    let mut w8 = BattleWorld::new(&setup, regs.clone()).unwrap();
    w8.set_threads(8);
    let hashes8 = run(&mut w8, &commands, 3_000);
    if let Some(i) = hashes.iter().zip(&hashes8).position(|(a, b)| a != b) {
        panic!("1 vs 8 threads diverge at tick {}", i + 1);
    }
    let mut restored = BattleWorld::restore(&snap, regs).unwrap();
    let tail = run(&mut restored, &commands, 3_000);
    if let Some(i) = hashes[1_000..].iter().zip(&tail).position(|(a, b)| a != b) {
        panic!("restored run diverges at tick {}", 1_000 + i + 1);
    }
}

#[test]
fn a_rear_attack_hurts_more_than_a_frontal_one() {
    // The attacker always comes from x = 300 heading +x at regiment 1 on
    // x = 340; only the defender's facing differs: toward the attacker
    // (frontal) or away from it (rear), so terrain plays no part. Soldiers
    // cannot turn (`soldier_turn_rate` 0), so the arc holds for the whole
    // fight instead of the few ticks a defender needs to face its attacker.
    let mut front_total = S::ZERO;
    let mut rear_total = S::ZERO;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game");
    let mut stiff = il_data::Registries::load_root(&root).unwrap_or_else(|d| panic!("{d}"));
    stiff.rules.movement.soldier_turn_rate = S::ZERO;
    let regs = Arc::new(stiff);
    for seed in 1..=5u64 {
        for rear in [false, true] {
            let mut setup = two_hastati(60, 300.0, 0.0, 340.0, if rear { 0.0 } else { 180.0 });
            setup.seed = seed;
            let mut w = BattleWorld::new(&setup, regs.clone()).unwrap();
            let before = hp_sum(&w, 1);
            run(&mut w, &[attack_regiment(1, 0, 0, 1)], 1_200);
            let damage = before - hp_sum(&w, 1);
            if rear {
                rear_total = rear_total + damage;
            } else {
                front_total = front_total + damage;
            }
        }
    }
    assert!(
        rear_total > front_total,
        "rear {rear_total:?} vs front {front_total:?} over five seeds"
    );
}

#[test]
fn a_charge_opens_a_window_and_doubles_the_mass_inside_it() {
    let mut setup = two_sides(120);
    setup.sides[0].regiments = vec![at(1, "rome:hastati", 120, 300.0, 150.0, 0.0)];
    setup.sides[1].regiments = vec![at(2, "persia:cavalry", 60, 380.0, 150.0, 180.0)];
    let regs = regs();
    let cav = regs
        .units
        .get(regs.units.lookup(&cid("persia:cavalry")).unwrap());
    let (mass, mult, window) = (
        cav.mass,
        regs.rules.combat.charge_mass_mult,
        u32::from(regs.rules.combat.charge_window_ticks),
    );
    let mut w = BattleWorld::new(&setup, regs).unwrap();
    let commands = [attack_regiment(1, 1, 1, 0)];
    let cav_mass = |w: &BattleWorld| -> Vec<S> {
        let ids = w.ecs().resource::<Ids>();
        let r = w.ecs().get::<Regiment>(regiment_entity(w, 1)).unwrap();
        r.soldiers
            .iter()
            .map(|s| {
                w.ecs()
                    .get::<Body>(ids.soldier_entity(*s).unwrap())
                    .unwrap()
                    .m
            })
            .collect()
    };

    // Run until the cavalry engages; it must be running by then.
    let mut charge_tick = None;
    let mut charge_event: Option<RegimentId> = None;
    while w.tick().0 < 1_500 && charge_tick.is_none() {
        let next = w.tick().next();
        let batch: Vec<Command> = commands
            .iter()
            .filter(|c| c.tick == next)
            .cloned()
            .collect();
        let out = w.step(&batch);
        assert!(out.rejected.is_empty());
        for e in &out.events {
            if let BattleEvent::Charge { regiment, target } = e {
                assert_eq!(*regiment, RegimentId(1));
                charge_event = Some(*target);
            }
        }
        let c = w.ecs().get::<Combat>(regiment_entity(&w, 1)).unwrap();
        if c.engaged {
            charge_tick = Some(w.tick().0);
        }
    }
    let t = charge_tick.expect("cavalry never engaged");
    let c = *w.ecs().get::<Combat>(regiment_entity(&w, 1)).unwrap();
    let order = w
        .ecs()
        .get::<il_sim_battle::components::Order>(regiment_entity(&w, 1))
        .unwrap();
    assert_eq!(order.speed, SpeedMode::Run, "not charging at a run");
    assert_eq!(c.charge_until, Tick(t + window));
    assert_eq!(charge_event, Some(RegimentId(0)));
    assert!(
        cav_mass(&w).iter().all(|m| *m == mass * mult),
        "mass not raised"
    );

    // Inside the window the mass stays raised; on its last tick it drops.
    run(&mut w, &commands, t + window - 1);
    assert!(cav_mass(&w).iter().all(|m| *m == mass * mult));
    run(&mut w, &commands, t + window);
    assert!(cav_mass(&w).iter().all(|m| *m == mass), "mass not restored");

    // A restore inside the window brings the raised mass back.
    let mut w2 = BattleWorld::new(&setup, common::regs()).unwrap();
    run(&mut w2, &commands, t + 10);
    let snap = w2.snapshot();
    let restored = BattleWorld::restore(&snap, common::regs()).unwrap();
    assert!(cav_mass(&restored).iter().all(|m| *m == mass * mult));
}
